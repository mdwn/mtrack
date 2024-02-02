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
use std::{error::Error, sync::Arc};

use serde::Deserialize;

use crate::controller::Driver;

use super::midi::{self, ToMidiEvent};

/// Allows users to specify various controllers.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub(super) enum Controller {
    Midi(MidiController),
    Keyboard,
}

impl Controller {
    /// Creates a controller driver from the config.
    pub(super) fn driver(
        &self,
        midi_device: Option<Arc<dyn crate::midi::Device>>,
    ) -> Result<Arc<dyn Driver>, Box<dyn Error>> {
        match self {
            Controller::Midi(config) => match midi_device {
                Some(midi_device) => Ok(Arc::new(crate::controller::midi::Driver::new(
                    midi_device,
                    config.play.to_midi_event()?,
                    config.prev.to_midi_event()?,
                    config.next.to_midi_event()?,
                    config.stop.to_midi_event()?,
                    config.all_songs.to_midi_event()?,
                    config.playlist.to_midi_event()?,
                ))),
                None => Err("No MIDI device found for MIDI controller.".into()),
            },
            Controller::Keyboard => Ok(Arc::new(crate::controller::keyboard::Driver::new())),
        }
    }
}
#[derive(Deserialize)]
pub(super) struct KeyboardController {}

/// The configuration that maps MIDI events to controller messages.
#[derive(Deserialize)]
pub(super) struct MidiController {
    /// The MIDI event to look for to play the current song in the playlist.
    play: midi::Event,
    /// The MIDI event to look for to move the playlist to the previous item.
    prev: midi::Event,
    /// The MIDI event to look for to move the playlist to the next item.
    next: midi::Event,
    /// The MIDI event to look for to stop playback.
    stop: midi::Event,
    /// The MIDI event to look for to switch from the current playlist to an all songs playlist.
    all_songs: midi::Event,
    /// The MIDI event to look for to switch back to the current playlist.
    playlist: midi::Event,
}
