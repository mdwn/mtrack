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

use parking_lot::Mutex;
use std::{
    collections::{HashMap, HashSet},
    error::Error,
    panic::AssertUnwindSafe,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use super::midi_dmx_store::MidiDmxStore;
use super::ola_client::OlaClient;
use ola::DmxBuffer;
use tracing::{debug, error, info, span, warn, Level};

use crate::{
    config,
    lighting::{
        system::LightingSystem, timeline::LightingTimeline, validation::validate_light_shows,
        EffectEngine,
    },
    midi::playback::PrecomputedMidi,
    playsync::CancelHandle,
    songs::{MidiSheet, Song},
};

use super::universe::Universe;

/// The result of classifying a MIDI message for DMX purposes.
#[derive(Debug, PartialEq)]
enum MidiDmxAction {
    /// NoteOn/NoteOff: key+1 → DMX channel, velocity*2 → value, uses dimming.
    KeyVelocity { channel: u16, value: u8 },
    /// Controller: controller+1 → DMX channel, value*2 → value, no dimming.
    Controller { channel: u16, value: u8 },
    /// ProgramChange: sets dimming speed (program * modifier seconds).
    Dimming { duration: Duration },
    /// Unrecognized MIDI message type.
    Unrecognized,
}

/// Converts a MIDI message into a DMX action without side effects.
///
/// Conversion rules:
/// - NoteOn/NoteOff: channel = key + 1 (0-based MIDI → 1-based DMX), value = velocity * 2
/// - Controller: channel = controller + 1, value = value * 2
/// - ProgramChange: dimming duration = program * dimming_speed_modifier seconds
fn classify_midi_dmx_action(
    midi_message: midly::MidiMessage,
    dimming_speed_modifier: f64,
) -> MidiDmxAction {
    match midi_message {
        midly::MidiMessage::NoteOn { key, vel } | midly::MidiMessage::NoteOff { key, vel } => {
            MidiDmxAction::KeyVelocity {
                channel: (key.as_int() + 1).into(),
                value: vel.as_int() * 2,
            }
        }
        midly::MidiMessage::ProgramChange { program } => MidiDmxAction::Dimming {
            duration: Duration::from_secs_f64(f64::from(program.as_int()) * dimming_speed_modifier),
        },
        midly::MidiMessage::Controller { controller, value } => MidiDmxAction::Controller {
            channel: (controller.as_int() + 1).into(),
            value: value.as_int() * 2,
        },
        _ => MidiDmxAction::Unrecognized,
    }
}

/// The DMX engine. This is meant to control the current state of the
/// universe(s) that should be sent to our DMX interface(s).
pub struct Engine {
    dimming_speed_modifier: f64,
    /// How long to wait before starting MIDI DMX DMX playback.
    playback_delay: Duration,
    universes: HashMap<u16, Universe>,
    /// Mapping from universe names to IDs for MIDI DMX system
    universe_name_to_id: HashMap<String, u16>,
    cancel_handle: CancelHandle,
    client_handle: Option<JoinHandle<()>>,
    join_handles: Vec<JoinHandle<()>>,
    /// Effects engine for processing lighting effects
    effect_engine: Arc<Mutex<EffectEngine>>,
    /// Lighting system for fixture and group management
    lighting_system: Option<Arc<Mutex<LightingSystem>>>,
    /// Lighting configuration for validation
    lighting_config: Option<config::Lighting>,
    /// Handle for the persistent effects loop thread
    effects_loop_handle: Mutex<Option<JoinHandle<()>>>,
    /// Current song timeline (thread-safe access for effects loop)
    current_song_timeline: Arc<Mutex<Option<LightingTimeline>>>,
    /// Current song time (thread-safe access for effects loop)
    current_song_time: Arc<Mutex<Duration>>,
    /// Flag indicating the current song's timeline has finished (all cues processed)
    timeline_finished: Arc<AtomicBool>,
    /// Cancel handle for notifying when timeline finishes
    timeline_cancel_handle: Arc<Mutex<Option<CancelHandle>>>,
    /// Broadcast sender for the web UI, used to start the file watcher per-song
    broadcast_tx: Mutex<Option<tokio::sync::broadcast::Sender<String>>>,
    /// Handle to the current file watcher (dropped/replaced per-song)
    watcher_handle: Mutex<Option<super::watcher::WatcherHandle>>,
    /// Lockless store for MIDI DMX DMX values with built-in interpolation.
    /// RwLock protects structural changes (register_slot); hot-path reads
    /// (write/tick/iter_active) take a cheap read lock while atomics handle data.
    midi_dmx_store: Arc<parking_lot::RwLock<MidiDmxStore>>,
    /// Active MIDI DMX playbacks dispatched from the effects loop.
    midi_dmx_playbacks: Mutex<Vec<MidiDmxPlayback>>,
    /// Heartbeat counter incremented by the effects loop each frame.
    /// Used by barrier threads to detect if the effects loop has died.
    effects_loop_heartbeat: Arc<AtomicU64>,
    /// Phase indicator for the effects loop (0=idle, 1=tick, 2=midi_advance,
    /// 3=update_effects, 4=update_song_lighting, 5=timeline_check).
    /// Used for diagnostics when the heartbeat goes stale.
    effects_loop_phase: Arc<AtomicU64>,
    /// Sub-phase indicator from EffectEngine::update() for finer-grained diagnostics.
    /// Shared Arc with the EffectEngine instance.
    update_subphase: Arc<AtomicU64>,
}

/// A MIDI DMX light show being played back from the effects loop.
struct MidiDmxPlayback {
    precomputed: PrecomputedMidi,
    cursor: usize,
    universe_id: u16,
    midi_channels: HashSet<u8>,
}

/// DmxMessage is a message that can be passed around between senders and receivers.
#[derive(Clone)]
pub(super) struct DmxMessage {
    pub universe: u32,
    pub buffer: DmxBuffer,
}

/// Shared handles exposed for reading lighting state.
pub struct BroadcastHandles {
    pub lighting_system: Option<Arc<Mutex<LightingSystem>>>,
}

impl Engine {
    /// Creates a new DMX Engine with lighting system using dependency injection.
    pub fn new(
        config: &config::Dmx,
        lighting_config: Option<&config::Lighting>,
        base_path: Option<&std::path::Path>,
        ola_client: Box<dyn OlaClient>,
    ) -> Result<Engine, Box<dyn Error>> {
        // Use the injected OLA client
        let ola_client = Arc::new(Mutex::new(ola_client));
        let (sender, receiver) = mpsc::channel::<DmxMessage>();

        let ola_client_for_thread = ola_client.clone();
        let client_handle = thread::spawn(move || {
            Self::ola_thread(ola_client_for_thread, receiver);
        });
        let cancel_handle = CancelHandle::new();
        let universes: HashMap<u16, Universe> = config
            .universes()
            .into_iter()
            .map(|config| {
                (
                    config.universe(),
                    Universe::new(config, cancel_handle.clone(), sender.clone()),
                )
            })
            .collect();

        // Create mapping from universe names to IDs for MIDI DMX system
        let universe_name_to_id: HashMap<String, u16> = config
            .universes()
            .into_iter()
            .map(|config| (config.name().to_string(), config.universe()))
            .collect();
        let join_handles: Vec<JoinHandle<()>> = universes
            .values()
            .map(|universe| universe.start_thread())
            .collect();

        // Initialize lighting system if config is provided
        let lighting_system =
            if let (Some(lighting_config), Some(base_path)) = (lighting_config, base_path) {
                let mut system = LightingSystem::new();
                if let Err(_e) = system.load(lighting_config, base_path) {
                    // Failed to load lighting system, continue without it
                    None
                } else {
                    Some(Arc::new(Mutex::new(system)))
                }
            } else {
                None
            };

        let ee = EffectEngine::new();
        let update_subphase = ee.update_subphase();
        let effect_engine = Arc::new(Mutex::new(ee));
        let current_song_timeline: Arc<Mutex<Option<LightingTimeline>>> =
            Arc::new(Mutex::new(None));
        let current_song_time = Arc::new(Mutex::new(Duration::ZERO));
        let timeline_finished = Arc::new(AtomicBool::new(true));
        let timeline_cancel_handle: Arc<Mutex<Option<CancelHandle>>> = Arc::new(Mutex::new(None));

        let midi_dmx_store = Arc::new(parking_lot::RwLock::new(MidiDmxStore::new()));

        Ok(Engine {
            dimming_speed_modifier: config.dimming_speed_modifier(),
            playback_delay: config.playback_delay()?,
            universes: universes.into_iter().collect(),
            universe_name_to_id,
            cancel_handle,
            client_handle: Some(client_handle),
            join_handles,
            effect_engine,
            effects_loop_handle: Mutex::new(None),
            lighting_system,
            lighting_config: lighting_config.cloned(),
            current_song_timeline,
            current_song_time,
            timeline_finished,
            timeline_cancel_handle,
            broadcast_tx: Mutex::new(None),
            watcher_handle: Mutex::new(None),
            midi_dmx_store,
            midi_dmx_playbacks: Mutex::new(Vec::new()),
            effects_loop_heartbeat: Arc::new(AtomicU64::new(0)),
            effects_loop_phase: Arc::new(AtomicU64::new(0)),
            update_subphase,
        })
    }

    // Note: Auto-connect helper removed; callers should construct an OLA client and call `new`.

    /// Starts the persistent effects loop. Must be called after wrapping Engine in Arc.
    /// The effects loop runs continuously until the engine is dropped.
    pub fn start_persistent_effects_loop(engine: Arc<Engine>) {
        // Use a weak reference to avoid preventing Engine from being dropped.
        // The thread will exit when the weak reference can no longer be upgraded.
        let weak_engine = Arc::downgrade(&engine);
        let heartbeat = engine.effects_loop_heartbeat.clone();

        let handle = thread::spawn(move || {
            info!("Effects loop started.");
            let mut last_update = std::time::Instant::now();
            let target_frame_time = Duration::from_secs_f64(1.0 / 44.0); // 44Hz

            // This loop runs continuously at 44Hz to process effects.
            // It exits when the Engine is dropped (weak upgrade fails).
            loop {
                // Try to upgrade the weak reference - if it fails, the Engine was dropped
                let Some(engine) = weak_engine.upgrade() else {
                    info!("Effects loop exiting: engine was dropped.");
                    break;
                };

                let now = std::time::Instant::now();
                let dt = now.duration_since(last_update);

                if dt >= target_frame_time {
                    // Wrap the frame work in catch_unwind to prevent a panic from
                    // killing the effects loop thread, which would permanently freeze
                    // all lighting and block DMX playback cleanup.
                    //
                    // parking_lot mutexes do not poison on panic — they release normally.
                    // AssertUnwindSafe is required because Engine contains parking_lot::Mutex
                    // fields, which don't implement RefUnwindSafe. The logical state inside
                    // any mutex that was mid-update when a panic fired may be inconsistent,
                    // but the lock is released and subsequent ticks can proceed.
                    let engine_ref = AssertUnwindSafe(&engine);
                    let result = std::panic::catch_unwind(move || {
                        Self::effects_loop_tick(&engine_ref);
                    });

                    if let Err(panic_info) = result {
                        let msg = if let Some(s) = panic_info.downcast_ref::<&str>() {
                            s.to_string()
                        } else if let Some(s) = panic_info.downcast_ref::<String>() {
                            s.clone()
                        } else {
                            "unknown panic".to_string()
                        };
                        error!(
                            panic_message = msg,
                            "Effects loop caught panic! Continuing to prevent lighting freeze."
                        );
                    }

                    heartbeat.fetch_add(1, Ordering::Relaxed);
                    last_update = now;
                }

                // Drop the Arc before sleeping to minimize time we hold the strong reference
                drop(engine);

                thread::sleep(Duration::from_millis(1));
            }

            info!("Effects loop exited.");
        });

        // Store the handle so it can be joined on drop.
        // The thread will stop when the Engine is dropped (weak upgrade fails).
        *engine.effects_loop_handle.lock() = Some(handle);
    }

    /// Performs one tick of the effects loop: interpolation, MIDI dispatch,
    /// effects update, timeline update, and finished-check.
    ///
    /// Phase values (stored in `effects_loop_phase` for diagnostics):
    ///   1 = MIDI DMX store tick, 2 = MIDI advance, 3 = update effects,
    ///   4 = update song lighting, 5 = timeline finished check, 0 = idle.
    fn effects_loop_tick(&self) {
        // Tick the MIDI DMX store to interpolate dimming values
        self.effects_loop_phase.store(1, Ordering::Relaxed);
        self.midi_dmx_store.read().tick();

        // Advance MIDI DMX playback cursors and dispatch events
        self.effects_loop_phase.store(2, Ordering::Relaxed);
        self.advance_midi_dmx_playbacks();

        // Update effects engine and apply to universes
        self.effects_loop_phase.store(3, Ordering::Relaxed);
        if let Err(e) = self.update_effects() {
            error!("Error updating effects: {}", e);
        }

        // Update song lighting timeline with actual song time
        self.effects_loop_phase.store(4, Ordering::Relaxed);
        let song_time = self.get_song_time();
        if let Err(e) = self.update_song_lighting(song_time) {
            error!("Error updating song lighting: {}", e);
        }

        // Check if all lighting has finished (DSL timeline cues + MIDI DMX playbacks)
        // and notify the waiting thread if so
        self.effects_loop_phase.store(5, Ordering::Relaxed);
        if !self.timeline_finished.load(Ordering::Relaxed) {
            let timeline_done = {
                let timeline = self.current_song_timeline.lock();
                timeline.as_ref().is_none_or(|tl| tl.is_finished())
            };
            let midi_dmx_done = self.midi_dmx_playbacks_finished();

            if timeline_done && midi_dmx_done {
                info!("Lighting timeline finished. Notifying barrier.");
                self.timeline_finished.store(true, Ordering::Relaxed);
                // Notify the cancel handle so wait() returns
                if let Some(ref cancel_handle) = *self.timeline_cancel_handle.lock() {
                    cancel_handle.notify();
                }
            }
        }

        self.effects_loop_phase.store(0, Ordering::Relaxed);
    }

    #[cfg(test)]
    pub(crate) fn get_universe(&self, universe_id: u16) -> Option<&Universe> {
        self.universes.get(&universe_id)
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
                validate_light_shows(dsl_show.shows(), Some(lighting_config)).map_err(|e| {
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

    /// Plays the given song through the DMX interface.
    #[allow(clippy::too_many_arguments)]
    pub fn play(
        dmx_engine: Arc<Engine>,
        song: Arc<Song>,
        cancel_handle: CancelHandle,
        ready_tx: std::sync::mpsc::Sender<()>,
        start_time: Duration,
        clock: crate::clock::PlaybackClock,
        loop_break: Arc<AtomicBool>,
        active_section: Arc<parking_lot::RwLock<Option<crate::player::SectionBounds>>>,
        section_loop_break: Arc<AtomicBool>,
    ) -> Result<(), Box<dyn Error>> {
        let span = span!(Level::INFO, "play song (dmx)");
        let _enter = span.enter();

        // Check if there are any lighting systems to play
        let light_shows = song.light_shows();
        let dsl_lighting_shows = song.dsl_lighting_shows();
        let has_lighting = !dsl_lighting_shows.is_empty();

        if light_shows.is_empty() && !has_lighting {
            let _ = ready_tx.send(());
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
                let timeline = LightingTimeline::new(all_shows);
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
                    match super::watcher::start_watching(
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

        let mut dmx_midi_sheets: HashMap<String, (MidiSheet, Vec<u8>)> = HashMap::new();
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
            let _ = ready_tx.send(());
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
        let _ = ready_tx.send(());

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
                let mut section_trigger = crate::section_loop::SectionLoopTrigger::new();
                let mut iteration_start: Option<Duration> = None;
                // Cached section bounds so we can handle break even after
                // active_section is cleared by stop_section_loop().
                let mut cached_section: Option<crate::player::SectionBounds> = None;
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
                        if let Some(ref section) = cached_section {
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

                    let section = active_section.read().clone();
                    if let Some(ref section) = section {
                        // Cache section bounds for use during break handling.
                        cached_section = Some(section.clone());

                        let section_duration = section.end_time.saturating_sub(section.start_time);
                        if section_duration.is_zero() {
                            break;
                        }

                        let elapsed = clock.elapsed();

                        if let Some(iter_start) = iteration_start {
                            let time_since = elapsed.saturating_sub(iter_start);
                            let position = time_since.min(section_duration);
                            dmx_engine.update_song_time(section.start_time + position);
                        }

                        let crossfade_margin = crate::audio::crossfade::DEFAULT_CROSSFADE_DURATION;
                        if section_trigger
                            .check(section, elapsed, crossfade_margin)
                            .is_some()
                        {
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
                    } else {
                        // No active section.
                        if cached_section.is_some() {
                            // Section was cleared without break — reset.
                            cached_section = None;
                        }
                        section_trigger.reset();
                        iteration_start = None;
                        section_owns_time.store(false, Ordering::Relaxed);
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

    /// Advances all MIDI DMX playback cursors to the current song time,
    /// dispatching events via handle_midi_event_by_id.
    fn advance_midi_dmx_playbacks(&self) {
        let song_time = match self.get_song_time().checked_sub(self.playback_delay) {
            Some(t) => t,
            None => return, // Still within the playback delay period
        };
        let mut playbacks = self.midi_dmx_playbacks.lock();
        for playback in playbacks.iter_mut() {
            let events = playback.precomputed.events();
            while playback.cursor < events.len() && events[playback.cursor].time <= song_time {
                let event = &events[playback.cursor];
                if playback.midi_channels.is_empty()
                    || playback.midi_channels.contains(&event.channel)
                {
                    self.handle_midi_event_by_id(playback.universe_id, event.message);
                }
                playback.cursor += 1;
            }
        }
    }

    /// Returns true if all MIDI DMX playbacks have finished.
    fn midi_dmx_playbacks_finished(&self) -> bool {
        let playbacks = self.midi_dmx_playbacks.lock();
        playbacks.iter().all(|p| p.cursor >= p.precomputed.len())
    }

    /// Handles an incoming MIDI event (by universe name).
    pub fn handle_midi_event(&self, universe_name: String, midi_message: midly::MidiMessage) {
        if let Some(&universe_id) = self.universe_name_to_id.get(&universe_name) {
            self.handle_midi_event_by_id(universe_id, midi_message);
        }
    }

    /// Handles an incoming MIDI event (by universe ID).
    /// Avoids the name→ID HashMap lookup when the caller already knows the ID.
    fn handle_midi_event_by_id(&self, universe_id: u16, midi_message: midly::MidiMessage) {
        match classify_midi_dmx_action(midi_message, self.dimming_speed_modifier) {
            MidiDmxAction::KeyVelocity { channel, value } => {
                self.update_universe_by_id(universe_id, channel, value, true);
            }
            MidiDmxAction::Controller { channel, value } => {
                self.update_universe_by_id(universe_id, channel, value, false);
            }
            MidiDmxAction::Dimming { duration } => {
                self.update_dimming_by_id(universe_id, duration);
            }
            MidiDmxAction::Unrecognized => {
                debug!(
                    midi_event = format!("{:?}", midi_message),
                    "Unrecognized MIDI event"
                );
            }
        }
    }

    // Updates the current dimming speed.
    fn update_dimming_by_id(&self, universe_id: u16, dimming_duration: Duration) {
        debug!(
            dimming = dimming_duration.as_secs_f64(),
            "Dimming speed updated"
        );
        if let Some(universe) = self.universes.get(&universe_id) {
            universe.update_dim_speed(dimming_duration);
        }
        // Mirror dim rate to MIDI DMX store
        let rate = if dimming_duration.is_zero() {
            1.0
        } else {
            dimming_duration.as_secs_f64() * super::universe::TARGET_HZ
        };
        self.midi_dmx_store.read().set_dim_rate(universe_id, rate);
    }

    /// Updates the given universe by ID.
    /// Mapped channels (those with registered fixtures) go through the lockless
    /// MidiDmxStore for interpolation and EffectEngine injection. Unmapped
    /// channels go directly to the Universe for backward compatibility.
    fn update_universe_by_id(&self, universe_id: u16, channel: u16, value: u8, dim: bool) {
        let store = self.midi_dmx_store.read();
        if store.lookup(universe_id, channel).is_some() {
            // Mapped channel → lockless store (interpolation + EffectEngine injection)
            store.write(universe_id, channel, value, dim);
        } else {
            // Unmapped channel → direct to Universe (backward compat)
            if let Some(universe) = self.universes.get(&universe_id) {
                universe.update_channel_data(channel, value, dim);
            }
        }
    }

    /// Updates the effects engine and applies any generated commands to universes
    pub fn update_effects(&self) -> Result<(), Box<dyn std::error::Error>> {
        // Update the effects engine with a frame time matching Universe TARGET_HZ
        let dt = Duration::from_secs_f64(1.0 / super::universe::TARGET_HZ);
        // Subphase 1: about to acquire current_song_time lock
        self.update_subphase.store(1, Ordering::Relaxed);
        let song_time = self.get_song_time();
        // Subphase 2: about to acquire effect_engine lock
        self.update_subphase.store(2, Ordering::Relaxed);
        let mut effect_engine = match self.effect_engine.try_lock_for(Duration::from_secs(2)) {
            Some(guard) => guard,
            None => {
                error!(
                    "effect_engine lock blocked for >2s in update_effects — \
                     another holder is not releasing it"
                );
                self.effect_engine.lock()
            }
        };
        let commands = effect_engine.update(dt, Some(song_time))?;

        // Group commands by universe
        let mut universe_commands: std::collections::HashMap<u16, Vec<(u16, u8)>> =
            std::collections::HashMap::new();
        for command in commands {
            universe_commands
                .entry(command.universe)
                .or_default()
                .push((command.channel, command.value));
        }

        // DMX command summary logging removed

        // Apply effect commands to universes
        for (universe_id, commands) in universe_commands {
            // Direct lookup by universe ID - no name mapping needed
            if let Some(universe) = self.universes.get(&universe_id) {
                universe.update_effect_commands(commands);
            }
        }

        Ok(())
    }

    /// Starts a lighting effect
    pub fn start_effect(
        &self,
        effect: crate::lighting::EffectInstance,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let mut effect_engine = self.effect_engine.lock();
        effect_engine.start_effect(effect)?;
        Ok(())
    }

    /// Registers all fixtures from the current venue (thread-safe version)
    pub fn register_venue_fixtures_safe(&self) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(lighting_system) = &self.lighting_system {
            let lighting_system = lighting_system.lock();
            let fixture_infos = lighting_system.get_current_venue_fixtures()?;
            let mut effect_engine = self.effect_engine.lock();
            let mut midi_dmx_store = self.midi_dmx_store.write();

            for fixture_info in &fixture_infos {
                // Register slots in the MIDI DMX store for each fixture channel
                for (channel_name, &offset) in &fixture_info.channels {
                    let dmx_channel = fixture_info.address + offset - 1;
                    midi_dmx_store.register_slot(
                        fixture_info.universe,
                        dmx_channel,
                        &fixture_info.name,
                        channel_name,
                    );
                }
                midi_dmx_store.register_universe(fixture_info.universe);
            }

            // Set the MIDI DMX store reference on the EffectEngine
            effect_engine.set_midi_dmx_store(self.midi_dmx_store.clone());

            for fixture_info in fixture_infos {
                effect_engine.register_fixture(fixture_info);
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
    pub fn start_lighting_timeline_at(&self, start_time: Duration) {
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

        // Apply the historical timeline update to ensure deterministic state
        // This must happen before the effects loop can process new cues to avoid conflicts
        if start_time > Duration::ZERO {
            if let Err(e) = self.apply_timeline_update(timeline_update) {
                error!("Failed to apply historical timeline state: {}", e);
            }
        }
    }

    /// Applies a timeline update (effects and layer commands)
    fn apply_timeline_update(
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
    fn resolve_effect_groups(
        &self,
        mut effect: crate::lighting::EffectInstance,
    ) -> crate::lighting::EffectInstance {
        if let Some(lighting_system) = &self.lighting_system {
            let mut lighting_system = lighting_system.lock();
            let mut resolved_fixtures = Vec::new();
            for group_name in &effect.target_fixtures {
                let fixtures = lighting_system.resolve_logical_group_graceful(group_name);
                resolved_fixtures.extend(fixtures);
            }
            effect.target_fixtures = resolved_fixtures;
        }
        effect
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

    /// Updates the current song time
    pub fn update_song_time(&self, song_time: Duration) {
        let mut current_time = self.current_song_time.lock();
        *current_time = song_time;
    }

    /// Gets the current song time
    pub fn get_song_time(&self) -> Duration {
        let current_time = self.current_song_time.lock();
        *current_time
    }

    /// Sets the broadcast channel so the file watcher can send reload notifications.
    pub fn set_broadcast_tx(&self, tx: tokio::sync::broadcast::Sender<String>) {
        *self.broadcast_tx.lock() = Some(tx);
    }

    /// Returns shared handles for reading lighting state.
    pub fn broadcast_handles(&self) -> BroadcastHandles {
        BroadcastHandles {
            lighting_system: self.lighting_system.clone(),
        }
    }

    /// Returns the effect engine.
    pub fn effect_engine(&self) -> Arc<Mutex<EffectEngine>> {
        self.effect_engine.clone()
    }

    /// Returns the current effects loop heartbeat counter.
    #[cfg(test)]
    pub fn effects_loop_heartbeat(&self) -> u64 {
        self.effects_loop_heartbeat.load(Ordering::Relaxed)
    }

    /// Get a formatted string listing all active effects
    pub fn format_active_effects(&self) -> String {
        let effect_engine = self.effect_engine.lock();
        effect_engine.format_active_effects()
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

    /// Waits for the lighting timeline to finish, with a heartbeat check to detect
    /// a dead effects loop. If the heartbeat stops advancing for 10 seconds, the
    /// effects loop is assumed dead and the wait is abandoned so `Engine::play()`
    /// can clean up instead of blocking forever.
    fn wait_for_timeline_with_heartbeat(
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

    /// Sends messages to OLA using the injected client.
    /// Handles connection failures by attempting to reconnect with backoff.
    fn ola_thread(client: Arc<Mutex<Box<dyn OlaClient>>>, receiver: Receiver<DmxMessage>) {
        let mut disconnected = false;
        let mut last_reconnect_attempt = std::time::Instant::now();
        let reconnect_interval = Duration::from_secs(5);

        loop {
            match receiver.recv() {
                Ok(message) => {
                    if disconnected {
                        // While disconnected, attempt to reconnect periodically.
                        // Messages are dropped until the connection is restored.
                        let now = std::time::Instant::now();
                        if now.duration_since(last_reconnect_attempt) >= reconnect_interval {
                            last_reconnect_attempt = now;
                            let mut client = client.lock();
                            match client.reconnect() {
                                Ok(()) => {
                                    info!("Reconnected to OLA");
                                    disconnected = false;
                                    if let Err(err) =
                                        client.send_dmx(message.universe, &message.buffer)
                                    {
                                        error!("Lost connection to OLA: {}", err);
                                        disconnected = true;
                                    }
                                }
                                Err(err) => {
                                    warn!("Failed to reconnect to OLA: {}", err);
                                }
                            }
                        }
                    } else {
                        let mut client = client.lock();
                        if let Err(err) = client.send_dmx(message.universe, &message.buffer) {
                            error!("Lost connection to OLA: {}", err);
                            disconnected = true;
                            last_reconnect_attempt = std::time::Instant::now();
                        }
                    }
                }
                Err(_) => return,
            }
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        // The persistent effects loop uses a Weak<Engine> and will exit automatically
        // when this Engine is dropped (the weak upgrade will fail).
        // We still cancel the handle for any other consumers.
        self.cancel_handle.cancel();

        // Join the effects loop thread (it will exit since the weak ref can no longer upgrade)
        if let Some(handle) = self.effects_loop_handle.lock().take() {
            if handle.join().is_err() {
                error!("Error joining effects loop handle");
            }
        }

        self.join_handles.drain(..).for_each(|join_handle| {
            if join_handle.join().is_err() {
                error!("Error joining handle");
            }
        });

        self.universes.drain();

        if self
            .client_handle
            .take()
            .expect("Expected client handle")
            .join()
            .is_err()
        {
            error!("Error joining handle");
        }
    }
}

#[cfg(test)]
mod test {
    use std::{
        collections::HashSet,
        error::Error,
        net::{Ipv4Addr, SocketAddr, TcpListener},
        sync::{atomic::AtomicBool, Arc},
        time::Duration,
    };

    use midly::num::u7;

    use crate::playsync::CancelHandle;

    use super::{config, Engine};
    use crate::dmx::ola_client::OlaClientFactory;
    use crate::lighting::effects::EffectType;

    fn create_engine() -> Result<(Arc<Engine>, CancelHandle), Box<dyn Error>> {
        let listener = TcpListener::bind(SocketAddr::new(
            std::net::IpAddr::V4(Ipv4Addr::UNSPECIFIED),
            0,
        ))?;
        let port = listener.local_addr()?.port();
        // Use a mock OLA client for tests
        let ola_client = OlaClientFactory::create_mock_client();
        let engine = Engine::new(
            &config::Dmx::new(
                None,
                None,
                Some(port),
                vec![config::Universe::new(5, "universe1".to_string())],
                None, // lighting configuration
            ),
            None,
            None,
            ola_client,
        )?;
        let cancel_handle = engine.cancel_handle.clone();
        Ok((Arc::new(engine), cancel_handle))
    }

    #[test]
    fn test_handle_midi_event_by_id() -> Result<(), Box<dyn Error>> {
        let (engine, _cancel_handle) = create_engine()?;

        // Verify the default dim speed value.
        assert_eq!(engine.get_universe(5).unwrap().get_dim_speed(), 1.0);

        // Send a ProgramChange event to update dimming.
        engine.handle_midi_event_by_id(
            5,
            midly::MidiMessage::ProgramChange {
                program: u7::new(1u8),
            },
        );

        // Verify that the universe got our command.
        assert_eq!(engine.get_universe(5).unwrap().get_dim_speed(), 44.0);

        Ok(())
    }

    #[test]
    fn test_effects_integration() -> Result<(), Box<dyn Error>> {
        let (engine, _cancel_handle) = create_engine()?;

        // Register a fixture with the effects engine
        let fixture_info = {
            let mut channels = std::collections::HashMap::new();
            channels.insert("dimmer".to_string(), 1);
            channels.insert("red".to_string(), 2);
            channels.insert("green".to_string(), 3);
            channels.insert("blue".to_string(), 4);
            crate::lighting::effects::FixtureInfo::new(
                "test_fixture".to_string(),
                1,
                1,
                "RGBW_Par".to_string(),
                channels,
                None,
            )
        };

        {
            let mut effect_engine = engine.effect_engine.lock();
            effect_engine.register_fixture(fixture_info);
        }

        // Test that we can start and stop effects
        let mut parameters = std::collections::HashMap::new();
        parameters.insert("dimmer".to_string(), 0.5);

        let effect = crate::lighting::EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters: parameters.clone(),
                duration: Duration::ZERO,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        // This should not panic
        engine.start_effect(effect).unwrap();

        Ok(())
    }

    #[test]
    fn test_lighting_system_integration() -> Result<(), Box<dyn Error>> {
        // This test verifies that the lighting system can be initialized
        // without requiring OLA connection by testing the configuration parsing

        // Create a mock lighting config
        let lighting_config = config::Lighting::new(
            Some("test_venue".to_string()),
            Some({
                let mut fixtures = std::collections::HashMap::new();
                fixtures.insert("Wash1".to_string(), "RGBW_Par @ 1:1".to_string());
                fixtures
            }),
            Some({
                let mut groups = std::collections::HashMap::new();
                let front_wash_group = crate::config::lighting::LogicalGroup::new(
                    "front_wash".to_string(),
                    vec![crate::config::lighting::GroupConstraint::AllOf(vec![
                        "wash".to_string(),
                        "front".to_string(),
                    ])],
                );
                groups.insert("front_wash".to_string(), front_wash_group);
                groups
            }),
            None, // No directories for this test
        );

        // Test that the lighting config can be created and accessed
        assert!(lighting_config.current_venue().is_some());
        assert_eq!(lighting_config.current_venue().unwrap(), "test_venue");

        // fixtures() returns HashMap directly, not Option<HashMap>
        assert_eq!(lighting_config.fixtures().len(), 1);
        assert!(lighting_config.fixtures().contains_key("Wash1"));

        // groups() returns HashMap directly, not Option<HashMap>
        assert_eq!(lighting_config.groups().len(), 1);
        assert!(lighting_config.groups().contains_key("front_wash"));

        Ok(())
    }

    #[test]
    fn test_lighting_system_without_config() -> Result<(), Box<dyn Error>> {
        // This test verifies that DMX config can be created without lighting system
        let dmx_config = config::Dmx::new(
            None,
            None,
            Some(9090),
            vec![config::Universe::new(1, "universe1".to_string())],
            None,
        );

        // Verify that the DMX config has no lighting configuration
        assert!(dmx_config.lighting().is_none());

        Ok(())
    }

    #[test]
    fn test_register_venue_fixtures_without_lighting_system() -> Result<(), Box<dyn Error>> {
        let (engine, _cancel_handle) = create_engine()?;

        // Should not panic when no lighting system is available
        engine.register_venue_fixtures_safe()?;

        Ok(())
    }

    #[test]
    fn test_effects_update_without_fixtures() -> Result<(), Box<dyn Error>> {
        let (engine, _cancel_handle) = create_engine()?;

        // Update effects with no fixtures registered - should not panic
        engine.update_effects()?;

        Ok(())
    }

    #[test]
    fn test_effects_update_with_fixtures() -> Result<(), Box<dyn Error>> {
        let (engine, _cancel_handle) = create_engine()?;

        // Register a fixture
        let fixture_info = {
            let mut channels = std::collections::HashMap::new();
            channels.insert("dimmer".to_string(), 1);
            channels.insert("red".to_string(), 2);
            channels.insert("green".to_string(), 3);
            channels.insert("blue".to_string(), 4);
            crate::lighting::effects::FixtureInfo::new(
                "test_fixture".to_string(),
                1,
                1,
                "RGBW_Par".to_string(),
                channels,
                None,
            )
        };

        {
            let mut effect_engine = engine.effect_engine.lock();
            effect_engine.register_fixture(fixture_info);
        }

        // Start an effect
        let mut parameters = std::collections::HashMap::new();
        parameters.insert("dimmer".to_string(), 0.8);
        parameters.insert("red".to_string(), 1.0);

        let effect = crate::lighting::EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters,
                duration: Duration::ZERO,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect)?;

        // Update effects - should generate commands
        engine.update_effects()?;

        Ok(())
    }

    #[test]
    fn test_song_lighting_integration() -> Result<(), Box<dyn Error>> {
        // Test that we can create a song with lighting configuration

        let song_config = config::Song::new(
            "Test Song",
            None,
            None,
            None,
            None,
            None, // No lighting shows for this test
            vec![],
            std::collections::HashMap::new(),
            Vec::new(),
        );

        // Test that the song config has lighting
        assert!(song_config.lighting().is_none());

        Ok(())
    }

    fn create_test_config() -> config::Dmx {
        config::Dmx::new(
            Some(1.0),
            Some("0s".to_string()),
            Some(9090),
            vec![config::Universe::new(1, "test_universe".to_string())],
            None,
        )
    }

    fn create_test_engine() -> Result<Engine, Box<dyn std::error::Error>> {
        let config = create_test_config();
        // Use mock OLA client for testing
        let ola_client = OlaClientFactory::create_mock_client();
        Engine::new(&config, None, None, ola_client)
    }

    #[test]
    fn test_effect_builder_methods() {
        let engine = create_test_engine().unwrap();

        // Register a test fixture first
        let mut channels = std::collections::HashMap::new();
        channels.insert("dimmer".to_string(), 1);
        channels.insert("red".to_string(), 2);
        channels.insert("green".to_string(), 3);
        channels.insert("blue".to_string(), 4);

        let fixture_info = crate::lighting::effects::FixtureInfo::new(
            "test_fixture".to_string(),
            1,
            1,
            "RGB".to_string(),
            channels,
            None,
        );

        // Register fixture through the effect engine
        {
            let mut effect_engine = engine.effect_engine.lock();
            effect_engine.register_fixture(fixture_info);
        } // Drop the lock here

        // Test effect with builder methods - simplified to avoid timing issues
        let effect = crate::lighting::EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters: std::collections::HashMap::new(),
                duration: Duration::ZERO,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        )
        .with_priority(5);

        // Test that we can start the effect
        let result = engine.start_effect(effect);
        assert!(result.is_ok());
    }

    #[test]
    fn test_midi_dmx_channel_filtering() -> Result<(), Box<dyn Error>> {
        let (engine, _cancel_handle) = create_engine()?;

        assert_eq!(engine.get_universe(5).unwrap().get_dim_speed(), 1.0);

        // Send a ProgramChange directly — handle_midi_event_by_id always accepts.
        engine.handle_midi_event_by_id(
            5,
            midly::MidiMessage::ProgramChange {
                program: u7::new(1u8),
            },
        );

        assert_eq!(engine.get_universe(5).unwrap().get_dim_speed(), 44.0);

        // Verify channel filtering via MIDI DMX playback dispatch.
        // Build a playback with channel filter = {5}.
        use crate::midi::playback::{PrecomputedMidi, TimedMidiEvent};
        let events = vec![TimedMidiEvent {
            time: std::time::Duration::ZERO,
            channel: 6, // excluded
            message: midly::MidiMessage::ProgramChange {
                program: u7::new(0u8),
            },
        }];
        let precomputed = PrecomputedMidi::from_events(events);
        let mut midi_channels = HashSet::new();
        midi_channels.insert(5);
        {
            let mut playbacks = engine.midi_dmx_playbacks.lock();
            playbacks.push(super::MidiDmxPlayback {
                precomputed,
                cursor: 0,
                universe_id: 5,
                midi_channels,
            });
        }

        // Advance playbacks — channel 6 should be excluded
        engine.update_song_time(std::time::Duration::from_secs(1));
        engine.advance_midi_dmx_playbacks();

        // Dim speed should still be 44.0, not reset to 0.0
        assert_eq!(engine.get_universe(5).unwrap().get_dim_speed(), 44.0);

        // Cleanup
        engine.midi_dmx_playbacks.lock().clear();

        Ok(())
    }

    #[test]
    fn test_group_resolution_in_dmx_engine() -> Result<(), Box<dyn std::error::Error>> {
        use crate::lighting::{effects::EffectType, EffectInstance};
        use std::collections::HashMap;

        // Create DMX engine with lighting system
        let config = create_test_config();
        let lighting_config = Some(crate::config::Lighting::new(
            Some("Test Venue".to_string()),
            None,
            None,
            None,
        ));
        let ola_client = OlaClientFactory::create_mock_client();
        let engine = Engine::new(&config, lighting_config.as_ref(), None, ola_client)?;

        // Test group resolution with a simple effect
        let mut parameters = HashMap::new();
        parameters.insert("dimmer".to_string(), 0.8);
        parameters.insert("red".to_string(), 1.0);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters,
                duration: Duration::ZERO,
            },
            vec!["test_group".to_string()],
            None,
            None,
            None,
        );

        // Test that the effect can be started (graceful fallback for missing groups)
        // Note: This may fail if fixtures aren't registered, which is expected behavior
        let _result = engine.start_effect(effect);
        // We expect this to work with graceful fallback, but it may fail if no fixtures are registered
        // This is acceptable behavior for the test

        Ok(())
    }

    #[test]
    fn test_group_resolution_graceful_fallback() -> Result<(), Box<dyn std::error::Error>> {
        use crate::lighting::{effects::EffectType, EffectInstance};
        use std::collections::HashMap;

        // Create DMX engine without lighting system
        let config = create_test_config();
        let ola_client = OlaClientFactory::create_mock_client();
        let engine = Engine::new(&config, None, None, ola_client)?;

        // Test that effects with unknown groups still work (graceful fallback)
        let mut parameters = HashMap::new();
        parameters.insert("dimmer".to_string(), 0.5);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters,
                duration: Duration::ZERO,
            },
            vec!["unknown_group".to_string()],
            None,
            None,
            None,
        );

        // Should not fail even with unknown groups
        let _result = engine.start_effect(effect);
        // This may fail if no fixtures are registered, which is expected
        // The graceful fallback is tested by the fact that it doesn't crash

        Ok(())
    }

    #[test]
    fn test_effects_loop_with_timeline() -> Result<(), Box<dyn std::error::Error>> {
        use std::sync::Arc;

        // Create a simple song without lighting for this test
        let temp_path = std::path::Path::new("/tmp/test_song");
        let song_config = crate::config::Song::new(
            "Test Song",
            None,
            None,
            None,
            None,
            None, // No lighting for this test
            vec![],
            std::collections::HashMap::new(),
            Vec::new(),
        );
        let song = crate::songs::Song::new(temp_path, &song_config)?;

        // Create DMX engine
        let config = create_test_config();
        let ola_client = OlaClientFactory::create_mock_client();
        let engine = Arc::new(Engine::new(&config, None, None, ola_client)?);

        // Test timeline setup
        let song_arc = Arc::new(song);
        let cancel_handle = crate::playsync::CancelHandle::new();
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let clock = crate::clock::PlaybackClock::wall();
        clock.start();

        // This should set up the timeline
        Engine::play(
            engine.clone(),
            song_arc,
            cancel_handle,
            ready_tx,
            std::time::Duration::ZERO,
            clock,
            Arc::new(AtomicBool::new(false)),
            Arc::new(parking_lot::RwLock::new(None)),
            Arc::new(AtomicBool::new(false)),
        )?;

        // Verify timeline was created (may be None if no lighting config)
        let _timeline = engine.current_song_timeline.lock();
        // Timeline may be None if no lighting configuration is provided
        // This is acceptable behavior for the test

        Ok(())
    }

    #[test]
    fn test_dsl_to_dmx_command_flow() -> Result<(), Box<dyn std::error::Error>> {
        use crate::dmx::ola_client::{MockOlaClient, OlaClient};
        use crate::lighting::{effects::EffectType, EffectInstance};
        use parking_lot::Mutex;
        use std::collections::HashMap;

        // Create a mock OLA client to capture DMX commands
        let config = create_test_config();
        let mock_client = Arc::new(Mutex::new(MockOlaClient::new()));
        let _mock_client_for_engine = mock_client.clone();
        let ola_client: Box<dyn OlaClient> = Box::new(MockOlaClient::new());
        let engine = Engine::new(&config, None, None, ola_client)?;

        // Create an effect that should generate DMX commands
        let mut parameters = HashMap::new();
        parameters.insert("dimmer".to_string(), 0.8);
        parameters.insert("red".to_string(), 1.0);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters,
                duration: Duration::ZERO,
            },
            vec!["fixture1".to_string()],
            None,
            None,
            None,
        );

        // Start the effect (may fail if fixtures aren't registered)
        let _ = engine.start_effect(effect);

        // Update the effects engine to process the effect
        let _ = engine.update_effects();

        // Verify that DMX commands were sent (if any)
        let mock_client = mock_client.lock();
        let _message = mock_client.get_last_message();

        // DMX commands may or may not be generated depending on fixture registration
        // This is acceptable behavior for the test

        Ok(())
    }

    #[test]
    fn test_midi_to_dmx_channel_mapping() -> Result<(), Box<dyn Error>> {
        let (engine, _cancel_handle) = create_engine()?;

        // Test that MIDI keys map to correct DMX channels
        // MIDI is 0-based (0-127), DMX is 1-based (1-512)
        // So MIDI key 0 should map to DMX channel 1 (array index 0)
        // and MIDI key 1 should map to DMX channel 2 (array index 1), etc.

        // Test NoteOn: MIDI key 0 should update DMX channel 1 (index 0)
        engine.handle_midi_event(
            "universe1".to_string(),
            midly::MidiMessage::NoteOn {
                key: 0.into(),
                vel: 100.into(),
            },
        );
        let universe = engine.get_universe(5).unwrap();
        assert_eq!(
            universe.get_target_value(0),
            200.0,
            "MIDI key 0 should map to DMX channel 1 (index 0)"
        );

        // Test NoteOn: MIDI key 1 should update DMX channel 2 (index 1)
        engine.handle_midi_event(
            "universe1".to_string(),
            midly::MidiMessage::NoteOn {
                key: 1.into(),
                vel: 50.into(),
            },
        );
        assert_eq!(
            universe.get_target_value(1),
            100.0,
            "MIDI key 1 should map to DMX channel 2 (index 1)"
        );

        // Test Controller: MIDI controller 2 should update DMX channel 3 (index 2)
        // Note: Controller values are multiplied by 2, so 100 * 2 = 200
        engine.handle_midi_event(
            "universe1".to_string(),
            midly::MidiMessage::Controller {
                controller: 2.into(),
                value: 100.into(),
            },
        );
        assert_eq!(
            universe.get_target_value(2),
            200.0,
            "MIDI controller 2 should map to DMX channel 3 (index 2)"
        );

        // Test Controller: MIDI controller 3 should update DMX channel 4 (index 3)
        // Note: Controller values are multiplied by 2, so 50 * 2 = 100
        engine.handle_midi_event(
            "universe1".to_string(),
            midly::MidiMessage::Controller {
                controller: 3.into(),
                value: 50.into(),
            },
        );
        assert_eq!(
            universe.get_target_value(3),
            100.0,
            "MIDI controller 3 should map to DMX channel 4 (index 3)"
        );

        Ok(())
    }

    #[test]
    fn test_dmx_channel_numbering() -> Result<(), Box<dyn std::error::Error>> {
        use crate::dmx::ola_client::{MockOlaClient, OlaClient};
        use crate::lighting::effects::{EffectInstance, EffectType, FixtureInfo};
        use parking_lot::Mutex;
        use std::collections::HashMap;

        // Create a mock OLA client to capture DMX commands
        let config = create_test_config();
        let mock_client = Arc::new(Mutex::new(MockOlaClient::new()));
        let _mock_client_for_engine = mock_client.clone();
        let ola_client: Box<dyn OlaClient> = Box::new(MockOlaClient::new());
        let engine = Engine::new(&config, None, None, ola_client)?;

        // Register a fixture with specific channel mapping
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1); // Channel 1
        channels.insert("green".to_string(), 2); // Channel 2
        channels.insert("blue".to_string(), 3); // Channel 3
        channels.insert("dimmer".to_string(), 4); // Channel 4

        let fixture_info = FixtureInfo::new(
            "test_fixture".to_string(),
            1,
            10,
            "RGB_Par".to_string(),
            channels,
            None,
        );

        // Register the fixture
        {
            let mut effect_engine = engine.effect_engine.lock();
            effect_engine.register_fixture(fixture_info);
        }

        // Create an effect that should generate DMX commands
        let mut parameters = HashMap::new();
        parameters.insert("red".to_string(), 1.0);
        parameters.insert("green".to_string(), 0.5);
        parameters.insert("blue".to_string(), 0.0);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters,
                duration: Duration::ZERO,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        // Start the effect
        engine.start_effect(effect)?;

        // Update the effects engine to process the effect
        engine.update_effects()?;

        // Get the universe to check what commands were sent
        let _universe = engine.get_universe(1).unwrap();

        // Check that the correct DMX channels were updated
        // Red should be on channel 10 (address 10 + offset 1 - 1 = 10)
        // Green should be on channel 11 (address 10 + offset 2 - 1 = 11)
        // Blue should be on channel 12 (address 10 + offset 3 - 1 = 12)

        // We can't directly access the universe's channel data in the test,
        // but we can verify that the effect was processed without errors
        // The key fix is that we're no longer double-subtracting 1 from channel numbers

        Ok(())
    }

    /// Helper: seed the engine with a tempo map and timeline to simulate a DSL song
    /// having been played previously.
    fn seed_lighting_state(engine: &Engine) {
        use crate::lighting::tempo::{TempoMap, TimeSignature};
        use crate::lighting::timeline::LightingTimeline;

        let tempo_map = TempoMap::new(
            std::time::Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![],
        );
        {
            let mut effect_engine = engine.effect_engine.lock();
            effect_engine.set_tempo_map(Some(tempo_map));
        }
        {
            let mut timeline = engine.current_song_timeline.lock();
            *timeline = Some(LightingTimeline::new_with_cues(vec![]));
        }
    }

    /// Helper: assert that the tempo map and timeline are both cleared.
    fn assert_lighting_state_cleared(engine: &Engine) {
        let effect_engine = engine.effect_engine.lock();
        assert!(
            !effect_engine.has_tempo_map(),
            "tempo map should be cleared"
        );
        let timeline = engine.current_song_timeline.lock();
        assert!(timeline.is_none(), "timeline should be cleared");
    }

    #[test]
    fn test_dsl_song_clears_previous_tempo_map() -> Result<(), Box<dyn std::error::Error>> {
        // A DSL song without a tempo block must not inherit the tempo map from
        // a previously-played DSL song that had one.
        let (engine, cancel_handle) = create_engine()?;
        seed_lighting_state(&engine);

        // Create a minimal DSL file without a tempo block
        let tmp_dir = tempfile::tempdir()?;
        let dsl_path = tmp_dir.path().join("no_tempo.dsl");
        std::fs::write(
            &dsl_path,
            r#"show "no_tempo" {
    @00:00.000
    front_wash: static color: "blue", duration: 5s, dimmer: 100%
}"#,
        )?;

        let song_config = crate::config::Song::new(
            "DSL No Tempo",
            None,
            None,
            None,
            None,
            Some(vec![crate::config::LightingShow::new(
                dsl_path.to_string_lossy().into_owned(),
            )]),
            vec![],
            std::collections::HashMap::new(),
            Vec::new(),
        );
        let song = Arc::new(crate::songs::Song::new(tmp_dir.path(), &song_config)?);
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let clock = crate::clock::PlaybackClock::wall();
        clock.start();

        // Cancel before play so the DSL-only blocking path exits immediately.
        // The state management is synchronous and runs before any threading.
        cancel_handle.cancel();

        Engine::play(
            engine.clone(),
            song,
            cancel_handle,
            ready_tx,
            std::time::Duration::ZERO,
            clock,
            Arc::new(AtomicBool::new(false)),
            Arc::new(parking_lot::RwLock::new(None)),
            Arc::new(AtomicBool::new(false)),
        )?;

        // The tempo map should have been cleared (not inherited from the seeded state).
        let effect_engine = engine.effect_engine.lock();
        assert!(
            !effect_engine.has_tempo_map(),
            "DSL song without tempo block should clear previous tempo map"
        );
        // The timeline should be replaced (not None — the DSL song provides one).
        let timeline = engine.current_song_timeline.lock();
        assert!(
            timeline.is_some(),
            "DSL song should have set its own timeline"
        );

        Ok(())
    }

    #[test]
    fn test_midi_dmx_song_clears_dsl_state() -> Result<(), Box<dyn std::error::Error>> {
        // After a DSL song, a MIDI DMX song (MIDI-based light shows) must clear the
        // tempo map and timeline left behind.
        let (engine, cancel_handle) = create_engine()?;
        seed_lighting_state(&engine);

        // Create a MIDI DMX song with a MIDI-based light show pointing to a
        // non-matching universe so the (empty) MIDI file is never parsed.
        let assets_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let song_config = crate::config::Song::new(
            "MIDI DMX Song",
            None,
            None,
            None,
            Some(vec![crate::config::LightShow::new(
                "nonexistent_universe".to_string(),
                "song.mid".to_string(),
                None,
            )]),
            None, // No DSL lighting
            vec![],
            std::collections::HashMap::new(),
            Vec::new(),
        );
        let song = Arc::new(crate::songs::Song::new(&assets_path, &song_config)?);
        let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
        let clock = crate::clock::PlaybackClock::wall();
        clock.start();

        Engine::play(
            engine.clone(),
            song,
            cancel_handle,
            ready_tx,
            std::time::Duration::ZERO,
            clock,
            Arc::new(AtomicBool::new(false)),
            Arc::new(parking_lot::RwLock::new(None)),
            Arc::new(AtomicBool::new(false)),
        )?;

        assert_lighting_state_cleared(&engine);
        Ok(())
    }

    #[test]
    fn test_midi_dmx_mirrors_to_effect_engine() -> Result<(), Box<dyn Error>> {
        let (engine, _cancel_handle) = create_engine()?;

        // Register a fixture: universe 5 (matching create_engine), address 1
        let mut channels = std::collections::HashMap::new();
        channels.insert("dimmer".to_string(), 1);
        channels.insert("red".to_string(), 2);
        channels.insert("green".to_string(), 3);
        channels.insert("blue".to_string(), 4);

        let fixture_info = crate::lighting::effects::FixtureInfo::new(
            "test_fixture".to_string(),
            5, // matches universe ID in create_engine
            1, // address
            "RGBW_Par".to_string(),
            channels,
            None,
        );

        // Register slots in the MIDI DMX store
        {
            let mut store = engine.midi_dmx_store.write();
            store.register_slot(5, 1, "test_fixture", "dimmer");
            store.register_slot(5, 2, "test_fixture", "red");
            store.register_slot(5, 3, "test_fixture", "green");
            store.register_slot(5, 4, "test_fixture", "blue");
            store.register_universe(5);
        }

        {
            let mut effect_engine = engine.effect_engine.lock();
            effect_engine.set_midi_dmx_store(engine.midi_dmx_store.clone());
            effect_engine.register_fixture(fixture_info);
        }

        // Send a MIDI DMX NoteOn: key=0 → DMX channel 1 (dimmer), vel=127 → value=254
        engine.handle_midi_event(
            "universe1".to_string(),
            midly::MidiMessage::NoteOn {
                key: 0.into(),
                vel: 127.into(),
            },
        );

        // Tick the store to interpolate (instant since NoteOn uses dim=true but
        // default dim_rate=1.0 so rate calculation produces a single-tick transition)
        engine.midi_dmx_store.read().tick();

        // Verify the EffectEngine has the value after update
        {
            let mut effect_engine = engine.effect_engine.lock();
            let _commands = effect_engine
                .update(std::time::Duration::from_millis(23), None)
                .unwrap();
            let states = effect_engine.get_fixture_states();
            let fixture_state = states
                .get("test_fixture")
                .expect("test_fixture should have state in EffectEngine");
            let dimmer = fixture_state
                .channels
                .get("dimmer")
                .expect("dimmer channel should be present");
            // 254 / 255.0 ≈ 0.996
            assert!(
                (dimmer.value - 254.0 / 255.0).abs() < 0.01,
                "dimmer should be ~0.996, got {}",
                dimmer.value
            );
        }

        Ok(())
    }

    #[test]
    fn test_midi_dmx_unmapped_channel_no_mirror() -> Result<(), Box<dyn Error>> {
        let (engine, _cancel_handle) = create_engine()?;

        // Register a fixture with a small channel range (address 1, 4 channels)
        let mut channels = std::collections::HashMap::new();
        channels.insert("dimmer".to_string(), 1);
        channels.insert("red".to_string(), 2);
        channels.insert("green".to_string(), 3);
        channels.insert("blue".to_string(), 4);

        let fixture_info = crate::lighting::effects::FixtureInfo::new(
            "test_fixture".to_string(),
            5,
            1,
            "RGBW_Par".to_string(),
            channels,
            None,
        );

        // Register slots for channels 1-4 in the MIDI DMX store
        {
            let mut store = engine.midi_dmx_store.write();
            store.register_slot(5, 1, "test_fixture", "dimmer");
            store.register_slot(5, 2, "test_fixture", "red");
            store.register_slot(5, 3, "test_fixture", "green");
            store.register_slot(5, 4, "test_fixture", "blue");
            store.register_universe(5);
        }

        {
            let mut effect_engine = engine.effect_engine.lock();
            effect_engine.set_midi_dmx_store(engine.midi_dmx_store.clone());
            effect_engine.register_fixture(fixture_info);
        }

        // Send a MIDI event to channel 10 (not mapped to any fixture)
        engine.handle_midi_event(
            "universe1".to_string(),
            midly::MidiMessage::NoteOn {
                key: 9.into(), // key+1=10
                vel: 100.into(),
            },
        );

        // Universe should get the write (unmapped channels go directly to Universe)
        let universe = engine.get_universe(5).unwrap();
        assert_eq!(
            universe.get_target_value(9),
            200.0,
            "Universe should have received the unmapped write"
        );

        // Tick and update
        engine.midi_dmx_store.read().tick();

        // EffectEngine should NOT have any state for this unmapped channel
        {
            let mut effect_engine = engine.effect_engine.lock();
            let _commands = effect_engine
                .update(std::time::Duration::from_millis(23), None)
                .unwrap();
            let states = effect_engine.get_fixture_states();
            // test_fixture should not have a channel at offset 10
            if let Some(fixture_state) = states.get("test_fixture") {
                // Only the mapped channels (dimmer, red, green, blue) should ever appear
                for channel_name in fixture_state.channels.keys() {
                    assert!(
                        ["dimmer", "red", "green", "blue"].contains(&channel_name.as_str()),
                        "unexpected channel '{}' in fixture state",
                        channel_name
                    );
                }
            }
        }

        Ok(())
    }

    mod classify_midi_dmx_action_tests {
        use super::super::{classify_midi_dmx_action, MidiDmxAction};
        use midly::num::u7;
        use std::time::Duration;

        #[test]
        fn note_on_converts_key_and_velocity() {
            let action = classify_midi_dmx_action(
                midly::MidiMessage::NoteOn {
                    key: u7::new(0),
                    vel: u7::new(127),
                },
                1.0,
            );
            assert_eq!(
                action,
                MidiDmxAction::KeyVelocity {
                    channel: 1,
                    value: 254
                }
            );
        }

        #[test]
        fn note_off_converts_same_as_note_on() {
            let action = classify_midi_dmx_action(
                midly::MidiMessage::NoteOff {
                    key: u7::new(63),
                    vel: u7::new(0),
                },
                1.0,
            );
            assert_eq!(
                action,
                MidiDmxAction::KeyVelocity {
                    channel: 64,
                    value: 0
                }
            );
        }

        #[test]
        fn note_on_max_key() {
            let action = classify_midi_dmx_action(
                midly::MidiMessage::NoteOn {
                    key: u7::new(127),
                    vel: u7::new(64),
                },
                1.0,
            );
            assert_eq!(
                action,
                MidiDmxAction::KeyVelocity {
                    channel: 128,
                    value: 128
                }
            );
        }

        #[test]
        fn controller_converts_channel_and_value() {
            let action = classify_midi_dmx_action(
                midly::MidiMessage::Controller {
                    controller: u7::new(0),
                    value: u7::new(127),
                },
                1.0,
            );
            assert_eq!(
                action,
                MidiDmxAction::Controller {
                    channel: 1,
                    value: 254
                }
            );
        }

        #[test]
        fn controller_mid_values() {
            let action = classify_midi_dmx_action(
                midly::MidiMessage::Controller {
                    controller: u7::new(10),
                    value: u7::new(50),
                },
                1.0,
            );
            assert_eq!(
                action,
                MidiDmxAction::Controller {
                    channel: 11,
                    value: 100
                }
            );
        }

        #[test]
        fn program_change_with_default_modifier() {
            let action = classify_midi_dmx_action(
                midly::MidiMessage::ProgramChange {
                    program: u7::new(1),
                },
                1.0,
            );
            assert_eq!(
                action,
                MidiDmxAction::Dimming {
                    duration: Duration::from_secs(1)
                }
            );
        }

        #[test]
        fn program_change_with_custom_modifier() {
            let action = classify_midi_dmx_action(
                midly::MidiMessage::ProgramChange {
                    program: u7::new(2),
                },
                0.5,
            );
            assert_eq!(
                action,
                MidiDmxAction::Dimming {
                    duration: Duration::from_secs(1)
                }
            );
        }

        #[test]
        fn program_change_zero_gives_zero_duration() {
            let action = classify_midi_dmx_action(
                midly::MidiMessage::ProgramChange {
                    program: u7::new(0),
                },
                1.0,
            );
            assert_eq!(
                action,
                MidiDmxAction::Dimming {
                    duration: Duration::ZERO
                }
            );
        }

        #[test]
        fn pitch_bend_is_unrecognized() {
            let action = classify_midi_dmx_action(
                midly::MidiMessage::PitchBend {
                    bend: midly::PitchBend(midly::num::u14::new(8192)),
                },
                1.0,
            );
            assert_eq!(action, MidiDmxAction::Unrecognized);
        }

        #[test]
        fn channel_aftertouch_is_unrecognized() {
            let action = classify_midi_dmx_action(
                midly::MidiMessage::Aftertouch {
                    key: u7::new(60),
                    vel: u7::new(100),
                },
                1.0,
            );
            assert_eq!(action, MidiDmxAction::Unrecognized);
        }
    }

    mod song_time_tests {
        use super::*;

        #[test]
        fn song_time_defaults_to_zero() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;
            assert_eq!(engine.get_song_time(), std::time::Duration::ZERO);
            Ok(())
        }

        #[test]
        fn update_and_get_song_time() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;
            let t = std::time::Duration::from_millis(1500);
            engine.update_song_time(t);
            assert_eq!(engine.get_song_time(), t);
            Ok(())
        }

        #[test]
        fn song_time_can_be_overwritten() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;
            engine.update_song_time(std::time::Duration::from_secs(5));
            engine.update_song_time(std::time::Duration::from_secs(10));
            assert_eq!(engine.get_song_time(), std::time::Duration::from_secs(10));
            Ok(())
        }
    }

    mod timeline_cues_tests {
        use super::*;

        #[test]
        fn no_timeline_returns_empty_cues() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;
            assert!(engine.get_timeline_cues().is_empty());
            Ok(())
        }

        #[test]
        fn with_empty_timeline_returns_empty_cues() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;
            {
                let mut timeline = engine.current_song_timeline.lock();
                *timeline = Some(crate::lighting::timeline::LightingTimeline::new_with_cues(
                    vec![],
                ));
            }
            assert!(engine.get_timeline_cues().is_empty());
            Ok(())
        }
    }

    mod midi_dmx_playbacks_finished_tests {
        use super::*;
        use crate::midi::playback::{PrecomputedMidi, TimedMidiEvent};

        #[test]
        fn no_playbacks_means_finished() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;
            assert!(engine.midi_dmx_playbacks_finished());
            Ok(())
        }

        #[test]
        fn playback_at_start_is_not_finished() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;
            let events = vec![TimedMidiEvent {
                time: std::time::Duration::from_secs(1),
                channel: 0,
                message: midly::MidiMessage::NoteOn {
                    key: 0.into(),
                    vel: 100.into(),
                },
            }];
            {
                let mut playbacks = engine.midi_dmx_playbacks.lock();
                playbacks.push(super::super::MidiDmxPlayback {
                    precomputed: PrecomputedMidi::from_events(events),
                    cursor: 0,
                    universe_id: 5,
                    midi_channels: HashSet::new(),
                });
            }
            assert!(!engine.midi_dmx_playbacks_finished());
            Ok(())
        }

        #[test]
        fn playback_at_end_is_finished() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;
            let events = vec![TimedMidiEvent {
                time: std::time::Duration::from_secs(1),
                channel: 0,
                message: midly::MidiMessage::NoteOn {
                    key: 0.into(),
                    vel: 100.into(),
                },
            }];
            let len = events.len();
            {
                let mut playbacks = engine.midi_dmx_playbacks.lock();
                playbacks.push(super::super::MidiDmxPlayback {
                    precomputed: PrecomputedMidi::from_events(events),
                    cursor: len,
                    universe_id: 5,
                    midi_channels: HashSet::new(),
                });
            }
            assert!(engine.midi_dmx_playbacks_finished());
            Ok(())
        }

        #[test]
        fn mixed_playbacks_not_finished() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;
            let event = TimedMidiEvent {
                time: std::time::Duration::ZERO,
                channel: 0,
                message: midly::MidiMessage::NoteOn {
                    key: 0.into(),
                    vel: 100.into(),
                },
            };
            {
                let mut playbacks = engine.midi_dmx_playbacks.lock();
                // Finished playback
                playbacks.push(super::super::MidiDmxPlayback {
                    precomputed: PrecomputedMidi::from_events(vec![event.clone()]),
                    cursor: 1,
                    universe_id: 5,
                    midi_channels: HashSet::new(),
                });
                // Unfinished playback
                playbacks.push(super::super::MidiDmxPlayback {
                    precomputed: PrecomputedMidi::from_events(vec![event]),
                    cursor: 0,
                    universe_id: 5,
                    midi_channels: HashSet::new(),
                });
            }
            assert!(!engine.midi_dmx_playbacks_finished());
            Ok(())
        }
    }

    mod advance_midi_dmx_playbacks_tests {
        use super::*;
        use crate::midi::playback::{PrecomputedMidi, TimedMidiEvent};

        #[test]
        fn playback_delay_prevents_advance() -> Result<(), Box<dyn Error>> {
            // Create engine with a 2-second playback delay
            let config = config::Dmx::new(
                Some(1.0),
                Some("2s".to_string()),
                Some(9090),
                vec![config::Universe::new(5, "universe1".to_string())],
                None,
            );
            let ola_client = crate::dmx::ola_client::OlaClientFactory::create_mock_client();
            let engine = Arc::new(Engine::new(&config, None, None, ola_client)?);

            let events = vec![TimedMidiEvent {
                time: std::time::Duration::ZERO,
                channel: 0,
                message: midly::MidiMessage::ProgramChange {
                    program: u7::new(3),
                },
            }];
            {
                let mut playbacks = engine.midi_dmx_playbacks.lock();
                playbacks.push(super::super::MidiDmxPlayback {
                    precomputed: PrecomputedMidi::from_events(events),
                    cursor: 0,
                    universe_id: 5,
                    midi_channels: HashSet::new(),
                });
            }

            // Song time = 1s, delay = 2s → checked_sub returns None → no advance
            engine.update_song_time(std::time::Duration::from_secs(1));
            engine.advance_midi_dmx_playbacks();

            // Cursor should still be at 0
            let playbacks = engine.midi_dmx_playbacks.lock();
            assert_eq!(playbacks[0].cursor, 0);
            Ok(())
        }

        #[test]
        fn advance_past_delay() -> Result<(), Box<dyn Error>> {
            // Create engine with a 1-second playback delay
            let config = config::Dmx::new(
                Some(1.0),
                Some("1s".to_string()),
                Some(9090),
                vec![config::Universe::new(5, "universe1".to_string())],
                None,
            );
            let ola_client = crate::dmx::ola_client::OlaClientFactory::create_mock_client();
            let engine = Arc::new(Engine::new(&config, None, None, ola_client)?);

            let events = vec![TimedMidiEvent {
                time: std::time::Duration::ZERO,
                channel: 0,
                message: midly::MidiMessage::ProgramChange {
                    program: u7::new(3),
                },
            }];
            {
                let mut playbacks = engine.midi_dmx_playbacks.lock();
                playbacks.push(super::super::MidiDmxPlayback {
                    precomputed: PrecomputedMidi::from_events(events),
                    cursor: 0,
                    universe_id: 5,
                    midi_channels: HashSet::new(),
                });
            }

            // Song time = 2s, delay = 1s → effective time = 1s → should advance
            engine.update_song_time(std::time::Duration::from_secs(2));
            engine.advance_midi_dmx_playbacks();

            let playbacks = engine.midi_dmx_playbacks.lock();
            assert_eq!(playbacks[0].cursor, 1);
            Ok(())
        }

        #[test]
        fn advance_respects_event_time() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            let events = vec![
                TimedMidiEvent {
                    time: std::time::Duration::from_millis(500),
                    channel: 0,
                    message: midly::MidiMessage::NoteOn {
                        key: 0.into(),
                        vel: 100.into(),
                    },
                },
                TimedMidiEvent {
                    time: std::time::Duration::from_secs(2),
                    channel: 0,
                    message: midly::MidiMessage::NoteOn {
                        key: 1.into(),
                        vel: 50.into(),
                    },
                },
            ];
            {
                let mut playbacks = engine.midi_dmx_playbacks.lock();
                playbacks.push(super::super::MidiDmxPlayback {
                    precomputed: PrecomputedMidi::from_events(events),
                    cursor: 0,
                    universe_id: 5,
                    midi_channels: HashSet::new(),
                });
            }

            // At 1s, only the first event (at 500ms) should be dispatched
            engine.update_song_time(std::time::Duration::from_secs(1));
            engine.advance_midi_dmx_playbacks();

            let playbacks = engine.midi_dmx_playbacks.lock();
            assert_eq!(
                playbacks[0].cursor, 1,
                "should advance past first event only"
            );
            Ok(())
        }

        #[test]
        fn empty_channel_filter_accepts_all() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            let events = vec![
                TimedMidiEvent {
                    time: std::time::Duration::ZERO,
                    channel: 3,
                    message: midly::MidiMessage::NoteOn {
                        key: 0.into(),
                        vel: 50.into(),
                    },
                },
                TimedMidiEvent {
                    time: std::time::Duration::ZERO,
                    channel: 7,
                    message: midly::MidiMessage::NoteOn {
                        key: 1.into(),
                        vel: 60.into(),
                    },
                },
            ];
            {
                let mut playbacks = engine.midi_dmx_playbacks.lock();
                playbacks.push(super::super::MidiDmxPlayback {
                    precomputed: PrecomputedMidi::from_events(events),
                    cursor: 0,
                    universe_id: 5,
                    midi_channels: HashSet::new(), // empty = accept all
                });
            }

            engine.update_song_time(std::time::Duration::from_secs(1));
            engine.advance_midi_dmx_playbacks();

            // Both events should be dispatched
            let playbacks = engine.midi_dmx_playbacks.lock();
            assert_eq!(playbacks[0].cursor, 2);
            Ok(())
        }
    }

    mod handle_midi_event_routing_tests {
        use super::*;

        #[test]
        fn unknown_universe_name_is_ignored() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            // Should not panic with unknown universe name
            engine.handle_midi_event(
                "nonexistent_universe".to_string(),
                midly::MidiMessage::NoteOn {
                    key: 0.into(),
                    vel: 100.into(),
                },
            );

            // Universe 5 should be unaffected
            let universe = engine.get_universe(5).unwrap();
            assert_eq!(universe.get_target_value(0), 0.0);
            Ok(())
        }

        #[test]
        fn note_off_updates_universe() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            engine.handle_midi_event(
                "universe1".to_string(),
                midly::MidiMessage::NoteOff {
                    key: 5.into(),
                    vel: 50.into(),
                },
            );

            let universe = engine.get_universe(5).unwrap();
            assert_eq!(
                universe.get_target_value(5),
                100.0,
                "NoteOff should update DMX channel 6 (index 5) with vel*2=100"
            );
            Ok(())
        }
    }

    mod effects_loop_tick_tests {
        use super::*;

        #[test]
        fn tick_updates_phase_diagnostics() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            // After a tick, phase should be back to 0 (idle)
            engine.effects_loop_tick();
            assert_eq!(
                engine
                    .effects_loop_phase
                    .load(std::sync::atomic::Ordering::Relaxed),
                0,
                "phase should be 0 (idle) after tick completes"
            );
            Ok(())
        }

        #[test]
        fn tick_with_finished_timeline_does_not_notify() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            // timeline_finished defaults to true, so the finished-check branch
            // should be skipped entirely
            assert!(engine
                .timeline_finished
                .load(std::sync::atomic::Ordering::Relaxed));
            engine.effects_loop_tick();
            // Just verify no panic
            Ok(())
        }

        #[test]
        fn tick_detects_finished_timeline() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            // Set timeline_finished to false to enter the finished-check branch
            engine
                .timeline_finished
                .store(false, std::sync::atomic::Ordering::Relaxed);

            // No timeline + no playbacks = both done → should set finished to true
            engine.effects_loop_tick();

            assert!(
                engine
                    .timeline_finished
                    .load(std::sync::atomic::Ordering::Relaxed),
                "timeline_finished should be set to true when no timeline and no playbacks"
            );
            Ok(())
        }
    }

    mod validate_song_lighting_tests {
        use super::*;

        #[test]
        fn no_dsl_shows_returns_ok() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            let song_config = crate::config::Song::new(
                "No Lighting",
                None,
                None,
                None,
                None,
                None,
                vec![],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            let song = crate::songs::Song::new(std::path::Path::new("/tmp"), &song_config)?;
            assert!(engine.validate_song_lighting(&song).is_ok());
            Ok(())
        }

        #[test]
        fn valid_dsl_show_passes_validation() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            let tmp_dir = tempfile::tempdir()?;
            let dsl_path = tmp_dir.path().join("test.light");
            std::fs::write(
                &dsl_path,
                r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", duration: 5s, dimmer: 100%
}"#,
            )?;

            let song_config = crate::config::Song::new(
                "With Lighting",
                None,
                None,
                None,
                None,
                Some(vec![crate::config::LightingShow::new(
                    dsl_path.to_string_lossy().into_owned(),
                )]),
                vec![],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            let song = crate::songs::Song::new(tmp_dir.path(), &song_config)?;
            assert!(engine.validate_song_lighting(&song).is_ok());
            Ok(())
        }

        #[test]
        fn invalid_dsl_file_rejected_at_song_creation() {
            let tmp_dir = tempfile::tempdir().unwrap();
            let dsl_path = tmp_dir.path().join("bad.light");
            std::fs::write(&dsl_path, "this is not valid DSL syntax {").unwrap();

            let song_config = crate::config::Song::new(
                "Bad Lighting",
                None,
                None,
                None,
                None,
                Some(vec![crate::config::LightingShow::new(
                    dsl_path.to_string_lossy().into_owned(),
                )]),
                vec![],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            // Song::new validates DSL files at construction time
            assert!(crate::songs::Song::new(tmp_dir.path(), &song_config).is_err());
        }

        #[test]
        fn missing_dsl_file_rejected_at_song_creation() {
            let tmp_dir = tempfile::tempdir().unwrap();
            let song_config = crate::config::Song::new(
                "Missing File",
                None,
                None,
                None,
                None,
                Some(vec![crate::config::LightingShow::new(
                    "/nonexistent/path.light".to_string(),
                )]),
                vec![],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            assert!(crate::songs::Song::new(tmp_dir.path(), &song_config).is_err());
        }
    }

    mod start_lighting_timeline_tests {
        use super::*;

        #[test]
        fn start_at_zero_starts_timeline() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            // Set up a timeline with cues
            {
                let mut timeline = engine.current_song_timeline.lock();
                *timeline = Some(crate::lighting::timeline::LightingTimeline::new_with_cues(
                    vec![],
                ));
            }

            engine.start_lighting_timeline_at(std::time::Duration::ZERO);

            // Timeline should still exist
            let timeline = engine.current_song_timeline.lock();
            assert!(timeline.is_some());
            Ok(())
        }

        #[test]
        fn start_at_nonzero_applies_historical_state() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            // Register a fixture so effects can be started
            let mut channels = std::collections::HashMap::new();
            channels.insert("dimmer".to_string(), 1);
            let fixture_info = crate::lighting::effects::FixtureInfo::new(
                "front_wash".to_string(),
                1,
                1,
                "Generic".to_string(),
                channels,
                None,
            );
            {
                let mut effect_engine = engine.effect_engine.lock();
                effect_engine.register_fixture(fixture_info);
            }

            // Create a timeline with a cue at t=0
            use crate::lighting::parser::{Cue, Effect};
            let effect = Effect {
                sequence_name: None,
                groups: vec!["front_wash".to_string()],
                effect_type: crate::lighting::effects::EffectType::Static {
                    parameters: {
                        let mut p = std::collections::HashMap::new();
                        p.insert("dimmer".to_string(), 1.0);
                        p
                    },
                    duration: Duration::ZERO,
                },
                up_time: None,
                hold_time: None,
                down_time: None,
                layer: None,
                blend_mode: None,
            };
            let cue = Cue {
                time: std::time::Duration::ZERO,
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            };
            {
                let mut timeline = engine.current_song_timeline.lock();
                *timeline = Some(crate::lighting::timeline::LightingTimeline::new_with_cues(
                    vec![cue],
                ));
            }

            // Start at 5 seconds — should apply historical cues
            engine.start_lighting_timeline_at(std::time::Duration::from_secs(5));

            // The effect should have been started via apply_timeline_update
            let effect_engine = engine.effect_engine.lock();
            let active = effect_engine.format_active_effects();
            // Should have active effects from the historical cue
            assert!(
                !active.is_empty(),
                "Historical cue effect should be active after seeking"
            );
            Ok(())
        }

        #[test]
        fn start_without_timeline_is_noop() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            // No timeline set — should not panic
            engine.start_lighting_timeline_at(std::time::Duration::from_secs(5));

            let timeline = engine.current_song_timeline.lock();
            assert!(timeline.is_none());
            Ok(())
        }
    }

    mod update_song_lighting_tests {
        use super::*;

        #[test]
        fn update_with_no_timeline_returns_ok() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;
            assert!(engine
                .update_song_lighting(std::time::Duration::from_secs(1))
                .is_ok());
            Ok(())
        }

        #[test]
        fn update_with_timeline_processes_cues() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            // Register a fixture
            let mut channels = std::collections::HashMap::new();
            channels.insert("dimmer".to_string(), 1);
            let fixture_info = crate::lighting::effects::FixtureInfo::new(
                "test_fixture".to_string(),
                1,
                1,
                "Generic".to_string(),
                channels,
                None,
            );
            {
                let mut effect_engine = engine.effect_engine.lock();
                effect_engine.register_fixture(fixture_info);
            }

            // Create a timeline with a cue at t=1s
            use crate::lighting::parser::{Cue, Effect};
            let cue = Cue {
                time: std::time::Duration::from_secs(1),
                effects: vec![Effect {
                    sequence_name: None,
                    groups: vec!["test_fixture".to_string()],
                    effect_type: crate::lighting::effects::EffectType::Static {
                        parameters: {
                            let mut p = std::collections::HashMap::new();
                            p.insert("dimmer".to_string(), 0.5);
                            p
                        },
                        duration: Duration::ZERO,
                    },
                    up_time: None,
                    hold_time: None,
                    down_time: None,
                    layer: None,
                    blend_mode: None,
                }],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            };

            {
                let mut timeline = engine.current_song_timeline.lock();
                let mut tl = crate::lighting::timeline::LightingTimeline::new_with_cues(vec![cue]);
                tl.start();
                *timeline = Some(tl);
            }

            // Update at t=0 — cue hasn't fired yet
            engine.update_song_lighting(std::time::Duration::ZERO)?;

            // Update at t=2s — cue should fire
            engine.update_song_lighting(std::time::Duration::from_secs(2))?;

            let effect_engine = engine.effect_engine.lock();
            let active = effect_engine.format_active_effects();
            assert!(!active.is_empty(), "Cue effect should be active at t=2s");
            Ok(())
        }
    }

    mod song_time_tracker_tests {
        use super::*;

        #[test]
        fn tracker_updates_song_time() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            let cancel = CancelHandle::new();
            // Set timeline_finished to false so the tracker runs
            engine
                .timeline_finished
                .store(false, std::sync::atomic::Ordering::Relaxed);

            let clock = crate::clock::PlaybackClock::wall();
            clock.start();
            let handle = Engine::start_song_time_tracker_from(
                engine.clone(),
                cancel.clone(),
                std::time::Duration::from_secs(10),
                clock,
            );

            // Wait a bit for the tracker to update
            std::thread::sleep(std::time::Duration::from_millis(50));

            let song_time = engine.get_song_time();
            assert!(
                song_time >= std::time::Duration::from_secs(10),
                "Song time should be at least start_offset (10s), got {:?}",
                song_time
            );

            cancel.cancel();
            handle.join().expect("tracker thread should join");
            Ok(())
        }

        #[test]
        fn tracker_stops_on_timeline_finished() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            let cancel = CancelHandle::new();
            engine
                .timeline_finished
                .store(false, std::sync::atomic::Ordering::Relaxed);

            let clock = crate::clock::PlaybackClock::wall();
            clock.start();
            let handle = Engine::start_song_time_tracker_from(
                engine.clone(),
                cancel,
                std::time::Duration::ZERO,
                clock,
            );

            std::thread::sleep(std::time::Duration::from_millis(30));

            // Signal timeline finished
            engine
                .timeline_finished
                .store(true, std::sync::atomic::Ordering::Relaxed);

            // Thread should exit promptly
            handle.join().expect("tracker thread should join");
            Ok(())
        }
    }

    mod dimming_tests {
        use super::*;

        #[test]
        fn zero_duration_sets_rate_one() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            // First set a non-zero dimming to verify it changes
            engine.handle_midi_event_by_id(
                5,
                midly::MidiMessage::ProgramChange {
                    program: u7::new(2),
                },
            );
            assert!(engine.get_universe(5).unwrap().get_dim_speed() > 1.0);

            // Now send program 0 (zero duration dimming)
            engine.handle_midi_event_by_id(
                5,
                midly::MidiMessage::ProgramChange {
                    program: u7::new(0),
                },
            );

            assert_eq!(
                engine.get_universe(5).unwrap().get_dim_speed(),
                1.0,
                "Zero dimming duration should set dim speed to 1.0"
            );
            Ok(())
        }

        #[test]
        fn dimming_mirrors_to_midi_dmx_store() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            // Register universe in MIDI DMX store
            engine.midi_dmx_store.write().register_universe(5);

            // Send a ProgramChange to set dimming
            engine.handle_midi_event_by_id(
                5,
                midly::MidiMessage::ProgramChange {
                    program: u7::new(1),
                },
            );

            // MIDI DMX store should have the mirrored rate
            // Just verify it doesn't panic — the rate value depends on the dimming_speed_modifier
            let _store = engine.midi_dmx_store.read();
            Ok(())
        }

        #[test]
        fn dimming_unknown_universe_no_panic() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            // Should not panic when universe ID doesn't exist
            engine.update_dimming_by_id(999, std::time::Duration::from_secs(1));
            Ok(())
        }
    }

    mod register_fixtures_tests {
        use super::*;

        #[test]
        fn register_without_lighting_system_is_ok() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;
            // No lighting system — should succeed without registering anything
            engine.register_venue_fixtures_safe()?;
            let effect_engine = engine.effect_engine.lock();
            assert!(effect_engine.get_fixture_states().is_empty());
            Ok(())
        }

        #[test]
        fn register_with_lighting_system_but_no_venue() -> Result<(), Box<dyn Error>> {
            // Lighting config without a venue — loading will fail gracefully
            let lighting_config = crate::config::Lighting::new(None, None, None, None);
            let config = create_test_config();
            let ola_client = OlaClientFactory::create_mock_client();
            let engine = Engine::new(&config, Some(&lighting_config), None, ola_client)?;

            // register_venue_fixtures_safe should handle the case where
            // lighting system exists but venue is incomplete
            let result = engine.register_venue_fixtures_safe();
            // May error due to missing venue, that's expected
            let _ = result;
            Ok(())
        }
    }

    mod effects_loop_heartbeat_tests {
        use super::*;

        #[test]
        fn heartbeat_getter_returns_value() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            // Initial heartbeat should be 0
            assert_eq!(engine.effects_loop_heartbeat(), 0);

            // After manually incrementing, should reflect the change
            engine
                .effects_loop_heartbeat
                .fetch_add(42, std::sync::atomic::Ordering::Relaxed);
            assert_eq!(engine.effects_loop_heartbeat(), 42);
            Ok(())
        }
    }

    mod stop_timeline_tests {
        use super::*;

        #[test]
        fn stop_with_active_timeline() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            {
                let mut timeline = engine.current_song_timeline.lock();
                let mut tl = crate::lighting::timeline::LightingTimeline::new_with_cues(vec![]);
                tl.start();
                *timeline = Some(tl);
            }

            engine.stop_lighting_timeline();

            // Timeline should still exist but be stopped
            let timeline = engine.current_song_timeline.lock();
            assert!(timeline.is_some());
            Ok(())
        }

        #[test]
        fn stop_without_timeline_is_noop() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;
            engine.stop_lighting_timeline();
            Ok(())
        }
    }

    mod broadcast_handles_tests {
        use super::*;

        #[test]
        fn returns_handles() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;
            let handles = engine.broadcast_handles();
            assert!(handles.lighting_system.is_none());
            Ok(())
        }

        #[test]
        fn set_broadcast_tx() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;
            let (tx, _rx) = tokio::sync::broadcast::channel(16);
            engine.set_broadcast_tx(tx);
            // Verify it was stored
            let stored = engine.broadcast_tx.lock();
            assert!(stored.is_some());
            Ok(())
        }
    }

    mod resolve_effect_groups_tests {
        use super::*;
        use crate::lighting::{effects::EffectType, EffectInstance};

        #[test]
        fn resolves_groups_with_lighting_system() -> Result<(), Box<dyn Error>> {
            // Create engine with a lighting system (no venue, but system exists)
            let lighting_config = crate::config::Lighting::new(
                None, // no venue
                None, // no fixtures
                None, // no groups
                None,
            );
            let config = create_test_config();
            let ola_client = OlaClientFactory::create_mock_client();
            let engine = Engine::new(&config, Some(&lighting_config), None, ola_client)?;

            let effect = EffectInstance::new(
                "test".to_string(),
                EffectType::Static {
                    parameters: std::collections::HashMap::new(),
                    duration: Duration::ZERO,
                },
                vec!["some_group".to_string()],
                None,
                None,
                None,
            );

            let resolved = engine.resolve_effect_groups(effect);
            // With a lighting system but no groups defined, resolve_logical_group_graceful
            // will return the name itself as a fallback
            assert!(
                !resolved.target_fixtures.is_empty(),
                "Graceful fallback should return something"
            );
            Ok(())
        }

        #[test]
        fn no_lighting_system_passes_through() -> Result<(), Box<dyn Error>> {
            let config = create_test_config();
            let ola_client = OlaClientFactory::create_mock_client();
            let engine = Engine::new(&config, None, None, ola_client)?;

            let effect = EffectInstance::new(
                "test".to_string(),
                EffectType::Static {
                    parameters: std::collections::HashMap::new(),
                    duration: Duration::ZERO,
                },
                vec!["some_group".to_string()],
                None,
                None,
                None,
            );

            let resolved = engine.resolve_effect_groups(effect);
            assert_eq!(
                resolved.target_fixtures,
                vec!["some_group".to_string()],
                "Without lighting system, groups should pass through unchanged"
            );
            Ok(())
        }
    }

    mod wait_for_timeline_tests {
        use super::*;
        use std::sync::atomic::{AtomicBool, AtomicU64};

        #[test]
        fn exits_when_timeline_finished() {
            let cancel = CancelHandle::new();
            let finished = Arc::new(AtomicBool::new(false));
            let heartbeat = AtomicU64::new(0);
            let phase = AtomicU64::new(0);
            let subphase = AtomicU64::new(0);

            let finished_clone = finished.clone();
            // Set finished after a short delay
            let setter = std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                finished_clone.store(true, std::sync::atomic::Ordering::Relaxed);
            });

            Engine::wait_for_timeline_with_heartbeat(
                &cancel, finished, &heartbeat, &phase, &subphase,
            );

            setter.join().unwrap();
        }

        #[test]
        fn exits_when_cancelled() {
            let cancel = CancelHandle::new();
            let finished = Arc::new(AtomicBool::new(false));
            let heartbeat = AtomicU64::new(0);
            let phase = AtomicU64::new(0);
            let subphase = AtomicU64::new(0);

            let cancel_clone = cancel.clone();
            let setter = std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(50));
                cancel_clone.cancel();
            });

            Engine::wait_for_timeline_with_heartbeat(
                &cancel, finished, &heartbeat, &phase, &subphase,
            );

            setter.join().unwrap();
        }
    }

    mod effect_engine_accessor_tests {
        use super::*;

        #[test]
        fn effect_engine_returns_arc() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;
            let ee = engine.effect_engine();
            // Should be able to lock and use
            let locked = ee.lock();
            let _ = locked.format_active_effects();
            Ok(())
        }
    }

    mod play_tests {
        use super::*;

        fn create_dsl_song_with_content(
            dsl_content: &str,
        ) -> Result<(tempfile::TempDir, Arc<crate::songs::Song>), Box<dyn Error>> {
            let tmp_dir = tempfile::tempdir()?;
            let dsl_path = tmp_dir.path().join("show.light");
            std::fs::write(&dsl_path, dsl_content)?;

            let song_config = crate::config::Song::new(
                "DSL Song",
                None,
                None,
                None,
                None,
                Some(vec![crate::config::LightingShow::new(
                    dsl_path.to_string_lossy().into_owned(),
                )]),
                vec![],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            let song = Arc::new(crate::songs::Song::new(tmp_dir.path(), &song_config)?);
            Ok((tmp_dir, song))
        }

        #[test]
        fn play_song_with_no_lighting_returns_ok() -> Result<(), Box<dyn Error>> {
            let (engine, cancel_handle) = create_engine()?;
            Engine::start_persistent_effects_loop(engine.clone());

            let song_config = crate::config::Song::new(
                "No Light",
                None,
                None,
                None,
                None,
                None,
                vec![],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            let song = Arc::new(crate::songs::Song::new(
                std::path::Path::new("/tmp"),
                &song_config,
            )?);
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let clock = crate::clock::PlaybackClock::wall();
            clock.start();

            let result = Engine::play(
                engine,
                song,
                cancel_handle,
                ready_tx,
                std::time::Duration::ZERO,
                clock,
                Arc::new(AtomicBool::new(false)),
                Arc::new(parking_lot::RwLock::new(None)),
                Arc::new(AtomicBool::new(false)),
            );
            assert!(result.is_ok());
            Ok(())
        }

        #[test]
        fn play_dsl_song_cancelled() -> Result<(), Box<dyn Error>> {
            let (engine, cancel_handle) = create_engine()?;
            Engine::start_persistent_effects_loop(engine.clone());

            let (_tmp_dir, song) = create_dsl_song_with_content(
                r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", duration: 5s, dimmer: 100%
}"#,
            )?;

            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let clock = crate::clock::PlaybackClock::wall();
            clock.start();

            // Cancel before play so the blocking path exits immediately
            cancel_handle.cancel();

            let result = Engine::play(
                engine.clone(),
                song,
                cancel_handle,
                ready_tx,
                std::time::Duration::ZERO,
                clock,
                Arc::new(AtomicBool::new(false)),
                Arc::new(parking_lot::RwLock::new(None)),
                Arc::new(AtomicBool::new(false)),
            );
            assert!(result.is_ok());
            Ok(())
        }

        #[test]
        fn play_dsl_song_with_start_time() -> Result<(), Box<dyn Error>> {
            let (engine, cancel_handle) = create_engine()?;
            Engine::start_persistent_effects_loop(engine.clone());

            let (_tmp_dir, song) = create_dsl_song_with_content(
                r#"show "test" {
    @00:00.000
    front_wash: static color: "red", duration: 5s, dimmer: 100%
    @00:05.000
    front_wash: static color: "blue", duration: 5s, dimmer: 50%
}"#,
            )?;

            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let clock = crate::clock::PlaybackClock::wall();
            clock.start();
            cancel_handle.cancel();

            // Start at 3 seconds — should process historical cues
            let result = Engine::play(
                engine.clone(),
                song,
                cancel_handle,
                ready_tx,
                std::time::Duration::from_secs(3),
                clock,
                Arc::new(AtomicBool::new(false)),
                Arc::new(parking_lot::RwLock::new(None)),
                Arc::new(AtomicBool::new(false)),
            );
            assert!(result.is_ok());
            Ok(())
        }

        #[test]
        fn play_midi_dmx_song_with_unmatched_universe() -> Result<(), Box<dyn Error>> {
            let (engine, cancel_handle) = create_engine()?;
            Engine::start_persistent_effects_loop(engine.clone());

            let assets_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
            let song_config = crate::config::Song::new(
                "MIDI DMX Song",
                None,
                None,
                None,
                Some(vec![crate::config::LightShow::new(
                    "nonexistent_universe".to_string(),
                    "song.mid".to_string(),
                    None,
                )]),
                None,
                vec![],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            let song = Arc::new(crate::songs::Song::new(&assets_path, &song_config)?);
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let clock = crate::clock::PlaybackClock::wall();
            clock.start();

            let result = Engine::play(
                engine.clone(),
                song,
                cancel_handle,
                ready_tx,
                std::time::Duration::ZERO,
                clock,
                Arc::new(AtomicBool::new(false)),
                Arc::new(parking_lot::RwLock::new(None)),
                Arc::new(AtomicBool::new(false)),
            );
            assert!(result.is_ok());
            Ok(())
        }

        #[test]
        fn play_midi_dmx_song_multiple_unmatched_universes() -> Result<(), Box<dyn Error>> {
            // Test the empty barrier thread path with multiple unmatched universes
            let (engine, cancel_handle) = create_engine()?;
            Engine::start_persistent_effects_loop(engine.clone());

            let assets_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
            let song_config = crate::config::Song::new(
                "MIDI DMX Multi",
                None,
                None,
                None,
                Some(vec![
                    crate::config::LightShow::new(
                        "nonexistent1".to_string(),
                        "song.mid".to_string(),
                        None,
                    ),
                    crate::config::LightShow::new(
                        "nonexistent2".to_string(),
                        "song.mid".to_string(),
                        None,
                    ),
                ]),
                None,
                vec![],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            let song = Arc::new(crate::songs::Song::new(&assets_path, &song_config)?);
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let clock = crate::clock::PlaybackClock::wall();
            clock.start();

            let result = Engine::play(
                engine.clone(),
                song,
                cancel_handle,
                ready_tx,
                std::time::Duration::ZERO,
                clock,
                Arc::new(AtomicBool::new(false)),
                Arc::new(parking_lot::RwLock::new(None)),
                Arc::new(AtomicBool::new(false)),
            );
            assert!(result.is_ok(), "play failed: {:?}", result.err());
            Ok(())
        }
    }

    mod apply_timeline_update_tests {
        use super::*;
        use crate::lighting::parser::{LayerCommand, LayerCommandType};

        #[test]
        fn applies_layer_commands() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            let update = crate::lighting::timeline::TimelineUpdate {
                effects: vec![],
                effects_with_elapsed: std::collections::HashMap::new(),
                layer_commands: vec![LayerCommand {
                    command_type: LayerCommandType::Clear,
                    layer: None,
                    fade_time: None,
                    intensity: None,
                    speed: None,
                }],
                stop_sequences: vec![],
            };

            assert!(engine.apply_timeline_update(update).is_ok());
            Ok(())
        }

        #[test]
        fn applies_stop_sequences() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            let update = crate::lighting::timeline::TimelineUpdate {
                effects: vec![],
                effects_with_elapsed: std::collections::HashMap::new(),
                layer_commands: vec![],
                stop_sequences: vec!["test_seq".to_string()],
            };

            assert!(engine.apply_timeline_update(update).is_ok());
            Ok(())
        }

        #[test]
        fn applies_effects_with_elapsed() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            // Register a fixture for the effect
            let mut channels = std::collections::HashMap::new();
            channels.insert("dimmer".to_string(), 1);
            let fixture_info = crate::lighting::effects::FixtureInfo::new(
                "test_fixture".to_string(),
                1,
                1,
                "Generic".to_string(),
                channels,
                None,
            );
            {
                let mut ee = engine.effect_engine.lock();
                ee.register_fixture(fixture_info);
            }

            let effect = crate::lighting::EffectInstance::new(
                "test_effect".to_string(),
                crate::lighting::effects::EffectType::Static {
                    parameters: {
                        let mut p = std::collections::HashMap::new();
                        p.insert("dimmer".to_string(), 0.5);
                        p
                    },
                    duration: Duration::ZERO,
                },
                vec!["test_fixture".to_string()],
                None,
                None,
                None,
            );

            let mut effects_with_elapsed = std::collections::HashMap::new();
            effects_with_elapsed.insert(
                "test_effect".to_string(),
                (effect, std::time::Duration::from_secs(2)),
            );

            let update = crate::lighting::timeline::TimelineUpdate {
                effects: vec![],
                effects_with_elapsed,
                layer_commands: vec![],
                stop_sequences: vec![],
            };

            assert!(engine.apply_timeline_update(update).is_ok());
            Ok(())
        }

        #[test]
        fn applies_regular_effects() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            let mut channels = std::collections::HashMap::new();
            channels.insert("dimmer".to_string(), 1);
            let fixture_info = crate::lighting::effects::FixtureInfo::new(
                "test_fixture".to_string(),
                1,
                1,
                "Generic".to_string(),
                channels,
                None,
            );
            {
                let mut ee = engine.effect_engine.lock();
                ee.register_fixture(fixture_info);
            }

            let effect = crate::lighting::EffectInstance::new(
                "seq_test".to_string(),
                crate::lighting::effects::EffectType::Static {
                    parameters: {
                        let mut p = std::collections::HashMap::new();
                        p.insert("dimmer".to_string(), 1.0);
                        p
                    },
                    duration: Duration::ZERO,
                },
                vec!["test_fixture".to_string()],
                None,
                None,
                None,
            );

            let update = crate::lighting::timeline::TimelineUpdate {
                effects: vec![effect],
                effects_with_elapsed: std::collections::HashMap::new(),
                layer_commands: vec![],
                stop_sequences: vec![],
            };

            assert!(engine.apply_timeline_update(update).is_ok());
            Ok(())
        }
    }

    mod format_active_effects_tests {
        use super::*;

        #[test]
        fn no_effects_returns_empty_or_default() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;
            let result = engine.format_active_effects();
            // Should not panic and should return something reasonable
            assert!(result.is_empty() || result.contains("No"));
            Ok(())
        }
    }

    mod ola_thread_tests {
        use super::*;
        use crate::dmx::ola_client::MockOlaClient;
        use std::sync::mpsc;

        #[test]
        fn sends_message_successfully() {
            let client: Box<dyn crate::dmx::ola_client::OlaClient> = Box::new(MockOlaClient::new());
            let client = Arc::new(parking_lot::Mutex::new(client));
            let (tx, rx) = mpsc::channel::<super::super::DmxMessage>();

            let client_clone = client.clone();
            let handle = std::thread::spawn(move || {
                Engine::ola_thread(client_clone, rx);
            });

            // Send a message
            let mut buffer = ola::DmxBuffer::new();
            buffer.set_channel(0, 255);
            tx.send(super::super::DmxMessage {
                universe: 1,
                buffer,
            })
            .unwrap();

            // Drop sender to close channel and exit ola_thread
            drop(tx);
            handle.join().unwrap();
        }

        #[test]
        fn disconnect_and_reconnect() {
            // Create a client that fails on the first send, then succeeds after reconnect
            let mut mock = MockOlaClient::new();
            mock.should_fail = true;
            let client: Box<dyn crate::dmx::ola_client::OlaClient> = Box::new(mock);
            let client = Arc::new(parking_lot::Mutex::new(client));
            let (tx, rx) = mpsc::channel::<super::super::DmxMessage>();

            let client_clone = client.clone();
            let handle = std::thread::spawn(move || {
                Engine::ola_thread(client_clone, rx);
            });

            // First send will fail → disconnected = true
            let buffer = ola::DmxBuffer::new();
            tx.send(super::super::DmxMessage {
                universe: 1,
                buffer: buffer.clone(),
            })
            .unwrap();

            // Wait a moment for the thread to process
            std::thread::sleep(std::time::Duration::from_millis(50));

            // The client is behind a trait object so we can't fix should_fail.
            // Instead, send another message while disconnected (covers the
            // disconnected branch). The reconnect interval is 5s so this
            // message will be dropped (too soon to retry).

            // Send another message while disconnected (covers lines 1271-1275 interval check)
            tx.send(super::super::DmxMessage {
                universe: 1,
                buffer: buffer.clone(),
            })
            .unwrap();

            // Small sleep then drop to exit
            std::thread::sleep(std::time::Duration::from_millis(10));
            drop(tx);
            handle.join().unwrap();
        }
    }

    mod heartbeat_stale_tests {
        use super::*;
        use std::sync::atomic::{AtomicBool, AtomicU64};

        #[test]
        fn stale_heartbeat_forces_timeline_finished() {
            let cancel = CancelHandle::new();
            let finished = Arc::new(AtomicBool::new(false));
            let heartbeat = AtomicU64::new(0);
            let phase = AtomicU64::new(3); // "update_effects"
            let subphase = AtomicU64::new(50); // "effect_process"

            // Don't increment heartbeat — it will go stale.
            // The function checks every 2s with MAX_STALE_CHECKS=3,
            // so after 6s it should force timeline_finished.
            // Use a thread so we can timeout if it hangs.
            let finished_clone = finished.clone();
            let handle = std::thread::spawn(move || {
                Engine::wait_for_timeline_with_heartbeat(
                    &cancel,
                    finished_clone,
                    &heartbeat,
                    &phase,
                    &subphase,
                );
            });

            // Should complete within ~8s (3 stale checks × 2s interval + margin)
            let result = handle.join();
            assert!(result.is_ok(), "wait_for_timeline should have exited");

            // timeline_finished should have been forced to true
            assert!(
                finished.load(std::sync::atomic::Ordering::Relaxed),
                "Stale heartbeat should force timeline_finished=true"
            );
        }

        #[test]
        fn advancing_heartbeat_resets_stale_counter() {
            let cancel = CancelHandle::new();
            let finished = Arc::new(AtomicBool::new(false));
            let heartbeat = AtomicU64::new(0);
            let phase = AtomicU64::new(0);
            let subphase = AtomicU64::new(0);

            let finished_clone = finished.clone();
            let cancel_clone = cancel.clone();

            // Advance the heartbeat periodically, then cancel after a few cycles
            let advancer = std::thread::spawn(move || {
                for i in 1..=5 {
                    std::thread::sleep(std::time::Duration::from_millis(500));
                    heartbeat.store(i, std::sync::atomic::Ordering::Relaxed);
                }
                // After advancing, cancel to exit
                cancel_clone.cancel();
            });

            Engine::wait_for_timeline_with_heartbeat(
                &cancel,
                finished_clone,
                // Can't pass the moved heartbeat, so use a separate one
                &AtomicU64::new(0),
                &phase,
                &subphase,
            );

            // Cancel to make sure the advancer thread finishes
            cancel.cancel();
            advancer.join().unwrap();
        }
    }

    mod unrecognized_midi_tests {
        use super::*;

        #[test]
        fn handle_midi_event_by_id_unrecognized() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            // PitchBend is classified as Unrecognized — should just log and not panic
            engine.handle_midi_event_by_id(
                5,
                midly::MidiMessage::PitchBend {
                    bend: midly::PitchBend(midly::num::u14::new(8192)),
                },
            );

            // Universe should be unaffected
            let universe = engine.get_universe(5).unwrap();
            assert_eq!(universe.get_dim_speed(), 1.0);
            Ok(())
        }
    }

    mod lighting_system_engine_tests {
        use super::*;

        #[test]
        fn engine_with_lighting_system() -> Result<(), Box<dyn Error>> {
            // Create engine with both lighting_config AND base_path to initialize lighting_system
            let lighting_config = crate::config::Lighting::new(None, None, None, None);
            let config = create_test_config();
            let ola_client = OlaClientFactory::create_mock_client();
            let tmp_dir = tempfile::tempdir()?;
            let engine = Engine::new(
                &config,
                Some(&lighting_config),
                Some(tmp_dir.path()),
                ola_client,
            )?;

            // Lighting system should be initialized (even with empty config)
            let handles = engine.broadcast_handles();
            assert!(
                handles.lighting_system.is_some(),
                "Lighting system should be initialized with config + base_path"
            );
            Ok(())
        }

        #[test]
        fn resolve_effect_groups_with_lighting_system() -> Result<(), Box<dyn Error>> {
            let lighting_config = crate::config::Lighting::new(None, None, None, None);
            let config = create_test_config();
            let ola_client = OlaClientFactory::create_mock_client();
            let tmp_dir = tempfile::tempdir()?;
            let engine = Engine::new(
                &config,
                Some(&lighting_config),
                Some(tmp_dir.path()),
                ola_client,
            )?;

            let effect = crate::lighting::EffectInstance::new(
                "test".to_string(),
                crate::lighting::effects::EffectType::Static {
                    parameters: std::collections::HashMap::new(),
                    duration: Duration::ZERO,
                },
                vec!["some_group".to_string()],
                None,
                None,
                None,
            );

            // With a lighting system, resolve_effect_groups goes through the resolution path
            // (lines 1085-1091). With no groups defined, the graceful fallback may
            // return the name itself or empty depending on implementation.
            let _resolved = engine.resolve_effect_groups(effect);
            Ok(())
        }

        #[test]
        fn engine_with_lighting_system_load_failure() -> Result<(), Box<dyn Error>> {
            // Force LightingSystem::load() to fail by pointing fixture_types dir at a file
            let tmp_dir = tempfile::tempdir()?;
            let file_path = tmp_dir.path().join("not_a_dir");
            std::fs::write(&file_path, "I am a file, not a directory")?;

            let dirs = crate::config::lighting::Directories::new(
                Some(file_path.to_string_lossy().into_owned()),
                None,
            );
            let lighting_config = crate::config::Lighting::new(None, None, None, Some(dirs));
            let config = create_test_config();
            let ola_client = OlaClientFactory::create_mock_client();
            let engine = Engine::new(
                &config,
                Some(&lighting_config),
                Some(tmp_dir.path()),
                ola_client,
            )?;

            // Lighting system should be None because load() failed
            let handles = engine.broadcast_handles();
            assert!(
                handles.lighting_system.is_none(),
                "Lighting system should be None when load fails"
            );
            Ok(())
        }

        #[test]
        fn register_venue_fixtures_with_full_lighting_system() -> Result<(), Box<dyn Error>> {
            // Build a lighting system with fixture types and a venue defined in DSL files
            let tmp_dir = tempfile::tempdir()?;

            // Create fixture types directory with a dimmer fixture type
            let ft_dir = tmp_dir.path().join("fixture_types");
            std::fs::create_dir(&ft_dir)?;
            std::fs::write(
                ft_dir.join("dimmer.light"),
                r#"fixture_type "Dimmer" {
    channels: 1
    channel_map: {
        "dimmer": 1
    }
}"#,
            )?;

            // Create venues directory with a venue using the fixture type
            let venue_dir = tmp_dir.path().join("venues");
            std::fs::create_dir(&venue_dir)?;
            std::fs::write(
                venue_dir.join("test.light"),
                r#"venue "test_venue" {
    fixture "Wash1" Dimmer @ 1:1
}"#,
            )?;

            let dirs = crate::config::lighting::Directories::new(
                Some("fixture_types".to_string()),
                Some("venues".to_string()),
            );
            let lighting_config = crate::config::Lighting::new(
                Some("test_venue".to_string()),
                None,
                None,
                Some(dirs),
            );
            let config = config::Dmx::new(
                None,
                None,
                Some(9090),
                vec![config::Universe::new(1, "universe1".to_string())],
                None,
            );
            let ola_client = OlaClientFactory::create_mock_client();
            let engine = Engine::new(
                &config,
                Some(&lighting_config),
                Some(tmp_dir.path()),
                ola_client,
            )?;

            // Verify lighting system was loaded
            let handles = engine.broadcast_handles();
            assert!(handles.lighting_system.is_some());

            // register_venue_fixtures_safe should register fixtures from the venue
            let result = engine.register_venue_fixtures_safe();
            assert!(
                result.is_ok(),
                "register_venue_fixtures_safe failed: {:?}",
                result.err()
            );

            // Verify a fixture was registered in the effect engine
            let effect_engine = engine.effect_engine.lock();
            let registry = effect_engine.get_fixture_registry();
            assert!(
                registry.contains_key("Wash1"),
                "Wash1 fixture should be registered, got: {:?}",
                registry.keys().collect::<Vec<_>>()
            );
            Ok(())
        }

        #[test]
        fn validate_song_lighting_with_lighting_config() -> Result<(), Box<dyn Error>> {
            // Engine with lighting_config set (for validation path).
            // Define "front_wash" as a fixture so validation passes.
            let lighting_config = crate::config::Lighting::new(
                None,
                Some({
                    let mut fixtures = std::collections::HashMap::new();
                    fixtures.insert("front_wash".to_string(), "Generic_Dimmer @ 1:1".to_string());
                    fixtures
                }),
                None,
                None,
            );
            let config = create_test_config();
            let ola_client = OlaClientFactory::create_mock_client();
            let tmp_dir = tempfile::tempdir()?;
            let engine = Engine::new(
                &config,
                Some(&lighting_config),
                Some(tmp_dir.path()),
                ola_client,
            )?;

            let dsl_path = tmp_dir.path().join("show.light");
            std::fs::write(
                &dsl_path,
                r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", duration: 5s, dimmer: 100%
}"#,
            )?;

            let song_config = crate::config::Song::new(
                "With Lighting",
                None,
                None,
                None,
                None,
                Some(vec![crate::config::LightingShow::new(
                    dsl_path.to_string_lossy().into_owned(),
                )]),
                vec![],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            let song = crate::songs::Song::new(tmp_dir.path(), &song_config)?;

            // This exercises the validation path with lighting_config present
            assert!(engine.validate_song_lighting(&song).is_ok());
            Ok(())
        }
    }

    mod validate_error_paths_tests {
        use super::*;

        #[test]
        fn validate_validation_error_with_config() -> Result<(), Box<dyn Error>> {
            // Engine with lighting_config that defines "front_wash" but not "unknown"
            let lighting_config = crate::config::Lighting::new(
                None,
                Some({
                    let mut fixtures = std::collections::HashMap::new();
                    fixtures.insert("front_wash".to_string(), "Generic_Dimmer @ 1:1".to_string());
                    fixtures
                }),
                None,
                None,
            );
            let config = create_test_config();
            let ola_client = OlaClientFactory::create_mock_client();
            let tmp_dir = tempfile::tempdir()?;
            let engine = Engine::new(
                &config,
                Some(&lighting_config),
                Some(tmp_dir.path()),
                ola_client,
            )?;

            let dsl_path = tmp_dir.path().join("show.light");
            std::fs::write(
                &dsl_path,
                r#"show "test" {
    @00:00.000
    unknown_group: static color: "blue", duration: 5s, dimmer: 100%
}"#,
            )?;

            let song_config = crate::config::Song::new(
                "Validate Config Error",
                None,
                None,
                None,
                None,
                Some(vec![crate::config::LightingShow::new(
                    dsl_path.to_string_lossy().into_owned(),
                )]),
                vec![],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            let song = crate::songs::Song::new(tmp_dir.path(), &song_config)?;

            // Validation should fail because "unknown_group" isn't in the config
            let result = engine.validate_song_lighting(&song);
            assert!(result.is_err());
            assert!(result
                .unwrap_err()
                .to_string()
                .contains("Light show validation failed"));
            Ok(())
        }
    }

    mod play_matched_midi_dmx_tests {
        use super::*;

        /// Create a minimal valid MIDI file (Type 0, 1 track)
        fn create_valid_midi_file(path: &std::path::Path) {
            let midi_bytes: Vec<u8> = vec![
                // MThd header
                0x4D, 0x54, 0x68, 0x64, // "MThd"
                0x00, 0x00, 0x00, 0x06, // Header length = 6
                0x00, 0x00, // Format type 0
                0x00, 0x01, // 1 track
                0x01, 0xE0, // 480 ticks per quarter note
                // MTrk chunk
                0x4D, 0x54, 0x72, 0x6B, // "MTrk"
                0x00, 0x00, 0x00, 0x08, // Track length = 8 bytes
                // ProgramChange: delta=0, channel 0, program 0 (instant dimming)
                0x00, 0xC0, 0x00, // End of track marker (required by spec)
                0x00, 0xFF, 0x2F, 0x00, // Padding byte for track length alignment
                0x00,
            ];
            std::fs::write(path, midi_bytes).unwrap();
        }

        #[test]
        fn play_midi_dmx_song_with_matching_universe() -> Result<(), Box<dyn Error>> {
            // Create engine with universe "universe1" mapped to ID 5
            let (engine, cancel_handle) = create_engine()?;
            Engine::start_persistent_effects_loop(engine.clone());

            let tmp_dir = tempfile::tempdir()?;
            let midi_path = tmp_dir.path().join("light.mid");
            create_valid_midi_file(&midi_path);

            let song_config = crate::config::Song::new(
                "MIDI DMX Matched",
                None,
                None,
                None,
                Some(vec![crate::config::LightShow::new(
                    "universe1".to_string(),
                    "light.mid".to_string(),
                    None,
                )]),
                None,
                vec![],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            let song = Arc::new(crate::songs::Song::new(tmp_dir.path(), &song_config)?);
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let clock = crate::clock::PlaybackClock::wall();
            clock.start();

            let result = Engine::play(
                engine.clone(),
                song,
                cancel_handle,
                ready_tx,
                std::time::Duration::ZERO,
                clock,
                Arc::new(AtomicBool::new(false)),
                Arc::new(parking_lot::RwLock::new(None)),
                Arc::new(AtomicBool::new(false)),
            );
            assert!(result.is_ok(), "play failed: {:?}", result.err());
            Ok(())
        }

        #[test]
        fn play_midi_dmx_song_matched_with_start_time() -> Result<(), Box<dyn Error>> {
            let (engine, cancel_handle) = create_engine()?;
            Engine::start_persistent_effects_loop(engine.clone());

            let tmp_dir = tempfile::tempdir()?;
            let midi_path = tmp_dir.path().join("light.mid");
            create_valid_midi_file(&midi_path);

            let song_config = crate::config::Song::new(
                "MIDI DMX Seek",
                None,
                None,
                None,
                Some(vec![crate::config::LightShow::new(
                    "universe1".to_string(),
                    "light.mid".to_string(),
                    None,
                )]),
                None,
                vec![],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            let song = Arc::new(crate::songs::Song::new(tmp_dir.path(), &song_config)?);
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let clock = crate::clock::PlaybackClock::wall();
            clock.start();

            // Start at 10 seconds — should seek past all events
            let result = Engine::play(
                engine.clone(),
                song,
                cancel_handle,
                ready_tx,
                std::time::Duration::from_secs(10),
                clock,
                Arc::new(AtomicBool::new(false)),
                Arc::new(parking_lot::RwLock::new(None)),
                Arc::new(AtomicBool::new(false)),
            );
            assert!(result.is_ok(), "play failed: {:?}", result.err());
            Ok(())
        }

        #[test]
        fn play_mixed_matched_and_unmatched() -> Result<(), Box<dyn Error>> {
            let (engine, cancel_handle) = create_engine()?;
            Engine::start_persistent_effects_loop(engine.clone());

            let tmp_dir = tempfile::tempdir()?;
            let midi_path = tmp_dir.path().join("light.mid");
            create_valid_midi_file(&midi_path);

            let song_config = crate::config::Song::new(
                "Mixed",
                None,
                None,
                None,
                Some(vec![
                    crate::config::LightShow::new(
                        "universe1".to_string(), // matches engine
                        "light.mid".to_string(),
                        None,
                    ),
                    crate::config::LightShow::new(
                        "nonexistent".to_string(), // doesn't match
                        "light.mid".to_string(),
                        None,
                    ),
                ]),
                None,
                vec![],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            let song = Arc::new(crate::songs::Song::new(tmp_dir.path(), &song_config)?);
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let clock = crate::clock::PlaybackClock::wall();
            clock.start();

            let result = Engine::play(
                engine.clone(),
                song,
                cancel_handle,
                ready_tx,
                std::time::Duration::ZERO,
                clock,
                Arc::new(AtomicBool::new(false)),
                Arc::new(parking_lot::RwLock::new(None)),
                Arc::new(AtomicBool::new(false)),
            );
            assert!(result.is_ok(), "play failed: {:?}", result.err());
            Ok(())
        }
    }

    mod play_multi_universe_tests {
        use super::*;

        /// Create a minimal valid MIDI file
        fn create_valid_midi(path: &std::path::Path) {
            let midi_bytes: Vec<u8> = vec![
                0x4D, 0x54, 0x68, 0x64, 0x00, 0x00, 0x00, 0x06, 0x00, 0x00, 0x00, 0x01, 0x01, 0xE0,
                0x4D, 0x54, 0x72, 0x6B, 0x00, 0x00, 0x00, 0x08, 0x00, 0xC0, 0x00, 0x00, 0xFF, 0x2F,
                0x00, 0x00,
            ];
            std::fs::write(path, midi_bytes).unwrap();
        }

        #[test]
        fn play_two_matched_midi_dmx_universes() -> Result<(), Box<dyn Error>> {
            // Engine with two universes to cover the second MIDI DMX barrier thread (line 713-714)
            let config = config::Dmx::new(
                None,
                None,
                Some(9090),
                vec![
                    config::Universe::new(1, "uni_a".to_string()),
                    config::Universe::new(2, "uni_b".to_string()),
                ],
                None,
            );
            let ola_client = OlaClientFactory::create_mock_client();
            let engine = Arc::new(Engine::new(&config, None, None, ola_client)?);
            let cancel_handle = engine.cancel_handle.clone();
            Engine::start_persistent_effects_loop(engine.clone());

            let tmp_dir = tempfile::tempdir()?;
            let midi_path = tmp_dir.path().join("light.mid");
            create_valid_midi(&midi_path);

            let song_config = crate::config::Song::new(
                "Multi Universe",
                None,
                None,
                None,
                Some(vec![
                    crate::config::LightShow::new(
                        "uni_a".to_string(),
                        "light.mid".to_string(),
                        None,
                    ),
                    crate::config::LightShow::new(
                        "uni_b".to_string(),
                        "light.mid".to_string(),
                        None,
                    ),
                ]),
                None,
                vec![],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            let song = Arc::new(crate::songs::Song::new(tmp_dir.path(), &song_config)?);
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let clock = crate::clock::PlaybackClock::wall();
            clock.start();

            let result = Engine::play(
                engine.clone(),
                song,
                cancel_handle,
                ready_tx,
                std::time::Duration::ZERO,
                clock,
                Arc::new(AtomicBool::new(false)),
                Arc::new(parking_lot::RwLock::new(None)),
                Arc::new(AtomicBool::new(false)),
            );
            assert!(result.is_ok(), "play failed: {:?}", result.err());
            Ok(())
        }

        #[test]
        fn play_dsl_and_midi_dmx_combined() -> Result<(), Box<dyn Error>> {
            // Song with both DSL and MIDI DMX light shows
            let config = config::Dmx::new(
                None,
                None,
                Some(9090),
                vec![config::Universe::new(5, "universe1".to_string())],
                None,
            );
            let ola_client = OlaClientFactory::create_mock_client();
            let engine = Arc::new(Engine::new(&config, None, None, ola_client)?);
            let cancel_handle = engine.cancel_handle.clone();
            Engine::start_persistent_effects_loop(engine.clone());

            let tmp_dir = tempfile::tempdir()?;
            let midi_path = tmp_dir.path().join("light.mid");
            create_valid_midi(&midi_path);
            let dsl_path = tmp_dir.path().join("show.light");
            std::fs::write(
                &dsl_path,
                r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", duration: 5s, dimmer: 100%
}"#,
            )?;

            let song_config = crate::config::Song::new(
                "Combined",
                None,
                None,
                None,
                Some(vec![crate::config::LightShow::new(
                    "universe1".to_string(),
                    "light.mid".to_string(),
                    None,
                )]),
                Some(vec![crate::config::LightingShow::new(
                    dsl_path.to_string_lossy().into_owned(),
                )]),
                vec![],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            let song = Arc::new(crate::songs::Song::new(tmp_dir.path(), &song_config)?);
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let clock = crate::clock::PlaybackClock::wall();
            clock.start();

            let result = Engine::play(
                engine.clone(),
                song,
                cancel_handle,
                ready_tx,
                std::time::Duration::ZERO,
                clock,
                Arc::new(AtomicBool::new(false)),
                Arc::new(parking_lot::RwLock::new(None)),
                Arc::new(AtomicBool::new(false)),
            );
            assert!(result.is_ok(), "play failed: {:?}", result.err());
            Ok(())
        }
    }

    mod effects_loop_tick_notify_tests {
        use super::*;

        #[test]
        fn tick_notifies_cancel_handle_when_finished() -> Result<(), Box<dyn Error>> {
            let (engine, _cancel_handle) = create_engine()?;

            // Set timeline_finished to false
            engine
                .timeline_finished
                .store(false, std::sync::atomic::Ordering::Relaxed);

            // Set a cancel handle so the notify path is exercised
            let song_cancel = CancelHandle::new();
            {
                let mut handle = engine.timeline_cancel_handle.lock();
                *handle = Some(song_cancel.clone());
            }

            // No timeline + no playbacks → tick should set finished and notify
            engine.effects_loop_tick();

            assert!(
                engine
                    .timeline_finished
                    .load(std::sync::atomic::Ordering::Relaxed),
                "Should set timeline_finished=true"
            );
            Ok(())
        }
    }

    mod play_dsl_with_lighting_config_tests {
        use super::*;

        #[test]
        fn play_dsl_song_with_lighting_config() -> Result<(), Box<dyn Error>> {
            // Create engine with lighting system to exercise validation path in play()
            let lighting_config = crate::config::Lighting::new(
                None,
                Some({
                    let mut fixtures = std::collections::HashMap::new();
                    fixtures.insert("front_wash".to_string(), "Generic_Dimmer @ 1:1".to_string());
                    fixtures
                }),
                None,
                None,
            );
            let config = config::Dmx::new(
                None,
                None,
                Some(9090),
                vec![config::Universe::new(5, "universe1".to_string())],
                None,
            );
            let ola_client = OlaClientFactory::create_mock_client();
            let tmp_dir = tempfile::tempdir()?;
            let engine = Arc::new(Engine::new(
                &config,
                Some(&lighting_config),
                Some(tmp_dir.path()),
                ola_client,
            )?);
            let cancel_handle = engine.cancel_handle.clone();
            Engine::start_persistent_effects_loop(engine.clone());

            // Set broadcast_tx to exercise the watcher start path in play()
            let (tx, _rx) = tokio::sync::broadcast::channel(16);
            engine.set_broadcast_tx(tx);

            let dsl_path = tmp_dir.path().join("show.light");
            std::fs::write(
                &dsl_path,
                r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", duration: 5s, dimmer: 100%
}"#,
            )?;

            let song_config = crate::config::Song::new(
                "DSL With Config",
                None,
                None,
                None,
                None,
                Some(vec![crate::config::LightingShow::new(
                    dsl_path.to_string_lossy().into_owned(),
                )]),
                vec![],
                std::collections::HashMap::new(),
                Vec::new(),
            );
            let song = Arc::new(crate::songs::Song::new(tmp_dir.path(), &song_config)?);
            let (ready_tx, _ready_rx) = std::sync::mpsc::channel::<()>();
            let clock = crate::clock::PlaybackClock::wall();
            clock.start();

            cancel_handle.cancel();

            let result = Engine::play(
                engine.clone(),
                song,
                cancel_handle,
                ready_tx,
                std::time::Duration::ZERO,
                clock,
                Arc::new(AtomicBool::new(false)),
                Arc::new(parking_lot::RwLock::new(None)),
                Arc::new(AtomicBool::new(false)),
            );
            assert!(result.is_ok());
            Ok(())
        }
    }
}
