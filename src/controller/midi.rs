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
use tracing::{error, info, span, warn, Level};

use super::{TransportAction, TransportBindings};
use crate::{config, midi, midi::Device, player::Player};

/// The MIDI event configured for each transport action.
type MidiEvents = TransportBindings<LiveEvent<'static>>;

/// The MIDI event `config` binds to `action`, `None` for none.
fn midi_event_for(
    config: &config::MidiController,
    action: TransportAction,
) -> Result<Option<LiveEvent<'static>>, Box<dyn Error>> {
    use crate::dmx::engine::AmbientMode;
    Ok(match action {
        TransportAction::Play => Some(config.play()?),
        TransportAction::Prev => Some(config.prev()?),
        TransportAction::Next => Some(config.next()?),
        TransportAction::Stop => Some(config.stop()?),
        TransportAction::AllSongs => Some(config.all_songs()?),
        TransportAction::Playlist => Some(config.playlist()?),
        TransportAction::SectionAck => config.section_ack()?,
        TransportAction::StopSectionLoop => config.stop_section_loop()?,
        TransportAction::AmbientMode(AmbientMode::On) => config.ambient_on()?,
        TransportAction::AmbientMode(AmbientMode::Off) => config.ambient_off()?,
        TransportAction::AmbientMode(AmbientMode::Auto) => config.ambient_auto()?,
        TransportAction::CancelAutoPlay => config.cancel_auto_play()?,
    })
}

/// The transport action a MIDI event is configured as, `None` for one
/// that is not.
fn classify_midi_event(events: &MidiEvents, event: &LiveEvent<'_>) -> Option<TransportAction> {
    events.classify(|bound| bound == event)
}

/// Stops a device's event watch when dropped.
struct StopWatchOnDrop(Arc<dyn Device>);

impl Drop for StopWatchOnDrop {
    fn drop(&mut self) {
        self.0.stop_watch_events();
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
                    events: MidiEvents::new(|action| midi_event_for(&config, action))?,
                }))
            }
            None => Err("No MIDI device to use for MIDI configuration".into()),
        }
    }
}

