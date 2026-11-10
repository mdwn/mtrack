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

//! The status the phone watches: built from the player's one transport
//! status, cut down to a few hundred bytes, and pushed only when it says
//! something new.

use std::time::{Duration, Instant};

use prost::Message;

use crate::player::PlayerStatus;
use crate::proto::ble::v1::Status;

use super::super::grpc::{auto_play_to_proto, elapsed_to_proto};

/// The most bytes an encoded [`Status`] takes. A read of the status
/// characteristic returns it bare, in one attribute value, and a long
/// playlist or song name must not grow it past what an attribute holds.
pub(crate) const MAX_STATUS_LEN: usize = 512;

/// How often a status goes out while a song plays.
pub(crate) const PLAYING_INTERVAL: Duration = Duration::from_millis(250);

/// How early a poll may fall against [`PLAYING_INTERVAL`] and still count
/// as due. The poll ticks at that interval, and without the allowance one
/// tick landing a few milliseconds earlier than the last would skip a
/// whole cycle and halve the rate.
const POLL_JITTER: Duration = Duration::from_millis(25);

/// The status a [`PlayerStatus`] reports to a phone. The conversions are
/// gRPC's own, so both surfaces read the player's state the same way.
#[allow(clippy::result_large_err)]
pub(crate) fn from_player(
    status: &PlayerStatus,
    gains_revision: u32,
    ambient: Option<&Ambient>,
    locked: bool,
) -> Result<Status, tonic::Status> {
    let song = status.current_song.as_ref();
    Ok(Status {
        playlist_name: status.playlist.name().to_string(),
        song_name: song.map(|s| s.name().to_string()).unwrap_or_default(),
        song_duration: song.and_then(|s| prost_types::Duration::try_from(s.length()).ok()),
        playing: status.playing,
        elapsed: elapsed_to_proto(status.elapsed.clone())?,
        position: status.playlist.entry().map(|index| index as u32),
        auto_play: auto_play_to_proto(status.auto_play.clone())?,
        loop_section: status.active_section.as_ref().map(|s| s.name.clone()),
        gains_revision: Some(gains_revision),
        ambient_mode: ambient.map(|a| a.mode.clone()),
        ambient_state: ambient.map(|a| a.state.clone()),
        locked: Some(locked),
    })
}

/// The ambient light as last read, in the words gRPC's `AmbientStatus` uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Ambient {
    pub(crate) mode: String,
    pub(crate) state: String,
}

/// How often the ambient light is read. Its mode changes by hand and its
/// state at the pace of a fade, so twice a second loses nothing.
const AMBIENT_INTERVAL: Duration = Duration::from_millis(500);

/// Reads the ambient light every [`AMBIENT_INTERVAL`] into `latest`, and
/// tells `wanted` when it changed. This is apart from the status poll
/// because the read is behind the DMX engine's ambient lock, which the
/// effects loop holds while it starts a show: the read can wait as long as
/// [`crate::player::STATUS_DMX_READ_TIMEOUT`], and a Stop must not be told
/// late behind it. A read that does not answer keeps the last one.
async fn poll_ambient(
    player: &crate::player::Player,
    latest: &parking_lot::Mutex<Option<Ambient>>,
    wanted: &tokio::sync::Notify,
) {
    let mut ticks = tokio::time::interval(AMBIENT_INTERVAL);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        let read = match player.ambient_status().await {
            // No DMX engine: there is no ambient light to speak of.
            None => None,
            Some(crate::player::DmxRead::Read(ambient)) => Some(Ambient {
                mode: ambient.report.mode.as_str().to_string(),
                state: ambient.report.state.to_string(),
            }),
            Some(_) => continue,
        };
        let mut latest = latest.lock();
        if *latest != read {
            *latest = read;
            wanted.notify_one();
        }
    }
}

/// Whether `current` is worth sending to a phone that last had `sent`, at
/// `sent_at`: anything but the elapsed time changed, or a song is playing
/// and a quarter second has passed, or the position moved while stopped (a
/// seek, which nothing else would announce).
pub(crate) fn should_send(sent: &Status, sent_at: Instant, now: Instant, current: &Status) -> bool {
    let mut same_but_elapsed = current.clone();
    same_but_elapsed.elapsed = sent.elapsed;
    if same_but_elapsed != *sent {
        return true;
    }
    if current.playing {
        return now.saturating_duration_since(sent_at) + POLL_JITTER >= PLAYING_INTERVAL;
    }
    current.elapsed != sent.elapsed
}

