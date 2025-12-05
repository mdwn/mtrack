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
use std::{error::Error, io, sync::Arc};

use midly::live::LiveEvent;
use tokio::{sync::mpsc, task::JoinHandle};
use tracing::{error, info, span, Level};

use crate::{config, midi::Device, player::Player};

/// A controller that controls a player using MIDI.
pub struct Driver {
    /// The player.
    player: Arc<Player>,
    /// The MIDI device.
    midi_device: Arc<dyn Device>,
    /// The MIDI event to look for to play the current song in the playlist.
    play: LiveEvent<'static>,
    /// The MIDI event to look for to move the playlist to the previous item.
    prev: LiveEvent<'static>,
    /// The MIDI event to look for to move the playlist to the next item.
    next: LiveEvent<'static>,
    /// The MIDI event to look for to stop playback.
    stop: LiveEvent<'static>,
    /// The MIDI event to look for to switch from the current playlist to an all songs playlist.
    all_songs: LiveEvent<'static>,
    /// The MIDI event to look for to switch back to the current playlist.
    playlist: LiveEvent<'static>,
}
impl Driver {
    pub fn new(
        config: config::MidiController,
        player: Arc<Player>,
    ) -> Result<Arc<Self>, Box<dyn Error>> {
        match player.midi_device() {
            Some(midi_device) => Ok(Arc::new(Driver {
                player,
                midi_device,
                play: config.play()?,
                prev: config.prev()?,
                next: config.next()?,
                stop: config.stop()?,
                all_songs: config.all_songs()?,
                playlist: config.playlist()?,
            })),
            None => Err("No MIDI device to use for MIDI configuration".into()),
        }
    }
}

