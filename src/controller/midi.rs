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
use std::{error::Error, io, sync::Arc};

use midly::live::LiveEvent;
use tokio::{sync::mpsc, task::JoinHandle};
use tracing::{error, info, span, Level};

use crate::{config, midi, midi::Device, player::Player};

/// Recognized MIDI controller actions.
#[derive(Debug, PartialEq)]
enum MidiAction {
    Play,
    Prev,
    Next,
    Stop,
    AllSongs,
    Playlist,
    SectionAck,
    StopSectionLoop,
    Unrecognized,
}

/// MIDI events that the controller recognizes.
struct MidiEvents {
    play: LiveEvent<'static>,
    prev: LiveEvent<'static>,
    next: LiveEvent<'static>,
    stop: LiveEvent<'static>,
    all_songs: LiveEvent<'static>,
    playlist: LiveEvent<'static>,
    section_ack: Option<LiveEvent<'static>>,
    stop_section_loop: Option<LiveEvent<'static>>,
}

/// Classifies a parsed MIDI event against the known controller events.
fn classify_midi_event(events: &MidiEvents, event: &LiveEvent<'_>) -> MidiAction {
    if *event == events.play {
        MidiAction::Play
    } else if *event == events.prev {
        MidiAction::Prev
    } else if *event == events.next {
        MidiAction::Next
    } else if *event == events.stop {
        MidiAction::Stop
    } else if *event == events.all_songs {
        MidiAction::AllSongs
    } else if *event == events.playlist {
        MidiAction::Playlist
    } else if events.section_ack.as_ref() == Some(event) {
        MidiAction::SectionAck
    } else if events.stop_section_loop.as_ref() == Some(event) {
        MidiAction::StopSectionLoop
    } else {
        MidiAction::Unrecognized
    }
}

/// A controller that controls a player using MIDI.
pub struct Driver {
    /// The player.
    player: Arc<Player>,
    /// The MIDI device.
    midi_device: Arc<dyn Device>,
    /// The recognized MIDI events.
    events: MidiEvents,
}

impl Driver {
    pub fn new(
        config: config::MidiController,
        player: Arc<Player>,
    ) -> Result<Arc<Self>, Box<dyn Error>> {
        match player.midi_device() {
            Some(midi_device) => {
                if let Some(ms) = config.morningstar() {
                    info!("Registering Morningstar song change notifier");
                    player.add_song_change_notifier(Arc::new(midi::morningstar::Notifier::new(
                        ms.clone(),
                        midi_device.clone(),
                    )));
                }
                Ok(Arc::new(Driver {
                    player,
                    midi_device,
                    events: MidiEvents {
                        play: config.play()?,
                        prev: config.prev()?,
                        next: config.next()?,
                        stop: config.stop()?,
                        all_songs: config.all_songs()?,
                        playlist: config.playlist()?,
                        section_ack: config.section_ack()?,
                        stop_section_loop: config.stop_section_loop()?,
                    },
                }))
            }
            None => Err("No MIDI device to use for MIDI configuration".into()),
        }
    }
}

