use std::any::Any;
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
use std::{error::Error, fmt, sync::Arc};

use crate::config;
use crate::playsync::CancelHandle;
use crate::songs::Song;
use std::collections::HashMap;
use std::sync::Barrier;

pub mod cpal;
pub mod format;
pub mod mixer;
pub mod mock;
pub mod sample_source;

// Re-export the format types for backward compatibility
pub use format::{SampleFormat, TargetFormat};

pub trait Device: Any + fmt::Display + std::marker::Send + std::marker::Sync {
    /// Plays the given song through the audio interface.
    fn play(
        &self,
        song: Arc<Song>,
        mappings: &HashMap<String, Vec<u16>>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
    ) -> Result<(), Box<dyn Error>>;

    #[cfg(test)]
    fn to_mock(&self) -> Result<Arc<mock::Device>, Box<dyn Error>>;
}

/// Lists devices known to cpal.
pub fn list_devices() -> Result<Vec<Box<dyn Device>>, Box<dyn Error>> {
    cpal::Device::list()
}

/// Gets a device with the given name.
pub fn get_device(config: Option<config::Audio>) -> Result<Arc<dyn Device>, Box<dyn Error>> {
    let config = match config {
        Some(config) => config,
        None => return Err("there must be an audio device specified".into()),
    };

    let device = config.device();
    if device.starts_with("mock") {
        return Ok(Arc::new(mock::Device::get(device)));
    };

    Ok(Arc::new(cpal::Device::get(config)?))
}
