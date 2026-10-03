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

    pub fn constraints(&self) -> &[GroupConstraint] {
        &self.constraints
    }
}

/// Where a project's fixture types live, relative to the project, when the
/// config does not say: `lighting.directories.fixture_types` moves it.
pub const DEFAULT_FIXTURE_TYPES_DIR: &str = "lighting/fixture_types";

/// Where a project's venues live, relative to the project, when the config
/// does not say: `lighting.directories.venues` moves it.
pub const DEFAULT_VENUES_DIR: &str = "lighting/venues";

/// A directory as configured, or its default: the one resolution the
/// engine, the web API, MCP and the CLI share. An empty value is unset.
pub fn resolve_dir<'a>(configured: Option<&'a str>, default: &'a str) -> &'a str {
    configured.filter(|d| !d.is_empty()).unwrap_or(default)
}

/// The project's fixture types directory for a profile's lighting section,
/// or the default when it has none.
pub fn fixture_types_dir(lighting: Option<&Lighting>) -> &str {
    lighting.map_or(DEFAULT_FIXTURE_TYPES_DIR, Lighting::fixture_types_dir)
}

/// The project's venues directory for a profile's lighting section, or the
/// default when it has none.
pub fn venues_dir(lighting: Option<&Lighting>) -> &str {
    lighting.map_or(DEFAULT_VENUES_DIR, Lighting::venues_dir)
}

/// What the loader says about a profile that still patches fixtures inline
/// (`dmx.lighting.fixtures`, retired): they never lit, and venue files are
/// the one way to patch a fixture.
pub const RETIRED_INLINE_FIXTURES: &str = "dmx.lighting.fixtures is no longer supported: fixtures \
     are patched in venue files — run `mtrack migrate --apply` to move these into one";

/// A YAML representation of the lighting configuration.
#[derive(Deserialize, Serialize, Clone)]
pub struct Lighting {
    /// The current venue selection.
    current_venue: Option<String>,

    /// The retired inline fixture map (`fixtures:`), read only so the
    /// loader can refuse it by name and `mtrack migrate` can move it into a
    /// venue file. Never written back.
    #[serde(rename = "fixtures", default, skip_serializing)]
    retired_fixtures: Option<HashMap<String, String>>,

    /// Logical group definitions with role-based constraints.
    groups: Option<HashMap<String, LogicalGroup>>,

    /// Where fixture types and venues are read from, when not the defaults
    /// ([`DEFAULT_FIXTURE_TYPES_DIR`], [`DEFAULT_VENUES_DIR`]). Each is
    /// independent: naming one leaves the other at its default.
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
    /// A lighting configuration built in code — tests, and the web editor's
    /// patch check, which loads a venue from its files with no engine.
    pub fn new(
        current_venue: Option<String>,
        groups: Option<HashMap<String, LogicalGroup>>,
        directories: Option<Directories>,
    ) -> Lighting {
        Lighting {
            current_venue,
            retired_fixtures: None,
            groups,
            directories,
        }
    }

    /// Gets the current venue.
    pub fn current_venue(&self) -> Option<&str> {
        self.current_venue.as_deref()
    }

    /// Gets the logical groups.
    pub fn groups(&self) -> &HashMap<String, LogicalGroup> {
        static EMPTY: std::sync::LazyLock<HashMap<String, LogicalGroup>> =
            std::sync::LazyLock::new(HashMap::new);
        self.groups.as_ref().unwrap_or(&EMPTY)
    }

    /// Gets the directories configuration as written (an override of the
    /// defaults). Code that reads or writes the project's files asks
    /// [`Self::fixture_types_dir`] and [`Self::venues_dir`] instead.
    pub fn directories(&self) -> Option<&Directories> {
        self.directories.as_ref()
    }

    /// The fixture types directory, relative to the project: the
    /// configured one, else [`DEFAULT_FIXTURE_TYPES_DIR`].
    pub fn fixture_types_dir(&self) -> &str {
        resolve_dir(
            self.directories.as_ref().and_then(|d| d.fixture_types()),
            DEFAULT_FIXTURE_TYPES_DIR,
        )
    }

    /// The venues directory, relative to the project: the configured one,
    /// else [`DEFAULT_VENUES_DIR`].
    pub fn venues_dir(&self) -> &str {
        resolve_dir(
            self.directories.as_ref().and_then(|d| d.venues()),
            DEFAULT_VENUES_DIR,
        )
    }

    /// The retired inline fixture map, when the file still has one: what
    /// `mtrack migrate` moves into a venue file. Nothing else reads it.
    pub fn retired_inline_fixtures(&self) -> Option<&HashMap<String, String>> {
        self.retired_fixtures.as_ref().filter(|f| !f.is_empty())
    }

    /// The refusal for this section, when it still patches fixtures inline.
    pub fn retired_field_error(&self) -> Option<&'static str> {
        self.retired_inline_fixtures()
            .map(|_| RETIRED_INLINE_FIXTURES)
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

impl Directories {
    /// Directories built in code — tests, and the web editor's patch check.
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
        let l = Lighting::new(Some("club".to_string()), None, None);
        assert_eq!(l.current_venue(), Some("club"));
    }

