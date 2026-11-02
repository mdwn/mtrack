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

/// Runs `read` on the blocking pool, bounded by [`STATUS_DMX_READ_TIMEOUT`].
async fn bounded_read<T: Send + 'static>(
    read: impl FnOnce() -> T + Send + 'static,
    timeout: Duration,
) -> DmxRead<T> {
    match tokio::time::timeout(timeout, tokio::task::spawn_blocking(read)).await {
        Ok(Ok(value)) => DmxRead::Read(value),
        Ok(Err(e)) => DmxRead::Failed(e.to_string()),
        Err(_) => DmxRead::TimedOut,
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
        let read = bounded_read(move || dmx.ambient_report(), timeout).await;
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
            bounded_read(
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
            ambient: None,
            lighting: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A DMX read that does not come back in time is reported as timed
    /// out, not waited on: a status read must answer while the effects loop
    /// holds the lock it reads.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_dmx_read_that_hangs_times_out() {
        let release = Arc::new(std::sync::Barrier::new(2));
        let held = release.clone();
        let started = std::time::Instant::now();
        let read = bounded_read(move || held.wait(), Duration::from_millis(50)).await;
        assert!(matches!(read, DmxRead::TimedOut));
        assert!(started.elapsed() < Duration::from_secs(1));
        release.wait();
    }

    /// A read that panics is reported as failed, not as nothing there.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_dmx_read_that_panics_fails() {
        let read: DmxRead<()> = bounded_read(|| panic!("wedged"), Duration::from_millis(500)).await;
        assert!(matches!(read, DmxRead::Failed(_)));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_dmx_read_that_answers_is_read() {
        assert_eq!(
            bounded_read(|| 7, Duration::from_millis(500)).await,
            DmxRead::Read(7)
        );
    }
}
