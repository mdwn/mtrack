// Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
//
// This program is free software: you can redistribute it and/or modify it under
// the terms of the GNU General Public License as published by the Free Software
// Foundation, version 3.
//
// This program is distributed in the hope that it will be useful, but WITHOUT
// ANY WARRANTY; without even the implied warranty of MERCHANTABILITY or FITNESS
// FOR A PARTICULAR PURPOSE. See the GNU General Public License for more details.
//
// You should have received a copy of the GNU General Public License along with
// this program. If not, see <https://www.gnu.org/licenses/>.
//

//! What a phone is told of the player over Bluetooth: the shared status
//! (`controller::remote_status`) cut to fit an attribute, pushed only when
//! it says something new.

use std::time::Instant;

use prost::Message;

use crate::proto::player::v1::RemoteStatus;

use super::super::remote_status::should_send;

/// The most bytes an encoded [`RemoteStatus`] takes. A read of the status
/// characteristic returns it bare, in one attribute value, and a long
/// playlist or song name must not grow it past what an attribute holds.
pub(crate) const MAX_STATUS_LEN: usize = 512;

/// `status` encoded, in at most [`MAX_STATUS_LEN`] bytes: when it is longer,
/// the longest name loses bytes (at a character boundary) until it fits.
/// The rest of a status is a handful of numbers, so it always does.
pub(crate) fn encode_capped(status: &RemoteStatus) -> Vec<u8> {
    let mut status = status.clone();
    loop {
        let len = status.encoded_len();
        if len <= MAX_STATUS_LEN {
            return status.encode_to_vec();
        }
        let excess = len - MAX_STATUS_LEN;
        let auto_song = status.auto_play.as_mut().map(|a| &mut a.song);
        let longest = [
            Some(&mut status.playlist_name),
            Some(&mut status.song_name),
            auto_song,
            status.loop_section.as_mut(),
        ]
        .into_iter()
        .flatten()
        .max_by_key(|name| name.len())
        .expect("a status has a playlist name field");
        let mut keep = longest.len().saturating_sub(excess);
        while !longest.is_char_boundary(keep) {
            keep -= 1;
        }
        longest.truncate(keep);
    }
}

/// What the phone is told of the player, and when.
#[derive(Default)]
pub(crate) struct StatusFeed {
    /// The last status pushed, and when.
    sent: Option<(RemoteStatus, Instant)>,
    /// The latest status polled, encoded: what a read returns.
    latest: Vec<u8>,
}

impl StatusFeed {
    /// Takes the latest poll. The status to push, encoded, when it is due.
    pub(crate) fn offer(&mut self, now: Instant, current: RemoteStatus) -> Option<Vec<u8>> {
        let encoded = encode_capped(&current);
        self.latest.clone_from(&encoded);
        let due = match &self.sent {
            None => true,
            Some((sent, at)) => should_send(sent, *at, now, &current),
        };
        if !due {
            return None;
        }
        self.sent = Some((current, now));
        Some(encoded)
    }

    /// Takes back the last push, which never went out: the next poll sends.
    pub(crate) fn unsend(&mut self) {
        self.sent = None;
    }

    /// The latest status, encoded; empty before the first poll.
    pub(crate) fn latest(&self) -> &[u8] {
        &self.latest
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn secs(s: i64) -> Option<prost_types::Duration> {
        Some(prost_types::Duration {
            seconds: s,
            nanos: 0,
        })
    }

    fn playing_at(elapsed: i64) -> RemoteStatus {
        RemoteStatus {
            playlist_name: "Set".into(),
            song_name: "Song".into(),
            song_duration: secs(200),
            playing: true,
            elapsed: secs(elapsed),
            position: Some(1),
            auto_play: None,
            loop_section: None,
            gains_revision: Some(0),
            ambient_mode: None,
            ambient_state: None,
            locked: Some(false),
        }
    }

    /// The poll tells a phone the ambient light's mode and state, and of a
    /// change to the mode without being asked.
    #[test]
    fn a_long_name_is_cut_so_the_status_fits_one_attribute() {
        let mut status = playing_at(1);
        status.song_name = "é".repeat(2000);
        status.playlist_name = "p".repeat(100);
        let encoded = encode_capped(&status);
        assert!(encoded.len() <= MAX_STATUS_LEN, "{}", encoded.len());
        let decoded = RemoteStatus::decode(encoded.as_slice()).expect("a valid status");
        // The longest name gave way, at a character boundary, the rest stayed.
        assert!(decoded.song_name.chars().all(|c| c == 'é'));
        assert!(!decoded.song_name.is_empty());
        assert_eq!(decoded.playlist_name, "p".repeat(100));
        assert_eq!(decoded.elapsed, status.elapsed);
        assert_eq!(decoded.song_duration, status.song_duration);
    }

    #[test]
    fn every_name_too_long_still_fits() {
        let mut status = playing_at(1);
        status.playlist_name = "a".repeat(5000);
        status.song_name = "b".repeat(5000);
        status.auto_play = Some(crate::proto::player::v1::AutoPlayStatus {
            song: "c".repeat(5000),
            starts_in: secs(3),
        });
        status.loop_section = Some("d".repeat(5000));
        let encoded = encode_capped(&status);
        assert!(encoded.len() <= MAX_STATUS_LEN, "{}", encoded.len());
        let decoded = RemoteStatus::decode(encoded.as_slice()).expect("a valid status");
        assert_eq!(
            decoded.auto_play.expect("auto play kept").starts_in,
            secs(3)
        );
    }

    #[test]
    fn the_levels_and_ambient_fields_survive_the_cut() {
        let mut status = playing_at(1);
        status.gains_revision = Some(u32::MAX);
        status.ambient_mode = Some("auto".into());
        status.ambient_state = Some("unavailable".into());
        status.locked = Some(true);
        status.playlist_name = "a".repeat(5000);
        status.song_name = "b".repeat(5000);
        status.loop_section = Some("d".repeat(5000));
        let encoded = encode_capped(&status);
        assert!(encoded.len() <= MAX_STATUS_LEN, "{}", encoded.len());
        let decoded = RemoteStatus::decode(encoded.as_slice()).expect("a valid status");
        assert_eq!(decoded.gains_revision, Some(u32::MAX));
        assert_eq!(decoded.ambient_mode.as_deref(), Some("auto"));
        assert_eq!(decoded.ambient_state.as_deref(), Some("unavailable"));
        assert_eq!(decoded.locked, Some(true));
    }

    #[test]
    fn a_short_status_is_encoded_whole() {
        let status = playing_at(1);
        assert_eq!(encode_capped(&status), status.encode_to_vec());
    }

    #[test]
    fn the_feed_sends_first_then_only_what_is_due_and_always_reads_latest() {
        let start = Instant::now();
        let mut feed = StatusFeed::default();
        assert!(feed.latest().is_empty());

        assert!(feed.offer(start, playing_at(1)).is_some());
        let quiet = start + Duration::from_millis(50);
        assert!(feed.offer(quiet, playing_at(2)).is_none());
        // A read sees the poll that was not pushed.
        assert_eq!(feed.latest(), playing_at(2).encode_to_vec());

        // A push that never went out is offered again.
        let due = start + Duration::from_millis(300);
        assert!(feed.offer(due, playing_at(3)).is_some());
        feed.unsend();
        assert!(feed.offer(due, playing_at(3)).is_some());
    }
}
