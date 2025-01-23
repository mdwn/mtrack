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

use serde::Deserialize;

use crate::dmx::universe::UniverseConfig;

/// A YAML representation of the DMX configuration.
#[derive(Deserialize)]
pub(super) struct Dmx {
    /// Controls the dim speed modifier. A modifier of 1.0 means a dim speed of 1 == 1.0 second.
    dim_speed_modifier: Option<f64>,

    /// The configuration of devices to universes.
    universes: Vec<Universe>,
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
