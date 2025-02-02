// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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
use std::{
    collections::HashMap,
    error::Error,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Barrier, RwLock,
    },
    thread,
    time::Duration,
};
use tokio::{
    sync::{oneshot, Mutex},
    task::JoinHandle,
};
use tracing::{error, info, span, Level, Span};

use crate::songs::Songs;
use crate::{
    audio, config, dmx, midi, playlist, playlist::Playlist, playsync::CancelHandle, songs::Song,
};

struct PlayHandles {
    join: JoinHandle<()>,
    cancel: CancelHandle,
}

/// Plays back individual wav files as multichannel audio for the configured audio interface.
pub struct Player {
    /// The device to play audio through.
    device: Arc<dyn audio::Device>,
    /// Mappings of tracks to output channels.
    mappings: Arc<HashMap<String, Vec<u16>>>,
    /// The MIDI device to play MIDI back through.
    midi_device: Option<Arc<dyn midi::Device>>,
    /// The DMX engine to use.
    dmx_engine: Option<Arc<RwLock<dmx::engine::Engine>>>,
    /// The playlist to use.
    playlist: Arc<Playlist>,
    /// The all songs playlist.
    all_songs: Arc<Playlist>,
    /// Switches between the playlist and the all songs playlist.
    use_all_songs: AtomicBool,
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
    ) -> Result<Player, Box<dyn Error>> {
        let midi_device = midi::get_device(config.midi())?;
        Self::new_with_midi_device(songs, playlist, midi_device, config)
    }

    pub fn new_with_midi_device(
        songs: Arc<Songs>,
        playlist: Arc<Playlist>,
        midi_device: Option<Arc<dyn midi::Device>>,
        config: &config::Player,
    ) -> Result<Player, Box<dyn Error>> {
        let device = audio::get_device(config.audio())?;
        let dmx_engine = dmx::create_engine(config.dmx())?;
        let status_events = StatusEvents::new(config.status_events())?;

        let player = Player {
            device,
            mappings: Arc::new(config.track_mappings().clone()),
            midi_device,
            dmx_engine,
            playlist,
            all_songs: playlist::from_songs(songs)?,
            use_all_songs: AtomicBool::new(false),
            join: Arc::new(Mutex::new(None)),
            stop_run: Arc::new(AtomicBool::new(false)),
            span: span!(Level::INFO, "player"),
        };

        if player.midi_device.is_none() {
            return Ok(player);
        }

        // Emit the event for the first track if needed.
        Player::emit_midi_event(player.midi_device.clone(), player.get_playlist().current());

        if let Some(status_events) = status_events {
            let midi_device = player
                .midi_device
                .clone()
                .expect("MIDI device must be present");
            let join = player.join.clone();
            tokio::spawn(Player::report_status(midi_device, join, status_events));
        }

        Ok(player)
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

    /// Reports status as MIDI events.
    async fn report_status(
        midi_device: Arc<dyn midi::Device>,
        join: Arc<Mutex<Option<PlayHandles>>>,
        status_events: StatusEvents,
    ) {
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

    /// Plays the song at the current position. Returns true if the song was submitted successfully.
    pub async fn play(&self) -> Result<(), Box<dyn Error>> {
        let _enter = self.span.enter();

        let mut join = self.join.lock().await;

        let playlist = self.get_playlist().clone();
        if join.is_some() {
            info!(
                current_song = playlist.current().name(),
                "Player is already playing a song."
            );
            return Ok(());
        }

        let song = playlist.current();
        let cancel_handle = CancelHandle::new();
        let (play_tx, play_rx) = oneshot::channel::<()>();

        let join_handle = {
            let song = song.clone();
            let device = self.device.clone();
            let midi_device = self.midi_device.clone();
            let dmx_engine = self.dmx_engine.clone();
            let cancel_handle = cancel_handle.clone();
            let mappings = self.mappings.clone();
            tokio::task::spawn_blocking(move || {
                Player::play_files(
                    device,
                    mappings,
                    midi_device,
                    dmx_engine,
                    song,
                    cancel_handle,
                    play_tx,
                );
            })
        };
        *join = Some(PlayHandles {
            join: join_handle,
            cancel: cancel_handle,
        });

        let join_mutex = self.join.clone();
        let stop_run = self.stop_run.clone();
        let song = song.clone();
        let midi_device = self.midi_device.clone();
        tokio::spawn(async move {
            if let Err(e) = play_rx.await {
                error!(err = e.to_string(), "Error receiving signal");
                return;
            }
            let mut join = join_mutex.lock().await;

            let mut cancelled = false;
            // Only move to the next playlist entry if this wasn't cancelled.
            if let Some(join) = join.as_ref() {
                cancelled = join.cancel.is_cancelled();
                if !cancelled {
                    Player::next_and_emit(midi_device.clone(), playlist);
                }
            }

            info!(
                song = song.name(),
                cancelled = cancelled,
                "Song finished playing."
            );

            // Remove the handles and reset stop run.
            *join = None;
            stop_run.store(false, Ordering::Relaxed);
        });

        Ok(())
    }

    fn play_files(
        device: Arc<dyn audio::Device>,
        mappings: Arc<HashMap<String, Vec<u16>>>,
        midi_device: Option<Arc<dyn midi::Device>>,
        dmx_engine: Option<Arc<RwLock<dmx::engine::Engine>>>,
        song: Arc<Song>,
        cancel_handle: CancelHandle,
        play_tx: oneshot::Sender<()>,
    ) {
        let song = song.clone();
        let cancel_handle = cancel_handle.clone();

        // Set up the play barrier, which will synchronize the three calls to play.
        let barrier = Arc::new(Barrier::new({
            let mut num_barriers = 1;
            if song.midi_playback().is_some() && midi_device.is_some() {
                num_barriers += 1;
            }
            if !song.light_shows().is_empty() && dmx_engine.is_some() {
                num_barriers += song.light_shows().len();
            }
            num_barriers
        }));

        let audio_join_handle = {
            let device = device.clone();
            let song = song.clone();
            let barrier = barrier.clone();
            let cancel_handle = cancel_handle.clone();

            thread::spawn(move || {
                let song_name = song.name().to_string();

                if let Err(e) = device.play(song, &mappings, cancel_handle, barrier) {
                    error!(
                        err = e.as_ref(),
                        song = song_name,
                        "Error while playing song"
                    );
                }
            })
        };

        let dmx_join_handle = dmx_engine.map(|dmx_engine| {
            let dmx_engine = dmx_engine.clone();
            let song = song.clone();
            let barrier = barrier.clone();
            let cancel_handle = cancel_handle.clone();

            thread::spawn(move || {
                let song_name = song.name().to_string();

                if let Err(e) = dmx::engine::Engine::play(dmx_engine, song, cancel_handle, barrier)
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

                if let Err(e) = midi_device.play(song, cancel_handle, barrier) {
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

        if play_tx.send(()).is_err() {
            error!("Error while sending to finish channel.")
        }
    }

    /// Wait for the current song to stop. Just waits until the is playing mutex is finished. Returns true
    /// if a song was waited for.
    pub async fn wait_for_current_song(&self) -> Result<bool, Box<dyn Error>> {
        let _enter = self.span.enter();

        let mut join = self.join.lock().await;
        Ok(match join.as_mut() {
            Some(join) => {
                info!(
                    song = self.get_playlist().current().name(),
                    "Waiting for song to finish.",
                );
                // Wait for the mutex to become available and immediately drop it.
                (&mut join.join).await?;
                true
            }
            None => false,
        })
    }

    /// Next goes to the next entry in the playlist.
    pub async fn next(&self) -> Result<Arc<Song>, Box<dyn Error>> {
        let join = self.join.lock().await;
        let playlist = self.get_playlist();
        if join.is_some() {
            let current = playlist.current();
            info!(
                current_song = current.name(),
                "Can't go to next, player is active."
            );
            return Ok(current);
        }
        Ok(Player::next_and_emit(self.midi_device.clone(), playlist))
    }

    /// Prev goes to the previous entry in the playlist.
    pub async fn prev(&self) -> Result<Arc<Song>, Box<dyn Error>> {
        let join = self.join.lock().await;
        let playlist = self.get_playlist();
        if join.is_some() {
            let current = playlist.current();
            info!(
                current_song = current.name(),
                "Can't go to previous, player is active."
            );
            return Ok(current);
        }
        Ok(Player::prev_and_emit(self.midi_device.clone(), playlist))
    }

    /// Stop will stop a song if a song is playing.
    pub async fn stop(&mut self) -> Result<(), Box<dyn Error>> {
        let mut join = self.join.lock().await;

        let join = match join.as_mut() {
            Some(join) => join,
            None => {
                info!("Player is not active, nothing to stop.");
                return Ok(());
            }
        };

        if self
            .stop_run
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            info!("The previous stop is still processing.");
            return Ok(());
        }

        info!(
            song = self.get_playlist().current().name(),
            "Stopping playback."
        );

        // Cancel the playback.
        join.cancel.cancel();
        Ok(())
    }

    /// Switch to the all songs playlist.
    pub async fn switch_to_all_songs(&self) -> Result<(), Box<dyn Error>> {
        println!("Switching to all songs");
        let join = self.join.lock().await;
        if join.is_some() {
            info!(
                current_song = self.get_playlist().current().name(),
                "Can't switch to all songs, player is active."
            );
            return Ok(());
        }

        self.use_all_songs.store(true, Ordering::Relaxed);
        let song = self.get_playlist().current();
        Player::emit_midi_event(self.midi_device.clone(), song.clone());
        Ok(())
    }

    /// Switch to the regular playlist.
    pub async fn switch_to_playlist(&self) -> Result<(), Box<dyn Error>> {
        println!("Switching to playlist");
        let join = self.join.lock().await;
        if join.is_some() {
            info!(
                current_song = self.get_playlist().current().name(),
                "Can't switch to playlist, player is active."
            );
            return Ok(());
        }

        self.use_all_songs.store(false, Ordering::Relaxed);
        let song = self.get_playlist().current();
        Player::emit_midi_event(self.midi_device.clone(), song.clone());
        Ok(())
    }

    /// Gets the current playlist used by the player.
    pub fn get_playlist(&self) -> Arc<Playlist> {
        if self.use_all_songs.load(Ordering::Relaxed) {
            return self.all_songs.clone();
        }

        self.playlist.clone()
    }

    /// Gets the all songs playlist used by the player.
    #[cfg(test)]
    pub fn get_all_songs_playlist(&self) -> Arc<Playlist> {
        self.all_songs.clone()
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
    use std::{collections::HashMap, error::Error, path::PathBuf, sync::Arc};

    use crate::{config, midi, playlist::Playlist, songs, test::eventually};

    use super::Player;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_player() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(&PathBuf::from("assets/songs"))?;
        let midi_device = Arc::new(midi::test::Device::get("mock-midi-device"));
        let mut player = Player::new_with_midi_device(
            songs.clone(),
            Playlist::new(
                &config::Playlist::deserialize(&PathBuf::from("assets/playlist.yaml"))?,
                songs,
            )?,
            Some(midi_device.clone()),
            &config::Player::new(
                config::Controller::Keyboard,
                config::Audio::new("mock-device"),
                Some(config::Midi::new("mock-midi-device", None)),
                None,
                HashMap::new(),
                "assets/songs",
            ),
        )?;
        let binding = player.audio_device();
        let device = binding.to_mock()?;

        // Direct the player.
        println!("Playlist -> Song 1");
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        player.next().await?;
        println!("Playlist -> Song 3");
        assert_eq!(player.get_playlist().current().name(), "Song 3");

        player.prev().await?;
        println!("Playlist -> Song 1");
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        println!("Switch to AllSongs");
        player.switch_to_all_songs().await?;
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        player.next().await?;
        println!("AllSongs -> Song 10");
        assert_eq!(player.get_playlist().current().name(), "Song 10");

        // No emitted events yet
        assert!(midi_device.get_emitted_event().is_none());

        player.next().await?;
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

        player.next().await?;
        println!("AllSongs -> Song 3");
        assert_eq!(player.get_playlist().current().name(), "Song 3");

        assert!(midi_device.get_emitted_event().is_none());

        player.switch_to_playlist().await?;
        println!("Switch to Playlist");
        assert_eq!(player.get_playlist().current().name(), "Song 1");

        player.next().await?;
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

        player.stop().await?;
        eventually(|| !device.is_playing(), "Song never stopped playing");

        // Player should not have moved to the next song.
        assert_eq!(player.get_playlist().current().name(), "Song 5");

        assert!(midi_device.get_emitted_event().is_none());

        Ok(())
    }
}
