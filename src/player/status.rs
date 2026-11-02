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

use std::{sync::Arc, time::Duration};

use super::{Player, ReactiveLoopState, ReactiveLoopStateKind, SectionBounds, TransportSnapshot};
use crate::{dmx::engine::AmbientReport, playlist::Playlist, songs::Song};

/// How long a status read waits on the DMX engine. The effects loop holds
/// the timeline lock while it renders, and the ambient lock while it starts
/// a show: a status read is the diagnostic of last resort, and must answer
/// rather than hang alongside a loop that has wedged holding either.
pub const STATUS_DMX_READ_TIMEOUT: Duration = Duration::from_millis(500);

/// A read of the DMX engine for status, bounded by
/// [`STATUS_DMX_READ_TIMEOUT`]. A read that timed out or failed says so,
/// rather than passing for "nothing there": that is the one thing a status
/// read of the lights is meant to tell apart.
#[derive(Clone, Debug, PartialEq)]
pub enum DmxRead<T> {
    Read(T),
    TimedOut,
    Failed(String),
}

/// Ambient light's mode and state, with whose settings it plays.
#[derive(Clone, Debug, PartialEq)]
pub struct AmbientStatus {
    pub report: AmbientReport,
    /// Whose settings the show plays, when there is a show.
    pub source: Option<crate::playlist::AmbientSource>,
}

/// Whether the current song's lighting will fire: armed, how many cues it
/// has and has left, and when the next is.
#[derive(Clone, Debug, PartialEq)]
pub struct LightingStatus {
    pub armed: bool,
    pub cues_total: usize,
    pub cues_remaining: usize,
    pub next_cue: Option<Duration>,
}

/// The player's status, read once and serialized by each surface: the web
/// UI's playback message, MCP's `status`, and gRPC's `Status`. Built four
/// ways, they drifted, and only MCP bounded its read of the DMX engine.
#[derive(Clone)]
pub struct PlayerStatus {
    /// The active playlist.
    pub playlist: Arc<Playlist>,
    /// The playlist's current song.
    pub current_song: Option<Arc<Song>>,
    pub playing: bool,
    /// The position in the song: on the playback clock while playing, the
    /// position kept for the next play while stopped. Every loop jump has
    /// already put it back as it fell due.
    pub elapsed: Result<Option<Duration>, String>,
    /// The position the next play starts at, while one is kept.
    pub pending_start: Option<Duration>,
    pub active_section: Option<SectionBounds>,
    pub reactive_loop_state: ReactiveLoopState,
    pub locked: bool,
    /// The song an auto play waits to start, and how long until it does.
    pub auto_play: Option<(String, Duration)>,
    /// The looping song ending, and how long until its fade is over.
    pub ending: Option<(String, Duration)>,
    /// A hardware reload a config change asked for, waiting for the player
    /// to be idle: cleared once it has run.
    pub hardware_reload_pending: bool,
    /// Ambient light, `None` without a DMX engine.
    pub ambient: Option<DmxRead<AmbientStatus>>,
    /// The current song's lighting, `None` without a DMX engine;
    /// `Some(Read(None))` with no show loaded.
    pub lighting: Option<DmxRead<Option<LightingStatus>>>,
}

impl PlayerStatus {
    /// The transport-state view of this status, for change detection.
    pub fn transport(&self) -> TransportSnapshot {
        TransportSnapshot {
            playlist_name: self.playlist.name().to_string(),
            current_song: self.current_song.as_ref().map(|s| s.name().to_string()),
            playing: self.playing,
            section_loop: self.active_section.as_ref().map(|b| b.name.clone()),
            reactive_loop_state: ReactiveLoopStateKind::from(&self.reactive_loop_state),
            locked: self.locked,
        }
    }
}

/// A read of the DMX engine on the blocking pool, shared by every status
/// asking while it is outstanding, so a wedged lock holds one blocking
/// thread, not one per poll: unbounded, the web UI's poll alone took several
/// a second, and the blocking pool's limit was reached in minutes, after
/// which every `spawn_blocking` in the process queued behind them.
pub(super) struct InflightRead<T: Clone> {
    pending: parking_lot::Mutex<Option<SharedRead<T>>>,
}

type SharedRead<T> =
    futures_util::future::Shared<futures_util::future::BoxFuture<'static, Result<T, String>>>;

impl<T: Clone> Default for InflightRead<T> {
    fn default() -> Self {
        InflightRead {
            pending: parking_lot::Mutex::new(None),
        }
    }
}

impl<T: Clone + Send + Sync + 'static> InflightRead<T> {
    /// Waits up to `timeout` for the read outstanding, or for `read`, run
    /// on the blocking pool, when none is.
    pub(super) async fn read(
        &self,
        read: impl FnOnce() -> T + Send + 'static,
        timeout: Duration,
    ) -> DmxRead<T> {
        use futures_util::FutureExt;
        let shared = {
            let mut pending = self.pending.lock();
            match pending.as_ref() {
                Some(outstanding) if outstanding.peek().is_none() => outstanding.clone(),
                _ => {
                    let started = tokio::task::spawn_blocking(read)
                        .map(|done| done.map_err(|e| e.to_string()))
                        .boxed()
                        .shared();
                    *pending = Some(started.clone());
                    started
                }
            }
        };
        match tokio::time::timeout(timeout, shared).await {
            Ok(Ok(value)) => DmxRead::Read(value),
            Ok(Err(e)) => DmxRead::Failed(e),
            Err(_) => DmxRead::TimedOut,
        }
    }
}

