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

use std::collections::HashMap;

use super::super::effects::{
    EffectError, EffectInstance, EffectType, FixtureCapabilities, FixtureInfo, TempoAwareFrequency,
};

/// Validate fixture capabilities based on its type and channels
pub(crate) fn validate_fixture_capabilities(fixture: &FixtureInfo) -> Result<(), EffectError> {
    // Check for common capability mismatches
    if fixture.fixture_type.contains("RGB") && !fixture.channels.contains_key("red") {
        return Err(EffectError::Parameter(format!(
            "RGB fixture '{}' missing red channel",
            fixture.name
        )));
    }

    if fixture.fixture_type.contains("RGB") && !fixture.channels.contains_key("green") {
        return Err(EffectError::Parameter(format!(
            "RGB fixture '{}' missing green channel",
            fixture.name
        )));
    }

    if fixture.fixture_type.contains("RGB") && !fixture.channels.contains_key("blue") {
        return Err(EffectError::Parameter(format!(
            "RGB fixture '{}' missing blue channel",
            fixture.name
        )));
    }

    if fixture.fixture_type.contains("Strobe") && !fixture.channels.contains_key("strobe") {
        return Err(EffectError::Parameter(format!(
            "Strobe fixture '{}' missing strobe channel",
            fixture.name
        )));
    }

    if fixture.fixture_type.contains("MovingHead") && !fixture.channels.contains_key("pan") {
        return Err(EffectError::Parameter(format!(
            "Moving head fixture '{}' missing pan channel",
            fixture.name
        )));
    }

    if fixture.fixture_type.contains("MovingHead") && !fixture.channels.contains_key("tilt") {
        return Err(EffectError::Parameter(format!(
            "Moving head fixture '{}' missing tilt channel",
            fixture.name
        )));
    }

    Ok(())
}

/// Validate an effect before starting it
pub(crate) fn validate_effect(
    fixture_registry: &HashMap<String, FixtureInfo>,
    effect: &EffectInstance,
) -> Result<(), EffectError> {
    // Validate fixtures
    for fixture_name in &effect.target_fixtures {
        if !fixture_registry.contains_key(fixture_name) {
            return Err(EffectError::Fixture(format!(
                "Fixture '{}' not found",
                fixture_name
            )));
        }
    }

    // Validate effect compatibility with fixture special cases
    validate_effect_compatibility(fixture_registry, effect)?;

    // Validate effect parameters
    match &effect.effect_type {
        EffectType::Static { parameters, .. } => {
            for (key, value) in parameters {
                if *value < 0.0 || *value > 1.0 {
                    return Err(EffectError::Parameter(format!(
                        "Parameter '{}' must be between 0.0 and 1.0, got {}",
                        key, value
                    )));
                }
            }
        }
        EffectType::Strobe { frequency, .. } => {
            // For tempo-aware frequencies, we can't validate at parse time
            // They'll be validated when converted to Hz during processing
            // For fixed frequencies, validate now
            match frequency {
                TempoAwareFrequency::Fixed(freq) if *freq < 0.0 => {
                    return Err(EffectError::Parameter(format!(
                        "Strobe frequency must be non-negative, got {}",
                        freq
                    )));
                }
                _ => {}
            }
        }
        EffectType::Pulse { frequency, .. } => {
            // For tempo-aware frequencies, we can't validate at parse time
            // They'll be validated when converted to Hz during processing
            // For fixed frequencies, validate now
            match frequency {
                TempoAwareFrequency::Fixed(freq) if *freq <= 0.0 => {
                    return Err(EffectError::Parameter(format!(
                        "Pulse frequency must be positive, got {}",
                        freq
                    )));
                }
                _ => {}
            }
        }
        _ => {} // Other effect types don't need validation yet
    }

    // Validate timing
    let total_duration = effect.total_duration();
    if total_duration.as_secs_f64() < 0.0 {
        return Err(EffectError::Timing(format!(
            "Effect total duration must be non-negative, got {}s",
            total_duration.as_secs_f64()
        )));
    }

    Ok(())
}

