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
use std::{io, sync::Arc};

use midly::live::LiveEvent;
use tokio::{
    sync::mpsc::{self, Sender},
    task::JoinHandle,
};
use tracing::{error, info, span, Level};

use crate::midi;

use super::Event;

/// A controller that controls a player using MIDI.
pub struct Driver {
    /// The device that the driver will monitor.
    device: Arc<dyn midi::Device>,
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
        device: Arc<dyn midi::Device>,
        play: LiveEvent<'static>,
        prev: LiveEvent<'static>,
        next: LiveEvent<'static>,
        stop: LiveEvent<'static>,
        all_songs: LiveEvent<'static>,
        playlist: LiveEvent<'static>,
    ) -> Driver {
        Driver {
            device,
            play,
            prev,
            next,
            stop,
            all_songs,
            playlist,
        }
    }
}

impl super::Driver for Driver {
    fn monitor_events(&self, events_tx: Sender<Event>) -> JoinHandle<Result<(), io::Error>> {
        let (midi_events_tx, mut midi_events_rx) = mpsc::channel::<Vec<u8>>(10);
        let device = self.device.clone();
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

            device
                .watch_events(midi_events_tx)
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;

            loop {
                let raw_event = match midi_events_rx.blocking_recv() {
                    Some(raw_event) => raw_event,
                    None => {
                        info!("MIDI watcher closed.");
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
                    events_tx.blocking_send(Event::Play)
                } else if event == prev {
                    events_tx.blocking_send(Event::Prev)
                } else if event == next {
                    events_tx.blocking_send(Event::Next)
                } else if event == stop {
                    events_tx.blocking_send(Event::Stop)
                } else if event == all_songs {
                    events_tx.blocking_send(Event::AllSongs)
                } else if event == playlist {
                    events_tx.blocking_send(Event::Playlist)
                } else {
                    Ok(())
                }
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            }
        })
    }
}

impl Drop for Driver {
    fn drop(&mut self) {
        self.device.stop_watch_events();
    }
}

#[cfg(test)]
mod test {
    use std::{error::Error, path::PathBuf, sync::Arc};

    use crate::{
        audio, config, controller::Controller, midi, player::Player, playlist::Playlist,
        test::test::eventually,
    };

