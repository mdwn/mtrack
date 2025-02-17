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

use std::time::Duration;

use duration_string::DurationString;
use serde::Deserialize;

/// The default DMX dimming speed.
pub const DEFAULT_OLA_PORT: u16 = 9010;
pub const DEFAULT_DMX_DIMMING_SPEED_MODIFIER: f64 = 1.0;
pub const DEFAULT_DMX_PLAYBACK_DELAY: Duration = Duration::ZERO;

/// A YAML representation of the DMX configuration.
#[derive(Deserialize, Clone)]
pub struct Dmx {
    /// Controls the dim speed modifier. A modifier of 1.0 means a dim speed of 1 == 1.0 second.
    dim_speed_modifier: Option<f64>,

    /// Controls how long to wait before playback of a DMX lighting file starts.
    playback_delay: Option<String>,

    /// The OLA port. Defaults to the default OLA port.
    ola_port: Option<u16>,

    /// The configuration of devices to universes.
    universes: Vec<Universe>,
}

impl Dmx {
    /// Creates a new DMX configuration.
    pub fn new(
        dim_speed_modifier: Option<f64>,
        playback_delay: Option<String>,
        ola_port: Option<u16>,
        universes: Vec<Universe>,
    ) -> Dmx {
        Dmx {
            dim_speed_modifier,
            playback_delay,
            ola_port,
            universes,
        }
    }
    /// Gets the dimming speed modifier.
    pub fn dimming_speed_modifier(&self) -> f64 {
        self.dim_speed_modifier
            .unwrap_or(DEFAULT_DMX_DIMMING_SPEED_MODIFIER)
    }

    /// Gets the playback delay.
    pub fn playback_delay(&self) -> Result<Duration, duration_string::Error> {
        self.playback_delay
            .as_ref()
            .map_or(Ok(DEFAULT_DMX_PLAYBACK_DELAY), |duration| {
                Ok(DurationString::from_string(duration.clone())?.into())
            })
    }

    /// Gets the OLA port to use.
    pub fn ola_port(&self) -> u16 {
        self.ola_port.unwrap_or(DEFAULT_OLA_PORT)
    }

    /// Converts the configuration into universe configs.
    pub fn universes(&self) -> Vec<Universe> {
        self.universes.clone()
    }
}

/// A YAML representation of a DMX universe configuration.
#[derive(Deserialize, Clone)]
pub struct Universe {
    /// The OpenLighting universe.
    universe: u16,

    /// The name of this universe. Will be mapped to a universe by the player.
    name: String,
}

impl Universe {
    /// Creates a new universe configuration.
    pub fn new(universe: u16, name: String) -> Universe {
        Universe { universe, name }
    }

    /// Gets the OpenLighting universe.
    pub fn universe(&self) -> u16 {
        self.universe
    }

    /// Gets the name of the universe.
    pub fn name(&self) -> &str {
        &self.name
    }
}