impl super::Driver for Driver {
    fn monitor_events(&self) -> JoinHandle<Result<(), io::Error>> {
        // Deep enough to absorb a footswitch burst or an unfiltered clock
        // stream while the consumer awaits player calls; the producer side
        // drops (never blocks) when this fills.
        let (midi_events_tx, mut midi_events_rx) = mpsc::channel::<Vec<u8>>(256);
        let player = self.player.clone();
        let device = self.midi_device.clone();
        let events = self.events.clone();

        // Watched before the task below exists, so that the task only ever
        // stops a watch that has started.
        {
            let span = span!(Level::INFO, "MIDI driver");
            let _enter = span.enter();

            info!("MIDI driver started.");

            if let Err(e) = device.watch_events(midi_events_tx) {
                error!(err = e.to_string(), "Error watching MIDI events");
                let e = io::Error::other(e.to_string());
                return tokio::spawn(async move { Err(e) });
            }
        }

        // Stops the watch however this task ends. A restart aborts it, and
        // the device's watcher ran on: nothing could watch that device again,
        // and a mock device's watcher thread kept the runtime from shutting
        // down. Made here, not on the task, as a task aborted before its first
        // poll never runs any of its body.
        let watch = StopWatchOnDrop(device);
        tokio::spawn(async move {
            let _watch = watch;
            loop {
                let raw_event = match midi_events_rx.recv().await {
                    Some(raw_event) => raw_event,
                    // Closed while the driver runs: the device let the watch
                    // go, a reopen that could not take it over, say. As an
                    // error, it does not read as running, and the next
                    // update starts it again.
                    None => {
                        warn!("MIDI watcher closed.");
                        return Err(io::Error::other("the MIDI device stopped watching events"));
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

                if let Some(action) = classify_midi_event(&events, &event) {
                    action.run(&player).await;
                }
            }
        })
    }
}

#[cfg(test)]
mod test {
    use std::error::Error;

    use crate::{
        config::{self, midi::ToMidiEvent, MidiController},
        controller::Controller,
        midi::Device,
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

        let player = crate::testutil::player::assets_player()
            .midi("mock-midi-device")
            .build()?;
        player.await_hardware_ready().await;
        let playlist = player.get_playlist();
        let binding = player
            .audio_device()
            .expect("audio device should be present");
        let device = binding.to_mock()?;
        let binding = player.midi_device().expect("MIDI device not found");
        let midi_device = binding.to_mock()?;

        let ambient_on_event = config::midi::note_on(16, 8, 127);
        let ambient_off_event = config::midi::note_on(16, 9, 127);
        let ambient_auto_event = config::midi::note_on(16, 10, 127);
        let mut ambient_on_buf: Vec<u8> = Vec::with_capacity(8);
        let mut ambient_off_buf: Vec<u8> = Vec::with_capacity(8);
        let mut ambient_auto_buf: Vec<u8> = Vec::with_capacity(8);
        ambient_on_event
            .to_midi_event()?
            .write(&mut ambient_on_buf)?;
        ambient_off_event
            .to_midi_event()?
            .write(&mut ambient_off_buf)?;
        ambient_auto_event
            .to_midi_event()?
            .write(&mut ambient_auto_buf)?;

        let driver = super::Driver::new(
            MidiController::new(
                play_event,
                prev_event,
                next_event,
                stop_event,
                all_songs_event,
                playlist_event,
            )
            .with_ambient(ambient_on_event, ambient_off_event, ambient_auto_event),
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

        // The ambient mode events.
        use crate::dmx::engine::AmbientMode;
        midi_device.mock_event(&ambient_on_buf);
        eventually(|| player.ambient_mode() == AmbientMode::On, "ambient on");
        midi_device.mock_event(&ambient_off_buf);
        eventually(|| player.ambient_mode() == AmbientMode::Off, "ambient off");
        midi_device.mock_event(&ambient_auto_buf);
        eventually(
            || player.ambient_mode() == AmbientMode::Auto,
            "ambient auto",
        );

        midi_device.stop_watch_events();

        Ok(())
    }

    /// A driver's task stops the device's watch when it ends, but one
    /// aborted before its first poll ran none of its body: the watch
    /// started for it ran on, and nothing could watch the device again.
    #[tokio::test(flavor = "current_thread")]
    async fn a_driver_aborted_before_it_runs_stops_its_watch() -> Result<(), Box<dyn Error>> {
        let player = crate::testutil::player::assets_player()
            .midi("mock-aborted-driver")
            .all_songs_only()
            .build()?;
        player.await_hardware_ready().await;
        let device = player.midi_device().expect("midi device");
        let driver = super::Driver::new(
            MidiController::new(
                config::midi::note_on(16, 0, 127),
                config::midi::note_on(16, 1, 127),
                config::midi::note_on(16, 2, 127),
                config::midi::note_on(16, 3, 127),
                config::midi::note_on(16, 4, 127),
                config::midi::note_on(16, 5, 127),
            ),
            player.clone(),
        )?;
        let handle = crate::controller::Driver::monitor_events(&*driver);
        // As a stop landing right after the start does: on this runtime the
        // task has not been polled yet.
        handle.abort();
        let _ = handle.await;
        let (tx, _rx) = tokio::sync::mpsc::channel(1);
        let rewatch = device.watch_events(tx);
        device.stop_watch_events();
        assert!(
            rewatch.is_ok(),
            "the aborted driver left the device watched: {:?}",
            rewatch.err().map(|e| e.to_string())
        );
        Ok(())
    }

    /// A device that lets the watch go while the driver runs, as a reopen
    /// that cannot take it over does, ended the driver's task as a clean
    /// stop, and the controller read as running while deaf. It ends as an
    /// error, which the next update retries.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_watch_the_device_lets_go_ends_the_driver_with_an_error() -> Result<(), Box<dyn Error>>
    {
        let player = crate::testutil::player::assets_player()
            .midi("mock-watch-let-go")
            .all_songs_only()
            .build()?;
        player.await_hardware_ready().await;
        let device = player.midi_device().expect("midi device");
        let driver = super::Driver::new(
            MidiController::new(
                config::midi::note_on(16, 0, 127),
                config::midi::note_on(16, 1, 127),
                config::midi::note_on(16, 2, 127),
                config::midi::note_on(16, 3, 127),
                config::midi::note_on(16, 4, 127),
                config::midi::note_on(16, 5, 127),
            ),
            player.clone(),
        )?;
        let handle = crate::controller::Driver::monitor_events(&*driver);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        device.stop_watch_events();
        let result = tokio::time::timeout(crate::testutil::PATIENCE, handle)
            .await
            .expect("the driver ran on with nothing watching")?;
        assert!(result.is_err(), "the driver ended as if stopped");
        Ok(())
    }

    mod classify_midi_event_tests {
        use super::super::{classify_midi_event, MidiEvents, TransportAction};
        use crate::config::midi::{note_on, ToMidiEvent};
        use midly::live::LiveEvent;

        /// Keys 0 to 11 on channel 16, one per action in
        /// [`TransportAction::ALL`]'s order.
        fn make_test_events() -> MidiEvents {
            let mut key = 0;
            MidiEvents::new(|_| {
                let event = note_on(16, key, 127).to_midi_event()?;
                key += 1;
                Ok(Some(event))
            })
            .unwrap()
        }

        #[test]
        fn recognizes_ambient_modes_and_cancel_auto_play() {
            use crate::dmx::engine::AmbientMode;
            let events = make_test_events();
            for (key, action) in [
                (8, Some(TransportAction::AmbientMode(AmbientMode::On))),
                (9, Some(TransportAction::AmbientMode(AmbientMode::Off))),
                (10, Some(TransportAction::AmbientMode(AmbientMode::Auto))),
                (11, Some(TransportAction::CancelAutoPlay)),
            ] {
                let event = note_on(16, key, 127).to_midi_event().unwrap();
                assert_eq!(classify_midi_event(&events, &event), action, "key {key}");
            }
        }

        #[test]
        fn recognizes_play() {
            let events = make_test_events();
            let event = note_on(16, 0, 127).to_midi_event().unwrap();
            assert_eq!(
                classify_midi_event(&events, &event),
                Some(TransportAction::Play)
            );
        }

        #[test]
        fn recognizes_prev() {
            let events = make_test_events();
            let event = note_on(16, 1, 127).to_midi_event().unwrap();
            assert_eq!(
                classify_midi_event(&events, &event),
                Some(TransportAction::Prev)
            );
        }

        #[test]
        fn recognizes_next() {
            let events = make_test_events();
            let event = note_on(16, 2, 127).to_midi_event().unwrap();
            assert_eq!(
                classify_midi_event(&events, &event),
                Some(TransportAction::Next)
            );
        }

        #[test]
        fn recognizes_stop() {
            let events = make_test_events();
            let event = note_on(16, 3, 127).to_midi_event().unwrap();
            assert_eq!(
                classify_midi_event(&events, &event),
                Some(TransportAction::Stop)
            );
        }

        #[test]
        fn recognizes_all_songs() {
            let events = make_test_events();
            let event = note_on(16, 4, 127).to_midi_event().unwrap();
            assert_eq!(
                classify_midi_event(&events, &event),
                Some(TransportAction::AllSongs)
            );
        }

        #[test]
        fn recognizes_playlist() {
            let events = make_test_events();
            let event = note_on(16, 5, 127).to_midi_event().unwrap();
            assert_eq!(
                classify_midi_event(&events, &event),
                Some(TransportAction::Playlist)
            );
        }

        #[test]
        fn unrecognized_note() {
            let events = make_test_events();
            let event = note_on(16, 99, 127).to_midi_event().unwrap();
            assert_eq!(classify_midi_event(&events, &event), None);
        }

        #[test]
        fn wrong_channel_is_unrecognized() {
            let events = make_test_events();
            // Play is on channel 16, note 0 — try channel 1
            let event = note_on(1, 0, 127).to_midi_event().unwrap();
            assert_eq!(classify_midi_event(&events, &event), None);
        }

        #[test]
        fn different_velocity_still_matches() {
            let events = make_test_events();
            // Play is note_on(16, 0, 127) — try with velocity 64
            let event = note_on(16, 0, 64).to_midi_event().unwrap();
            // LiveEvent NoteOn equality checks velocity too, so different velocity won't match
            assert_eq!(classify_midi_event(&events, &event), None);
        }

        #[test]
        fn program_change_is_unrecognized() {
            let events = make_test_events();
            let event = LiveEvent::Midi {
                channel: 15.into(),
                message: midly::MidiMessage::ProgramChange { program: 27.into() },
            };
            assert_eq!(classify_midi_event(&events, &event), None);
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
            assert_eq!(classify_midi_event(&events, &event), None);
        }
    }
}
