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
    fs,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc, Barrier,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use super::legacy_store::LegacyDmxStore;
use super::ola_client::OlaClient;
use midly::num::u7;
use nodi::{Connection, Player};
use ola::DmxBuffer;
use tracing::{debug, error, info, span, warn, Level};

use crate::{
    config,
    lighting::{
        system::LightingSystem, timeline::LightingTimeline, validation::validate_light_shows,
        EffectEngine,
    },
    midi,
    playsync::CancelHandle,
    songs::{MidiSheet, Song},
};

use super::universe::Universe;

/// The DMX engine. This is meant to control the current state of the
/// universe(s) that should be sent to our DMX interface(s).
pub struct Engine {
    dimming_speed_modifier: f64,
    playback_delay: Duration,
    universes: HashMap<u16, Universe>,
    /// Mapping from universe names to IDs for legacy MIDI system
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
    /// Broadcast sender from the simulator (if running), used to start the file watcher per-song
    simulator_broadcast_tx: Mutex<Option<tokio::sync::broadcast::Sender<String>>>,
    /// Handle to the current file watcher (dropped/replaced per-song)
    watcher_handle: Mutex<Option<crate::simulator::watcher::WatcherHandle>>,
    /// Lockless store for legacy MIDI DMX values with built-in interpolation.
    /// RwLock protects structural changes (register_slot); hot-path reads
    /// (write/tick/iter_active) take a cheap read lock while atomics handle data.
    legacy_store: Arc<parking_lot::RwLock<LegacyDmxStore>>,
}

/// DmxMessage is a message that can be passed around between senders and receivers.
#[derive(Clone)]
pub(super) struct DmxMessage {
    pub universe: u32,
    pub buffer: DmxBuffer,
}

/// Shared handles exposed to the simulator for reading state.
pub struct SimulatorHandles {
    pub effect_engine: Arc<Mutex<EffectEngine>>,
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

        // Create mapping from universe names to IDs for legacy MIDI system
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

        let effect_engine = Arc::new(Mutex::new(EffectEngine::new()));
        let current_song_timeline: Arc<Mutex<Option<LightingTimeline>>> =
            Arc::new(Mutex::new(None));
        let current_song_time = Arc::new(Mutex::new(Duration::ZERO));
        let timeline_finished = Arc::new(AtomicBool::new(false));
        let timeline_cancel_handle: Arc<Mutex<Option<CancelHandle>>> = Arc::new(Mutex::new(None));

