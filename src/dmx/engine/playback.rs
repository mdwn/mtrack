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
    collections::{HashMap, HashSet},
    error::Error,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use tracing::{error, info, span, warn, Level};

use crate::{playsync::CancelHandle, songs::Song};

use super::{Engine, MidiDmxPlayback};

/// How much of a song [`Engine::release_song`] lets go of.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Release {
    /// Its playback is over: its MIDI DMX playbacks go and its timeline
    /// stops, kept for status. Its effects run on to their own ends, at its
    /// tempo, until the next song starts (or the stage goes dark); the show
    /// file watcher stays for a live edit.
    Ended,
    /// The next song has no show of its own: the timeline and tempo go too,
    /// or it would answer status with the last song's cues and inherit its
    /// tempo, and the watcher, whose reload would put the last song's show
    /// back under it.
    Replaced,
    /// The stage goes dark between songs: everything the song left fades
    /// out over `fade` as a loop break's fade, and with `floor` what the
    /// rig holds lit that nothing writes any more too (#573) -- left as it
    /// is under ambient at full, still in view where ambient does not light.
    Dark { fade: Duration, floor: bool },
}

/// The lighting timeline of the pass a crossfading loop jump starts (see
/// [`super::timeline::song_lighting_timeline`]), built ahead of the jump.
#[derive(Default)]
struct NextPassTimeline {
    /// The jump it is for, and where its build comes.
    building: Option<(
        crate::section_loop::SectionJump,
        std::sync::mpsc::Receiver<Option<crate::lighting::timeline::LightingTimeline>>,
    )>,
}

