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
use std::{
    error::Error,
    fmt,
    sync::{Arc, Barrier},
};

use crate::{playsync::CancelHandle, songs::Song};

mod dmxengine;

/// A DMX device that can play MIDI files as DMX.
pub trait Device: fmt::Display + std::marker::Send + std::marker::Sync {
    /// Plays the given file through the DMX interface interface.
    fn play(
        &self,
        song: Arc<Song>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
    ) -> Result<(), Box<dyn Error>>;
}

/// Gets a device with the given name.
pub fn get_device() -> Arc<dyn Device> {
    Arc::new(dmxengine::get())
}
