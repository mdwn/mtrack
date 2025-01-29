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
    any::Any,
    error::Error,
    fmt,
    sync::{Arc, Barrier},
};

use midly::live::LiveEvent;
use tokio::sync::mpsc::Sender;

use crate::{config, playsync::CancelHandle, songs::Song};

pub(crate) mod midir;
mod mock;

/// A MIDI device that can play MIDI files and listen for inputs.
pub trait Device: Any + fmt::Display + std::marker::Send + std::marker::Sync {
    /// Watches MIDI input for events and sends them to the given sender.
    fn watch_events(&self, sender: Sender<Vec<u8>>) -> Result<(), Box<dyn Error>>;

    /// Stops watching events.
    fn stop_watch_events(&self);

    /// Plays the given song through the MIDI interface.
    fn play(
        &self,
        song: Arc<Song>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
    ) -> Result<(), Box<dyn Error>>;

    /// Emits an event.
    fn emit(&self, midi_event: Option<LiveEvent<'static>>) -> Result<(), Box<dyn Error>>;

    #[cfg(test)]
    fn to_mock(&self) -> Result<Arc<mock::Device>, Box<dyn Error>>;
}

/// Lists devices known to midir.
pub fn list_devices() -> Result<Vec<Box<dyn Device>>, Box<dyn Error>> {
    midir::list()
}

/// Gets a device with the given name.
pub fn get_device(config: Option<config::Midi>) -> Result<Option<Arc<dyn Device>>, Box<dyn Error>> {
    let config = match config {
        Some(config) => config,
        None => return Ok(None),
    };

    let device = config.device();
    if device.starts_with("mock") {
        return Ok(Some(Arc::new(mock::Device::get(device))));
    };

    Ok(Some(Arc::new(midir::get(&config)?)))
}

#[cfg(test)]
pub mod test {
    pub use super::mock::Device;
}
