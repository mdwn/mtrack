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
use std::{io, sync::Arc};

use tokio::{sync::mpsc::Sender, task::JoinHandle};

use super::Event;
use crate::controller;

pub enum SubDriver {
    Keyboard(Arc<controller::keyboard::Driver>),
    Midi(Arc<controller::midi::Driver>),
}

/// A controller that controls a player using multiple other drivers.
pub struct Driver {
    /// The device that the driver will monitor.
    sub_drivers: Vec<SubDriver>,
}
impl Driver {
    pub fn new(sub_drivers: Vec<SubDriver>) -> Driver {
        Driver { sub_drivers }
    }
}

impl super::Driver for Driver {
    fn monitor_events(&self, events_tx: Sender<Event>) -> JoinHandle<Result<(), io::Error>> {
        let join_handles = self
            .sub_drivers
            .iter()
            .map(|driver| match driver {
                SubDriver::Midi(arc) => arc.as_ref().monitor_events(events_tx.clone()),
                SubDriver::Keyboard(arc) => arc.as_ref().monitor_events(events_tx.clone()),
            })
            .collect::<Vec<_>>();

        tokio::spawn(async move {
            let results = futures::future::join_all(join_handles).await;
            if results.iter().all(|result| result.is_ok()) {
                Ok(())
            } else {
                Err(io::Error::last_os_error())
            }
        })
    }
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, error::Error, path::Path, sync::Arc};

    use crate::{
        config,
        controller::{self, multi::SubDriver, Controller},
        player::Player,
        playlist::Playlist,
        songs,
        testutil::eventually,
    };

    #[tokio::test(flavor = "multi_thread")]
    async fn test_multi_controller() -> Result<(), Box<dyn Error>> {
        // Set up all of the MIDI events and the MIDI controller driver.
        let subscriber = tracing_subscriber::fmt()
            // ... add configuration
            .finish();
        let _default_guard = tracing::subscriber::set_default(subscriber);
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

        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let player = Player::new(
            songs.clone(),
            Playlist::new(
                &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
                songs,
            )?,
            &config::Player::new(
                config::Controller::Keyboard,
                config::Audio::new("mock-device"),
                Some(config::Midi::new("mock-midi-device", None)),
                None,
                HashMap::new(),
                "assets/songs",
            ),
        )?;
        let playlist = player.get_playlist();
        let all_songs_playlist = player.get_all_songs_playlist();
        let binding = player.audio_device();
        let device = binding.to_mock()?;
        let binding = player.midi_device().expect("MIDI device not found");
        let midi_device = binding.to_mock()?;

        let driver = Arc::new(super::Driver::new(vec![SubDriver::Midi(Arc::new(
            controller::midi::Driver::new(
                midi_device.clone(),
                play_event,
                prev_event,
                next_event,
                stop_event,
                all_songs_event,
                playlist_event,
            ),
        ))]));

        let _controller = Controller::new_from_driver(player, driver);

        println!("Playlist: {}", playlist);
        println!("AllSongs: {}", all_songs_playlist);

        // Test the controller directing the player.
        println!("Playlist -> Song 1");
        eventually(
            || playlist.current().name() == "Song 1",
            "Playlist never became Song 1",
        );
        midi_device.mock_event(&next_buf);
        println!("Playlist -> Song 3");
        eventually(
            || playlist.current().name() == "Song 3",
            "Playlist never became Song 3",
        );
        midi_device.mock_event(&next_buf);
        println!("Playlist -> Song 5");
        eventually(
            || playlist.current().name() == "Song 5",
            "Playlist never became Song 5",
        );
        midi_device.mock_event(&next_buf);
        println!("Playlist -> Song 7");
        eventually(
            || playlist.current().name() == "Song 7",
            "Playlist never became Song 7",
        );
        midi_device.mock_event(&prev_buf);
        println!("Playlist -> Song 5");
        eventually(
            || playlist.current().name() == "Song 5",
            "Playlist never became Song 5",
        );
        println!("Switch to AllSongs");
        midi_device.mock_event(&all_songs_buf);
        eventually(
            || all_songs_playlist.current().name() == "Song 1",
            "All Songs Playlist never became Song 1",
        );
        println!("AllSongs -> Song 10");
        midi_device.mock_event(&next_buf);
        eventually(
            || all_songs_playlist.current().name() == "Song 10",
            "All Songs Playlist never became Song 10",
        );
        println!("AllSongs -> Song 2");
        midi_device.mock_event(&next_buf);
        eventually(
            || all_songs_playlist.current().name() == "Song 2",
            "All Songs Playlist never became Song 2",
        );
        println!("AllSongs -> Song 10");
        midi_device.mock_event(&prev_buf);
        eventually(
            || all_songs_playlist.current().name() == "Song 10",
            "All Songs Playlist never became Song 10",
        );
        println!("Switch to Playlist");
        midi_device.mock_event(&playlist_buf);
        eventually(
            || playlist.current().name() == "Song 5",
            "Playlist never became Song 5",
        );
        println!("Playlist -> Song 7");
        midi_device.mock_event(&next_buf);
        eventually(
            || playlist.current().name() == "Song 7",
            "Playlist never became Song 7",
        );
        midi_device.mock_event(&play_buf);
        eventually(|| device.is_playing(), "Song never started playing");
        midi_device.mock_event(&stop_buf);
        eventually(|| !device.is_playing(), "Song never stopped playing");

        Ok(())
    }
}
