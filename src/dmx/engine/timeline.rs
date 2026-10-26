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

/// The pass of a crossfading song loop that the lights are fading out of: the
/// effects engine and timeline the last pass ran in, moved aside whole when
/// the next pass began, so it plays on exactly as it was -- its remaining
/// cues included -- while the two frames cross.
pub(crate) struct OutgoingPass {
    pub(super) effect_engine: crate::lighting::EffectEngine,
    pub(super) timeline: Option<crate::lighting::timeline::LightingTimeline>,
    /// What the outgoing pass's time and the crossfade's progress run on.
    pub(super) clock: PassClock,
    /// How long the passes overlap.
    pub(super) overlap: Duration,
}

/// What an [`OutgoingPass`] keeps time by.
pub(crate) enum PassClock {
    /// The incoming pass's song time: a crossfade between passes of one
    /// looping song, the outgoing pass `lead` (a pass's length less the
    /// overlap) ahead.
    Song { lead: Duration },
    /// The wall clock from a loop break (a playlist `fade_out`): the looping song
    /// fades out over the next song's start, whose song time is not running
    /// yet and has nothing to do with it. The outgoing pass plays on from
    /// `from`, its song time at the break.
    Wall {
        started: std::time::Instant,
        from: Duration,
    },
}

impl OutgoingPass {
    /// How far through the crossfade it is (0.0 to 1.0 and beyond), and the
    /// outgoing pass's own song time, given the incoming pass's.
    pub(super) fn timing(&self, song_time: Duration) -> (f64, Duration) {
        let (into, time) = match self.clock {
            PassClock::Song { lead } => (song_time, song_time + lead),
            PassClock::Wall { started, from } => {
                let elapsed = started.elapsed();
                (elapsed, from + elapsed)
            }
        };
        (
            into.as_secs_f64() / self.overlap.as_secs_f64().max(f64::EPSILON),
            time,
        )
    }

    /// Whether this pass fades a loop break out, which outlives the playback
    /// it came from: only a stop or seek ends it early.
    fn is_break_fade(&self) -> bool {
        matches!(self.clock, PassClock::Wall { .. })
    }
}

/// The lighting timeline for a song's DSL shows, ready to start; `None` for a
/// song without any. With audio, the audio ends the song as it always has;
/// without, the last effect does rather than the last cue.
pub(crate) fn song_lighting_timeline(
    song: &Song,
) -> Option<crate::lighting::timeline::LightingTimeline> {
    let all_shows: Vec<_> = song
        .dsl_lighting_shows()
        .iter()
        .flat_map(|dsl_show| dsl_show.shows().values().cloned())
        .collect();
    if all_shows.is_empty() {
        return None;
    }
    let mut timeline = crate::lighting::timeline::LightingTimeline::new(all_shows);
    let audio_length = song.duration();
    timeline.set_end_cap((!audio_length.is_zero()).then_some(audio_length));
    Some(timeline)
}

impl Engine {
    /// Begins the next pass of a song looping with a crossfade of `overlap`:
    /// the running effects engine and timeline become the outgoing pass, `lead`
    /// ahead in song time, and a fresh engine for the same rig starts `next`
    /// (the song's timeline, from [`song_lighting_timeline`]) from the top.
    /// [`Engine::update_effects`] crosses the two over the overlap.
    pub fn begin_loop_crossfade(
        &self,
        next: Option<crate::lighting::timeline::LightingTimeline>,
        lead: Duration,
        overlap: Duration,
    ) {
        // Song time first: a frame rendered between the swap and the reset
        // would read the old pass's time as the new one's, and end the
        // crossfade before it began.
        self.update_song_time(Duration::ZERO);
        // The engines swap and the outgoing one is put aside under the one
        // lock (taken in the effects loop's order: engine, then pass), so no
        // frame renders the fresh engine alone.
        {
            let mut effect_engine = self.effect_engine.lock();
            let fresh = Self::fresh_engine_showing(&effect_engine);
            let outgoing_engine = std::mem::replace(&mut *effect_engine, fresh);
            *self.outgoing_pass.lock() = Some(OutgoingPass {
                effect_engine: outgoing_engine,
                timeline: None,
                clock: PassClock::Song { lead },
                overlap,
            });
        }
        // Then the timelines. Until the old one joins its pass, that pass's
        // cues wait a frame; none are lost, since a timeline fires every cue
        // up to the time it is given.
        let outgoing_timeline = {
            let mut current = self.current_song_timeline.lock();
            let outgoing = current.take();
            *current = next.map(|mut timeline| {
                timeline.start();
                timeline
            });
            outgoing
        };
        if let Some(pass) = self.outgoing_pass.lock().as_mut() {
            pass.timeline = outgoing_timeline;
        }
    }

