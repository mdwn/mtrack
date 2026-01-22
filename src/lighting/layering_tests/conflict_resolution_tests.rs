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
fn test_sophisticated_conflict_resolution() {
    let mut engine = EffectEngine::new();

    // Create a test fixture
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

    // Test 1: Effects in different layers should not conflict
    let static_bg = create_effect_with_layering(
        "static_bg".to_string(),
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

    let dimmer_mg = create_effect_with_layering(
        "dimmer_mg".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.5,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Midground,
        BlendMode::Multiply,
    );

    // Both effects should coexist (different layers)
    engine.start_effect(static_bg).unwrap();
    engine.start_effect(dimmer_mg).unwrap();
    assert_eq!(engine.active_effects_count(), 2);

    // Test 2: Static effects in the same layer should conflict
    let static_fg = create_effect_with_layering(
        "static_fg".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("blue".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background, // Same layer as static_bg
        BlendMode::Replace,
    );

    // This should stop the background static effect (same layer, same type)
    engine.start_effect(static_fg).unwrap();
    assert_eq!(engine.active_effects_count(), 2); // dimmer + new static
    assert!(!engine.has_effect("static_bg"));
    assert!(engine.has_effect("dimmer_mg"));
    assert!(engine.has_effect("static_fg"));

    // Test 3: Compatible blend modes should layer
    let pulse_mg = create_effect_with_layering(
        "pulse_mg".to_string(),
        EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.3,
            frequency: TempoAwareFrequency::Fixed(2.0),
            duration: None,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Midground,
        BlendMode::Multiply,
    );

    // This should layer with the existing dimmer (same layer, compatible blend modes)
    engine.start_effect(pulse_mg).unwrap();
    assert_eq!(engine.active_effects_count(), 3); // dimmer + static + pulse
    assert!(engine.has_effect("dimmer_mg"));
    assert!(engine.has_effect("pulse_mg"));

    // Test 4: Replace blend mode should stop conflicting effects
    let static_replace = create_effect_with_layering(
        "static_replace".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("green".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background, // Same layer as static_fg
        BlendMode::Replace,
    );

    // This should stop the existing static effect (same type, same layer, Replace mode)
    engine.start_effect(static_replace).unwrap();
    assert_eq!(engine.active_effects_count(), 3); // dimmer + pulse + new static
    assert!(!engine.has_effect("static_fg"));
    assert!(engine.has_effect("static_replace"));
}
#[test]
fn test_priority_based_conflict_resolution() {
    let mut engine = EffectEngine::new();

    // Create test fixtures
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);

    let fixture1 = FixtureInfo {
        name: "fixture1".to_string(),
        universe: 1,
        address: 1,
        channels: channels.clone(),
        fixture_type: "RGB_Par".to_string(),
        max_strobe_frequency: Some(20.0),
    };

    let fixture2 = FixtureInfo {
        name: "fixture2".to_string(),
        universe: 1,
        address: 2,
        channels: channels.clone(),
        fixture_type: "RGB_Par".to_string(),
        max_strobe_frequency: Some(20.0),
    };

    engine.register_fixture(fixture1);
    engine.register_fixture(fixture2);

    // Test 1: Higher priority effect stops lower priority effect in same layer
    let low_priority = create_effect_with_layering(
        "low_priority".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["fixture1".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    )
    .with_priority(1);

    let high_priority = create_effect_with_layering(
        "high_priority".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("blue".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["fixture1".to_string()],
        EffectLayer::Background, // Same layer
        BlendMode::Replace,
    )
    .with_priority(10);

    engine.start_effect(low_priority).unwrap();
    engine.start_effect(high_priority).unwrap();

    assert_eq!(engine.active_effects_count(), 1);
    assert!(!engine.has_effect("low_priority"));
    assert!(engine.has_effect("high_priority"));

    // Test 2: Effects on different fixtures should not conflict
    let different_fixture_effect = create_effect_with_layering(
        "different_fixture_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("green".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["fixture2".to_string()], // Different fixture
        EffectLayer::Background,      // Same layer
        BlendMode::Replace,
    )
    .with_priority(5);

    engine.start_effect(different_fixture_effect).unwrap();

    // Should not conflict because different fixtures
    assert_eq!(engine.active_effects_count(), 2); // high_priority + different_fixture_effect
    assert!(engine.has_effect("high_priority"));
    assert!(engine.has_effect("different_fixture_effect"));

    // Test 3: Higher priority effect stops lower priority effect on same fixture
    let low_priority_same_fixture = create_effect_with_layering(
        "low_priority_same_fixture".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("yellow".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["fixture2".to_string()], // Same fixture as different_fixture_effect
        EffectLayer::Background,
        BlendMode::Replace,
    )
    .with_priority(1);

    let high_priority_same_fixture = create_effect_with_layering(
        "high_priority_same_fixture".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("purple".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["fixture2".to_string()], // Same fixture as low_priority_same_fixture
        EffectLayer::Background,      // Same layer
        BlendMode::Replace,
    )
    .with_priority(15); // Higher priority than different_fixture_effect

    engine.start_effect(low_priority_same_fixture).unwrap();
    engine.start_effect(high_priority_same_fixture).unwrap();

    // High priority should stop both lower priority effects on same fixture
    assert_eq!(engine.active_effects_count(), 2); // high_priority + high_priority_same_fixture
    assert!(engine.has_effect("high_priority"));
    assert!(!engine.has_effect("different_fixture_effect"));
    assert!(!engine.has_effect("low_priority_same_fixture"));
    assert!(engine.has_effect("high_priority_same_fixture"));
}
#[test]
fn test_effect_type_conflict_combinations() {
    let mut engine = EffectEngine::new();

    // Create test fixture
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    channels.insert("strobe".to_string(), 4);
    // No dimmer channel - Chase should work with RGB channels

    let fixture = FixtureInfo {
        name: "test_fixture".to_string(),
        universe: 1,
        address: 1,
        channels,
        fixture_type: "RGB_Par".to_string(),
        max_strobe_frequency: Some(20.0),
    };
    engine.register_fixture(fixture);

    // Test Static vs ColorCycle conflict
    let static_effect = create_effect_with_layering(
        "static_effect".to_string(),
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

    let color_cycle_effect = create_effect_with_layering(
        "color_cycle_effect".to_string(),
        EffectType::ColorCycle {
            colors: vec![
                Color {
                    r: 255,
                    g: 0,
                    b: 0,
                    w: None,
                },
                Color {
                    r: 0,
                    g: 255,
                    b: 0,
                    w: None,
                },
            ],
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background, // Same layer
        BlendMode::Replace,
    );

    engine.start_effect(static_effect).unwrap();
    engine.start_effect(color_cycle_effect).unwrap();

    // Static and ColorCycle should conflict
    assert_eq!(engine.active_effects_count(), 1);
    assert!(!engine.has_effect("static_effect"));
    assert!(engine.has_effect("color_cycle_effect"));

    // Test Rainbow vs Static conflict
    let rainbow_effect = create_effect_with_layering(
        "rainbow_effect".to_string(),
        EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(1.0),
            saturation: 1.0,
            brightness: 1.0,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    let static_effect2 = create_effect_with_layering(
        "static_effect2".to_string(),
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
        BlendMode::Replace,
    );

    engine.start_effect(rainbow_effect).unwrap();
    engine.start_effect(static_effect2).unwrap();

    // Rainbow and Static should conflict - static should win (last one wins)
    assert_eq!(engine.active_effects_count(), 1); // static_effect2
    assert!(!engine.has_effect("rainbow_effect"));
    assert!(engine.has_effect("static_effect2"));

    // Test Strobe vs Strobe conflict
    let strobe1 = create_effect_with_layering(
        "strobe1".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(2.0),
            duration: None,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    let strobe2 = create_effect_with_layering(
        "strobe2".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(4.0),
            duration: None,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background, // Same layer
        BlendMode::Replace,
    );

    engine.start_effect(strobe1).unwrap();
    engine.start_effect(strobe2).unwrap();

    // Strobe and Strobe should conflict
    assert_eq!(engine.active_effects_count(), 2); // static_effect2 + strobe2
    assert!(!engine.has_effect("strobe1"));
    assert!(engine.has_effect("strobe2"));

    // Test Chase vs Chase conflict
    let chase1 = create_effect_with_layering(
        "chase1".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    let chase2 = create_effect_with_layering(
        "chase2".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Snake,
            speed: TempoAwareSpeed::Fixed(2.0),
            direction: ChaseDirection::RightToLeft,
            transition: CycleTransition::Snap,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background, // Same layer
        BlendMode::Replace,
    );

    engine.start_effect(chase1).unwrap();
    engine.start_effect(chase2).unwrap();

    // Chase and Chase should conflict
    assert_eq!(engine.active_effects_count(), 3); // static_effect2 + strobe2 + chase2
    assert!(!engine.has_effect("chase1"));
    assert!(engine.has_effect("chase2"));

    // Test Dimmer and Pulse compatibility (should layer)
    let dimmer_effect = create_effect_with_layering(
        "dimmer_effect".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.5,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Multiply,
    );

    let pulse_effect = create_effect_with_layering(
        "pulse_effect".to_string(),
        EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.3,
            frequency: TempoAwareFrequency::Fixed(2.0),
            duration: None,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background, // Same layer
        BlendMode::Multiply,
    );

    engine.start_effect(dimmer_effect).unwrap();
    engine.start_effect(pulse_effect).unwrap();

    // Dimmer and Pulse should be compatible (they layer)
    assert_eq!(engine.active_effects_count(), 5); // static_effect2 + strobe2 + chase2 + dimmer + pulse
    assert!(engine.has_effect("dimmer_effect"));
    assert!(engine.has_effect("pulse_effect"));
}
#[test]
fn test_disabled_effects_not_participating_in_conflicts() {
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

    // Create a disabled effect
    let mut disabled_effect = create_effect_with_layering(
        "disabled_effect".to_string(),
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
    disabled_effect.enabled = false; // Disable the effect

    // Create a conflicting effect
    let conflicting_effect = create_effect_with_layering(
        "conflicting_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("blue".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background, // Same layer, same type
        BlendMode::Replace,
    );

    // Start the disabled effect first
    engine.start_effect(disabled_effect).unwrap();
    assert_eq!(engine.active_effects_count(), 1);
    assert!(engine.has_effect("disabled_effect"));

    // Start the conflicting effect
    engine.start_effect(conflicting_effect).unwrap();

    // The disabled effect should not be stopped because it's disabled
    // The conflicting effect should still be added
    assert_eq!(engine.active_effects_count(), 2);
    assert!(engine.has_effect("disabled_effect"));
    assert!(engine.has_effect("conflicting_effect"));

    // Test that disabled effects don't stop other effects
    let another_effect = create_effect_with_layering(
        "another_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("green".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background, // Same layer, same type
        BlendMode::Replace,
    );

    engine.start_effect(another_effect).unwrap();

    // The disabled effect should still be there, but the conflicting effect should be stopped
    assert_eq!(engine.active_effects_count(), 2); // disabled + another
    assert!(engine.has_effect("disabled_effect"));
    assert!(!engine.has_effect("conflicting_effect"));
    assert!(engine.has_effect("another_effect"));
}
#[test]
fn test_fixture_overlap_without_conflicts() {
    let mut engine = EffectEngine::new();

    // Create test fixtures
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);

    let fixture1 = FixtureInfo {
        name: "fixture1".to_string(),
        universe: 1,
        address: 1,
        channels: channels.clone(),
        fixture_type: "RGB_Par".to_string(),
        max_strobe_frequency: Some(20.0),
    };

    let fixture2 = FixtureInfo {
        name: "fixture2".to_string(),
        universe: 1,
        address: 2,
        channels: channels.clone(),
        fixture_type: "RGB_Par".to_string(),
        max_strobe_frequency: Some(20.0),
    };

    engine.register_fixture(fixture1);
    engine.register_fixture(fixture2);

    // Test effects targeting different fixtures (no overlap)
    let effect1 = create_effect_with_layering(
        "effect1".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["fixture1".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    let effect2 = create_effect_with_layering(
        "effect2".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("blue".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["fixture2".to_string()], // Different fixture
        EffectLayer::Background,      // Same layer, same type
        BlendMode::Replace,
    );

    engine.start_effect(effect1).unwrap();
    engine.start_effect(effect2).unwrap();

    // No overlap, so no conflict
    assert_eq!(engine.active_effects_count(), 2);
    assert!(engine.has_effect("effect1"));
    assert!(engine.has_effect("effect2"));

    // Test effects with partial overlap
    let effect3 = create_effect_with_layering(
        "effect3".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("green".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["fixture1".to_string(), "fixture2".to_string()], // Both fixtures
        EffectLayer::Background,
        BlendMode::Replace,
    );

    let effect4 = create_effect_with_layering(
        "effect4".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.5,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        },
        vec!["fixture1".to_string()], // Only fixture1
        EffectLayer::Background,      // Same layer
        BlendMode::Multiply,
    );

    engine.start_effect(effect3).unwrap();
    engine.start_effect(effect4).unwrap();

    // Overlap on fixture1, but different types (Static vs Dimmer)
    // Dimmer is generally compatible, so should layer
    // effect3 should stop effect1 and effect2 because they're all static effects
    assert_eq!(engine.active_effects_count(), 2); // effect3 + effect4
    assert!(!engine.has_effect("effect1"));
    assert!(!engine.has_effect("effect2"));
    assert!(engine.has_effect("effect3"));
    assert!(engine.has_effect("effect4"));
}
#[test]
fn test_channel_conflict_detection_behavior() {
    let mut engine = EffectEngine::new();

    // Create test fixtures
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);

    let fixture1 = FixtureInfo {
        name: "fixture1".to_string(),
        universe: 1,
        address: 1,
        channels: channels.clone(),
        fixture_type: "RGB_Par".to_string(),
        max_strobe_frequency: Some(20.0),
    };

    let fixture2 = FixtureInfo {
        name: "fixture2".to_string(),
        universe: 1,
        address: 2,
        channels: channels.clone(),
        fixture_type: "RGB_Par".to_string(),
        max_strobe_frequency: Some(20.0),
    };

    engine.register_fixture(fixture1);
    engine.register_fixture(fixture2);

    // Test that channel conflicts currently always return false
    let effect1 = create_effect_with_layering(
        "effect1".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["fixture1".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    let effect2 = create_effect_with_layering(
        "effect2".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("blue".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["fixture2".to_string()],
        EffectLayer::Foreground, // Different layer
        BlendMode::Replace,
    );

    engine.start_effect(effect1).unwrap();
    engine.start_effect(effect2).unwrap();

    // Since channel conflicts always return false, effects in different layers should coexist
    assert_eq!(engine.active_effects_count(), 2);
    assert!(engine.has_effect("effect1"));
    assert!(engine.has_effect("effect2"));

    // Test with same layer but different fixtures
    let effect3 = create_effect_with_layering(
        "effect3".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("green".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["fixture1".to_string()],
        EffectLayer::Background, // Same layer as effect1
        BlendMode::Replace,
    );

    engine.start_effect(effect3).unwrap();

    // Same layer, same type, same fixture - should conflict
    assert_eq!(engine.active_effects_count(), 2); // effect2 + effect3
    assert!(!engine.has_effect("effect1"));
    assert!(engine.has_effect("effect2"));
    assert!(engine.has_effect("effect3"));
}
