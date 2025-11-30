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

use std::collections::{HashMap, HashSet};
use std::error::Error;

use super::parser::LightShow;
use crate::config::Lighting;

/// Validation result containing information about the validation.
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// All groups/fixtures referenced in the shows
    pub groups: HashSet<String>,
    /// Invalid groups/fixtures (if config was provided)
    pub invalid_groups: Vec<String>,
}

impl ValidationResult {
    /// Returns true if validation passed (no invalid groups)
    pub fn is_valid(&self) -> bool {
        self.invalid_groups.is_empty()
    }
}

/// Collects all fixture group names used in the given shows.
pub fn collect_groups(shows: &HashMap<String, LightShow>) -> HashSet<String> {
    let mut groups = HashSet::new();

    for show in shows.values() {
        for cue in &show.cues {
            for effect in &cue.effects {
                for group in &effect.groups {
                    groups.insert(group.clone());
                }
            }
        }
    }

    groups
}

/// Validates that all groups/fixtures referenced in the shows exist in the config.
/// Returns a ValidationResult with information about the validation.
/// If config is None, only collects groups without validation.
pub fn validate_groups(
    shows: &HashMap<String, LightShow>,
    config: Option<&Lighting>,
) -> ValidationResult {
    let groups = collect_groups(shows);

    let invalid_groups = if let Some(lighting_config) = config {
        let valid_groups = lighting_config.groups();
        let valid_fixtures = lighting_config.fixtures();
        let mut all_valid_names: HashSet<String> = valid_groups.keys().cloned().collect();
        all_valid_names.extend(valid_fixtures.keys().cloned());

        groups
            .iter()
            .filter(|group| !all_valid_names.contains(*group))
            .cloned()
            .collect()
    } else {
        Vec::new()
    };

    ValidationResult {
        groups,
        invalid_groups,
    }
}

