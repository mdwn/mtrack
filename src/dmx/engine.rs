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

use std::sync::{RwLock};

use midly::num::u7;
use tracing::info;

/// We only support 1 universe for now.
const SUPPORTED_UNIVERSES: u8 = 1;

/// The DMX engine. This is meant to control the current state of the
/// universe(s) that should be sent to our DMX interface(s).
pub struct Engine {
    universes: Vec<RwLock<Vec<u8>>>,
    max_channels_seen: Vec<u16>,
}

impl Engine {
    /// Creates a new DMX Engine.
    pub fn new() -> Engine {
        Engine{
            universes: (0..SUPPORTED_UNIVERSES).map(|_| {RwLock::new(vec![0; UNIVERSE_SIZE])}).collect(),
            max_channels_seen: vec![0; UNIVERSE_SIZE],
        }
    }

    /// Handles an incoming MIDI event.
    pub fn handle_midi_event(&mut self, midi_message: midly::MidiMessage) {
        match midi_message {
            midly::MidiMessage::NoteOn { key, vel } => {
                self.handle_key_velocity(0, key, vel);
            }
            midly::MidiMessage::NoteOff { key, vel } => {
                self.handle_key_velocity(0, key, vel);
            }
            _ => {
                info!(
                    midi_event = format!("{:?}", midi_message),
                    "Unrecognized MIDI event"
                );
            }
        }
    }

    /// Gets the given universe.
    pub fn get_universe(&self, universe_number: usize) -> Vec<u8> {
        self.universes
        .get(universe_number).expect(format!("Universe {} not expected", universe_number).as_str())
        .read().expect(format!("Unable to get lock for universe {}", universe_number).as_str()).clone()
    }

    /// Handles MIDI events that use a key and velocity.
    fn handle_key_velocity(&mut self, universe_number: usize, key: u7, velocity: u7) {
        return self.update_universe(universe_number, key.as_int(), velocity.as_int()*2)
    }

}