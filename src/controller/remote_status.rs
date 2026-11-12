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

//! The status a phone watches, read from the player and cut down to what a
//! remote needs. One reader and one change detector for both carriers:
//! Bluetooth pushes the status it builds, and gRPC's `WatchStatus` streams
//! it.

use std::time::{Duration, Instant};

use crate::player::PlayerStatus;
use crate::proto::player::v1::RemoteStatus;

use super::grpc::{auto_play_to_proto, elapsed_to_proto};

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
) -> Result<RemoteStatus, tonic::Status> {
    let song = status.current_song.as_ref();
    Ok(RemoteStatus {
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
/// `sent_at`, over Bluetooth: [`should_send_every`] at [`PLAYING_INTERVAL`].
pub(crate) fn should_send(
    sent: &RemoteStatus,
    sent_at: Instant,
    now: Instant,
    current: &RemoteStatus,
) -> bool {
    should_send_every(sent, sent_at, now, current, PLAYING_INTERVAL)
}

/// Whether `current` is worth sending to a client that last had `sent`, at
/// `sent_at`: anything but the elapsed time changed, or a song is playing
/// and `interval` has passed, or the position moved while stopped (a seek,
/// which nothing else would announce).
pub(crate) fn should_send_every(
    sent: &RemoteStatus,
    sent_at: Instant,
    now: Instant,
    current: &RemoteStatus,
    interval: Duration,
) -> bool {
    let mut same_but_elapsed = current.clone();
    same_but_elapsed.elapsed = sent.elapsed;
    if same_but_elapsed != *sent {
        return true;
    }
    if current.playing {
        return now.saturating_duration_since(sent_at) + POLL_JITTER >= interval;
    }
    current.elapsed != sent.elapsed
}

/// Polls the player every [`PLAYING_INTERVAL`], and at once whenever `wanted`
/// is told (a phone's request was just answered), and hands each status to
/// `publish`. A status that cannot be read is skipped: the phone keeps the
/// last one.
pub(crate) async fn poll(
    player: std::sync::Arc<crate::player::Player>,
    wanted: std::sync::Arc<tokio::sync::Notify>,
    publish: impl Fn(RemoteStatus),
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
                Err(e) => tracing::debug!(error = %e, "status not read"),
            }
        }
    };
    tokio::join!(status, poll_ambient(&player, &ambient, &wanted));
}

#[cfg(test)]
mod tests {
    use super::*;
    use prost::Message;

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
            cancel: false,
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
            |s: &mut RemoteStatus| s.playing = false,
            |s: &mut RemoteStatus| s.song_name = "Other".into(),
            |s: &mut RemoteStatus| s.playlist_name = "Other".into(),
            |s: &mut RemoteStatus| s.position = Some(2),
            |s: &mut RemoteStatus| s.position = None,
            |s: &mut RemoteStatus| s.loop_section = Some("Chorus".into()),
            |s: &mut RemoteStatus| s.song_duration = secs(1),
            |s: &mut RemoteStatus| s.gains_revision = Some(1),
            |s: &mut RemoteStatus| s.ambient_mode = Some("on".into()),
            |s: &mut RemoteStatus| s.ambient_state = Some("playing".into()),
            |s: &mut RemoteStatus| s.locked = Some(true),
            |s: &mut RemoteStatus| {
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
}