/// Validates light shows and returns an error if validation fails.
/// This is the main validation function that should be used when loading shows.
pub fn validate_light_shows(
    shows: &HashMap<String, LightShow>,
    config: Option<&Lighting>,
) -> Result<(), Box<dyn Error>> {
    let result = validate_groups(shows, config);

    if !result.is_valid() {
        let mut error_msg = format!(
            "Light show validation failed: {} invalid group(s)/fixture(s) referenced",
            result.invalid_groups.len()
        );
        for group in &result.invalid_groups {
            error_msg.push_str(&format!("\n  - {} (not found in config)", group));
        }
        return Err(error_msg.into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::lighting::{GroupConstraint, LogicalGroup};
    use crate::lighting::parser::parse_light_shows;
    use std::collections::HashMap;

    fn create_test_shows() -> HashMap<String, crate::lighting::parser::LightShow> {
        let content = r#"show "Test Show 1" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
    back_wash: static color: "red", dimmer: 80%
}

show "Test Show 2" {
    @00:00.000
    movers: cycle color: "green", color: "yellow", speed: 1.0
    strobes: strobe frequency: 4
}"#;

        parse_light_shows(content).expect("Failed to parse test shows")
    }

    fn create_test_config() -> Lighting {
        let mut groups = HashMap::new();
        groups.insert(
            "front_wash".to_string(),
            LogicalGroup::new(
                "front_wash".to_string(),
                vec![GroupConstraint::AllOf(vec![
                    "wash".to_string(),
                    "front".to_string(),
                ])],
            ),
        );
        groups.insert(
            "back_wash".to_string(),
            LogicalGroup::new(
                "back_wash".to_string(),
                vec![GroupConstraint::AllOf(vec![
                    "wash".to_string(),
                    "back".to_string(),
                ])],
            ),
        );
        groups.insert(
            "movers".to_string(),
            LogicalGroup::new(
                "movers".to_string(),
                vec![GroupConstraint::AnyOf(vec![
                    "moving_head".to_string(),
                    "spot".to_string(),
                ])],
            ),
        );
        groups.insert(
            "strobes".to_string(),
            LogicalGroup::new(
                "strobes".to_string(),
                vec![GroupConstraint::AllOf(vec!["strobe".to_string()])],
            ),
        );

        let mut fixtures = HashMap::new();
        fixtures.insert(
            "emergency_light".to_string(),
            "Emergency @ 1:500".to_string(),
        );

        Lighting::new(
            Some("main_stage".to_string()),
            Some(fixtures),
            Some(groups),
            None, // Don't need directories for validation tests
        )
    }

    #[test]
    fn test_collect_groups_basic() {
        let shows = create_test_shows();
        let groups = collect_groups(&shows);

        assert_eq!(groups.len(), 4);
        assert!(groups.contains("front_wash"));
        assert!(groups.contains("back_wash"));
        assert!(groups.contains("movers"));
        assert!(groups.contains("strobes"));
    }

    #[test]
    fn test_collect_groups_empty_show() {
        let content = r#"show "Empty Show" {
}"#;
        let shows = parse_light_shows(content).expect("Failed to parse empty show");
        let groups = collect_groups(&shows);

        assert_eq!(groups.len(), 0);
    }

    #[test]
    fn test_collect_groups_duplicate_groups() {
        let content = r#"show "Duplicate Groups" {
    @00:00.000
    front_wash: static color: "blue"
    @00:05.000
    front_wash: static color: "red"
}"#;
        let shows = parse_light_shows(content).expect("Failed to parse show");
        let groups = collect_groups(&shows);

        // Should only have one entry for front_wash even though it's used twice
        assert_eq!(groups.len(), 1);
        assert!(groups.contains("front_wash"));
    }

    #[test]
    fn test_validate_groups_without_config() {
        let shows = create_test_shows();
        let result = validate_groups(&shows, None);

        assert_eq!(result.groups.len(), 4);
        assert_eq!(result.invalid_groups.len(), 0);
        assert!(result.is_valid());
    }

    #[test]
    fn test_validate_groups_with_valid_config() {
        let shows = create_test_shows();
        let config = create_test_config();
        let result = validate_groups(&shows, Some(&config));

        assert_eq!(result.groups.len(), 4);
        assert_eq!(result.invalid_groups.len(), 0);
        assert!(result.is_valid());
    }

    #[test]
    fn test_validate_groups_with_invalid_groups() {
        let content = r#"show "Invalid Show" {
    @00:00.000
    front_wash: static color: "blue"
    invalid_group: static color: "red"
    another_invalid: static color: "green"
}"#;
        let shows = parse_light_shows(content).expect("Failed to parse show");
        let config = create_test_config();
        let result = validate_groups(&shows, Some(&config));

        assert_eq!(result.groups.len(), 3);
        assert_eq!(result.invalid_groups.len(), 2);
        assert!(!result.is_valid());
        assert!(result.invalid_groups.contains(&"invalid_group".to_string()));
        assert!(result
            .invalid_groups
            .contains(&"another_invalid".to_string()));
        // front_wash should be valid
        assert!(!result.invalid_groups.contains(&"front_wash".to_string()));
    }

    #[test]
    fn test_validate_groups_with_fixtures() {
        let content = r#"show "Fixture Show" {
    @00:00.000
    emergency_light: static color: "red"
}"#;
        let shows = parse_light_shows(content).expect("Failed to parse show");
        let config = create_test_config();
        let result = validate_groups(&shows, Some(&config));

        // emergency_light is defined as a fixture in config, so it should be valid
        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.invalid_groups.len(), 0);
        assert!(result.is_valid());
    }

    #[test]
    fn test_validate_light_shows_valid() {
        let shows = create_test_shows();
        let config = create_test_config();

        let result = validate_light_shows(&shows, Some(&config));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_light_shows_valid_no_config() {
        let shows = create_test_shows();

        let result = validate_light_shows(&shows, None);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_light_shows_invalid() {
        let content = r#"show "Invalid Show" {
    @00:00.000
    front_wash: static color: "blue"
    invalid_group: static color: "red"
}"#;
        let shows = parse_light_shows(content).expect("Failed to parse show");
        let config = create_test_config();

        let result = validate_light_shows(&shows, Some(&config));
        assert!(result.is_err());

        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("invalid_group"));
        assert!(error_msg.contains("validation failed"));
    }

    #[test]
    fn test_validate_light_shows_multiple_invalid() {
        let content = r#"show "Multiple Invalid" {
    @00:00.000
    invalid1: static color: "blue"
    invalid2: static color: "red"
    invalid3: static color: "green"
}"#;
        let shows = parse_light_shows(content).expect("Failed to parse show");
        let config = create_test_config();

        let result = validate_light_shows(&shows, Some(&config));
        assert!(result.is_err());

        let error_msg = result.unwrap_err().to_string();
        assert!(error_msg.contains("3 invalid"));
        assert!(error_msg.contains("invalid1"));
        assert!(error_msg.contains("invalid2"));
        assert!(error_msg.contains("invalid3"));
    }

    #[test]
    fn test_validate_light_shows_empty() {
        let content = r#"show "Empty Show" {
}"#;
        let shows = parse_light_shows(content).expect("Failed to parse show");
        let config = create_test_config();

        let result = validate_light_shows(&shows, Some(&config));
        assert!(result.is_ok());
    }

    #[test]
    fn test_validation_result_is_valid() {
        let mut result = ValidationResult {
            groups: HashSet::new(),
            invalid_groups: Vec::new(),
        };
        assert!(result.is_valid());

        result.invalid_groups.push("invalid".to_string());
        assert!(!result.is_valid());
    }

    #[test]
    fn test_collect_groups_multiple_shows() {
        let content = r#"show "Show 1" {
    @00:00.000
    group_a: static color: "blue"
    group_b: static color: "red"
}

show "Show 2" {
    @00:00.000
    group_b: static color: "green"
    group_c: static color: "yellow"
}"#;
        let shows = parse_light_shows(content).expect("Failed to parse shows");
        let groups = collect_groups(&shows);

        // Should have 3 unique groups (group_b appears in both shows)
        assert_eq!(groups.len(), 3);
        assert!(groups.contains("group_a"));
        assert!(groups.contains("group_b"));
        assert!(groups.contains("group_c"));
    }

    #[test]
    fn test_validate_groups_partial_match() {
        let content = r#"show "Partial Match" {
    @00:00.000
    front_wash: static color: "blue"
    valid_group: static color: "red"
    invalid_group: static color: "green"
}"#;
        let shows = parse_light_shows(content).expect("Failed to parse show");

        // Create config with only some groups
        let mut groups = HashMap::new();
        groups.insert(
            "front_wash".to_string(),
            LogicalGroup::new(
                "front_wash".to_string(),
                vec![GroupConstraint::AllOf(vec!["wash".to_string()])],
            ),
        );
        groups.insert(
            "valid_group".to_string(),
            LogicalGroup::new(
                "valid_group".to_string(),
                vec![GroupConstraint::AllOf(vec!["valid".to_string()])],
            ),
        );

        let config = Lighting::new(None, None, Some(groups), None);

        let result = validate_groups(&shows, Some(&config));

        assert_eq!(result.groups.len(), 3);
        assert_eq!(result.invalid_groups.len(), 1);
        assert!(result.invalid_groups.contains(&"invalid_group".to_string()));
        assert!(!result.invalid_groups.contains(&"front_wash".to_string()));
        assert!(!result.invalid_groups.contains(&"valid_group".to_string()));
    }
}