impl NextPassTimeline {
    /// Starts building the timeline for `jump`, unless it is already.
    fn build(&mut self, song: &Arc<Song>, jump: &crate::section_loop::SectionJump) {
        if self.building.as_ref().is_some_and(|(of, _)| of == jump) {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let song = song.clone();
        let spawned = thread::Builder::new()
            .name("mtrack-dmx-pass-build".into())
            .spawn(move || {
                let _ = tx.send(super::timeline::song_lighting_timeline(&song));
            });
        // Without a thread, it is built at the jump, as it always was.
        self.building = spawned.is_ok().then_some((jump.clone(), rx));
    }

    /// The timeline for `jump`: the one built ahead, waiting for its build
    /// to finish if it has not; built now if none was for it.
    fn take(
        &mut self,
        song: &Song,
        jump: &crate::section_loop::SectionJump,
    ) -> Option<crate::lighting::timeline::LightingTimeline> {
        match self.building.take() {
            Some((of, rx)) if of == *jump => rx
                .recv()
                .unwrap_or_else(|_| super::timeline::song_lighting_timeline(song)),
            _ => super::timeline::song_lighting_timeline(song),
        }
    }
}

impl Engine {
    /// Plays the given song through the DMX interface.
    pub fn play(
        dmx_engine: Arc<Engine>,
        song: Arc<Song>,
        sync: crate::playsync::PlaybackSync,
    ) -> Result<(), Box<dyn Error>> {
        let crate::playsync::PlaybackSync {
            cancel_handle,
            mut ready_tx,
            clock,
            start_time,
            loop_control,
        } = sync;
        let crate::playsync::LoopControl {
            loop_break,
            section_jumps,
            ..
        } = loop_control;
        // A song owns the lights: a fixture test stops when one starts.
        dmx_engine.release_test_output();

        // Claim this playback's generation before anything else: before
        // this song's timeline goes in, and for a song with no lights of its
        // own too. The outgoing playback's tracker and jump follower may
        // still be awake for up to their sleep interval, carrying the
        // previous song's time and jumps; from here their writes are dropped
        // rather than landing on this one's timeline.
        let generation = dmx_engine.begin_playback_generation();
        // The last song's show file watcher goes now too, before this one's
        // timeline goes in, whatever this song lights -- a reload it has
        // yet to make would put the last song's show in its place -- and
        // any rebuild of the last song's show still on its way in with it.
        let watcher_generation = dmx_engine.retire_watcher();
        let span = span!(Level::INFO, "play song (dmx)");
        let _enter = span.enter();

        // Check if there are any lighting systems to play
        let light_shows = song.light_shows();
        let dsl_lighting_shows = song.dsl_lighting_shows();
        let has_lighting = !dsl_lighting_shows.is_empty();

        if light_shows.is_empty() && !has_lighting {
            // A song with no lights of its own leaves ambient light playing.
            if dmx_engine.ambient_playing() {
                ready_tx.send();
                return Ok(());
            }
            // A song with no lighting at all still has to evict the previous
            // song's state, or `status.lighting` and `get_timeline_cues()` keep
            // answering with that song's cues.
            dmx_engine.release_song(Release::Replaced);
            ready_tx.send();
            return Ok(());
        }

        info!(
            song = song.name(),
            duration = song.duration_string(),
            "Playing song DMX."
        );

        // The song's lights take over from ambient light, fading it out
        // under the song's show.
        dmx_engine.leave_ambient();

        // Register fixtures with the effects engine if lighting system is available
        if let Err(_e) = dmx_engine.register_venue_fixtures_safe() {
            // The song plays without fixtures; the failure is recorded and
            // logged by the registration (`venue_registration`), and shown
            // in the status and readiness views.
        }

        // Setup song lighting if available - work directly with DSL shows
        if has_lighting {
            info!(
                "Setup lighting timeline with {} DSL light shows",
                dsl_lighting_shows.len()
            );

            if let Some(timeline) = super::timeline::song_lighting_timeline(&song) {
                // Set or clear the tempo map — a song without a tempo block must not
                // inherit one from the previous song. Shows without their own tempo
                // block fall back to the song's tempo map (song.yaml `tempo:`).
                {
                    let mut effect_engine = dmx_engine.effect_engine.lock();
                    effect_engine
                        .set_tempo_map(timeline.tempo_map_or(song.lighting_tempo_map().as_ref()));
                }
                {
                    let mut current_timeline = dmx_engine.current_song_timeline.lock();
                    *current_timeline = Some(timeline);
                }
            }
        } else {
            // Clear lighting state from previous song so MIDI DMX songs
            // don't inherit a stale tempo map or timeline.
            dmx_engine.release_song(Release::Replaced);
        }

        let universe_ids: HashSet<u16> = dmx_engine.universes.keys().cloned().collect();

        let mut dmx_midi_sheets: HashMap<String, (crate::songs::MidiSheet, Vec<u8>)> =
            HashMap::new();
        for light_show in song.light_shows().iter() {
            let universe_name = light_show.universe_name();
            if let Some(&universe_id) = dmx_engine.universe_name_to_id.get(&universe_name) {
                if !universe_ids.contains(&universe_id) {
                    continue;
                }

                dmx_midi_sheets.insert(
                    universe_name.clone(),
                    (light_show.dmx_midi_sheet()?, light_show.midi_channels()),
                );
            }
        }

        // Build MIDI DMX playbacks for effects-loop dispatch, installed by
        // the timeline start below. Drain the map to take ownership of
        // MidiSheets (avoids cloning event vecs). They must be in BEFORE
        // resetting timeline_finished to avoid a race where the effects loop
        // sees empty playbacks + no timeline and sets finished=true. The
        // events run on delayed song time, as the DSL timeline starts from:
        // those up to the start point are skipped, and the state they built
        // up put back as the start clears the store (#583).
        let mut midi_dmx_playbacks = Vec::new();
        for (universe_name, (midi_sheet, channels)) in dmx_midi_sheets.drain() {
            let Some(&universe_id) = dmx_engine.universe_name_to_id.get(&universe_name) else {
                continue;
            };
            midi_dmx_playbacks.push(MidiDmxPlayback {
                precomputed: midi_sheet.precomputed,
                cursor: 0,
                universe_id,
                midi_channels: HashSet::from_iter(channels),
                start_dim_rate: dmx_engine
                    .universes
                    .get(&universe_id)
                    .map_or(crate::dmx::universe::DEFAULT_DIM_TICKS, |universe| {
                        universe.dim_rate()
                    }),
            });
        }
        let has_midi_dmx = !midi_dmx_playbacks.is_empty();

        // Start the lighting timeline at the specified time
        // The song time goes to the start time inside the start, before its
        // timeline arms, so the effects loop uses the right time from there.
        dmx_engine.start_playback_lighting(
            start_time,
            super::timeline::MidiDmxStart::Install(midi_dmx_playbacks),
        );

        // Start file watcher for hot-reload if broadcast channel is available.
        {
            let broadcast_tx = dmx_engine.broadcast_tx.lock();
            if let Some(tx) = broadcast_tx.as_ref() {
                let file_paths: Vec<std::path::PathBuf> = dsl_lighting_shows
                    .iter()
                    .map(|s| s.file_path().to_path_buf())
                    .collect();
                if !file_paths.is_empty() {
                    match crate::dmx::watcher::start_watching(
                        Arc::downgrade(&dmx_engine),
                        watcher_generation,
                        file_paths,
                        song.lighting_tempo_map(),
                        tx.clone(),
                    ) {
                        Ok(handle) => {
                            *dmx_engine.watcher_handle.lock() = Some(handle);
                        }
                        Err(e) => {
                            warn!("Failed to start light show file watcher: {}", e);
                        }
                    }
                }
            }
        }

        // Note: Effects loop is now persistent and started in Engine::new()

        if !has_midi_dmx && !has_lighting {
            info!(song = song.name(), "Song has no matching light shows.");
            // The `!has_lighting` branch above already dropped the previous
            // song's timeline and tempo map.
            ready_tx.send();
            return Ok(());
        }

        // Reset timeline finished flag for new song AFTER populating playbacks.
        // This must be set to false before starting the song time tracker, since the
        // tracker exits when timeline_finished is true.
        dmx_engine.timeline_finished.store(false, Ordering::Relaxed);

        // Start song time tracking (per-song, tracks elapsed time).
        // Must start AFTER timeline_finished is reset, otherwise the tracker
        // sees true from the previous song and exits immediately.
        // Flag set by the section loop thread when it takes over song time writes.
        // Until set, the song time tracker writes normally.
        let section_owns_time = Arc::new(AtomicBool::new(false));

        let song_time_tracker = Self::start_song_time_tracker_with_section(
            dmx_engine.clone(),
            cancel_handle.clone(),
            start_time,
            clock.clone(),
            Some(section_owns_time.clone()),
            generation,
        );

        // Store the cancel handle so the effects loop can notify when everything finishes
        {
            let mut handle = dmx_engine.timeline_cancel_handle.lock();
            *handle = Some(cancel_handle.clone());
        }

        // Signal readiness — all setup is complete.
        ready_tx.send();

        // Wait for the clock to start (the "go" signal from play_files):
        // blocked until it is started, then spinning only until it reads
        // past zero (an audio clock's first buffer), not for the whole of
        // the other subsystems' startup.
        clock.wait_for_start_or_cancel(&cancel_handle);
        while clock.elapsed() == Duration::ZERO {
            if cancel_handle.is_cancelled() {
                dmx_engine.timeline_finished.store(true, Ordering::Relaxed);
                if let Err(e) = song_time_tracker.join() {
                    error!("Error waiting for song time tracker to stop: {:?}", e);
                }
                return Ok(());
            }
            std::hint::spin_loop();
        }

        // Loop jumps: the lights go back where and when the playback's jumps
        // go -- committed by the audio, or the playback's own publisher
        // without it -- sections and a looping song's passes alike, and play
        // on from the song position the jumps leave: the playback time less
        // what they have put back. Once one has, this writes the song time in
        // place of the tracker, whose time leaves the loops out. A jump with
        // no overlap restarts the timeline at its target; a pass with one
        // (a `loop_crossfade`) crosses the lights over from the outgoing pass
        // to the new one over the overlap (`begin_loop_crossfade`).
        //
        // It follows the log until the playback is over, past the show's
        // last cue: a section looping there, or armed then, re-cues it each
        // time round (#606).
        let follow_done = Arc::new(AtomicBool::new(false));
        let stop_following = Arc::new(AtomicBool::new(false));
        let section_loop_thread = {
            let cancel_handle = cancel_handle.clone();
            let section_owns_time = section_owns_time.clone();
            let dmx_engine = dmx_engine.clone();
            let clock = clock.clone();
            let song = song.clone();
            let follow_done = follow_done.clone();
            let stop_following = stop_following.clone();
            let mut jumps = crate::playsync::JumpCursor::new(section_jumps.clone());
            thread::spawn(move || {
                // Only while this playback has the lights: one started since
                // (a song that follows before this one's threads are all
                // down) owns the timeline and the song time.
                let ours = || dmx_engine.playback_generation() == generation;
                // The timeline of the pass a crossfading jump starts, built
                // on a thread of its own as soon as the jump is committed,
                // and taken up as it falls due: building it may stat and
                // parse the song's show files again, and copies their
                // shows, which is not to be done at the seam.
                let mut next_pass = NextPassTimeline::default();
                while !cancel_handle.is_cancelled() && !stop_following.load(Ordering::Relaxed) {
                    // Closed first: a jump committed before it closed is then
                    // seen by the peek below.
                    let closed = jumps.log().is_closed();
                    // Section bounds are song-absolute; the clock counts from
                    // this playback's start, so offset by the start position.
                    let elapsed = start_time + clock.elapsed();
                    if !ours() {
                        break;
                    }
                    while let Some(jump) = jumps.due(elapsed).filter(|_| ours()) {
                        let position = jumps.song_position(elapsed);
                        info!(
                            from = ?jump.from,
                            to = ?jump.to,
                            overlap = ?jump.overlap,
                            "DMX loop: going back"
                        );
                        // Rewind the clock and take ownership of it *before*
                        // arming. Arming first leaves a window in which the
                        // song-time tracker is still writing end-of-section
                        // time, and the 44Hz effects loop can advance the
                        // freshly rewound timeline straight past every cue
                        // in the section -- silencing the loop for good,
                        // since the cue pointer never moves backwards.
                        section_owns_time.store(true, Ordering::Relaxed);
                        if jump.overlap.is_zero() {
                            // The restart clears the MIDI DMX store: what the
                            // events before the section set goes back with it
                            // (#583). A crossfade still crossing out carries
                            // on from where it is, before the song time goes
                            // back under it.
                            dmx_engine.keep_crossfade_on_own_time();
                            dmx_engine.restart_section_rewinding(position, &|| {
                                dmx_engine.update_song_time_for_generation(position, generation)
                            });
                        } else {
                            // The outgoing pass is `from - to` ahead of the
                            // new one in song time, and fades out over the
                            // overlap as the new one fades in. The store
                            // carries the last pass's values into the
                            // crossfade, MIDI DMX joining the new pass.
                            dmx_engine.begin_loop_crossfade_with(
                                next_pass.take(&song, &jump),
                                jump.from,
                                jump.to,
                                jump.overlap,
                                super::timeline::MidiDmxStart::Join,
                            );
                            dmx_engine.update_song_time_for_generation(position, generation);
                        }
                        // A show restarted runs again, however long ago its
                        // last cue went: marked under the timeline's lock,
                        // after the restart, so the effects loop cannot mark
                        // it over on the strength of the show before.
                        {
                            let _timeline = dmx_engine.current_song_timeline.lock();
                            dmx_engine.timeline_finished.store(false, Ordering::Relaxed);
                        }
                    }
                    let position = jumps.song_position(elapsed);
                    if let Some(next) = jumps.peek() {
                        if !next.overlap.is_zero() {
                            next_pass.build(&song, &next);
                        }
                        // A jump committed and coming: this takes the time
                        // from the tracker now, not once the jump falls due.
                        // Left to the tracker until then, the time went on
                        // past the section's end, and a cue on it played for
                        // a frame before the restart.
                        section_owns_time.store(true, Ordering::Relaxed);
                        dmx_engine
                            .update_song_time_for_generation(position.min(next.from), generation);
                    } else {
                        if !jumps.consumed().is_zero() {
                            dmx_engine.update_song_time_for_generation(position, generation);
                        }
                        if closed {
                            follow_done.store(true, Ordering::Relaxed);
                        }
                    }
                    thread::sleep(Duration::from_millis(10));
                }
                // Superseded, it has no more to follow.
                if !ours() {
                    follow_done.store(true, Ordering::Relaxed);
                }
            })
        };

        // Wait for the show to finish and no jump to be left to take it back
        // (the log closed: the audio, or its stand-in, has stopped), or for
        // the playback to be cancelled or its loop broken. A song without
        // loops ends as it did: its show's end comes before, or with, the end
        // of its audio, which closes the log as it stops. The timeline is
        // watched with a heartbeat, so a dead effects loop is recovered from,
        // each time the show is (re)started.
        let mut timeline_watcher: Option<JoinHandle<()>> = None;
        loop {
            if cancel_handle.is_cancelled() || loop_break.load(Ordering::Relaxed) {
                break;
            }
            let finished = dmx_engine.timeline_finished.load(Ordering::Relaxed);
            if finished && follow_done.load(Ordering::Relaxed) {
                break;
            }
            if timeline_watcher.as_ref().is_some_and(|w| w.is_finished()) {
                if let Some(Err(e)) = timeline_watcher.take().map(JoinHandle::join) {
                    error!("Error while joining timeline watcher thread: {:?}", e);
                }
            }
            if !finished && timeline_watcher.is_none() {
                let cancel_handle = cancel_handle.clone();
                let timeline_finished = dmx_engine.timeline_finished.clone();
                let heartbeat = dmx_engine.effects_loop_heartbeat.clone();
                let phase = dmx_engine.effects_loop_phase.clone();
                let subphase = dmx_engine.update_subphase.clone();
                timeline_watcher = Some(thread::spawn(move || {
                    Self::wait_for_timeline_with_heartbeat(
                        &cancel_handle,
                        timeline_finished,
                        &heartbeat,
                        &phase,
                        &subphase,
                    );
                }));
            }
            thread::sleep(Duration::from_millis(10));
        }

        // Song playback finished - signal the song time tracker, the
        // timeline watcher and the jump follower to stop.
        stop_following.store(true, Ordering::Relaxed);
        dmx_engine.timeline_finished.store(true, Ordering::Relaxed);
        cancel_handle.notify();
        if let Some(watcher) = timeline_watcher {
            if let Err(e) = watcher.join() {
                error!("Error while joining timeline watcher thread: {:?}", e);
            }
        }

        if let Err(e) = song_time_tracker.join() {
            error!("Error waiting for song time tracker to stop: {:?}", e);
        }

        if let Err(e) = section_loop_thread.join() {
            error!("Error joining section loop thread: {:?}", e);
        }

        // Final cleanup.
        dmx_engine.release_song(Release::Ended);

        info!("DMX playback stopped.");

        Ok(())
    }

