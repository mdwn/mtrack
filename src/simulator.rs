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

pub mod server;
pub mod state;
pub mod watcher;

use parking_lot::Mutex;
use std::sync::Arc;
use tokio::sync::{broadcast, watch};
use tracing::info;

use crate::state::StateSnapshot;

/// Configuration for the lighting simulator.
#[derive(Debug, Clone)]
pub struct SimulatorConfig {
    pub port: u16,
    pub address: String,
}

impl Default for SimulatorConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            address: "127.0.0.1".to_string(),
        }
    }
}

/// Handle to a running simulator, returned so callers can shut it down.
///
/// When dropped, the server shutdown signal is sent and the sampler task is aborted.
pub struct SimulatorHandle {
    _shutdown_tx: tokio::sync::oneshot::Sender<()>,
    sampler_handle: tokio::task::JoinHandle<()>,
    server_handle: tokio::task::JoinHandle<()>,
    /// The broadcast channel sender, exposed so the DmxEngine can pass it to the file watcher.
    pub broadcast_tx: broadcast::Sender<String>,
}

impl Drop for SimulatorHandle {
    fn drop(&mut self) {
        self.sampler_handle.abort();
        self.server_handle.abort();
    }
}

/// Starts the simulator server as a tokio task.
///
/// Returns a handle that shuts down the server when dropped. The handle's `broadcast_tx`
/// should be passed to the DmxEngine via `set_simulator_broadcast_tx` so the file watcher
/// can send reload notifications through the same WebSocket channel.
pub async fn start(
    config: SimulatorConfig,
    state_rx: watch::Receiver<Arc<StateSnapshot>>,
    lighting_system: Option<Arc<Mutex<crate::lighting::system::LightingSystem>>>,
) -> Result<SimulatorHandle, Box<dyn std::error::Error + Send + Sync>> {
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let (broadcast_tx, _) = broadcast::channel::<String>(64);

    // Build metadata JSON once from the lighting system
    let metadata_json = state::build_metadata_json(lighting_system.as_ref());

    // Spawn the watch→broadcast bridge
    let sampler_tx = broadcast_tx.clone();
    let sampler_handle = tokio::spawn(state::sampler_loop(state_rx, sampler_tx));

    // Spawn the HTTP/WS server
    let port = config.port;
    let address = config.address;
    info!(port, %address, "Starting lighting simulator");
    let server_handle = tokio::spawn(server::run(
        broadcast_tx.clone(),
        metadata_json,
        address,
        port,
        shutdown_rx,
    ));

    Ok(SimulatorHandle {
        _shutdown_tx: shutdown_tx,
        sampler_handle,
        server_handle,
        broadcast_tx,
    })
}
