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
use crate::config;
use crate::player::Player;
use std::error::Error;
use std::io;
use std::sync::Arc;
use tokio::task::JoinHandle;

pub(crate) mod grpc;
mod mcp;
mod midi;
mod osc;

pub trait Driver: Send + Sync + 'static {
    fn monitor_events(&self) -> JoinHandle<Result<(), io::Error>>;
}

/// Status of a single controller.
#[derive(Clone, serde::Serialize)]
pub struct ControllerStatus {
    /// The kind of controller (grpc, osc, midi).
    pub kind: String,
    /// Whether the controller started successfully.
    pub status: String,
    /// Additional info (e.g. port number, device name).
    pub detail: Option<String>,
    /// Error message if the controller failed to start.
    pub error: Option<String>,
}

/// Controls a playlist.
pub struct Controller {
    entries: Vec<Entry>,
    statuses: Vec<ControllerStatus>,
}

/// One controller from the config, as started.
struct Entry {
    /// Its config, as YAML: what tells an unchanged controller from a
    /// changed one on the next start.
    fingerprint: Option<String>,
    /// Whether it is a MIDI controller, which registers song-change
    /// notifiers on the player.
    midi: bool,
    /// The player's MIDI device when it started, which a MIDI controller
    /// listens on. A hardware reload opens a new one.
    midi_device: Option<Arc<dyn crate::midi::Device>>,
    /// Its task, when it started.
    handle: Option<JoinHandle<Result<(), io::Error>>>,
}

impl Controller {
    /// Creates a new controller with the given config. Individual controller
    /// failures are logged and tracked but do not prevent other controllers
    /// from starting.
    pub fn new(config: Vec<config::Controller>, player: Arc<Player>) -> Controller {
        let mut controller = Controller {
            entries: Vec::new(),
            statuses: Vec::new(),
        };
        for config in config {
            controller.start(config, player.clone());
        }
        controller
    }

    /// Starts the controllers in `config`, leaving those already running
    /// with the same config as they are, and stops the rest. Restarting one
    /// tears down its transport, and a config change asked for over MCP or
    /// gRPC is answered on the transport of a controller that is running:
    /// restarting them all for a change to another dropped the reply (#578).
    ///
    /// A controller that failed to start is started again, as nothing else
    /// would retry it. So is a MIDI controller started on another MIDI
    /// device than the player's: a hardware reload opens the device anew,
    /// and one kept for its unchanged config went on listening to the old
    /// one, deaf to the pedalboard. MIDI controllers' song-change notifiers
    /// can only be dropped all together, so if any MIDI controller starts or
    /// stops, they all restart. `force` restarts every one. Returns whether
    /// any started or stopped.
    pub fn update(
        &mut self,
        config: Vec<config::Controller>,
        player: Arc<Player>,
        force: bool,
    ) -> bool {
        let midi_device = player.midi_device();
        let on_midi_device = |entry: &Entry| {
            matches!((&entry.midi_device, &midi_device),
                (Some(started), Some(current)) if Arc::ptr_eq(started, current))
        };
        // Which running controller, if any, each one in the config keeps.
        let mut taken = vec![false; self.entries.len()];
        let mut kept: Vec<Option<usize>> = config
            .iter()
            .map(|c| {
                let fingerprint = crate::util::to_yaml_string(c).ok();
                if force || fingerprint.is_none() {
                    return None;
                }
                let found = (0..self.entries.len()).find(|&i| {
                    !taken[i]
                        && self.statuses[i].status == "running"
                        && self.entries[i].fingerprint == fingerprint
                        && (!self.entries[i].midi || on_midi_device(&self.entries[i]))
                })?;
                taken[found] = true;
                Some(found)
            })
            .collect();
        let midi_changes = config
            .iter()
            .zip(&kept)
            .any(|(c, k)| k.is_none() && matches!(c, config::Controller::Midi(_)))
            || self
                .entries
                .iter()
                .zip(&taken)
                .any(|(entry, taken)| entry.midi && !taken);
        if midi_changes {
            for (c, k) in config.iter().zip(kept.iter_mut()) {
                if matches!(c, config::Controller::Midi(_)) {
                    *k = None;
                }
            }
        }
        if kept.iter().all(Option::is_some) && kept.len() == self.entries.len() {
            return false;
        }

        // Those going stop before any starts, so a changed one can bind the
        // port it had.
        let mut old: Vec<Option<(Entry, ControllerStatus)>> = std::mem::take(&mut self.entries)
            .into_iter()
            .zip(std::mem::take(&mut self.statuses))
            .map(Some)
            .collect();
        for (i, slot) in old.iter_mut().enumerate() {
            if kept.contains(&Some(i)) {
                continue;
            }
            if let Some((entry, status)) = slot.take() {
                tracing::info!(kind = %status.kind, "Stopping controller");
                if let Some(handle) = entry.handle {
                    handle.abort();
                }
            }
        }
        if midi_changes {
            player.clear_song_change_notifiers();
        }
        for (config, kept) in config.into_iter().zip(kept) {
            match kept.and_then(|i| old[i].take()) {
                Some((entry, status)) => {
                    self.entries.push(entry);
                    self.statuses.push(status);
                }
                None => self.start(config, player.clone()),
            }
        }
        true
    }

