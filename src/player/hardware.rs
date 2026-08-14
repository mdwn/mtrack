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
use parking_lot::RwLock;
use std::{
    collections::HashMap,
    error::Error,
    path::{Path, PathBuf},
    sync::Arc,
    thread,
    time::Duration,
};
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

use crate::samples::SampleEngine;
use crate::trigger::TriggerEngine;
use crate::{audio, config, dmx, midi, samples};

use super::{
    AudioOutputHealth, ClockSource, HardwareState, HardwareStatusSnapshot, Player,
    SongChangeNotifier, StatusEvents, SubsystemStatus,
};

/// How long a terminal failure keeps being retried before init gives up.
///
/// Not zero: a device can be briefly unopenable at boot while udev and ALSA
/// finish with it, and abandoning it on the first attempt would turn that race
/// into a permanent misconfiguration report.
const TERMINAL_GRACE: Duration = Duration::from_secs(5);

/// How often an unchanged init failure is repeated in the log.
const FAILURE_LOG_INTERVAL: Duration = Duration::from_secs(30);

/// Why [`Player::retry_until_ready`] stopped without a device.
pub(super) enum InitFailure {
    /// A reload or shutdown cancelled this init round. Says nothing about the
    /// device, so nothing is reported to the operator.
    Cancelled,
    /// The device will not come up without someone changing something. Carries
    /// the reason, because a subsystem reported as failed with no explanation
    /// leaves the operator exactly where the perpetual retry did.
    Terminal(String),
}

/// Whether a device constructor's error could still succeed on a later attempt.
///
/// Implemented per error type rather than by string matching, so the answer
/// lives with the code that knows it.
pub(super) trait Retryable {
    /// Whether this failure will still be a failure however long we wait.
    fn is_terminal(&self) -> bool;
}

impl Retryable for audio::AudioError {
    fn is_terminal(&self) -> bool {
        audio::AudioError::is_terminal(self)
    }
}

/// MIDI and DMX constructors report as strings and are all treated as
/// retryable, which is the behaviour they had before this trait existed.
///
/// Both are genuinely worth classifying — a MIDI port that is present and
/// refuses to open is the same shape of problem as the audio case — but doing
/// it needs typed errors from those subsystems, and guessing from message text
/// is how a transient failure gets abandoned by accident.
impl Retryable for String {
    fn is_terminal(&self) -> bool {
        false
    }
}