impl Player {
    /// The player's status, every surface's: see [`PlayerStatus`]. The DMX
    /// engine is read on the blocking pool, each read bounded by
    /// [`STATUS_DMX_READ_TIMEOUT`].
    ///
    /// `is_playing()` briefly acquires the play-join lock, so this should
    /// not be called from contexts that hold that lock.
    pub async fn status_snapshot(&self) -> PlayerStatus {
        self.status_with_timeout(STATUS_DMX_READ_TIMEOUT).await
    }

    /// The status the playback surfaces show (the web UI's playback updates,
    /// gRPC's `Status`): [`Player::status_snapshot`] reading only the
    /// ambient light of the DMX engine, not the song's lighting, which they
    /// do not show.
    pub async fn playback_status(&self) -> PlayerStatus {
        let mut status = self.transport_status().await;
        status.ambient = self.ambient_status_within(STATUS_DMX_READ_TIMEOUT).await;
        status
    }

    /// [`Player::status_snapshot`] with the DMX reads bounded by `timeout`.
    pub(super) async fn status_with_timeout(&self, timeout: Duration) -> PlayerStatus {
        let mut status = self.transport_status().await;
        let (ambient, lighting) = tokio::join!(
            self.ambient_status_within(timeout),
            self.lighting_status_within(timeout)
        );
        status.ambient = ambient;
        status.lighting = lighting;
        status
    }

    /// Ambient light's mode and state, as the status reads it: `None`
    /// without a DMX engine, the read bounded by
    /// [`STATUS_DMX_READ_TIMEOUT`].
    pub async fn ambient_status(&self) -> Option<DmxRead<AmbientStatus>> {
        self.ambient_status_within(STATUS_DMX_READ_TIMEOUT).await
    }

    async fn ambient_status_within(&self, timeout: Duration) -> Option<DmxRead<AmbientStatus>> {
        let dmx = self.dmx_engine()?;
        let read = self
            .ambient_reads
            .read(move || dmx.ambient_report(), timeout)
            .await;
        let source = self.ambient_source();
        Some(match read {
            DmxRead::Read(report) => DmxRead::Read(AmbientStatus {
                source: report.show.as_ref().and(source),
                report,
            }),
            DmxRead::TimedOut => DmxRead::TimedOut,
            DmxRead::Failed(e) => DmxRead::Failed(e),
        })
    }

    async fn lighting_status_within(
        &self,
        timeout: Duration,
    ) -> Option<DmxRead<Option<LightingStatus>>> {
        let dmx = self.dmx_engine()?;
        Some(
            self.lighting_reads
                .read(
                    move || {
                        dmx.lighting_arming_state().map(
                            |(armed, cues_total, cues_remaining, next_cue)| LightingStatus {
                                armed,
                                cues_total,
                                cues_remaining,
                                next_cue,
                            },
                        )
                    },
                    timeout,
                )
                .await,
        )
    }

    /// The status without the DMX engine's reads: what the transport
    /// snapshot is made from, at its poll rate.
    pub(super) async fn transport_status(&self) -> PlayerStatus {
        let playlist = self.get_playlist();
        let current_song = playlist.current();
        let playing = self.is_playing().await;
        let elapsed = self.elapsed().await.map_err(|e| e.to_string());
        let (pending_start, auto_play, ending) = {
            let transport = self.transport.lock();
            (
                transport.pending_start.map(|p| p.position),
                transport.auto_play_status(),
                transport.ending_status(),
            )
        };
        PlayerStatus {
            playlist,
            current_song,
            playing,
            elapsed,
            pending_start,
            active_section: self.active_section(),
            reactive_loop_state: self.reactive_loop_state(),
            locked: self.is_locked(),
            auto_play,
            ending,
            hardware_reload_pending: self
                .hardware_reload_pending
                .load(std::sync::atomic::Ordering::Relaxed),
            ambient: None,
            lighting: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DMX read that does not come back in time is reported as timed
    /// out, not waited on, and every read asked while it is outstanding
    /// waits on it rather than starting another: with the lock it reads
    /// held, the blocking threads stay at one however often status polls.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_dmx_read_that_hangs_times_out_on_one_thread() {
        let reads = InflightRead::<()>::default();
        // The lock the read waits for, held until `released` is set.
        let released = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let begun = std::time::Instant::now();
        for _ in 0..50 {
            let (released, started) = (released.clone(), started.clone());
            let read = reads
                .read(
                    move || {
                        started.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        while !released.load(std::sync::atomic::Ordering::SeqCst) {
                            std::thread::sleep(Duration::from_millis(1));
                        }
                    },
                    Duration::from_millis(5),
                )
                .await;
            assert!(matches!(read, DmxRead::TimedOut));
        }
        assert!(begun.elapsed() < Duration::from_secs(5));
        assert_eq!(
            started.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a read per poll"
        );
        released.store(true, std::sync::atomic::Ordering::SeqCst);
        // Once it is over, the next read is a fresh one.
        assert_eq!(
            reads.read(|| (), Duration::from_millis(500)).await,
            DmxRead::Read(())
        );
        assert_eq!(
            reads.read(|| (), Duration::from_millis(500)).await,
            DmxRead::Read(())
        );
    }

    /// A read that panics is reported as failed, not as nothing there.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_dmx_read_that_panics_fails() {
        let reads = InflightRead::<()>::default();
        let read = reads
            .read(|| panic!("wedged"), Duration::from_millis(500))
            .await;
        assert!(matches!(read, DmxRead::Failed(_)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_dmx_read_that_answers_is_read() {
        let reads = InflightRead::<i32>::default();
        assert_eq!(
            reads.read(|| 7, Duration::from_millis(500)).await,
            DmxRead::Read(7)
        );
        assert_eq!(
            reads.read(|| 8, Duration::from_millis(500)).await,
            DmxRead::Read(8),
            "a read done is not answered again"
        );
    }
}
