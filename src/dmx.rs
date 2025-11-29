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

pub mod engine;
pub mod ola_client;
pub mod universe;

use crate::config;
use crate::dmx::ola_client::OlaClientFactory;
use engine::Engine;
use ola::client::StreamingClientConfig;
use std::thread;
use std::time::Duration;
use std::{error::Error, path::Path, sync::Arc};

/// Gets a device with the given name.
pub fn create_engine(
    config: Option<&config::Dmx>,
    base_path: Option<&Path>,
) -> Result<Option<Arc<Engine>>, Box<dyn Error>> {
    let config = match config {
        Some(config) => config,
        None => return Ok(None),
    };

    // Use the lighting config from the DMX config if available
    let lighting_config = config.lighting();

    // Build a real OLA client and construct the engine
    let ola_client_config = StreamingClientConfig {
        server_port: config.ola_port(),
        ..Default::default()
    };
    // Retry connecting to OLA a few times with backoff
    let ola_client = {
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
            (None, Some(e)) => return Err(e),
            (None, None) => unreachable!(),
        }
    };

    let engine = Arc::new(Engine::new(config, lighting_config, base_path, ola_client)?);

    // Start the persistent effects loop
    Engine::start_persistent_effects_loop(engine.clone());

    Ok(Some(engine))
}