/// `status` encoded, in at most [`MAX_STATUS_LEN`] bytes: when it is longer,
/// the longest name loses bytes (at a character boundary) until it fits.
/// The rest of a status is a handful of numbers, so it always does.
pub(crate) fn encode_capped(status: &Status) -> Vec<u8> {
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
    sent: Option<(Status, Instant)>,
    /// The latest status polled, encoded: what a read returns.
    latest: Vec<u8>,
}

impl StatusFeed {
    /// Takes the latest poll. The status to push, encoded, when it is due.
    pub(crate) fn offer(&mut self, now: Instant, current: Status) -> Option<Vec<u8>> {
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

/// Polls the player every [`PLAYING_INTERVAL`], and at once whenever `wanted`
/// is told (a phone's request was just answered), and hands each status to
/// `publish`. A status that cannot be read is skipped: the phone keeps the
/// last one.
pub(crate) async fn poll(
    player: std::sync::Arc<crate::player::Player>,
    wanted: std::sync::Arc<tokio::sync::Notify>,
    publish: impl Fn(Status),
) {
    let ambient = parking_lot::Mutex::new(None);
    let status = async {
        let mut ticks = tokio::time::interval(PLAYING_INTERVAL);
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = ticks.tick() => {}
                () = wanted.notified() => {}
            }
            // No DMX reads here: the ambient light is whatever
            // `poll_ambient` last read.
            let gains_revision = player.gains_revision();
            let transport = player.transport_status().await;
            let ambient = ambient.lock().clone();
            match from_player(
                &transport,
                gains_revision,
                ambient.as_ref(),
                player.is_locked(),
            ) {
                Ok(status) => publish(status),
                Err(e) => tracing::debug!(error = %e, "Bluetooth status not read"),
            }
        }
    };
    tokio::join!(status, poll_ambient(&player, &ambient, &wanted));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secs(s: i64) -> Option<prost_types::Duration> {
        Some(prost_types::Duration {
            seconds: s,
            nanos: 0,
        })
    }

    fn playing_at(elapsed: i64) -> Status {
        Status {
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
    #[tokio::test(flavor = "multi_thread")]
    async fn the_poll_reports_the_ambient_light() {
        use crate::config;

        // A DMX engine with no ambient show: on has none to play.
        let player = crate::testutil::player::assets_player()
            .dmx(Some(config::Dmx::new(
                None,
                None,
                Some(9090),
                vec![config::Universe::new(1, "test".to_string())],
                None,
            )))
            .ready()
            .await
            .expect("player");
        let (seen, mut statuses) = tokio::sync::mpsc::unbounded_channel();
        let poll = tokio::spawn(poll(
            player.clone(),
            std::sync::Arc::new(tokio::sync::Notify::new()),
            move |status| {
                let _ = seen.send(status);
            },
        ));
        let mut wait_for = async |mode: &str, state: &str| {
            let found = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let status = statuses.recv().await.expect("the poll ended");
                    if status.ambient_mode.as_deref() == Some(mode)
                        && status.ambient_state.as_deref() == Some(state)
                    {
                        return;
                    }
                }
            })
            .await;
            assert!(found.is_ok(), "no status said {mode}/{state}");
        };
        wait_for("auto", "off").await;

