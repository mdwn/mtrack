// Copyright (C) 2026 Michael Wilson <mike@mdwn.dev>
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

//! A profile's `dmx:` section as written: light, so the web UI's WebAssembly module compiles it as it is
//! and the UI edits it by the generated types. What it means is `dmx`'s.

use super::lighting_data::Lighting;
use serde::{Deserialize, Serialize};

/// A YAML representation of the DMX configuration.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone)]
pub struct Dmx {
    /// Controls the dim speed modifier. A modifier of 1.0 means a dim speed of 1 == 1.0 second.
    #[serde(default)]
    pub(super) dim_speed_modifier: Option<f64>,

    /// Controls how long to wait before playback of a DMX lighting file starts.
    #[serde(default)]
    pub(super) playback_delay: Option<String>,

    /// The OLA port. Defaults to the default OLA port.
    #[serde(default)]
    pub(super) ola_port: Option<u16>,

    /// The port of olad's web server, used only to check that the configured
    /// universes have an output port patched to them. Defaults to olad's.
    #[serde(default)]
    pub(super) ola_http_port: Option<u16>,

    /// The configuration of devices to universes.
    pub(super) universes: Vec<Universe>,

    /// The lighting configuration.
    #[serde(default)]
    pub(super) lighting: Option<Lighting>,

    /// When true, use a no-op OLA client if the OLA daemon is unavailable.
    /// Allows the effects engine to run (e.g. for TUI display) without OLA.
    #[serde(default)]
    pub(super) null_client: bool,
}

/// A YAML representation of a DMX universe configuration.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone)]
pub struct Universe {
    /// The OpenLighting universe.
    pub(super) universe: u16,

    /// The name of this universe. Will be mapped to a universe by the player.
    pub(super) name: String,
}
