// Copyright (C) 2024 Michael Wilson <mike@mdwn.dev>
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

use crate::playsync::CancelHandle;
use crate::songs::Song;
use std::collections::HashMap;
use std::sync::Barrier;

mod cpal;
mod mock;

/// An audio device that can play songs back.
pub trait Device: fmt::Display + std::marker::Send + std::marker::Sync {
    /// Plays the given song through the audio interface.
    fn play(
        &self,
        song: Arc<Song>,
        mappings: &HashMap<String, u16>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
    ) -> Result<(), Box<dyn Error>>;
}

/// Lists devices known to cpal.
pub fn list_devices() -> Result<Vec<Box<dyn Device>>, Box<dyn Error>> {
    cpal::Device::list()
}

/// Gets a device with the given name.
pub fn get_device(name: &String) -> Result<Arc<dyn Device>, Box<dyn Error>> {
    if name.starts_with("mock") {
        return Ok(Arc::new(mock::Device::get(name)));
    };

    Ok(Arc::new(cpal::Device::get(name)?))
}

#[cfg(test)]
pub mod test {
    // Reexport the mock device direclty for testing.
    pub use super::mock::Device;
}