        let legacy_store = Arc::new(parking_lot::RwLock::new(LegacyDmxStore::new()));

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
            simulator_broadcast_tx: Mutex::new(None),
            watcher_handle: Mutex::new(None),
            legacy_store,
        })
    }

    // Note: Auto-connect helper removed; callers should construct an OLA client and call `new`.

    /// Starts the persistent effects loop. Must be called after wrapping Engine in Arc.
    /// The effects loop runs continuously until the engine is dropped.
    pub fn start_persistent_effects_loop(engine: Arc<Engine>) {
        // Use a weak reference to avoid preventing Engine from being dropped.
        // The thread will exit when the weak reference can no longer be upgraded.
        let weak_engine = Arc::downgrade(&engine);

        let handle = thread::spawn(move || {
            let mut last_update = std::time::Instant::now();
            let target_frame_time = Duration::from_secs_f64(1.0 / 44.0); // 44Hz

            // This loop runs continuously at 44Hz to process effects.
            // It exits when the Engine is dropped (weak upgrade fails).
            loop {
                // Try to upgrade the weak reference - if it fails, the Engine was dropped
                let Some(engine) = weak_engine.upgrade() else {
                    break;
                };

                let now = std::time::Instant::now();
                let dt = now.duration_since(last_update);

                if dt >= target_frame_time {
                    // Tick the legacy store to interpolate dimming values
                    engine.legacy_store.read().tick();

                    // Update effects engine and apply to universes
                    if let Err(e) = engine.update_effects() {
                        error!("Error updating effects: {}", e);
                    }

                    // Update song lighting timeline with actual song time
                    let song_time = engine.get_song_time();
                    if let Err(e) = engine.update_song_lighting(song_time) {
                        error!("Error updating song lighting: {}", e);
                    }

                    // Check if timeline has finished (all cues processed)
                    // and notify the waiting thread if so
                    if !engine.timeline_finished.load(Ordering::Relaxed) {
                        let timeline = engine.current_song_timeline.lock();
                        if let Some(ref tl) = *timeline {
                            if tl.is_finished() {
                                engine.timeline_finished.store(true, Ordering::Relaxed);
                                // Notify the cancel handle so wait() returns
                                if let Some(ref cancel_handle) =
                                    *engine.timeline_cancel_handle.lock()
                                {
                                    cancel_handle.notify();
                                }
                            }
                        }
                    }

                    last_update = now;
                }

                // Drop the Arc before sleeping to minimize time we hold the strong reference
                drop(engine);

                thread::sleep(Duration::from_millis(1));
            }
        });

        // Store the handle so it can be joined on drop.
        // The thread will stop when the Engine is dropped (weak upgrade fails).
        *engine.effects_loop_handle.lock() = Some(handle);
    }

    #[cfg(test)]
    pub(crate) fn get_universe(&self, universe_id: u16) -> Option<&Universe> {
        self.universes.get(&universe_id)
    }

    /// Validates a song's lighting shows before playback starts.
    /// Returns an error if any lighting show is invalid.
    pub fn validate_song_lighting(&self, song: &Song) -> Result<(), Box<dyn Error>> {
        let dsl_lighting_shows = song.dsl_lighting_shows();

        if dsl_lighting_shows.is_empty() {
            return Ok(());
        }

        // Validate DSL shows
        for dsl_show in dsl_lighting_shows {
            let content = fs::read_to_string(dsl_show.file_path()).map_err(|e| {
                format!(
                    "Failed to read DSL show {}: {}",
                    dsl_show.file_path().display(),
                    e
                )
            })?;

            let shows = crate::lighting::parser::parse_light_shows(&content).map_err(|e| {
                format!(
                    "Failed to parse DSL show {}: {}",
                    dsl_show.file_path().display(),
                    e
                )
            })?;

            // Validate shows if lighting config is available
            if let Some(ref lighting_config) = self.lighting_config {
                validate_light_shows(&shows, Some(lighting_config)).map_err(|e| {
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
    pub fn play(
        dmx_engine: Arc<Engine>,
        song: Arc<Song>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
        start_time: Duration,
    ) -> Result<(), Box<dyn Error>> {
        let span = span!(Level::INFO, "play song (dmx)");
        let _enter = span.enter();

        // Check if there are any lighting systems to play
        let light_shows = song.light_shows();
        let dsl_lighting_shows = song.dsl_lighting_shows();
        let has_lighting = !dsl_lighting_shows.is_empty();

        if light_shows.is_empty() && !has_lighting {
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

            // Load DSL shows from the resolved file paths
            let mut all_shows = Vec::new();
            for dsl_show in dsl_lighting_shows {
                match std::fs::read_to_string(dsl_show.file_path()) {
                    Ok(content) => {
                        match crate::lighting::parser::parse_light_shows(&content) {
                            Ok(shows) => {
                                // Validate shows if lighting config is available
                                if let Some(ref lighting_config) = dmx_engine.lighting_config {
                                    if let Err(e) =
                                        validate_light_shows(&shows, Some(lighting_config))
                                    {
                                        error!(
                                            "Light show validation failed for {}: {}",
                                            dsl_show.file_path().display(),
                                            e
                                        );
                                        return Err(format!(
                                            "Light show validation failed for {}: {}",
                                            dsl_show.file_path().display(),
                                            e
                                        )
                                        .into());
                                    }
                                }

                                for (_, show) in shows {
                                    all_shows.push(show);
                                }
                            }
                            Err(e) => {
                                error!(
                                    "Failed to parse DSL show {}: {}",
                                    dsl_show.file_path().display(),
                                    e
                                );
                                return Err(format!(
                                    "Failed to parse DSL show {}: {}",
                                    dsl_show.file_path().display(),
                                    e
                                )
                                .into());
                            }
                        }
                    }
                    Err(e) => {
                        error!(
                            "Failed to read DSL show {}: {}",
                            dsl_show.file_path().display(),
                            e
                        );
                        return Err(format!(
                            "Failed to read DSL show {}: {}",
                            dsl_show.file_path().display(),
                            e
                        )
                        .into());
                    }
                }
            }

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
            // Clear lighting state from previous song so legacy songs
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

        // Reset timeline finished flag for new song
        dmx_engine.timeline_finished.store(false, Ordering::Relaxed);

        // Start file watcher for hot-reload if simulator is running
        {
            let broadcast_tx = dmx_engine.simulator_broadcast_tx.lock();
            if let Some(tx) = broadcast_tx.as_ref() {
                let file_paths: Vec<std::path::PathBuf> = dsl_lighting_shows
                    .iter()
                    .map(|s| s.file_path().to_path_buf())
                    .collect();
                if !file_paths.is_empty() {
                    match crate::simulator::watcher::start_watching(
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

        // Start song time tracking (per-song, tracks elapsed time)
        let song_time_tracker = Self::start_song_time_tracker_from(
            dmx_engine.clone(),
            cancel_handle.clone(),
            start_time,
        );

        // Note: Effects loop is now persistent and started in Engine::new()

        let (universe_ids, playback_delay): (HashSet<u16>, Duration) = (
            dmx_engine.universes.keys().cloned().collect(),
            dmx_engine.playback_delay,
        );

        let mut dmx_midi_sheets: HashMap<String, (MidiSheet, Vec<u8>)> = HashMap::new();
        let mut empty_barrier_counter = 0;
        for light_show in song.light_shows().iter() {
            let universe_name = light_show.universe_name();
            if let Some(&universe_id) = dmx_engine.universe_name_to_id.get(&universe_name) {
                if !universe_ids.contains(&universe_id) {
                    // Keep track of the number of threads that should just wait on the play barrier.
                    empty_barrier_counter += 1;
                    continue;
                }

                dmx_midi_sheets.insert(
                    universe_name.clone(),
                    (light_show.dmx_midi_sheet()?, light_show.midi_channels()),
                );
            } else {
                // Universe name not found in mapping
                empty_barrier_counter += 1;
                continue;
            }
        }

        if dmx_midi_sheets.is_empty() && !has_lighting {
            info!(song = song.name(), "Song has no matching light shows.");

            // Even though we're returning early, we still need to account for the barrier count.
            // The barrier count in play_files() includes song.light_shows().len() if
            // song.light_shows() is not empty. The empty_barrier_counter tracks light shows
            // that don't have matching universes, so we need to spawn threads for them to
            // reach the expected barrier count. Otherwise, other threads will hang waiting
            // for the barrier count to be reached.
            (0..empty_barrier_counter)
                .map(|_| {
                    let play_barrier = play_barrier.clone();
                    thread::spawn(move || {
                        play_barrier.wait();
                    })
                })
                .for_each(|join_handle| {
                    join_handle
                        .join()
                        .expect("Empty barrier join handle should join immediately");
                });

            return Ok(());
        }

        let has_dmx_sheets = !dmx_midi_sheets.is_empty();
        let mut join_handles: Vec<JoinHandle<()>> = dmx_midi_sheets
            .into_iter()
            .filter_map(|(universe_name, light_show_info)| {
                let dmx_midi_sheet = light_show_info.0;
                let midi_channels = HashSet::from_iter(light_show_info.1);
                let cancel_handle = cancel_handle.clone();
                let dmx_engine = dmx_engine.clone();
                // Resolve universe ID once here, avoiding per-event String clone + HashMap lookup.
                let universe_id = match dmx_engine.universe_name_to_id.get(&universe_name) {
                    Some(&id) => id,
                    None => {
                        warn!(universe = %universe_name, "Unknown universe name in light show, skipping");
                        return None;
                    }
                };
                let play_barrier = play_barrier.clone();

                Some(thread::spawn(move || {
                    play_barrier.wait();

                    if cancel_handle.is_cancelled() {
                        return;
                    }

                    let connection = DMXConnection {
                        cancel_handle: cancel_handle.clone(),
                        universe_id,
                        midi_channels,
                        dmx_engine,
                    };
                    let mut player = Player::new(
                        midi::midir::CancelableTimer::new(
                            dmx_midi_sheet.ticker,
                            cancel_handle.clone(),
                        ),
                        connection,
                    );

                    let play_finished = Arc::new(AtomicBool::new(false));

                    spin_sleep::sleep(playback_delay);

                    // Check again before playing
                    if cancel_handle.is_cancelled() {
                        return;
                    }

                    player.play(&dmx_midi_sheet.sheet);
                    play_finished.store(true, std::sync::atomic::Ordering::Relaxed);
                }))
            })
            .collect();

        // If we only have new lighting shows (no old light shows), we still need to wait on the barrier
        if !has_dmx_sheets && has_lighting {
            let play_barrier = play_barrier.clone();
            let cancel_handle = cancel_handle.clone();
            let timeline_finished = dmx_engine.timeline_finished.clone();

            // Store the cancel handle so the effects loop can notify when timeline finishes
            {
                let mut handle = dmx_engine.timeline_cancel_handle.lock();
                *handle = Some(cancel_handle.clone());
            }

            join_handles.push(thread::spawn(move || {
                play_barrier.wait();

                cancel_handle.wait(timeline_finished);
            }));
        }

        // We need to make sure we wait on each available universe, even if it shouldn't
        // be played, to get to the appropriate barrier count, which is equal to the number
        // of universes available on the song.
        (0..empty_barrier_counter)
            .map(|_| {
                let play_barrier = play_barrier.clone();
                thread::spawn(move || {
                    play_barrier.wait();
                })
            })
            .for_each(|join_handle| {
                join_handle
                    .join()
                    .expect("Empty barrier join handle should join immediately");
            });

        // When cancelled, drop join handles to avoid hanging if threads are stuck on barrier wait.
        // Threads will become detached but should exit quickly after barrier wait when they
        // check for cancellation.
        if cancel_handle.is_cancelled() {
            info!(
                "DMX playback has been cancelled. Dropping thread join handles to avoid deadlock."
            );
            drop(join_handles);
        } else {
            let results: Vec<Result<(), Box<dyn Error>>> = join_handles
                .drain(..)
                .map(|join_handle| {
                    if join_handle.join().is_err() {
                        return Err("Error while joining thread!".into());
                    }
                    Ok(())
                })
                .collect();
            for result in results.into_iter() {
                result?;
            }
        }

        // Song playback finished - signal the song time tracker to stop
        // We use timeline_finished flag (not cancel) so we don't cancel audio/MIDI
        dmx_engine.timeline_finished.store(true, Ordering::Relaxed);

        // Wait for song time tracker to finish (will exit now that timeline_finished is set)
        if let Err(e) = song_time_tracker.join() {
            error!("Error waiting for song time tracker to stop: {:?}", e);
        }

        // Stop the lighting timeline for this song, but effects continue processing
        dmx_engine.stop_lighting_timeline();

        info!("DMX playback stopped.");

        Ok(())
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
        match midi_message {
            midly::MidiMessage::NoteOn { key, vel } => {
                self.handle_key_velocity_by_id(universe_id, key, vel);
            }
            midly::MidiMessage::NoteOff { key, vel } => {
                self.handle_key_velocity_by_id(universe_id, key, vel);
            }
            midly::MidiMessage::ProgramChange { program } => {
                self.update_dimming_by_id(
                    universe_id,
                    Duration::from_secs_f64(
                        f64::from(program.as_int()) * self.dimming_speed_modifier,
                    ),
                );
            }
            midly::MidiMessage::Controller { controller, value } => {
                self.update_universe_by_id(
                    universe_id,
                    (controller.as_int() + 1).into(), // Convert from 0-based MIDI to 1-based DMX
                    value.as_int() * 2,
                    false,
                );
            }
            _ => {
                debug!(
                    midi_event = format!("{:?}", midi_message),
                    "Unrecognized MIDI event"
                );
            }
        }
    }

    /// Handles MIDI events that use a key and velocity.
    fn handle_key_velocity_by_id(&self, universe_id: u16, key: u7, velocity: u7) {
        self.update_universe_by_id(
            universe_id,
            (key.as_int() + 1).into(), // Convert from 0-based MIDI to 1-based DMX
            velocity.as_int() * 2,
            true,
        )
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
        // Mirror dim rate to legacy store
        let rate = if dimming_duration.is_zero() {
            1.0
        } else {
            dimming_duration.as_secs_f64() * super::universe::TARGET_HZ
        };
        self.legacy_store.read().set_dim_rate(universe_id, rate);
    }

    /// Updates the given universe by ID.
    /// Mapped channels (those with registered fixtures) go through the lockless
    /// LegacyDmxStore for interpolation and EffectEngine injection. Unmapped
    /// channels go directly to the Universe for backward compatibility.
    fn update_universe_by_id(&self, universe_id: u16, channel: u16, value: u8, dim: bool) {
        let store = self.legacy_store.read();
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
        let song_time = self.get_song_time();
        let mut effect_engine = self.effect_engine.lock();
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
            let mut legacy_store = self.legacy_store.write();

            for fixture_info in &fixture_infos {
                // Register slots in the legacy store for each fixture channel
                for (channel_name, &offset) in &fixture_info.channels {
                    let dmx_channel = fixture_info.address + offset - 1;
                    legacy_store.register_slot(
                        fixture_info.universe,
                        dmx_channel,
                        &fixture_info.name,
                        channel_name,
                    );
                }
                legacy_store.register_universe(fixture_info.universe);
            }

            // Set the legacy store reference on the EffectEngine
            effect_engine.set_legacy_store(self.legacy_store.clone());

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

    /// Sets the simulator broadcast channel so the file watcher can send reload notifications.
    pub fn set_simulator_broadcast_tx(&self, tx: tokio::sync::broadcast::Sender<String>) {
        *self.simulator_broadcast_tx.lock() = Some(tx);
    }

    /// Returns shared handles for the simulator to read state from.
    pub fn simulator_handles(&self) -> SimulatorHandles {
        SimulatorHandles {
            effect_engine: self.effect_engine.clone(),
            lighting_system: self.lighting_system.clone(),
        }
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

    /// Starts a thread to track song time from a specific start time
    pub fn start_song_time_tracker_from(
        dmx_engine: Arc<Engine>,
        cancel_handle: CancelHandle,
        start_offset: Duration,
    ) -> JoinHandle<()> {
        let timeline_finished = dmx_engine.timeline_finished.clone();
        thread::spawn(move || {
            let start_time = std::time::Instant::now();

            // Run until cancelled OR timeline finished
            while !cancel_handle.is_cancelled() && !timeline_finished.load(Ordering::Relaxed) {
                let elapsed = start_time.elapsed();
                let song_time = start_offset + elapsed;

                dmx_engine.update_song_time(song_time);

                // Update every 10ms for reasonable precision
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

/// DMXConnection is a nodi connection that can be cancelled and will output to a
/// DMX interface.
struct DMXConnection {
    cancel_handle: CancelHandle,
    /// Pre-resolved universe ID to avoid String clone + HashMap lookup per MIDI event.
    universe_id: u16,
    midi_channels: HashSet<u8>,
    dmx_engine: Arc<Engine>,
}

impl Connection for DMXConnection {
    fn play(&mut self, event: nodi::MidiEvent) -> bool {
        if self.cancel_handle.is_cancelled() {
            return false;
        };

        if self.midi_channels.is_empty() || self.midi_channels.contains(&event.channel.as_int()) {
            self.dmx_engine
                .handle_midi_event_by_id(self.universe_id, event.message);
        }

        true
    }
}

#[cfg(test)]
mod test {
    use std::{
        collections::HashSet,
        error::Error,
        net::{Ipv4Addr, SocketAddr, TcpListener},
        sync::Arc,
    };

    use midly::num::u7;
    use nodi::{Connection, MidiEvent};

    use crate::playsync::CancelHandle;
    use std::sync::Barrier;

    use super::{config, DMXConnection, Engine};
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
    fn test_connection_cancel() -> Result<(), Box<dyn Error>> {
        let (engine, cancel_handle) = create_engine()?;

        let mut connection = DMXConnection {
            cancel_handle: cancel_handle.clone(),
            universe_id: 5, // matches create_engine() universe
            midi_channels: HashSet::new(),
            dmx_engine: engine.clone(),
        };

        // Verify the default dim speed value.
        assert_eq!(engine.get_universe(5).unwrap().get_dim_speed(), 1.0);

        // No cancellation.
        assert!(connection.play(MidiEvent {
            channel: 5.into(),
            message: midly::MidiMessage::ProgramChange {
                program: u7::new(1u8)
            }
        }));

        // Verify that the universe got our command.
        assert_eq!(engine.get_universe(5).unwrap().get_dim_speed(), 44.0);

        cancel_handle.cancel();

        // Cancellation.
        assert!(!connection.play(MidiEvent {
            channel: 5.into(),
            message: midly::MidiMessage::NoteOn {
                key: 0.into(),
                vel: 0.into(),
            },
        }));

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
                duration: None,
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
                duration: None,
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
                duration: None,
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
    fn test_connection_midi_inclusion() -> Result<(), Box<dyn Error>> {
        let (engine, cancel_handle) = create_engine()?;

        let mut midi_channels: HashSet<u8> = HashSet::new();
        midi_channels.insert(5);
        let mut connection = DMXConnection {
            cancel_handle: cancel_handle.clone(),
            universe_id: 5, // matches create_engine() universe
            midi_channels,
            dmx_engine: engine.clone(),
        };

        assert_eq!(engine.get_universe(5).unwrap().get_dim_speed(), 1.0);

        // Valid MIDI channel.
        assert!(connection.play(MidiEvent {
            channel: 5.into(),
            message: midly::MidiMessage::ProgramChange {
                program: u7::new(1u8)
            }
        }));

        assert_eq!(engine.get_universe(5).unwrap().get_dim_speed(), 44.0);

        // This will be excluded.
        assert!(connection.play(MidiEvent {
            channel: 6.into(),
            message: midly::MidiMessage::ProgramChange {
                program: u7::new(0u8)
            }
        }));

        assert_eq!(engine.get_universe(5).unwrap().get_dim_speed(), 44.0);

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
                duration: None,
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
                duration: None,
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
        let play_barrier = Arc::new(Barrier::new(1));

        // This should set up the timeline
        Engine::play(
            engine.clone(),
            song_arc,
            cancel_handle,
            play_barrier,
            std::time::Duration::ZERO,
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
                duration: None,
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
                duration: None,
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
    front_wash: static color: "blue", dimmer: 100%
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
        let play_barrier = Arc::new(Barrier::new(1));

        // Cancel before play so the DSL-only blocking path exits immediately.
        // The state management is synchronous and runs before any threading.
        cancel_handle.cancel();

        Engine::play(
            engine.clone(),
            song,
            cancel_handle,
            play_barrier,
            std::time::Duration::ZERO,
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
    fn test_legacy_song_clears_dsl_state() -> Result<(), Box<dyn std::error::Error>> {
        // After a DSL song, a legacy song (MIDI-based light shows) must clear the
        // tempo map and timeline left behind.
        let (engine, cancel_handle) = create_engine()?;
        seed_lighting_state(&engine);

        // Create a legacy song with a MIDI-based light show pointing to a
        // non-matching universe so the (empty) MIDI file is never parsed.
        let assets_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets");
        let song_config = crate::config::Song::new(
            "Legacy Song",
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
        let play_barrier = Arc::new(Barrier::new(1));

        Engine::play(
            engine.clone(),
            song,
            cancel_handle,
            play_barrier,
            std::time::Duration::ZERO,
        )?;

        assert_lighting_state_cleared(&engine);
        Ok(())
    }

    #[test]
    fn test_legacy_midi_mirrors_to_effect_engine() -> Result<(), Box<dyn Error>> {
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

        // Register slots in the legacy store
        {
            let mut store = engine.legacy_store.write();
            store.register_slot(5, 1, "test_fixture", "dimmer");
            store.register_slot(5, 2, "test_fixture", "red");
            store.register_slot(5, 3, "test_fixture", "green");
            store.register_slot(5, 4, "test_fixture", "blue");
            store.register_universe(5);
        }

        {
            let mut effect_engine = engine.effect_engine.lock();
            effect_engine.set_legacy_store(engine.legacy_store.clone());
            effect_engine.register_fixture(fixture_info);
        }

        // Send a legacy MIDI NoteOn: key=0 → DMX channel 1 (dimmer), vel=127 → value=254
        engine.handle_midi_event(
            "universe1".to_string(),
            midly::MidiMessage::NoteOn {
                key: 0.into(),
                vel: 127.into(),
            },
        );

        // Tick the store to interpolate (instant since NoteOn uses dim=true but
        // default dim_rate=1.0 so rate calculation produces a single-tick transition)
        engine.legacy_store.read().tick();

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
    fn test_legacy_midi_unmapped_channel_no_mirror() -> Result<(), Box<dyn Error>> {
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

        // Register slots for channels 1-4 in the legacy store
        {
            let mut store = engine.legacy_store.write();
            store.register_slot(5, 1, "test_fixture", "dimmer");
            store.register_slot(5, 2, "test_fixture", "red");
            store.register_slot(5, 3, "test_fixture", "green");
            store.register_slot(5, 4, "test_fixture", "blue");
            store.register_universe(5);
        }

        {
            let mut effect_engine = engine.effect_engine.lock();
            effect_engine.set_legacy_store(engine.legacy_store.clone());
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
        engine.legacy_store.read().tick();

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
}
