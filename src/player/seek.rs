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
use std::sync::atomic::Ordering;
use std::time::Duration;

use tracing::{info, warn};

use super::{Player, ReactiveLoopState};
use crate::songs::Song;

/// How long a seek waits for the previous playback to wind down before
/// abandoning it. Normal wind-down is well under a second; this only fires
/// when a playback thread failed to observe cancellation.
const SEEK_WINDDOWN_TIMEOUT: Duration = Duration::from_secs(10);

impl Player {
    /// Pauses playback and stores the exact song position for the next play.
    ///
    /// The playback engine has stop and position-based start primitives rather
    /// than a native paused state. Capture elapsed first, stop all synchronized
    /// subsystems, then persist that position through the normal stopped seek
    /// path so the next `play()` resumes from it.
    pub async fn pause(&self) -> Result<Option<Duration>, Box<dyn Error>> {
        let Some(position) = self.elapsed().await? else {
            return Ok(None);
        };
        // The stop and the position kept for the next play are one step
        // under the join lock, and the song counts as playing throughout,
        // even one already paused, whose position the stop clears before
        // it is kept again. A playlist edit between them would find neither
        // playing nor paused, and give an off-list song's place to the
        // entry after it, which play would then start at this song's
        // position (#576).
        let mut join = self.join.lock().await;
        self.song_playing.store(true, Ordering::Relaxed);
        self.stop_for_pause(&mut join).await;
        let position = self.keep_paused_position(position);
        self.song_playing.store(false, Ordering::Relaxed);
        Ok(Some(position))
    }

    /// Keeps `position` in the current song for the next play to resume
    /// from, and returns it as a position in the pass it is on.
    fn keep_paused_position(&self, position: Duration) -> Duration {
        let Some(song) = self.get_playlist().current() else {
            return position;
        };
        // A looping song's elapsed runs on over its passes: where it is in
        // the pass it is on is what to resume from.
        let position = song.position_in_pass(position);
        // Paused on the song's very end: there is nothing left to resume,
        // so the next play starts it over.
        if check_start_position(&song, position, "").is_err() {
            return position;
        }
        info!(song = song.name(), position = ?position, "Paused; play resumes here");
        *self.pending_start.lock() = Some(position);
        position
    }

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
        check_start_position(&song, position, "seek to")?;

        // A looping song ending: stopped a moment from now, and nothing to
        // restart.
        if join.is_some() && self.loop_end.load(Ordering::Relaxed) {
            return Err("Cannot seek: the song is ending".into());
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
        // A seek restarts this song: a break asked of it is called off.
        self.forget_loop_break();

        info!(song = song.name(), position = ?position, "Seeking");

        // Seeking exits any section loop.
        *self.active_section.write() = None;
        *self.reactive_loop_state.write() = ReactiveLoopState::Idle;

        // Fade the audio to avoid a click, then cancel playback. The cleanup
        // task observes the cancellation and takes the StopCancelled path,
        // leaving player state to us.
        // Cancel while holding the play_start_time lock: a playback still in
        // startup stamps play_start_time under that lock and skips the stamp
        // when cancelled, so this ordering keeps it from writing the pre-seek
        // position back after we clear it.
        self.fade_out_current_audio();
        self.cut_break_fade();
        {
            let mut play_start_time = self.play_start_time.lock().await;
            handles.cancel.cancel();
            *play_start_time = None;
        }

        // Wait for all subsystems to wind down (play_files joins its audio/
        // MIDI/DMX threads before completing) so the restart can't race
        // shutdown of the previous playback. The wait is bounded: this runs
        // with the join lock held, and an unbounded await here would wedge
        // every control call (play/stop/next/is_playing and the web UI
        // poller) behind a playback thread that failed to observe
        // cancellation. On timeout the handle is dropped — detaching the
        // wind-down like stop() does — and the seek reports failure rather
        // than restarting on top of a device that may still be held.
        match tokio::time::timeout(SEEK_WINDDOWN_TIMEOUT, handles.join).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => {
                warn!(err = %e, "Error waiting for playback to wind down during seek");
            }
            Err(_) => {
                // The song a seek takes out of `join` counts as playing
                // until it restarts; abandoned, it is not.
                self.song_playing.store(false, Ordering::Relaxed);
                warn!(
                    timeout = ?SEEK_WINDDOWN_TIMEOUT,
                    "Playback did not wind down in time during seek; abandoning it"
                );
                return Err(format!(
                    "Seek aborted: previous playback did not wind down within {:?}. \
                     Stop and play to recover.",
                    SEEK_WINDDOWN_TIMEOUT
                )
                .into());
            }
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

/// Turns down a start position at or past the song's end: audio has nothing
/// to play there, so playback would fail and the playlist skip the song
/// (#584). A song with no length to go by -- no audio and no DSL show, a MIDI
/// file or a MIDI DMX show alone -- takes any start, as it always has.
pub(super) fn check_start_position(
    song: &Song,
    position: Duration,
    action: &str,
) -> Result<(), String> {
    let length = song.length();
    if !length.is_zero() && position >= length {
        return Err(format!(
            "Cannot {} {:?}: at or beyond song length {:?}",
            action, position, length
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_song_with_no_length_takes_any_start() {
        // A song with no audio and no DSL show has no length to go by.
        let song = Song::default();
        assert!(check_start_position(&song, Duration::from_secs(30), "play from").is_ok());
    }
}