    /// Begins fading a looping song's lights out after a loop break, over
    /// `fade` (its playlist `fade_out`): the running effects engine and timeline
    /// play on as the outgoing pass, on the wall clock from now, while a fresh
    /// engine for the same rig waits for the next song's show. That pass
    /// outlives this playback -- its cleanup and the next song's timeline
    /// start leave it be -- and ends when the fade does, or with a stop or
    /// seek ([`Engine::end_break_fade`]). A pass crossfade in progress gives
    /// way: its older pass goes at once.
    pub fn begin_break_fade(&self, fade: Duration) {
        let from = self.to_delayed_song_time(self.get_song_time());
        {
            let mut effect_engine = self.effect_engine.lock();
            let fresh = Self::fresh_engine_showing(&effect_engine);
            let outgoing_engine = std::mem::replace(&mut *effect_engine, fresh);
            *self.outgoing_pass.lock() = Some(OutgoingPass {
                effect_engine: outgoing_engine,
                timeline: None,
                clock: PassClock::Wall {
                    started: std::time::Instant::now(),
                    from,
                },
                overlap: fade,
            });
        }
        let outgoing_timeline = self.current_song_timeline.lock().take();
        if let Some(pass) = self.outgoing_pass.lock().as_mut() {
            pass.timeline = outgoing_timeline;
        }
    }

    /// A fresh engine for the same rig to swap in at a loop pass or break,
    /// showing what `current` shows until its first blended frame replaces
    /// it: with nothing to show, a snapshot taken between the swap and that
    /// frame -- the web UI's, `get_fixture_state`'s -- read black (#555).
    fn fresh_engine_showing(
        current: &crate::lighting::EffectEngine,
    ) -> crate::lighting::EffectEngine {
        let mut fresh = current.sibling();
        fresh.set_shown(Some((
            current.shown_fixture_states().clone(),
            current.shown_poses().clone(),
        )));
        fresh
    }

    /// Ends a loop break's fade early, for a stop or seek.
    pub fn end_break_fade(&self) {
        let mut outgoing = self.outgoing_pass.lock();
        if outgoing.as_ref().is_some_and(OutgoingPass::is_break_fade) {
            *outgoing = None;
        }
    }

    /// Drops a crossfade between passes of the playback being replaced or
    /// stopped. A loop break's fade outlives it.
    fn drop_pass_crossfade(&self) {
        let mut outgoing = self.outgoing_pass.lock();
        if outgoing.as_ref().is_some_and(|pass| !pass.is_break_fade()) {
            *outgoing = None;
        }
    }

    /// Advances the outgoing pass's timeline to `song_time` (the incoming
    /// pass's) and starts what it fires in the outgoing pass's engine.
    fn update_outgoing_pass(&self, song_time: Duration) {
        let update = {
            let mut outgoing = self.outgoing_pass.lock();
            let Some(pass) = outgoing.as_mut() else {
                return;
            };
            let (_, time) = pass.timing(song_time);
            match pass.timeline.as_mut() {
                Some(timeline) => timeline.update(time),
                None => return,
            }
        };
        // Groups resolve against the lighting system before the outgoing
        // pass is locked again, as for the incoming pass.
        let mut effects = update.effects;
        effects.sort_by_key(|e| if e.id.starts_with("seq_") { 0 } else { 1 });
        let effects: Vec<_> = effects
            .into_iter()
            .map(|effect| self.resolve_effect_groups(effect))
            .collect();
        let mut outgoing = self.outgoing_pass.lock();
        let Some(pass) = outgoing.as_mut() else {
            return;
        };
        for cmd in &update.layer_commands {
            pass.effect_engine.apply_layer_command(cmd);
        }
        for sequence_name in &update.stop_sequences {
            pass.effect_engine.stop_sequence(sequence_name);
        }
        for effect in effects {
            if let Err(e) = pass.effect_engine.start_effect(effect) {
                error!(
                    "Failed to start lighting effect in the outgoing loop pass: {}",
                    e
                );
            }
        }
    }

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

        self.update_outgoing_pass(song_time);
        self.apply_timeline_update(timeline_update)
    }

    /// Starts the lighting timeline at a specific time
    ///
    /// `start_time` is song-absolute; it is shifted into the delayed lighting time
    /// base so the reconstructed state matches what the live loop advances from.
    pub fn start_lighting_timeline_at(&self, start_time: Duration) {
        let start_time = self.to_delayed_song_time(start_time);
        // A crossfade in progress belongs to the playback being replaced; a
        // loop break's fade carries on over this song's start.
        self.drop_pass_crossfade();
        // Clear effects from previous song before starting new timeline
        {
            let mut effect_engine = self.effect_engine.lock();
            effect_engine.stop_all_effects();
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
        // A pass being faded out stops with the playback; the song time that
        // drives the fade does too. A loop break's fade, on its own clock,
        // outlives the playback it came from.
        self.drop_pass_crossfade();
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
    ///
    /// Best-effort, not airtight: the generation is read and the time stored as
    /// two operations, so a writer preempted between them can still land after
    /// a new playback has begun. It closes the window that stays open for a
    /// whole tracker sleep interval, not the one that lasts two instructions.
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
