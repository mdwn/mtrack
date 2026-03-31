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
use midly::live::LiveEvent;
use parking_lot::RwLock;
use std::fmt;
use std::{
    collections::HashMap,
    error::Error,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, SystemTime},
};
use tokio::{
    sync::{oneshot, Mutex},
    task::JoinHandle,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, span, warn, Level, Span};

use crate::samples::SampleEngine;
use crate::songs::{self, Songs};
use crate::trigger::TriggerEngine;
use crate::{
    audio, config, dmx, midi,
    playlist::{self, Playlist},
    playsync::CancelHandle,
    samples,
    songs::Song,
};

/// Direction for playlist navigation.
enum PlaylistDirection {
    Next,
    Prev,
}

impl fmt::Display for PlaylistDirection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PlaylistDirection::Next => write!(f, "next"),
            PlaylistDirection::Prev => write!(f, "previous"),
        }
    }
}

/// Holds the ingredients for constructing a per-song `PlaybackClock`.
/// When an audio device is present, clocks are derived from its hardware
/// sample counter. Otherwise, clocks fall back to `Instant::now()`.
#[derive(Clone)]
enum ClockSource {
    Audio {
        sample_counter: Arc<std::sync::atomic::AtomicU64>,
        sample_rate: u32,
    },
    Wall,
}

impl ClockSource {
    fn new_clock(&self) -> crate::clock::PlaybackClock {
        match self {
            ClockSource::Audio {
                sample_counter,
                sample_rate,
            } => crate::clock::PlaybackClock::from_sample_counter(
                sample_counter.clone(),
                *sample_rate,
            ),
            ClockSource::Wall => crate::clock::PlaybackClock::wall(),
        }
    }
}

/// Notified when the current song changes. Implementations capture whatever
/// device handles they need at construction time, keeping the player unaware
/// of protocol-specific details.
pub trait SongChangeNotifier: Send + Sync {
    /// Called when the player advances to a new song.
    fn notify(&self, song: &Song);
}

/// Groups all hardware device state so it can be atomically swapped on reload.
#[derive(Clone)]
struct HardwareState {
    device: Option<Arc<dyn audio::Device>>,
    mappings: Option<Arc<HashMap<String, Vec<u16>>>>,
    midi_device: Option<Arc<dyn midi::Device>>,
    dmx_engine: Option<Arc<dmx::engine::Engine>>,
    sample_engine: Option<Arc<RwLock<SampleEngine>>>,
    trigger_engine: Option<Arc<TriggerEngine>>,
    clock_source: ClockSource,
    /// Notifiers invoked on every song change.
    song_change_notifiers: Vec<Arc<dyn SongChangeNotifier>>,
    /// The hostname of the active hardware profile (None if no profile matched).
    profile_name: Option<String>,
    /// The resolved machine hostname used for profile matching.
    hostname: Option<String>,
}

/// Alias for the shared state sampler sender stored on Player.
type StateTx = Arc<
    parking_lot::Mutex<Option<Arc<tokio::sync::watch::Sender<Arc<crate::state::StateSnapshot>>>>>,
>;

struct PlayHandles {
    join: JoinHandle<()>,
    cancel: CancelHandle,
}

/// Groups the parameters needed for `play_files` to avoid excessive argument counts.
struct PlaybackContext {
    device: Option<Arc<dyn audio::Device>>,
    mappings: Option<Arc<HashMap<String, Vec<u16>>>>,
    midi_device: Option<Arc<dyn midi::Device>>,
    dmx_engine: Option<Arc<dmx::engine::Engine>>,
    clock: crate::clock::PlaybackClock,
    song: Arc<Song>,
    cancel_handle: CancelHandle,
    play_tx: oneshot::Sender<Result<(), String>>,
    start_time: Duration,
    play_start_time: Arc<Mutex<Option<SystemTime>>>,
    /// Shared flag to break out of the loop gracefully.
    loop_break: Arc<AtomicBool>,
    /// Active section loop bounds (shared with player).
    active_section: Arc<parking_lot::RwLock<Option<SectionBounds>>>,
    /// Shared flag to break out of a section loop.
    section_loop_break: Arc<AtomicBool>,
    /// Accumulated time consumed by section loop iterations.
    loop_time_consumed: Arc<parking_lot::Mutex<Duration>>,
}

/// Groups hardware devices for constructing a Player without discovering real hardware.
pub struct PlayerDevices {
    pub audio: Option<Arc<dyn audio::Device>>,
    pub mappings: Option<Arc<HashMap<String, Vec<u16>>>>,
    pub midi: Option<Arc<dyn midi::Device>>,
    pub dmx_engine: Option<Arc<dmx::engine::Engine>>,
    pub sample_engine: Option<Arc<RwLock<SampleEngine>>>,
    pub trigger_engine: Option<Arc<TriggerEngine>>,
}

/// Status of a single hardware subsystem.
#[derive(Clone, serde::Serialize)]
pub struct SubsystemStatus {
    pub status: String,
    pub name: Option<String>,
}

/// Snapshot of all hardware subsystem statuses.
#[derive(Clone, serde::Serialize)]
pub struct HardwareStatusSnapshot {
    pub init_done: bool,
    pub hostname: Option<String>,
    pub profile: Option<String>,
    pub audio: SubsystemStatus,
    pub midi: SubsystemStatus,
    pub dmx: SubsystemStatus,
    pub trigger: SubsystemStatus,
}

/// Plays back individual wav files as multichannel audio for the configured audio interface.
#[derive(Clone)]
pub struct Player {
    /// All hardware device state, behind a lock so it can be atomically swapped on reload.
    hardware: Arc<parking_lot::RwLock<HardwareState>>,
    /// Base path for resolving sample/DMX config files. None in test paths.
    base_path: Option<PathBuf>,
    /// All playlists keyed by name. Always includes "all_songs".
    playlists: Arc<parking_lot::RwLock<HashMap<String, Arc<Playlist>>>>,
    /// The name of the currently active playlist.
    active_playlist: Arc<parking_lot::RwLock<String>>,
    /// The persisted active playlist name (last non-all_songs choice). Used by
    /// MIDI/OSC `Playlist` action to return to the user's "real" playlist.
    persisted_playlist: Arc<parking_lot::RwLock<String>>,
    /// The time that the last play action occurred.
    play_start_time: Arc<Mutex<Option<SystemTime>>>,
    /// Keeps track of the player joins. There should only be one task on here at a time.
    join: Arc<Mutex<Option<PlayHandles>>>,
    /// After stop is set, this will be set to true. This will prevent stop from being run again until
    /// it is unset, which should be handled by a cleanup async process after playback finishes.
    stop_run: Arc<AtomicBool>,
    /// The logging span.
    span: Span,
    /// Mutable configuration store for runtime config changes.
    config_store: Arc<parking_lot::Mutex<Option<Arc<config::ConfigStore>>>>,
    /// Cancellation token for the current hardware init round. On reload,
    /// the old token is cancelled and a new one is created.
    init_cancel: Arc<parking_lot::Mutex<CancellationToken>>,
    /// Broadcast channel sender, stored so async init can wire DMX engine.
    broadcast_tx: Arc<parking_lot::Mutex<Option<tokio::sync::broadcast::Sender<String>>>>,
    /// Watch channel to signal hardware init completion.
    init_done_tx: Arc<tokio::sync::watch::Sender<bool>>,
    /// State sampler watch sender. Shared so async init can start the sampler
    /// when the DMX engine becomes available, and restart it on reload.
    state_tx: StateTx,
    /// When true, state-altering operations (config changes, song edits, etc.)
    /// are rejected. Playback controls always work. Locked by default on startup.
    locked: Arc<AtomicBool>,
    /// Active controllers (gRPC, OSC, MIDI). Replaced on reload.
    controller: Arc<parking_lot::Mutex<Option<crate::controller::Controller>>>,
    /// Signal to break out of a song loop gracefully (not a hard cancel).
    /// Set by play()/next() when the current song is looping. The playback
    /// loop checks this flag and exits cleanly, allowing the cleanup task
    /// to advance the playlist and start the next song.
    loop_break: Arc<AtomicBool>,
    /// Active section loop bounds. When Some, audio/MIDI/DMX subsystems
    /// loop the specified time region instead of the full song.
    active_section: Arc<parking_lot::RwLock<Option<SectionBounds>>>,
    /// Signal to stop section looping. The current iteration finishes and
    /// the song continues from the section end.
    section_loop_break: Arc<AtomicBool>,
    /// Accumulated real time consumed by section loop iterations.
    /// Each completed loop iteration adds section_duration to this value.
    /// Subtracted from raw elapsed to get the true song position.
    loop_time_consumed: Arc<parking_lot::Mutex<Duration>>,
    /// Reactive loop state machine.
    reactive_loop_state: Arc<parking_lot::RwLock<ReactiveLoopState>>,
    /// Notification engine for section loop audio feedback.
    notification_engine: Arc<crate::notification::NotificationEngine>,
}

/// Bounds of an active section loop.
///
/// Used together with `section_loop_break: Arc<AtomicBool>` to form a
/// state machine shared by the audio, MIDI, and DMX engine threads:
///
/// | State    | `active_section`  | `section_loop_break` |
/// |----------|-------------------|----------------------|
/// | Idle     | `None`            | `false`              |
/// | Looping  | `Some(bounds)`    | `false`              |
/// | Breaking | `Some(bounds)`*   | `true`               |
///
/// Transitions:
///   - `start_section_loop()`: Idle → Looping (sets `active_section`,
///     clears `section_loop_break`)
///   - `stop_section_loop()`: Looping → Breaking → Idle (sets
///     `section_loop_break` first, then clears `active_section`)
///   - Each engine thread detects the break and exits its loop
///
/// (*) `active_section` is cleared shortly after `section_loop_break` is
/// set. DMX caches the section bounds so it can compute a resume position
/// even after the field is cleared.
///
/// Trigger scheduling within each engine is handled by
/// [`crate::section_loop::SectionLoopTrigger`].
#[derive(Debug, Clone)]
pub struct SectionBounds {
    pub name: String,
    pub start_time: Duration,
    pub end_time: Duration,
}

/// State machine for reactive section looping.
///
/// In reactive mode, sections are "offered" to the performer as playback
/// enters each section boundary. The performer can ack to arm the loop,
/// and later break to exit.
#[derive(Debug, Clone, Default)]
pub enum ReactiveLoopState {
    /// No section is currently being offered or looped.
    #[default]
    Idle,
    /// Playback entered a section; waiting for performer ack.
    SectionOffered(SectionBounds),
    /// Performer acked; loop will engage at section end.
    LoopArmed(SectionBounds),
    /// Section loop is active.
    Looping(SectionBounds),
    /// Break requested; will exit at end of current iteration.
    BreakRequested(SectionBounds),
}

