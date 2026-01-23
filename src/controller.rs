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
use std::error::Error;
use std::io;
use std::sync::Arc;
use tokio::task::JoinHandle;

use crate::config;
use crate::player::Player;

mod grpc;
mod midi;
mod osc;

pub trait Driver: Send + Sync + 'static {
    fn monitor_events(&self) -> JoinHandle<Result<(), io::Error>>;
}

/// Controls a playlist.
pub struct Controller {
    handles: Vec<JoinHandle<Result<(), io::Error>>>,
}

impl Controller {
    /// Creates a new controller with the given config.
    pub fn new(
        config: Vec<config::Controller>,
        player: Arc<Player>,
    ) -> Result<Controller, Box<dyn Error>> {
        let mut controller_drivers = Vec::new();
        for config in config {
            let player = player.clone();
            let driver: Arc<dyn Driver> = match config {
                config::Controller::Grpc(config) => grpc::Driver::new(config, player)?,
                config::Controller::Osc(config) => osc::Driver::new(config, player)?,
                config::Controller::Midi(config) => midi::Driver::new(config, player)?,
                _ => return Err("unexpected controller type".into()),
            };
            controller_drivers.push(driver);
        }
        Ok(Self::new_from_drivers(controller_drivers))
    }

    /// Creates a new controller from multiple drivers.
    pub fn new_from_drivers(drivers: Vec<Arc<dyn Driver>>) -> Controller {
        let mut handles = Vec::new();
        for driver in drivers {
            handles.push(driver.monitor_events());
        }
        Controller { handles }
    }

    /// Join will block until the controller finishes.
    pub async fn join(&mut self) -> Result<(), io::Error> {
        for handle in &mut self.handles {
            handle.await??;
        }

        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::{collections::HashMap, error::Error, io, path::Path, sync::Arc};

    use tokio::{
        sync::{Barrier, Mutex},
        task::JoinHandle,
    };
    use tracing::error;

    use crate::{config, player::Player, playlist::Playlist, songs, testutil::eventually};

    use super::Driver;

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
        player: Arc<Player>,
        current_event: Arc<Mutex<TestEvent>>,
        barrier: Arc<Barrier>,
    }

    impl TestDriver {
        /// Creates a new test driver which is explicitly controlled by the next_event function.
        fn new(player: Arc<Player>, current_event: TestEvent) -> TestDriver {
            let current_event = Arc::new(Mutex::new(current_event));
            let barrier = Arc::new(Barrier::new(2));
            TestDriver {
                player,
                current_event,
                barrier,
            }
        }

        /// Signals the next event to the monitor thread.
        async fn next_event(&self, event: TestEvent) {
            {
                let mut current_event = self.current_event.lock().await;
                *current_event = event;
            }
            // Wait until the thread goes to receive the event.
            self.barrier.wait().await;
            // Wait until the thread has locked the mutex.
            self.barrier.wait().await;
        }
    }

    impl Driver for TestDriver {
        fn monitor_events(&self) -> JoinHandle<Result<(), io::Error>> {
            let barrier = self.barrier.clone();
            let current_event = self.current_event.clone();
            let player = self.player.clone();
            let result: JoinHandle<Result<(), io::Error>> = tokio::spawn(async move {
                loop {
                    // Wait for next event to set the current event.
                    barrier.wait().await;
                    let current_event = current_event.lock().await;
                    // Let next event know that we got the event.
                    barrier.wait().await;
                    match *current_event {
                        TestEvent::Unset => unreachable!("current event should not be unset"),
                        TestEvent::Play => {
                            if let Err(e) = player.play().await {
                                error!(err = e.as_ref(), "Error playing song");
                            }
                        }
                        TestEvent::Prev => {
                            player.prev().await;
                        }
                        TestEvent::Next => {
                            player.next().await;
                        }
                        TestEvent::Stop => {
                            player.stop().await;
                        }
                        TestEvent::AllSongs => {
                            player.switch_to_all_songs().await;
                        }
                        TestEvent::Playlist => {
                            player.switch_to_playlist().await;
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
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let player = Arc::new(Player::new(
            songs.clone(),
            Playlist::new(
                "playlist",
                &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
                songs,
            )?,
            &config::Player::new(
                vec![],
                config::Audio::new("mock-device"),
                None,
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?);
        let playlist = player.get_playlist();
        let binding = player.audio_device();
        let device = binding.to_mock()?;

        let driver = Arc::new(TestDriver::new(player.clone(), TestEvent::Unset));
        let mut controller = super::Controller::new_from_drivers(vec![driver.clone()]);

        println!("Playlist: {}", playlist);

        // Test the controller directing the player.
        println!("Playlist -> Song 1");
        eventually(
            || playlist.current().name() == "Song 1",
            "Playlist never became Song 1",
        );
        driver.next_event(TestEvent::Next).await;
        println!("Playlist -> Song 3");
        eventually(
            || playlist.current().name() == "Song 3",
            "Playlist never became Song 3",
        );
        driver.next_event(TestEvent::Next).await;
        println!("Playlist -> Song 5");
        eventually(
            || playlist.current().name() == "Song 5",
            "Playlist never became Song 5",
        );
        driver.next_event(TestEvent::Next).await;
        println!("Playlist -> Song 7");
        eventually(
            || playlist.current().name() == "Song 7",
            "Playlist never became Song 7",
        );
        driver.next_event(TestEvent::Prev).await;
        println!("Playlist -> Song 5");
        eventually(
            || playlist.current().name() == "Song 5",
            "Playlist never became Song 5",
        );
        println!("Switch to AllSongs");
        driver.next_event(TestEvent::AllSongs).await;
        eventually(
            || player.get_playlist().current().name() == "Song 1",
            "All Songs Playlist never became Song 1",
        );
        println!("AllSongs -> Song 10");
        driver.next_event(TestEvent::Next).await;
        eventually(
            || player.get_playlist().current().name() == "Song 10",
            "All Songs Playlist never became Song 10",
        );
        println!("AllSongs -> Song 2");
        driver.next_event(TestEvent::Next).await;
        eventually(
            || player.get_playlist().current().name() == "Song 2",
            "All Songs Playlist never became Song 2",
        );
        println!("AllSongs -> Song 10");
        driver.next_event(TestEvent::Prev).await;
        eventually(
            || player.get_playlist().current().name() == "Song 10",
            "All Songs Playlist never became Song 10",
        );
        println!("Switch to Playlist");
        driver.next_event(TestEvent::Playlist).await;
        eventually(
            || playlist.current().name() == "Song 5",
            "Playlist never became Song 5",
        );
        println!("Playlist -> Song 7");
        driver.next_event(TestEvent::Next).await;
        eventually(
            || playlist.current().name() == "Song 7",
            "Playlist never became Song 7",
        );
        driver.next_event(TestEvent::Play).await;
        eventually(|| device.is_playing(), "Song never started playing");
        driver.next_event(TestEvent::Stop).await;
        eventually(|| !device.is_playing(), "Song never stopped playing");

        println!("Close");
        driver.next_event(TestEvent::Close).await;
        assert!(
            controller.join().await.is_ok(),
            "Error waiting for controller",
        );

        Ok(())
    }
}
