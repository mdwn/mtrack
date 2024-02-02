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

use tokio::sync::mpsc::Sender;

use crate::{playsync::CancelHandle, songs::Song};

mod midir;
mod mock;

/// A MIDI device that can play MIDI files and listen for inputs.
pub trait Device: fmt::Display + std::marker::Send + std::marker::Sync {
    /// Returns the name of the device.
    fn name(&self) -> String;

    /// Watches MIDI input for events and sends them to the given sender.
    fn watch_events(&self, sender: Sender<Vec<u8>>) -> Result<(), Box<dyn Error>>;

    /// Stops watching events.
    fn stop_watch_events(&self);

    /// Plays the given song through the MIDI interface.
    fn play(&self, song: Arc<Song>, cancel_handle: CancelHandle) -> Result<(), Box<dyn Error>>;

    /// Emits an event.
    fn emit(&self, song: Arc<Song>) -> Result<(), Box<dyn Error>>;
}

/// Lists devices known to midir.
pub fn list_devices() -> Result<Vec<Box<dyn Device>>, Box<dyn Error>> {
    midir::list()
}

/// Gets a device with the given name.
pub fn get_device(name: &String) -> Result<Arc<dyn Device>, Box<dyn Error>> {
    if name.starts_with("mock") {
        return Ok(Arc::new(mock::Device::get(name)));
    };

    Ok(Arc::new(midir::get(name)?))
}

#[cfg(test)]
pub mod test {
    pub use super::mock::Device;
}
