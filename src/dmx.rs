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
use engine::Engine;
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

    Ok(Some(Arc::new(Engine::new(
        config,
        lighting_config,
        base_path,
    )?)))
}
