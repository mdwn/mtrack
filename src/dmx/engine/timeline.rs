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

use std::{error::Error, time::Duration};

use tracing::{error, warn};

use crate::songs::Song;

use super::Engine;

impl Engine {
    /// Validates a song's lighting shows against the engine's lighting config.
    /// Returns an error if any lighting show references invalid groups or fixtures.
    pub fn validate_song_lighting(&self, song: &Song) -> Result<(), Box<dyn Error>> {
        let dsl_lighting_shows = song.dsl_lighting_shows();

        if dsl_lighting_shows.is_empty() {
            return Ok(());
        }

        // Validate group/fixture references against lighting config
        if let Some(ref lighting_config) = self.lighting_config {
            for dsl_show in dsl_lighting_shows {
                crate::lighting::validation::validate_light_shows(
                    dsl_show.shows(),
                    Some(lighting_config),
                )
                .map_err(|e| {
                    format!(
                        "Light show validation failed for {}: {}",
                        dsl_show.file_path().display(),
                        e
                    )
                })?;
            }
        }

        Ok(())
    }

    /// Updates the lighting timeline with the current song time
    pub fn update_song_lighting(
        &self,
        song_time: std::time::Duration,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let timeline_update = {
            let mut current_timeline = self.current_song_timeline.lock();
            if let Some(timeline) = current_timeline.as_mut() {
                timeline.update(song_time)
            } else {
                crate::lighting::timeline::TimelineUpdate::default()
            }
        };

        self.apply_timeline_update(timeline_update)
    }

    /// Starts the lighting timeline at a specific time
    ///
    /// `start_time` is song-absolute; it is shifted into the delayed lighting time
    /// base so the reconstructed state matches what the live loop advances from.
    pub fn start_lighting_timeline_at(&self, start_time: Duration) {
        let start_time = self.to_delayed_song_time(start_time);
        // Clear effects from previous song before starting new timeline
        {
            let mut effect_engine = self.effect_engine.lock();
            effect_engine.stop_all_effects();
        }
        // Clear effect deduplication caches so the new song's first frame is applied
        for universe in self.universes.values() {
            universe.clear_effect_cache();
        }

        let timeline_update = {
            let mut current_timeline = self.current_song_timeline.lock();
            if let Some(timeline) = current_timeline.as_mut() {
                if start_time == Duration::ZERO {
                    timeline.start();
                    crate::lighting::timeline::TimelineUpdate::default()
                } else {
                    // Process historical cues to ensure deterministic state
                    timeline.start_at(start_time)
                }
            } else {
                crate::lighting::timeline::TimelineUpdate::default()
            }
        };

        // A show that is loaded but exhausted the moment it is armed will not
        // fire anything for the whole playback, and says nothing about it. That
        // is the shape of #332, so it is worth a line in the log.
        {
            let timeline = self.current_song_timeline.lock();
            if let Some(tl) = timeline.as_ref() {
                if tl.cue_count() > 0 && tl.cues_remaining() == 0 {
                    warn!(
                        cues = tl.cue_count(),
                        start_time_ms = start_time.as_millis(),
                        "lighting timeline armed with no cues ahead of the pointer — \
                         no cues will fire for this playback"
                    );
                }
            }
        }

        // Apply the historical timeline update to ensure deterministic state
        // This must happen before the effects loop can process new cues to avoid conflicts
        if start_time > Duration::ZERO {
            if let Err(e) = self.apply_timeline_update(timeline_update) {
                error!("Failed to apply historical timeline state: {}", e);
            }
        }
    }

    /// Applies a timeline update (effects and layer commands)
    pub(super) fn apply_timeline_update(
        &self,
        timeline_update: crate::lighting::timeline::TimelineUpdate,
    ) -> Result<(), Box<dyn Error>> {
        // Process layer commands first (they affect subsequent effects)
        if !timeline_update.layer_commands.is_empty() {
            let mut effects_engine = self.effect_engine.lock();
            for cmd in &timeline_update.layer_commands {
                effects_engine.apply_layer_command(cmd);
            }
        }

        // Process stop sequence commands
        if !timeline_update.stop_sequences.is_empty() {
            let mut effects_engine = self.effect_engine.lock();
            for sequence_name in &timeline_update.stop_sequences {
                effects_engine.stop_sequence(sequence_name);
            }
        }

        // Start effects with pre-calculated elapsed time (from seeking)
        // Sort by cue_time to ensure chronological order — later effects properly
        // conflict with and stop earlier ones
        let mut effects_sorted: Vec<_> = timeline_update.effects_with_elapsed.values().collect();
        effects_sorted.sort_by_key(|(effect, _)| effect.cue_time.unwrap_or(Duration::ZERO));

        for (effect, elapsed_time) in effects_sorted {
            let resolved = self.resolve_effect_groups(effect.clone());
            let mut effect_engine = self.effect_engine.lock();
            if let Err(e) = effect_engine.start_effect_with_elapsed(resolved, *elapsed_time) {
                error!("Failed to start lighting effect with elapsed time: {}", e);
            }
        }

        // Handle regular effects (from normal timeline updates).
        // Sort so sequence effects start before song effects. When a sequence's
        // last cue (timing anchor) fires at the same time as the next show cue,
        // starting sequence effects first ensures show-level effects win any
        // Replace-blend conflicts via the conflict resolution in start_effect().
        let mut effects = timeline_update.effects;
        effects.sort_by_key(|e| if e.id.starts_with("seq_") { 0 } else { 1 });
        for effect in effects {
            let resolved = self.resolve_effect_groups(effect);
            if let Err(e) = self.start_effect(resolved) {
                error!("Failed to start lighting effect: {}", e);
            }
        }

        Ok(())
    }

