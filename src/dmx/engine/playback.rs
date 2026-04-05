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
use crate::midi::playback::PrecomputedMidi;

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
            active_section,
            section_loop_break,
            ..
        } = loop_control;
        let span = span!(Level::INFO, "play song (dmx)");
        let _enter = span.enter();

        // Check if there are any lighting systems to play
        let light_shows = song.light_shows();
        let dsl_lighting_shows = song.dsl_lighting_shows();
        let has_lighting = !dsl_lighting_shows.is_empty();

        if light_shows.is_empty() && !has_lighting {
            ready_tx.send();
            return Ok(());
        }

        info!(
            song = song.name(),
            duration = song.duration_string(),
            "Playing song DMX."
        );

        // Register fixtures with the effects engine if lighting system is available
        if let Err(_e) = dmx_engine.register_venue_fixtures_safe() {
            // Failed to register venue fixtures, continue without them
        }

        // Setup song lighting if available - work directly with DSL shows
        if has_lighting {
            info!(
                "Setup lighting timeline with {} DSL light shows",
                dsl_lighting_shows.len()
            );

            // Collect cached shows from DSL lighting shows
            let all_shows: Vec<_> = dsl_lighting_shows
                .iter()
                .flat_map(|dsl_show| dsl_show.shows().values().cloned())
                .collect();

            if !all_shows.is_empty() {
                let timeline = crate::lighting::timeline::LightingTimeline::new(all_shows);
                // Set or clear the tempo map — a song without a tempo block must not
                // inherit one from the previous song.
                {
                    let mut effect_engine = dmx_engine.effect_engine.lock();
                    effect_engine.set_tempo_map(timeline.tempo_map().cloned());
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

        // Reset song time to start time for new song BEFORE starting timeline
        // This ensures the effects loop uses the correct time when updating
        dmx_engine.update_song_time(start_time);

        // Start the lighting timeline at the specified time
        dmx_engine.start_lighting_timeline_at(start_time);

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
                        dmx_engine.lighting_system.clone(),
                        dmx_engine.lighting_config.clone(),
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

        if dmx_midi_sheets.is_empty() && !has_lighting {
            info!(song = song.name(), "Song has no matching light shows.");
            ready_tx.send();
            return Ok(());
        }

        // Build MIDI DMX playbacks and store them for effects-loop dispatch.
        // Drain the map to take ownership of MidiSheets (avoids cloning event vecs).
        // This must happen BEFORE resetting timeline_finished to avoid a race where
        // the effects loop sees empty playbacks + no timeline and sets finished=true.
        {
            let mut playbacks = dmx_engine.midi_dmx_playbacks.lock();
            playbacks.clear();
            for (universe_name, (midi_sheet, channels)) in dmx_midi_sheets.drain() {
                let midi_channels = HashSet::from_iter(channels);
                let universe_id = match dmx_engine.universe_name_to_id.get(&universe_name) {
                    Some(&id) => id,
                    None => continue,
                };
                let events = midi_sheet.precomputed.into_events();
                // Seek cursor past start_time
                let cursor = events.partition_point(|e| e.time < start_time);
                playbacks.push(MidiDmxPlayback {
                    precomputed: PrecomputedMidi::from_events(events),
                    cursor,
                    universe_id,
                    midi_channels,
                });
            }
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

        // Section loop: continuously update song time to wrap within section bounds.
        // Runs alongside the timeline watcher and song time tracker. When active,
        // it overrides the song time tracker's writes with the wrapped time.
        let section_loop_thread = {
            let cancel_handle = cancel_handle.clone();
            let active_section = active_section.clone();
            let section_loop_break = section_loop_break.clone();
            let section_owns_time = section_owns_time.clone();
            let dmx_engine = dmx_engine.clone();
            let clock = clock.clone();
            let timeline_finished = dmx_engine.timeline_finished.clone();
            thread::spawn(move || {
                let mut section_monitor = crate::section_loop::SectionLoopMonitor::new();
                let mut iteration_start: Option<Duration> = None;
                // After loop break: (resume_time, clock_at_break). The thread
                // keeps writing song time advancing from the resume point.
                let mut continue_from: Option<(Duration, Duration)> = None;

                loop {
                    if cancel_handle.is_cancelled() || timeline_finished.load(Ordering::Relaxed) {
                        break;
                    }

                    // Post-break: advance song time from resume point.
                    if let Some((resume_time, break_clock)) = continue_from {
                        let since_break = clock.elapsed().saturating_sub(break_clock);
                        dmx_engine.update_song_time(resume_time + since_break);
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }

                    // Check for loop break first (before reading active_section,
                    // which may already be cleared by stop_section_loop).
                    if section_loop_break.load(Ordering::Relaxed) {
                        if let Some(section) = section_monitor.cached_section().cloned() {
                            let elapsed = clock.elapsed();
                            let current_pos = if let Some(iter_start) = iteration_start {
                                let time_since = elapsed.saturating_sub(iter_start);
                                let sd = section.end_time.saturating_sub(section.start_time);
                                section.start_time + time_since.min(sd)
                            } else {
                                section.end_time
                            };
                            info!(
                                position = ?current_pos,
                                "DMX section loop: breaking, continuing from current position"
                            );
                            dmx_engine.update_song_time(current_pos);
                            dmx_engine.start_lighting_timeline_at(current_pos);
                            {
                                let mut playbacks = dmx_engine.midi_dmx_playbacks.lock();
                                for playback in playbacks.iter_mut() {
                                    let events = playback.precomputed.events();
                                    playback.cursor =
                                        events.partition_point(|e| e.time < current_pos);
                                }
                            }
                            continue_from = Some((current_pos, elapsed));
                        } else {
                            // No cached section — just hand back to tracker.
                            section_owns_time.store(false, Ordering::Relaxed);
                        }
                        thread::sleep(Duration::from_millis(10));
                        continue;
                    }

                    let elapsed = clock.elapsed();
                    match section_monitor.poll(&active_section, elapsed) {
                        crate::section_loop::LoopPoll::Waiting(ref section) => {
                            let section_duration =
                                section.end_time.saturating_sub(section.start_time);
                            if section_duration.is_zero() {
                                break;
                            }
                            if let Some(iter_start) = iteration_start {
                                let time_since = elapsed.saturating_sub(iter_start);
                                let position = time_since.min(section_duration);
                                dmx_engine.update_song_time(section.start_time + position);
                            }
                        }
                        crate::section_loop::LoopPoll::Triggered(ref section) => {
                            info!(
                                section = section.name,
                                "DMX section loop: resetting for next iteration"
                            );
                            dmx_engine.start_lighting_timeline_at(section.start_time);
                            {
                                let mut playbacks = dmx_engine.midi_dmx_playbacks.lock();
                                for playback in playbacks.iter_mut() {
                                    let events = playback.precomputed.events();
                                    playback.cursor =
                                        events.partition_point(|e| e.time < section.start_time);
                                }
                            }
                            dmx_engine.update_song_time(section.start_time);
                            section_owns_time.store(true, Ordering::Relaxed);
                            iteration_start = Some(elapsed);
                        }
                        crate::section_loop::LoopPoll::SectionCleared => {
                            iteration_start = None;
                            section_owns_time.store(false, Ordering::Relaxed);
                        }
                        crate::section_loop::LoopPoll::NoSection => {
                            iteration_start = None;
                            section_owns_time.store(false, Ordering::Relaxed);
                        }
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

        if let Err(e) = song_time_tracker.join() {
            error!("Error waiting for song time tracker to stop: {:?}", e);
        }

        // Loop if the song has loop_playback enabled.
        while song.loop_playback()
            && !cancel_handle.is_cancelled()
            && !loop_break.load(Ordering::Relaxed)
        {
            info!(
                song = song.name(),
                "DMX loop: restarting timeline from beginning"
            );

            // Reset state for new loop iteration.
            dmx_engine.update_song_time(Duration::ZERO);
            dmx_engine.start_lighting_timeline_at(Duration::ZERO);

            // Reset MIDI DMX playback cursors to the beginning.
            {
                let mut playbacks = dmx_engine.midi_dmx_playbacks.lock();
                for playback in playbacks.iter_mut() {
                    playback.cursor = 0;
                }
            }

            dmx_engine.timeline_finished.store(false, Ordering::Relaxed);

            // Start a new song time tracker for this loop iteration.
            let loop_time_tracker = Self::start_song_time_tracker_from(
                dmx_engine.clone(),
                cancel_handle.clone(),
                Duration::ZERO,
                clock.clone(),
            );

            // Wait for timeline to finish again.
            let loop_watcher = {
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

            if let Err(e) = loop_watcher.join() {
                error!("Error while joining loop timeline watcher: {:?}", e);
            }

            dmx_engine.timeline_finished.store(true, Ordering::Relaxed);

            if let Err(e) = loop_time_tracker.join() {
                error!("Error waiting for loop time tracker to stop: {:?}", e);
            }
        }

        // Stop section loop thread.
        section_loop_break.store(true, Ordering::Relaxed);
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

    /// Starts a thread to track song time from a specific start time
    pub fn start_song_time_tracker_from(
        dmx_engine: Arc<Engine>,
        cancel_handle: CancelHandle,
        start_offset: Duration,
        clock: crate::clock::PlaybackClock,
    ) -> JoinHandle<()> {
        Self::start_song_time_tracker_with_section(
            dmx_engine,
            cancel_handle,
            start_offset,
            clock,
            None,
        )
    }

    fn start_song_time_tracker_with_section(
        dmx_engine: Arc<Engine>,
        cancel_handle: CancelHandle,
        start_offset: Duration,
        clock: crate::clock::PlaybackClock,
        section_owns_time: Option<Arc<AtomicBool>>,
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
                    dmx_engine.update_song_time(song_time);
                }

                thread::sleep(Duration::from_millis(10));
            }
        })
    }
}
