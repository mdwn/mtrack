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
use std::{
    error::Error,
    sync::{atomic::Ordering, Arc},
    thread,
    time::{Duration, SystemTime},
};
use tokio::sync::oneshot;
use tracing::{error, info};

use crate::{dmx, playsync::CancelHandle, songs::Song};

use super::{
    decide_cleanup_action, resolve_playback_outcome, CleanupAction, PlayHandles, PlaybackContext,
    PlaybackResult, Player, ReactiveLoopState,
};

impl Player {
    /// Plays the song at the current position. Returns the song if playback started successfully.
    /// Returns None if a song is already playing.
    /// Returns an error if lighting show validation fails.
    pub async fn play(&self) -> Result<Option<Arc<Song>>, Box<dyn Error>> {
        self.play_from(Duration::ZERO).await
    }

    /// Plays the song starting from a specific time position.
    /// Returns the song if playback started successfully.
    /// Returns None if a song is already playing.
    /// Returns an error if lighting show validation fails.
    pub async fn play_from(
        &self,
        start_time: Duration,
    ) -> Result<Option<Arc<Song>>, Box<dyn Error>> {
        // Reject playback if hardware hasn't finished initializing.
        if !*self.init_done_tx.borrow() {
            return Err("Hardware is still initializing".into());
        }

        let mut join = self.join.lock().await;
        if join.is_some() {
            // If the current song is looping, break out immediately with crossfade.
            // Fade out current audio, then cancel so subsystems exit. The cleanup
            // task sees loop_broken and auto-plays the next song.
            if self.is_current_song_looping() {
                info!("Breaking out of song loop to advance playlist.");
                self.fade_out_current_audio();
                self.loop_break.store(true, Ordering::Relaxed);
                if let Some(ref handles) = *join {
                    handles.cancel.cancel();
                }
                return Ok(None);
            }
            info!("Player is already playing a song.");
            return Ok(None);
        }

        self.play_from_locked(start_time, &mut join).await
    }

