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
    ClockSource, HardwareState, HardwareStatusSnapshot, Player, SongChangeNotifier, StatusEvents,
    SubsystemStatus,
};

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

    /// Retries a device constructor perpetually until it succeeds or the
    /// cancellation token is triggered. Device construction runs in a
    /// blocking task since hardware discovery does blocking I/O.
    pub(super) async fn retry_until_ready<T, E, F>(
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
