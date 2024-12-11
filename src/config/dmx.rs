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

use serde::Deserialize;

use crate::dmx::universe::UniverseConfig;

/// A YAML representation of the DMX configuration.
#[derive(Deserialize)]
pub(super) struct DMX {
    /// The configuration of devices to universes.
    universes: Vec<Universe>,
}

impl DMX {
    /// Converts the configuration into universe configs.
    pub(super) fn to_configs(&self) -> Vec<UniverseConfig> {
        self.universes
            .iter()
            .map(|u| u.to_universe_config())
            .collect()
    }
}

/// A YAML representation of a DMX universe configuration.
#[derive(Deserialize)]
pub(super) struct Universe {
    /// The serial device that corresponds to this universe.
    device: String,

    /// Whether or not this is an FTDI device (ENTTEC USB DMX Pro).
    ftdi: bool,
}

impl Universe {
    /// Converts this universe configuration into a UniverseConfiguration object.
    pub(super) fn to_universe_config(&self) -> UniverseConfig {
        UniverseConfig {
            device: self.device.clone(),
            ftdi: self.ftdi,
        }
    }
}