    /// Inner implementation of play_from that assumes the caller already holds the join lock
    /// and has verified it is `None` (no active playback).
    pub(super) async fn play_from_locked(
        &self,
        start_time: Duration,
        join: &mut Option<PlayHandles>,
    ) -> Result<Option<Arc<Song>>, Box<dyn Error>> {
        let _enter = self.span.enter();

        let playlist = self.get_playlist().clone();
        let song = match playlist.current() {
            Some(song) => song,
            None => {
                info!("Playlist is empty, nothing to play.");
                return Ok(None);
            }
        };

        // Load samples for this song (if not already loaded)
        self.load_song_samples(&song);

        // Load per-song notification audio overrides.
        if let Some(notif_config) = song.notification_audio() {
            let base_path = song.base_path();
            self.notification_engine.set_song_overrides(
                &notif_config.event_overrides(),
                notif_config.section_overrides(),
                base_path,
            );
        } else {
            self.notification_engine.clear_song_overrides();
        }

        // Clone hardware Arcs under a short read lock.
        let hw = self.hardware.read().clone();

        // Validate lighting shows before starting playback
        if let Some(ref dmx_engine) = hw.dmx_engine {
            if let Err(e) = dmx_engine.validate_song_lighting(&song) {
                error!(
                    song = song.name(),
                    err = e.as_ref(),
                    "Lighting show validation failed, preventing song playback"
                );
                return Err(e);
            }
        }

        // Warn about tracks with no mapping in the config.
        if let Some(ref mappings) = hw.mappings {
            crate::verify::warn_unmapped_tracks(&song, mappings);
        }

        let play_start_time = self.play_start_time.clone();

        let cancel_handle = CancelHandle::new();
        let cancel_handle_for_cleanup = cancel_handle.clone();
        let (play_tx, play_rx) = oneshot::channel::<Result<(), String>>();

        // Reset loop time consumed for the new song.
        *self.loop_time_consumed.lock() = Duration::ZERO;

        let join_handle = {
            let ctx = PlaybackContext {
                device: hw.device.clone(),
                mappings: hw.mappings.clone(),
                midi_device: hw.midi_device.clone(),
                dmx_engine: hw.dmx_engine.clone(),
                clock: hw.clock_source.new_clock(),
                song: song.clone(),
                cancel_handle: cancel_handle.clone(),
                play_tx,
                start_time,
                play_start_time: play_start_time.clone(),
                loop_control: crate::playsync::LoopControl {
                    loop_break: self.loop_break.clone(),
                    active_section: self.active_section.clone(),
                    section_loop_break: self.section_loop_break.clone(),
                    loop_time_consumed: self.loop_time_consumed.clone(),
                },
            };
            tokio::task::spawn_blocking(move || {
                Player::play_files(ctx);
            })
        };
        *join = Some(PlayHandles {
            join: join_handle,
            cancel: cancel_handle.clone(),
        });

        // Spawn section boundary polling task for reactive looping.
        {
            let player = self.clone();
            let cancel = cancel_handle.clone();
            // Reset reactive state for new song.
            *self.reactive_loop_state.write() = ReactiveLoopState::Idle;
            tokio::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_millis(50));
                loop {
                    interval.tick().await;
                    if cancel.is_cancelled() {
                        break;
                    }
                    if let Ok(Some(elapsed)) = player.elapsed().await {
                        player.check_section_boundaries(elapsed);
                    }
                }
            });
        }

        {
            let player = self.clone();
            let song = song.clone();
            tokio::spawn(async move {
                let result = match play_rx.await {
                    Ok(Ok(())) => PlaybackResult::Success,
                    Ok(Err(e)) => PlaybackResult::Failed(e),
                    Err(_e) => PlaybackResult::SenderDropped,
                };

                let cancelled = cancel_handle_for_cleanup.is_cancelled();
                let loop_broken = player.loop_break.swap(false, Ordering::Relaxed);

                info!(
                    song = song.name(),
                    cancelled = cancelled,
                    loop_broken = loop_broken,
                    "Song finished playing."
                );

                let action = decide_cleanup_action(result, cancelled, loop_broken);
                if action == CleanupAction::StopCancelled {
                    // stop() already cleared join and play_start_time.
                    // Touching them here would clobber state from a new play() that
                    // may have started after stop() returned.
                    return;
                }

                // Natural finish or loop break: advance playlist and clean up.
                let mut join = player.join.lock().await;
                if let Some(song) = playlist.next() {
                    player.emit_song_change(&song);
                }

                {
                    let mut play_start_time = player.play_start_time.lock().await;
                    *play_start_time = None;
                }

                *join = None;
                player.stop_run.store(false, Ordering::Relaxed);
                let should_auto_play = action == CleanupAction::LoopBreakAndPlay;
                drop(join);

                // If loop was broken (play/next during loop), auto-play the next song.
                // Use spawn_blocking + block_on to avoid the non-Send future issue
                // with tokio::sync::Mutex guards inside play_from.
                if should_auto_play {
                    let player_for_play = player;
                    tokio::task::spawn_blocking(move || {
                        let rt = tokio::runtime::Handle::current();
                        if let Err(e) = rt.block_on(player_for_play.play()) {
                            error!(err = %e, "Failed to auto-play next song after loop break");
                        }
                    });
                }
            });
        }

        Ok(Some(song))
    }

    pub(super) fn play_files(ctx: PlaybackContext) {
        let PlaybackContext {
            device,
            mappings,
            midi_device,
            dmx_engine,
            clock,
            song,
            cancel_handle,
            play_tx,
            start_time,
            play_start_time,
            loop_control,
        } = ctx;

        // Check if any subsystems are active.
        let has_audio = device.is_some() && mappings.is_some();
        let has_midi = song.midi_playback().is_some() && midi_device.is_some();
        let has_dmx = dmx_engine.is_some();

        if !has_audio && !has_midi && !has_dmx {
            info!(
                song = song.name(),
                "No playback subsystems active for this song; completing immediately"
            );
            if play_tx.send(Ok(())).is_err() {
                error!("Error while sending to finish channel (receiver dropped).");
            }
            return;
        }

        // Each subsystem signals readiness on this channel. Once all have
        // reported ready, we start the clock as the "go" signal.
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<()>();
        let mut expected_ready: usize = 0;

        let audio_outcome: Arc<parking_lot::Mutex<Option<Result<(), String>>>> =
            Arc::new(parking_lot::Mutex::new(None));

        let audio_join_handle = if let (Some(device), Some(mappings)) = (device, mappings) {
            let song = song.clone();
            let cancel_handle = cancel_handle.clone();
            let audio_outcome = audio_outcome.clone();
            let ready_tx = crate::playsync::ReadyGuard::new(ready_tx.clone());
            let clock = clock.clone();
            let loop_control = loop_control.clone();
            expected_ready += 1;

            Some(thread::spawn(move || {
                let song_name = song.name().to_string();
                let result = device.play_from(
                    song,
                    &mappings,
                    crate::playsync::PlaybackSync {
                        cancel_handle,
                        ready_tx,
                        clock,
                        start_time,
                        loop_control,
                    },
                );
                if let Err(ref e) = result {
                    error!(
                        err = e.as_ref(),
                        song = song_name,
                        "Error while playing song"
                    );
                }
                let outcome = result.map_err(|e| e.to_string());
                let mut guard = audio_outcome.lock();
                *guard = Some(outcome);
            }))
        } else {
            None
        };

        let dmx_join_handle = dmx_engine.map(|dmx_engine| {
            let dmx_engine = dmx_engine.clone();
            let song = song.clone();
            let cancel_handle = cancel_handle.clone();
            let clock = clock.clone();
            let ready_tx = crate::playsync::ReadyGuard::new(ready_tx.clone());
            let loop_control = loop_control.clone();
            expected_ready += 1;

            thread::spawn(move || {
                let song_name = song.name().to_string();

                if let Err(e) = dmx::engine::Engine::play(
                    dmx_engine,
                    song,
                    crate::playsync::PlaybackSync {
                        cancel_handle,
                        ready_tx,
                        clock,
                        start_time,
                        loop_control,
                    },
                ) {
                    error!(
                        err = e.as_ref(),
                        song = song_name,
                        "Error while playing DMX"
                    );
                }
            })
        });

        let midi_join_handle = if let Some(midi_device) = midi_device {
            let midi_device = midi_device.clone();
            let song = song.clone();
            let cancel_handle = cancel_handle.clone();
            let ready_tx = crate::playsync::ReadyGuard::new(ready_tx.clone());
            let clock = clock.clone();
            let loop_control = loop_control.clone();
            expected_ready += 1;

            Some(thread::spawn(move || {
                let song_name = song.name().to_string();

                if let Err(e) = midi_device.play_from(
                    song,
                    crate::playsync::PlaybackSync {
                        cancel_handle,
                        ready_tx,
                        clock,
                        start_time,
                        loop_control,
                    },
                ) {
                    error!(
                        err = e.as_ref(),
                        song = song_name,
                        "Error while playing song"
                    );
                }
            }))
        } else {
            None
        };

        // Drop the original sender so the channel closes when all subsystem
        // clones are dropped (important for error handling).
        drop(ready_tx);

        // Wait for all subsystems to signal readiness.
        for _ in 0..expected_ready {
            if ready_rx.recv().is_err() {
                // A subsystem dropped its sender without signaling (likely panicked).
                // Start the clock anyway so other subsystems don't spin forever.
                break;
            }
        }

        // Start the clock — this is the "go" signal for all subsystems.
        clock.start();

        // Set play_start_time NOW, at the exact moment playback begins.
        // Offset backwards by start_time so elapsed() reflects song position.
        // We use blocking_lock because we're in a spawn_blocking context.
        {
            let mut pst = play_start_time.blocking_lock();
            *pst = Some(SystemTime::now() - start_time);
        }

        if let Some(audio_join_handle) = audio_join_handle {
            if let Err(e) = audio_join_handle.join() {
                error!("Error waiting for audio to stop playing: {:?}", e)
            }
        }

        if let Some(dmx_join_handle) = dmx_join_handle {
            if let Err(e) = dmx_join_handle.join() {
                error!("Error waiting for DMX to stop playing: {:?}", e)
            }
        }

        if let Some(midi_join_handle) = midi_join_handle {
            if let Err(e) = midi_join_handle.join() {
                error!("Error waiting for MIDI to stop playing: {:?}", e)
            }
        }

        let outcome = resolve_playback_outcome(has_audio, audio_outcome.lock().take());
        if play_tx.send(outcome).is_err() {
            error!("Error while sending to finish channel (receiver dropped).")
        }
    }

    /// Stop will stop a song if a song is playing.
    pub async fn stop(&self) -> Option<Arc<Song>> {
        let mut join = self.join.lock().await;

        let play_handles = match join.take() {
            Some(handles) => handles,
            None => {
                info!("Player is not active, nothing to stop.");
                return None;
            }
        };

        let song = match self.get_playlist().current() {
            Some(song) => song,
            None => {
                info!("Playlist is empty, nothing to stop.");
                play_handles.cancel.cancel();
                drop(play_handles.join);
                return None;
            }
        };
        info!(song = song.name(), "Stopping playback.");

        play_handles.cancel.cancel();

        // Reset play start time — the cleanup task skips this when cancelled
        // so we must do it here.
        {
            let mut play_start_time = self.play_start_time.lock().await;
            *play_start_time = None;
        }

        drop(play_handles.join);
        drop(join);

        Some(song)
    }

    /// Returns true if a song is currently playing.
    pub async fn is_playing(&self) -> bool {
        self.join.lock().await.is_some()
    }

    /// Returns true if the current song has loop_playback enabled.
    /// Does not require the join lock — just checks the playlist's current song.
    pub fn is_current_song_looping(&self) -> bool {
        self.get_playlist()
            .current()
            .map(|song| song.loop_playback())
            .unwrap_or(false)
    }

    /// Gets the elapsed time from the play start time.
    pub async fn elapsed(&self) -> Result<Option<Duration>, Box<dyn Error>> {
        let play_start_time = self.play_start_time.lock().await;
        Ok(match *play_start_time {
            Some(play_start_time) => {
                let raw = play_start_time.elapsed()?;
                let consumed = *self.loop_time_consumed.lock();
                Some(raw.saturating_sub(consumed))
            }
            None => None,
        })
    }

    /// Adds time consumed by a section loop iteration. Called by subsystems
    /// when a section loop triggers to keep the reported elapsed time correct.
    pub fn add_loop_time_consumed(&self, duration: Duration) {
        let mut consumed = self.loop_time_consumed.lock();
        *consumed += duration;
    }

    /// Activates section looping for the named section in the current song.
    /// The song must be playing and the section must exist with a valid beat grid.
    pub async fn loop_section(&self, section_name: &str) -> Result<(), Box<dyn Error>> {
        let join = self.join.lock().await;
        if join.is_none() {
            return Err("Cannot loop section: no song is playing".into());
        }

        let playlist = self.get_playlist();
        let song = playlist
            .current()
            .ok_or("Cannot loop section: no current song")?;

        let (start_time, end_time) = song.resolve_section(section_name).ok_or_else(|| {
            format!(
                "Section '{}' not found or cannot be resolved (missing beat grid?)",
                section_name
            )
        })?;

        // Reject if playback has already passed the section end.
        let elapsed = self.elapsed().await?.unwrap_or(Duration::ZERO);
        if elapsed >= end_time {
            return Err(format!(
                "Cannot loop section '{}': playback has already passed the section end",
                section_name
            )
            .into());
        }

        info!(
            song = song.name(),
            section = section_name,
            start = ?start_time,
            end = ?end_time,
            "Activating section loop"
        );

        {
            let mut active = self.active_section.write();
            *active = Some(super::SectionBounds {
                name: section_name.to_string(),
                start_time,
                end_time,
            });
        }

        // Reset section_loop_break so the loop runs.
        self.section_loop_break.store(false, Ordering::Relaxed);

        // Play confirmation tone.
        self.play_confirmation_tone();

        Ok(())
    }

    /// Deactivates section looping. The current iteration finishes and the
    /// song continues from the section end point.
    pub fn stop_section_loop(&self) {
        info!("Stopping section loop");

        // Update reactive state machine.
        {
            let mut state = self.reactive_loop_state.write();
            match &*state {
                ReactiveLoopState::Looping(_) | ReactiveLoopState::BreakRequested(_) => {
                    if let ReactiveLoopState::Looping(bounds) = state.clone() {
                        *state = ReactiveLoopState::BreakRequested(bounds);
                        self.play_notification(
                            crate::notification::NotificationEvent::BreakRequested,
                        );
                    }
                }
                ReactiveLoopState::LoopArmed(_) => {
                    // Cancel the arm, return to idle.
                    *state = ReactiveLoopState::Idle;
                }
                _ => {}
            }
        }

        self.section_loop_break.store(true, Ordering::Relaxed);
        // Clear active section so the UI stops wrapping elapsed time
        // and shows normal playback state.
        *self.active_section.write() = None;
    }

    /// Returns the currently active section bounds, if any.
    pub fn active_section(&self) -> Option<super::SectionBounds> {
        self.active_section.read().clone()
    }

    /// Returns the current reactive loop state.
    pub fn reactive_loop_state(&self) -> ReactiveLoopState {
        self.reactive_loop_state.read().clone()
    }

    /// Acknowledges the current section in reactive looping mode.
    ///
    /// When a section has been offered (state is `SectionOffered`), this
    /// arms the loop so it will engage at the section end.
    pub async fn section_ack(&self) -> Result<(), Box<dyn Error>> {
        let mut state = self.reactive_loop_state.write();
        match state.clone() {
            ReactiveLoopState::SectionOffered(bounds) => {
                info!(section = bounds.name.as_str(), "Section loop armed via ack");
                *state = ReactiveLoopState::LoopArmed(bounds.clone());

                // Set active_section to engage the existing loop machinery.
                {
                    let mut active = self.active_section.write();
                    *active = Some(bounds);
                }
                self.section_loop_break.store(false, Ordering::Relaxed);
                drop(state);

                self.play_notification(crate::notification::NotificationEvent::LoopArmed);

                Ok(())
            }
            _ => Err("No section currently offered to acknowledge".into()),
        }
    }

    /// Checks whether the playhead has entered or left a section boundary.
    ///
    /// Called periodically by the section boundary polling task. Handles
    /// reactive state transitions: offering sections, detecting no-ack
    /// timeouts, and firing exit notifications.
    pub fn check_section_boundaries(&self, elapsed: Duration) {
        let playlist = self.get_playlist();
        let song = match playlist.current() {
            Some(s) => s,
            None => return,
        };

        let sections = song.sections();
        if sections.is_empty() {
            return;
        }

        // Find which section the playhead is currently in.
        let current_section = sections.iter().find_map(|s| {
            let (start, end) = song.resolve_section(&s.name)?;
            if elapsed >= start && elapsed < end {
                Some(super::SectionBounds {
                    name: s.name.clone(),
                    start_time: start,
                    end_time: end,
                })
            } else {
                None
            }
        });

        let mut state = self.reactive_loop_state.write();

        match (&*state, current_section) {
            // Idle + entered a section → offer it.
            (ReactiveLoopState::Idle, Some(bounds)) => {
                info!(
                    section = bounds.name.as_str(),
                    "Offering section for reactive loop"
                );
                self.play_notification(crate::notification::NotificationEvent::SectionEntering {
                    section_name: bounds.name.clone(),
                });
                *state = ReactiveLoopState::SectionOffered(bounds);
            }

            // SectionOffered but playhead has left the section → no ack, return to idle.
            (ReactiveLoopState::SectionOffered(offered), None) => {
                info!(
                    section = offered.name.as_str(),
                    "Section passed without ack, returning to idle"
                );
                self.play_notification(crate::notification::NotificationEvent::LoopExited);
                *state = ReactiveLoopState::Idle;
            }

            // SectionOffered but in a different section → update the offer.
            (ReactiveLoopState::SectionOffered(offered), Some(bounds))
                if offered.name != bounds.name =>
            {
                info!(
                    old = offered.name.as_str(),
                    new = bounds.name.as_str(),
                    "Section changed, updating offer"
                );
                self.play_notification(crate::notification::NotificationEvent::SectionEntering {
                    section_name: bounds.name.clone(),
                });
                *state = ReactiveLoopState::SectionOffered(bounds);
            }

            // BreakRequested + left section → loop exited.
            (ReactiveLoopState::BreakRequested(_), None) => {
                info!("Section loop exited after break");
                self.play_notification(crate::notification::NotificationEvent::LoopExited);
                *state = ReactiveLoopState::Idle;
            }

            // LoopArmed → Looping transition happens when the audio engine
            // actually begins looping. We detect this by checking if
            // active_section is set and we've wrapped around.
            (ReactiveLoopState::LoopArmed(bounds), Some(_)) => {
                // Check if the audio engine has started looping.
                let loop_consumed = *self.loop_time_consumed.lock();
                if loop_consumed > Duration::ZERO {
                    *state = ReactiveLoopState::Looping(bounds.clone());
                }
            }

            _ => {}
        }
    }

    /// Well-known track name for the section loop confirmation tone.
    /// Users route this in their audio track mappings, e.g.:
    /// ```yaml
    /// track_mappings:
    ///   mtrack:looping: [9]
    /// ```
    pub const LOOP_CONFIRMATION_TRACK: &'static str = "mtrack:looping";

    /// Plays a notification sound through the mixer via the notification engine.
    pub(super) fn play_notification(&self, event: crate::notification::NotificationEvent) {
        let hw = self.hardware.read();
        let device = match hw.device.as_ref() {
            Some(d) => d,
            None => return,
        };
        let mixer = match device.mixer() {
            Some(m) => m,
            None => return,
        };
        let mappings = match hw.mappings.as_ref() {
            Some(m) => m,
            None => return,
        };

        self.notification_engine.play(event, &mixer, mappings);
    }

    /// Plays the section loop confirmation tone through the mixer.
    /// Delegates to the notification engine.
    fn play_confirmation_tone(&self) {
        self.play_notification(crate::notification::NotificationEvent::LoopArmed);
    }

    /// Sets fade-out envelopes on all current audio sources for a smooth
    /// crossfade transition when breaking out of a loop.
    pub(super) fn fade_out_current_audio(&self) {
        let hw = self.hardware.read();
        if let Some(ref device) = hw.device {
            if let Some(mixer) = device.mixer() {
                let crossfade_samples =
                    crate::audio::crossfade::default_crossfade_samples(mixer.sample_rate());
                let source_ids: Vec<u64> = {
                    let sources = mixer.get_active_sources();
                    let guard = sources.read();
                    guard.iter().map(|s| s.lock().id).collect()
                };
                if !source_ids.is_empty() {
                    let fade_out = Arc::new(crate::audio::crossfade::GainEnvelope::fade_out(
                        crossfade_samples,
                        crate::audio::crossfade::CrossfadeCurve::Linear,
                    ));
                    mixer.set_gain_envelope(&source_ids, fade_out);
                }
            }
        }
    }
}