    /// Resolves group names in an effect's target_fixtures to actual fixture names
    /// using the lighting system. If no lighting system is available, returns the
    /// effect unchanged (groups are passed through as-is).
    pub(super) fn resolve_effect_groups(
        &self,
        effect: crate::lighting::EffectInstance,
    ) -> crate::lighting::EffectInstance {
        if let Some(lighting_system) = &self.lighting_system {
            let mut lighting_system = lighting_system.lock();
            lighting_system.resolve_effect_groups(effect)
        } else {
            effect
        }
    }

    /// Stops the lighting timeline
    pub fn stop_lighting_timeline(&self) {
        let mut current_timeline = self.current_song_timeline.lock();
        if let Some(timeline) = current_timeline.as_mut() {
            timeline.stop();
        }

        // Note: We do NOT stop effects here - they should continue running
        // until they naturally complete or until the next song starts.
        // Effects are only cleared when a new song starts (in start_lighting_timeline_at)
        // or when explicitly stopping playback.
    }

    /// Updates the current song time.
    pub fn update_song_time(&self, song_time: Duration) {
        self.current_song_time.store(
            song_time.as_nanos() as u64,
            std::sync::atomic::Ordering::Relaxed,
        );
    }

    /// Updates the current song time on behalf of a specific playback.
    ///
    /// A write stamped with a superseded generation is dropped. See
    /// [`Engine::playback_generation`] for why one stale write is fatal rather
    /// than merely untidy.
    pub fn update_song_time_for_generation(&self, song_time: Duration, generation: u64) {
        if generation != self.playback_generation() {
            return;
        }
        self.update_song_time(song_time);
    }

    /// Starts a new playback generation, superseding the previous one's
    /// song-time writers. Returns the new generation.
    pub fn begin_playback_generation(&self) -> u64 {
        self.playback_generation
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
            + 1
    }

    /// The generation writes must carry to be accepted.
    pub fn playback_generation(&self) -> u64 {
        self.playback_generation
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Gets the current song time
    pub fn get_song_time(&self) -> Duration {
        Duration::from_nanos(
            self.current_song_time
                .load(std::sync::atomic::Ordering::Relaxed),
        )
    }

    /// Song time with the configured `dmx.playback_delay` applied, or `None` while
    /// still inside the delay window.
    ///
    /// Audio leaves late — decode/transcode, the CPAL/ALSA stream buffer, device
    /// latency — while DMX frames go out with only frame time and fixture response
    /// in the way. So lights driven straight off the transport clock fire early
    /// relative to what the audience hears, and this is the knob that pulls them
    /// back. `None` means the show has not started yet as far as the lights are
    /// concerned: hold, don't fire at zero.
    pub(crate) fn delayed_song_time(&self) -> Option<Duration> {
        self.get_song_time().checked_sub(self.playback_delay)
    }

    /// Shift a song-absolute time into the delayed lighting time base.
    ///
    /// Used for the reconstruction points that don't come from the live clock
    /// (playback start, section-loop restart), so a seek lands on the same show
    /// state the live loop will go on to advance from.
    pub(crate) fn to_delayed_song_time(&self, song_time: Duration) -> Duration {
        song_time.saturating_sub(self.playback_delay)
    }

    /// A one-call answer to "is the lighting actually going to fire".
    ///
    /// Returns `None` when no show is loaded. Otherwise reports whether the
    /// timeline is armed and where its cue pointer sits — the state that
    /// previously could only be inferred by watching for effects and waiting
    /// to see whether any arrived.
    pub fn lighting_arming_state(&self) -> Option<(bool, usize, usize, Option<Duration>)> {
        let timeline = self.current_song_timeline.lock();
        timeline.as_ref().map(|tl| {
            (
                tl.is_armed(),
                tl.cue_count(),
                tl.cues_remaining(),
                tl.next_cue_time(),
            )
        })
    }

    /// Gets all cues from the current timeline with their times and indices
    pub fn get_timeline_cues(&self) -> Vec<(Duration, usize)> {
        let timeline = self.current_song_timeline.lock();
        if let Some(timeline) = timeline.as_ref() {
            timeline.cues()
        } else {
            Vec::new()
        }
    }
}
