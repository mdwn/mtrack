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
use std::error::Error;

use midly::live::LiveEvent;
use serde::Deserialize;

use self::midi::ToMidiEvent;

use super::midi;

/// The configuration for emitting status events.
#[derive(Deserialize, Clone)]
pub struct StatusEvents {
    /// The events to emit to clear the status.
    off_events: Vec<midi::Event>,
    /// The events to emit to indicate that the player is idling and waiting for input.
    idling_events: Vec<midi::Event>,
    /// The events to emit to indicate that the player is currently playing.
    playing_events: Vec<midi::Event>,
}

impl StatusEvents {
    /// Gets the off events.
    pub fn off_events(&self) -> Result<Vec<LiveEvent<'static>>, Box<dyn Error>> {
        self.off_events
            .iter()
            .map(|event| event.to_midi_event())
            .collect()
    }

    /// Gets the idling events.
    pub fn idling_events(&self) -> Result<Vec<LiveEvent<'static>>, Box<dyn Error>> {
        self.idling_events
            .iter()
            .map(|event| event.to_midi_event())
            .collect()
    }

    /// Gets the playing events.
    pub fn playing_events(&self) -> Result<Vec<LiveEvent<'static>>, Box<dyn Error>> {
        self.playing_events
            .iter()
            .map(|event| event.to_midi_event())
            .collect()
    }
}
