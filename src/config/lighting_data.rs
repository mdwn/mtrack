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

//! A profile's `dmx.lighting:` section as written: light, so the web UI's WebAssembly module compiles it as it is
//! and the UI edits it by the generated types. What it means is `lighting`'s.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Group constraint types for role-based group resolution
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone, Debug)]
pub enum GroupConstraint {
    /// All of these tags must be present
    AllOf(Vec<String>),
    /// Any of these tags must be present
    AnyOf(Vec<String>),
    /// Prefer fixtures with these tags
    Prefer(Vec<String>),
    /// Minimum number of fixtures required
    MinCount(usize),
    /// Maximum number of fixtures allowed
    MaxCount(usize),
    /// Fallback to this group if primary group fails
    FallbackTo(String),
    /// Allow group to be empty if no fixtures match (graceful degradation)
    AllowEmpty(bool),
}

/// Group definition with role-based constraints
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct LogicalGroup {
    /// The name of the group
    pub(super) name: String,
    /// Constraints for resolving this group
    pub(super) constraints: Vec<GroupConstraint>,
}

/// Where a project's fixture types live, relative to the project, when the
/// config does not say: `lighting.directories.fixture_types` moves it.
pub const DEFAULT_FIXTURE_TYPES_DIR: &str = "lighting/fixture_types";

/// Where a project's venues live, relative to the project, when the config
/// does not say: `lighting.directories.venues` moves it.
pub const DEFAULT_VENUES_DIR: &str = "lighting/venues";

/// A YAML representation of the lighting configuration.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Deserialize, Serialize, Clone)]
pub struct Lighting {
    /// The current venue selection.
    #[serde(default)]
    pub(super) current_venue: Option<String>,

    /// The retired inline fixture map (`fixtures:`), read only so the
    /// loader can refuse it by name and `mtrack migrate` can move it into a
    /// venue file. Never written back.
    #[serde(rename = "fixtures", default, skip_serializing)]
    pub(super) retired_fixtures: Option<HashMap<String, String>>,

    /// Logical group definitions with role-based constraints.
    #[serde(default)]
    pub(super) groups: Option<HashMap<String, LogicalGroup>>,

    /// Where fixture types and venues are read from, when not the defaults
    /// ([`DEFAULT_FIXTURE_TYPES_DIR`], [`DEFAULT_VENUES_DIR`]). Each is
    /// independent: naming one leaves the other at its default.
    #[serde(default)]
    pub(super) directories: Option<Directories>,

    /// The show the rig plays while no song is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) ambient: Option<super::ambient::Ambient>,
}

/// Directory configuration for loading fixture types and venues.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone)]
pub struct Directories {
    /// Directory containing fixture type definitions.
    #[serde(default)]
    pub(super) fixture_types: Option<String>,

    /// Directory containing venue definitions.
    #[serde(default)]
    pub(super) venues: Option<String>,
}
