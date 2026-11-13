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

//! The watch hub: the typed feeds the gRPC `Watch` stream reads.
//!
//! The pollers that watch the player (its playback, its log, the current
//! song's waveform, its config store) and the light show watcher send what
//! they find as [`SocketMessage`]s on one broadcast channel. `Watch` maps
//! each to its proto message. The hub also owns the waveform cache, which the pollers fill
//! and the surfaces send a new client from.
//!
//! The hub belongs to the [`Player`], which both servers hold, so a player
//! with only one of them configured (or with the web server unable to
//! start) feeds it all the same.

use std::sync::Arc;

use tokio::sync::broadcast;
use tokio::task::JoinHandle;

use super::state::{self, WaveformCache};
use super::wire::messages::SocketMessage;
use crate::player::Player;

/// A song's tracks' peaks: each track's name and its peaks.
pub type TrackPeaks = Vec<(String, Vec<f32>)>;

/// How many messages a slow reader may fall behind by before it skips.
const CAPACITY: usize = 128;

/// The typed feeds and the waveform cache. Cloning shares them.
#[derive(Clone)]
pub struct WatchHub {
    tx: broadcast::Sender<SocketMessage>,
    waveform_cache: WaveformCache,
}

impl Default for WatchHub {
    fn default() -> Self {
        Self::new()
    }
}

impl WatchHub {
    /// A hub with no readers and an empty cache.
    pub fn new() -> Self {
        WatchHub {
            tx: broadcast::channel(CAPACITY).0,
            waveform_cache: state::new_waveform_cache(),
        }
    }

    /// The sending end, for whatever reports on the feeds.
    pub fn sender(&self) -> broadcast::Sender<SocketMessage> {
        self.tx.clone()
    }

    /// A new reader of the feeds, from now on. A reader that falls behind
    /// is told how many it missed and carries on.
    pub fn subscribe(&self) -> broadcast::Receiver<SocketMessage> {
        self.tx.subscribe()
    }

    /// The waveform cache: song name to its tracks' peaks.
    pub fn waveform_cache(&self) -> &WaveformCache {
        &self.waveform_cache
    }

    /// The cached waveform of the song the playlist is on: its name and
    /// its tracks' peaks. `None` with no current song or none cached, which
    /// is when a new reader is sent nothing and waits for the poller.
    pub fn cached_waveform(&self, player: &Player) -> Option<(String, TrackPeaks)> {
        let song_name = player.get_playlist().current()?.name().to_string();
        let peaks = self.waveform_cache.lock().get(&song_name).cloned()?;
        Some((song_name, peaks))
    }

    /// Starts the pollers that feed the hub, and the pre-warm of the
    /// waveform cache. They stop when the returned handle is dropped.
    pub fn start(&self, player: Arc<Player>) -> HubTasks {
        HubTasks(vec![
            tokio::spawn(state::playback_poller(player.clone(), self.tx.clone())),
            tokio::spawn(state::log_poller(self.tx.clone())),
            tokio::spawn(state::waveform_poller(
                player.clone(),
                self.tx.clone(),
                self.waveform_cache.clone(),
            )),
            tokio::spawn(state::waveform_prewarmer(
                player.clone(),
                self.waveform_cache.clone(),
            )),
            tokio::spawn(state::config_watcher(player, self.tx.clone())),
        ])
    }
}

/// The tasks [`WatchHub::start`] spawned, aborted when dropped.
pub struct HubTasks(Vec<JoinHandle<()>>);

impl Drop for HubTasks {
    fn drop(&mut self) {
        for task in &self.0 {
            task.abort();
        }
    }
}
