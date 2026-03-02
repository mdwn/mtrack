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

pub mod engine;
pub mod legacy_store;
pub mod ola_client;
pub mod universe;
pub mod watcher;

use crate::config;
use crate::dmx::ola_client::OlaClientFactory;
use engine::Engine;
#[cfg(not(test))]
use ola::client::StreamingClientConfig;
#[cfg(not(test))]
use std::thread;
#[cfg(not(test))]
use std::time::Duration;
use std::{error::Error, path::Path, sync::Arc};
use tracing::info;

/// Creates a DMX engine, connecting to the OLA daemon for output.
/// Falls back to a no-op OLA client if the OLA daemon is unavailable, so the
/// lighting/effects engine can still run without physical hardware (web UI, etc.).
pub fn create_engine(
    config: Option<&config::Dmx>,
    base_path: Option<&Path>,
) -> Result<Option<Arc<Engine>>, Box<dyn Error>> {
    create_engine_inner(config, base_path, true)
}

fn create_engine_inner(
    config: Option<&config::Dmx>,
    base_path: Option<&Path>,
    allow_null_client: bool,
) -> Result<Option<Arc<Engine>>, Box<dyn Error>> {
    let config = match config {
        Some(config) => config,
        None => return Ok(None),
    };

    // Use the lighting config from the DMX config if available
    let lighting_config = config.lighting();

    // Build a real OLA client and construct the engine
    // In test mode, use a mock client to avoid hanging on OLA connection
    #[cfg(test)]
    let ola_client = {
        let _ = allow_null_client; // Only used in non-test builds
        OlaClientFactory::create_mock_client_unconditional()
    };

    #[cfg(not(test))]
    let ola_client = if config.null_client() {
        info!("null_client enabled, skipping OLA connection");
        Box::new(ola_client::NullOlaClient) as Box<dyn ola_client::OlaClient>
    } else {
        let ola_client_config = StreamingClientConfig {
            server_port: config.ola_port(),
            auto_start: false,
        };
        // Retry connecting to OLA a few times with backoff
        let mut last_err: Option<Box<dyn Error>> = None;
        let mut found: Option<Box<dyn ola_client::OlaClient>> = None;
        for attempt in 0..10 {
            if attempt > 0 {
                thread::sleep(Duration::from_secs(5));
            }
            match OlaClientFactory::create_real_client(ola_client_config.clone()) {
                Ok(client) => {
                    found = Some(client);
                    break;
                }
                Err(e) => {
                    last_err = Some(e);
                }
            }
        }
        match (found, last_err) {
            (Some(client), _) => client,
            (None, Some(e)) => {
                if allow_null_client {
                    info!("OLA not available, using null DMX client");
                    Box::new(ola_client::NullOlaClient) as Box<dyn ola_client::OlaClient>
                } else {
                    return Err(e);
                }
            }
            (None, None) => unreachable!(),
        }
    };

    let engine = Arc::new(Engine::new(config, lighting_config, base_path, ola_client)?);

    info!(
        lighting = lighting_config.is_some(),
        "DMX engine initialized"
    );

    // Start the persistent effects loop
    Engine::start_persistent_effects_loop(engine.clone());

    Ok(Some(engine))
}