impl Player {
    /// Creates a new player and spawns asynchronous hardware discovery.
    ///
    /// Returns immediately with no hardware initialized. Device discovery
    /// runs in background tasks that retry perpetually until each device is
    /// found or the init round is cancelled (via `reload_hardware`). Use
    /// `await_hardware_ready()` to wait for init to complete (mainly useful
    /// in tests).
    pub fn new(
        playlists: HashMap<String, Arc<Playlist>>,
        active_playlist: String,
        config: &config::Player,
        base_path: Option<&Path>,
    ) -> Result<Arc<Player>, Box<dyn Error>> {
        let devices = PlayerDevices {
            audio: None,
            mappings: None,
            midi: None,
            dmx_engine: None,
            sample_engine: None,
            trigger_engine: None,
        };

        let player = Arc::new(Self::new_with_devices(
            devices,
            playlists,
            active_playlist,
            base_path,
        )?);

        // Mark as not ready — async init will set to true when complete.
        // Use send_modify() because send() is a no-op when no receivers exist yet.
        player.init_done_tx.send_modify(|v| *v = false);

        // Spawn async hardware init.
        let init_player = player.clone();
        let config = config.clone();
        let bp = base_path.map(Path::to_path_buf);
        tokio::spawn(async move {
            init_player.init_hardware_async(config, bp).await;
        });

        Ok(player)
    }

    /// Asynchronously discovers and initializes all hardware devices.
    ///
    /// Retries each device perpetually until found or cancelled. Devices are
    /// written to `HardwareState` as they become available, so playback can
    /// use whatever hardware is ready. Respects dependency ordering:
    ///   Phase 1: Audio + DMX (parallel)
    ///   Phase 2: MIDI (needs DMX), Sample engine (needs Audio) — parallel
    ///   Phase 3: Trigger engine (needs Sample engine), status reporting
    async fn init_hardware_async(
        self: &Arc<Self>,
        config: config::Player,
        base_path: Option<PathBuf>,
    ) {
        let cancel = self.init_cancel.lock().clone();

        let hostname = config::resolve_hostname();
        info!(hostname = %hostname, "Resolved hostname for hardware profiles");

        let profiles = config.profiles(&hostname);
        let profile = match profiles.first() {
            Some(p) => (*p).clone(),
            None => {
                info!("No matching hardware profile found; starting with no hardware");
                {
                    let mut hw = self.hardware.write();
                    hw.hostname = Some(hostname);
                }
                self.init_done_tx.send_modify(|v| *v = true);
                return;
            }
        };

        // Store the active profile name and hostname.
        {
            let mut hw = self.hardware.write();
            hw.profile_name = Some(profile.hostname().unwrap_or("default").to_string());
            hw.hostname = Some(hostname);
        }

        info!(
            hostname = profile.hostname().unwrap_or("default"),
            device = profile
                .audio_config()
                .map(|ac| ac.audio().device())
                .unwrap_or("none"),
            "Using hardware profile"
        );

        // Phase 1: Audio + DMX in parallel (independent subsystems).
        let audio_config = profile.audio_config().cloned();
        let dmx_config = profile.dmx().cloned();
        let bp = base_path.clone();
        let cancel1 = cancel.clone();
        let cancel2 = cancel.clone();

        let (audio_result, dmx_result) =
            tokio::join!(
                async {
                    if let Some(audio_config) = audio_config {
                        Self::retry_until_ready("audio device", cancel1, move || {
                            match audio::get_device(Some(audio_config.audio().clone())) {
                                Ok(device) => {
                                    info!(
                                        device = audio_config.audio().device(),
                                        "Audio device initialized"
                                    );
                                    Ok((
                                        device.clone(),
                                        audio_config.track_mappings_hash(),
                                        audio_config.audio().clone(),
                                    ))
                                }
                                Err(e) => Err(format!("audio device: {}", e)),
                            }
                        })
                        .await
                    } else {
                        info!("Audio not configured in profile; proceeding without audio");
                        None
                    }
                },
                async {
                    if let Some(dmx_config) = dmx_config {
                        let bp = bp.clone();
                        Self::retry_until_ready("dmx engine", cancel2, move || {
                            dmx::create_engine(Some(&dmx_config), bp.as_deref())
                                .map_err(|e| e.to_string())
                        })
                        .await
                        .flatten()
                    } else {
                        info!("DMX not configured in profile; proceeding without DMX");
                        None
                    }
                }
            );

        if cancel.is_cancelled() {
            return;
        }

        // Write Phase 1 results to hardware state.
        let (device, mappings, resolved_audio) = match audio_result {
            Some((device, mappings, resolved_audio)) => {
                let clock_source = match device.sample_counter().zip(device.sample_rate()) {
                    Some((counter, rate)) => ClockSource::Audio {
                        sample_counter: counter,
                        sample_rate: rate,
                    },
                    None => ClockSource::Wall,
                };

                let mut hw = self.hardware.write();
                hw.device = Some(device.clone());
                hw.mappings = Some(Arc::new(mappings.clone()));
                hw.clock_source = clock_source;
                (Some(device), Some(mappings), Some(resolved_audio))
            }
            None => (None, None, None),
        };

        if let Some(ref dmx_engine) = dmx_result {
            self.hardware.write().dmx_engine = Some(dmx_engine.clone());
            // Wire the broadcast channel if one has been set.
            if let Some(ref tx) = *self.broadcast_tx.lock() {
                dmx_engine.set_broadcast_tx(tx.clone());
            }
            // Start the state sampler if a sender was provided. The cancel
            // token ensures this sampler stops when hardware is reloaded.
            if let Some(ref state_tx) = *self.state_tx.lock() {
                let effect_engine = dmx_engine.effect_engine();
                crate::state::start_sampler_cancellable(
                    effect_engine,
                    state_tx.clone(),
                    cancel.clone(),
                );
            }
        }

        if cancel.is_cancelled() {
            return;
        }

        // Phase 2: MIDI (needs DMX) + Sample engine (needs Audio) — parallel.
        let midi_config = profile.midi().cloned();
        let cancel3 = cancel.clone();
        let dmx_engine_for_midi = dmx_result.clone();

        let (midi_result, sample_engine) = tokio::join!(
            async {
                if let Some(midi_config) = midi_config {
                    Self::retry_until_ready("midi device", cancel3, move || {
                        midi::get_device(Some(midi_config.clone()), dmx_engine_for_midi.clone())
                            .map_err(|e| e.to_string())
                    })
                    .await
                    .flatten()
                } else {
                    info!("MIDI not configured in profile; proceeding without MIDI");
                    None
                }
            },
            async {
                // Sample engine init is synchronous and doesn't need retries.
                init_sample_engine(
                    &device,
                    &mappings,
                    resolved_audio.as_ref(),
                    &config,
                    &profile,
                    base_path.as_deref(),
                )
            }
        );

        if cancel.is_cancelled() {
            return;
        }

        // Write Phase 2 results.
        if let Some(ref midi_device) = midi_result {
            self.hardware.write().midi_device = Some(midi_device.clone());
        }
        if let Some(ref se) = sample_engine {
            self.hardware.write().sample_engine = Some(se.clone());
        }

        // Phase 3: Trigger engine (needs sample engine) + post-init wiring.
        let trigger_engine = match init_trigger_engine(&profile, &sample_engine) {
            Ok(te) => te,
            Err(e) => {
                warn!(error = %e, "Failed to initialize trigger engine");
                None
            }
        };
        if let Some(ref te) = trigger_engine {
            self.hardware.write().trigger_engine = Some(te.clone());
        }

        // Start controllers now that all hardware is ready.
        self.start_controllers(profile.controllers().to_vec());

        // MIDI post-init: emit initial track event + start status reporting.
        // This runs after start_controllers so that song change notifiers
        // (e.g. Morningstar) are registered before the first song fires.
        if midi_result.is_some() {
            if let Some(song) = self.get_playlist().current() {
                self.emit_song_change(&song);
            }

            let status_events = match StatusEvents::new(config.status_events()) {
                Ok(se) => se,
                Err(e) => {
                    warn!(error = %e, "Failed to create status events");
                    None
                }
            };

            if let Some(status_events) = status_events {
                let player = self.clone();
                tokio::spawn(async move {
                    player.report_status(status_events).await;
                });
            }
        }

        self.init_done_tx.send_modify(|v| *v = true);
        info!("Hardware initialization complete");
    }

