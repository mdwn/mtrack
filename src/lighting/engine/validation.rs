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
    if let Some(total_duration) = effect.total_duration() {
        if total_duration.as_secs_f64() < 0.0 {
            return Err(EffectError::Timing(format!(
                "Effect total duration must be non-negative, got {}s",
                total_duration.as_secs_f64()
            )));
        }
    }

    Ok(())
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
