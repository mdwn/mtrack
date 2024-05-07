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
use std::error::Error;
use std::io;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinError;
use tokio::{sync::mpsc::Sender, task::JoinHandle};
use tracing::{error, info, span, Level};

use crate::player::Player;

pub mod keyboard;
pub mod midi;

/// Controller events that will trigger behavior in the player.
#[derive(Debug)]
pub enum Event {
    /// Plays the track at the current position in the playlist.
    Play,

    /// Moves the current playlist position to the previous position.
    /// If a song is currently playing, does nothing.
    Prev,

    /// Moves the current playlist position to the next position.
    /// If a song is currently playing, does nothing.
    Next,

    /// Stops the currently playing song. If no song is playing, does nothing.
    Stop,

    /// Switches the playlist to the all songs playlist, which is an alphabetized
    /// playlist consisting of all songs in the song registry.
    AllSongs,

    /// Switches the playlist to the configured playlist.
    Playlist,
}

pub trait Driver: Send + Sync + 'static {
    fn monitor_events(&self, events_tx: Sender<Event>) -> JoinHandle<Result<(), io::Error>>;
}

/// Controls a playlist.
pub struct Controller {
    handle: JoinHandle<()>,
}

impl Controller {
    /// Creates a new controller with the given driver.
    pub fn new(player: Player, driver: Arc<dyn Driver>) -> Result<Controller, Box<dyn Error>> {
        Ok(Controller {
            handle: tokio::spawn(async move { Controller::trigger_events(player, driver).await }),
        })
    }

    /// Join will block until the controller finishes.
    pub async fn join(&mut self) -> Result<(), JoinError> {
        (&mut self.handle).await
    }

    /// Triggers player events by watching the driver and getting events from it.
    async fn trigger_events(mut player: Player, driver: Arc<dyn Driver>) {
        let span = span!(Level::INFO, "controller");
        let _enter = span.enter();

        let (events_tx, mut events_rx) = mpsc::channel(1);
        let join_handle = driver.monitor_events(events_tx);

        info!(
            first_song = player.get_playlist().current().name,
            "Controller started."
        );

        loop {
            if let Some(event) = events_rx.recv().await {
                info!(event = format!("{:?}", event), "Received event.");

                if let Err(e) = match event {
                    Event::Play => player.play().await,
                    Event::Prev => {
                        if let Err(e) = player.prev().await {
                            Err(e)
                        } else {
                            Ok(())
                        }
                    }
                    Event::Next => {
                        if let Err(e) = player.next().await {
                            Err(e)
                        } else {
                            Ok(())
                        }
                    }
                    Event::Stop => player.stop().await,
                    Event::AllSongs => player.switch_to_all_songs().await,
                    Event::Playlist => player.switch_to_playlist().await,
                } {
                    error!("Error talking to player: {}", e);
                }
            } else {
                info!("Controller closing.");
                if let Err(e) = join_handle.await {
                    error!("Error waiting for event monitor to stop: {}", e);
                }
                return;
            }
        }
    }
}

#[cfg(test)]
mod test {
    use std::{
        collections::HashMap,
        error::Error,
        io,
        path::PathBuf,
        sync::{Arc, Barrier, Mutex},
    };

    use tokio::{sync::mpsc::Sender, task::JoinHandle};

    use crate::{audio, config, player::Player, playlist::Playlist, test::eventually};

    use super::{Driver, Event};

    #[derive(Debug)]
    enum TestEvent {
        Unset,
        Play,
        Prev,
        Next,
        Stop,
        AllSongs,
        Playlist,
        Close,
    }

    struct TestDriver {
        current_event: Arc<Mutex<TestEvent>>,
        barrier: Arc<Barrier>,
    }

    impl TestDriver {
        /// Creates a new test driver which is explicitly controlled by the next_event function.
        fn new(current_event: TestEvent) -> TestDriver {
            let current_event = Arc::new(Mutex::new(current_event));
            let barrier = Arc::new(Barrier::new(2));
            TestDriver {
                current_event,
                barrier,
            }
        }

        /// Signals the next event to the monitor thread.
        fn next_event(&self, event: TestEvent) {
            {
                let mut current_event = self.current_event.lock().expect("failed to get lock");
                *current_event = event;
            }
            // Wait until the thread goes to receive the event.
            self.barrier.wait();
            // Wait until the thread has locked the mutex.
            self.barrier.wait();
        }
    }

