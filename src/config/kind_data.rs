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

//! Which kind of config file a YAML file is: light, so the web UI's WebAssembly module compiles it as it is
//! and the UI edits it by the generated types. What it means is `kind`'s.

use serde::{Deserialize, Serialize};

/// Identifies the type of an mtrack YAML configuration file.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConfigKind {
    Song,
    Playlist,
    #[serde(rename = "hardware_profile")]
    HardwareProfile,
}
