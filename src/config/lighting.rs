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

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Group constraint types for role-based group resolution
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
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct LogicalGroup {
    /// The name of the group
    name: String,
    /// Constraints for resolving this group
    constraints: Vec<GroupConstraint>,
}

impl LogicalGroup {
    #[cfg(test)]
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
#[derive(Deserialize, Serialize, Clone)]
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
#[derive(Deserialize, Serialize, Clone)]
pub struct Directories {
    /// Directory containing fixture type definitions.
    fixture_types: Option<String>,

    /// Directory containing venue definitions.
    venues: Option<String>,
}

impl Lighting {
    #[cfg(test)]
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

    /// Returns the raw inline fixtures map (without cloning).
    pub fn inline_fixtures_raw(&self) -> Option<&HashMap<String, String>> {
        self.fixtures.as_ref()
    }

    /// Clears inline fixtures.
    pub fn clear_inline_fixtures(&mut self) {
        self.fixtures = None;
    }

    /// Sets the venues directory, creating the directories struct if needed.
    pub fn set_venues_dir(&mut self, dir: String) {
        match &mut self.directories {
            Some(dirs) => dirs.venues = Some(dir),
            None => {
                self.directories = Some(Directories {
                    fixture_types: None,
                    venues: Some(dir),
                })
            }
        }
    }
}

impl Directories {
    /// Gets the fixture types directory.
    pub fn fixture_types(&self) -> Option<&str> {
        self.fixture_types.as_deref()
    }

    /// Gets the venues directory.
    pub fn venues(&self) -> Option<&str> {
        self.venues.as_deref()
    }
}

#[cfg(test)]
impl Directories {
    pub fn new(fixture_types: Option<String>, venues: Option<String>) -> Self {
        Self {
            fixture_types,
            venues,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lighting_current_venue_some() {
        let l = Lighting::new(Some("club".to_string()), None, None, None);
        assert_eq!(l.current_venue(), Some("club"));
    }

    #[test]
    fn lighting_current_venue_none() {
        let l = Lighting::new(None, None, None, None);
        assert_eq!(l.current_venue(), None);
    }

    #[test]
    fn fixtures_default_empty() {
        let l = Lighting::new(None, None, None, None);
        assert!(l.fixtures().is_empty());
    }

    #[test]
    fn fixtures_populated() {
        let mut fixtures = HashMap::new();
        fixtures.insert("par1".to_string(), "generic_par".to_string());
        fixtures.insert("mover1".to_string(), "moving_head".to_string());
        let l = Lighting::new(None, Some(fixtures), None, None);
        let f = l.fixtures();
        assert_eq!(f.len(), 2);
        assert_eq!(f.get("par1").unwrap(), "generic_par");
    }

    #[test]
    fn groups_default_empty() {
        let l = Lighting::new(None, None, None, None);
        assert!(l.groups().is_empty());
    }

    #[test]
    fn groups_populated() {
        let mut groups = HashMap::new();
        groups.insert(
            "front".to_string(),
            LogicalGroup::new("front".to_string(), vec![GroupConstraint::MinCount(2)]),
        );
        let l = Lighting::new(None, None, Some(groups), None);
        let g = l.groups();
        assert_eq!(g.len(), 1);
        let front = g.get("front").unwrap();
        assert_eq!(front.name(), "front");
        assert_eq!(front.constraints().len(), 1);
    }

    #[test]
    fn directories_none() {
        let l = Lighting::new(None, None, None, None);
        assert!(l.directories().is_none());
    }

    #[test]
    fn directories_some() {
        let dirs = Directories::new(Some("/fixtures".to_string()), Some("/venues".to_string()));
        let l = Lighting::new(None, None, None, Some(dirs));
        let d = l.directories().unwrap();
        assert_eq!(d.fixture_types(), Some("/fixtures"));
        assert_eq!(d.venues(), Some("/venues"));
    }

    #[test]
    fn directories_partial() {
        let dirs = Directories::new(Some("/fixtures".to_string()), None);
        assert_eq!(dirs.fixture_types(), Some("/fixtures"));
        assert_eq!(dirs.venues(), None);
    }

    #[test]
    fn logical_group_accessors() {
        let group = LogicalGroup::new(
            "wash".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["par".to_string()]),
                GroupConstraint::MaxCount(4),
                GroupConstraint::AllowEmpty(true),
            ],
        );
        assert_eq!(group.name(), "wash");
        assert_eq!(group.constraints().len(), 3);
    }

    #[test]
    fn group_constraint_variants() {
        // Just ensure all variants construct properly.
        let constraints = [
            GroupConstraint::AllOf(vec!["a".to_string()]),
            GroupConstraint::AnyOf(vec!["b".to_string()]),
            GroupConstraint::Prefer(vec!["c".to_string()]),
            GroupConstraint::MinCount(1),
            GroupConstraint::MaxCount(10),
            GroupConstraint::FallbackTo("other".to_string()),
            GroupConstraint::AllowEmpty(false),
        ];
        assert_eq!(constraints.len(), 7);
    }

    #[test]
    fn serde_round_trip() {
        let yaml = r#"
            current_venue: "main_stage"
            fixtures:
              par1: generic_par
              mover1: moving_head
            directories:
              fixture_types: /path/to/fixtures
              venues: /path/to/venues
        "#;
        let lighting: Lighting = config::Config::builder()
            .add_source(config::File::from_str(yaml, config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(lighting.current_venue(), Some("main_stage"));
        assert_eq!(lighting.fixtures().len(), 2);
        let dirs = lighting.directories().unwrap();
        assert_eq!(dirs.fixture_types(), Some("/path/to/fixtures"));
        assert_eq!(dirs.venues(), Some("/path/to/venues"));
    }

    #[test]
    fn serde_minimal() {
        let yaml = "{}";
        let lighting: Lighting = config::Config::builder()
            .add_source(config::File::from_str(yaml, config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(lighting.current_venue(), None);
        assert!(lighting.fixtures().is_empty());
        assert!(lighting.groups().is_empty());
        assert!(lighting.directories().is_none());
    }
}