impl super::Driver for Driver {
    fn monitor_events(&self) -> JoinHandle<Result<(), io::Error>> {
        let (midi_events_tx, mut midi_events_rx) = mpsc::channel::<Vec<u8>>(10);
        let player = self.player.clone();
        let device = self.midi_device.clone();
        let play = self.play;
        let prev = self.prev;
        let next = self.next;
        let stop = self.stop;
        let all_songs = self.all_songs;
        let playlist = self.playlist;

        tokio::task::spawn_blocking(move || {
            let span = span!(Level::INFO, "MIDI driver");
            let _enter = span.enter();

            info!("MIDI driver started.");

            if let Err(e) = device
                .watch_events(midi_events_tx)
                .map_err(|e| io::Error::other(e.to_string()))
            {
                error!(err = e.to_string(), "Error watching MIDI events");
            }
        });

        let device = self.midi_device.clone();
        tokio::spawn(async move {
            loop {
                let raw_event = match midi_events_rx.recv().await {
                    Some(raw_event) => raw_event,
                    None => {
                        info!("MIDI watcher closed.");
                        device.stop_watch_events();
                        return Ok(());
                    }
                };

                let event = match LiveEvent::parse(&raw_event) {
                    Ok(event) => event,
                    Err(e) => {
                        error!(err = format!("{:?}", e), "Error parsing event.");
                        continue;
                    }
                };

                if event == play {
                    if let Err(e) = player.play().await {
                        error!(err = e.as_ref(), "Failed to play song: {}", e);
                    }
                } else if event == prev {
                    player.prev().await;
                } else if event == next {
                    player.next().await;
                } else if event == stop {
                    player.stop().await;
                } else if event == all_songs {
                    player.switch_to_all_songs().await;
                } else if event == playlist {
                    player.switch_to_playlist().await;
                }
            }
        })
    }
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, error::Error, path::Path, sync::Arc};

    use crate::{
        config::{self, midi::ToMidiEvent, MidiController},
        controller::Controller,
        midi::Device,
        player::Player,
        playlist::Playlist,
        songs,
        testutil::eventually,
    };

    #[tokio::test(flavor = "multi_thread")]
    async fn test_midi_controller() -> Result<(), Box<dyn Error>> {
        // Set up all of the MIDI events and the MIDI controller driver.
        let play_event = config::midi::note_on(16, 0, 127);
        let prev_event = config::midi::note_on(16, 1, 127);
        let next_event = config::midi::note_on(16, 2, 127);
        let stop_event = config::midi::note_on(16, 3, 127);
        let all_songs_event = config::midi::note_on(16, 4, 127);
        let playlist_event = config::midi::note_on(16, 5, 127);

        let unrecognized_event = midly::live::LiveEvent::Midi {
            channel: 15.into(),
            message: midly::MidiMessage::ProgramChange { program: 27.into() },
        };

        let mut play_buf: Vec<u8> = Vec::with_capacity(8);
        let mut prev_buf: Vec<u8> = Vec::with_capacity(8);
        let mut next_buf: Vec<u8> = Vec::with_capacity(8);
        let mut stop_buf: Vec<u8> = Vec::with_capacity(8);
        let mut all_songs_buf: Vec<u8> = Vec::with_capacity(8);
        let mut playlist_buf: Vec<u8> = Vec::with_capacity(8);
        let mut unrecognized_buf: Vec<u8> = Vec::with_capacity(8);
        let invalid_buf: Vec<u8> = vec![1, 2, 3, 4, 5, 6, 7, 8];

        play_event.to_midi_event()?.write(&mut play_buf)?;
        prev_event.to_midi_event()?.write(&mut prev_buf)?;
        next_event.to_midi_event()?.write(&mut next_buf)?;
        stop_event.to_midi_event()?.write(&mut stop_buf)?;
        all_songs_event.to_midi_event()?.write(&mut all_songs_buf)?;
        playlist_event.to_midi_event()?.write(&mut playlist_buf)?;
        unrecognized_event.write(&mut unrecognized_buf)?;

        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let player = Arc::new(Player::new(
            songs.clone(),
            Playlist::new(
                &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
                songs,
            )?,
            &config::Player::new(
                vec![config::Controller::Keyboard],
                config::Audio::new("mock-device"),
                Some(config::Midi::new("mock-midi-device", None)),
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?);
        let playlist = player.get_playlist();
        let all_songs_playlist = player.get_all_songs_playlist();
        let binding = player.audio_device();
        let device = binding.to_mock()?;
        let binding = player.midi_device().expect("MIDI device not found");
        let midi_device = binding.to_mock()?;

        let driver = super::Driver::new(
            MidiController::new(
                play_event,
                prev_event,
                next_event,
                stop_event,
                all_songs_event,
                playlist_event,
            ),
            player,
        )?;

        let _controller = Controller::new_from_drivers(vec![driver]);

        println!("Playlist: {}", playlist);
        println!("AllSongs: {}", all_songs_playlist);

        // Test the controller directing the player. Make sure we put
        // unrecognized events in between to make sure that they're ignored.
        println!("Playlist -> Song 1");
        eventually(
            || playlist.current().name() == "Song 1",
            "Playlist never became Song 1",
        );

        // Add small delay to ensure state is stable before next event
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // This invalid event should have no impact.
        midi_device.mock_event(&invalid_buf);
        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&next_buf);

        println!("Playlist -> Song 3");
        eventually(
            || playlist.current().name() == "Song 3",
            "Playlist never became Song 3",
        );

        // Add delay between transitions
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&next_buf);
        println!("Playlist -> Song 5");
        eventually(
            || playlist.current().name() == "Song 5",
            "Playlist never became Song 5",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&next_buf);
        println!("Playlist -> Song 7");
        eventually(
            || playlist.current().name() == "Song 7",
            "Playlist never became Song 7",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&prev_buf);
        println!("Playlist -> Song 5");
        eventually(
            || playlist.current().name() == "Song 5",
            "Playlist never became Song 5",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        println!("Switch to AllSongs");
        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&all_songs_buf);
        eventually(
            || all_songs_playlist.current().name() == "Song 1",
            "All Songs Playlist never became Song 1",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        println!("AllSongs -> Song 10");
        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&next_buf);
        eventually(
            || all_songs_playlist.current().name() == "Song 10",
            "All Songs Playlist never became Song 10",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        println!("AllSongs -> Song 2");
        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&next_buf);
        eventually(
            || all_songs_playlist.current().name() == "Song 2",
            "All Songs Playlist never became Song 2",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        println!("AllSongs -> Song 10");
        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&prev_buf);
        eventually(
            || all_songs_playlist.current().name() == "Song 10",
            "All Songs Playlist never became Song 10",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        println!("Switch to Playlist");
        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&playlist_buf);
        eventually(
            || playlist.current().name() == "Song 5",
            "Playlist never became Song 5",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        println!("Playlist -> Song 7");
        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&next_buf);
        eventually(
            || playlist.current().name() == "Song 7",
            "Playlist never became Song 7",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&play_buf);
        eventually(|| device.is_playing(), "Song never started playing");

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&stop_buf);
        eventually(|| !device.is_playing(), "Song never stopped playing");

        midi_device.stop_watch_events();

        Ok(())
    }
}
