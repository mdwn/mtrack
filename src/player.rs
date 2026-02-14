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
use crate::{
    audio, config, dmx, midi,
    playlist::{self, Playlist},
    playsync::CancelHandle,
    songs::Song,
};

struct PlayHandles {
    join: JoinHandle<()>,
    cancel: CancelHandle,
}

/// Groups the parameters needed for `play_files` to avoid excessive argument counts.
struct PlaybackContext {
    device: Arc<dyn audio::Device>,
    mappings: Arc<HashMap<String, Vec<u16>>>,
    midi_device: Option<Arc<dyn midi::Device>>,
    dmx_engine: Option<Arc<dmx::engine::Engine>>,
    song: Arc<Song>,
    cancel_handle: CancelHandle,
    play_tx: oneshot::Sender<Result<(), String>>,
    start_time: Duration,
}

/// Plays back individual wav files as multichannel audio for the configured audio interface.
#[derive(Clone)]
pub struct Player {
    /// The device to play audio through.
    device: Arc<dyn audio::Device>,
    /// Mappings of tracks to output channels.
    mappings: Arc<HashMap<String, Vec<u16>>>,
    /// The MIDI device to play MIDI back through.
    midi_device: Option<Arc<dyn midi::Device>>,
    /// The DMX engine to use.
    dmx_engine: Option<Arc<dmx::engine::Engine>>,
    /// The sample engine for MIDI-triggered samples.
    sample_engine: Option<Arc<RwLock<SampleEngine>>>,
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
    /// Creates a new player.
    pub fn new(
        songs: Arc<Songs>,
        playlist: Arc<Playlist>,
        config: &config::Player,
        base_path: Option<&Path>,
    ) -> Result<Player, Box<dyn Error>> {
        let span = span!(Level::INFO, "player");
        let _enter = span.enter();

        let device = Self::wait_for_ok("audio device".to_string(), || {
            audio::get_device(config.audio())
        })?;
        let dmx_engine = Self::wait_for_ok("dmx engine".to_string(), || {
            dmx::create_engine(config.dmx(), base_path)
        })?;
        let midi_device = Self::wait_for_ok("midi device".to_string(), || {
            midi::get_device(config.midi(), dmx_engine.clone())
        })?;
        let status_events = Self::wait_for_ok("status events".to_string(), || {
            StatusEvents::new(config.status_events())
        })?;

        // Initialize the sample engine if the audio device supports it
        let sample_engine = match (device.mixer(), device.source_sender()) {
            (Some(mixer), Some(source_tx)) => {
                let max_voices = config.max_sample_voices();
                let buffer_size = config.audio().map(|a| a.buffer_size()).unwrap_or(1024);
                let mut engine = SampleEngine::new(mixer, source_tx, max_voices, buffer_size);

                // Load global samples config if available
                if let Some(base_path) = base_path {
                    match config.samples_config(base_path) {
                        Ok(samples_config) => {
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
            _ => None,
        };

        let player = Player {
            device,
            mappings: Arc::new(config.track_mappings().clone()),
            midi_device,
            dmx_engine,
            sample_engine,
            playlist,
            all_songs: playlist::from_songs(songs)?,
            use_all_songs: Arc::new(AtomicBool::new(false)),
            play_start_time: Arc::new(Mutex::new(None)),
            join: Arc::new(Mutex::new(None)),
            stop_run: Arc::new(AtomicBool::new(false)),
            span: span.clone(),
        };

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

    /// Wait for constructor function to return an Ok(result) variant.
    /// Respects MTRACK_DEVICE_RETRY_LIMIT: if set to N, tries at most N times then returns
    /// the last error. If unset or 0, retries indefinitely (original behavior).
    fn wait_for_ok<T, E, F>(name: String, constructor: F) -> Result<T, Box<dyn Error>>
    where
        E: Display + Into<Box<dyn Error>>,
        F: Fn() -> Result<T, E>,
    {
        let max_attempts = std::env::var("MTRACK_DEVICE_RETRY_LIMIT")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(0);
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
    pub fn audio_device(&self) -> Arc<dyn audio::Device> {
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
        crate::verify::warn_unmapped_tracks(&song, &self.mappings);

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
                let playback_ok = match play_rx.await {
                    Ok(Ok(())) => true,
                    Ok(Err(e)) => {
                        error!(
                            err = %e,
                            song = song.name(),
                            "Playback failed (e.g. audio error); playlist not advanced"
                        );
                        false
                    }
                    Err(_e) => {
                        error!("Error receiving playback signal (receiver dropped)");
                        false
                    }
                };
                let mut join = join_mutex.lock().await;

                let cancelled = cancel_handle_for_cleanup.is_cancelled();
                // Only move to the next playlist entry if not cancelled. On playback failure we still
                // advance so the user is not stuck, but we already logged the error above.
                if !cancelled {
                    if !playback_ok {
                        warn!("Advancing playlist despite playback failure so user is not stuck");
                    }
                    Player::next_and_emit(midi_device.clone(), playlist);
                }

                // Reset the play start time as well.
                {
                    let mut play_start_time = play_start_time.lock().await;
                    *play_start_time = None;
                }

                info!(
                    song = song.name(),
                    cancelled = cancelled,
                    playback_ok = playback_ok,
                    "Song finished playing."
                );

                // Remove the handles and reset stop run.
                // Note: stop() may have already cleared this, but we ensure it's cleared here too
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

        // Set up the play barrier, which will synchronize the three calls to play.
        let barrier = Arc::new(Barrier::new({
            let mut num_barriers = 1;
            if song.midi_playback().is_some() && midi_device.is_some() {
                num_barriers += 1;
            }
            if !song.light_shows().is_empty() && dmx_engine.is_some() {
                num_barriers += song.light_shows().len();
            }
            if !song.dsl_lighting_shows().is_empty() && dmx_engine.is_some() {
                num_barriers += 1; // One barrier for the lighting timeline
            }
            num_barriers
        }));

        let audio_outcome: Arc<parking_lot::Mutex<Option<Result<(), String>>>> =
            Arc::new(parking_lot::Mutex::new(None));

        let audio_join_handle = {
            let device = device.clone();
            let song = song.clone();
            let barrier = barrier.clone();
            let cancel_handle = cancel_handle.clone();
            let audio_outcome = audio_outcome.clone();

            thread::spawn(move || {
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
            })
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

        if let Err(e) = audio_join_handle.join() {
            error!("Error waiting for audio to stop playing: {:?}", e)
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

        let outcome = audio_outcome.lock().take().unwrap_or_else(|| {
            warn!(
                "Audio thread did not set outcome (e.g. panicked before setting); \
                 treating as success so playlist is not stuck"
            );
            Ok(())
        });
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

        drop(play_handles.join);
        drop(join);

        // stop_run is cleared by the cleanup task when it runs (after play_rx fires). As a fallback
        // (e.g. if cleanup never runs because play_tx was dropped or the blocking task hangs),
        // clear stop_run after a timeout so the user can call stop() again.
        const STOP_RUN_FALLBACK_TIMEOUT: Duration = Duration::from_secs(30);
        let stop_run = self.stop_run.clone();
        tokio::spawn(async move {
            tokio::time::sleep(STOP_RUN_FALLBACK_TIMEOUT).await;
            if stop_run
                .compare_exchange(true, false, Ordering::Relaxed, Ordering::Relaxed)
                .is_ok()
            {
                warn!(
                    "Stop cleanup did not complete within {:?}; cleared stop_run so further stop() can be attempted",
                    STOP_RUN_FALLBACK_TIMEOUT
                );
            }
        });

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

    /// Gets the current playlist used by the player.
    pub fn get_playlist(&self) -> Arc<Playlist> {
        if self.use_all_songs.load(Ordering::Relaxed) {
            return self.all_songs.clone();
        }

        self.playlist.clone()
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

#[cfg(test)]
mod test {
    use std::{collections::HashMap, error::Error, fs, path::Path, sync::Arc};

    use crate::{config, playlist::Playlist, songs, testutil::eventually};

    use super::Player;

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
                config::Audio::new("mock-device"),
                Some(config::Midi::new("mock-midi-device", None)),
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?;
        let binding = player.audio_device();
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
                config::Audio::new("mock-device"),
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

    #[tokio::test(flavor = "multi_thread")]
    async fn test_player_accepts_valid_lighting_shows() -> Result<(), Box<dyn Error>> {
        // Create a temporary directory for test files
        let temp_dir = tempfile::tempdir()?;
        let temp_path = temp_dir.path();

        // Create a valid lighting show file
        let lighting_show_content = r#"show "Test Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
}"#;
        let lighting_file = temp_path.join("valid_show.light");
        fs::write(&lighting_file, lighting_show_content)?;

        // Create a WAV file for the song
        let wav_file = temp_path.join("track.wav");
        crate::testutil::write_wav(wav_file.clone(), vec![vec![1_i32, 2_i32, 3_i32]], 44100)?;

        // Create a song with the valid lighting show
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
            vec![config::Track::new(
                "track".to_string(),
                wav_file.file_name().unwrap().to_str().unwrap(),
                Some(1),
            )],
            HashMap::new(),
            Vec::new(),
        );

        // Create a lighting config with the valid group
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
                config::Audio::new("mock-device"),
                Some(config::Midi::new("mock-midi-device", None)),
                Some(dmx_config),
                HashMap::new(),
                temp_path.to_str().unwrap(),
            ),
            Some(temp_path),
        )?;

        // Test validation directly through the DMX engine to avoid starting playback
        // This tests that validation works without the complexity of actual playback
        let dmx_engine = player
            .dmx_engine()
            .expect("DMX engine should be present for this test");
        let song = player.get_playlist().current();
        let validation_result = dmx_engine.validate_song_lighting(&song);
        assert!(
            validation_result.is_ok(),
            "DMX engine should accept song with valid lighting show: {:?}",
            validation_result.err()
        );

        // Verify that play() would succeed by checking it returns quickly
        // We use a timeout to detect if play() hangs (which would indicate a problem)
        let play_result =
            tokio::time::timeout(std::time::Duration::from_millis(500), player.play()).await;

        match play_result {
            Ok(Ok(Some(_))) => {
                // play() succeeded quickly, validation passed
                // Stop immediately to avoid hanging on cleanup
                player.stop().await;
            }
            Ok(Ok(None)) => {
                panic!("Unexpected: song already playing");
            }
            Ok(Err(e)) => {
                panic!(
                    "play() should succeed with valid lighting show, got error: {}",
                    e
                );
            }
            Err(_) => {
                panic!("play() timed out after 500ms - this suggests it's hanging during validation or thread spawning");
            }
        }

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_player_accepts_song_without_lighting_shows() -> Result<(), Box<dyn Error>> {
        // Create a temporary directory for test files
        let temp_dir = tempfile::tempdir()?;
        let temp_path = temp_dir.path();

        // Create a WAV file for the song
        let wav_file = temp_path.join("track.wav");
        crate::testutil::write_wav(wav_file.clone(), vec![vec![1_i32, 2_i32, 3_i32]], 44100)?;

        // Create a song without lighting shows
        let song_config = config::Song::new(
            "Test Song",
            None,
            None,
            None,
            None,
            None, // No lighting shows
            vec![config::Track::new(
                "track".to_string(),
                wav_file.file_name().unwrap().to_str().unwrap(),
                Some(1),
            )],
            HashMap::new(),
            Vec::new(),
        );

        // Create DMX config without lighting (or with lighting, shouldn't matter)
        let dmx_config = config::Dmx::new(
            Some(1.0),
            Some("0s".to_string()),
            Some(9090),
            vec![config::Universe::new(1, "test_universe".to_string())],
            None, // No lighting config
        );

        // Create a simple playlist with one song
        let playlist_songs = vec!["Test Song".to_string()];
        let playlist_config = config::Playlist::new(&playlist_songs);
        let song = songs::Song::new(temp_path, &song_config)?;
        let songs_map = HashMap::from([("Test Song".to_string(), Arc::new(song))]);
        let songs = Arc::new(songs::Songs::new(songs_map));
        let playlist = Playlist::new("Test Playlist", &playlist_config, songs.clone())?;

        // Create player with DMX engine
        let player = Player::new(
            songs,
            playlist,
            &config::Player::new(
                vec![],
                config::Audio::new("mock-device"),
                Some(config::Midi::new("mock-midi-device", None)),
                Some(dmx_config),
                HashMap::new(),
                temp_path.to_str().unwrap(),
            ),
            Some(temp_path),
        )?;

        // Try to play the song - it should succeed even without lighting shows
        let result = player.play().await;
        assert!(
            matches!(result, Ok(Some(_))),
            "Player should accept song without lighting shows"
        );

        // Stop playback immediately - validation test doesn't need full playback
        // Don't wait for cleanup - validation is what we're testing, not playback completion
        player.stop().await;

        // Give a brief moment for stop to take effect, but don't wait for full cleanup
        // The validation test has already verified that play() succeeded (validation passed)
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

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
                config::Audio::new("mock-device"),
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
}