    #[tokio::test(flavor = "multi_thread")]
    async fn test_midi_controller() -> Result<(), Box<dyn Error>> {
        // Set up all of the MIDI events and the MIDI controller driver.
        let midi_device = Arc::new(midi::test::Device::get("mock-midi-device"));
        let play_event = midly::live::LiveEvent::Midi {
            channel: 16.into(),
            message: midly::MidiMessage::NoteOn {
                key: 0.into(),
                vel: 127.into(),
            },
        };
        let prev_event = midly::live::LiveEvent::Midi {
            channel: 16.into(),
            message: midly::MidiMessage::NoteOn {
                key: 1.into(),
                vel: 127.into(),
            },
        };
        let next_event = midly::live::LiveEvent::Midi {
            channel: 16.into(),
            message: midly::MidiMessage::NoteOn {
                key: 2.into(),
                vel: 127.into(),
            },
        };
        let stop_event = midly::live::LiveEvent::Midi {
            channel: 16.into(),
            message: midly::MidiMessage::NoteOn {
                key: 3.into(),
                vel: 127.into(),
            },
        };
        let all_songs_event = midly::live::LiveEvent::Midi {
            channel: 16.into(),
            message: midly::MidiMessage::NoteOn {
                key: 4.into(),
                vel: 127.into(),
            },
        };
        let playlist_event = midly::live::LiveEvent::Midi {
            channel: 16.into(),
            message: midly::MidiMessage::NoteOn {
                key: 5.into(),
                vel: 127.into(),
            },
        };
        let driver = Arc::new(super::Driver::new(
            midi_device.clone(),
            play_event,
            prev_event,
            next_event,
            stop_event,
            all_songs_event,
            playlist_event,
        ));

        let mut play_buf: Vec<u8> = Vec::with_capacity(8);
        let mut prev_buf: Vec<u8> = Vec::with_capacity(8);
        let mut next_buf: Vec<u8> = Vec::with_capacity(8);
        let mut stop_buf: Vec<u8> = Vec::with_capacity(8);
        let mut all_songs_buf: Vec<u8> = Vec::with_capacity(8);
        let mut playlist_buf: Vec<u8> = Vec::with_capacity(8);

        play_event.write(&mut play_buf)?;
        prev_event.write(&mut prev_buf)?;
        next_event.write(&mut next_buf)?;
        stop_event.write(&mut stop_buf)?;
        all_songs_event.write(&mut all_songs_buf)?;
        playlist_event.write(&mut playlist_buf)?;

        let device = Arc::new(audio::test::Device::get("mock-device"));
        let songs = config::get_all_songs(&PathBuf::from("assets/songs"))?;
        let playlist =
            config::parse_playlist(&PathBuf::from("assets/playlist.yaml"), songs.clone())?;
        let all_songs_playlist = Playlist::from_songs(songs.clone())?;
        let player = Player::new(
            device.clone(),
            None,
            playlist.clone(),
            all_songs_playlist.clone(),
        );
        let _controller = Controller::new(player, driver)?;

        println!("Playlist: {}", playlist);
        println!("AllSongs: {}", all_songs_playlist);

        // Test the controller directing the player.
        println!("Playlist -> Song 1");
        eventually(
            || playlist.current().name == "Song 1",
            "Playlist never became Song 1",
        );
        midi_device.mock_event(&next_buf);
        println!("Playlist -> Song 3");
        eventually(
            || playlist.current().name == "Song 3",
            "Playlist never became Song 3",
        );
        midi_device.mock_event(&next_buf);
        println!("Playlist -> Song 5");
        eventually(
            || playlist.current().name == "Song 5",
            "Playlist never became Song 5",
        );
        midi_device.mock_event(&next_buf);
        println!("Playlist -> Song 7");
        eventually(
            || playlist.current().name == "Song 7",
            "Playlist never became Song 7",
        );
        midi_device.mock_event(&prev_buf);
        println!("Playlist -> Song 5");
        eventually(
            || playlist.current().name == "Song 5",
            "Playlist never became Song 5",
        );
        println!("Switch to AllSongs");
        midi_device.mock_event(&all_songs_buf);
        eventually(
            || all_songs_playlist.current().name == "Song 1",
            "All Songs Playlist never became Song 1",
        );
        println!("AllSongs -> Song 10");
        midi_device.mock_event(&next_buf);
        eventually(
            || all_songs_playlist.current().name == "Song 10",
            "All Songs Playlist never became Song 10",
        );
        println!("AllSongs -> Song 2");
        midi_device.mock_event(&next_buf);
        eventually(
            || all_songs_playlist.current().name == "Song 2",
            "All Songs Playlist never became Song 2",
        );
        println!("AllSongs -> Song 10");
        midi_device.mock_event(&prev_buf);
        eventually(
            || all_songs_playlist.current().name == "Song 10",
            "All Songs Playlist never became Song 10",
        );
        println!("Switch to Playlist");
        midi_device.mock_event(&playlist_buf);
        eventually(
            || playlist.current().name == "Song 5",
            "Playlist never became Song 5",
        );
        println!("Playlist -> Song 7");
        midi_device.mock_event(&next_buf);
        eventually(
            || playlist.current().name == "Song 7",
            "Playlist never became Song 7",
        );
        midi_device.mock_event(&play_buf);
        eventually(|| device.is_playing(), "Song never started playing");
        midi_device.mock_event(&stop_buf);
        eventually(|| !device.is_playing(), "Song never stopped playing");

        Ok(())
    }
}
