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

//! What a phone on Wi-Fi asks of the player beyond the playback calls:
//! `WatchStatus`, the compact status Bluetooth pushes, streamed over gRPC,
//! and `GetNetworkInfo`, where the player is on the network.

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::{stream::BoxStream, StreamExt};
use tokio::sync::{watch, Notify};
use tonic::Status;

use crate::controller::remote_status::{poll, should_send_every};
use crate::player::Player;
use crate::proto::player::v1::{GetNetworkInfoResponse, RemoteStatus};

pub(super) type StatusStream = BoxStream<'static, Result<RemoteStatus, Status>>;

/// How often a status goes out while a song plays and nothing else
/// changes: enough for a client to keep its clock true.
pub(super) const PLAYING_INTERVAL: Duration = Duration::from_secs(1);

/// Ends the task that polls the player for a status stream when the stream
/// is dropped, as it is when the client goes away.
struct EndsPoll(tokio::task::JoinHandle<()>);

impl Drop for EndsPoll {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// The status stream: the player's status at once, then whenever it
/// changes, and every [`PLAYING_INTERVAL`] while a song plays.
///
/// The shared poll reads the player every quarter second (and its ambient
/// light twice a second) for this stream alone, and keeps only the latest
/// status that is due in a watch channel. A client that reads slowly skips
/// the statuses between and gets the newest, and never holds the poll up.
pub(super) fn watch(player: Arc<Player>) -> StatusStream {
    watch_every(player, PLAYING_INTERVAL)
}

fn watch_every(player: Arc<Player>, interval: Duration) -> StatusStream {
    let (latest, statuses) = watch::channel(None::<RemoteStatus>);
    let sent: parking_lot::Mutex<Option<(RemoteStatus, Instant)>> = parking_lot::Mutex::new(None);
    let task = tokio::spawn(async move {
        poll(player, Arc::new(Notify::new()), move |current| {
            let now = Instant::now();
            let mut sent = sent.lock();
            let due = match &*sent {
                None => true,
                Some((last, at)) => should_send_every(last, *at, now, &current, interval),
            };
            if due {
                *sent = Some((current.clone(), now));
                latest.send_replace(Some(current));
            }
        })
        .await;
    });
    futures_util::stream::unfold((statuses, EndsPoll(task)), |(mut statuses, ends)| async {
        // An error is the poll gone, which only an abort does.
        statuses.changed().await.ok()?;
        let status = statuses.borrow_and_update().clone()?;
        Some((Ok(status), (statuses, ends)))
    })
    .boxed()
}

/// Where the player is on the network: its host name, its addresses and
/// the port gRPC listens on.
pub(super) fn network_info(player: &Player) -> GetNetworkInfoResponse {
    GetNetworkInfoResponse {
        hostname: crate::config::resolve_hostname(),
        addresses: addresses(),
        grpc_port: player.grpc_port().map(u32::from),
    }
}

/// The addresses of this machine's interfaces a phone could reach it on,
/// as text, without duplicates: IPv4 and IPv6, with loopback and
/// link-local addresses (which mean nothing without an interface) left out.
fn addresses() -> Vec<String> {
    let found = if_addrs::get_if_addrs().unwrap_or_else(|e| {
        // Said once: a phone asks again every few seconds. The usual cause
        // is a service unit written before this call existed, whose sandbox has no
        // AF_NETLINK (`mtrack systemd` prints the current unit).
        static WARNED: std::sync::Once = std::sync::Once::new();
        WARNED.call_once(|| {
            tracing::warn!(
                error = %e,
                "Could not read this machine's network addresses, so a phone on \
                 Bluetooth will not be told where to find the player on Wi-Fi. If \
                 this runs under systemd, its unit may need AF_NETLINK in \
                 RestrictAddressFamilies."
            );
        });
        Vec::new()
    });
    reachable(found.iter().map(|interface| interface.ip()))
}

/// The addresses of `ips` worth telling a phone, as text, IPv4 first.
fn reachable(ips: impl Iterator<Item = std::net::IpAddr>) -> Vec<String> {
    use std::net::IpAddr;
    let mut v4 = Vec::new();
    let mut v6 = Vec::new();
    for ip in ips {
        if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
            continue;
        }
        match ip {
            IpAddr::V4(ip) if !ip.is_link_local() => v4.push(ip.to_string()),
            // fe80::/10 is link-local.
            IpAddr::V6(ip) if (ip.segments()[0] & 0xffc0) != 0xfe80 => v6.push(ip.to_string()),
            _ => {}
        }
    }
    v4.sort();
    v4.dedup();
    v6.sort();
    v6.dedup();
    v4.extend(v6);
    v4
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::StreamExt;

    fn ips(list: &[&str]) -> Vec<std::net::IpAddr> {
        list.iter().map(|ip| ip.parse().unwrap()).collect()
    }

    #[test]
    fn only_addresses_a_phone_can_use_are_listed() {
        let found = reachable(
            ips(&[
                "127.0.0.1",
                "192.168.1.20",
                "169.254.3.4",
                "::1",
                "fe80::1",
                "fd00::5",
                "2001:db8::7",
                "0.0.0.0",
                "224.0.0.1",
                "10.0.0.2",
                "192.168.1.20",
            ])
            .into_iter(),
        );
        assert_eq!(
            found,
            ["10.0.0.2", "192.168.1.20", "2001:db8::7", "fd00::5"]
        );
    }