    #[test]
    fn lighting_current_venue_none() {
        let l = Lighting::new(None, None, None);
        assert_eq!(l.current_venue(), None);
    }

    #[test]
    fn a_section_built_in_code_has_no_retired_fixtures() {
        let l = Lighting::new(None, None, None);
        assert!(l.retired_inline_fixtures().is_none());
        assert!(l.retired_field_error().is_none());
    }

    #[test]
    fn groups_default_empty() {
        let l = Lighting::new(None, None, None);
        assert!(l.groups().is_empty());
    }

    #[test]
    fn groups_populated() {
        let mut groups = HashMap::new();
        groups.insert(
            "front".to_string(),
            LogicalGroup::new("front".to_string(), vec![GroupConstraint::MinCount(2)]),
        );
        let l = Lighting::new(None, Some(groups), None);
        let g = l.groups();
        assert_eq!(g.len(), 1);
        let front = g.get("front").unwrap();
        assert_eq!(front.name(), "front");
        assert_eq!(front.constraints().len(), 1);
    }

    #[test]
    fn directories_resolve_per_directory_to_the_defaults() {
        let dirs = |ft: Option<&str>, v: Option<&str>| {
            Some(Directories::new(
                ft.map(str::to_string),
                v.map(str::to_string),
            ))
        };
        for (directories, types, venues) in [
            (None, DEFAULT_FIXTURE_TYPES_DIR, DEFAULT_VENUES_DIR),
            (
                dirs(None, None),
                DEFAULT_FIXTURE_TYPES_DIR,
                DEFAULT_VENUES_DIR,
            ),
            (dirs(Some("t"), None), "t", DEFAULT_VENUES_DIR),
            (dirs(None, Some("v")), DEFAULT_FIXTURE_TYPES_DIR, "v"),
            (dirs(Some("t"), Some("v")), "t", "v"),
            (
                dirs(Some(""), Some("")),
                DEFAULT_FIXTURE_TYPES_DIR,
                DEFAULT_VENUES_DIR,
            ),
        ] {
            let l = Lighting::new(None, None, directories);
            assert_eq!(l.fixture_types_dir(), types);
            assert_eq!(l.venues_dir(), venues);
            assert_eq!(fixture_types_dir(Some(&l)), types);
            assert_eq!(venues_dir(Some(&l)), venues);
        }
        assert_eq!(fixture_types_dir(None), DEFAULT_FIXTURE_TYPES_DIR);
        assert_eq!(venues_dir(None), DEFAULT_VENUES_DIR);
    }

    #[test]
    fn directories_none() {
        let l = Lighting::new(None, None, None);
        assert!(l.directories().is_none());
    }

    #[test]
    fn directories_some() {
        let dirs = Directories::new(Some("/fixtures".to_string()), Some("/venues".to_string()));
        let l = Lighting::new(None, None, Some(dirs));
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
        // Read, so the loader can refuse it by name; never written back.
        assert_eq!(lighting.retired_inline_fixtures().map(|f| f.len()), Some(2));
        assert_eq!(
            lighting.retired_field_error(),
            Some(RETIRED_INLINE_FIXTURES)
        );
        let written = crate::util::to_yaml_string(&lighting).unwrap();
        assert!(!written.contains("fixtures:"), "{written}");
        assert!(!written.contains("par1"), "{written}");
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
        assert!(lighting.retired_field_error().is_none());
        assert!(lighting.groups().is_empty());
        assert!(lighting.directories().is_none());
    }
}

#[cfg(test)]
mod migration_advice_tests {
    /// Parses a `Lighting` block the way the loader does, via the `config`
    /// crate rather than a serde_yaml dependency this crate does not carry.
    fn parse(yaml: &str) -> Result<super::Lighting, config::ConfigError> {
        config::Config::builder()
            .add_source(config::File::from_str(yaml, config::FileFormat::Yaml))
            .build()?
            .try_deserialize()
    }

    /// The shape the venue-group migration message tells people to write must
    /// actually load.
    ///
    /// `groups` is a map keyed by name, and the advice printed a sequence — so
    /// following it produced a config that fails to deserialize, which is worse
    /// than the error it was replacing.
    #[test]
    fn the_migration_advice_shape_deserializes() {
        // The advice itself, not a copy of it. Parsing a hand-written string
        // here left the real message free to say anything: reverting it to a
        // sequence kept this test green, which is the failure it exists to
        // prevent.
        let advice = crate::lighting::parser::fixture_venue::migration_yaml("wash");
        let dedented: String = advice
            .lines()
            .map(|line| line.strip_prefix("    ").unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n");
        let lighting: super::Lighting =
            parse(&dedented).expect("the shape the migration message prints must load");
        assert!(lighting.groups().contains_key("wash"));
        assert_eq!(lighting.groups()["wash"].name(), "wash");
    }

    #[test]
    fn a_sequence_of_groups_does_not_load() {
        // Guards the advice against drifting back to a list.
        let yaml = "groups:\n  - name: wash\n    constraints:\n      - AllOf: [\"wash\"]\n";
        assert!(
            parse(yaml).is_err(),
            "a sequence must not silently load, or the advice cannot be checked"
        );
    }
}
