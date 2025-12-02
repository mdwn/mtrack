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

use std::collections::HashMap;
use std::error::Error;
use std::path::Path;

use tracing::info;

use super::parser::{parse_fixture_types, parse_venues};
use super::types::{Fixture, FixtureType, Group, Venue};
use crate::config::lighting::{GroupConstraint, LogicalGroup};
use crate::config::Lighting;

/// The lighting system configuration.
pub struct LightingSystem {
    /// Global fixture types.
    fixture_types: HashMap<String, FixtureType>,

    /// Venues.
    venues: HashMap<String, Venue>,

    /// Current venue.
    current_venue: Option<String>,

    /// Inline fixtures.
    inline_fixtures: HashMap<String, String>,

    /// Inline groups.

    /// Logical groups with role-based constraints.
    logical_groups: HashMap<String, LogicalGroup>,

    /// Cached group resolutions per venue.
    group_cache: HashMap<String, HashMap<String, Vec<String>>>,
}

impl LightingSystem {
    /// Creates a new lighting system.
    pub fn new() -> LightingSystem {
        LightingSystem {
            fixture_types: HashMap::new(),
            venues: HashMap::new(),
            current_venue: None,
            inline_fixtures: HashMap::new(),
            logical_groups: HashMap::new(),
            group_cache: HashMap::new(),
        }
    }

    /// Loads the lighting configuration.
    pub fn load(&mut self, config: &Lighting, base_path: &Path) -> Result<(), Box<dyn Error>> {
        info!(
            "Loading lighting system from base path: {}",
            base_path.display()
        );

        // Set current venue
        if let Some(venue) = config.current_venue() {
            self.current_venue = Some(venue.to_string());
        }

        // Load inline fixtures and groups
        self.inline_fixtures = config.fixtures();
        self.logical_groups = config.groups();

        // Load directories if configured
        if let Some(dirs) = config.directories() {
            if let Some(fixture_types_dir) = dirs.fixture_types() {
                let path = base_path.join(fixture_types_dir);
                self.load_fixture_types_directory(&path)?;
            }

            if let Some(venues_dir) = dirs.venues() {
                let path = base_path.join(venues_dir);
                self.load_venues_directory(&path)?;
            }
        }

        Ok(())
    }

