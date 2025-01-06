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

use std::collections::HashMap;

use serde::Deserialize;

use crate::dmx::universe::UniverseConfig;

/// A YAML representation of the DMX configuration.
#[derive(Deserialize)]
pub(super) struct Dmx {
    /// Controls the dim speed modifier. A modifier of 1.0 means a dim speed of 1 == 1.0 second.
    dim_speed_modifier: Option<f64>,

    /// The configuration of devices to universes.
    universes: Vec<Universe>,

    /// MIDI input to DMX channels.
    midi_input_to_channels: HashMap<String, MidiInputToChannels>,
}

impl Dmx {
    /// Gets the dimming speed modifier.
    pub(super) fn get_dimming_speed_modifier(&self) -> Option<f64> {
        self.dim_speed_modifier
    }

    /// Converts the configuration into universe configs.
    pub(super) fn to_configs(&self) -> Vec<UniverseConfig> {
        self.universes
            .iter()
            .map(|u| u.to_universe_config())
            .collect()
    }

    pub(super) fn to_midi_input_to_channels(
        &self,
    ) -> HashMap<String, crate::dmx::engine::MidiInputToChannels> {
        self.midi_input_to_channels
            .iter()
            .map(|(universe_name, midi_input_to_channels)| {
                (
                    universe_name.clone(),
                    midi_input_to_channels.to_midi_input_channels(),
                )
            })
            .collect()
    }
}

/// A YAML representation of a DMX universe configuration.
#[derive(Deserialize)]
pub(super) struct Universe {
    /// The OpenLighting universe.
    universe: u16,

    /// The name of this universe. Will be mapped to a universe by the player.
    name: String,
}

impl Universe {
    /// Converts this universe configuration into a UniverseConfiguration object.
    pub(super) fn to_universe_config(&self) -> UniverseConfig {
        UniverseConfig {
            universe: self.universe,
            name: self.name.clone(),
        }
    }
}

/// A YAML representation of a mapping of MIDI input to DMX channels.
#[derive(Deserialize)]
pub(super) struct MidiInputToChannels {
    // The MIDI channel to monitor.
    channel: u8,

    // The MIDI input key value or CC value.
    source: u8,

    // The DMX channels that should be signaled based on the incoming value.
    target_dmx_channels: Vec<u8>,
}

impl MidiInputToChannels {
    pub(super) fn to_midi_input_channels(&self) -> crate::dmx::engine::MidiInputToChannels {
        crate::dmx::engine::MidiInputToChannels::new(
            self.channel,
            self.source,
            self.target_dmx_channels.clone(),
        )
    }
}