impl Player {
    /// Asynchronously discovers and initializes all hardware devices.
    ///
    /// Retries each device perpetually until found or cancelled. Devices are
    /// written to `HardwareState` as they become available, so playback can
    /// use whatever hardware is ready. Respects dependency ordering:
    ///   Phase 1: Audio + DMX (parallel)
    ///   Phase 2: MIDI (needs DMX), Sample engine (needs Audio) — parallel
    ///   Phase 3: Trigger engine (needs Sample engine), status reporting
    pub(super) async fn init_hardware_async(
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

        let (audio_result, dmx_result) = tokio::join!(
            async {
                if let Some(audio_config) = audio_config {
                    let outcome = Self::retry_until_ready("audio device", cancel1, move || {
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
                            Err(e) => Err(e),
                        }
                    })
                    .await;
                    self.record_init_outcome("audio", outcome)
                } else {
                    info!("Audio not configured in profile; proceeding without audio");
                    None
                }
            },
            async {
                if let Some(dmx_config) = dmx_config {
                    let bp = bp.clone();
                    let outcome = Self::retry_until_ready("dmx engine", cancel2, move || {
                        dmx::create_engine(Some(&dmx_config), bp.as_deref())
                            .map_err(|e| e.to_string())
                    })
                    .await;
                    self.record_init_outcome("dmx", outcome).flatten()
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

                // Build the shared track gains from the profile config and
                // install them into the device mixer so the audio callback
                // can read them lock-free. Rebuilt on every (re)load from
                // the persisted config, so gains survive restarts.
                let track_gains = Arc::new(crate::audio::track_gains::TrackGains::from_config(
                    &mappings,
                    profile.audio_config().map(|ac| ac.track_gains()),
                ));
                if let Some(mixer) = device.mixer() {
                    mixer.set_track_gains(track_gains.clone());
                }

                // Player-wide metronome defaults (master volume, click
                // sounds) for songs that don't override them.
                device.set_metronome_defaults(config.metronome().cloned());

                let mut hw = self.hardware.write();
                hw.device = Some(device.clone());
                hw.mappings = Some(Arc::new(mappings.clone()));
                hw.track_gains = Some(track_gains);
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
                    let outcome = Self::retry_until_ready("midi device", cancel3, move || {
                        midi::get_device(Some(midi_config.clone()), dmx_engine_for_midi.clone())
                            .map_err(|e| e.to_string())
                    })
                    .await;
                    self.record_init_outcome("midi", outcome).flatten()
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

            let status_events = match StatusEvents::new(
                profile
                    .status_events()
                    .cloned()
                    .or_else(|| config.status_events()),
            ) {
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

    /// Unwraps an init outcome, recording a terminal failure so status can
    /// report it.
    ///
    /// A subsystem that gave up must say why. Without the reason it reads as
    /// "not connected", which is indistinguishable from "not configured" and
    /// leaves the operator exactly where the perpetual retry did — except now
    /// it is quiet about it too.
    fn record_init_outcome<T>(
        &self,
        subsystem: &str,
        outcome: Result<T, InitFailure>,
    ) -> Option<T> {
        match outcome {
            Ok(value) => Some(value),
            Err(InitFailure::Cancelled) => None,
            Err(InitFailure::Terminal(why)) => {
                self.hardware
                    .write()
                    .init_errors
                    .insert(subsystem.to_string(), why);
                None
            }
        }
    }

    /// Retries a device constructor until it succeeds, gives up, or the
    /// cancellation token is triggered. Device construction runs in a
    /// blocking task since hardware discovery does blocking I/O.
    ///
    /// Retrying forever is right for a device that has not appeared yet and
    /// wrong for one that has appeared and refuses its configuration. Because
    /// subsystems are joined by phase, the second case used to hold the whole
    /// rig uninitialised: no MIDI, no samples, no triggers, and `init_done`
    /// never firing — so a misconfigured device took away the web UI that is
    /// how you would fix it.
    pub(super) async fn retry_until_ready<T, E, F>(
        name: &str,
        cancel: CancellationToken,
        constructor: F,
    ) -> Result<T, InitFailure>
    where
        T: Send + 'static,
        E: std::fmt::Display + Retryable + Send + Sync + 'static,
        F: Fn() -> Result<T, E> + Send + Sync + 'static,
    {
        Self::retry_until_ready_with_grace(name, cancel, TERMINAL_GRACE, constructor).await
    }

    /// [`Self::retry_until_ready`] with the grace window supplied, so tests can
    /// exercise giving up without spending the real one on every run.
    pub(super) async fn retry_until_ready_with_grace<T, E, F>(
        name: &str,
        cancel: CancellationToken,
        terminal_grace: Duration,
        constructor: F,
    ) -> Result<T, InitFailure>
    where
        T: Send + 'static,
        E: std::fmt::Display + Retryable + Send + Sync + 'static,
        F: Fn() -> Result<T, E> + Send + Sync + 'static,
    {
        let name = name.to_string();
        let constructor = Arc::new(constructor);
        let mut terminal_since: Option<std::time::Instant> = None;
        let mut last_logged: Option<(String, std::time::Instant)> = None;

        loop {
            let ctor = constructor.clone();
            let result = tokio::task::spawn_blocking(move || ctor()).await;

            match result {
                Ok(Ok(value)) => return Ok(value),
                Ok(Err(e)) => {
                    // Repeats are rate-limited. The same line at 2Hz for the
                    // length of a set buries everything else in the journal
                    // someone reads afterwards, and says nothing the first one
                    // did not. A changed message is always logged.
                    let message = e.to_string();
                    let stale = last_logged.as_ref().is_none_or(|(previous, at)| {
                        previous != &message || at.elapsed() >= FAILURE_LOG_INTERVAL
                    });
                    if stale {
                        warn!("Could not get {name}: {message}");
                        last_logged = Some((message.clone(), std::time::Instant::now()));
                    }

                    if e.is_terminal() {
                        // Not abandoned on the first attempt: a device can be
                        // briefly unopenable at boot while udev and ALSA finish
                        // with it, and giving up instantly would turn that race
                        // into a permanent misconfiguration report.
                        let since = *terminal_since.get_or_insert_with(std::time::Instant::now);
                        if since.elapsed() >= terminal_grace {
                            error!(
                                "Giving up on {name}: {message}. This will not succeed on its own, \
                                 so the rest of the rig is starting without it."
                            );
                            return Err(InitFailure::Terminal(message));
                        }
                    } else {
                        // The situation changed, so the grace window starts over
                        // if it turns terminal again.
                        terminal_since = None;
                    }
                }
                Err(e) => {
                    error!("Device init task panicked for {name}: {e}");
                }
            }

            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("Hardware init cancelled for {name}");
                    return Err(InitFailure::Cancelled);
                }
                _ = tokio::time::sleep(Duration::from_millis(500)) => {}
            }
        }
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
            track_gains: None,
            midi_device: None,
            dmx_engine: None,
            sample_engine: None,
            trigger_engine: None,
            clock_source: ClockSource::Wall,
            song_change_notifiers: Vec::new(),
            profile_name: None,
            hostname: None,
            init_errors: HashMap::new(),
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

    /// Whether the audio device this player holds open is the one `name` names.
    ///
    /// Asked before probing a device from the web UI. The held device will
    /// refuse a second open, so probing it reports a failure for the one device
    /// that is demonstrably working — its live health is the better answer and
    /// is already in [`Self::hardware_status`].
    pub fn holds_audio_device(&self, name: &str) -> bool {
        self.hardware
            .read()
            .device
            .as_ref()
            .is_some_and(|d| d.matches_name(name))
    }

    /// Returns a snapshot of all hardware subsystem statuses.
    pub fn hardware_status(&self) -> HardwareStatusSnapshot {
        let init_done = *self.init_done_tx.borrow();
        let hw = self.hardware.read();

        let status_for =
            |subsystem: &str, present: bool, name: Option<String>| -> SubsystemStatus {
                if present {
                    return SubsystemStatus {
                        status: "connected".to_string(),
                        name,
                        error: None,
                    };
                }
                // Checked before `init_done`, because a subsystem that gave up is
                // reportable immediately — waiting for the rest of the rig to
                // finish coming up before admitting it would show "initializing"
                // for something that has already stopped trying.
                if let Some(why) = hw.init_errors.get(subsystem) {
                    return SubsystemStatus {
                        status: "failed".to_string(),
                        name: None,
                        error: Some(why.clone()),
                    };
                }
                if !init_done {
                    SubsystemStatus {
                        status: "initializing".to_string(),
                        name: None,
                        error: None,
                    }
                } else {
                    SubsystemStatus {
                        status: "not_connected".to_string(),
                        name: None,
                        error: None,
                    }
                }
            };

        let liveness_window = hw
            .device
            .as_ref()
            .map(|d| d.liveness_window())
            .unwrap_or(audio::health::LIVENESS_WINDOW);
        let audio_output = hw.device.as_ref().and_then(|d| d.output_health()).map(|h| {
            let as_ms = |d: Option<Duration>| d.map(|d| d.as_millis() as u64);
            AudioOutputHealth {
                status: h.status(liveness_window),
                callback_alive: h.callback_alive(liveness_window),
                writing_signal: h.writing_signal(audio::health::SIGNAL_WINDOW),
                since_last_callback_ms: as_ms(h.since_last_callback),
                since_last_signal_ms: as_ms(h.since_last_signal),
                callbacks: h.callbacks,
                recoveries: h.recoveries,
                underruns: h.underruns,
                last_error: h.last_error.clone(),
            }
        });

        HardwareStatusSnapshot {
            init_done,
            hostname: hw.hostname.clone(),
            profile: hw.profile_name.clone(),
            audio: status_for(
                "audio",
                hw.device.is_some(),
                hw.device.as_ref().map(|d| d.to_string()),
            ),
            audio_output,
            midi: status_for(
                "midi",
                hw.midi_device.is_some(),
                hw.midi_device.as_ref().map(|d| d.to_string()),
            ),
            dmx: status_for(
                "dmx",
                hw.dmx_engine.is_some(),
                hw.dmx_engine.as_ref().map(|_| "DMX Engine".to_string()),
            ),
            trigger: status_for(
                "trigger",
                hw.trigger_engine.is_some(),
                hw.trigger_engine
                    .as_ref()
                    .map(|_| "Trigger Engine".to_string()),
            ),
        }
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

    /// Adds a notifier that will be called on every song change.
    pub fn add_song_change_notifier(&self, notifier: Arc<dyn SongChangeNotifier>) {
        self.hardware.write().song_change_notifiers.push(notifier);
    }

    /// Reports status as MIDI events.
    pub(super) async fn report_status(&self, status_events: StatusEvents) {
        let _enter = self.span.enter();
        info!("Reporting status");

        let midi_device = match self.hardware.read().midi_device.clone() {
            Some(device) => device,
            None => {
                warn!("MIDI device not present for status reporting; skipping");
                return;
            }
        };
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
}

/// Initializes the sample engine if the audio device supports mixing and source input.
pub(super) fn init_sample_engine(
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
pub(super) fn init_trigger_engine(
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

#[cfg(test)]
mod test {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A failure nobody has classified. Stands in for MIDI and DMX, whose
    /// errors are all retryable today.
    struct Transient;
    impl std::fmt::Display for Transient {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "not here yet")
        }
    }
    impl Retryable for Transient {
        fn is_terminal(&self) -> bool {
            false
        }
    }

    struct Terminal;
    impl std::fmt::Display for Terminal {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "found but could not be opened")
        }
    }
    impl Retryable for Terminal {
        fn is_terminal(&self) -> bool {
            true
        }
    }

    const SHORT_GRACE: Duration = Duration::from_millis(200);

    /// The defect this exists to fix: a device that will never open used to
    /// spin forever, and because subsystems are joined by phase, held the whole
    /// rig uninitialised behind it.
    #[tokio::test]
    async fn a_terminal_failure_gives_up_with_its_reason() {
        // Bounded, because without the fix this does not fail — it never
        // returns at all, which is the defect. A hanging test reports nothing.
        let outcome: Result<(), _> = tokio::time::timeout(
            Duration::from_secs(5),
            Player::retry_until_ready_with_grace(
                "test device",
                CancellationToken::new(),
                SHORT_GRACE,
                || Err(Terminal),
            ),
        )
        .await
        .expect("init never gave up on a device that will never open");

        match outcome {
            Err(InitFailure::Terminal(why)) => assert!(
                why.contains("could not be opened"),
                "the reason must survive, or 'failed' tells the operator nothing: {why}"
            ),
            Err(InitFailure::Cancelled) => panic!("gave up for the wrong reason"),
            Ok(()) => panic!("a terminal failure must not report success"),
        }
    }

    /// The behaviour that must survive: a device that has not turned up yet may
    /// still turn up, and waiting is the whole point of the retry.
    #[tokio::test]
    async fn a_transient_failure_keeps_waiting() {
        let cancel = CancellationToken::new();
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = attempts.clone();

        let handle = tokio::spawn({
            let cancel = cancel.clone();
            async move {
                Player::retry_until_ready_with_grace::<(), _, _>(
                    "test device",
                    cancel,
                    SHORT_GRACE,
                    move || {
                        counter.fetch_add(1, Ordering::SeqCst);
                        Err(Transient)
                    },
                )
                .await
            }
        });

        // Well past the grace window a terminal failure would have used up.
        tokio::time::sleep(Duration::from_millis(1200)).await;
        assert!(
            !handle.is_finished(),
            "a transient failure must not be abandoned"
        );
        assert!(
            attempts.load(Ordering::SeqCst) > 1,
            "it should still be retrying"
        );

        cancel.cancel();
        assert!(matches!(
            handle.await.expect("task panicked"),
            Err(InitFailure::Cancelled)
        ));
    }

    /// The grace window is why giving up is not instant: a device can be
    /// briefly unopenable at boot while udev and ALSA finish with it.
    #[tokio::test]
    async fn a_terminal_failure_that_clears_inside_the_grace_window_succeeds() {
        let attempts = Arc::new(AtomicUsize::new(0));
        let counter = attempts.clone();

        let outcome = Player::retry_until_ready_with_grace(
            "test device",
            CancellationToken::new(),
            Duration::from_secs(30),
            move || {
                if counter.fetch_add(1, Ordering::SeqCst) < 2 {
                    Err(Terminal)
                } else {
                    Ok(())
                }
            },
        )
        .await;

        assert!(
            outcome.is_ok(),
            "a device that opens on a later attempt must be used, not abandoned"
        );
    }

    /// A subsystem that gave up must say so and say why. Reporting it as
    /// "not connected" would be indistinguishable from "not configured".
    #[tokio::test]
    async fn a_failed_subsystem_is_reported_with_its_reason() {
        let player = crate::player::Player::new_with_devices(
            crate::player::PlayerDevices {
                audio: None,
                mappings: None,
                midi: None,
                dmx_engine: None,
                sample_engine: None,
                trigger_engine: None,
            },
            HashMap::new(),
            "all_songs".to_string(),
            None,
        )
        .expect("player");
        player.hardware.write().init_errors.insert(
            "audio".to_string(),
            "'X' was found but could not be opened".to_string(),
        );

        let status = player.hardware_status();
        assert_eq!(status.audio.status, "failed");
        assert_eq!(
            status.audio.error.as_deref(),
            Some("'X' was found but could not be opened")
        );
        // Untouched subsystems must not be tarred with it.
        assert_ne!(status.midi.status, "failed");
        assert!(status.midi.error.is_none());
    }
}
