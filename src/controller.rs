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
use tokio_util::sync::CancellationToken;

pub(crate) mod grpc;
mod mcp;
mod midi;
mod osc;

pub trait Driver: Send + Sync + 'static {
    fn monitor_events(&self) -> JoinHandle<Result<(), io::Error>>;
}

/// How long a controller starting waits for those it replaces to be gone. A
/// driver stuck in blocking code, which an abort cannot interrupt, may never
/// end, and its replacement would wait on it for good while reading "running".
#[cfg(not(test))]
const STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(5);
#[cfg(test)]
const STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(1);

/// Aborts a child task when the owning controller task is cancelled.
///
/// Tokio does not automatically cancel tasks spawned by a parent task. Without
/// this guard, restarting controllers leaves the old UDP task alive and holding
/// the OSC port while its channel receiver has already been dropped, and a
/// controller shut down leaves its driver's task running.
struct AbortOnDrop<T>(JoinHandle<T>);

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
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
    /// Whether it is a MIDI controller refused for coming after another:
    /// the one MIDI device can be watched by only one. It registers
    /// nothing, and does not count as a MIDI controller.
    duplicate: bool,
    /// The player's MIDI device when it started, which a MIDI controller
    /// listens on. A hardware reload opens a new one if the MIDI config
    /// changed or the device is gone, and otherwise keeps it.
    midi_device: Option<Arc<dyn crate::midi::Device>>,
    /// Its task, when it started.
    handle: Option<JoinHandle<Result<(), io::Error>>>,
    /// Stops its task, which ends once the driver's own task is gone.
    stop: CancellationToken,
    /// What its task waited on before running the driver, cancelled once
    /// the controllers it replaced are gone. Its own end does not say they
    /// are: a wait that timed out ends it while they may still hold the
    /// port, so those waiting on it wait on this too.
    after: Option<CancellationToken>,
    /// Why its task ended, when it failed after it started: a listener
    /// binds its port on the task, not before it.
    failure: Arc<parking_lot::Mutex<Option<String>>>,
}

impl Entry {
    /// Runs `driver` on a task of its own, once `after` is cancelled: the
    /// controllers stopped before it may still hold the port it binds. If
    /// they are not gone within [`STOP_WAIT`], it does not run, and fails.
    fn run(
        driver: Arc<dyn Driver>,
        after: Option<CancellationToken>,
        fingerprint: Option<String>,
        midi: bool,
        midi_device: Option<Arc<dyn crate::midi::Device>>,
    ) -> Entry {
        let stop = CancellationToken::new();
        let failure = Arc::new(parking_lot::Mutex::new(None));
        let handle = {
            let stop = stop.clone();
            let failure = failure.clone();
            let after = after.clone();
            tokio::spawn(async move {
                if let Some(after) = after {
                    // A stop ends the wait at once: those started after
                    // this one wait for `after` as well as for its end.
                    let gone = tokio::select! {
                        gone = tokio::time::timeout(STOP_WAIT, after.cancelled()) => gone.is_ok(),
                        _ = stop.cancelled() => return Ok(()),
                    };
                    // Not started at all: one still holding its port or its
                    // MIDI watch would only have this one fail, or worse,
                    // both run. As an error, the next update tries again.
                    if !gone {
                        let e = format!(
                            "not started: the controller it replaces had not stopped after {}s",
                            STOP_WAIT.as_secs()
                        );
                        tracing::warn!(error = %e, "Controller not started");
                        *failure.lock() = Some(e.clone());
                        return Err(io::Error::other(e));
                    }
                }
                let mut task = AbortOnDrop(driver.monitor_events());
                let result = tokio::select! {
                    result = &mut task.0 => result,
                    _ = stop.cancelled() => {
                        // Aborted and waited out, so that this task's end
                        // means the driver's listener is gone too.
                        task.0.abort();
                        let _ = (&mut task.0).await;
                        return Ok(());
                    }
                };
                let result = result.unwrap_or_else(|e| Err(io::Error::other(e)));
                if let Err(e) = &result {
                    tracing::warn!(error = %e, "Controller stopped with an error");
                    *failure.lock() = Some(e.to_string());
                }
                result
            })
        };
        Entry {
            fingerprint,
            midi,
            duplicate: false,
            midi_device,
            handle: Some(handle),
            stop,
            after,
            failure,
        }
    }

