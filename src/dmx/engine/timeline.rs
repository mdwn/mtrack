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

use tracing::{debug, error, warn};

use crate::lighting::engine::StopScope;
use crate::songs::Song;

use super::pass::{OutgoingPass, PassClock};
use super::Engine;

/// What a timeline fired in a tick, by the arming of the timeline it came
/// from (its epoch): the one key every engine's cues go back to it by.
/// Taken from a timeline under its lock, its groups resolved with no lock
/// held, it is applied under the engine's lock, and a seek, a section
/// loop's restart or a hand-over of the timeline to a pass can come in
/// between: a cue taken before a restart then landed on top of the state
/// the restart put in, and one taken before a pass swap on the fresh
/// engine rather than its own. So it goes to whichever engine that arming
/// drives by then -- the main one, a pass's, ambient's -- and nowhere if
/// it is gone.
pub(super) struct Fired {
    epoch: u64,
    /// A history going in first: ambient's show starting over at its end.
    restart: Option<crate::lighting::timeline::SeekHistory>,
    update: crate::lighting::timeline::TimelineUpdate,
}

impl Fired {
    /// What the timeline armed as `epoch` fired.
    pub(super) fn new(epoch: u64, update: crate::lighting::timeline::TimelineUpdate) -> Fired {
        Fired {
            epoch,
            restart: None,
            update,
        }
    }

    /// The same, after `history` -- the state the timeline restarted from.
    pub(super) fn restarted(
        epoch: u64,
        history: crate::lighting::timeline::SeekHistory,
        update: crate::lighting::timeline::TimelineUpdate,
    ) -> Fired {
        Fired {
            epoch,
            restart: Some(history),
            update,
        }
    }

    /// The arming of the timeline it came from.
    pub(super) fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Nothing to apply.
    fn is_empty(&self) -> bool {
        self.restart.is_none() && self.update.is_empty()
    }

    /// Applies it to `engine`, the one its timeline drives.
    pub(super) fn apply(self, engine: &mut crate::lighting::EffectEngine) {
        let mut errors = match self.restart {
            // Its own show only: the MIDI DMX store is every engine's.
            Some(history) => engine.start_from(history, StopScope::Show),
            None => Vec::new(),
        };
        errors.extend(engine.apply_timeline_update(self.update));
        for e in errors {
            error!("Failed to start lighting effect: {}", e);
        }
    }
}

/// What a lighting start does with the MIDI DMX playbacks, as it clears the
/// store: put in under the engine's lock with the clear, they can neither
/// dispatch an event ahead of it, which it would wipe, nor leave a frame
/// rendering the store empty.
pub(super) enum MidiDmxStart {
    /// Leaves them as they are.
    Leave,
    /// Installs a playback's, from the start point.
    Install(Vec<super::MidiDmxPlayback>),
    /// Moves the ones there to the start point (a section loop's restart).
    Seek,
    /// Moves the ones there to where a crossfading loop's pass starts,
    /// leaving the store as the outgoing pass had it: from the beginning,
    /// or for a pass begun late from where it joins, the channels the
    /// events before it set put back but one they set only later kept as
    /// the store carries it into the crossfade -- rather than sending
    /// everything before that in one frame.
    Join,
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
        .flat_map(|dsl_show| dsl_show.shows().values().cloned().collect::<Vec<_>>())
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
    /// Begins the next pass of a song looping with a crossfade of `overlap`,
    /// the jump going back from song time `from` to `to`: the running effects
    /// engine and timeline become the outgoing pass, `from - to` ahead in
    /// song time, and a fresh engine for the same rig starts `next` (the
    /// song's timeline, from [`song_lighting_timeline`]) at `to`. That is the
    /// top, or part way in for a pass begun late (a start, seek or section
    /// break inside the crossfade), which the audio starts there too.
    /// [`Engine::update_effects`] crosses the two over the overlap.
    pub fn begin_loop_crossfade(
        &self,
        next: Option<crate::lighting::timeline::LightingTimeline>,
        from: Duration,
        to: Duration,
        overlap: Duration,
    ) {
        self.begin_loop_crossfade_with(next, from, to, overlap, MidiDmxStart::Leave);
    }

