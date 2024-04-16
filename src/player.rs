// Copyright (C) 2024 Michael Wilson <mike@mdwn.dev>
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
    collections::HashMap,
    error::Error,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Barrier,
    },
    thread,
};

use tokio::{
    sync::{oneshot, Mutex},
    task::JoinHandle,
};
use tracing::{error, info, span, Level, Span};

use crate::{audio, midi, playlist::Playlist, playsync::CancelHandle, songs::Song};

struct PlayHandles {
    join: JoinHandle<()>,
    cancel: CancelHandle,
}

/// Plays back individual wav files as multichannel audio for the configured audio interface.
pub struct Player {
    /// The device to play audio through.
    device: Arc<dyn audio::Device>,
    /// Mappings of tracks to output channels.
    mappings: Arc<HashMap<String, u16>>,
    /// The MIDI device to play MIDI back through.
    midi_device: Option<Arc<dyn midi::Device>>,
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
        device: Arc<dyn audio::Device>,
        mappings: HashMap<String, u16>,
        midi_device: Option<Arc<dyn midi::Device>>,
        playlist: Arc<Playlist>,
        all_songs_playlist: Arc<Playlist>,
    ) -> Player {
        let player = Player {
            device,
            mappings: Arc::new(mappings),
            midi_device,
            playlist,
            all_songs: all_songs_playlist,
            use_all_songs: AtomicBool::new(false),
            join: Arc::new(Mutex::new(None)),
            stop_run: Arc::new(AtomicBool::new(false)),
            span: span!(Level::INFO, "player"),
        };

        // Emit the event for the first track if needed.
        Player::emit_midi_event(player.midi_device.clone(), player.get_playlist().current());

        player
    }

    /// Plays the song at the current position. Returns true if the song was submitted successfully.
    pub async fn play(&self) -> Result<(), Box<dyn Error>> {
        let _enter = self.span.enter();

        let mut join = self.join.lock().await;

        let playlist = self.get_playlist().clone();
        if join.is_some() {
            info!(
                current_song = playlist.current().name,
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
            let cancel_handle = cancel_handle.clone();
            let mappings = self.mappings.clone();
            tokio::task::spawn_blocking(move || {
                Player::play_files(device, mappings, midi_device, song, cancel_handle, play_tx);
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
                song = song.name,
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
        mappings: Arc<HashMap<String, u16>>,
        midi_device: Option<Arc<dyn midi::Device>>,
        song: Arc<Song>,
        cancel_handle: CancelHandle,
        play_tx: oneshot::Sender<()>,
    ) {
        let song = song.clone();
        let cancel_handle = cancel_handle.clone();
        let barrier = Arc::new(match midi_device {
            Some(_) => Barrier::new(2),
            None => Barrier::new(1),
        });

        let audio_join_handle = {
            let device = device.clone();
            let song = song.clone();
            let barrier = barrier.clone();
            let cancel_handle = cancel_handle.clone();

            thread::spawn(move || {
                let song_name = song.name.to_string();

                barrier.wait();
                if let Err(e) = device.play(song, &mappings, cancel_handle) {
                    error!(
                        err = e.as_ref(),
                        song = song_name,
                        "Error while playing song"
                    );
                }
            })
        };

        if let Some(midi_device) = midi_device {
            let midi_device = midi_device.clone();
            let song = song.clone();
            let barrier = barrier.clone();
            let cancel_handle = cancel_handle.clone();

            let midi_join_handle = thread::spawn(move || {
                let song_name = song.name.to_string();

                barrier.wait();
                if let Err(e) = midi_device.play(song, cancel_handle) {
                    error!(
                        err = e.as_ref(),
                        song = song_name,
                        "Error while playing song"
                    );
                }
            });

            if let Err(e) = midi_join_handle.join() {
                error!("Error waiting for MIDI to stop playing: {:?}", e)
            }
        }

        if let Err(e) = audio_join_handle.join() {
            error!("Error waiting for audio to stop playing: {:?}", e)
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
        if join.is_none() {
            return Ok(false);
        }
        info!(
            song = self.get_playlist().current().name,
            "Waiting for song to finish.",
        );

        // Wait for the mutex to become available and immediately drop it.
        (&mut join.as_mut().unwrap().join).await?;
        Ok(true)
    }

    /// Next goes to the next entry in the playlist.
    pub async fn next(&self) -> Result<Arc<Song>, Box<dyn Error>> {
        let join = self.join.lock().await;
        let playlist = self.get_playlist();
        if join.is_some() {
            let current = playlist.current();
            info!(
                current_song = current.name,
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
                current_song = current.name,
                "Can't go to previous, player is active."
            );
            return Ok(current);
        }
        Ok(Player::prev_and_emit(self.midi_device.clone(), playlist))
    }

    /// Stop will stop a song if a song is playing.
    pub async fn stop(&mut self) -> Result<(), Box<dyn Error>> {
        let mut join = self.join.lock().await;

        if join.is_none() {
            info!("Player is not active, nothing to stop.");
            return Ok(());
        }

        if self
            .stop_run
            .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
            .is_err()
        {
            info!("The previous stop is still processing.");
            return Ok(());
        }

        info!(
            song = self.get_playlist().current().name,
            "Stopping playback."
        );

        // Cancel the playback.
        let join = join.as_mut().unwrap();
        join.cancel.cancel();
        Ok(())
    }

    /// Switch to the all songs playlist.
    pub async fn switch_to_all_songs(&self) -> Result<(), Box<dyn Error>> {
        println!("Switching to all songs");
        let join = self.join.lock().await;
        if join.is_some() {
            info!(
                current_song = self.get_playlist().current().name,
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
                current_song = self.get_playlist().current().name,
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
            if let Err(e) = midi_device.emit(song.clone()) {
                error!("Error emitting MIDI event: {:?}", e);
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, error::Error, path::PathBuf, sync::Arc};

    use crate::{audio, config, midi, playlist::Playlist, test::eventually};

    use super::Player;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_player() -> Result<(), Box<dyn Error>> {
        let device = Arc::new(audio::test::Device::get("mock-device"));
        let mappings: HashMap<String, u16> = HashMap::new();
        let midi_device = Arc::new(midi::test::Device::get("mock-midi-device"));
        let songs = config::get_all_songs(&PathBuf::from("assets/songs"))?;
        let playlist =
            config::parse_playlist(&PathBuf::from("assets/playlist.yaml"), songs.clone())?;
        let all_songs_playlist = Playlist::from_songs(songs.clone())?;
        let mut player = Player::new(
            device.clone(),
            mappings,
            Some(midi_device.clone()),
            playlist.clone(),
            all_songs_playlist.clone(),
        );

        // Direct the player.
        println!("Playlist -> Song 1");
        assert_eq!(player.get_playlist().current().name, "Song 1");

        player.next().await?;
        println!("Playlist -> Song 3");
        assert_eq!(player.get_playlist().current().name, "Song 3");

        player.prev().await?;
        println!("Playlist -> Song 1");
        assert_eq!(player.get_playlist().current().name, "Song 1");

        println!("Switch to AllSongs");
        player.switch_to_all_songs().await?;
        assert_eq!(player.get_playlist().current().name, "Song 1");

        player.next().await?;
        println!("AllSongs -> Song 10");
        assert_eq!(player.get_playlist().current().name, "Song 10");

        // No emitted events yet
        assert!(midi_device.get_emitted_event().is_none());

        player.next().await?;
        println!("AllSongs -> Song 2");
        assert_eq!(player.get_playlist().current().name, "Song 2");

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
        assert_eq!(player.get_playlist().current().name, "Song 3");

        assert!(midi_device.get_emitted_event().is_none());

        player.switch_to_playlist().await?;
        println!("Switch to Playlist");
        assert_eq!(player.get_playlist().current().name, "Song 1");

        player.next().await?;
        println!("Playlist -> Song 3");
        assert_eq!(player.get_playlist().current().name, "Song 3");

        player.play().await?;

        // Playlist should have moved to next song.
        eventually(
            || player.get_playlist().current().name == "Song 5",
            format!(
                "Song never moved to next, on song {}",
                player.get_playlist().current().name
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
        assert_eq!(player.get_playlist().current().name, "Song 5");

        assert!(midi_device.get_emitted_event().is_none());

        Ok(())
    }
}
