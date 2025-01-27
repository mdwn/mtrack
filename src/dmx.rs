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
use std::{
    error::Error,
    sync::{Arc, RwLock},
};

use engine::Engine;
use universe::Universe;

use crate::config;

pub mod engine;
pub mod universe;

/// Gets a device with the given name.
pub fn create_engine(
    config: Option<&config::Dmx>,
) -> Result<Option<Arc<RwLock<Engine>>>, Box<dyn Error>> {
    let config = match config {
        Some(config) => config,
        None => return Ok(None),
    };
    Ok(Some(Arc::new(RwLock::new(Engine::new(config)?))))
}
