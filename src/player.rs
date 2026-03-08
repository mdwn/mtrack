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
use std::{
    collections::HashMap,
    error::Error,
    fmt::Display,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Barrier,
    },
    thread,
    time::{Duration, SystemTime},
};
use tokio::{
    sync::{oneshot, Mutex},
    task::JoinHandle,
};
use tracing::{error, info, span, warn, Level, Span};

use crate::samples::SampleEngine;
use crate::songs::Songs;
use crate::trigger::TriggerEngine;
use crate::{
    audio, config, dmx, midi,
    playlist::{self, Playlist},
    playsync::CancelHandle,
    samples,
    songs::Song,
};

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
    song: Arc<Song>,
    cancel_handle: CancelHandle,
    play_tx: oneshot::Sender<Result<(), String>>,
    start_time: Duration,
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

/// Plays back individual wav files as multichannel audio for the configured audio interface.
#[derive(Clone)]
pub struct Player {
    /// The device to play audio through (optional if absent from profile).
    device: Option<Arc<dyn audio::Device>>,
    /// Mappings of tracks to output channels (optional if no audio configured).
    mappings: Option<Arc<HashMap<String, Vec<u16>>>>,
    /// The MIDI device to play MIDI back through.
    midi_device: Option<Arc<dyn midi::Device>>,
    /// The DMX engine to use.
    dmx_engine: Option<Arc<dmx::engine::Engine>>,
    /// The sample engine for MIDI-triggered samples.
    sample_engine: Option<Arc<RwLock<SampleEngine>>>,
    /// The audio trigger engine for piezo triggers.
    /// Held to keep the engine (and its cpal stream + forwarding thread) alive.
    #[allow(dead_code)]
    trigger_engine: Option<Arc<TriggerEngine>>,
    /// The playlist to use.
    playlist: Arc<Playlist>,
    /// The all songs playlist.
    all_songs: Arc<Playlist>,
    /// Switches between the playlist and the all songs playlist.
    use_all_songs: Arc<AtomicBool>,
    /// The time that the last play action occurred.
    play_start_time: Arc<Mutex<Option<SystemTime>>>,
    /// Keeps track of the player joins. There should only be one task on here at a time.
    join: Arc<Mutex<Option<PlayHandles>>>,
    /// After stop is set, this will be set to true. This will prevent stop from being run again until
    /// it is unset, which should be handled by a cleanup async process after playback finishes.
    stop_run: Arc<AtomicBool>,
    /// The logging span.
    span: Span,
}

impl Player {
    /// Creates a new player by discovering hardware devices from the config.
    pub fn new(
        songs: Arc<Songs>,
        playlist: Arc<Playlist>,
        config: &config::Player,
        base_path: Option<&Path>,
    ) -> Result<Player, Box<dyn Error>> {
        let span = span!(Level::INFO, "player");
        let _enter = span.enter();

        let hostname = config::resolve_hostname();
        info!(hostname = %hostname, "Resolved hostname for hardware profiles");

        // Get the first matching profile
        let profiles = config.profiles(&hostname);
        let profile = profiles
            .first()
            .ok_or("No matching hardware profile found")?;

        info!(
            hostname = profile.hostname().unwrap_or("default"),
            device = profile
                .audio_config()
                .map(|ac| ac.audio().device())
                .unwrap_or("none"),
            "Using hardware profile"
        );

        // Audio: if present in profile, required. If absent, optional.
        let (device, mappings, resolved_audio) = if let Some(audio_config) = profile.audio_config()
        {
            let (device, mappings, resolved_audio) = Self::wait_for_ok("audio device", || {
                match audio::get_device(Some(audio_config.audio().clone())) {
                    Ok(device) => {
                        info!(
                            device = audio_config.audio().device(),
                            "Audio device initialized"
                        );
                        Ok((
                            device.clone(),
                            audio_config.track_mappings().clone(),
                            audio_config.audio().clone(),
                        ))
                    }
                    Err(e) => Err(format!("audio device: {}", e)),
                }
            })?;
            (Some(device), Some(mappings), Some(resolved_audio))
        } else {
            info!("Audio not configured in profile; proceeding without audio");
            (None, None, None)
        };

        // DMX: if present in profile, required. If absent, optional.
        // Always allows null OLA fallback so the web UI / simulator can run without hardware.
        let dmx_engine = if let Some(dmx_config) = profile.dmx() {
            Self::wait_for_ok("dmx engine", || {
                dmx::create_engine(Some(dmx_config), base_path)
            })?
        } else {
            info!("DMX not configured in profile; proceeding without DMX");
            None
        };

        // MIDI: if present in profile, required. If absent, optional.
        let midi_device = if let Some(midi_config) = profile.midi() {
            Self::wait_for_ok("midi device", || {
                midi::get_device(Some(midi_config.clone()), dmx_engine.clone())
            })?
        } else {
            info!("MIDI not configured in profile; proceeding without MIDI");
            None
        };

        let status_events = Self::wait_for_ok("status events", || {
            StatusEvents::new(config.status_events())
        })?;

        let sample_engine = init_sample_engine(
            &device,
            &mappings,
            resolved_audio.as_ref(),
            config,
            profile,
            base_path,
        );

        let trigger_engine = init_trigger_engine(profile, &sample_engine)?;

        let devices = PlayerDevices {
            audio: device,
            mappings: mappings.map(Arc::new),
            midi: midi_device,
            dmx_engine,
            sample_engine,
            trigger_engine,
        };

        let player = Self::new_with_devices(devices, playlist, songs)?;

        if player.midi_device.is_some() {
            // Emit the event for the first track if needed.
            Player::emit_midi_event(player.midi_device.clone(), player.get_playlist().current());

            if let Some(status_events) = status_events {
                let midi_device = player
                    .midi_device
                    .clone()
                    .expect("MIDI device must be present");
                let join = player.join.clone();
                tokio::spawn(Player::report_status(
                    span.clone(),
                    midi_device,
                    join,
                    status_events,
                ));
            }
        }

        Ok(player)
    }

