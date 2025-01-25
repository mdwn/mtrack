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
pub const DEFAULT_DMX_DIMMING_SPEED_MODIFIER: f64 = 1.0;
pub const DEFAULT_DMX_PLAYBACK_DELAY: Duration = Duration::ZERO;

/// A YAML representation of the DMX configuration.
#[derive(Deserialize, Clone)]
pub(crate) struct Dmx {
    /// Controls the dim speed modifier. A modifier of 1.0 means a dim speed of 1 == 1.0 second.
    dim_speed_modifier: Option<f64>,

    /// Controls how long to wait before playback of a DMX lighting file starts.
    playback_delay: Option<String>,

    /// The configuration of devices to universes.
    universes: Vec<Universe>,
}

impl Dmx {
    /// Creates a new DMX configuration.
    pub(crate) fn new(
        dim_speed_modifier: Option<f64>,
        playback_delay: Option<String>,
        universes: Vec<Universe>,
    ) -> Dmx {
        Dmx {
            dim_speed_modifier,
            playback_delay,
            universes,
        }
    }
    /// Gets the dimming speed modifier.
    pub(crate) fn dimming_speed_modifier(&self) -> f64 {
        self.dim_speed_modifier
            .unwrap_or(DEFAULT_DMX_DIMMING_SPEED_MODIFIER)
    }

    /// Gets the playback delay.
    pub(crate) fn playback_delay(&self) -> Result<Duration, duration_string::Error> {
        self.playback_delay
            .as_ref()
            .map_or(Ok(DEFAULT_DMX_PLAYBACK_DELAY), |duration| {
                Ok(DurationString::from_string(duration.clone())?.into())
            })
    }

    /// Converts the configuration into universe configs.
    pub(crate) fn universes(&self) -> Vec<Universe> {
        self.universes.clone()
    }
}

/// A YAML representation of a DMX universe configuration.
#[derive(Deserialize, Clone)]
pub(crate) struct Universe {
    /// The OpenLighting universe.
    universe: u16,

    /// The name of this universe. Will be mapped to a universe by the player.
    name: String,
}

impl Universe {
    /// Creates a new universe configuration.
    pub(crate) fn new(universe: u16, name: String) -> Universe {
        Universe { universe, name }
    }

    /// Gets the OpenLighting universe.
    pub(crate) fn universe(&self) -> u16 {
        self.universe
    }

    /// Gets the name of the universe.
    pub(crate) fn name(&self) -> String {
        self.name.clone()
    }
}
