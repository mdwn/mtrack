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

use super::common::*;
#[cfg(test)]
use crate::lighting::effects::*;
use crate::lighting::engine::EffectEngine;
use std::collections::HashMap;
use std::time::Duration;

#[test]
fn test_channel_state_blending() {
    // Test the ChannelState blending logic directly
    let base_state = ChannelState::new(0.8, EffectLayer::Background, BlendMode::Replace);
    let overlay_state = ChannelState::new(0.5, EffectLayer::Foreground, BlendMode::Multiply);

    let blended = base_state.blend_with(overlay_state);

    // Multiply: 0.8 * 0.5 = 0.4
    assert!((blended.value - 0.4).abs() < 0.01);
    assert_eq!(blended.layer, EffectLayer::Foreground); // Higher layer wins
    assert_eq!(blended.blend_mode, BlendMode::Multiply); // Higher layer's blend mode
}
#[test]
fn test_fixture_state_blending() {
    let mut fixture1 = FixtureState::new();
    fixture1.set_channel(
        "red".to_string(),
        ChannelState::new(1.0, EffectLayer::Background, BlendMode::Replace),
    );
    fixture1.set_channel(
        "green".to_string(),
        ChannelState::new(0.5, EffectLayer::Background, BlendMode::Replace),
    );

    let mut fixture2 = FixtureState::new();
    fixture2.set_channel(
        "green".to_string(),
        ChannelState::new(0.8, EffectLayer::Foreground, BlendMode::Multiply),
    );
    fixture2.set_channel(
        "blue".to_string(),
        ChannelState::new(0.3, EffectLayer::Foreground, BlendMode::Replace),
    );

    fixture1.blend_with(&fixture2);

    // Green should be blended: 0.5 * 0.8 = 0.4
    let green_state = fixture1.channels.get("green").unwrap();
    assert!((green_state.value - 0.4).abs() < 0.01);

    // Blue should be added (new channel)
    let blue_state = fixture1.channels.get("blue").unwrap();
    assert!((blue_state.value - 0.3).abs() < 0.01);

    // Red should be unchanged
    let red_state = fixture1.channels.get("red").unwrap();
    assert!((red_state.value - 1.0).abs() < 0.01);
}
#[test]
fn test_blend_mode_loss_debug() {
    use super::super::effects::*;
    use super::super::engine::EffectEngine;
    use super::super::parser::parse_light_shows;
    use std::collections::HashMap;

    // Initialize tracing

    // Test DSL that should use multiply blend mode
    let dsl_with_multiply = r#"show "Blend Mode Loss Test" {
    @00:00.000
    front_wash: static color: "blue", layer: background, blend_mode: replace
    
    @00:02.000
    front_wash: dimmer start_level: 1.0, end_level: 0.5, duration: 5s, layer: midground, blend_mode: multiply
}"#;

    // Parse the DSL
    let result = parse_light_shows(dsl_with_multiply);
    assert!(
        result.is_ok(),
        "DSL should parse successfully: {:?}",
        result
    );

    let shows = result.unwrap();
    let show = shows.get("Blend Mode Loss Test").unwrap();

    // Check that the dimmer effect has the correct blend mode
    let dimmer_cue = &show.cues[1];
    let dimmer_effect = &dimmer_cue.effects[0];
    assert_eq!(dimmer_effect.blend_mode, Some(BlendMode::Multiply));
    println!(
        "✅ DSL parsing: dimmer effect has blend_mode = {:?}",
        dimmer_effect.blend_mode
    );

    // Create effect engine and register fixtures
    let mut engine = EffectEngine::new();

    // Create a test fixture
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    channels.insert("strobe".to_string(), 4);

    let fixture = FixtureInfo {
        name: "front_wash".to_string(),
        universe: 1,
        address: 1,
        channels,
        fixture_type: "Astera-PixelBrick".to_string(),
        max_strobe_frequency: Some(20.0), // Test fixture with strobe
    };

    engine.register_fixture(fixture);

    // Create effect instances from the DSL effects
    let static_effect = create_effect_with_layering(
        "static_blue".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 0.0);
                params.insert("green".to_string(), 0.0);
                params.insert("blue".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["front_wash".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    let dimmer_effect = create_effect_with_layering(
        "dimmer_multiply".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.5,
            duration: Duration::from_secs(5),
            curve: DimmerCurve::Linear,
        },
        vec!["front_wash".to_string()],
        EffectLayer::Midground,
        BlendMode::Multiply,
    );

    println!("✅ Created effects:");
    println!(
        "  Static effect: blend_mode = {:?}",
        static_effect.blend_mode
    );
    println!(
        "  Dimmer effect: blend_mode = {:?}",
        dimmer_effect.blend_mode
    );

    // Start the static effect
    engine.start_effect(static_effect).unwrap();

    // Update to apply static effect
    let _commands = engine.update(Duration::from_secs(0), None).unwrap();
    println!("✅ Applied static effect");

    // Start the dimmer effect
    engine.start_effect(dimmer_effect).unwrap();

    // Update to apply dimmer effect
    let _commands = engine.update(Duration::from_secs(2), None).unwrap();
    println!("✅ Applied dimmer effect");

    // The debug output should show where the blend mode is being lost
}
#[test]
fn test_timeline_blend_mode_loss() {
    use super::super::effects::*;
    use super::super::engine::EffectEngine;
    use super::super::parser::parse_light_shows;
    use super::super::timeline::LightingTimeline;
    use std::collections::HashMap;

    // Initialize tracing

    // Test DSL that should use multiply blend mode
    let dsl_with_multiply = r#"show "Timeline Blend Mode Test" {
    @00:00.000
    front_wash: static color: "blue", layer: background, blend_mode: replace
    
    @00:02.000
    front_wash: dimmer start_level: 1.0, end_level: 0.5, duration: 5s, layer: midground, blend_mode: multiply
}"#;

    // Parse the DSL
    let result = parse_light_shows(dsl_with_multiply);
    assert!(
        result.is_ok(),
        "DSL should parse successfully: {:?}",
        result
    );

    let shows = result.unwrap();
    let show = shows.get("Timeline Blend Mode Test").unwrap();

    // Check that the dimmer effect has the correct blend mode
    let dimmer_cue = &show.cues[1];
    let dimmer_effect = &dimmer_cue.effects[0];
    assert_eq!(dimmer_effect.blend_mode, Some(BlendMode::Multiply));
    println!(
        "✅ DSL parsing: dimmer effect has blend_mode = {:?}",
        dimmer_effect.blend_mode
    );

    // Create timeline from the show
    let mut timeline = LightingTimeline::new_with_cues(show.cues.clone());
    println!("✅ Created timeline with {} cues", show.cues.len());

    // Create effect engine and register fixtures
    let mut engine = EffectEngine::new();

    // Create a test fixture
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    channels.insert("strobe".to_string(), 4);

    let fixture = FixtureInfo {
        name: "front_wash".to_string(),
        universe: 1,
        address: 1,
        channels,
        fixture_type: "Astera-PixelBrick".to_string(),
        max_strobe_frequency: Some(20.0), // Test fixture with strobe
    };

    engine.register_fixture(fixture);

    // Start the timeline
    timeline.start();
    println!("✅ Started timeline");

    // Update timeline to get effects at different times
    let result_at_0s = timeline.update(Duration::from_secs(0));
    println!("✅ Timeline at 0s: {} effects", result_at_0s.effects.len());
    for effect in &result_at_0s.effects {
        println!(
            "  Effect: {} blend_mode = {:?}",
            effect.id, effect.blend_mode
        );
    }

    let result_at_2s = timeline.update(Duration::from_secs(2));
    println!("✅ Timeline at 2s: {} effects", result_at_2s.effects.len());
    for effect in &result_at_2s.effects {
        println!(
            "  Effect: {} blend_mode = {:?}",
            effect.id, effect.blend_mode
        );
    }

    // Start the effects from timeline
    for effect in result_at_0s.effects {
        engine.start_effect(effect).unwrap();
    }

    // Update to apply static effect
    let _commands = engine.update(Duration::from_secs(0), None).unwrap();
    println!("✅ Applied static effect from timeline");

    // Start the dimmer effect from timeline
    for effect in result_at_2s.effects {
        engine.start_effect(effect).unwrap();
    }

    // Update to apply dimmer effect
    let _commands = engine.update(Duration::from_secs(2), None).unwrap();
    println!("✅ Applied dimmer effect from timeline");

    // The debug output should show where the blend mode is being lost
}
#[test]
fn test_blend_mode_compatibility_matrix() {
    let mut engine = EffectEngine::new();

    // Create test fixture
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);

    let fixture = FixtureInfo {
        name: "test_fixture".to_string(),
        universe: 1,
        address: 1,
        channels,
        fixture_type: "RGB_Par".to_string(),
        max_strobe_frequency: Some(20.0),
    };
    engine.register_fixture(fixture);

    // Test Replace mode conflicts with everything
    let replace_effect = create_effect_with_layering(
        "replace_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    let multiply_effect = create_effect_with_layering(
        "multiply_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("blue".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background, // Same layer
        BlendMode::Multiply,
    );

    engine.start_effect(replace_effect).unwrap();
    engine.start_effect(multiply_effect).unwrap();

    // Replace should conflict with Multiply (same layer, same type)
    assert_eq!(engine.active_effects_count(), 1);
    assert!(!engine.has_effect("replace_effect"));
    assert!(engine.has_effect("multiply_effect"));

    // Test compatible blend modes can layer
    let add_effect = create_effect_with_layering(
        "add_effect".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.5,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Add,
    );

    let overlay_effect = create_effect_with_layering(
        "overlay_effect".to_string(),
        EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.3,
            frequency: TempoAwareFrequency::Fixed(2.0),
            duration: None,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background, // Same layer
        BlendMode::Overlay,
    );

    engine.start_effect(add_effect).unwrap();
    engine.start_effect(overlay_effect).unwrap();

    // Add and Overlay should be compatible (different types, compatible blend modes)
    assert_eq!(engine.active_effects_count(), 3); // multiply + add + overlay
    assert!(engine.has_effect("add_effect"));
    assert!(engine.has_effect("overlay_effect"));

    // Test all blend mode combinations
    let blend_modes = [
        BlendMode::Replace,
        BlendMode::Multiply,
        BlendMode::Add,
        BlendMode::Overlay,
        BlendMode::Screen,
    ];

    for (i, mode1) in blend_modes.iter().enumerate() {
        for (j, mode2) in blend_modes.iter().enumerate() {
            let effect1 = create_effect_with_layering(
                format!("test_mode1_{}_{}", i, j),
                EffectType::Static {
                    parameters: {
                        let mut params = HashMap::new();
                        params.insert("red".to_string(), 1.0);
                        params
                    },
                    duration: None,
                },
                vec!["test_fixture".to_string()],
                EffectLayer::Background,
                *mode1,
            );

            let effect2 = create_effect_with_layering(
                format!("test_mode2_{}_{}", i, j),
                EffectType::Static {
                    parameters: {
                        let mut params = HashMap::new();
                        params.insert("blue".to_string(), 1.0);
                        params
                    },
                    duration: None,
                },
                vec!["test_fixture".to_string()],
                EffectLayer::Background, // Same layer
                *mode2,
            );

            // Clear engine for each test
            engine.stop_all_effects();

            engine.start_effect(effect1).unwrap();
            let count_before = engine.active_effects_count();
            engine.start_effect(effect2).unwrap();
            let count_after = engine.active_effects_count();

            // Verify expected behavior based on blend mode compatibility
            let should_conflict = !engine.blend_modes_are_compatible_public(*mode1, *mode2);
            if should_conflict {
                assert_eq!(
                    count_after, count_before,
                    "Blend modes {:?} and {:?} should conflict",
                    mode1, mode2
                );
            } else {
                assert_eq!(
                    count_after,
                    count_before + 1,
                    "Blend modes {:?} and {:?} should be compatible",
                    mode1,
                    mode2
                );
            }
        }
    }
}