    /// Retries a device constructor perpetually until it succeeds or the
    /// cancellation token is triggered. Device construction runs in a
    /// blocking task since hardware discovery does blocking I/O.
    async fn retry_until_ready<T, E, F>(
        name: &str,
        cancel: CancellationToken,
        constructor: F,
    ) -> Option<T>
    where
        T: Send + 'static,
        E: std::fmt::Display + Send + Sync + 'static,
        F: Fn() -> Result<T, E> + Send + Sync + 'static,
    {
        let name = name.to_string();
        let constructor = Arc::new(constructor);

        loop {
            let ctor = constructor.clone();
            let result = tokio::task::spawn_blocking(move || ctor()).await;

            match result {
                Ok(Ok(value)) => return Some(value),
                Ok(Err(e)) => {
                    warn!("Could not get {name}: {e}");
                }
                Err(e) => {
                    error!("Device init task panicked for {name}: {e}");
                }
            }

            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("Hardware init cancelled for {name}");
                    return None;
                }
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            }
        }
    }

    /// Creates a new player with pre-constructed devices.
    ///
    /// This is the core constructor used by `new()` after device discovery,
    /// and can be called directly in tests with mock devices.
    pub fn new_with_devices(
        devices: PlayerDevices,
        playlists: HashMap<String, Arc<Playlist>>,
        active_playlist: String,
        base_path: Option<&Path>,
    ) -> Result<Player, Box<dyn Error>> {
        // Store the clock source so each song can create a fresh PlaybackClock.
        let clock_source = match devices
            .audio
            .as_ref()
            .and_then(|d| Some((d.sample_counter()?, d.sample_rate()?)))
        {
            Some((counter, rate)) => ClockSource::Audio {
                sample_counter: counter,
                sample_rate: rate,
            },
            None => ClockSource::Wall,
        };

        let hw = HardwareState {
            device: devices.audio,
            mappings: devices.mappings,
            midi_device: devices.midi,
            dmx_engine: devices.dmx_engine,
            sample_engine: devices.sample_engine,
            trigger_engine: devices.trigger_engine,
            clock_source,
            song_change_notifiers: Vec::new(),
            profile_name: None,
            hostname: None,
        };

        let (init_done_tx, _init_done_rx) = tokio::sync::watch::channel(true);

        // Resolve the active playlist: use the requested name if it exists, else fall back to all_songs.
        let resolved_active = if playlists.contains_key(&active_playlist) {
            active_playlist
        } else {
            "all_songs".to_string()
        };

        Ok(Player {
            hardware: Arc::new(parking_lot::RwLock::new(hw)),
            base_path: base_path.map(Path::to_path_buf),
            playlists: Arc::new(parking_lot::RwLock::new(playlists)),
            active_playlist: Arc::new(parking_lot::RwLock::new(resolved_active.clone())),
            persisted_playlist: Arc::new(parking_lot::RwLock::new(
                if resolved_active == "all_songs" {
                    "playlist".to_string()
                } else {
                    resolved_active
                },
            )),
            play_start_time: Arc::new(Mutex::new(None)),
            join: Arc::new(Mutex::new(None)),
            stop_run: Arc::new(AtomicBool::new(false)),
            span: span!(Level::INFO, "player"),
            config_store: Arc::new(parking_lot::Mutex::new(None)),
            init_cancel: Arc::new(parking_lot::Mutex::new(CancellationToken::new())),
            broadcast_tx: Arc::new(parking_lot::Mutex::new(None)),
            init_done_tx: Arc::new(init_done_tx),
            state_tx: Arc::new(parking_lot::Mutex::new(None)),
            locked: Arc::new(AtomicBool::new(true)),
            controller: Arc::new(parking_lot::Mutex::new(None)),
            loop_break: Arc::new(AtomicBool::new(false)),
            active_section: Arc::new(parking_lot::RwLock::new(None)),
            section_loop_break: Arc::new(AtomicBool::new(false)),
            loop_time_consumed: Arc::new(parking_lot::Mutex::new(Duration::ZERO)),
            reactive_loop_state: Arc::new(parking_lot::RwLock::new(ReactiveLoopState::Idle)),
            notification_engine: Arc::new(crate::notification::NotificationEngine::with_defaults(
                44100,
            )),
        })
    }

    /// Waits until hardware initialization is complete. Mainly useful in tests.
    #[cfg(test)]
    pub async fn await_hardware_ready(&self) {
        let mut rx = self.init_done_tx.subscribe();
        while !*rx.borrow_and_update() {
            if rx.changed().await.is_err() {
                break;
            }
        }
    }

    /// Gets the audio device currently in use by the player.
    #[cfg(test)]
    pub fn audio_device(&self) -> Option<Arc<dyn audio::Device>> {
        self.hardware.read().device.clone()
    }

    /// Gets the MIDI device currently in use by the player.
    pub fn midi_device(&self) -> Option<Arc<dyn midi::Device>> {
        self.hardware.read().midi_device.clone()
    }

    /// Adds a notifier that will be called on every song change.
    pub fn add_song_change_notifier(&self, notifier: Arc<dyn SongChangeNotifier>) {
        self.hardware.write().song_change_notifiers.push(notifier);
    }

    /// Returns a snapshot of all hardware subsystem statuses.
    pub fn hardware_status(&self) -> HardwareStatusSnapshot {
        let init_done = *self.init_done_tx.borrow();
        let hw = self.hardware.read();

        let status_for = |present: bool, name: Option<String>| -> SubsystemStatus {
            if present {
                SubsystemStatus {
                    status: "connected".to_string(),
                    name,
                }
            } else if !init_done {
                SubsystemStatus {
                    status: "initializing".to_string(),
                    name: None,
                }
            } else {
                SubsystemStatus {
                    status: "not_connected".to_string(),
                    name: None,
                }
            }
        };

        HardwareStatusSnapshot {
            init_done,
            hostname: hw.hostname.clone(),
            profile: hw.profile_name.clone(),
            audio: status_for(
                hw.device.is_some(),
                hw.device.as_ref().map(|d| d.to_string()),
            ),
            midi: status_for(
                hw.midi_device.is_some(),
                hw.midi_device.as_ref().map(|d| d.to_string()),
            ),
            dmx: status_for(
                hw.dmx_engine.is_some(),
                hw.dmx_engine.as_ref().map(|_| "DMX Engine".to_string()),
            ),
            trigger: status_for(
                hw.trigger_engine.is_some(),
                hw.trigger_engine
                    .as_ref()
                    .map(|_| "Trigger Engine".to_string()),
            ),
        }
    }

    /// Processes a MIDI event for triggered samples.
    /// This should be called by the MIDI controller when events are received.
    /// Uses std::sync::RwLock for minimal latency (no async overhead).
    pub fn process_sample_trigger(&self, raw_event: &[u8]) {
        let sample_engine = self.hardware.read().sample_engine.clone();
        if let Some(ref sample_engine) = sample_engine {
            let engine = sample_engine.read();
            engine.process_midi_event(raw_event);
        }
    }

    /// Loads the sample configuration for a song.
    /// This preloads samples for the song so they're ready for instant playback.
    /// Note: Active voices continue playing through song transitions.
    fn load_song_samples(&self, song: &Song) {
        let sample_engine = self.hardware.read().sample_engine.clone();
        if let Some(ref sample_engine) = sample_engine {
            // Load the new song's sample config if it has one
            let samples_config = song.samples_config();
            if !samples_config.samples().is_empty() || !samples_config.sample_triggers().is_empty()
            {
                let mut engine = sample_engine.write();
                if let Err(e) = engine.load_song_config(samples_config, song.base_path()) {
                    warn!(
                        song = song.name(),
                        error = %e,
                        "Failed to load song sample config"
                    );
                } else {
                    info!(
                        song = song.name(),
                        samples = samples_config.samples().len(),
                        triggers = samples_config.sample_triggers().len(),
                        "Loaded song sample config"
                    );
                }
            }
        }
    }

    /// Stops all triggered sample playback.
    pub fn stop_samples(&self) {
        let sample_engine = self.hardware.read().sample_engine.clone();
        if let Some(ref sample_engine) = sample_engine {
            let engine = sample_engine.read();
            engine.stop_all();
        }
    }

    /// Gets the DMX engine currently in use by the player (for testing).
    #[cfg(test)]
    pub fn dmx_engine(&self) -> Option<Arc<dmx::engine::Engine>> {
        self.hardware.read().dmx_engine.clone()
    }

    /// Gets all cues from the current song's lighting timeline.
    pub fn get_cues(&self) -> Vec<(Duration, usize)> {
        let dmx_engine = self.hardware.read().dmx_engine.clone();
        if let Some(ref dmx_engine) = dmx_engine {
            dmx_engine.get_timeline_cues()
        } else {
            Vec::new()
        }
    }

    /// Returns handles needed for reading lighting state, or None if no DMX engine is configured.
    pub fn broadcast_handles(&self) -> Option<dmx::engine::BroadcastHandles> {
        self.hardware
            .read()
            .dmx_engine
            .clone()
            .map(|e| e.broadcast_handles())
    }

    /// Stores the broadcast channel and wires it to the DMX engine if one exists.
    /// If the DMX engine hasn't initialized yet, the channel is stored and will
    /// be wired when the engine comes up during async init.
    pub fn set_broadcast_tx(&self, tx: tokio::sync::broadcast::Sender<String>) {
        let dmx_engine = self.hardware.read().dmx_engine.clone();
        if let Some(ref engine) = dmx_engine {
            engine.set_broadcast_tx(tx.clone());
        }
        *self.broadcast_tx.lock() = Some(tx);
    }

    /// Stores the state sampler watch sender. When the DMX engine comes up
    /// during async init, the sampler will be started using this sender.
    pub fn set_state_tx(&self, tx: tokio::sync::watch::Sender<Arc<crate::state::StateSnapshot>>) {
        *self.state_tx.lock() = Some(Arc::new(tx));
    }

    /// Sets the config store on the player. Called once after startup.
    pub fn set_config_store(&self, store: Arc<config::ConfigStore>) {
        *self.config_store.lock() = Some(store);
    }

    /// Returns the config store, if one has been set.
    pub fn config_store(&self) -> Option<Arc<config::ConfigStore>> {
        self.config_store.lock().clone()
    }

    /// Reinitializes all hardware devices from the current config.
    ///
    /// Rejects the request if the player is currently playing. Cancels any
    /// in-flight init, resets hardware to empty, and spawns a new async init
    /// round. Returns immediately — does not wait for devices to be found.
    pub async fn reload_hardware(self: &Arc<Self>) -> Result<(), Box<dyn Error>> {
        if self.is_playing().await {
            return Err("Cannot reload hardware during playback".into());
        }

        let config = self
            .config_store()
            .ok_or("No config store available")?
            .read_config()
            .await;

        // Cancel the previous init round.
        {
            let mut cancel = self.init_cancel.lock();
            cancel.cancel();
            *cancel = CancellationToken::new();
        }

        // Reset hardware to empty.
        *self.hardware.write() = HardwareState {
            device: None,
            mappings: None,
            midi_device: None,
            dmx_engine: None,
            sample_engine: None,
            trigger_engine: None,
            clock_source: ClockSource::Wall,
            song_change_notifiers: Vec::new(),
            profile_name: None,
            hostname: None,
        };
        self.init_done_tx.send_modify(|v| *v = false);

        // Spawn new async init.
        let init_player = self.clone();
        let bp = self.base_path.clone();
        tokio::spawn(async move {
            init_player.init_hardware_async(config, bp).await;
        });

        info!("Hardware reload initiated");
        Ok(())
    }

    /// Starts controllers from the given config. Called at startup and on reload.
    /// Requires `Arc<Player>` because controllers hold a reference to the player.
    pub fn start_controllers(self: &Arc<Self>, config: Vec<config::Controller>) {
        // Shut down any existing controllers.
        if let Some(old) = self.controller.lock().take() {
            info!("Shutting down existing controllers");
            old.shutdown();
        }

        if config.is_empty() {
            info!("No controllers configured");
            return;
        }

        let controller = crate::controller::Controller::new(config, Arc::clone(self));
        *self.controller.lock() = Some(controller);
        info!("Controllers started");
    }

    /// Reloads controllers from the current config store.
    /// Requires `Arc<Player>` because controllers hold a reference to the player.
    pub async fn reload_controllers(self: &Arc<Self>) -> Result<(), Box<dyn Error>> {
        let config = self
            .config_store()
            .ok_or("No config store available")?
            .read_config()
            .await;

        let hostname = config::resolve_hostname();
        let controllers = config
            .profiles(&hostname)
            .first()
            .map(|p| p.controllers().to_vec())
            .unwrap_or_default();

        self.start_controllers(controllers);
        Ok(())
    }

    /// Returns the status of all active controllers.
    pub fn controller_statuses(&self) -> Vec<crate::controller::ControllerStatus> {
        match self.controller.lock().as_ref() {
            Some(controller) => controller.statuses().to_vec(),
            None => vec![],
        }
    }

    /// Shuts down all active controllers. Called during process shutdown.
    pub fn shutdown_controllers(&self) {
        if let Some(controller) = self.controller.lock().take() {
            controller.shutdown();
        }
    }

    /// Reports status as MIDI events.
    async fn report_status(&self, status_events: StatusEvents) {
        let _enter = self.span.enter();
        info!("Reporting status");

        let midi_device = self
            .hardware
            .read()
            .midi_device
            .clone()
            .expect("MIDI device must be present for status reporting");
        let join = self.join.clone();

        // This thread will run until the process is terminated.
        let _join_handle = tokio::spawn(async move {
            loop {
                {
                    let join = join.lock().await;

                    let emit_result: Result<(), Box<dyn Error>> = if join.is_none() {
                        status_events
                            .idling_events
                            .iter()
                            .try_for_each(|event| midi_device.emit(Some(*event)))
                    } else {
                        status_events
                            .playing_events
                            .iter()
                            .try_for_each(|event| midi_device.emit(Some(*event)))
                    };

                    if let Err(err) = emit_result {
                        error!(err = err.as_ref(), "error emitting status event")
                    }
                }

                tokio::time::sleep(Duration::from_secs(1)).await;

                {
                    let status_event_emit_result: Result<(), Box<dyn Error>> = status_events
                        .off_events
                        .iter()
                        .try_for_each(|event| midi_device.emit(Some(*event)));

                    if let Err(err) = status_event_emit_result {
                        error!(err = err.as_ref(), "error emitting off status event");
                    }
                }

                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        });
    }

    /// Plays a specific song by name, starting from the given time.
    /// Switches to the all_songs playlist (session-only, not persisted) and
    /// navigates to the song before calling play_from.
    /// Returns an error if the song is not found.
    pub async fn play_song_from(
        &self,
        song_name: &str,
        start_time: Duration,
    ) -> Result<Option<Arc<Song>>, Box<dyn Error>> {
        // Reject playback if hardware hasn't finished initializing.
        if !*self.init_done_tx.borrow() {
            return Err("Hardware is still initializing".into());
        }

        let mut join = self.join.lock().await;
        if join.is_some() {
            info!("Player is already playing a song.");
            return Ok(None);
        }

        let all_songs = self.get_all_songs_playlist();
        if all_songs.navigate_to(song_name).is_none() {
            return Err(format!("Song '{}' not found", song_name).into());
        }
        *self.active_playlist.write() = "all_songs".to_string();

        // Start playback with the lock already held.
        self.play_from_locked(start_time, &mut join).await
    }

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
    async fn play_from_locked(
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
                loop_break: self.loop_break.clone(),
                active_section: self.active_section.clone(),
                section_loop_break: self.section_loop_break.clone(),
                loop_time_consumed: self.loop_time_consumed.clone(),
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

    fn play_files(ctx: PlaybackContext) {
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
            loop_break,
            active_section,
            section_loop_break,
            loop_time_consumed,
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
            let ready_tx = ready_tx.clone();
            let clock = clock.clone();
            let loop_break = loop_break.clone();
            let active_section = active_section.clone();
            let section_loop_break = section_loop_break.clone();
            let loop_time_consumed = loop_time_consumed.clone();
            expected_ready += 1;

            Some(thread::spawn(move || {
                let song_name = song.name().to_string();
                let result = device.play_from(
                    song,
                    &mappings,
                    cancel_handle,
                    ready_tx,
                    clock,
                    start_time,
                    loop_break,
                    active_section,
                    section_loop_break,
                    loop_time_consumed,
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
            let ready_tx = ready_tx.clone();
            let loop_break = loop_break.clone();
            let active_section = active_section.clone();
            let section_loop_break = section_loop_break.clone();
            expected_ready += 1;

            thread::spawn(move || {
                let song_name = song.name().to_string();

                if let Err(e) = dmx::engine::Engine::play(
                    dmx_engine,
                    song,
                    cancel_handle,
                    ready_tx,
                    start_time,
                    clock,
                    loop_break,
                    active_section,
                    section_loop_break,
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
            let ready_tx = ready_tx.clone();
            let clock = clock.clone();
            let loop_break = loop_break.clone();
            let active_section = active_section.clone();
            let section_loop_break = section_loop_break.clone();
            expected_ready += 1;

            Some(thread::spawn(move || {
                let song_name = song.name().to_string();

                if let Err(e) = midi_device.play_from(
                    song,
                    cancel_handle,
                    ready_tx,
                    start_time,
                    clock,
                    loop_break,
                    active_section,
                    section_loop_break,
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

    /// Navigates the playlist in the given direction, emitting the song-change
    /// event. Returns the current song if the player is active.
    ///
    /// Holds the join lock across the entire operation to prevent a concurrent
    /// `play_from()` from starting between the is-playing check and the
    /// playlist position advance.
    async fn navigate(&self, direction: PlaylistDirection) -> Option<Arc<Song>> {
        let join = self.join.lock().await;
        if join.is_some() {
            // If the current song is looping, break out immediately with crossfade.
            if self.is_current_song_looping() {
                info!("Breaking out of song loop via {} navigation.", direction);
                self.fade_out_current_audio();
                self.loop_break.store(true, Ordering::Relaxed);
                if let Some(ref handles) = *join {
                    handles.cancel.cancel();
                }
                return self.get_playlist().current();
            }
            let current = self.get_playlist().current();
            if let Some(ref song) = current {
                info!(
                    current_song = song.name(),
                    "Can't go to {}, player is active.", direction
                );
            }
            return current;
        }
        let playlist = self.get_playlist();
        let song = match direction {
            PlaylistDirection::Next => playlist.next()?,
            PlaylistDirection::Prev => playlist.prev()?,
        };
        self.emit_song_change(&song);
        drop(join);
        self.load_song_samples(&song);
        Some(song)
    }

    /// Next goes to the next entry in the playlist.
    pub async fn next(&self) -> Option<Arc<Song>> {
        self.navigate(PlaylistDirection::Next).await
    }

    /// Prev goes to the previous entry in the playlist.
    pub async fn prev(&self) -> Option<Arc<Song>> {
        self.navigate(PlaylistDirection::Prev).await
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

    /// Switches the active playlist by name. Returns an error if the name
    /// doesn't exist in the map or if the player is currently playing.
    /// Switching to "all_songs" is session-only (not persisted to config).
    pub async fn switch_to_playlist(&self, name: &str) -> Result<(), String> {
        {
            let join = self.join.lock().await;
            if join.is_some() {
                if let Some(current) = self.get_playlist().current() {
                    info!(
                        current_song = current.name(),
                        "Can't switch to {}, player is active.", name
                    );
                }
                return Err("Cannot switch playlist while playing".to_string());
            }
        }

        // Validate the name exists.
        {
            let playlists = self.playlists.read();
            if !playlists.contains_key(name) {
                return Err(format!("Playlist '{}' not found", name));
            }
        }

        *self.active_playlist.write() = name.to_string();

        // Persist the choice to the config store, unless it's "all_songs" (session-only).
        if name != "all_songs" {
            *self.persisted_playlist.write() = name.to_string();
            if let Some(store) = self.config_store() {
                if let Err(e) = store.set_active_playlist(name.to_string()).await {
                    warn!("Failed to persist active playlist: {}", e);
                }
            }
        }

        if let Some(song) = self.get_playlist().current() {
            self.emit_song_change(&song);
        }

        Ok(())
    }

    /// Returns the persisted active playlist name (the last non-all_songs choice).
    /// This is what MIDI/OSC `Playlist` action uses to "go back to my real playlist".
    pub fn persisted_playlist_name(&self) -> String {
        self.persisted_playlist.read().clone()
    }

    /// Returns a sorted list of all playlist names.
    pub fn list_playlists(&self) -> Vec<String> {
        let playlists = self.playlists.read();
        let mut names: Vec<String> = playlists.keys().cloned().collect();
        names.sort();
        names
    }

    /// Returns a snapshot of all playlists.
    pub fn playlists_snapshot(&self) -> HashMap<String, Arc<Playlist>> {
        self.playlists.read().clone()
    }

    /// Returns the track-to-output-channel mappings, if audio is configured.
    pub fn track_mappings(&self) -> Option<Arc<HashMap<String, Vec<u16>>>> {
        self.hardware.read().mappings.clone()
    }

    /// Returns the song registry from the all-songs playlist.
    pub fn songs(&self) -> Arc<Songs> {
        let playlists = self.playlists.read();
        playlists
            .get("all_songs")
            .expect("all_songs must always be present")
            .registry()
            .clone()
    }

    /// Gets the all-songs playlist (every song in the registry).
    pub fn get_all_songs_playlist(&self) -> Arc<Playlist> {
        let playlists = self.playlists.read();
        playlists
            .get("all_songs")
            .expect("all_songs must always be present")
            .clone()
    }

    /// Gets the current playlist used by the player.
    pub fn get_playlist(&self) -> Arc<Playlist> {
        let name = self.active_playlist.read().clone();
        let playlists = self.playlists.read();
        playlists
            .get(&name)
            .or_else(|| playlists.get("all_songs"))
            .expect("all_songs must always be present")
            .clone()
    }

    /// Reinitializes all song-related state by rescanning songs from disk and
    /// rebuilding all playlists. Call this after any mutation that affects songs
    /// (import, create, config edit, etc.).
    pub fn reload_songs(
        &self,
        songs_path: &std::path::Path,
        playlists_dir: Option<&std::path::Path>,
        legacy_playlist_path: Option<&std::path::Path>,
    ) {
        let new_songs = match songs::get_all_songs(songs_path) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to rescan songs: {}", e);
                return;
            }
        };

        let new_playlists =
            match load_playlists(playlists_dir, legacy_playlist_path, new_songs.clone()) {
                Ok(p) => p,
                Err(e) => {
                    warn!("Failed to rebuild playlists: {}", e);
                    return;
                }
            };

        // Preserve active playlist name if it still exists; fall back to all_songs.
        {
            let mut active = self.active_playlist.write();
            if !new_playlists.contains_key(active.as_str()) {
                *active = "all_songs".to_string();
            }
        }

        *self.playlists.write() = new_playlists;
        info!("Reloaded song state");
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
            *active = Some(SectionBounds {
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
    pub fn active_section(&self) -> Option<SectionBounds> {
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
                Some(SectionBounds {
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
    fn play_notification(&self, event: crate::notification::NotificationEvent) {
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
    fn fade_out_current_audio(&self) {
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

    /// Returns true if the player is in locked mode (state-altering operations blocked).
    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::Relaxed)
    }

    /// Sets the locked state. When locked, state-altering operations are rejected
    /// but playback controls continue to work.
    pub fn set_locked(&self, locked: bool) {
        self.locked.store(locked, Ordering::Relaxed);
    }

    /// Returns the effect engine, if a DMX engine is configured.
    #[cfg(test)]
    pub fn effect_engine(&self) -> Option<Arc<parking_lot::Mutex<crate::lighting::EffectEngine>>> {
        self.hardware
            .read()
            .dmx_engine
            .clone()
            .map(|e| e.effect_engine())
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

    /// Gets a formatted string listing all active lighting effects
    pub fn format_active_effects(&self) -> Option<String> {
        self.hardware
            .read()
            .dmx_engine
            .clone()
            .map(|engine| engine.format_active_effects())
    }

    /// Emits the per-song MIDI event and notifies all song-change notifiers.
    fn emit_song_change(&self, song: &Song) {
        let hw = self.hardware.read();
        let midi_device = hw.midi_device.clone();
        let notifiers = hw.song_change_notifiers.clone();
        drop(hw);

        if let Some(ref device) = midi_device {
            if let Err(e) = device.emit(song.midi_event()) {
                error!("Error emitting MIDI event: {:?}", e);
            }
        }
        for notifier in &notifiers {
            notifier.notify(song);
        }
    }
}

/// Initializes the sample engine if the audio device supports mixing and source input.
fn init_sample_engine(
    device: &Option<Arc<dyn audio::Device>>,
    mappings: &Option<HashMap<String, Vec<u16>>>,
    resolved_audio: Option<&config::Audio>,
    config: &config::Player,
    profile: &config::Profile,
    base_path: Option<&Path>,
) -> Option<Arc<RwLock<SampleEngine>>> {
    let (mixer, source_tx) = device
        .as_ref()
        .and_then(|d| d.mixer().and_then(|m| d.source_sender().map(|s| (m, s))))?;

    let max_voices = config.max_sample_voices();
    let buffer_size = resolved_audio.map(|a| a.buffer_size()).unwrap_or(1024);
    let track_mappings = mappings.as_ref().cloned().unwrap_or_default();
    let mut engine = SampleEngine::new(mixer, source_tx, max_voices, buffer_size, track_mappings);

    // Load global samples config if available
    if let Some(base_path) = base_path {
        match config.samples_config(base_path) {
            Ok(mut samples_config) => {
                // Add MIDI triggers from profile's trigger config
                if let Some(trigger_config) = profile.trigger() {
                    samples_config.add_triggers(trigger_config.midi_triggers());
                }
                if let Err(e) = engine.load_global_config(&samples_config, base_path) {
                    warn!(error = %e, "Failed to load global samples config");
                }
            }
            Err(e) => {
                warn!(error = %e, "Failed to parse samples config");
            }
        }
    }

    Some(Arc::new(RwLock::new(engine)))
}

/// Initializes the trigger engine if configured and sample engine is available.
/// Unlike audio/MIDI devices, triggers are non-essential — fail immediately
/// rather than retrying indefinitely.
fn init_trigger_engine(
    profile: &config::Profile,
    sample_engine: &Option<Arc<RwLock<SampleEngine>>>,
) -> Result<Option<Arc<TriggerEngine>>, Box<dyn Error>> {
    let (trigger_config, sample_engine) = match (
        profile.trigger().filter(|t| t.has_audio_inputs()),
        sample_engine,
    ) {
        (Some(tc), Some(se)) => (tc, se),
        _ => return Ok(None),
    };

    match TriggerEngine::new(trigger_config) {
        Ok(engine) => {
            let engine: Arc<TriggerEngine> = Arc::new(engine);

            // Spawn a forwarding thread: reads TriggerActions and dispatches
            // to the sample engine. When the TriggerEngine drops, the sender
            // closes and the receiver returns Err, ending the thread.
            let receiver = engine.subscribe();
            let se = sample_engine.clone();
            thread::Builder::new()
                .name("trigger-fwd".to_string())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        while let Ok(action) = receiver.recv() {
                            match action {
                                samples::TriggerAction::Trigger(event) => {
                                    let engine = se.read();
                                    engine.trigger(&event);
                                }
                                samples::TriggerAction::Release { group } => {
                                    let engine = se.read();
                                    engine.release(&group);
                                }
                            }
                        }
                    }));
                    if result.is_err() {
                        error!("Trigger forwarding thread panicked");
                    }
                    info!("Trigger forwarding thread exiting");
                })?;

            Ok(Some(engine))
        }
        Err(e) => {
            warn!(error = %e, "Failed to initialize trigger engine, continuing without triggers");
            Ok(None)
        }
    }
}

/// Describes how to report status via MIDI.
pub struct StatusEvents {
    /// The events to emit to clear the status.
    off_events: Vec<LiveEvent<'static>>,
    /// The events to emit to indicate that the player is idling and waiting for input.
    idling_events: Vec<LiveEvent<'static>>,
    /// The events to emit to indicate that the player is currently playing.
    playing_events: Vec<LiveEvent<'static>>,
}

impl StatusEvents {
    /// Creates a new status events configuration.
    pub fn new(
        config: Option<config::StatusEvents>,
    ) -> Result<Option<StatusEvents>, Box<dyn Error>> {
        Ok(match config {
            Some(config) => Some(StatusEvents {
                off_events: config.off_events()?,
                idling_events: config.idling_events()?,
                playing_events: config.playing_events()?,
            }),
            None => None,
        })
    }
}

/// The result of receiving a playback completion signal.
#[derive(Debug)]
enum PlaybackResult {
    Success,
    Failed(String),
    SenderDropped,
}

/// What the cleanup task should do after playback finishes.
#[derive(Debug, PartialEq)]
enum CleanupAction {
    AdvancePlaylist,
    StopCancelled,
    /// Loop was broken via play/next — advance playlist and auto-play next song.
    LoopBreakAndPlay,
}

/// Decides whether to advance the playlist or stop after playback finishes.
fn decide_cleanup_action(
    result: PlaybackResult,
    cancelled: bool,
    loop_broken: bool,
) -> CleanupAction {
    // Loop break takes priority over cancel — we intentionally cancel
    // playback to break out of a loop immediately, but the intent is
    // to advance and play, not to stop.
    if loop_broken {
        return CleanupAction::LoopBreakAndPlay;
    }
    if cancelled {
        return CleanupAction::StopCancelled;
    }
    match &result {
        PlaybackResult::Failed(e) => {
            warn!(
                err = %e,
                "Advancing playlist despite playback failure so user is not stuck"
            );
        }
        PlaybackResult::SenderDropped => {
            error!("Error receiving playback signal (receiver dropped)");
        }
        PlaybackResult::Success => {}
    }
    CleanupAction::AdvancePlaylist
}

/// Resolves the final playback outcome from the audio thread result.
fn resolve_playback_outcome(
    has_audio: bool,
    audio_outcome: Option<Result<(), String>>,
) -> Result<(), String> {
    if has_audio {
        audio_outcome.unwrap_or_else(|| {
            warn!(
                "Audio thread did not set outcome (e.g. panicked before setting); \
                 treating as success so playlist is not stuck"
            );
            Ok(())
        })
    } else {
        Ok(())
    }
}

/// Loads all playlists from a directory and/or legacy playlist path.
/// Always includes the computed "all_songs" playlist.
pub fn load_playlists(
    playlists_dir: Option<&Path>,
    legacy_playlist_path: Option<&Path>,
    songs: Arc<Songs>,
) -> Result<HashMap<String, Arc<Playlist>>, Box<dyn Error>> {
    let mut playlists = HashMap::new();

    // Always create all_songs.
    playlists.insert(
        "all_songs".to_string(),
        playlist::from_songs(songs.clone())?,
    );

    // Load playlists from directory.
    if let Some(dir) = playlists_dir {
        if dir.is_dir() {
            let mut entries: Vec<_> = std::fs::read_dir(dir)?
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path().is_file()
                        && e.path()
                            .extension()
                            .is_some_and(|ext| ext == "yaml" || ext == "yml")
                })
                .collect();
            entries.sort_by_key(|e| e.file_name());

            for entry in entries {
                let path = entry.path();
                let name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or_default()
                    .to_string();
                if name.is_empty() || name == "all_songs" {
                    continue;
                }
                match config::Playlist::deserialize(&path) {
                    Ok(playlist_config) => {
                        match Playlist::new(&name, &playlist_config, songs.clone()) {
                            Ok(pl) => {
                                info!(name = %name, "Loaded playlist from directory");
                                playlists.insert(name, pl);
                            }
                            Err(e) => {
                                warn!(name = %name, error = %e, "Playlist references missing songs, skipping");
                            }
                        }
                    }
                    Err(e) => {
                        warn!(path = ?path, error = %e, "Failed to parse playlist file, skipping");
                    }
                }
            }
        }
    }

    // Load legacy playlist file as name "playlist" (if not already loaded from dir).
    if let Some(legacy_path) = legacy_playlist_path {
        if !playlists.contains_key("playlist") {
            match config::Playlist::deserialize(legacy_path) {
                Ok(playlist_config) => {
                    match Playlist::new("playlist", &playlist_config, songs.clone()) {
                        Ok(pl) => {
                            info!("Loaded legacy playlist");
                            playlists.insert("playlist".to_string(), pl);
                        }
                        Err(e) => {
                            info!("Legacy playlist references missing songs ({}); skipping", e);
                        }
                    }
                }
                Err(_) => {
                    info!("Legacy playlist file not found or invalid; skipping");
                }
            }
        }
    }

    Ok(playlists)
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, error::Error, fs, path::Path, sync::Arc};

    use crate::{
        config,
        playlist::Playlist,
        songs,
        testutil::{eventually, eventually_async},
    };

    use super::*;

    /// Test helper: builds a playlists map from a single playlist + songs registry.
    fn test_playlists(
        playlist: Arc<Playlist>,
        songs: Arc<Songs>,
    ) -> HashMap<String, Arc<Playlist>> {
        let mut playlists = HashMap::new();
        playlists.insert(
            "all_songs".to_string(),
            playlist::from_songs(songs).unwrap(),
        );
        playlists.insert("playlist".to_string(), playlist);
        playlists
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_player() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let player = Player::new(
            test_playlists(playlist, songs.clone()),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                Some(config::Midi::new("mock-midi-device", None)),
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;
        let midi_device = player
            .midi_device()
            .expect("MIDI should be present")
            .to_mock()?;

        // Direct the player.
        println!("Playlist -> Song 1");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        player.next().await;
        println!("Playlist -> Song 3");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 3");

        player.prev().await;
        println!("Playlist -> Song 1");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        println!("Switch to AllSongs");
        player.switch_to_playlist("all_songs").await.unwrap();
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        player.next().await;
        println!("AllSongs -> Song 10");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 10");

        // No emitted events yet
        assert!(midi_device.get_emitted_event().is_none());

        player.next().await;
        println!("AllSongs -> Song 2");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 2");

        let expected_event = midly::live::LiveEvent::Midi {
            channel: 15.into(),
            message: midly::MidiMessage::ProgramChange { program: 0.into() },
        };
        let actual_event_buf = midi_device
            .get_emitted_event()
            .expect("expected emitted event");
        let actual_event = midly::live::LiveEvent::parse(&actual_event_buf)?;
        assert_eq!(expected_event, actual_event);

        midi_device.reset_emitted_event();

        player.next().await;
        println!("AllSongs -> Song 3");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 3");

        assert!(midi_device.get_emitted_event().is_none());

        player.switch_to_playlist("playlist").await.unwrap();
        println!("Switch to Playlist");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        player.next().await;
        println!("Playlist -> Song 3");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 3");

        player.play().await?;

        // Playlist should have moved to next song.
        eventually(
            || player.get_playlist().current().unwrap().name() == "Song 5",
            format!(
                "Song never moved to next, on song {}",
                player.get_playlist().current().unwrap().name()
            )
            .as_str(),
        );

        // Next song should have emitted an event.
        let expected_event = midly::live::LiveEvent::Midi {
            channel: 15.into(),
            message: midly::MidiMessage::ProgramChange { program: 5.into() },
        };
        let actual_event_buf = midi_device
            .get_emitted_event()
            .expect("expected emitted event");
        let actual_event = midly::live::LiveEvent::parse(&actual_event_buf)?;
        assert_eq!(expected_event, actual_event);

        midi_device.reset_emitted_event();

        // Play a song and cancel it.
        player.play().await?;
        println!("Play Song 5.");
        eventually(|| device.is_playing(), "Song never started playing");

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");

        // Player should not have moved to the next song.
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 5");

        assert!(midi_device.get_emitted_event().is_none());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_player_rejects_invalid_lighting_shows() -> Result<(), Box<dyn Error>> {
        // Create a temporary directory for test files
        let temp_dir = tempfile::tempdir()?;
        let temp_path = temp_dir.path();

        // Create a valid lighting show file with invalid group reference
        let lighting_show_content = r#"show "Test Show" {
    @00:00.000
    invalid_group: static color: "blue", duration: 5s, dimmer: 60%
}"#;
        let lighting_file = temp_path.join("invalid_show.light");
        fs::write(&lighting_file, lighting_show_content)?;

        // Create a song with the invalid lighting show
        let song_config = config::Song::new(
            "Test Song",
            None,
            None,
            None,
            None,
            Some(vec![config::LightingShow::new(
                lighting_file
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string(),
            )]),
            vec![],
            HashMap::new(),
            Vec::new(),
        );

        // Create a lighting config with valid groups (but not "invalid_group")
        let mut groups = HashMap::new();
        groups.insert(
            "front_wash".to_string(),
            config::lighting::LogicalGroup::new(
                "front_wash".to_string(),
                vec![config::lighting::GroupConstraint::AllOf(vec![
                    "wash".to_string(),
                    "front".to_string(),
                ])],
            ),
        );
        let lighting_config =
            config::Lighting::new(Some("test_venue".to_string()), None, Some(groups), None);

        // Create DMX config with lighting
        let dmx_config = config::Dmx::new(
            Some(1.0),
            Some("0s".to_string()),
            Some(9090),
            vec![config::Universe::new(1, "test_universe".to_string())],
            Some(lighting_config),
        );

        // Create a simple playlist with one song
        let playlist_songs = vec!["Test Song".to_string()];
        let playlist_config = config::Playlist::new(&playlist_songs);
        let song = songs::Song::new(temp_path, &song_config)?;
        let songs_map = HashMap::from([("Test Song".to_string(), Arc::new(song))]);
        let songs = Arc::new(songs::Songs::new(songs_map));
        let playlist = Playlist::new("Test Playlist", &playlist_config, songs.clone())?;

        // Create player with DMX engine that has lighting config
        let player = Player::new(
            test_playlists(playlist, songs.clone()),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                Some(config::Midi::new("mock-midi-device", None)),
                Some(dmx_config),
                HashMap::new(),
                temp_path.to_str().unwrap(),
            ),
            Some(temp_path),
        )?;
        player.await_hardware_ready().await;

        // Try to play the song - it should fail due to invalid lighting show
        let result = player.play().await;
        assert!(
            result.is_err(),
            "Player should reject song with invalid lighting show"
        );

        Ok(())
    }

    /// Flexible helper to create a player with the standard test assets and
    /// optional subsystem configs. Waits for hardware init to complete.
    async fn make_test_player_with_config(
        audio: Option<config::Audio>,
        midi: Option<config::Midi>,
        dmx: Option<config::Dmx>,
    ) -> Result<Arc<Player>, Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let player = Player::new(
            test_playlists(playlist, songs),
            "playlist".to_string(),
            &config::Player::new(vec![], audio, midi, dmx, HashMap::new(), "assets/songs"),
            None,
        )?;
        player.await_hardware_ready().await;
        Ok(player)
    }

    /// Helper to create a player with the standard test assets (audio + MIDI).
    async fn make_test_player() -> Result<Arc<Player>, Box<dyn Error>> {
        make_test_player_with_config(
            Some(config::Audio::new("mock-device")),
            Some(config::Midi::new("mock-midi-device", None)),
            None,
        )
        .await
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stop_when_not_playing() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // stop() when nothing is playing should return None.
        let result = player.stop().await;
        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_is_playing() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        assert!(!player.is_playing().await);

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        assert!(player.is_playing().await);

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_elapsed_stopped() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // elapsed() when not playing should be Ok(None).
        let elapsed = player.elapsed().await?;
        assert!(elapsed.is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_elapsed_while_playing() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // play_start_time is set inside play_files after clock.start(),
        // so there may be a brief gap after is_playing becomes true.
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            if player.elapsed().await?.is_some() {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "elapsed should have a value while playing"
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_concurrent_play_returns_none() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        // First play should succeed.
        let result = player.play().await?;
        assert!(result.is_some());
        eventually(|| device.is_playing(), "Song never started playing");

        // Second play while already playing should return Ok(None).
        let result = player.play().await?;
        assert!(result.is_none(), "play() while playing should return None");

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_navigation() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        let song = player.next().await.unwrap();
        assert_eq!(song.name(), "Song 3");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 3");

        let song = player.prev().await.unwrap();
        assert_eq!(song.name(), "Song 1");
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_switch_playlists() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        assert_eq!(player.get_playlist().name(), "playlist");
        player.switch_to_playlist("all_songs").await.unwrap();
        assert_eq!(player.get_playlist().name(), "all_songs");
        player.switch_to_playlist("playlist").await.unwrap();
        assert_eq!(player.get_playlist().name(), "playlist");

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_all_songs_playlist() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let all = player.get_all_songs_playlist();
        assert_eq!(all.name(), "all_songs");
        assert!(!all.songs().is_empty());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_format_active_effects_no_dmx() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        // No DMX engine configured, should return None.
        assert!(player.format_active_effects().is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_dmx_engine_none_without_config() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        assert!(player.dmx_engine().is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_cues_empty_without_lighting() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let cues = player.get_cues();
        assert!(cues.is_empty());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_player_rejects_song_with_multiple_invalid_groups() -> Result<(), Box<dyn Error>> {
        // Create a temporary directory for test files
        let temp_dir = tempfile::tempdir()?;
        let temp_path = temp_dir.path();

        // Create a valid lighting show file with multiple invalid group references
        let lighting_show_content = r#"show "Test Show" {
    @00:00.000
    invalid_group_1: static color: "blue", duration: 5s, dimmer: 60%
    invalid_group_2: static color: "red", duration: 5s, dimmer: 80%
}"#;
        let lighting_file = temp_path.join("invalid_groups.light");
        fs::write(&lighting_file, lighting_show_content)?;

        // Create a song with the invalid lighting show
        let song_config = config::Song::new(
            "Test Song",
            None,
            None,
            None,
            None,
            Some(vec![config::LightingShow::new(
                lighting_file
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string(),
            )]),
            vec![],
            HashMap::new(),
            Vec::new(),
        );

        // Create a lighting config with valid groups (but not the invalid ones)
        let mut groups = HashMap::new();
        groups.insert(
            "front_wash".to_string(),
            config::lighting::LogicalGroup::new(
                "front_wash".to_string(),
                vec![config::lighting::GroupConstraint::AllOf(vec![
                    "wash".to_string(),
                    "front".to_string(),
                ])],
            ),
        );
        let lighting_config =
            config::Lighting::new(Some("test_venue".to_string()), None, Some(groups), None);

        // Create DMX config with lighting
        let dmx_config = config::Dmx::new(
            Some(1.0),
            Some("0s".to_string()),
            Some(9090),
            vec![config::Universe::new(1, "test_universe".to_string())],
            Some(lighting_config),
        );

        // Create a simple playlist with one song
        let playlist_songs = vec!["Test Song".to_string()];
        let playlist_config = config::Playlist::new(&playlist_songs);
        let song = songs::Song::new(temp_path, &song_config)?;
        let songs_map = HashMap::from([("Test Song".to_string(), Arc::new(song))]);
        let songs = Arc::new(songs::Songs::new(songs_map));
        let playlist = Playlist::new("Test Playlist", &playlist_config, songs.clone())?;

        // Create player with DMX engine that has lighting config
        let player = Player::new(
            test_playlists(playlist, songs.clone()),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                Some(config::Midi::new("mock-midi-device", None)),
                Some(dmx_config),
                HashMap::new(),
                temp_path.to_str().unwrap(),
            ),
            Some(temp_path),
        )?;
        player.await_hardware_ready().await;

        // Try to play the song - it should fail due to invalid lighting show groups
        let result = player.play().await;
        assert!(
            result.is_err(),
            "Player should reject song with invalid lighting show groups"
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stop_returns_current_song() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        let song = player.stop().await;
        assert!(song.is_some(), "stop() should return the current song");
        assert_eq!(song.unwrap().name(), "Song 1");

        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_play_from_nonzero_start() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        let result = player.play_from(Duration::from_millis(100)).await?;
        assert!(result.is_some(), "play_from should succeed");

        eventually(|| device.is_playing(), "Song never started playing");

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_play_after_stop_restarts() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        // First play/stop cycle.
        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");
        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");

        // Second play should succeed (stop_run flag was reset).
        let result = player.play().await?;
        assert!(
            result.is_some(),
            "play() after stop should start a new song"
        );
        eventually(|| device.is_playing(), "Song never restarted");

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_audio_only_no_midi() -> Result<(), Box<dyn Error>> {
        let player =
            make_test_player_with_config(Some(config::Audio::new("mock-device")), None, None)
                .await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        assert!(
            player.midi_device().is_none(),
            "MIDI device should be absent"
        );

        // Song 2 has no midi_file, so barrier is audio-only.
        // Navigate to Song 2 (default playlist starts at Song 1).
        player.get_playlist().next(); // Song 3
        player.get_playlist().next(); // Song 5
                                      // Use the all songs playlist to reach Song 2 more easily.
        player.switch_to_playlist("all_songs").await.unwrap();
        // all_songs starts at Song 1, navigate to Song 2.
        player.get_playlist().next(); // Song 10
        player.get_playlist().next(); // Song 2
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 2");

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_midi_only_no_audio() -> Result<(), Box<dyn Error>> {
        let player = make_test_player_with_config(
            None,
            Some(config::Midi::new("mock-midi-device", None)),
            None,
        )
        .await?;

        assert!(
            player.audio_device().is_none(),
            "Audio device should be absent"
        );

        // Song 1 has midi_file. Barrier = 1 (MIDI only).
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        player.play().await?;

        // Natural finish: playlist should advance.
        eventually_async(
            || async { player.get_playlist().current().unwrap().name() != "Song 1" },
            "Playlist never advanced after MIDI-only playback",
        )
        .await;

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_no_subsystems_completes_immediately() -> Result<(), Box<dyn Error>> {
        // Build a Player directly with no subsystems to exercise the
        // num_barriers == 0 early-return in play_files().
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let devices = PlayerDevices {
            audio: None,
            mappings: None,
            midi: None,
            dmx_engine: None,
            sample_engine: None,
            trigger_engine: None,
        };
        let player = Player::new_with_devices(
            devices,
            test_playlists(playlist, songs),
            "playlist".to_string(),
            None,
        )?;

        assert!(player.audio_device().is_none());
        assert!(player.midi_device().is_none());
        assert!(player.dmx_engine().is_none());

        player.play().await?;

        // num_barriers == 0 → play_files returns immediately → playlist advances.
        eventually_async(
            || async { player.get_playlist().current().unwrap().name() != "Song 1" },
            "Playlist never advanced after no-subsystem playback",
        )
        .await;

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_natural_finish_clears_play_state() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // Song 1 is short (~0.7s). Let it finish naturally.
        player.play().await?;

        // Wait for natural finish: playlist advances.
        eventually_async(
            || async { !player.is_playing().await },
            "Player never stopped after natural finish",
        )
        .await;

        let elapsed = player.elapsed().await?;
        assert!(
            elapsed.is_none(),
            "elapsed() should be None after natural finish"
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_play_with_dmx_engine() -> Result<(), Box<dyn Error>> {
        let dmx_config = config::Dmx::new(
            None,
            None,
            Some(9090),
            vec![config::Universe::new(1, "test".to_string())],
            None,
        );
        let player = make_test_player_with_config(
            Some(config::Audio::new("mock-device")),
            None,
            Some(dmx_config),
        )
        .await?;

        assert!(
            player.dmx_engine().is_some(),
            "DMX engine should be present"
        );

        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_switch_playlist_while_playing_stays() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        assert_eq!(player.get_playlist().name(), "playlist");

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // Attempt switch while playing — should return error.
        let result = player.switch_to_playlist("all_songs").await;
        assert!(
            result.is_err(),
            "switch_to_playlist should fail while playing"
        );
        assert_eq!(
            player.get_playlist().name(),
            "playlist",
            "playlist should not change while playing"
        );

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_playlist_clamps_at_end_on_natural_finish() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // Navigate to Song 9 (last in playlist).
        // Playlist: Song 1, Song 3, Song 5, Song 7, Song 9
        player.next().await; // Song 3
        player.next().await; // Song 5
        player.next().await; // Song 7
        player.next().await; // Song 9
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 9");

        // Play Song 9 — short audio (0.5s), should finish naturally.
        player.play().await?;

        // After natural finish, playlist next() clamps at the last song.
        eventually_async(
            || async { !player.is_playing().await },
            "Player never stopped after Song 9 finished",
        )
        .await;

        // Playlist should still be at Song 9 (clamped, no wrap).
        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 9");

        Ok(())
    }

    // --- resolve_playback_outcome tests ---

    #[test]
    fn playback_outcome_no_audio() {
        assert_eq!(resolve_playback_outcome(false, None), Ok(()));
    }

    #[test]
    fn playback_outcome_audio_ok() {
        assert_eq!(resolve_playback_outcome(true, Some(Ok(()))), Ok(()));
    }

    #[test]
    fn playback_outcome_audio_err() {
        let err_msg = "device disconnected".to_string();
        assert_eq!(
            resolve_playback_outcome(true, Some(Err(err_msg.clone()))),
            Err(err_msg)
        );
    }

    #[test]
    fn playback_outcome_audio_none_panicked() {
        // Thread panicked before setting outcome — treated as success
        assert_eq!(resolve_playback_outcome(true, None), Ok(()));
    }

    // --- decide_cleanup_action tests ---

    #[test]
    fn cleanup_success_not_cancelled() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Success, false, false),
            CleanupAction::AdvancePlaylist
        );
    }

    #[test]
    fn cleanup_success_cancelled() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Success, true, false),
            CleanupAction::StopCancelled
        );
    }

    #[test]
    fn cleanup_failed_not_cancelled() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Failed("err".into()), false, false),
            CleanupAction::AdvancePlaylist
        );
    }

    #[test]
    fn cleanup_failed_cancelled() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Failed("err".into()), true, false),
            CleanupAction::StopCancelled
        );
    }

    #[test]
    fn cleanup_sender_dropped_not_cancelled() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::SenderDropped, false, false),
            CleanupAction::AdvancePlaylist
        );
    }

    #[test]
    fn cleanup_loop_broken() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Success, false, true),
            CleanupAction::LoopBreakAndPlay
        );
    }

    #[test]
    fn cleanup_loop_broken_takes_priority_over_cancel() {
        // If both cancelled and loop_broken, loop_broken wins — we intentionally
        // cancel to break out of the loop immediately, but the intent is to advance.
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Success, true, true),
            CleanupAction::LoopBreakAndPlay
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_dmx_utility_methods() -> Result<(), Box<dyn Error>> {
        let dmx_config = config::Dmx::new(
            None,
            None,
            Some(9090),
            vec![config::Universe::new(1, "test".to_string())],
            None,
        );
        let player = make_test_player_with_config(
            Some(config::Audio::new("mock-device")),
            None,
            Some(dmx_config),
        )
        .await?;

        // get_cues() with DMX engine present (no timeline loaded → empty)
        let cues = player.get_cues();
        assert!(cues.is_empty());

        // broadcast_handles() returns Some when DMX engine is present
        assert!(
            player.broadcast_handles().is_some(),
            "broadcast_handles should be Some with DMX engine"
        );

        // set_broadcast_tx() should not panic
        let (tx, _rx) = tokio::sync::broadcast::channel(1);
        player.set_broadcast_tx(tx);

        // effect_engine() returns Some when DMX engine is present
        assert!(
            player.effect_engine().is_some(),
            "effect_engine should be Some with DMX engine"
        );

        // format_active_effects() returns Some when DMX engine is present
        assert!(
            player.format_active_effects().is_some(),
            "format_active_effects should be Some with DMX engine"
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_track_mappings() -> Result<(), Box<dyn Error>> {
        // Player with audio → track_mappings is Some
        let player = make_test_player().await?;
        assert!(
            player.track_mappings().is_some(),
            "track_mappings should be Some when audio is configured"
        );

        // Player without audio → track_mappings is None
        let player = make_test_player_with_config(
            None,
            Some(config::Midi::new("mock-midi-device", None)),
            None,
        )
        .await?;
        assert!(
            player.track_mappings().is_none(),
            "track_mappings should be None without audio"
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_next_while_playing_returns_current() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        assert_eq!(player.get_playlist().current().unwrap().name(), "Song 1");

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // next() while playing should return the current song without advancing
        let song = player.next().await.unwrap();
        assert_eq!(
            song.name(),
            "Song 1",
            "next() while playing should return current song"
        );
        assert_eq!(
            player.get_playlist().current().unwrap().name(),
            "Song 1",
            "playlist should not advance while playing"
        );

        // prev() while playing should also return the current song
        let song = player.prev().await.unwrap();
        assert_eq!(
            song.name(),
            "Song 1",
            "prev() while playing should return current song"
        );

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[test]
    fn status_events_new_none() {
        let result = StatusEvents::new(None).unwrap();
        assert!(result.is_none());
    }

    /// Helper to create a player with no subsystems via new_with_devices.
    fn make_bare_player() -> Result<Player, Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let devices = PlayerDevices {
            audio: None,
            mappings: None,
            midi: None,
            dmx_engine: None,
            sample_engine: None,
            trigger_engine: None,
        };
        Player::new_with_devices(
            devices,
            test_playlists(playlist, songs),
            "playlist".to_string(),
            None,
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_process_sample_trigger_no_engine() -> Result<(), Box<dyn Error>> {
        let player = make_bare_player()?;
        player.process_sample_trigger(&[0x90, 60, 127]);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stop_samples_no_engine() -> Result<(), Box<dyn Error>> {
        let player = make_bare_player()?;
        player.stop_samples();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_broadcast_handles_no_dmx() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        assert!(player.broadcast_handles().is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_effect_engine_no_dmx() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        assert!(player.effect_engine().is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_set_broadcast_tx_no_dmx() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let (tx, _rx) = tokio::sync::broadcast::channel(1);
        player.set_broadcast_tx(tx);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn emit_song_change_no_device() -> Result<(), Box<dyn Error>> {
        let player = make_test_player_with_config(None, None, None).await?;
        let song = Song::new_for_test("test", &[]);
        // Should not panic when no MIDI device is configured.
        player.emit_song_change(&song);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_switch_to_playlist_while_playing_stays() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.switch_to_playlist("all_songs").await.unwrap();
        assert_eq!(player.get_playlist().name(), "all_songs");

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        let result = player.switch_to_playlist("playlist").await;
        assert!(
            result.is_err(),
            "switch_to_playlist should fail while playing"
        );
        assert_eq!(
            player.get_playlist().name(),
            "all_songs",
            "playlist should not change while playing"
        );

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_navigate_no_midi() -> Result<(), Box<dyn Error>> {
        let player = make_test_player_with_config(None, None, None).await?;

        player.next().await;
        let song = player.prev().await.unwrap();
        assert_eq!(song.name(), "Song 1");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_new_with_devices_all_none() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "test",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let devices = PlayerDevices {
            audio: None,
            mappings: None,
            midi: None,
            dmx_engine: None,
            sample_engine: None,
            trigger_engine: None,
        };
        let player = Player::new_with_devices(
            devices,
            test_playlists(playlist, songs),
            "test".to_string(),
            None,
        )?;
        assert!(player.audio_device().is_none());
        assert!(player.midi_device().is_none());
        assert!(player.dmx_engine().is_none());
        assert!(player.track_mappings().is_none());
        assert!(player.broadcast_handles().is_none());
        assert!(player.effect_engine().is_none());
        assert!(player.format_active_effects().is_none());
        assert!(player.get_cues().is_empty());
        assert!(!player.is_playing().await);
        assert!(player.elapsed().await?.is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_play_from_while_playing_returns_none() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        let result = player.play_from(Duration::from_secs(1)).await?;
        assert!(
            result.is_none(),
            "play_from while playing should return None"
        );

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_no_subsystem_player_play_and_navigate() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let devices = PlayerDevices {
            audio: None,
            mappings: None,
            midi: None,
            dmx_engine: None,
            sample_engine: None,
            trigger_engine: None,
        };
        let player = Player::new_with_devices(
            devices,
            test_playlists(playlist, songs),
            "playlist".to_string(),
            None,
        )?;

        player.process_sample_trigger(&[0x90, 60, 127]);
        player.stop_samples();

        let song = player.next().await.unwrap();
        assert_eq!(song.name(), "Song 3");

        let song = player.prev().await.unwrap();
        assert_eq!(song.name(), "Song 1");

        player.play().await?;
        eventually_async(
            || async { !player.is_playing().await },
            "Player never stopped after no-subsystem playback",
        )
        .await;

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_midi_device_accessor() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        assert!(player.midi_device().is_some());

        let player =
            make_test_player_with_config(Some(config::Audio::new("mock-device")), None, None)
                .await?;
        assert!(player.midi_device().is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_config_store_getter_setter() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // Initially None.
        assert!(player.config_store().is_none());

        // Set a config store.
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, "songs: songs\n")?;
        let cfg = config::Player::deserialize(&path)?;
        let store = std::sync::Arc::new(config::ConfigStore::new(cfg, path));
        player.set_config_store(store.clone());

        // Now it should be Some.
        let retrieved = player.config_store();
        assert!(retrieved.is_some());

        // Should be the same Arc (read_yaml returns same checksum).
        let (_, checksum1) = store.read_yaml().await?;
        let (_, checksum2) = retrieved.unwrap().read_yaml().await?;
        assert_eq!(checksum1, checksum2);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_reload_hardware_when_idle() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        assert!(player.audio_device().is_some());

        // Set up a config store with a profile that has no audio.
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.yaml");
        let yaml = "songs: songs\nprofiles:\n  - midi:\n      device: mock-midi-device\n";
        std::fs::write(&path, yaml)?;
        let cfg = config::Player::deserialize(&path)?;
        let store = std::sync::Arc::new(config::ConfigStore::new(cfg, path));
        player.set_config_store(store);

        // Reload should swap hardware — audio device should now be None.
        player.reload_hardware().await?;
        player.await_hardware_ready().await;
        assert!(
            player.audio_device().is_none(),
            "Audio device should be None after reload with no audio profile"
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_reload_hardware_during_playback_rejected() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        // Set up config store (needed for reload_hardware).
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("config.yaml");
        let yaml = "songs: songs\nprofiles:\n  - audio:\n      device: mock-device\n      track_mappings:\n        click: [1]\n";
        std::fs::write(&path, yaml)?;
        let cfg = config::Player::deserialize(&path)?;
        let store = std::sync::Arc::new(config::ConfigStore::new(cfg, path));
        player.set_config_store(store);

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // reload_hardware should fail during playback.
        let result = player.reload_hardware().await;
        assert!(
            result.is_err(),
            "reload_hardware should fail during playback"
        );
        assert!(
            result.unwrap_err().to_string().contains("during playback"),
            "error should mention playback"
        );

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_reload_hardware_no_config_store() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // No config store set — reload should fail.
        let result = player.reload_hardware().await;
        assert!(
            result.is_err(),
            "reload_hardware should fail without config store"
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_hardware_status_no_devices() -> Result<(), Box<dyn Error>> {
        let player = make_test_player_with_config(None, None, None).await?;
        let status = player.hardware_status();

        assert!(status.init_done);
        assert_eq!(status.audio.status, "not_connected");
        assert_eq!(status.midi.status, "not_connected");
        assert_eq!(status.dmx.status, "not_connected");
        assert_eq!(status.trigger.status, "not_connected");
        assert!(status.audio.name.is_none());
        assert!(status.midi.name.is_none());
        assert!(status.dmx.name.is_none());
        assert!(status.trigger.name.is_none());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_hardware_status_with_devices() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let status = player.hardware_status();

        assert!(status.init_done);
        assert_eq!(status.audio.status, "connected");
        assert!(status.audio.name.is_some());
        assert_eq!(status.midi.status, "connected");
        assert!(status.midi.name.is_some());

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_list_playlists() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let names = player.list_playlists();
        // Should contain at least "all_songs" and "playlist", sorted.
        assert!(names.contains(&"all_songs".to_string()));
        assert!(names.contains(&"playlist".to_string()));
        assert_eq!(
            names,
            {
                let mut sorted = names.clone();
                sorted.sort();
                sorted
            },
            "list_playlists should return sorted names"
        );
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_persisted_playlist_name() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        // Active playlist is "playlist", so persisted should also be "playlist".
        assert_eq!(player.persisted_playlist_name(), "playlist");

        // After switching to all_songs, persisted should still be "playlist".
        player.switch_to_playlist("all_songs").await.unwrap();
        assert_eq!(player.persisted_playlist_name(), "playlist");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_playlists_snapshot() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let snapshot = player.playlists_snapshot();
        assert!(snapshot.contains_key("all_songs"));
        assert!(snapshot.contains_key("playlist"));
        assert_eq!(snapshot.len(), 2);
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_reload_songs() -> Result<(), Box<dyn Error>> {
        let player = make_bare_player()?;
        let initial_count = player.songs().len();

        // Create a temp directory that mirrors assets/ layout with an extra song.
        let temp_dir = tempfile::tempdir()?;
        let songs_dir = temp_dir.path().join("songs");
        fs::create_dir_all(&songs_dir)?;

        // Copy existing song YAML configs into temp songs dir.
        let src = Path::new("assets/songs");
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            if entry.path().extension().is_some_and(|e| e == "yaml") {
                fs::copy(entry.path(), songs_dir.join(entry.file_name()))?;
            }
        }

        // Copy required audio/MIDI assets into the temp parent (songs configs use ../ paths).
        let assets = Path::new("assets");
        for entry in fs::read_dir(assets)? {
            let entry = entry?;
            if entry.path().is_file() {
                fs::copy(entry.path(), temp_dir.path().join(entry.file_name()))?;
            }
        }

        // Add a new song config referencing a copied audio file.
        let new_song_yaml =
            "name: \"New Test Song\"\ntracks:\n  - name: click\n    file: ../1Channel44.1k.wav\n";
        fs::write(songs_dir.join("newsong.yaml"), new_song_yaml)?;

        player.reload_songs(&songs_dir, None, None);
        assert!(
            player.songs().len() > initial_count,
            "reload_songs should discover the new song (was {}, now {})",
            initial_count,
            player.songs().len(),
        );
        Ok(())
    }

    #[test]
    fn test_load_playlists_standalone() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;

        // Load with legacy playlist only.
        let playlists =
            super::load_playlists(None, Some(Path::new("assets/playlist.yaml")), songs.clone())?;
        assert!(playlists.contains_key("all_songs"));
        assert!(playlists.contains_key("playlist"));
        assert_eq!(playlists["playlist"].songs().len(), 5);

        // Load with no playlists dir and no legacy — just all_songs.
        let playlists = super::load_playlists(None, None, songs.clone())?;
        assert!(playlists.contains_key("all_songs"));
        assert_eq!(playlists.len(), 1);

        // Load from a playlists directory.
        let temp_dir = tempfile::tempdir()?;
        let pl_dir = temp_dir.path();
        fs::write(pl_dir.join("my_set.yaml"), "songs:\n- Song 1\n- Song 3\n")?;
        let playlists = super::load_playlists(Some(pl_dir), None, songs)?;
        assert!(playlists.contains_key("all_songs"));
        assert!(playlists.contains_key("my_set"));
        assert_eq!(playlists["my_set"].songs().len(), 2);

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn play_song_from_valid() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let player = Player::new(
            test_playlists(playlist, songs.clone()),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                Some(config::Midi::new("mock-midi-device", None)),
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;
        let device = player.audio_device().expect("audio device").to_mock()?;

        let result = player
            .play_song_from("Song 2", std::time::Duration::ZERO)
            .await?;
        assert!(result.is_some());
        assert_eq!(result.unwrap().name(), "Song 2");
        eventually(|| device.is_playing(), "Song never started playing");

        // Switches to all_songs (session-only) for the duration of playback
        assert_eq!(player.get_playlist().name(), "all_songs");

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn play_song_from_not_found() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let player = Player::new(
            test_playlists(playlist, songs.clone()),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                Some(config::Midi::new("mock-midi-device", None)),
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;

        let result = player
            .play_song_from("Nonexistent Song", std::time::Duration::ZERO)
            .await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(err.to_string().contains("not found"));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn play_song_from_while_playing() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let player = Player::new(
            test_playlists(playlist, songs.clone()),
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                Some(config::Midi::new("mock-midi-device", None)),
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;
        let device = player.audio_device().expect("audio device").to_mock()?;

        // Start playing first
        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // play_song_from while already playing should return None
        let result = player
            .play_song_from("Song 2", std::time::Duration::ZERO)
            .await?;
        assert!(result.is_none());

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_loop_section_not_playing() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // loop_section when nothing is playing should error.
        let result = player.loop_section("verse").await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("no song is playing"));
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_loop_section_not_found() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // Test songs have no sections/beat grid, so any section name should fail.
        let result = player.loop_section("nonexistent").await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stop_section_loop_clears_state() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // active_section should be None initially.
        assert!(player.active_section().is_none());

        // Manually set active section state to simulate an active loop.
        {
            let mut active = player.active_section.write();
            *active = Some(SectionBounds {
                name: "test".to_string(),
                start_time: Duration::from_secs(1),
                end_time: Duration::from_secs(5),
            });
        }
        assert!(player.active_section().is_some());

        // stop_section_loop should clear it.
        player.stop_section_loop();
        assert!(player.active_section().is_none());
        assert!(player.section_loop_break.load(Ordering::Relaxed));

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_add_loop_time_consumed() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // Initially zero.
        assert_eq!(*player.loop_time_consumed.lock(), Duration::ZERO);

        // Accumulates correctly.
        player.add_loop_time_consumed(Duration::from_secs(2));
        assert_eq!(*player.loop_time_consumed.lock(), Duration::from_secs(2));

        player.add_loop_time_consumed(Duration::from_secs(3));
        assert_eq!(*player.loop_time_consumed.lock(), Duration::from_secs(5));

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_active_section_getter() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        assert!(player.active_section().is_none());

        let bounds = SectionBounds {
            name: "chorus".to_string(),
            start_time: Duration::from_secs(10),
            end_time: Duration::from_secs(20),
        };
        *player.active_section.write() = Some(bounds.clone());

        let active = player.active_section().unwrap();
        assert_eq!(active.name, "chorus");
        assert_eq!(active.start_time, Duration::from_secs(10));
        assert_eq!(active.end_time, Duration::from_secs(20));

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_is_current_song_looping() -> Result<(), Box<dyn Error>> {
        let player = make_test_player().await?;

        // Test songs don't have loop_playback set, so this should be false.
        assert!(!player.is_current_song_looping());

        Ok(())
    }
}
