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
use std::collections::HashMap;

/// Group constraint types for role-based group resolution
#[derive(Deserialize, Clone, Debug)]
#[allow(dead_code)]
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
#[derive(Deserialize, Clone, Debug)]
#[allow(dead_code)]
pub struct LogicalGroup {
    /// The name of the group
    name: String,
    /// Constraints for resolving this group
    constraints: Vec<GroupConstraint>,
}

impl LogicalGroup {
    #[allow(dead_code)]
    pub fn new(name: String, constraints: Vec<GroupConstraint>) -> Self {
        Self { name, constraints }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn constraints(&self) -> &Vec<GroupConstraint> {
        &self.constraints
    }
}

/// A YAML representation of the lighting configuration.
#[derive(Deserialize, Clone)]
#[allow(dead_code)]
pub struct Lighting {
    /// The current venue selection.
    current_venue: Option<String>,

    /// Simple fixture definitions (inline).
    fixtures: Option<HashMap<String, String>>,

    /// Logical group definitions with role-based constraints.
    groups: Option<HashMap<String, LogicalGroup>>,

    /// Directory paths for loading fixture types and venues.
    directories: Option<Directories>,
}

/// Directory configuration for loading fixture types and venues.
#[derive(Deserialize, Clone)]
#[allow(dead_code)]
pub struct Directories {
    /// Directory containing fixture type definitions.
    fixture_types: Option<String>,

    /// Directory containing venue definitions.
    venues: Option<String>,
}

#[allow(dead_code)]
impl Lighting {
    /// Creates a new lighting configuration.
    pub fn new(
        current_venue: Option<String>,
        fixtures: Option<HashMap<String, String>>,
        groups: Option<HashMap<String, LogicalGroup>>,
        directories: Option<Directories>,
    ) -> Lighting {
        Lighting {
            current_venue,
            fixtures,
            groups,
            directories,
        }
    }

    /// Gets the current venue.
    pub fn current_venue(&self) -> Option<&str> {
        self.current_venue.as_deref()
    }

    /// Gets the fixtures.
    pub fn fixtures(&self) -> HashMap<String, String> {
        self.fixtures.clone().unwrap_or_default()
    }

    /// Gets the logical groups.
    pub fn groups(&self) -> HashMap<String, LogicalGroup> {
        self.groups.clone().unwrap_or_default()
    }

    /// Gets the directories configuration.
    pub fn directories(&self) -> Option<&Directories> {
        self.directories.as_ref()
    }
}

#[allow(dead_code)]
impl Directories {
    /// Creates a new directories configuration.
    pub fn new(fixture_types: Option<String>, venues: Option<String>) -> Directories {
        Directories {
            fixture_types,
            venues,
        }
    }

    /// Gets the fixture types directory.
    pub fn fixture_types(&self) -> Option<&str> {
        self.fixture_types.as_deref()
    }

    /// Gets the venues directory.
    pub fn venues(&self) -> Option<&str> {
        self.venues.as_deref()
    }
}