    /// Lets go of what a song left driving the lights, as far as `scope`
    /// says (see [`Release`]).
    pub(super) fn release_song(&self, scope: Release) {
        let rebuilding = self.rebuilding();
        self.release_song_rebuilding(&rebuilding, scope);
    }

    /// [`Self::release_song`] under the rebuild lock, held already: a show
    /// rebuild under way finishes first, and none starts until the song is
    /// let go.
    pub(super) fn release_song_rebuilding(
        &self,
        rebuilding: &super::timeline::Rebuilding,
        scope: Release,
    ) {
        // What writes the rig on its own goes first, so nothing it writes
        // after relights what fades: MIDI DMX playback, and -- the song done
        // with -- the show file watcher, whose reload would put its show back.
        // A playback's end leaves the watcher: a live edit still rebuilds
        // the stopped show.
        self.midi_dmx_playbacks.lock().clear();
        if scope != Release::Ended {
            self.retire_watcher_rebuilding(rebuilding);
        }
        match scope {
            Release::Ended => self.stop_lighting_timeline(),
            Release::Replaced => *self.current_song_timeline.lock() = None,
            Release::Dark { fade, floor } => {
                // Nothing lit, nothing to fade: a fade over nothing would
                // only cross the next song in from black.
                let floor = if floor {
                    self.dark_floor(fade)
                } else {
                    Vec::new()
                };
                let lit = !self.effect_engine.lock().last_commands().is_empty();
                if lit || !floor.is_empty() {
                    self.begin_break_fade_over(rebuilding, fade, floor);
                }
            }
        }
        // The tempo last: a fade just begun keeps the song's in its engine.
        if scope != Release::Ended {
            self.effect_engine.lock().set_tempo_map(None);
        }
    }