    /// Loads fixture types from a directory.
    fn load_fixture_types_directory(&mut self, dir: &Path) -> Result<(), Box<dyn Error>> {
        if !dir.exists() {
            return Ok(()); // Directory doesn't exist, skip
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                // Recursively load subdirectories
                self.load_fixture_types_directory(&path)?;
            } else if path.extension().is_some_and(|ext| ext == "light") {
                // Load .light files
                self.load_fixture_types_file(&path)?;
            }
        }
        Ok(())
    }

    /// Loads venues from a directory.
    fn load_venues_directory(&mut self, dir: &Path) -> Result<(), Box<dyn Error>> {
        if !dir.exists() {
            return Ok(()); // Directory doesn't exist, skip
        }

        for entry in std::fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_dir() {
                // Recursively load subdirectories
                self.load_venues_directory(&path)?;
            } else if path.extension().is_some_and(|ext| ext == "light") {
                // Load .light files
                self.load_venue_file(&path)?;
            }
        }
        Ok(())
    }

    /// Loads fixture types from a file.
    fn load_fixture_types_file(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        let content = std::fs::read_to_string(path)?;

        // Parse fixture types from DSL content
        match parse_fixture_types(&content) {
            Ok(types) => {
                for (name, fixture_type) in types {
                    info!(fixture_type = name, "Loading fixture type");
                    self.fixture_types.insert(name, fixture_type);
                }
            }
            Err(_e) => {
                // Failed to parse fixture types, continue loading other files
            }
        }

        Ok(())
    }

    /// Loads venues from a file.
    fn load_venue_file(&mut self, path: &Path) -> Result<(), Box<dyn Error>> {
        let content = std::fs::read_to_string(path)?;

        match parse_venues(&content) {
            Ok(venues) => {
                for (name, venue) in venues {
                    info!(fixture_type = name, "Loading venue");
                    self.venues.insert(name, venue);
                }
            }
            Err(_e) => {
                // Failed to parse venues, continue loading other files
            }
        }

        Ok(())
    }

    /// Gets the current venue.
    pub fn current_venue(&self) -> Option<&str> {
        self.current_venue.as_deref()
    }

    /// Gets a group by name from the current venue.
    pub fn get_group(&self, group_name: &str) -> Result<&Group, Box<dyn Error>> {
        if let Some(venue_name) = self.current_venue() {
            if let Some(venue) = self.venues.get(venue_name) {
                if let Some(group) = venue.groups().get(group_name) {
                    return Ok(group);
                }
            }
        }
        Err(format!("Group '{}' not found in current venue", group_name).into())
    }

    /// Resolves a logical group to concrete fixture names for the current venue.
    /// Returns an empty vector if the group cannot be resolved (graceful fallback).
    pub fn resolve_logical_group(
        &mut self,
        group_name: &str,
    ) -> Result<Vec<String>, Box<dyn Error>> {
        let venue_name = self.current_venue().ok_or("No current venue selected")?;

        // Check cache first
        if let Some(cached) = self
            .group_cache
            .get(venue_name)
            .and_then(|venue_cache| venue_cache.get(group_name))
        {
            return Ok(cached.clone());
        }

        // Get the logical group definition
        let logical_group = self
            .logical_groups
            .get(group_name)
            .ok_or_else(|| format!("Logical group '{}' not found", group_name))?;

        // Get the current venue
        let venue = self
            .venues
            .get(venue_name)
            .ok_or_else(|| format!("Venue '{}' not found", venue_name))?;

        // Resolve fixtures based on constraints
        let resolved_fixtures = self.resolve_group_constraints(logical_group, venue)?;

        // Cache the result
        self.group_cache
            .entry(venue_name.to_string())
            .or_default()
            .insert(group_name.to_string(), resolved_fixtures.clone());

        Ok(resolved_fixtures)
    }

    /// Resolves a logical group with graceful fallback - returns empty vector if group cannot be resolved.
    /// This allows songs to work even when some groups aren't available at the current venue.
    pub fn resolve_logical_group_graceful(&mut self, group_name: &str) -> Vec<String> {
        match self.resolve_logical_group(group_name) {
            Ok(fixtures) => fixtures,
            Err(_) => {
                // Check if the group has a FallbackTo constraint
                let fallback_group =
                    if let Some(logical_group) = self.logical_groups.get(group_name) {
                        logical_group.constraints().iter().find_map(|constraint| {
                            if let GroupConstraint::FallbackTo(fallback_group) = constraint {
                                Some(fallback_group.clone())
                            } else {
                                None
                            }
                        })
                    } else {
                        None
                    };

                if let Some(fallback_group) = fallback_group {
                    return self.resolve_logical_group_graceful(&fallback_group);
                }

                // Try legacy group system as final fallback
                if let Ok(legacy_group) = self.get_group(group_name) {
                    return legacy_group.fixtures().clone();
                }
                Vec::new()
            }
        }
    }

    /// Gets all fixtures from the current venue for effects engine registration
    pub fn get_current_venue_fixtures(
        &self,
    ) -> Result<Vec<crate::lighting::effects::FixtureInfo>, Box<dyn Error>> {
        let venue_name = self.current_venue().ok_or("No current venue selected")?;
        let venue = self
            .venues
            .get(venue_name)
            .ok_or_else(|| format!("Venue '{}' not found", venue_name))?;

        let mut fixture_infos = Vec::new();

        for (name, fixture) in venue.fixtures() {
            // Get the fixture type to determine channel mapping
            let fixture_type = self
                .fixture_types
                .get(fixture.fixture_type())
                .ok_or_else(|| format!("Fixture type '{}' not found", fixture.fixture_type()))?;

            let fixture_info = crate::lighting::effects::FixtureInfo {
                name: name.clone(),
                universe: fixture.universe() as u16,
                address: fixture.start_channel(),
                fixture_type: fixture.fixture_type().to_string(),
                channels: fixture_type.channels().clone(),
                max_strobe_frequency: fixture_type.max_strobe_frequency(),
            };

            fixture_infos.push(fixture_info);
        }

        Ok(fixture_infos)
    }

    /// Resolves group constraints to fixture names.
    fn resolve_group_constraints(
        &self,
        logical_group: &LogicalGroup,
        venue: &Venue,
    ) -> Result<Vec<String>, Box<dyn Error>> {
        let mut candidates: Vec<&Fixture> = venue.fixtures().values().collect();
        let mut min_count = 1;
        let mut max_count = candidates.len();
        let mut allow_empty = false;
        let mut preferred_tags: Vec<String> = Vec::new();

        // Apply constraints
        for constraint in logical_group.constraints() {
            match constraint {
                GroupConstraint::AllOf(required_tags) => {
                    candidates.retain(|fixture| {
                        required_tags.iter().all(|tag| fixture.tags().contains(tag))
                    });
                }
                GroupConstraint::AnyOf(any_tags) => {
                    candidates
                        .retain(|fixture| any_tags.iter().any(|tag| fixture.tags().contains(tag)));
                }
                GroupConstraint::Prefer(tags) => {
                    // Store preferred tags for later sorting (after count constraints)
                    preferred_tags = tags.clone();
                }
                GroupConstraint::MinCount(count) => {
                    min_count = *count;
                }
                GroupConstraint::MaxCount(count) => {
                    max_count = *count;
                }
                GroupConstraint::AllowEmpty(allow) => {
                    allow_empty = *allow;
                }
                GroupConstraint::FallbackTo(_) => {
                    // FallbackTo is handled at a higher level in resolve_logical_group_graceful
                    // This constraint is processed during group resolution, not constraint resolution
                }
            }
        }

        // Apply count constraints
        if candidates.len() < min_count {
            if allow_empty {
                return Ok(Vec::new());
            } else {
                return Err(format!(
                    "Not enough fixtures found for group '{}': found {}, required {}",
                    logical_group.name(),
                    candidates.len(),
                    min_count
                )
                .into());
            }
        }

        // Sort fixtures: first by preference (if Prefer constraint exists), then by name
        // This ensures preferred fixtures are selected first, but within each preference level,
        // fixtures are sorted lexicographically for consistent chase ordering
        if !preferred_tags.is_empty() {
            // Sort by preference score first, then by name
            candidates.sort_by(|a, b| {
                let a_score = preferred_tags
                    .iter()
                    .filter(|tag| a.tags().contains(tag))
                    .count();
                let b_score = preferred_tags
                    .iter()
                    .filter(|tag| b.tags().contains(tag))
                    .count();
                // First compare by preference score (higher is better)
                match b_score.cmp(&a_score) {
                    std::cmp::Ordering::Equal => {
                        // Tiebreaker: sort by name
                        a.name().cmp(b.name())
                    }
                    other => other,
                }
            });
        } else {
            // No preference constraint - just sort by name
            candidates.sort_by(|a, b| a.name().cmp(b.name()));
        }

        // Take up to max_count fixtures
        let selected: Vec<String> = candidates
            .iter()
            .take(max_count)
            .map(|fixture| fixture.name().to_string())
            .collect();

        Ok(selected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_tag_based_group_resolution() {
        let mut system = LightingSystem::new();

        // Create a venue with tagged fixtures
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string(), "rgb".to_string()],
            ),
        );
        fixtures.insert(
            "Wash2".to_string(),
            Fixture::new(
                "Wash2".to_string(),
                "RGBW_Par".to_string(),
                1,
                7,
                vec!["wash".to_string(), "front".to_string(), "rgb".to_string()],
            ),
        );
        fixtures.insert(
            "Mover1".to_string(),
            Fixture::new(
                "Mover1".to_string(),
                "MovingHead".to_string(),
                1,
                101,
                vec!["moving_head".to_string(), "spot".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures, HashMap::new());
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define logical groups
        let front_wash_group = LogicalGroup::new(
            "front_wash".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["wash".to_string(), "front".to_string()]),
                GroupConstraint::MinCount(2),
            ],
        );

        let movers_group = LogicalGroup::new(
            "movers".to_string(),
            vec![
                GroupConstraint::AnyOf(vec!["moving_head".to_string()]),
                GroupConstraint::MinCount(1),
            ],
        );

        system
            .logical_groups
            .insert("front_wash".to_string(), front_wash_group);
        system
            .logical_groups
            .insert("movers".to_string(), movers_group);

        // Test resolution
        let front_wash_fixtures = system.resolve_logical_group("front_wash").unwrap();
        assert_eq!(front_wash_fixtures.len(), 2);
        assert!(front_wash_fixtures.contains(&"Wash1".to_string()));
        assert!(front_wash_fixtures.contains(&"Wash2".to_string()));

        let movers_fixtures = system.resolve_logical_group("movers").unwrap();
        assert_eq!(movers_fixtures.len(), 1);
        assert!(movers_fixtures.contains(&"Mover1".to_string()));
    }

    #[test]
    fn test_group_resolution_insufficient_fixtures() {
        let mut system = LightingSystem::new();

        // Create a venue with only one wash fixture
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures, HashMap::new());
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define a group that requires 3 fixtures
        let group = LogicalGroup::new(
            "front_wash".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["wash".to_string(), "front".to_string()]),
                GroupConstraint::MinCount(3),
            ],
        );

        system
            .logical_groups
            .insert("front_wash".to_string(), group);

        // Test that resolution fails
        let result = system.resolve_logical_group("front_wash");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Not enough fixtures found"));
    }

    #[test]
    fn test_prefer_constraint() {
        let mut system = LightingSystem::new();

        // Create fixtures with different tag combinations
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string()],
            ),
        );
        fixtures.insert(
            "Wash2".to_string(),
            Fixture::new(
                "Wash2".to_string(),
                "RGBW_Par".to_string(),
                1,
                7,
                vec![
                    "wash".to_string(),
                    "front".to_string(),
                    "premium".to_string(),
                ],
            ),
        );
        fixtures.insert(
            "Wash3".to_string(),
            Fixture::new(
                "Wash3".to_string(),
                "RGBW_Par".to_string(),
                1,
                13,
                vec![
                    "wash".to_string(),
                    "front".to_string(),
                    "premium".to_string(),
                    "rgb".to_string(),
                ],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures, HashMap::new());
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define group with preference for premium fixtures
        let group = LogicalGroup::new(
            "premium_wash".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["wash".to_string(), "front".to_string()]),
                GroupConstraint::Prefer(vec!["premium".to_string()]),
                GroupConstraint::MinCount(2),
                GroupConstraint::MaxCount(2),
            ],
        );

        system
            .logical_groups
            .insert("premium_wash".to_string(), group);

        // Test resolution - should prefer fixtures with "premium" tag
        let fixtures = system.resolve_logical_group("premium_wash").unwrap();
        assert_eq!(fixtures.len(), 2);
        // Should include the two premium fixtures (Wash2 and Wash3) first due to preference
        assert!(fixtures.contains(&"Wash2".to_string()));
        assert!(fixtures.contains(&"Wash3".to_string()));
        // Should not include the non-premium fixture (Wash1) since we only take 2
        assert!(!fixtures.contains(&"Wash1".to_string()));
    }

    #[test]
    fn test_max_count_constraint() {
        let mut system = LightingSystem::new();

        // Create multiple fixtures that match the criteria
        let mut fixtures = HashMap::new();
        for i in 1..=5 {
            fixtures.insert(
                format!("Wash{}", i),
                Fixture::new(
                    format!("Wash{}", i),
                    "RGBW_Par".to_string(),
                    1,
                    (i * 6) as u16,
                    vec!["wash".to_string(), "front".to_string()],
                ),
            );
        }

        let venue = Venue::new("Test Venue".to_string(), fixtures, HashMap::new());
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define group with max count constraint
        let group = LogicalGroup::new(
            "limited_wash".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["wash".to_string(), "front".to_string()]),
                GroupConstraint::MaxCount(3),
            ],
        );

        system
            .logical_groups
            .insert("limited_wash".to_string(), group);

        // Test resolution - should limit to 3 fixtures
        let fixtures = system.resolve_logical_group("limited_wash").unwrap();
        assert_eq!(fixtures.len(), 3);
    }

    #[test]
    fn test_any_of_constraint() {
        let mut system = LightingSystem::new();

        // Create fixtures with different tag combinations
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string()],
            ),
        );
        fixtures.insert(
            "Spot1".to_string(),
            Fixture::new(
                "Spot1".to_string(),
                "MovingHead".to_string(),
                1,
                7,
                vec!["spot".to_string()],
            ),
        );
        fixtures.insert(
            "Beam1".to_string(),
            Fixture::new(
                "Beam1".to_string(),
                "Beam".to_string(),
                1,
                13,
                vec!["beam".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures, HashMap::new());
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define group that accepts any of multiple tag types
        let group = LogicalGroup::new(
            "any_light".to_string(),
            vec![
                GroupConstraint::AnyOf(vec![
                    "wash".to_string(),
                    "spot".to_string(),
                    "beam".to_string(),
                ]),
                GroupConstraint::MinCount(2),
            ],
        );

        system.logical_groups.insert("any_light".to_string(), group);

        // Test resolution - should include fixtures with any of the specified tags
        let fixtures = system.resolve_logical_group("any_light").unwrap();
        assert_eq!(fixtures.len(), 3);
        assert!(fixtures.contains(&"Wash1".to_string()));
        assert!(fixtures.contains(&"Spot1".to_string()));
        assert!(fixtures.contains(&"Beam1".to_string()));
    }

    #[test]
    fn test_complex_constraint_combination() {
        let mut system = LightingSystem::new();

        // Create fixtures with various tag combinations
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string(), "rgb".to_string()],
            ),
        );
        fixtures.insert(
            "Wash2".to_string(),
            Fixture::new(
                "Wash2".to_string(),
                "RGBW_Par".to_string(),
                1,
                7,
                vec![
                    "wash".to_string(),
                    "front".to_string(),
                    "rgb".to_string(),
                    "premium".to_string(),
                ],
            ),
        );
        fixtures.insert(
            "Wash3".to_string(),
            Fixture::new(
                "Wash3".to_string(),
                "RGBW_Par".to_string(),
                1,
                13,
                vec![
                    "wash".to_string(),
                    "front".to_string(),
                    "rgb".to_string(),
                    "premium".to_string(),
                ],
            ),
        );
        fixtures.insert(
            "Wash4".to_string(),
            Fixture::new(
                "Wash4".to_string(),
                "RGBW_Par".to_string(),
                1,
                19,
                vec!["wash".to_string(), "front".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures, HashMap::new());
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define complex group with multiple constraints
        let group = LogicalGroup::new(
            "complex_group".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["wash".to_string(), "front".to_string()]),
                GroupConstraint::Prefer(vec!["premium".to_string()]),
                GroupConstraint::MinCount(2),
                GroupConstraint::MaxCount(3),
            ],
        );

        system
            .logical_groups
            .insert("complex_group".to_string(), group);

        // Test resolution - should prefer premium fixtures but limit to 3
        let fixtures = system.resolve_logical_group("complex_group").unwrap();
        assert_eq!(fixtures.len(), 3);
        // Should include the premium fixtures first
        assert!(fixtures.contains(&"Wash2".to_string()));
        assert!(fixtures.contains(&"Wash3".to_string()));
        // Should include one non-premium fixture
        assert!(fixtures.contains(&"Wash1".to_string()) || fixtures.contains(&"Wash4".to_string()));
    }

    #[test]
    fn test_group_resolution_no_current_venue() {
        let mut system = LightingSystem::new();

        // Don't set current venue
        system.current_venue = None;

        let group = LogicalGroup::new("test_group".to_string(), vec![GroupConstraint::MinCount(1)]);
        system
            .logical_groups
            .insert("test_group".to_string(), group);

        // Test that resolution fails without current venue
        let result = system.resolve_logical_group("test_group");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("No current venue selected"));
    }

    #[test]
    fn test_group_resolution_nonexistent_group() {
        let mut system = LightingSystem::new();

        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures, HashMap::new());
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Try to resolve a group that doesn't exist
        let result = system.resolve_logical_group("nonexistent_group");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Logical group 'nonexistent_group' not found"));
    }

    #[test]
    fn test_group_resolution_nonexistent_venue() {
        let mut system = LightingSystem::new();

        // Set current venue to one that doesn't exist
        system.current_venue = Some("Nonexistent Venue".to_string());

        let group = LogicalGroup::new("test_group".to_string(), vec![GroupConstraint::MinCount(1)]);
        system
            .logical_groups
            .insert("test_group".to_string(), group);

        // Test that resolution fails with nonexistent venue
        let result = system.resolve_logical_group("test_group");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Venue 'Nonexistent Venue' not found"));
    }

    #[test]
    fn test_group_caching() {
        let mut system = LightingSystem::new();

        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures, HashMap::new());
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        let group = LogicalGroup::new(
            "cached_group".to_string(),
            vec![GroupConstraint::MinCount(1)],
        );
        system
            .logical_groups
            .insert("cached_group".to_string(), group);

        // First resolution
        let fixtures1 = system.resolve_logical_group("cached_group").unwrap();
        assert_eq!(fixtures1.len(), 1);
        assert!(fixtures1.contains(&"Wash1".to_string()));

        // Second resolution should use cache
        let fixtures2 = system.resolve_logical_group("cached_group").unwrap();
        assert_eq!(fixtures2.len(), 1);
        assert!(fixtures2.contains(&"Wash1".to_string()));

        // Verify cache was populated
        assert!(system.group_cache.contains_key("Test Venue"));
        assert!(system
            .group_cache
            .get("Test Venue")
            .unwrap()
            .contains_key("cached_group"));
    }

    #[test]
    fn test_graceful_fallback_missing_group() {
        let mut system = LightingSystem::new();

        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures, HashMap::new());
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Try to resolve a group that doesn't exist - should return empty vector
        let fixtures = system.resolve_logical_group_graceful("nonexistent_group");
        assert_eq!(fixtures.len(), 0);
    }

    #[test]
    fn test_graceful_fallback_insufficient_fixtures() {
        let mut system = LightingSystem::new();

        // Create venue with only one wash fixture
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures, HashMap::new());
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define group that requires 3 fixtures but only 1 available
        let group = LogicalGroup::new(
            "front_wash".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["wash".to_string(), "front".to_string()]),
                GroupConstraint::MinCount(3),
            ],
        );

        system
            .logical_groups
            .insert("front_wash".to_string(), group);

        // Test graceful fallback - should return empty vector
        let fixtures = system.resolve_logical_group_graceful("front_wash");
        assert_eq!(fixtures.len(), 0);
    }

    #[test]
    fn test_allow_empty_constraint() {
        let mut system = LightingSystem::new();

        // Create venue with no matching fixtures
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Mover1".to_string(),
            Fixture::new(
                "Mover1".to_string(),
                "MovingHead".to_string(),
                1,
                1,
                vec!["moving_head".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures, HashMap::new());
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define group that requires wash fixtures but none exist, but allows empty
        let group = LogicalGroup::new(
            "wash_lights".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["wash".to_string()]),
                GroupConstraint::AllowEmpty(true),
            ],
        );

        system
            .logical_groups
            .insert("wash_lights".to_string(), group);

        // Test that group resolves to empty list when no fixtures match
        let fixtures = system.resolve_logical_group("wash_lights").unwrap();
        assert_eq!(fixtures.len(), 0);
    }

    #[test]
    fn test_multiple_groups_graceful() {
        let mut system = LightingSystem::new();

        let mut fixtures = HashMap::new();
        fixtures.insert(
            "Wash1".to_string(),
            Fixture::new(
                "Wash1".to_string(),
                "RGBW_Par".to_string(),
                1,
                1,
                vec!["wash".to_string(), "front".to_string()],
            ),
        );

        let venue = Venue::new("Test Venue".to_string(), fixtures, HashMap::new());
        system.venues.insert("Test Venue".to_string(), venue);
        system.current_venue = Some("Test Venue".to_string());

        // Define one group that exists and one that doesn't
        let front_wash_group = LogicalGroup::new(
            "front_wash".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["wash".to_string(), "front".to_string()]),
                GroupConstraint::MinCount(1),
            ],
        );
        let movers_group = LogicalGroup::new(
            "movers".to_string(),
            vec![
                GroupConstraint::AllOf(vec!["moving_head".to_string()]),
                GroupConstraint::MinCount(1),
            ],
        );

        system
            .logical_groups
            .insert("front_wash".to_string(), front_wash_group);
        system
            .logical_groups
            .insert("movers".to_string(), movers_group);

        // Test multiple group resolution
        let _group_names = ["front_wash".to_string(), "movers".to_string()];
        let results = system.resolve_logical_group_graceful("front_wash");

        // front_wash should have fixtures
        assert_eq!(results.len(), 1);
        assert!(results.contains(&"Wash1".to_string()));
    }
}