        let setter = player.clone();
        tokio::task::spawn_blocking(move || {
            setter.set_ambient_mode(crate::dmx::engine::AmbientMode::On)
        })
        .await
        .expect("set");
        wait_for("on", "unavailable").await;
        poll.abort();
    }

    /// A phone is told of the lock, and of a change to it made over its own
    /// link, at the next poll after the answer.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_poll_reports_the_lock_and_a_change_over_the_link() {
        use crate::controller::ble::dispatch::Dispatcher;
        use crate::proto::ble::v1::Request;
        use crate::proto::player::v1::SetLockRequest;

        let player = crate::testutil::player::assets_player()
            .build()
            .expect("player");
        player.await_hardware_ready().await;
        assert!(player.is_locked(), "a player starts locked");
        let wanted = std::sync::Arc::new(tokio::sync::Notify::new());
        let (seen, mut statuses) = tokio::sync::mpsc::unbounded_channel();
        let poll = tokio::spawn(poll(player.clone(), wanted.clone(), move |status| {
            let _ = seen.send(status);
        }));
        let mut wait_for = async |locked: bool| {
            let found = tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    let status = statuses.recv().await.expect("the poll ended");
                    if status.locked == Some(locked) {
                        return;
                    }
                }
            })
            .await;
            assert!(found.is_ok(), "no status said locked={locked}");
        };
        wait_for(true).await;

        let frame = Request {
            id: 1,
            method: "SetLock".to_string(),
            payload: SetLockRequest { locked: false }.encode_to_vec(),
        }
        .encode_to_vec();
        Dispatcher::new(player.clone()).handle(&frame).await;
        // The link nudges the poll after each answer.
        wanted.notify_one();
        wait_for(false).await;
        poll.abort();
    }

    /// A phone's status is what gRPC's `Status` says of the same player.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_phones_status_is_what_grpc_reports() {
        use crate::proto::player::v1::player_service_server::PlayerService;
        use crate::proto::player::v1::StatusRequest;

        let player = crate::testutil::player::assets_player()
            .build()
            .expect("player");
        player.await_hardware_ready().await;
        let mine = from_player(&player.transport_status().await, 0, None, true).expect("a status");
        let grpc = crate::controller::grpc::PlayerServer::new(player.clone())
            .status(tonic::Request::new(StatusRequest {}))
            .await
            .expect("status")
            .into_inner();

        assert_eq!(mine.playlist_name, grpc.playlist_name);
        let song = grpc.current_song.expect("a current song");
        assert!(!mine.song_name.is_empty());
        assert_eq!(mine.song_name, song.name);
        assert_eq!(mine.song_duration, song.duration);
        assert_eq!(mine.locked, Some(true));
        assert_eq!(mine.playing, grpc.playing);
        assert_eq!(mine.elapsed, grpc.elapsed);
        assert_eq!(mine.auto_play, grpc.auto_play);
        assert_eq!(
            mine.position,
            player.get_playlist().entry().map(|i| i as u32)
        );
        assert!(mine.position.is_some());
    }

    #[test]
    fn a_playing_status_goes_out_every_quarter_second_and_not_before() {
        let start = Instant::now();
        let sent = playing_at(1);
        let now = playing_at(2);
        assert!(!should_send(
            &sent,
            start,
            start + Duration::from_millis(100),
            &now
        ));
        assert!(should_send(
            &sent,
            start,
            start + Duration::from_millis(250),
            &now
        ));
        // A poll a little early still counts, or the rate would halve.
        assert!(should_send(
            &sent,
            start,
            start + Duration::from_millis(240),
            &now
        ));
    }

    #[test]
    fn any_change_but_elapsed_goes_out_at_once() {
        let start = Instant::now();
        let sent = playing_at(1);
        let soon = start + Duration::from_millis(5);
        for change in [
            |s: &mut Status| s.playing = false,
            |s: &mut Status| s.song_name = "Other".into(),
            |s: &mut Status| s.playlist_name = "Other".into(),
            |s: &mut Status| s.position = Some(2),
            |s: &mut Status| s.position = None,
            |s: &mut Status| s.loop_section = Some("Chorus".into()),
            |s: &mut Status| s.song_duration = secs(1),
            |s: &mut Status| s.gains_revision = Some(1),
            |s: &mut Status| s.ambient_mode = Some("on".into()),
            |s: &mut Status| s.ambient_state = Some("playing".into()),
            |s: &mut Status| s.locked = Some(true),
            |s: &mut Status| {
                s.auto_play = Some(crate::proto::player::v1::AutoPlayStatus {
                    song: "Next".into(),
                    starts_in: None,
                })
            },
        ] {
            let mut current = sent.clone();
            change(&mut current);
            assert!(should_send(&sent, start, soon, &current), "{current:?}");
        }
    }

    #[test]
    fn a_stopped_status_stays_quiet_until_something_changes() {
        let start = Instant::now();
        let mut sent = playing_at(5);
        sent.playing = false;
        let later = start + Duration::from_secs(10);
        assert!(!should_send(&sent, start, later, &sent.clone()));
        // A seek while stopped moves only the position, and is announced.
        let mut seeked = sent.clone();
        seeked.elapsed = secs(30);
        assert!(should_send(&sent, start, start, &seeked));
    }

    #[test]
    fn a_long_name_is_cut_so_the_status_fits_one_attribute() {
        let mut status = playing_at(1);
        status.song_name = "é".repeat(2000);
        status.playlist_name = "p".repeat(100);
        let encoded = encode_capped(&status);
        assert!(encoded.len() <= MAX_STATUS_LEN, "{}", encoded.len());
        let decoded = Status::decode(encoded.as_slice()).expect("a valid status");
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
        let decoded = Status::decode(encoded.as_slice()).expect("a valid status");
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
        let decoded = Status::decode(encoded.as_slice()).expect("a valid status");
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