    impl Driver for TestDriver {
        fn monitor_events(&self, events_tx: Sender<Event>) -> JoinHandle<Result<(), io::Error>> {
            let barrier = self.barrier.clone();
            let current_event = self.current_event.clone();
            let result: JoinHandle<Result<(), io::Error>> =
                tokio::task::spawn_blocking(move || {
                    loop {
                        // Wait for next event to set the current event.
                        barrier.wait();
                        let current_event = current_event.lock().expect("failed to get lock");
                        // Let next event know that we got the event.
                        barrier.wait();
                        match *current_event {
                            TestEvent::Unset => assert!(false, "current event should not be unset"),
                            TestEvent::Play => {
                                assert!(events_tx.blocking_send(Event::Play).is_ok())
                            }
                            TestEvent::Prev => {
                                assert!(events_tx.blocking_send(Event::Prev).is_ok())
                            }
                            TestEvent::Next => {
                                assert!(events_tx.blocking_send(Event::Next).is_ok())
                            }
                            TestEvent::Stop => {
                                assert!(events_tx.blocking_send(Event::Stop).is_ok())
                            }
                            TestEvent::AllSongs => {
                                assert!(events_tx.blocking_send(Event::AllSongs).is_ok())
                            }
                            TestEvent::Playlist => {
                                assert!(events_tx.blocking_send(Event::Playlist).is_ok())
                            }
                            TestEvent::Close => return Ok(()),
                        }
                    }
                });
            result
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_controller() -> Result<(), Box<dyn Error>> {
        let driver = Arc::new(TestDriver::new(TestEvent::Unset));
        let mappings: HashMap<String, Vec<u16>> = HashMap::new();
        let device = Arc::new(audio::test::Device::get("mock-device"));
        let songs = config::get_all_songs(&PathBuf::from("assets/songs"))?;
        let playlist =
            config::parse_playlist(&PathBuf::from("assets/playlist.yaml"), songs.clone())?;
        let all_songs_playlist = Playlist::from_songs(songs.clone())?;
        let player = Player::new(
            device.clone(),
            mappings,
            None,
            playlist.clone(),
            all_songs_playlist.clone(),
            None,
        );
        let mut controller = super::Controller::new(player, driver.clone())?;

        println!("Playlist: {}", playlist);
        println!("AllSongs: {}", all_songs_playlist);

        // Test the controller directing the player.
        println!("Playlist -> Song 1");
        eventually(
            || playlist.current().name == "Song 1",
            "Playlist never became Song 1",
        );
        driver.next_event(TestEvent::Next);
        println!("Playlist -> Song 3");
        eventually(
            || playlist.current().name == "Song 3",
            "Playlist never became Song 3",
        );
        driver.next_event(TestEvent::Next);
        println!("Playlist -> Song 5");
        eventually(
            || playlist.current().name == "Song 5",
            "Playlist never became Song 5",
        );
        driver.next_event(TestEvent::Next);
        println!("Playlist -> Song 7");
        eventually(
            || playlist.current().name == "Song 7",
            "Playlist never became Song 7",
        );
        driver.next_event(TestEvent::Prev);
        println!("Playlist -> Song 5");
        eventually(
            || playlist.current().name == "Song 5",
            "Playlist never became Song 5",
        );
        println!("Switch to AllSongs");
        driver.next_event(TestEvent::AllSongs);
        eventually(
            || all_songs_playlist.current().name == "Song 1",
            "All Songs Playlist never became Song 1",
        );
        println!("AllSongs -> Song 10");
        driver.next_event(TestEvent::Next);
        eventually(
            || all_songs_playlist.current().name == "Song 10",
            "All Songs Playlist never became Song 10",
        );
        println!("AllSongs -> Song 2");
        driver.next_event(TestEvent::Next);
        eventually(
            || all_songs_playlist.current().name == "Song 2",
            "All Songs Playlist never became Song 2",
        );
        println!("AllSongs -> Song 10");
        driver.next_event(TestEvent::Prev);
        eventually(
            || all_songs_playlist.current().name == "Song 10",
            "All Songs Playlist never became Song 10",
        );
        println!("Switch to Playlist");
        driver.next_event(TestEvent::Playlist);
        eventually(
            || playlist.current().name == "Song 5",
            "Playlist never became Song 5",
        );
        println!("Playlist -> Song 7");
        driver.next_event(TestEvent::Next);
        eventually(
            || playlist.current().name == "Song 7",
            "Playlist never became Song 7",
        );
        driver.next_event(TestEvent::Play);
        eventually(|| device.is_playing(), "Song never started playing");
        driver.next_event(TestEvent::Stop);
        eventually(|| !device.is_playing(), "Song never stopped playing");

        println!("Close");
        driver.next_event(TestEvent::Close);
        assert!(
            controller.join().await.is_ok(),
            "Error waiting for controller",
        );

        Ok(())
    }
}