    /// [`Self::begin_loop_crossfade`], moving the MIDI DMX playbacks to the
    /// new pass's start as its history goes in (see [`MidiDmxStart`]).
    pub(super) fn begin_loop_crossfade_with(
        &self,
        mut next: Option<crate::lighting::timeline::LightingTimeline>,
        from: Duration,
        to: Duration,
        overlap: Duration,
        midi: MidiDmxStart,
    ) {
        let lead = from.saturating_sub(to);
        // The timeline starts at `to` in the lights' delayed time base; the
        // crossfade counts from `to` against that time, so it begins
        // `dmx.playback_delay` after the audio's, as an on-time pass's does.
        let start = self.to_delayed_song_time(to);
        // A pass begun late starts from the state its cues before `to` leave,
        // as a seek does, rather than firing them all in its first frame.
        // Groups are resolved here, with no lock held.
        let history = next.as_mut().map(|timeline| {
            let mut history = timeline.start_at(start);
            history.map_effects(|effect| self.resolve_effect_groups(effect));
            history
        });
        // Song time first: a frame rendered between the swap and the reset
        // would read the old pass's time as the new one's, and end the
        // crossfade before it began.
        self.update_song_time(to);
        // The engines swap, the timelines with them, and the outgoing pass
        // is put aside, all under the engine's lock (taken in the effects
        // loop's order: engine, timeline, pass), so no frame renders the
        // fresh engine alone and a loop break begun meanwhile cannot take
        // one pass's engine with the other's timeline.
        let mut effect_engine = self.effect_engine.lock();
        let fresh = Self::fresh_engine_showing(&effect_engine);
        let outgoing_engine = std::mem::replace(&mut *effect_engine, fresh);
        if let Some(history) = history {
            for e in effect_engine.start_from(history, StopScope::Nothing) {
                error!("Failed to start lighting effect: {}", e);
            }
        }
        self.start_midi_dmx(midi, start);
        let timeline = std::mem::replace(&mut *self.current_song_timeline.lock(), next);
        let mut outgoing = self.outgoing_pass.lock();
        if let Some(pass) = outgoing.take() {
            self.break_fades.lock().retire(pass);
        }
        // What it drove the rig with until now held as it is, should it go
        // before its first frame as a pass.
        *outgoing = Some(OutgoingPass::crossing(
            outgoing_engine,
            timeline,
            PassClock::Song { lead, start: to },
            overlap,
            None,
            Vec::new(),
        ));
    }

    /// Begins fading a looping song's lights out after a loop break, over
    /// `fade` (its playlist `fade_out`): the running effects engine and timeline
    /// play on as the outgoing pass, on the wall clock from now, while a fresh
    /// engine for the same rig waits for the next song's show. That pass
    /// outlives this playback -- its cleanup and the next song's timeline
    /// start leave it be -- and ends when the fade does, sooner for a stop,
    /// pause or seek ([`Engine::hurry_break_fade`]). A crossfade between
    /// passes in progress carries on inside it, crossing into the engine it
    /// crossed into before (#560); an earlier break's fade goes on over it,
    /// at its own length.
    pub fn begin_break_fade(&self, fade: Duration) {
        self.begin_break_fade_over(fade, Vec::new());
    }