    /// Creates a new player with pre-constructed devices.
    ///
    /// This is the core constructor used by `new()` after device discovery,
    /// and can be called directly in tests with mock devices.
    pub fn new_with_devices(
        devices: PlayerDevices,
        playlist: Arc<Playlist>,
        songs: Arc<Songs>,
    ) -> Result<Player, Box<dyn Error>> {
        Ok(Player {
            device: devices.audio,
            mappings: devices.mappings,
            midi_device: devices.midi,
            dmx_engine: devices.dmx_engine,
            sample_engine: devices.sample_engine,
            trigger_engine: devices.trigger_engine,
            playlist,
            all_songs: playlist::from_songs(songs)?,
            use_all_songs: Arc::new(AtomicBool::new(false)),
            play_start_time: Arc::new(Mutex::new(None)),
            join: Arc::new(Mutex::new(None)),
            stop_run: Arc::new(AtomicBool::new(false)),
            span: span!(Level::INFO, "player"),
        })
    }

    /// Wait for constructor function to return an Ok(result) variant.
    /// Respects MTRACK_DEVICE_RETRY_LIMIT: if set to N, tries at most N times then returns
    /// the last error. If unset or 0, retries indefinitely (original behavior).
    fn wait_for_ok<T, E, F>(name: &str, constructor: F) -> Result<T, Box<dyn Error>>
    where
        E: Display + Into<Box<dyn Error>>,
        F: Fn() -> Result<T, E>,
    {
        let max_attempts = std::env::var("MTRACK_DEVICE_RETRY_LIMIT")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
        Self::wait_for_ok_with_limit(name, max_attempts, constructor)
    }

    /// Wait for constructor function to return an Ok(result) variant.
    /// If max_attempts is 0, retries indefinitely.
    fn wait_for_ok_with_limit<T, E, F>(
        name: &str,
        max_attempts: u32,
        constructor: F,
    ) -> Result<T, Box<dyn Error>>
    where
        E: Display + Into<Box<dyn Error>>,
        F: Fn() -> Result<T, E>,
    {
        let delay_ms = 500;
        let mut attempt = 0u32;

        loop {
            match constructor() {
                Ok(ok) => return Ok(ok),
                Err(err) => {
                    warn!("Could not get {name}! {err}");
                    attempt += 1;
                    if max_attempts > 0 && attempt >= max_attempts {
                        error!(
                            attempt = attempt,
                            limit = max_attempts,
                            "Retry limit reached, giving up"
                        );
                        return Err(err.into());
                    }
                    info!("Retrying after delay.");
                    thread::sleep(Duration::from_millis(delay_ms));
                }
            }
        }
    }

    /// Gets the audio device currently in use by the player.
    #[cfg(test)]
    pub fn audio_device(&self) -> Option<Arc<dyn audio::Device>> {
        self.device.clone()
    }

    /// Gets the MIDI device currently in use by the player.
    pub fn midi_device(&self) -> Option<Arc<dyn midi::Device>> {
        self.midi_device.clone()
    }