    /// Lets the show file watcher go: dropped, and any reload it has yet
    /// to make refused, edits it took in before it went included. Returns
    /// the generation a watcher started now reloads under.
    pub(in crate::dmx) fn retire_watcher(&self) -> u64 {
        let rebuilding = self.rebuilding();
        self.retire_watcher_rebuilding(&rebuilding)
    }

    /// [`Self::retire_watcher`] under the rebuild lock, held already, which
    /// a reload checks the generation under before it rebuilds.
    fn retire_watcher_rebuilding(&self, _rebuilding: &super::timeline::Rebuilding) -> u64 {
        let generation = self.watcher_generation.fetch_add(1, Ordering::SeqCst) + 1;
        self.watcher_handle.lock().take();
        generation
    }

    /// Waits for the lighting timeline to finish, with a heartbeat check to detect
    /// a dead effects loop. If the heartbeat stops advancing for 10 seconds, the
    /// effects loop is assumed dead and the wait is abandoned so `Engine::play()`
    /// can clean up instead of blocking forever.
    pub(super) fn wait_for_timeline_with_heartbeat(
        cancel_handle: &CancelHandle,
        timeline_finished: Arc<AtomicBool>,
        heartbeat: &AtomicU64,
        phase: &AtomicU64,
        update_subphase: &AtomicU64,
    ) {
        // Check every 5 seconds; declare dead after 2 consecutive stale checks (10s).
        const CHECK_INTERVAL: Duration = Duration::from_secs(5);
        const MAX_STALE_CHECKS: u32 = 2;

        let mut last_heartbeat = heartbeat.load(Ordering::Relaxed);
        let mut stale_count: u32 = 0;

        loop {
            if cancel_handle.wait_with_timeout(timeline_finished.clone(), CHECK_INTERVAL) {
                // Condition met (cancelled or timeline finished).
                return;
            }

            // Timed out — check if the effects loop is still alive.
            let current_heartbeat = heartbeat.load(Ordering::Relaxed);
            if current_heartbeat == last_heartbeat {
                stale_count += 1;
                let current_phase = phase.load(Ordering::Relaxed);
                let phase_name = match current_phase {
                    0 => "idle",
                    1 => "midi_dmx_store_tick",
                    2 => "midi_advance",
                    3 => "update_effects",
                    4 => "update_song_lighting",
                    5 => "timeline_finished_check",
                    _ => "unknown",
                };
                let current_subphase = update_subphase.load(Ordering::Relaxed);
                let subphase_name = match current_subphase {
                    0 => "idle",
                    1 => "get_song_time",
                    2 => "acquire_effect_lock",
                    10 => "fast_path_check",
                    20 => "state_setup",
                    30 => "midi_dmx_inject",
                    40 => "effect_sort",
                    50 => "effect_process",
                    60 => "completed_effects",
                    70 => "persist_state",
                    80 => "dmx_generate",
                    _ => "unknown",
                };
                if stale_count >= MAX_STALE_CHECKS {
                    error!(
                        phase = phase_name,
                        update_subphase = subphase_name,
                        "Effects loop heartbeat stale for {}s — assuming dead. \
                         Forcing timeline_finished to unblock DMX playback.",
                        CHECK_INTERVAL.as_secs() * u64::from(MAX_STALE_CHECKS),
                    );
                    timeline_finished.store(true, Ordering::Relaxed);
                    return;
                }
                warn!(
                    stale_count,
                    phase = phase_name,
                    update_subphase = subphase_name,
                    "Effects loop heartbeat stale — will force-finish if it persists."
                );
            } else {
                // Heartbeat is advancing; reset stale counter.
                stale_count = 0;
                last_heartbeat = current_heartbeat;
            }
        }
    }