    /// Stops the tasks of `entries`. Returns, if any had one, a token
    /// cancelled once they are all gone, and so are the controllers they
    /// waited on, for those starting to wait on. One that gave up waiting
    /// on a controller stuck holding its port ends long before that one
    /// does: taking its end for the stuck one's started the next on top of
    /// it (#578).
    fn stop_all(entries: Vec<Entry>) -> Option<CancellationToken> {
        let mut afters = Vec::new();
        let handles: Vec<_> = entries
            .into_iter()
            .filter_map(|entry| {
                entry.stop.cancel();
                afters.extend(entry.after);
                entry.handle
            })
            .collect();
        if handles.is_empty() && afters.is_empty() {
            return None;
        }
        let stopped = CancellationToken::new();
        let done = stopped.clone();
        tokio::spawn(async move {
            for handle in handles {
                let _ = handle.await;
            }
            // Not bounded here: those waiting on this token bound their own
            // wait, and read as an error if it runs out.
            for after in afters {
                after.cancelled().await;
            }
            done.cancel();
        });
        Some(stopped)
    }
}

/// Whether each of `config` is a MIDI controller after the first. A MIDI
/// controller watches the player's one MIDI device, which only one can: the
/// watch of any other fails.
fn midi_duplicates(config: &[config::Controller]) -> Vec<bool> {
    let mut seen = false;
    config
        .iter()
        .map(|c| {
            let midi = matches!(c, config::Controller::Midi(_));
            let duplicate = midi && seen;
            seen |= midi;
            duplicate
        })
        .collect()
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
        let duplicates = midi_duplicates(&config);
        for (config, duplicate) in config.into_iter().zip(duplicates) {
            controller.start(config, player.clone(), None, duplicate);
        }
        controller
    }

    /// Starts the controllers in `config`, leaving those already running
    /// with the same config as they are, and stops the rest. Restarting one
    /// tears down its transport, and a config change asked for over MCP or
    /// gRPC is answered on the transport of a controller that is running:
    /// restarting them all for a change to another dropped the reply (#578).
    ///
    /// A controller that failed to start, or failed since, is started again,
    /// as nothing else would retry it. So is a MIDI controller started on
    /// another MIDI device than the player's: a hardware reload of a changed
    /// MIDI config, or of a device gone, opens a new one, and one kept for
    /// its unchanged config went on listening to the old one, deaf to the
    /// pedalboard. MIDI controllers' song-change notifiers can only be
    /// dropped all together, so if any MIDI controller starts or stops, they
    /// all restart. `force` restarts every one.
    /// A MIDI controller after the first is refused, and stays refused
    /// without a retry: retrying it restarted the first one each time.
    /// Returns whether any started or stopped.
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
        let statuses = self.statuses();
        let duplicates = midi_duplicates(&config);
        let mut taken = vec![false; self.entries.len()];
        let mut kept: Vec<Option<usize>> = config
            .iter()
            .zip(&duplicates)
            .map(|(c, &duplicate)| {
                let fingerprint = crate::util::to_yaml_string(c).ok();
                if force || fingerprint.is_none() {
                    return None;
                }
                let found = (0..self.entries.len()).find(|&i| {
                    let entry = &self.entries[i];
                    !taken[i]
                        && entry.fingerprint == fingerprint
                        && if duplicate {
                            entry.duplicate
                        } else {
                            statuses[i].status == "running"
                                && (!entry.midi || on_midi_device(entry))
                        }
                })?;
                taken[found] = true;
                Some(found)
            })
            .collect();
        let is_midi = |(c, duplicate): (&config::Controller, &bool)| {
            matches!(c, config::Controller::Midi(_)) && !duplicate
        };
        let midi_changes = config
            .iter()
            .zip(&duplicates)
            .zip(&kept)
            .any(|(c, k)| k.is_none() && is_midi(c))
            || self
                .entries
                .iter()
                .zip(&taken)
                .any(|(entry, taken)| entry.midi && !taken);
        if midi_changes {
            for (c, k) in config.iter().zip(&duplicates).zip(kept.iter_mut()) {
                if is_midi(c) {
                    *k = None;
                }
            }
        }
        if kept.iter().all(Option::is_some) && kept.len() == self.entries.len() {
            return false;
        }

        // Those going stop before any starts, so a changed one can bind the
        // port it had. A stop takes effect on the stopped task's worker,
        // later: the controllers starting wait for those stopping to be gone,
        // or one would find its old port still bound and fail.
        let mut old: Vec<Option<(Entry, ControllerStatus)>> = std::mem::take(&mut self.entries)
            .into_iter()
            .zip(std::mem::take(&mut self.statuses))
            .map(Some)
            .collect();
        let mut stopping = Vec::new();
        for (i, slot) in old.iter_mut().enumerate() {
            if kept.contains(&Some(i)) {
                continue;
            }
            if let Some((entry, status)) = slot.take() {
                tracing::info!(kind = %status.kind, "Stopping controller");
                stopping.push(entry);
            }
        }
        let stopped = Entry::stop_all(stopping);
        if midi_changes {
            player.clear_song_change_notifiers();
        }
        for ((config, duplicate), kept) in config.into_iter().zip(duplicates).zip(kept) {
            match kept.and_then(|i| old[i].take()) {
                Some((entry, status)) => {
                    self.entries.push(entry);
                    self.statuses.push(status);
                }
                None => self.start(config, player.clone(), stopped.clone(), duplicate),
            }
        }
        true
    }

    /// Starts one controller, recording it whether it started or not. Its
    /// task runs the driver once `after` is cancelled. A `duplicate` MIDI
    /// controller is refused before its driver registers anything.
    fn start(
        &mut self,
        config: config::Controller,
        player: Arc<Player>,
        after: Option<CancellationToken>,
        duplicate: bool,
    ) {
        let fingerprint = crate::util::to_yaml_string(&config).ok();
        let midi = matches!(config, config::Controller::Midi(_)) && !duplicate;
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
            config::Controller::Midi(_) if duplicate => Err(
                "only one MIDI controller can watch the MIDI device, and another comes \
                 before this one; put its events in that one"
                    .into(),
            ),
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
        let (entry, status) = match result {
            Ok(driver) => (
                Entry::run(driver, after, fingerprint, midi, midi_device),
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
                    Entry {
                        fingerprint,
                        midi,
                        duplicate,
                        midi_device,
                        handle: None,
                        stop: CancellationToken::new(),
                        // Never waited on, but what it was to wait on is
                        // what its retry must wait on.
                        after,
                        failure: Arc::new(parking_lot::Mutex::new(None)),
                    },
                    ControllerStatus {
                        kind,
                        status: "error".to_string(),
                        detail,
                        error: Some(e.to_string()),
                    },
                )
            }
        };
        self.entries.push(entry);
        self.statuses.push(status);
    }

    /// Creates a new controller from multiple drivers.
    pub fn new_from_drivers(drivers: Vec<Arc<dyn Driver>>) -> Controller {
        Controller {
            entries: drivers
                .iter()
                .map(|driver| Entry::run(driver.clone(), None, None, false, None))
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

    /// Returns the status of all controllers. One that started and then
    /// failed, a listener that could not bind its port say, reads as the
    /// error it failed with.
    pub fn statuses(&self) -> Vec<ControllerStatus> {
        self.entries
            .iter()
            .zip(&self.statuses)
            .map(|(entry, status)| match entry.failure.lock().clone() {
                Some(error) => ControllerStatus {
                    status: "error".to_string(),
                    error: Some(error),
                    ..status.clone()
                },
                None => status.clone(),
            })
            .collect()
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

    /// A player with no controllers of its own, for tests that run some.
    async fn make_controllerless_player() -> Result<Arc<Player>, Box<dyn Error>> {
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
        Ok(player)
    }

    /// A MIDI controller whose six events are notes from `note` on.
    fn midi_controller(note: u8) -> config::Controller {
        config::Controller::Midi(config::MidiController::new(
            config::midi::note_on(16, note, 127),
            config::midi::note_on(16, note + 1, 127),
            config::midi::note_on(16, note + 2, 127),
            config::midi::note_on(16, note + 3, 127),
            config::midi::note_on(16, note + 4, 127),
            config::midi::note_on(16, note + 5, 127),
        ))
    }

    /// A player config with a mock MIDI device named `midi`.
    fn midi_player_config(controllers: Vec<config::Controller>, midi: &str) -> config::Player {
        config::Player::new(
            controllers,
            Some(config::Audio::new("mock-device")),
            Some(config::Midi::new(midi, None)),
            None,
            HashMap::new(),
            "assets/songs",
        )
    }

    /// A player with a mock MIDI device named `midi`, running `controllers`.
    async fn make_midi_player(
        controllers: Vec<config::Controller>,
        midi: &str,
    ) -> Result<Arc<Player>, Box<dyn Error>> {
        let songs = songs::get_all_songs(Path::new("assets/songs"))?;
        let mut playlists = HashMap::new();
        playlists.insert("all_songs".to_string(), playlist::from_songs(songs)?);
        let player = Player::new(
            playlists,
            "all_songs".to_string(),
            &midi_player_config(controllers, midi),
            None,
        )?;
        player.await_hardware_ready().await;
        Ok(player)
    }

    /// Two MIDI controllers on the one MIDI device: the second's watch
    /// failed, so it read as an error, every later update retried it, and
    /// as MIDI controllers restart together, the working first one was torn
    /// down with it each time. The second is refused, says why, and is left
    /// refused.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_second_midi_controller_is_refused_and_left_alone() -> Result<(), Box<dyn Error>> {
        let player = make_midi_player(vec![], "mock-second-midi-controller").await?;
        let configs = || vec![midi_controller(0), midi_controller(20)];
        let mut controller = super::Controller::new(configs(), player.clone());
        let statuses = controller.statuses();
        assert_eq!(statuses[0].status, "running");
        assert_eq!(statuses[1].status, "error");
        assert!(statuses[1]
            .error
            .as_deref()
            .is_some_and(|e| e.contains("only one MIDI controller")));
        let first = controller.tasks()[0].clone();

        let changed = controller.update(configs(), player.clone(), false);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let restarted = first.is_finished();
        let running = controller.statuses()[0].status == "running";
        controller.shutdown();
        assert!(!changed, "an unchanged config restarted controllers");
        assert!(!restarted, "the working MIDI controller was torn down");
        assert!(running, "the working MIDI controller failed");
        Ok(())
    }

    /// Runs a hardware reload of `player` to a config whose MIDI device is
    /// named `midi`, and returns whether its one controller was restarted,
    /// and whether the reload kept the MIDI device it had.
    async fn reload_midi_player(
        player: &Arc<Player>,
        midi: &str,
    ) -> Result<(bool, bool), Box<dyn Error>> {
        let dir = tempfile::tempdir()?;
        player.set_config_store(Arc::new(config::ConfigStore::new(
            midi_player_config(vec![midi_controller(0)], midi),
            dir.path().join("config.yaml"),
        )));
        let device = player.midi_device().expect("a MIDI device");
        let tasks = player.controller_tasks();
        assert_eq!(tasks.len(), 1);
        assert!(player.reload_hardware().await?.applied());
        player.await_hardware_ready().await;
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let restarted = tasks[0].is_finished();
        let kept = player
            .midi_device()
            .is_some_and(|now| Arc::ptr_eq(&now, &device));
        Ok((restarted, kept))
    }

    /// Every hardware reload opened the MIDI device anew, so a save of
    /// anything at all restarted every MIDI controller and its Morningstar
    /// notifiers. A reload of the same MIDI config keeps the device, and
    /// the controllers with it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reload_of_the_same_midi_config_keeps_the_midi_controllers(
    ) -> Result<(), Box<dyn Error>> {
        let midi = "mock-reload-same-midi";
        let player = make_midi_player(vec![midi_controller(0)], midi).await?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let (restarted, kept) = reload_midi_player(&player, midi).await?;
        player.shutdown_controllers();
        assert!(kept, "the MIDI device was opened anew");
        assert!(
            !restarted,
            "an unchanged MIDI controller was restarted by a reload that changed nothing"
        );
        Ok(())
    }

    /// A changed MIDI config still opens a new device, and the MIDI
    /// controllers restart to listen on it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reload_of_a_changed_midi_config_restarts_the_midi_controllers(
    ) -> Result<(), Box<dyn Error>> {
        let player = make_midi_player(vec![midi_controller(0)], "mock-reload-old-midi").await?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let (restarted, kept) = reload_midi_player(&player, "mock-reload-new-midi").await?;
        player.shutdown_controllers();
        assert!(!kept, "the old MIDI device was kept");
        assert!(restarted, "the MIDI controller went on with the old device");
        Ok(())
    }

    /// A device that cannot be opened afresh, unplugged say, is not kept:
    /// a new one is opened, and the MIDI controllers restart on it, as a
    /// reload did before it kept the device.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_reload_opens_a_midi_device_gone_anew() -> Result<(), Box<dyn Error>> {
        let midi = "mock-reload-gone-midi";
        let player = make_midi_player(vec![midi_controller(0)], midi).await?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        player
            .midi_device()
            .expect("a MIDI device")
            .to_mock()?
            .unplug();
        let (restarted, kept) = reload_midi_player(&player, midi).await?;
        player.shutdown_controllers();
        assert!(!kept, "the MIDI device gone was kept");
        assert!(
            restarted,
            "the MIDI controller went on with the device gone"
        );
        Ok(())
    }

    /// A changed controller was stopped before its replacement started, but
    /// a stop is an abort, which drops the old listener later, on its
    /// worker: the replacement bound the same port at once, and lost the
    /// race for it. It now starts once the old one is gone.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_changed_controller_binds_the_port_it_had() -> Result<(), Box<dyn Error>> {
        let player = make_controllerless_player().await?;
        let port = std::net::TcpListener::bind("127.0.0.1:0")?
            .local_addr()?
            .port();
        let mcp = |token: &str| {
            config::Controller::Mcp(config::McpController::with_bearer_token(port, token))
        };
        let mut failures = 0;
        for round in 0..10 {
            let mut controller = super::Controller::new(vec![mcp("old")], player.clone());
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            // A new token: the same port, a changed config.
            assert!(controller.update(vec![mcp(&format!("new{round}"))], player.clone(), false));
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            if controller.entries[0]
                .handle
                .as_ref()
                .expect("started")
                .is_finished()
            {
                failures += 1;
            }
            controller.shutdown();
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert_eq!(
            failures, 0,
            "the restarted controller lost the race for its own port {failures}/10 times"
        );
        Ok(())
    }

    /// A driver whose first listener is stuck in blocking code holding its
    /// port for a while, and which notes a later one started while it still
    /// holds it.
    struct StuckThenHeld {
        starts: Arc<std::sync::atomic::AtomicUsize>,
        held: Arc<std::sync::atomic::AtomicBool>,
        overlapped: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Driver for StuckThenHeld {
        fn monitor_events(&self) -> JoinHandle<Result<(), io::Error>> {
            use std::sync::atomic::Ordering;
            let n = self.starts.fetch_add(1, Ordering::SeqCst);
            if self.held.load(Ordering::SeqCst) {
                self.overlapped.store(true, Ordering::SeqCst);
            }
            if n == 0 {
                self.held.store(true, Ordering::SeqCst);
                let held = self.held.clone();
                tokio::spawn(async move {
                    std::thread::sleep(super::STOP_WAIT * 3);
                    held.store(false, Ordering::SeqCst);
                    Ok(())
                })
            } else {
                tokio::spawn(async { std::future::pending::<Result<(), io::Error>>().await })
            }
        }
    }

    /// Three updates: A is stuck, B gives up waiting on it and ends, and C
    /// waits on B. C took B's end for A's and started while A still held
    /// the port. It waits on A too, and gives up in turn.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_wait_past_a_timed_out_one_waits_for_the_stuck_controller() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let overlapped = Arc::new(AtomicBool::new(false));
        let driver = Arc::new(StuckThenHeld {
            starts: Arc::new(AtomicUsize::new(0)),
            held: Arc::new(AtomicBool::new(false)),
            overlapped: overlapped.clone(),
        });
        let a = super::Entry::run(driver.clone(), None, None, false, None);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let stopped = super::Entry::stop_all(vec![a]);
        let b = super::Entry::run(driver.clone(), stopped, None, false, None);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        let stopped = super::Entry::stop_all(vec![b]);
        let c = super::Entry::run(driver.clone(), stopped, None, false, None);
        tokio::time::sleep(super::STOP_WAIT * 2).await;
        let c_failure = c.failure.lock().clone();
        super::Entry::stop_all(vec![c]);
        assert!(
            !overlapped.load(Ordering::SeqCst),
            "C started while the stuck A still held the port (C failure: {c_failure:?})"
        );
    }

    /// A controller that gave up waiting on a stuck one, as an error, is
    /// retried by the next update. The retry waits on the stuck one too, and
    /// reads as the same error if it is still there.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_retry_of_a_timed_out_controller_waits_for_the_stuck_one() {
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        let overlapped = Arc::new(AtomicBool::new(false));
        let driver = Arc::new(StuckThenHeld {
            starts: Arc::new(AtomicUsize::new(0)),
            held: Arc::new(AtomicBool::new(false)),
            overlapped: overlapped.clone(),
        });
        let a = super::Entry::run(driver.clone(), None, None, false, None);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let stopped = super::Entry::stop_all(vec![a]);
        let b = super::Entry::run(driver.clone(), stopped, None, false, None);
        crate::testutil::eventually(|| b.failure.lock().is_some(), "B never gave up");
        let stopped = super::Entry::stop_all(vec![b]);
        let retry = super::Entry::run(driver.clone(), stopped, None, false, None);
        crate::testutil::eventually(
            || retry.failure.lock().is_some() || driver.starts.load(Ordering::SeqCst) > 1,
            "the retry neither started nor gave up",
        );
        let failure = retry.failure.lock().clone();
        super::Entry::stop_all(vec![retry]);
        assert!(
            !overlapped.load(Ordering::SeqCst),
            "the retry started while the stuck one still held the port"
        );
        assert!(
            failure
                .as_deref()
                .is_some_and(|e| e.contains("had not stopped")),
            "the retry did not read as an error: {failure:?}"
        );
    }

    /// A driver whose listener is slow to let go of its port, and which
    /// notes a second listener started while the first still holds it.
    struct SlowToRelease {
        held: Arc<std::sync::atomic::AtomicBool>,
        overlapped: Arc<std::sync::atomic::AtomicBool>,
    }

    impl Driver for SlowToRelease {
        fn monitor_events(&self) -> JoinHandle<Result<(), io::Error>> {
            use std::sync::atomic::Ordering;

            struct Release(Arc<std::sync::atomic::AtomicBool>);
            impl Drop for Release {
                fn drop(&mut self) {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    self.0.store(false, Ordering::SeqCst);
                }
            }

            if self.held.swap(true, Ordering::SeqCst) {
                self.overlapped.store(true, Ordering::SeqCst);
            }
            let release = Release(self.held.clone());
            tokio::spawn(async move {
                let _release = release;
                std::future::pending::<()>().await;
                Ok(())
            })
        }
    }

    /// A controller restarted starts once the one it replaces is gone, not
    /// while it may still hold the port they share.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_restarted_controller_waits_for_the_old_one_to_let_go() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let overlapped = Arc::new(AtomicBool::new(false));
        let driver = Arc::new(SlowToRelease {
            held: Arc::new(AtomicBool::new(false)),
            overlapped: overlapped.clone(),
        });
        let old = super::Entry::run(driver.clone(), None, None, false, None);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let stopped = super::Entry::stop_all(vec![old]);
        let new = super::Entry::run(driver.clone(), stopped, None, false, None);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(
            !overlapped.load(Ordering::SeqCst),
            "the new listener started while the old one still held the port"
        );
        assert!(
            driver.held.load(Ordering::SeqCst),
            "the new one never started"
        );
        super::Entry::stop_all(vec![new]);
    }

    /// Two updates in quick succession: the second stopped a controller
    /// still waiting for the one the first stopped. A stopped wait ended at
    /// once, so the second's replacement started while the original
    /// listener still held the port. A stopped wait now ends only once what
    /// it waited for is gone.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_controller_restarted_twice_at_once_waits_for_the_first() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let overlapped = Arc::new(AtomicBool::new(false));
        let driver = Arc::new(SlowToRelease {
            held: Arc::new(AtomicBool::new(false)),
            overlapped: overlapped.clone(),
        });
        let old = super::Entry::run(driver.clone(), None, None, false, None);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let stopped = super::Entry::stop_all(vec![old]);
        let waiting = super::Entry::run(driver.clone(), stopped, None, false, None);
        // The second update, before the first's replacement has started.
        let stopped = super::Entry::stop_all(vec![waiting]);
        let new = super::Entry::run(driver.clone(), stopped, None, false, None);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let started = driver.held.load(Ordering::SeqCst);
        super::Entry::stop_all(vec![new]);
        assert!(
            !overlapped.load(Ordering::SeqCst),
            "the second update's listener started while the original still held the port"
        );
        assert!(started, "the second update's listener never started");
    }

    /// A driver stuck in blocking code for a while, which an abort cannot
    /// interrupt, counting the times it was started.
    struct Stuck {
        starts: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl Driver for Stuck {
        fn monitor_events(&self) -> JoinHandle<Result<(), io::Error>> {
            self.starts
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            tokio::spawn(async {
                std::thread::sleep(super::STOP_WAIT * 3);
                Ok(())
            })
        }
    }

    /// A controller replacing one that never ended waited on it for good,
    /// reading "running". It gives up after a while, without starting, and
    /// reads as an error, which the next update retries.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_controller_replacing_one_that_never_ends_reads_as_an_error() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let starts = Arc::new(AtomicUsize::new(0));
        let driver = Arc::new(Stuck {
            starts: starts.clone(),
        });
        let old = super::Entry::run(driver.clone(), None, None, false, None);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let stopped = super::Entry::stop_all(vec![old]);
        let new = super::Entry::run(driver.clone(), stopped, None, false, None);
        let waited = std::time::Instant::now();
        crate::testutil::eventually(
            || new.failure.lock().is_some(),
            "the replacement waited on for good",
        );
        assert!(waited.elapsed() < super::STOP_WAIT * 2);
        assert!(new
            .failure
            .lock()
            .as_deref()
            .is_some_and(|e| e.contains("had not stopped")));
        assert_eq!(starts.load(Ordering::SeqCst), 1, "the replacement started");
    }

    /// A controller whose listener could not bind its port, which happens on
    /// its task after it has started, read as running, and so was never
    /// retried. It reads as the error, and the next update starts it again.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_controller_that_cannot_bind_its_port_is_retried() -> Result<(), Box<dyn Error>> {
        let player = make_controllerless_player().await?;
        let taken = std::net::TcpListener::bind("0.0.0.0:0")?;
        let port = taken.local_addr()?.port();
        let grpc = || config::Controller::Grpc(config::GrpcController::new(port));

        let mut controller = super::Controller::new(vec![grpc()], player.clone());
        crate::testutil::eventually(
            || controller.statuses()[0].status == "error",
            "a controller that could not bind its port reads as running",
        );
        assert!(controller.statuses()[0].error.is_some());

        drop(taken);
        assert!(
            controller.update(vec![grpc()], player.clone(), false),
            "the failed controller was not retried"
        );
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        assert_eq!(controller.statuses()[0].status, "running");
        assert!(!controller.entries[0]
            .handle
            .as_ref()
            .expect("started")
            .is_finished());
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