#[cfg(test)]
fn make_fixture(name: &str, fixture_type: &str, channels: Vec<(&str, u16)>) -> FixtureInfo {
    let ch: HashMap<String, u16> = channels.iter().map(|(n, o)| (n.to_string(), *o)).collect();
    FixtureInfo::new(name.to_string(), 1, 1, fixture_type.to_string(), ch, None)
}

/// Validate that the effect is compatible with fixture special cases
pub(crate) fn validate_effect_compatibility(
    fixture_registry: &HashMap<String, FixtureInfo>,
    effect: &EffectInstance,
) -> Result<(), EffectError> {
    for fixture_name in &effect.target_fixtures {
        if let Some(fixture_info) = fixture_registry.get(fixture_name) {
            // Check if the effect type is compatible with the fixture's special cases
            match &effect.effect_type {
                EffectType::ColorCycle { .. } => {
                    if !fixture_info.has_capability(FixtureCapabilities::RGB_COLOR) {
                        return Err(EffectError::Parameter(format!(
                            "Color cycle effect not compatible with fixture '{}' (no RGB capability)",
                            fixture_name
                        )));
                    }
                }
                EffectType::Strobe { .. } => {
                    // Strobe effects work with any fixture that has strobe, dimmer, or RGB capability
                    if !fixture_info.has_capability(FixtureCapabilities::STROBING)
                        && !fixture_info.has_capability(FixtureCapabilities::DIMMING)
                        && !fixture_info.has_capability(FixtureCapabilities::RGB_COLOR)
                    {
                        return Err(EffectError::Parameter(format!(
                            "Strobe effect not compatible with fixture '{}' (no strobe, dimmer, or RGB capability)",
                            fixture_name
                        )));
                    }
                }
                EffectType::Chase { .. } => {
                    // Chase effects work with any fixture that has RGB or dimmer capability
                    if !fixture_info.has_capability(FixtureCapabilities::RGB_COLOR)
                        && !fixture_info.has_capability(FixtureCapabilities::DIMMING)
                    {
                        return Err(EffectError::Parameter(format!(
                            "Chase effect not compatible with fixture '{}' (no RGB or dimmer capability)",
                            fixture_name
                        )));
                    }
                }
                EffectType::Rainbow { .. } => {
                    // Rainbow effects require RGB channels
                    if !fixture_info.has_capability(FixtureCapabilities::RGB_COLOR) {
                        return Err(EffectError::Parameter(format!(
                            "Rainbow effect not compatible with fixture '{}' (no RGB capability)",
                            fixture_name
                        )));
                    }
                }
                _ => {} // Other effects are generally compatible
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::lighting::effects::{
        ChaseDirection, ChasePattern, Color, CycleDirection, CycleTransition, TempoAwareFrequency,
        TempoAwareSpeed,
    };

    fn rgb_fixture(name: &str) -> FixtureInfo {
        make_fixture(name, "RGB Par", vec![("red", 1), ("green", 2), ("blue", 3)])
    }

    fn dimmer_fixture(name: &str) -> FixtureInfo {
        make_fixture(name, "Dimmer", vec![("dimmer", 1)])
    }

    fn strobe_fixture(name: &str) -> FixtureInfo {
        make_fixture(name, "Strobe Unit", vec![("strobe", 1)])
    }

    fn make_effect_instance(effect_type: EffectType, fixtures: Vec<&str>) -> EffectInstance {
        EffectInstance::new(
            "test".to_string(),
            effect_type,
            fixtures.into_iter().map(|s| s.to_string()).collect(),
            None,
            None,
            None,
        )
    }

    fn registry_with(fixtures: Vec<FixtureInfo>) -> HashMap<String, FixtureInfo> {
        fixtures.into_iter().map(|f| (f.name.clone(), f)).collect()
    }

    // ── validate_fixture_capabilities ──────────────────────────────

    #[test]
    fn valid_rgb_fixture() {
        let f = rgb_fixture("par1");
        assert!(validate_fixture_capabilities(&f).is_ok());
    }

    #[test]
    fn rgb_fixture_missing_red() {
        let f = make_fixture("par1", "RGB Par", vec![("green", 2), ("blue", 3)]);
        assert!(validate_fixture_capabilities(&f).is_err());
    }

    #[test]
    fn rgb_fixture_missing_green() {
        let f = make_fixture("par1", "RGB Par", vec![("red", 1), ("blue", 3)]);
        assert!(validate_fixture_capabilities(&f).is_err());
    }

    #[test]
    fn rgb_fixture_missing_blue() {
        let f = make_fixture("par1", "RGB Par", vec![("red", 1), ("green", 2)]);
        assert!(validate_fixture_capabilities(&f).is_err());
    }

    #[test]
    fn strobe_fixture_missing_strobe_channel() {
        let f = make_fixture("s1", "Strobe Unit", vec![("dimmer", 1)]);
        assert!(validate_fixture_capabilities(&f).is_err());
    }

    #[test]
    fn moving_head_missing_pan() {
        let f = make_fixture("mh1", "MovingHead", vec![("tilt", 2)]);
        assert!(validate_fixture_capabilities(&f).is_err());
    }

    #[test]
    fn moving_head_missing_tilt() {
        let f = make_fixture("mh1", "MovingHead", vec![("pan", 1)]);
        assert!(validate_fixture_capabilities(&f).is_err());
    }

    #[test]
    fn non_special_type_always_valid() {
        let f = make_fixture("generic", "Generic", vec![("intensity", 1)]);
        assert!(validate_fixture_capabilities(&f).is_ok());
    }

    // ── validate_effect — fixture existence ────────────────────────

    #[test]
    fn validate_effect_unknown_fixture() {
        let registry = registry_with(vec![rgb_fixture("par1")]);
        let effect = make_effect_instance(
            EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(5),
            },
            vec!["nonexistent"],
        );
        assert!(validate_effect(&registry, &effect).is_err());
    }

    #[test]
    fn validate_effect_known_fixture() {
        let registry = registry_with(vec![rgb_fixture("par1")]);
        let effect = make_effect_instance(
            EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(5),
            },
            vec!["par1"],
        );
        assert!(validate_effect(&registry, &effect).is_ok());
    }

    // ── validate_effect — parameter ranges ─────────────────────────

    #[test]
    fn validate_static_param_out_of_range() {
        let registry = registry_with(vec![rgb_fixture("par1")]);
        let mut params = HashMap::new();
        params.insert("red".to_string(), 1.5); // > 1.0
        let effect = make_effect_instance(
            EffectType::Static {
                parameters: params,
                duration: Duration::from_secs(5),
            },
            vec!["par1"],
        );
        assert!(validate_effect(&registry, &effect).is_err());
    }

    #[test]
    fn validate_static_param_negative() {
        let registry = registry_with(vec![rgb_fixture("par1")]);
        let mut params = HashMap::new();
        params.insert("red".to_string(), -0.1);
        let effect = make_effect_instance(
            EffectType::Static {
                parameters: params,
                duration: Duration::from_secs(5),
            },
            vec!["par1"],
        );
        assert!(validate_effect(&registry, &effect).is_err());
    }

    #[test]
    fn validate_strobe_negative_frequency() {
        let registry = registry_with(vec![strobe_fixture("s1")]);
        let effect = make_effect_instance(
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(-1.0),
                duration: Duration::from_secs(5),
            },
            vec!["s1"],
        );
        assert!(validate_effect(&registry, &effect).is_err());
    }

    #[test]
    fn validate_pulse_zero_frequency() {
        let registry = registry_with(vec![rgb_fixture("par1")]);
        let effect = make_effect_instance(
            EffectType::Pulse {
                base_level: 0.0,
                pulse_amplitude: 1.0,
                frequency: TempoAwareFrequency::Fixed(0.0),
                duration: Duration::from_secs(5),
            },
            vec!["par1"],
        );
        assert!(validate_effect(&registry, &effect).is_err());
    }

    // ── validate_effect_compatibility ──────────────────────────────

    #[test]
    fn color_cycle_requires_rgb() {
        let registry = registry_with(vec![dimmer_fixture("d1")]);
        let effect = make_effect_instance(
            EffectType::ColorCycle {
                colors: vec![Color::new(255, 0, 0)],
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: CycleDirection::Forward,
                transition: CycleTransition::Fade,
                duration: Duration::from_secs(10),
            },
            vec!["d1"],
        );
        assert!(validate_effect_compatibility(&registry, &effect).is_err());
    }

    #[test]
    fn color_cycle_ok_with_rgb() {
        let registry = registry_with(vec![rgb_fixture("par1")]);
        let effect = make_effect_instance(
            EffectType::ColorCycle {
                colors: vec![Color::new(255, 0, 0)],
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: CycleDirection::Forward,
                transition: CycleTransition::Fade,
                duration: Duration::from_secs(10),
            },
            vec!["par1"],
        );
        assert!(validate_effect_compatibility(&registry, &effect).is_ok());
    }

    #[test]
    fn rainbow_requires_rgb() {
        let registry = registry_with(vec![dimmer_fixture("d1")]);
        let effect = make_effect_instance(
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Fixed(1.0),
                saturation: 1.0,
                brightness: 1.0,
                duration: Duration::from_secs(10),
            },
            vec!["d1"],
        );
        assert!(validate_effect_compatibility(&registry, &effect).is_err());
    }

    #[test]
    fn chase_ok_with_rgb() {
        let registry = registry_with(vec![rgb_fixture("par1")]);
        let effect = make_effect_instance(
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
                duration: Duration::from_secs(10),
            },
            vec!["par1"],
        );
        assert!(validate_effect_compatibility(&registry, &effect).is_ok());
    }

    #[test]
    fn chase_ok_with_dimmer() {
        let registry = registry_with(vec![dimmer_fixture("d1")]);
        let effect = make_effect_instance(
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
                duration: Duration::from_secs(10),
            },
            vec!["d1"],
        );
        assert!(validate_effect_compatibility(&registry, &effect).is_ok());
    }

    #[test]
    fn strobe_ok_with_dimmer() {
        let registry = registry_with(vec![dimmer_fixture("d1")]);
        let effect = make_effect_instance(
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(10.0),
                duration: Duration::from_secs(1),
            },
            vec!["d1"],
        );
        assert!(validate_effect_compatibility(&registry, &effect).is_ok());
    }

    #[test]
    fn static_effect_ok_with_anything() {
        let registry = registry_with(vec![dimmer_fixture("d1")]);
        let effect = make_effect_instance(
            EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(5),
            },
            vec!["d1"],
        );
        assert!(validate_effect_compatibility(&registry, &effect).is_ok());
    }

    // ── Strobe with no strobe/dimmer/RGB capability ─────────────────

    #[test]
    fn strobe_incompatible_with_no_capability() {
        // A fixture with no strobe, dimmer, or RGB capability should fail for strobe effects
        let f = make_fixture("generic", "Generic", vec![("pan", 1), ("tilt", 2)]);
        let registry = registry_with(vec![f]);
        let effect = make_effect_instance(
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(10.0),
                duration: Duration::from_secs(5),
            },
            vec!["generic"],
        );
        let result = validate_effect_compatibility(&registry, &effect);
        assert!(
            result.is_err(),
            "Strobe should be incompatible with fixture lacking strobe/dimmer/RGB"
        );
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Strobe effect not compatible"),
            "Error should mention strobe incompatibility: {}",
            err
        );
    }

    #[test]
    fn strobe_ok_with_rgb() {
        let registry = registry_with(vec![rgb_fixture("par1")]);
        let effect = make_effect_instance(
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(10.0),
                duration: Duration::from_secs(5),
            },
            vec!["par1"],
        );
        assert!(validate_effect_compatibility(&registry, &effect).is_ok());
    }

    #[test]
    fn strobe_ok_with_strobe_fixture() {
        let registry = registry_with(vec![strobe_fixture("s1")]);
        let effect = make_effect_instance(
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(10.0),
                duration: Duration::from_secs(5),
            },
            vec!["s1"],
        );
        assert!(validate_effect_compatibility(&registry, &effect).is_ok());
    }

    // ── Chase with no RGB/dimmer capability ──────────────────────────

    #[test]
    fn chase_incompatible_with_no_rgb_or_dimmer() {
        // A fixture with no RGB or dimmer capability should fail for chase effects
        let f = make_fixture("generic", "Generic", vec![("pan", 1), ("tilt", 2)]);
        let registry = registry_with(vec![f]);
        let effect = make_effect_instance(
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
                duration: Duration::from_secs(10),
            },
            vec!["generic"],
        );
        let result = validate_effect_compatibility(&registry, &effect);
        assert!(
            result.is_err(),
            "Chase should be incompatible with fixture lacking RGB/dimmer"
        );
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("Chase effect not compatible"),
            "Error should mention chase incompatibility: {}",
            err
        );
    }

    // ── Rainbow with non-RGB fixture ─────────────────────────────────

    #[test]
    fn rainbow_ok_with_rgb() {
        let registry = registry_with(vec![rgb_fixture("par1")]);
        let effect = make_effect_instance(
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Fixed(1.0),
                saturation: 1.0,
                brightness: 1.0,
                duration: Duration::from_secs(10),
            },
            vec!["par1"],
        );
        assert!(validate_effect_compatibility(&registry, &effect).is_ok());
    }

    // ── Validate effect with valid strobe frequency ──────────────────

    #[test]
    fn validate_strobe_zero_frequency_ok() {
        let registry = registry_with(vec![strobe_fixture("s1")]);
        let effect = make_effect_instance(
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(0.0),
                duration: Duration::from_secs(5),
            },
            vec!["s1"],
        );
        // Zero frequency is valid (it disables strobing)
        assert!(validate_effect(&registry, &effect).is_ok());
    }

    // ── Validate effect with tempo-aware frequency ───────────────────

    #[test]
    fn validate_strobe_tempo_aware_frequency() {
        let registry = registry_with(vec![strobe_fixture("s1")]);
        let effect = make_effect_instance(
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Beats(2.0),
                duration: Duration::from_secs(5),
            },
            vec!["s1"],
        );
        // Tempo-aware frequencies can't be validated at parse time
        assert!(validate_effect(&registry, &effect).is_ok());
    }

    // ── Validate pulse with tempo-aware frequency ────────────────────

    #[test]
    fn validate_pulse_tempo_aware_frequency() {
        let registry = registry_with(vec![rgb_fixture("par1")]);
        let effect = make_effect_instance(
            EffectType::Pulse {
                base_level: 0.5,
                pulse_amplitude: 0.5,
                frequency: TempoAwareFrequency::Beats(1.0),
                duration: Duration::from_secs(5),
            },
            vec!["par1"],
        );
        // Tempo-aware frequencies can't be validated at parse time
        assert!(validate_effect(&registry, &effect).is_ok());
    }

    // ── Validate effect with fixture not in registry ─────────────────

    #[test]
    fn validate_effect_compatibility_unknown_fixture_skipped() {
        // If a fixture isn't in the registry, validate_effect_compatibility
        // should still succeed (the fixture check is done in validate_effect separately)
        let registry = registry_with(vec![rgb_fixture("par1")]);
        let effect = make_effect_instance(
            EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(5),
            },
            vec!["not_in_registry"],
        );
        // validate_effect_compatibility doesn't check fixture existence
        assert!(validate_effect_compatibility(&registry, &effect).is_ok());
    }
}