    /// Starts a thread to track song time from a specific start time.
    ///
    /// Claims a fresh playback generation, so any tracker still running from a
    /// previous playback stops being able to write.
    pub fn start_song_time_tracker_from(
        dmx_engine: Arc<Engine>,
        cancel_handle: CancelHandle,
        start_offset: Duration,
        clock: crate::clock::PlaybackClock,
    ) -> JoinHandle<()> {
        let generation = dmx_engine.begin_playback_generation();
        Self::start_song_time_tracker_with_section(
            dmx_engine,
            cancel_handle,
            start_offset,
            clock,
            None,
            generation,
        )
    }

    fn start_song_time_tracker_with_section(
        dmx_engine: Arc<Engine>,
        cancel_handle: CancelHandle,
        start_offset: Duration,
        clock: crate::clock::PlaybackClock,
        section_owns_time: Option<Arc<AtomicBool>>,
        generation: u64,
    ) -> JoinHandle<()> {
        let timeline_finished = dmx_engine.timeline_finished.clone();
        thread::spawn(move || {
            while !cancel_handle.is_cancelled() && !timeline_finished.load(Ordering::Relaxed) {
                // Skip writing when the section loop thread has taken over
                // song time updates (set when the first section loop triggers).
                let section_writing = section_owns_time
                    .as_ref()
                    .map(|f| f.load(Ordering::Relaxed))
                    .unwrap_or(false);

                if !section_writing {
                    let elapsed = clock.elapsed();
                    let song_time = start_offset + elapsed;
                    dmx_engine.update_song_time_for_generation(song_time, generation);
                }

                thread::sleep(Duration::from_millis(10));
            }
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn song_with_show(dir: &std::path::Path) -> Result<Arc<Song>, Box<dyn Error>> {
        let dsl_path = dir.join("show.dsl");
        std::fs::write(
            &dsl_path,
            r#"show "long" {
    @00:00.000
    front_wash: static color: "blue", duration: 30s, dimmer: 100%
}"#,
        )?;
        let song_config = crate::config::Song::new(
            "Built ahead",
            None,
            None,
            None,
            None,
            Some(vec![crate::config::LightingShow::new(
                dsl_path.to_string_lossy().into_owned(),
            )]),
            vec![],
            HashMap::new(),
            Vec::new(),
        );
        Ok(Arc::new(Song::new(dir, &song_config)?))
    }

    fn pass(at_ms: u64) -> crate::section_loop::SectionJump {
        crate::section_loop::SectionJump {
            at: Duration::from_millis(at_ms),
            from: Duration::from_millis(at_ms),
            to: Duration::ZERO,
            overlap: Duration::from_secs(4),
            pass: true,
        }
    }

    /// The next pass's timeline is the one built ahead for its jump, and
    /// built at the jump for one it was not built for, as it always was.
    #[test]
    fn the_next_pass_timeline_is_built_ahead_for_its_jump() -> Result<(), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        let song = song_with_show(dir.path())?;
        let mut next = NextPassTimeline::default();
        next.build(&song, &pass(96_000));
        // Asked again, the build under way is kept.
        next.build(&song, &pass(96_000));
        assert!(next.building.is_some());
        assert!(next.take(&song, &pass(96_000)).is_some());
        assert!(next.building.is_none());
        // Built for another jump: that build is dropped, and this one's
        // timeline built now.
        next.build(&song, &pass(192_000));
        assert!(next.take(&song, &pass(288_000)).is_some());
        assert!(next.building.is_none());
        Ok(())
    }
}
