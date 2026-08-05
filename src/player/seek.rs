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

use std::error::Error;
use std::time::Duration;

use tracing::{info, warn};

use super::{Player, ReactiveLoopState};

impl Player {
    /// Seeks to an absolute position within the current song.
    ///
    /// While playing, this is restart-based: the audio fades out briefly,
    /// playback is cancelled, all subsystems wind down, and playback restarts
    /// at the target position fully re-synchronized. While stopped, the
    /// position is stored and consumed by the next `play()`.
    ///
    /// Seeking clears any active section loop.
    pub async fn seek_to(&self, position: Duration) -> Result<(), Box<dyn Error>> {
        let mut join = self.join.lock().await;

        let song = self
            .get_playlist()
            .current()
            .ok_or("Cannot seek: playlist is empty")?;
        if position > song.duration() {
            return Err(format!(
                "Cannot seek to {:?}: beyond song duration {:?}",
                position,
                song.duration()
            )
            .into());
        }

        let handles = match join.take() {
            None => {
                // Not playing: remember the position for the next play().
                info!(song = song.name(), position = ?position, "Seek stored as pending start position");
                *self.pending_start.lock() = Some(position);
                return Ok(());
            }
            Some(handles) => handles,
        };

        info!(song = song.name(), position = ?position, "Seeking");

        // Seeking exits any section loop.
        *self.active_section.write() = None;
        *self.reactive_loop_state.write() = ReactiveLoopState::Idle;

        // Fade the audio to avoid a click, then cancel playback. The cleanup
        // task observes the cancellation and takes the StopCancelled path,
        // leaving player state to us.
        self.fade_out_current_audio();
        handles.cancel.cancel();
        {
            let mut play_start_time = self.play_start_time.lock().await;
            *play_start_time = None;
        }

        // Wait for all subsystems to wind down (play_files joins its audio/
        // MIDI/DMX threads before completing) so the restart can't race
        // shutdown of the previous playback.
        if let Err(e) = handles.join.await {
            warn!(err = %e, "Error waiting for playback to wind down during seek");
        }

        // Restart at the target position under the same join lock.
        self.play_from_locked(position, &mut join).await?;
        Ok(())
    }

    /// Seeks to the start of the named section of the current song.
    pub async fn seek_to_section(&self, section_name: &str) -> Result<(), Box<dyn Error>> {
        let song = self
            .get_playlist()
            .current()
            .ok_or("Cannot seek: playlist is empty")?;
        let (start_time, _) = song.resolve_section(section_name).ok_or_else(|| {
            format!(
                "Section '{}' not found or cannot be resolved (missing beat grid?)",
                section_name
            )
        })?;
        self.seek_to(start_time).await
    }

    /// Returns the pending start position set by seeking while stopped, if any.
    pub fn pending_start(&self) -> Option<Duration> {
        *self.pending_start.lock()
    }

    /// Takes (consumes) the pending start position.
    pub(super) fn take_pending_start(&self) -> Option<Duration> {
        self.pending_start.lock().take()
    }

    /// Clears the pending start position. Called when the playlist position
    /// changes so a stale seek doesn't apply to a different song.
    pub(super) fn clear_pending_start(&self) {
        *self.pending_start.lock() = None;
    }
}
