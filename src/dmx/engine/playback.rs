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
            {
                let mut effect_engine = dmx_engine.effect_engine.lock();
                effect_engine.set_tempo_map(None);
            }
            {
                let mut current_timeline = dmx_engine.current_song_timeline.lock();
                *current_timeline = None;
            }
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
                    effect_engine.set_tempo_map(
                        timeline
                            .tempo_map()
                            .cloned()
                            .or_else(|| song.lighting_tempo_map()),
                    );
                }
                {
                    let mut current_timeline = dmx_engine.current_song_timeline.lock();
                    *current_timeline = Some(timeline);
                }
            }
        } else {
            // Clear lighting state from previous song so MIDI DMX songs
            // don't inherit a stale tempo map or timeline.
            {
                let mut effect_engine = dmx_engine.effect_engine.lock();
                effect_engine.set_tempo_map(None);
            }
            {
                let mut current_timeline = dmx_engine.current_song_timeline.lock();
                *current_timeline = None;
            }
        }

        // Claim this playback's generation before anything writes song time.
        // The outgoing playback's tracker may still be awake for up to its
        // sleep interval, carrying the previous start offset; from here its
        // writes are dropped rather than landing on our freshly armed timeline.
        let generation = dmx_engine.begin_playback_generation();

        // Reset song time to start time for new song BEFORE starting timeline
        // This ensures the effects loop uses the correct time when updating
        dmx_engine.update_song_time(start_time);

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
                    .map_or(1.0, |universe| universe.dim_rate()),
            });
        }
        let has_midi_dmx = !midi_dmx_playbacks.is_empty();

        // Start the lighting timeline at the specified time
        dmx_engine.start_lighting_timeline_with(
            start_time,
            super::timeline::MidiDmxStart::Install(midi_dmx_playbacks),
        );

        // Start file watcher for hot-reload if broadcast channel is available
        {
            let broadcast_tx = dmx_engine.broadcast_tx.lock();
            if let Some(tx) = broadcast_tx.as_ref() {
                let file_paths: Vec<std::path::PathBuf> = dsl_lighting_shows
                    .iter()
                    .map(|s| s.file_path().to_path_buf())
                    .collect();
                if !file_paths.is_empty() {
                    match crate::dmx::watcher::start_watching(
                        file_paths,
                        dmx_engine.effect_engine.clone(),
                        dmx_engine.current_song_timeline.clone(),
                        dmx_engine.current_song_time.clone(),
                        dmx_engine.playback_delay,
                        dmx_engine.lighting_system.clone(),
                        dmx_engine.lighting_config.clone(),
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

        // Wait for the clock to start (the "go" signal from play_files).
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

        // Wait for the timeline to finish, using a heartbeat-aware loop that
        // can recover if the effects loop dies.
        let timeline_watcher = {
            let cancel_handle = cancel_handle.clone();
            let timeline_finished = dmx_engine.timeline_finished.clone();
            let heartbeat = dmx_engine.effects_loop_heartbeat.clone();
            let phase = dmx_engine.effects_loop_phase.clone();
            let subphase = dmx_engine.update_subphase.clone();
            thread::spawn(move || {
                Self::wait_for_timeline_with_heartbeat(
                    &cancel_handle,
                    timeline_finished,
                    &heartbeat,
                    &phase,
                    &subphase,
                );
            })
        };

        // A song looping with a crossfade starts its second pass the overlap
        // before the first ends, cues left in the first or not: those play
        // on in the outgoing pass. So the first pass's wait ends at that
        // boundary at the latest, as it ends when the cues run out.
        let first_boundary_thread = song.loop_crossfade().map(|_| {
            let cancel_handle = cancel_handle.clone();
            let timeline_finished = dmx_engine.timeline_finished.clone();
            let clock = clock.clone();
            let boundary = song.loop_pass_start(start_time, 1);
            thread::spawn(move || {
                while clock.elapsed() < boundary {
                    if cancel_handle.is_cancelled() || timeline_finished.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(2));
                }
                timeline_finished.store(true, Ordering::Relaxed);
                cancel_handle.notify();
            })
        });

        // Section loop: the lights go back where and when the playback's
        // section loop jumps go -- committed by the audio, or the playback's
        // own publisher without it -- and play on from the song position the
        // jumps leave: the playback time less what they have put back. Once
        // one has, this writes the song time in place of the tracker, whose
        // time leaves the loops out.
        let section_loop_thread = {
            let cancel_handle = cancel_handle.clone();
            let section_owns_time = section_owns_time.clone();
            let dmx_engine = dmx_engine.clone();
            let clock = clock.clone();
            let timeline_finished = dmx_engine.timeline_finished.clone();
            let mut jumps = crate::playsync::JumpCursor::new(section_jumps.clone());
            thread::spawn(move || {
                while !cancel_handle.is_cancelled() && !timeline_finished.load(Ordering::Relaxed) {
                    // Section bounds are song-absolute; the clock counts from
                    // this playback's start, so offset by the start position.
                    let elapsed = start_time + clock.elapsed();
                    while let Some(jump) = jumps.due(elapsed) {
                        let position = elapsed.saturating_sub(jumps.consumed());
                        info!(
                            from = ?jump.from,
                            to = ?jump.to,
                            "DMX section loop: going back"
                        );
                        // Rewind the clock and take ownership of it *before*
                        // arming. Arming first leaves a window in which the
                        // song-time tracker is still writing end-of-section
                        // time, and the 44Hz effects loop can advance the
                        // freshly rewound timeline straight past every cue
                        // in the section -- silencing the loop for good,
                        // since the cue pointer never moves backwards.
                        section_owns_time.store(true, Ordering::Relaxed);
                        dmx_engine.update_song_time(position);
                        // The restart clears the MIDI DMX store: what the
                        // events before the section set goes back with it
                        // (#583).
                        dmx_engine.start_lighting_timeline_with(
                            position,
                            super::timeline::MidiDmxStart::Seek,
                        );
                    }
                    let position = elapsed.saturating_sub(jumps.consumed());
                    if let Some(next) = jumps.peek() {
                        // A jump committed and coming: the lights are held
                        // at its end until it falls due. Left to the tracker,
                        // the time went on past it, and a cue on the
                        // section's end played for a frame before the restart.
                        section_owns_time.store(true, Ordering::Relaxed);
                        dmx_engine.update_song_time(position.min(next.from));
                    } else if !jumps.consumed().is_zero() {
                        dmx_engine.update_song_time(position);
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            })
        };

        if let Err(e) = timeline_watcher.join() {
            error!("Error while joining timeline watcher thread: {:?}", e);
        }

        // Song playback finished - signal the song time tracker to stop.
        dmx_engine.timeline_finished.store(true, Ordering::Relaxed);
        if let Some(thread) = first_boundary_thread {
            let _ = thread.join();
        }

        if let Err(e) = song_time_tracker.join() {
            error!("Error waiting for song time tracker to stop: {:?}", e);
        }

        // Loop if the song has loop_playback enabled.
        //
        // The audio restarts its sources when they reach the end of the file,
        // so the timeline restarts on the same boundaries: the first pass ran
        // from `start_time` to the end, every later one is a whole song. The
        // shared clock keeps counting across passes, so each pass measures its
        // song time from its own boundary. Restarting when the *timeline*
        // finishes instead would restart the moment the last cue fires — for
        // a show whose cues are all near the top, immediately and forever.
        //
        // With a `loop_crossfade`, each pass begins that long before the last
        // ends, and the lights cross from one to the other over the overlap
        // (`begin_loop_crossfade`) instead of cutting.
        let song_length = song.duration();
        let crossfade = song.loop_crossfade();
        let mut pass = 1;
        let mut pass_start = song.loop_pass_start(start_time, pass);
        while song.loop_playback()
            && !song_length.is_zero()
            && !cancel_handle.is_cancelled()
            && !loop_break.load(Ordering::Relaxed)
        {
            // Wait for the audio to reach the boundary.
            while clock.elapsed() < pass_start {
                if cancel_handle.is_cancelled() || loop_break.load(Ordering::Relaxed) {
                    break;
                }
                thread::sleep(Duration::from_millis(2));
            }
            if cancel_handle.is_cancelled() || loop_break.load(Ordering::Relaxed) {
                break;
            }

            info!(
                song = song.name(),
                "DMX loop: restarting timeline from beginning"
            );

            // Reset state for new loop iteration.
            match crossfade {
                Some(overlap) => dmx_engine.begin_loop_crossfade(
                    super::timeline::song_lighting_timeline(&song),
                    song_length.saturating_sub(overlap),
                    overlap,
                ),
                None => {
                    dmx_engine.update_song_time(Duration::ZERO);
                    // MIDI DMX starts over with it, at the dim rate the
                    // playback started with, as a section loop does.
                    dmx_engine.start_lighting_timeline_with(
                        Duration::ZERO,
                        super::timeline::MidiDmxStart::Seek,
                    );
                }
            }
            if crossfade.is_some() {
                // The store carries the last pass's values into the
                // crossfade; MIDI DMX starts over from the beginning.
                dmx_engine.rewind_midi_dmx_playbacks();
            }

            dmx_engine.timeline_finished.store(false, Ordering::Relaxed);

            // Start a new song time tracker for this loop iteration, counting
            // from this pass's boundary.
            // A crossfading pass's song time also drives the lights' blend
            // between passes, so its tracker runs to the end of the pass even
            // once its cues are all done.
            let pass_over = Arc::new(AtomicBool::new(false));
            let loop_time_tracker = Self::start_loop_song_time_tracker(
                dmx_engine.clone(),
                cancel_handle.clone(),
                pass_start,
                clock.clone(),
                crossfade.is_some().then(|| pass_over.clone()),
            );

            // Hold this pass until the next boundary. The effects loop may flag
            // the timeline finished long before then, which only stops the time
            // tracker, exactly as it does on the first pass.
            pass += 1;
            let pass_end = song.loop_pass_start(start_time, pass);
            while clock.elapsed() < pass_end {
                if cancel_handle.is_cancelled() || loop_break.load(Ordering::Relaxed) {
                    break;
                }
                thread::sleep(Duration::from_millis(2));
            }

            dmx_engine.timeline_finished.store(true, Ordering::Relaxed);
            pass_over.store(true, Ordering::Relaxed);

            if let Err(e) = loop_time_tracker.join() {
                error!("Error waiting for loop time tracker to stop: {:?}", e);
            }

            pass_start = pass_end;
        }

        // The section loop thread stops with the timeline. (It used to be
        // stopped by setting the player's section loop break, which then
        // stayed set into the songs after.)
        if let Err(e) = section_loop_thread.join() {
            error!("Error joining section loop thread: {:?}", e);
        }

        // Final cleanup.
        dmx_engine.stop_lighting_timeline();
        dmx_engine.midi_dmx_playbacks.lock().clear();

        info!("DMX playback stopped.");

        Ok(())
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

    /// Tracks song time for one pass of a looping song: the shared clock keeps
    /// counting across passes, so song time is the clock less the pass's start.
    /// It stops when the pass's timeline finishes, or, given `until`, when
    /// that is set instead.
    fn start_loop_song_time_tracker(
        dmx_engine: Arc<Engine>,
        cancel_handle: CancelHandle,
        pass_start: Duration,
        clock: crate::clock::PlaybackClock,
        until: Option<Arc<AtomicBool>>,
    ) -> JoinHandle<()> {
        let generation = dmx_engine.begin_playback_generation();
        let stop = until.unwrap_or_else(|| dmx_engine.timeline_finished.clone());
        thread::spawn(move || {
            while !cancel_handle.is_cancelled() && !stop.load(Ordering::Relaxed) {
                let song_time = clock.elapsed().saturating_sub(pass_start);
                dmx_engine.update_song_time_for_generation(song_time, generation);
                thread::sleep(Duration::from_millis(10));
            }
        })
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