    /// [`Self::begin_break_fade`], fading `floor` -- channels the rig holds
    /// lit that nothing writes any more -- out with it.
    pub(super) fn begin_break_fade_over(
        &self,
        fade: Duration,
        floor: Vec<crate::lighting::effects::DmxCommand>,
    ) {
        let from = self.to_delayed_song_time(self.get_song_time());
        {
            let mut effect_engine = self.effect_engine.lock();
            let fresh = Self::fresh_engine_showing(&effect_engine);
            let outgoing_engine = std::mem::replace(&mut *effect_engine, fresh);
            // The timeline goes with its engine, taken under the same lock
            // (engine, then timeline, as nothing takes them the other way).
            let timeline = self.current_song_timeline.lock().take();
            let inner = self.outgoing_pass.lock().take();
            self.break_fades.lock().begin(OutgoingPass::crossing(
                outgoing_engine,
                timeline,
                PassClock::Wall {
                    started: std::time::Instant::now(),
                    from,
                },
                fade,
                inner,
                floor,
            ));
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

    /// Ends every loop break's fade at once.
    #[cfg(test)]
    pub fn end_break_fade(&self) {
        self.break_fades.lock().clear();
    }

    /// Hurries every loop break's fade out, for a stop, pause or seek that
    /// cuts it: rather than ending it, which would hold the frame it had
    /// reached (#572), it goes the rest of the way over `fade` from now --
    /// sooner, never later, than it would have.
    pub fn hurry_break_fade(&self, fade: Duration) {
        self.break_fades.lock().hurry(fade);
    }

    /// Drops a crossfade between passes of the playback being replaced or
    /// stopped. A loop break's fade outlives it.
    fn drop_pass_crossfade(&self) {
        let pass = self.outgoing_pass.lock().take();
        if let Some(pass) = pass {
            self.break_fades.lock().retire(pass);
        }
    }

    /// What `fired` holds that is not empty, its effects' groups resolved:
    /// against the lighting system, with no engine's lock held.
    pub(super) fn resolve_fired(&self, fired: Vec<Fired>) -> Vec<Fired> {
        fired
            .into_iter()
            .filter(|fired| !fired.is_empty())
            .map(|mut fired| {
                if let Some(history) = fired.restart.as_mut() {
                    history.map_effects(|effect| self.resolve_effect_groups(effect));
                }
                fired
                    .update
                    .map_effects(|effect| self.resolve_effect_groups(effect));
                fired
            })
            .collect()
    }

    /// Applies what the show's timelines fired, each to the engine its
    /// arming drives now (see [`Fired`]): the main one, or a pass's,
    /// wherever it has gone in between -- inside a fade begun since, say.
    fn apply_fired(&self, fired: Vec<Fired>) {
        if fired.is_empty() {
            return;
        }
        // The engine's lock, then the timeline's, as everything takes them:
        // a swap, which moves both under the engine's, cannot come between
        // the check and the apply.
        let mut effect_engine = self.effect_engine.lock();
        let current = self
            .current_song_timeline
            .lock()
            .as_ref()
            .map(|timeline| timeline.epoch());
        let mut outgoing = self.outgoing_pass.lock();
        let mut fades = self.break_fades.lock();
        for fired in fired {
            let engine = if current == Some(fired.epoch) {
                Some(&mut *effect_engine)
            } else {
                outgoing
                    .iter_mut()
                    .chain(fades.iter_mut())
                    .find_map(|pass| pass.engine_of(fired.epoch))
            };
            match engine {
                Some(engine) => fired.apply(engine),
                None => {
                    debug!("Dropping lighting cues from a timeline since restarted or replaced")
                }
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
                    &dsl_show.shows(),
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
        let mut fired: Vec<Fired> = self
            .current_song_timeline
            .lock()
            .as_mut()
            .map(|timeline| Fired::new(timeline.epoch(), timeline.update(song_time)))
            .into_iter()
            .collect();
        // The outgoing pass's timeline, at its own time a pass ahead of the
        // incoming one's; a pass inside it, on its own; each loop break's
        // fade, on its own.
        {
            let mut outgoing = self.outgoing_pass.lock();
            let mut fades = self.break_fades.lock();
            // A loop break's fade that is over fires nothing more.
            fades.sweep(song_time);
            for pass in outgoing.iter_mut().chain(fades.iter_mut()) {
                pass.advance_timelines(song_time, &mut fired);
            }
        }
        self.apply_fired(self.resolve_fired(fired));
        Ok(())
    }

    /// Starts the lighting timeline at a specific time
    ///
    /// `start_time` is song-absolute; it is shifted into the delayed lighting time
    /// base so the reconstructed state matches what the live loop advances from.
    pub fn start_lighting_timeline_at(&self, start_time: Duration) {
        self.start_lighting_timeline_with(start_time, MidiDmxStart::Leave);
    }

    /// [`Self::start_lighting_timeline_at`], starting the MIDI DMX
    /// playbacks at the same point as it clears the store (#583).
    pub(super) fn start_lighting_timeline_with(&self, start_time: Duration, midi: MidiDmxStart) {
        // A crossfade in progress belongs to the playback being replaced; a
        // loop break's fade carries on over this song's start.
        self.drop_pass_crossfade();
        self.restart_lighting_timeline(start_time, midi);
    }

    /// Before a section's jump with no overlap: a crossfade between passes
    /// still in progress plays out on its own time, as the audio's and
    /// MIDI's do, rather than going back with the song time.
    pub(super) fn keep_crossfade_on_own_time(&self) {
        let song_time = self.to_delayed_song_time(self.get_song_time());
        if let Some(pass) = self.outgoing_pass.lock().as_mut() {
            pass.keep_own_time(song_time);
        }
    }

    /// A section's jump with no overlap, back to `start_time`: the timeline
    /// restarts there as for a seek, keeping a crossfade in progress (see
    /// [`Self::keep_crossfade_on_own_time`]).
    pub(super) fn restart_section_at(&self, start_time: Duration) {
        self.restart_lighting_timeline(start_time, MidiDmxStart::Seek);
    }

    fn restart_lighting_timeline(&self, start_time: Duration, midi: MidiDmxStart) {
        self.rebuild_at(
            self.to_delayed_song_time(start_time),
            None,
            StopScope::All,
            midi,
        );
    }

    /// A show file edited live: `timeline`, its shows parsed again, takes
    /// the current one's place, rebuilt where the lights are now, its
    /// effects on `tempo_map`. Only the show's effects stop: the MIDI DMX
    /// store goes on as MIDI set it, which nothing would put back mid-song
    /// (#583).
    pub(crate) fn reload_show(
        &self,
        timeline: crate::lighting::timeline::LightingTimeline,
        tempo_map: Option<crate::lighting::tempo::TempoMap>,
    ) {
        // Where the lights are, not the transport: rebuilding at the
        // undelayed time put the show the delay ahead of the rig, skipping
        // the cues in between (#593).
        self.rebuild_at(
            self.to_delayed_song_time(self.get_song_time()),
            Some((timeline, tempo_map)),
            StopScope::Show,
            MidiDmxStart::Leave,
        );
    }

    /// Rebuilds the main show at `at` (delayed song time): the current
    /// timeline armed there -- or `reload`'s, with its tempo, swapped in for
    /// it -- and the state its cues before `at` leave put into the main
    /// engine in place of what `scope` stops, `midi` done with the MIDI DMX
    /// playbacks as it goes in. The one way a seek, a section's restart, a
    /// song's start and a live reload rebuild the show.
    ///
    /// What goes and what comes in do so under the one lock, groups
    /// resolved first with none held: a frame between the two would find
    /// the channels unwritten and send them 0 -- a black flash at every seek
    /// and section loop (#581). The timeline fires nothing until the state
    /// is in; then its cues -- one at the start point too -- fire over it.
    fn rebuild_at(
        &self,
        at: Duration,
        reload: Option<(
            crate::lighting::timeline::LightingTimeline,
            Option<crate::lighting::tempo::TempoMap>,
        )>,
        scope: StopScope,
        midi: MidiDmxStart,
    ) {
        // A reloaded timeline is armed before it goes in, with no lock held.
        let reload = reload.map(|(mut timeline, tempo_map)| {
            let history = timeline.start_at(at);
            (timeline, tempo_map, history)
        });
        let (mut history, tempo_map) = {
            let mut current = self.current_song_timeline.lock();
            let (history, tempo_map) = match reload {
                Some((mut timeline, tempo_map, history)) => {
                    // The song's end bound belongs to the song, not the
                    // show text.
                    timeline.set_end_cap(current.as_ref().and_then(|t| t.end_cap()));
                    timeline.hold(true);
                    *current = Some(timeline);
                    (history, Some(tempo_map))
                }
                None => {
                    let history = match current.as_mut() {
                        Some(timeline) => {
                            timeline.hold(true);
                            timeline.start_at(at)
                        }
                        None => crate::lighting::timeline::SeekHistory {
                            at,
                            ..Default::default()
                        },
                    };
                    (history, None)
                }
            };
            // A show that is loaded but exhausted the moment it is armed
            // will not fire anything for the whole playback, and says
            // nothing about it. That is the shape of #332, so it is worth a
            // line in the log.
            if let Some(tl) = current.as_ref() {
                if tl.cue_count() > 0 && tl.cues_remaining() == 0 {
                    warn!(
                        cues = tl.cue_count(),
                        start_time_ms = at.as_millis(),
                        "lighting timeline armed with no cues ahead of the pointer — \
                         no cues will fire for this playback"
                    );
                }
            }
            (history, tempo_map)
        };
        history.map_effects(|effect| self.resolve_effect_groups(effect));
        let mut effect_engine = self.effect_engine.lock();
        if let Some(tempo_map) = tempo_map {
            effect_engine.set_tempo_map(tempo_map);
        }
        for e in effect_engine.start_from(history, scope) {
            error!("Failed to start lighting effect: {}", e);
        }
        self.start_midi_dmx(midi, at);
        drop(effect_engine);
        if let Some(timeline) = self.current_song_timeline.lock().as_mut() {
            timeline.hold(false);
        }
    }

    /// Does what `midi` says with the MIDI DMX playbacks for a show
    /// starting at `at` (delayed song time). Called under the engine's
    /// lock, with the show's start state going in.
    fn start_midi_dmx(&self, midi: MidiDmxStart, at: Duration) {
        match midi {
            MidiDmxStart::Leave => {}
            MidiDmxStart::Install(playbacks) => {
                *self.midi_dmx_playbacks.lock() = playbacks;
                self.seek_midi_dmx_playbacks(at);
            }
            MidiDmxStart::Seek => self.seek_midi_dmx_playbacks(at),
            MidiDmxStart::Join if at.is_zero() => self.rewind_midi_dmx_playbacks(),
            MidiDmxStart::Join => self.join_midi_dmx_playbacks(at),
        }
    }

    /// Applies what the timeline armed as `epoch` fired (effects and layer
    /// commands) to the engine that arming drives (see [`Fired`]).
    #[cfg(test)]
    pub(super) fn apply_timeline_update(
        &self,
        timeline_update: crate::lighting::timeline::TimelineUpdate,
        epoch: u64,
    ) -> Result<(), Box<dyn Error>> {
        self.apply_fired(self.resolve_fired(vec![Fired::new(epoch, timeline_update)]));
        Ok(())
    }

    /// Resolves group names in an effect's target_fixtures to actual fixture names
    /// using the lighting system. If no lighting system is available, returns the
    /// effect unchanged (groups are passed through as-is).
    pub(in crate::dmx) fn resolve_effect_groups(
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