    /// Starts one controller, recording it whether it started or not.
    fn start(&mut self, config: config::Controller, player: Arc<Player>) {
        let fingerprint = crate::util::to_yaml_string(&config).ok();
        let midi = matches!(config, config::Controller::Midi(_));
        // Read before the driver reads it: should a reload swap the device in
        // between, this names the older one, and the next update restarts it.
        let midi_device = if midi { player.midi_device() } else { None };
        let (kind, detail) = match &config {
            config::Controller::Grpc(c) => ("grpc".to_string(), Some(format!("port {}", c.port()))),
            config::Controller::Mcp(c) => (
                "mcp".to_string(),
                Some(format!("{}:{}", c.bind_address(), c.port())),
            ),
            config::Controller::Osc(c) => ("osc".to_string(), Some(format!("port {}", c.port()))),
            config::Controller::Midi(_) => ("midi".to_string(), None),
            _ => ("unknown".to_string(), None),
        };
        let result: Result<Arc<dyn Driver>, Box<dyn Error>> = match config {
            config::Controller::Grpc(config) => {
                grpc::Driver::new(config, player).map(|d| d as Arc<dyn Driver>)
            }
            config::Controller::Mcp(config) => {
                mcp::Driver::new(config, player).map(|d| d as Arc<dyn Driver>)
            }
            config::Controller::Osc(config) => {
                osc::Driver::new(config, player).map(|d| d as Arc<dyn Driver>)
            }
            config::Controller::Midi(config) => {
                midi::Driver::new(config, player).map(|d| d as Arc<dyn Driver>)
            }
            _ => Err("unexpected controller type".into()),
        };
        let (handle, status) = match result {
            Ok(driver) => (
                Some(driver.monitor_events()),
                ControllerStatus {
                    kind,
                    status: "running".to_string(),
                    detail,
                    error: None,
                },
            ),
            Err(e) => {
                tracing::warn!(kind = %kind, error = %e, "Controller failed to start");
                (
                    None,
                    ControllerStatus {
                        kind,
                        status: "error".to_string(),
                        detail,
                        error: Some(e.to_string()),
                    },
                )
            }
        };
        self.entries.push(Entry {
            fingerprint,
            midi,
            midi_device,
            handle,
        });
        self.statuses.push(status);
    }

    /// Creates a new controller from multiple drivers.
    pub fn new_from_drivers(drivers: Vec<Arc<dyn Driver>>) -> Controller {
        Controller {
            entries: drivers
                .iter()
                .map(|driver| Entry {
                    fingerprint: None,
                    midi: false,
                    midi_device: None,
                    handle: Some(driver.monitor_events()),
                })
                .collect(),
            statuses: drivers
                .iter()
                .map(|_| ControllerStatus {
                    kind: "unknown".to_string(),
                    status: "running".to_string(),
                    detail: None,
                    error: None,
                })
                .collect(),
        }
    }

    /// Returns the status of all controllers.
    pub fn statuses(&self) -> &[ControllerStatus] {
        &self.statuses
    }

    /// The tasks of the controllers that started, for a test to tell one
    /// left running from one restarted.
    #[cfg(test)]
    pub(crate) fn tasks(&self) -> Vec<tokio::task::AbortHandle> {
        self.entries
            .iter()
            .filter_map(|entry| entry.handle.as_ref().map(JoinHandle::abort_handle))
            .collect()
    }