impl super::Driver for Driver {
    fn monitor_events(&self) -> JoinHandle<Result<(), io::Error>> {
        let (midi_events_tx, mut midi_events_rx) = mpsc::channel::<Vec<u8>>(10);
        let player = self.player.clone();
        let device = self.midi_device.clone();
        let events = MidiEvents {
            play: self.events.play,
            prev: self.events.prev,
            next: self.events.next,
            stop: self.events.stop,
            all_songs: self.events.all_songs,
            playlist: self.events.playlist,
            section_ack: self.events.section_ack,
            stop_section_loop: self.events.stop_section_loop,
        };

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

                // Check for Morningstar SysEx ack responses.
                if midi::morningstar::check_ack(&raw_event) {
                    continue;
                }

                // Process triggered samples (synchronous, minimal latency)
                player.process_sample_trigger(&raw_event);

                let event = match LiveEvent::parse(&raw_event) {
                    Ok(event) => event,
                    Err(e) => {
                        error!(err = format!("{:?}", e), "Error parsing event.");
                        continue;
                    }
                };

                match classify_midi_event(&events, &event) {
                    MidiAction::Play => {
                        if let Err(e) = player.play().await {
                            error!(err = e.as_ref(), "Failed to play song: {}", e);
                        }
                    }
                    MidiAction::Prev => {
                        player.prev().await;
                    }
                    MidiAction::Next => {
                        player.next().await;
                    }
                    MidiAction::Stop => {
                        player.stop().await;
                    }
                    MidiAction::AllSongs => {
                        if let Err(e) = player.switch_to_playlist("all_songs").await {
                            error!("Failed to switch to all_songs: {}", e);
                        }
                    }
                    MidiAction::Playlist => {
                        let name = player.persisted_playlist_name();
                        if let Err(e) = player.switch_to_playlist(&name).await {
                            error!("Failed to switch to playlist {}: {}", name, e);
                        }
                    }
                    MidiAction::SectionAck => {
                        if let Err(e) = player.section_ack().await {
                            error!("Failed to ack section: {}", e);
                        }
                    }
                    MidiAction::StopSectionLoop => {
                        player.stop_section_loop();
                    }
                    MidiAction::Unrecognized => {}
                }
            }
        })
    }
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, error::Error, path::Path};

    use crate::{
        config::{self, midi::ToMidiEvent, MidiController},
        controller::Controller,
        midi::Device,
        player::Player,
        playlist,
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
        let pl = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let mut playlists = HashMap::new();
        playlists.insert(
            "all_songs".to_string(),
            playlist::from_songs(songs.clone())?,
        );
        playlists.insert("playlist".to_string(), pl);
        let player = Player::new(
            playlists,
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
        let playlist = player.get_playlist();
        let binding = player
            .audio_device()
            .expect("audio device should be present");
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
            player.clone(),
        )?;

        let _controller = Controller::new_from_drivers(vec![driver]);

        println!("Playlist: {}", playlist);

        // Test the controller directing the player. Make sure we put
        // unrecognized events in between to make sure that they're ignored.
        println!("Playlist -> Song 1");
        eventually(
            || playlist.current().unwrap().name() == "Song 1",
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
            || playlist.current().unwrap().name() == "Song 3",
            "Playlist never became Song 3",
        );

        // Add delay between transitions
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&next_buf);
        println!("Playlist -> Song 5");
        eventually(
            || playlist.current().unwrap().name() == "Song 5",
            "Playlist never became Song 5",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&next_buf);
        println!("Playlist -> Song 7");
        eventually(
            || playlist.current().unwrap().name() == "Song 7",
            "Playlist never became Song 7",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&prev_buf);
        println!("Playlist -> Song 5");
        eventually(
            || playlist.current().unwrap().name() == "Song 5",
            "Playlist never became Song 5",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        println!("Switch to AllSongs");
        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&all_songs_buf);
        eventually(
            || player.get_playlist().current().unwrap().name() == "Song 1",
            "All Songs Playlist never became Song 1",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        println!("AllSongs -> Song 10");
        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&next_buf);
        eventually(
            || player.get_playlist().current().unwrap().name() == "Song 10",
            "All Songs Playlist never became Song 10",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        println!("AllSongs -> Song 2");
        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&next_buf);
        eventually(
            || player.get_playlist().current().unwrap().name() == "Song 2",
            "All Songs Playlist never became Song 2",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        println!("AllSongs -> Song 10");
        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&prev_buf);
        eventually(
            || player.get_playlist().current().unwrap().name() == "Song 10",
            "All Songs Playlist never became Song 10",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        println!("Switch to Playlist");
        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&playlist_buf);
        eventually(
            || playlist.current().unwrap().name() == "Song 5",
            "Playlist never became Song 5",
        );

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        println!("Playlist -> Song 7");
        midi_device.mock_event(&unrecognized_buf);
        midi_device.mock_event(&next_buf);
        eventually(
            || playlist.current().unwrap().name() == "Song 7",
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

    mod classify_midi_event_tests {
        use super::super::{classify_midi_event, MidiAction, MidiEvents};
        use crate::config::midi::{note_on, ToMidiEvent};
        use midly::live::LiveEvent;

        fn make_test_events() -> MidiEvents {
            MidiEvents {
                play: note_on(16, 0, 127).to_midi_event().unwrap(),
                prev: note_on(16, 1, 127).to_midi_event().unwrap(),
                next: note_on(16, 2, 127).to_midi_event().unwrap(),
                stop: note_on(16, 3, 127).to_midi_event().unwrap(),
                all_songs: note_on(16, 4, 127).to_midi_event().unwrap(),
                playlist: note_on(16, 5, 127).to_midi_event().unwrap(),
                section_ack: Some(note_on(16, 6, 127).to_midi_event().unwrap()),
                stop_section_loop: Some(note_on(16, 7, 127).to_midi_event().unwrap()),
            }
        }

        #[test]
        fn recognizes_play() {
            let events = make_test_events();
            let event = note_on(16, 0, 127).to_midi_event().unwrap();
            assert_eq!(classify_midi_event(&events, &event), MidiAction::Play);
        }

        #[test]
        fn recognizes_prev() {
            let events = make_test_events();
            let event = note_on(16, 1, 127).to_midi_event().unwrap();
            assert_eq!(classify_midi_event(&events, &event), MidiAction::Prev);
        }

        #[test]
        fn recognizes_next() {
            let events = make_test_events();
            let event = note_on(16, 2, 127).to_midi_event().unwrap();
            assert_eq!(classify_midi_event(&events, &event), MidiAction::Next);
        }

        #[test]
        fn recognizes_stop() {
            let events = make_test_events();
            let event = note_on(16, 3, 127).to_midi_event().unwrap();
            assert_eq!(classify_midi_event(&events, &event), MidiAction::Stop);
        }

        #[test]
        fn recognizes_all_songs() {
            let events = make_test_events();
            let event = note_on(16, 4, 127).to_midi_event().unwrap();
            assert_eq!(classify_midi_event(&events, &event), MidiAction::AllSongs);
        }

        #[test]
        fn recognizes_playlist() {
            let events = make_test_events();
            let event = note_on(16, 5, 127).to_midi_event().unwrap();
            assert_eq!(classify_midi_event(&events, &event), MidiAction::Playlist);
        }

        #[test]
        fn unrecognized_note() {
            let events = make_test_events();
            let event = note_on(16, 99, 127).to_midi_event().unwrap();
            assert_eq!(
                classify_midi_event(&events, &event),
                MidiAction::Unrecognized
            );
        }

        #[test]
        fn wrong_channel_is_unrecognized() {
            let events = make_test_events();
            // Play is on channel 16, note 0 — try channel 1
            let event = note_on(1, 0, 127).to_midi_event().unwrap();
            assert_eq!(
                classify_midi_event(&events, &event),
                MidiAction::Unrecognized
            );
        }

        #[test]
        fn different_velocity_still_matches() {
            let events = make_test_events();
            // Play is note_on(16, 0, 127) — try with velocity 64
            let event = note_on(16, 0, 64).to_midi_event().unwrap();
            // LiveEvent NoteOn equality checks velocity too, so different velocity won't match
            assert_eq!(
                classify_midi_event(&events, &event),
                MidiAction::Unrecognized
            );
        }

        #[test]
        fn program_change_is_unrecognized() {
            let events = make_test_events();
            let event = LiveEvent::Midi {
                channel: 15.into(),
                message: midly::MidiMessage::ProgramChange { program: 27.into() },
            };
            assert_eq!(
                classify_midi_event(&events, &event),
                MidiAction::Unrecognized
            );
        }

        #[test]
        fn note_off_is_unrecognized() {
            let events = make_test_events();
            // Play is NoteOn ch16 note0 — NoteOff same params should not match
            let event = LiveEvent::Midi {
                channel: 15.into(), // midly channel 15 = MIDI channel 16
                message: midly::MidiMessage::NoteOff {
                    key: 0.into(),
                    vel: 127.into(),
                },
            };
            assert_eq!(
                classify_midi_event(&events, &event),
                MidiAction::Unrecognized
            );
        }
    }
}