    #[test]
    fn the_network_info_names_the_grpc_port_and_none_when_it_does_not() {
        let player = crate::testutil::player::memory_player(&["Song A"]).build();
        assert_eq!(network_info(&player).grpc_port, None);
        player.set_grpc_port(Some(50051));
        let info = network_info(&player);
        assert_eq!(info.grpc_port, Some(50051));
        assert_eq!(info.hostname, crate::config::resolve_hostname());
        assert!(info.addresses.iter().all(|a| !a.starts_with("127.")));
        player.set_grpc_port(None);
        assert_eq!(network_info(&player).grpc_port, None);
    }

    /// A player on one 30 second song, with a mock audio device.
    async fn long_song_player() -> (tempfile::TempDir, Arc<Player>) {
        use crate::config;
        use crate::songs;

        let dir = tempfile::tempdir().expect("dir");
        let song_dir = dir.path().join("songs").join("Long");
        std::fs::create_dir_all(&song_dir).expect("song dir");
        crate::testutil::write_wav(song_dir.join("t.wav"), vec![vec![0_i32; 8000 * 30]], 8000)
            .expect("wav");
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: Long\ntracks:\n  - name: t\n    file: t.wav\n",
        )
        .expect("song");
        let songs = songs::get_all_songs(&dir.path().join("songs")).expect("songs");
        let playlists = [(
            "all_songs".to_string(),
            crate::playlist::from_songs(songs).expect("playlist"),
        )]
        .into_iter()
        .collect();
        let player = Player::new(
            playlists,
            "all_songs".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                None,
                Default::default(),
                dir.path().join("songs").to_str().expect("utf-8"),
            ),
            None,
        )
        .expect("player");
        player.await_hardware_ready().await;
        (dir, player)
    }

    async fn next(stream: &mut StatusStream) -> RemoteStatus {
        tokio::time::timeout(crate::testutil::PATIENCE, stream.next())
            .await
            .expect("the stream said nothing")
            .expect("the stream ended")
            .expect("the stream failed")
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_first_status_comes_at_once_and_a_change_brings_another() {
        let player = crate::testutil::player::assets_player()
            .build()
            .expect("player");
        player.await_hardware_ready().await;
        let mut stream = watch(player.clone());
        let first = next(&mut stream).await;
        assert_eq!(first.locked, Some(true));
        assert!(!first.playing);

        player.set_locked(false);
        loop {
            if next(&mut stream).await.locked == Some(false) {
                break;
            }
        }
    }

    /// While a song plays and nothing else changes, a status still comes
    /// every interval, for the client's clock.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_playing_song_is_reported_every_second() {
        let (_dir, player) = long_song_player().await;
        player.play().await.expect("play");
        let mut stream = watch(player.clone());
        // The first status may predate the song's start; wait for one that
        // says it plays, then time the next two.
        let mut first = next(&mut stream).await;
        while !first.playing {
            first = next(&mut stream).await;
        }
        let started = Instant::now();
        let second = next(&mut stream).await;
        let waited = started.elapsed();
        assert!(second.playing);
        assert!(
            waited >= Duration::from_millis(700) && waited <= Duration::from_millis(1600),
            "the next status came after {waited:?}"
        );
        assert!(second.elapsed != first.elapsed, "the clock did not move");
        let _ = player.stop().await;
    }

    /// A client that reads late is told the newest status, not a queue of
    /// the ones it missed.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_slow_client_skips_to_the_latest_status() {
        let player = crate::testutil::player::assets_player()
            .build()
            .expect("player");
        player.await_hardware_ready().await;
        let mut stream = watch(player.clone());
        next(&mut stream).await;
        player.set_locked(false);
        tokio::time::sleep(Duration::from_millis(600)).await;
        player.set_locked(true);
        player.set_locked(false);
        tokio::time::sleep(Duration::from_millis(600)).await;
        // Everything between went to the one slot: what is read now is the
        // present, unlocked, and nothing older follows it.
        let now = next(&mut stream).await;
        assert_eq!(now.locked, Some(false));
        let more = tokio::time::timeout(Duration::from_millis(300), stream.next()).await;
        assert!(more.is_err(), "an older status was queued: {more:?}");
    }

    /// The player's count of strong references with only its own tasks
    /// holding it: the transport poller holds one while it refreshes, every
    /// 200 ms, so the least seen over a little more than one of its periods.
    async fn resting_count(player: &Arc<Player>) -> usize {
        let until = tokio::time::Instant::now() + Duration::from_millis(500);
        let mut least = Arc::strong_count(player);
        while tokio::time::Instant::now() < until {
            tokio::time::sleep(Duration::from_millis(5)).await;
            least = least.min(Arc::strong_count(player));
        }
        least
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dropping_the_stream_ends_the_poll() {
        let player = crate::testutil::player::assets_player()
            .build()
            .expect("player");
        player.await_hardware_ready().await;
        // The init round holds a clone for a moment after the hardware is
        // ready, and the transport poller one while it refreshes: counted
        // into the baseline, either let the count fall below it for good.
        crate::testutil::eventually(
            || player.init_round_finished(),
            "the init round never ended",
        );
        let resting = resting_count(&player).await;
        let mut stream = watch(player.clone());
        next(&mut stream).await;
        crate::testutil::eventually_async(
            || async { Arc::strong_count(&player) > resting },
            "the poll never held the player",
        )
        .await;
        drop(stream);
        crate::testutil::eventually_async(
            || async { Arc::strong_count(&player) == resting },
            "the poll outlived its client",
        )
        .await;
    }
}