    /// Shuts down all controller tasks by aborting their handles.
    pub fn shutdown(self) {
        for handle in self.entries.into_iter().filter_map(|entry| entry.handle) {
            handle.abort();
        }
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

    use crate::{
        config, player::Player, playlist, playlist::Playlist, songs, testutil::eventually,
    };

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
                            player.switch_to_playlist("all_songs").await.unwrap();
                        }
                        TestEvent::Playlist => {
                            player.switch_to_playlist("playlist").await.unwrap();
                        }
                        TestEvent::Close => return Ok(()),
                    }
                }
            });
            result
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_controller_new_with_grpc() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let mut playlists = HashMap::new();
        playlists.insert(
            "all_songs".to_string(),
            playlist::from_songs(songs.clone())?,
        );
        playlists.insert("playlist".to_string(), playlist);
        let player = Player::new(
            playlists,
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;

        // Use port 0 to let the OS pick an available port.
        let grpc_config = config::GrpcController::new(0);
        let controller =
            super::Controller::new(vec![config::Controller::Grpc(grpc_config)], player);
        assert!(controller.statuses().iter().all(|s| s.status == "running"));
        controller.shutdown();

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_controller_new_with_mcp() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let mut playlists = HashMap::new();
        playlists.insert(
            "all_songs".to_string(),
            playlist::from_songs(songs.clone())?,
        );
        playlists.insert("playlist".to_string(), playlist);
        let player = Player::new(
            playlists,
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;

        // Use port 0 to let the OS pick an available port.
        let mcp_config = config::McpController::new(0);
        let controller = super::Controller::new(vec![config::Controller::Mcp(mcp_config)], player);
        assert!(controller.statuses().iter().all(|s| s.status == "running"));
        controller.shutdown();

        Ok(())
    }

    /// A change to one controller restarted them all, the one carrying the
    /// request that made it included (#578). Only those whose config changed
    /// start or stop; the rest keep running, until a forced restart.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_change_to_one_controller_leaves_the_others_running() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let mut playlists = HashMap::new();
        playlists.insert("all_songs".to_string(), playlist::from_songs(songs)?);
        let player = Player::new(
            playlists,
            "all_songs".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;
        let grpc = || config::Controller::Grpc(config::GrpcController::new(0));
        let mcp = || config::Controller::Mcp(config::McpController::new(0));
        let task = |c: &super::Controller, i: usize| {
            c.entries[i]
                .handle
                .as_ref()
                .expect("running")
                .abort_handle()
        };

        let mut controller = super::Controller::new(vec![grpc()], player.clone());
        let grpc_task = task(&controller, 0);
        assert!(controller.update(vec![grpc(), mcp()], player.clone(), false));
        let mcp_task = task(&controller, 1);
        assert!(!controller.update(vec![grpc(), mcp()], player.clone(), false));
        assert!(controller.update(vec![mcp()], player.clone(), false));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(grpc_task.is_finished(), "the removed controller still runs");
        assert!(
            !mcp_task.is_finished(),
            "an unchanged controller was restarted"
        );
        assert_eq!(controller.statuses().len(), 1);

        assert!(controller.update(vec![mcp()], player.clone(), true));
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(mcp_task.is_finished(), "a forced restart left it running");
        controller.shutdown();
        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_controller_new_empty_config() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let mut playlists = HashMap::new();
        playlists.insert(
            "all_songs".to_string(),
            playlist::from_songs(songs.clone())?,
        );
        playlists.insert("playlist".to_string(), playlist);
        let player = Player::new(
            playlists,
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;

        // Empty config vec should produce a controller with no drivers.
        let controller = super::Controller::new(vec![], player);
        assert!(controller.statuses().is_empty());
        controller.shutdown();

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_controller_new_with_osc() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let mut playlists = HashMap::new();
        playlists.insert(
            "all_songs".to_string(),
            playlist::from_songs(songs.clone())?,
        );
        playlists.insert("playlist".to_string(), playlist);
        let player = Player::new(
            playlists,
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;

        let osc_config = config::OscController::new();
        let controller =
            super::Controller::new(vec![config::Controller::Osc(Box::new(osc_config))], player);
        assert!(controller.statuses().iter().all(|s| s.status == "running"));
        controller.shutdown();

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_controller() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let mut playlists = HashMap::new();
        playlists.insert(
            "all_songs".to_string(),
            playlist::from_songs(songs.clone())?,
        );
        playlists.insert("playlist".to_string(), playlist);
        let player = Player::new(
            playlists,
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
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

        let driver = Arc::new(TestDriver::new(player.clone(), TestEvent::Unset));
        let controller = super::Controller::new_from_drivers(vec![driver.clone()]);

        println!("Playlist: {}", playlist);

        // Test the controller directing the player.
        println!("Playlist -> Song 1");
        eventually(
            || playlist.current().unwrap().name() == "Song 1",
            "Playlist never became Song 1",
        );
        driver.next_event(TestEvent::Next).await;
        println!("Playlist -> Song 3");
        eventually(
            || playlist.current().unwrap().name() == "Song 3",
            "Playlist never became Song 3",
        );
        driver.next_event(TestEvent::Next).await;
        println!("Playlist -> Song 5");
        eventually(
            || playlist.current().unwrap().name() == "Song 5",
            "Playlist never became Song 5",
        );
        driver.next_event(TestEvent::Next).await;
        println!("Playlist -> Song 7");
        eventually(
            || playlist.current().unwrap().name() == "Song 7",
            "Playlist never became Song 7",
        );
        driver.next_event(TestEvent::Prev).await;
        println!("Playlist -> Song 5");
        eventually(
            || playlist.current().unwrap().name() == "Song 5",
            "Playlist never became Song 5",
        );
        println!("Switch to AllSongs");
        driver.next_event(TestEvent::AllSongs).await;
        eventually(
            || player.get_playlist().current().unwrap().name() == "Song 1",
            "All Songs Playlist never became Song 1",
        );
        println!("AllSongs -> Song 10");
        driver.next_event(TestEvent::Next).await;
        eventually(
            || player.get_playlist().current().unwrap().name() == "Song 10",
            "All Songs Playlist never became Song 10",
        );
        println!("AllSongs -> Song 2");
        driver.next_event(TestEvent::Next).await;
        eventually(
            || player.get_playlist().current().unwrap().name() == "Song 2",
            "All Songs Playlist never became Song 2",
        );
        println!("AllSongs -> Song 10");
        driver.next_event(TestEvent::Prev).await;
        eventually(
            || player.get_playlist().current().unwrap().name() == "Song 10",
            "All Songs Playlist never became Song 10",
        );
        println!("Switch to Playlist");
        driver.next_event(TestEvent::Playlist).await;
        eventually(
            || playlist.current().unwrap().name() == "Song 5",
            "Playlist never became Song 5",
        );
        println!("Playlist -> Song 7");
        driver.next_event(TestEvent::Next).await;
        eventually(
            || playlist.current().unwrap().name() == "Song 7",
            "Playlist never became Song 7",
        );
        driver.next_event(TestEvent::Play).await;
        eventually(|| device.is_playing(), "Song never started playing");
        driver.next_event(TestEvent::Stop).await;
        eventually(|| !device.is_playing(), "Song never stopped playing");

        println!("Close");
        driver.next_event(TestEvent::Close).await;
        controller.shutdown();

        Ok(())
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_controller_shutdown() -> Result<(), Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let playlist = Playlist::new(
            "playlist",
            &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
            songs.clone(),
        )?;
        let mut playlists = HashMap::new();
        playlists.insert(
            "all_songs".to_string(),
            playlist::from_songs(songs.clone())?,
        );
        playlists.insert("playlist".to_string(), playlist);
        let player = Player::new(
            playlists,
            "playlist".to_string(),
            &config::Player::new(
                vec![],
                Some(config::Audio::new("mock-device")),
                None,
                None,
                HashMap::new(),
                "assets/songs",
            ),
            None,
        )?;
        player.await_hardware_ready().await;

        let driver = Arc::new(TestDriver::new(player.clone(), TestEvent::Unset));
        let controller = super::Controller::new_from_drivers(vec![driver.clone()]);

        // Shutdown should abort all handles without panicking.
        controller.shutdown();

        Ok(())
    }
}