    /// Processes a MIDI event for triggered samples.
    /// This should be called by the MIDI controller when events are received.
    /// Uses std::sync::RwLock for minimal latency (no async overhead).
    pub fn process_sample_trigger(&self, raw_event: &[u8]) {
        if let Some(ref sample_engine) = self.sample_engine {
            let engine = sample_engine.read();
            engine.process_midi_event(raw_event);
        }
    }

    /// Loads the sample configuration for a song.
    /// This preloads samples for the song so they're ready for instant playback.
    /// Note: Active voices continue playing through song transitions.
    fn load_song_samples(&self, song: &Song) {
        if let Some(ref sample_engine) = self.sample_engine {
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
        if let Some(ref sample_engine) = self.sample_engine {
            let engine = sample_engine.read();
            engine.stop_all();
        }
    }

    /// Gets the DMX engine currently in use by the player (for testing).
    #[cfg(test)]
    pub fn dmx_engine(&self) -> Option<Arc<dmx::engine::Engine>> {
        self.dmx_engine.clone()
    }

    /// Gets all cues from the current song's lighting timeline.
    pub fn get_cues(&self) -> Vec<(Duration, usize)> {
        if let Some(ref dmx_engine) = self.dmx_engine {
            dmx_engine.get_timeline_cues()
        } else {
            Vec::new()
        }
    }

    /// Returns handles needed for reading lighting state, or None if no DMX engine is configured.
    pub fn broadcast_handles(&self) -> Option<dmx::engine::BroadcastHandles> {
        self.dmx_engine.as_ref().map(|e| e.broadcast_handles())
    }

    /// Passes the broadcast channel to the DmxEngine for file watcher hot-reload.
    pub fn set_broadcast_tx(&self, tx: tokio::sync::broadcast::Sender<String>) {
        if let Some(ref engine) = self.dmx_engine {
            engine.set_broadcast_tx(tx);
        }
    }

    /// Reports status as MIDI events.
    async fn report_status(
        span: Span,
        midi_device: Arc<dyn midi::Device>,
        join: Arc<Mutex<Option<PlayHandles>>>,
        status_events: StatusEvents,
    ) {
        let _enter = span.enter();
        info!("Reporting status");

        let midi_device = midi_device.clone();
        let join = join.clone();

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
        let _enter = self.span.enter();

        let mut join = self.join.lock().await;

        let playlist = self.get_playlist().clone();
        let song = playlist.current();
        if join.is_some() {
            info!(
                current_song = song.name(),
                "Player is already playing a song."
            );
            return Ok(None);
        }

        // Load samples for this song (if not already loaded)
        self.load_song_samples(&song);

        // Validate lighting shows before starting playback
        if let Some(ref dmx_engine) = self.dmx_engine {
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
        if let Some(ref mappings) = self.mappings {
            crate::verify::warn_unmapped_tracks(&song, mappings);
        }

        let play_start_time = self.play_start_time.clone();

        let cancel_handle = CancelHandle::new();
        let cancel_handle_for_cleanup = cancel_handle.clone();
        let (play_tx, play_rx) = oneshot::channel::<Result<(), String>>();

        let join_handle = {
            let ctx = PlaybackContext {
                device: self.device.clone(),
                mappings: self.mappings.clone(),
                midi_device: self.midi_device.clone(),
                dmx_engine: self.dmx_engine.clone(),
                song: song.clone(),
                cancel_handle: cancel_handle.clone(),
                play_tx,
                start_time,
            };
            tokio::task::spawn_blocking(move || {
                Player::play_files(ctx);
            })
        };
        *join = Some(PlayHandles {
            join: join_handle,
            cancel: cancel_handle,
        });

        {
            let mut play_start_time = play_start_time.lock().await;
            *play_start_time = Some(SystemTime::now());
        }

        {
            let join_mutex = self.join.clone();
            let stop_run = self.stop_run.clone();
            let song = song.clone();
            let midi_device = self.midi_device.clone();
            tokio::spawn(async move {
                let result = match play_rx.await {
                    Ok(Ok(())) => PlaybackResult::Success,
                    Ok(Err(e)) => PlaybackResult::Failed(e),
                    Err(_e) => PlaybackResult::SenderDropped,
                };

                let cancelled = cancel_handle_for_cleanup.is_cancelled();

                info!(
                    song = song.name(),
                    cancelled = cancelled,
                    "Song finished playing."
                );

                let action = decide_cleanup_action(result, cancelled);
                if action == CleanupAction::StopCancelled {
                    // stop() already cleared join and play_start_time.
                    // Touching them here would clobber state from a new play() that
                    // may have started after stop() returned.
                    return;
                }

                // Natural finish: advance playlist and clean up.
                let mut join = join_mutex.lock().await;
                Player::next_and_emit(midi_device.clone(), playlist);

                {
                    let mut play_start_time = play_start_time.lock().await;
                    *play_start_time = None;
                }

                *join = None;
                stop_run.store(false, Ordering::Relaxed);
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
            song,
            cancel_handle,
            play_tx,
            start_time,
        } = ctx;

        // Set up the play barrier, which will synchronize the subsystem threads.
        let has_audio = device.is_some();
        let has_midi = song.midi_playback().is_some() && midi_device.is_some();
        let num_light_shows = if dmx_engine.is_some() {
            song.light_shows().len()
        } else {
            0
        };
        let has_dsl_lighting = !song.dsl_lighting_shows().is_empty() && dmx_engine.is_some();
        let num_barriers =
            compute_barrier_count(has_audio, has_midi, num_light_shows, has_dsl_lighting);

        // If no subsystems are active, signal success immediately and return.
        if num_barriers == 0 {
            info!(
                song = song.name(),
                "No playback subsystems active for this song; completing immediately"
            );
            if play_tx.send(Ok(())).is_err() {
                error!("Error while sending to finish channel (receiver dropped).");
            }
            return;
        }

        let barrier = Arc::new(Barrier::new(num_barriers));

        let audio_outcome: Arc<parking_lot::Mutex<Option<Result<(), String>>>> =
            Arc::new(parking_lot::Mutex::new(None));

        let audio_join_handle = if let (Some(device), Some(mappings)) = (device, mappings) {
            let song = song.clone();
            let barrier = barrier.clone();
            let cancel_handle = cancel_handle.clone();
            let audio_outcome = audio_outcome.clone();

            Some(thread::spawn(move || {
                let song_name = song.name().to_string();
                let result = device.play_from(song, &mappings, cancel_handle, barrier, start_time);
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
            let barrier = barrier.clone();
            let cancel_handle = cancel_handle.clone();

            thread::spawn(move || {
                let song_name = song.name().to_string();

                if let Err(e) =
                    dmx::engine::Engine::play(dmx_engine, song, cancel_handle, barrier, start_time)
                {
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
            let barrier = barrier.clone();
            let cancel_handle = cancel_handle.clone();

            Some(thread::spawn(move || {
                let song_name = song.name().to_string();

                if let Err(e) = midi_device.play_from(song, cancel_handle, barrier, start_time) {
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

    /// If a song is currently playing, returns `Some(current_song)` so the caller can short-circuit.
    /// Returns `None` if the player is idle.
    async fn if_playing_then_current_song(&self) -> Option<Arc<Song>> {
        let join = self.join.lock().await;
        if join.is_some() {
            Some(self.get_playlist().current())
        } else {
            None
        }
    }

    /// Navigates the playlist using the given function, returning the current song
    /// if the player is active.
    async fn navigate<F>(&self, action: &str, nav_fn: F) -> Arc<Song>
    where
        F: FnOnce(Option<Arc<dyn midi::Device>>, Arc<Playlist>) -> Arc<Song>,
    {
        if let Some(current) = self.if_playing_then_current_song().await {
            info!(
                current_song = current.name(),
                "Can't go to {}, player is active.", action
            );
            return current;
        }
        let playlist = self.get_playlist();
        let song = nav_fn(self.midi_device.clone(), playlist);
        self.load_song_samples(&song);
        song
    }

    /// Next goes to the next entry in the playlist.
    pub async fn next(&self) -> Arc<Song> {
        self.navigate("next", Player::next_and_emit).await
    }

    /// Prev goes to the previous entry in the playlist.
    pub async fn prev(&self) -> Arc<Song> {
        self.navigate("previous", Player::prev_and_emit).await
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

        if self
            .stop_run
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            // Put the handles back since we're not stopping
            *join = Some(play_handles);
            info!("The previous stop is still processing.");
            return None;
        }

        let song = self.get_playlist().current();
        info!(song = song.name(), "Stopping playback.");

        play_handles.cancel.cancel();

        // Reset play start time — the cleanup task skips this when cancelled
        // so we must do it here.
        {
            let mut play_start_time = self.play_start_time.lock().await;
            *play_start_time = None;
        }

        // Reset stop_run immediately so play() is available right away.
        // The cleanup task won't touch join or play_start_time when cancelled,
        // so there's no clobber risk from a new play() starting before it runs.
        self.stop_run.store(false, Ordering::Relaxed);

        drop(play_handles.join);
        drop(join);

        Some(song)
    }

    /// Switches the active playlist if the player is idle.
    async fn switch_playlist(&self, use_all_songs: bool, label: &str) {
        if let Some(current) = self.if_playing_then_current_song().await {
            info!(
                current_song = current.name(),
                "Can't switch to {}, player is active.", label
            );
            return;
        }

        self.use_all_songs.store(use_all_songs, Ordering::Relaxed);
        let song = self.get_playlist().current();
        Player::emit_midi_event(self.midi_device.clone(), song.clone());
    }

    /// Switch to the all songs playlist.
    pub async fn switch_to_all_songs(&self) {
        self.switch_playlist(true, "all songs").await;
    }

    /// Switch to the regular playlist.
    pub async fn switch_to_playlist(&self) {
        self.switch_playlist(false, "playlist").await;
    }

    /// Returns the track-to-output-channel mappings, if audio is configured.
    pub fn track_mappings(&self) -> Option<&HashMap<String, Vec<u16>>> {
        self.mappings.as_deref()
    }

    /// Gets the all-songs playlist (every song in the registry).
    pub fn get_all_songs_playlist(&self) -> Arc<Playlist> {
        self.all_songs.clone()
    }

    /// Gets the current playlist used by the player.
    pub fn get_playlist(&self) -> Arc<Playlist> {
        if self.use_all_songs.load(Ordering::Relaxed) {
            return self.all_songs.clone();
        }

        self.playlist.clone()
    }

    /// Returns true if a song is currently playing.
    pub async fn is_playing(&self) -> bool {
        self.join.lock().await.is_some()
    }

    /// Returns the effect engine, if a DMX engine is configured.
    pub fn effect_engine(&self) -> Option<Arc<parking_lot::Mutex<crate::lighting::EffectEngine>>> {
        self.dmx_engine.as_ref().map(|e| e.effect_engine())
    }

    /// Gets the elapsed time from the play start time.
    pub async fn elapsed(&self) -> Result<Option<Duration>, Box<dyn Error>> {
        let play_start_time = self.play_start_time.lock().await;
        Ok(match *play_start_time {
            Some(play_start_time) => Some(play_start_time.elapsed()?),
            None => None,
        })
    }

    /// Gets a formatted string listing all active lighting effects
    pub fn format_active_effects(&self) -> Option<String> {
        self.dmx_engine
            .as_ref()
            .map(|engine| engine.format_active_effects())
    }

    /// Goes to the previous song and emits the MIDI event associated if one exists.
    fn prev_and_emit(
        midi_device: Option<Arc<dyn midi::Device>>,
        playlist: Arc<Playlist>,
    ) -> Arc<Song> {
        let song = playlist.prev();
        Player::emit_midi_event(midi_device, song.clone());
        song
    }

    /// Goes to the next song and emits the MIDI event associated if one exists.
    fn next_and_emit(
        midi_device: Option<Arc<dyn midi::Device>>,
        playlist: Arc<Playlist>,
    ) -> Arc<Song> {
        let song = playlist.next();
        Player::emit_midi_event(midi_device, song.clone());
        song
    }

    /// Emits a MIDI event for the given song if possible.
    fn emit_midi_event(midi_device: Option<Arc<dyn midi::Device>>, song: Arc<Song>) {
        if let Some(midi_device) = midi_device.clone() {
            let midi_event = song.midi_event();
            if let Err(e) = midi_device.emit(midi_event) {
                error!("Error emitting MIDI event: {:?}", e);
            }
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

/// Computes the number of barrier participants needed for playback synchronization.
fn compute_barrier_count(
    has_audio: bool,
    has_midi: bool,
    num_light_shows: usize,
    has_dsl_lighting: bool,
) -> usize {
    let mut count = 0;
    if has_audio {
        count += 1;
    }
    if has_midi {
        count += 1;
    }
    count += num_light_shows;
    if has_dsl_lighting {
        count += 1;
    }
    count
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
}

/// Decides whether to advance the playlist or stop after playback finishes.
fn decide_cleanup_action(result: PlaybackResult, cancelled: bool) -> CleanupAction {
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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_player() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let player = Player::new(
            songs.clone(),
            Playlist::new(
                "playlist",
                &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
                songs,
            )?,
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
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        player.next().await;
        println!("Playlist -> Song 3");
        assert_eq!(player.get_playlist().current().name(), "Song 3");

        player.prev().await;
        println!("Playlist -> Song 1");
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        println!("Switch to AllSongs");
        player.switch_to_all_songs().await;
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        player.next().await;
        println!("AllSongs -> Song 10");
        assert_eq!(player.get_playlist().current().name(), "Song 10");

        // No emitted events yet
        assert!(midi_device.get_emitted_event().is_none());

        player.next().await;
        println!("AllSongs -> Song 2");
        assert_eq!(player.get_playlist().current().name(), "Song 2");

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
        assert_eq!(player.get_playlist().current().name(), "Song 3");

        assert!(midi_device.get_emitted_event().is_none());

        player.switch_to_playlist().await;
        println!("Switch to Playlist");
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        player.next().await;
        println!("Playlist -> Song 3");
        assert_eq!(player.get_playlist().current().name(), "Song 3");

        player.play().await?;

        // Playlist should have moved to next song.
        eventually(
            || player.get_playlist().current().name() == "Song 5",
            format!(
                "Song never moved to next, on song {}",
                player.get_playlist().current().name()
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
        assert_eq!(player.get_playlist().current().name(), "Song 5");

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
    invalid_group: static color: "blue", dimmer: 60%
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
            songs,
            playlist,
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

        // Try to play the song - it should fail due to invalid lighting show
        let result = player.play().await;
        assert!(
            result.is_err(),
            "Player should reject song with invalid lighting show"
        );

        Ok(())
    }

    /// Flexible helper to create a player with the standard test assets and
    /// optional subsystem configs.
    fn make_test_player_with_config(
        audio: Option<config::Audio>,
        midi: Option<config::Midi>,
        dmx: Option<config::Dmx>,
    ) -> Result<Player, Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        Ok(Player::new(
            songs.clone(),
            Playlist::new(
                "playlist",
                &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
                songs,
            )?,
            &config::Player::new(vec![], audio, midi, dmx, HashMap::new(), "assets/songs"),
            None,
        )?)
    }

    /// Helper to create a player with the standard test assets (audio + MIDI).
    fn make_test_player() -> Result<Player, Box<dyn Error>> {
        make_test_player_with_config(
            Some(config::Audio::new("mock-device")),
            Some(config::Midi::new("mock-midi-device", None)),
            None,
        )
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_stop_when_not_playing() -> Result<(), Box<dyn Error>> {
        let player = make_test_player()?;

        // stop() when nothing is playing should return None.
        let result = player.stop().await;
        assert!(result.is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_is_playing() -> Result<(), Box<dyn Error>> {
        let player = make_test_player()?;
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
        let player = make_test_player()?;

        // elapsed() when not playing should be Ok(None).
        let elapsed = player.elapsed().await?;
        assert!(elapsed.is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_elapsed_while_playing() -> Result<(), Box<dyn Error>> {
        let player = make_test_player()?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        let elapsed = player.elapsed().await?;
        assert!(
            elapsed.is_some(),
            "elapsed should have a value while playing"
        );

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_concurrent_play_returns_none() -> Result<(), Box<dyn Error>> {
        let player = make_test_player()?;
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
        let player = make_test_player()?;

        assert_eq!(player.get_playlist().current().name(), "Song 1");

        let song = player.next().await;
        assert_eq!(song.name(), "Song 3");
        assert_eq!(player.get_playlist().current().name(), "Song 3");

        let song = player.prev().await;
        assert_eq!(song.name(), "Song 1");
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_switch_playlists() -> Result<(), Box<dyn Error>> {
        let player = make_test_player()?;

        assert_eq!(player.get_playlist().name(), "playlist");
        player.switch_to_all_songs().await;
        assert_eq!(player.get_playlist().name(), "all_songs");
        player.switch_to_playlist().await;
        assert_eq!(player.get_playlist().name(), "playlist");

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_all_songs_playlist() -> Result<(), Box<dyn Error>> {
        let player = make_test_player()?;
        let all = player.get_all_songs_playlist();
        assert_eq!(all.name(), "all_songs");
        assert!(!all.songs().is_empty());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_format_active_effects_no_dmx() -> Result<(), Box<dyn Error>> {
        let player = make_test_player()?;
        // No DMX engine configured, should return None.
        assert!(player.format_active_effects().is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_dmx_engine_none_without_config() -> Result<(), Box<dyn Error>> {
        let player = make_test_player()?;
        assert!(player.dmx_engine().is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_get_cues_empty_without_lighting() -> Result<(), Box<dyn Error>> {
        let player = make_test_player()?;
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
    invalid_group_1: static color: "blue", dimmer: 60%
    invalid_group_2: static color: "red", dimmer: 80%
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
            songs,
            playlist,
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
        let player = make_test_player()?;
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
        let player = make_test_player()?;
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
        let player = make_test_player()?;
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
            make_test_player_with_config(Some(config::Audio::new("mock-device")), None, None)?;
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
        player.switch_to_all_songs().await;
        // all_songs starts at Song 1, navigate to Song 2.
        player.get_playlist().next(); // Song 10
        player.get_playlist().next(); // Song 2
        assert_eq!(player.get_playlist().current().name(), "Song 2");

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
        )?;

        assert!(
            player.audio_device().is_none(),
            "Audio device should be absent"
        );

        // Song 1 has midi_file. Barrier = 1 (MIDI only).
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        player.play().await?;

        // Natural finish: playlist should advance.
        eventually_async(
            || async { player.get_playlist().current().name() != "Song 1" },
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
        let player = Player {
            device: None,
            mappings: None,
            midi_device: None,
            dmx_engine: None,
            sample_engine: None,
            trigger_engine: None,
            playlist: playlist.clone(),
            all_songs: crate::playlist::from_songs(songs)?,
            use_all_songs: Arc::new(AtomicBool::new(false)),
            play_start_time: Arc::new(Mutex::new(None)),
            join: Arc::new(Mutex::new(None)),
            stop_run: Arc::new(AtomicBool::new(false)),
            span: span!(Level::INFO, "test"),
        };

        assert!(player.audio_device().is_none());
        assert!(player.midi_device().is_none());
        assert!(player.dmx_engine().is_none());

        player.play().await?;

        // num_barriers == 0 → play_files returns immediately → playlist advances.
        eventually_async(
            || async { player.get_playlist().current().name() != "Song 1" },
            "Playlist never advanced after no-subsystem playback",
        )
        .await;

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_natural_finish_clears_play_state() -> Result<(), Box<dyn Error>> {
        let player = make_test_player()?;

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
        )?;

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
        let player = make_test_player()?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        assert_eq!(player.get_playlist().name(), "playlist");

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // Attempt switch while playing — should be a no-op.
        player.switch_to_all_songs().await;
        assert_eq!(
            player.get_playlist().name(),
            "playlist",
            "switch_to_all_songs should be a no-op while playing"
        );

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_playlist_clamps_at_end_on_natural_finish() -> Result<(), Box<dyn Error>> {
        let player = make_test_player()?;

        // Navigate to Song 9 (last in playlist).
        // Playlist: Song 1, Song 3, Song 5, Song 7, Song 9
        player.next().await; // Song 3
        player.next().await; // Song 5
        player.next().await; // Song 7
        player.next().await; // Song 9
        assert_eq!(player.get_playlist().current().name(), "Song 9");

        // Play Song 9 — short audio (0.5s), should finish naturally.
        player.play().await?;

        // After natural finish, playlist next() clamps at the last song.
        eventually_async(
            || async { !player.is_playing().await },
            "Player never stopped after Song 9 finished",
        )
        .await;

        // Playlist should still be at Song 9 (clamped, no wrap).
        assert_eq!(player.get_playlist().current().name(), "Song 9");

        Ok(())
    }

    // --- compute_barrier_count tests ---

    #[test]
    fn barrier_count_no_subsystems() {
        assert_eq!(compute_barrier_count(false, false, 0, false), 0);
    }

    #[test]
    fn barrier_count_audio_only() {
        assert_eq!(compute_barrier_count(true, false, 0, false), 1);
    }

    #[test]
    fn barrier_count_midi_only() {
        assert_eq!(compute_barrier_count(false, true, 0, false), 1);
    }

    #[test]
    fn barrier_count_one_legacy_show() {
        assert_eq!(compute_barrier_count(false, false, 1, false), 1);
    }

    #[test]
    fn barrier_count_three_legacy_shows() {
        assert_eq!(compute_barrier_count(false, false, 3, false), 3);
    }

    #[test]
    fn barrier_count_dsl_lighting_only() {
        assert_eq!(compute_barrier_count(false, false, 0, true), 1);
    }

    #[test]
    fn barrier_count_all_subsystems() {
        // 1 audio + 1 midi + 2 legacy + 1 dsl = 5
        assert_eq!(compute_barrier_count(true, true, 2, true), 5);
    }

    #[test]
    fn barrier_count_audio_and_midi_no_dmx() {
        assert_eq!(compute_barrier_count(true, true, 0, false), 2);
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
            decide_cleanup_action(PlaybackResult::Success, false),
            CleanupAction::AdvancePlaylist
        );
    }

    #[test]
    fn cleanup_success_cancelled() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Success, true),
            CleanupAction::StopCancelled
        );
    }

    #[test]
    fn cleanup_failed_not_cancelled() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Failed("err".into()), false),
            CleanupAction::AdvancePlaylist
        );
    }

    #[test]
    fn cleanup_failed_cancelled() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::Failed("err".into()), true),
            CleanupAction::StopCancelled
        );
    }

    #[test]
    fn cleanup_sender_dropped_not_cancelled() {
        assert_eq!(
            decide_cleanup_action(PlaybackResult::SenderDropped, false),
            CleanupAction::AdvancePlaylist
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
        )?;

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
        let player = make_test_player()?;
        assert!(
            player.track_mappings().is_some(),
            "track_mappings should be Some when audio is configured"
        );

        // Player without audio → track_mappings is None
        let player = make_test_player_with_config(
            None,
            Some(config::Midi::new("mock-midi-device", None)),
            None,
        )?;
        assert!(
            player.track_mappings().is_none(),
            "track_mappings should be None without audio"
        );

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_next_while_playing_returns_current() -> Result<(), Box<dyn Error>> {
        let player = make_test_player()?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        assert_eq!(player.get_playlist().current().name(), "Song 1");

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        // next() while playing should return the current song without advancing
        let song = player.next().await;
        assert_eq!(
            song.name(),
            "Song 1",
            "next() while playing should return current song"
        );
        assert_eq!(
            player.get_playlist().current().name(),
            "Song 1",
            "playlist should not advance while playing"
        );

        // prev() while playing should also return the current song
        let song = player.prev().await;
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
    fn wait_for_ok_succeeds_immediately() {
        let result = Player::wait_for_ok("test", || Ok::<_, String>(42));
        assert_eq!(result.unwrap(), 42);
    }

    #[test]
    fn wait_for_ok_retries_then_fails() {
        let attempt = std::sync::atomic::AtomicU32::new(0);
        let result = Player::wait_for_ok_with_limit("test device", 2, || {
            attempt.fetch_add(1, Ordering::Relaxed);
            Err::<(), String>("boom".into())
        });
        assert!(result.is_err());
        assert!(attempt.load(Ordering::Relaxed) >= 2);
    }

    #[test]
    fn wait_for_ok_succeeds_after_retry() {
        let attempt = std::sync::atomic::AtomicU32::new(0);
        let result = Player::wait_for_ok_with_limit("test device", 5, || {
            let n = attempt.fetch_add(1, Ordering::Relaxed);
            if n < 2 {
                Err::<u32, String>("not ready".into())
            } else {
                Ok(99)
            }
        });
        assert_eq!(result.unwrap(), 99);
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
        Ok(Player::new_with_devices(devices, playlist, songs)?)
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
        let player = make_test_player()?;
        assert!(player.broadcast_handles().is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_effect_engine_no_dmx() -> Result<(), Box<dyn Error>> {
        let player = make_test_player()?;
        assert!(player.effect_engine().is_none());
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_set_broadcast_tx_no_dmx() -> Result<(), Box<dyn Error>> {
        let player = make_test_player()?;
        let (tx, _rx) = tokio::sync::broadcast::channel(1);
        player.set_broadcast_tx(tx);
        Ok(())
    }

    #[test]
    fn emit_midi_event_no_device() {
        let song = Arc::new(Song::new_for_test("test", &[]));
        Player::emit_midi_event(None, song);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_switch_to_playlist_while_playing_stays() -> Result<(), Box<dyn Error>> {
        let player = make_test_player()?;
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;

        player.switch_to_all_songs().await;
        assert_eq!(player.get_playlist().name(), "all_songs");

        player.play().await?;
        eventually(|| device.is_playing(), "Song never started playing");

        player.switch_to_playlist().await;
        assert_eq!(
            player.get_playlist().name(),
            "all_songs",
            "switch_to_playlist should be a no-op while playing"
        );

        player.stop().await;
        eventually(|| !device.is_playing(), "Song never stopped playing");
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_prev_and_emit_no_midi() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "test",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;

        Player::next_and_emit(None, playlist.clone());
        let song = Player::prev_and_emit(None, playlist);
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
        let player = Player::new_with_devices(devices, playlist, songs)?;
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
        let player = make_test_player()?;
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
        let player = Player::new_with_devices(devices, playlist, songs)?;

        player.process_sample_trigger(&[0x90, 60, 127]);
        player.stop_samples();

        let song = player.next().await;
        assert_eq!(song.name(), "Song 3");

        let song = player.prev().await;
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
        let player = make_test_player()?;
        assert!(player.midi_device().is_some());

        let player =
            make_test_player_with_config(Some(config::Audio::new("mock-device")), None, None)?;
        assert!(player.midi_device().is_none());
        Ok(())
    }
}
