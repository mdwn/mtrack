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
fn test_effects_coexist_across_layers() {
    let mut engine = EffectEngine::new();

    // Create a test fixture
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);

    let fixture = FixtureInfo::new(
        "test_fixture".to_string(),
        1,
        1,
        "RGB_Par".to_string(),
        channels,
        Some(20.0),
    );
    engine.register_fixture(fixture);

    // Effects in different layers coexist
    let static_bg = create_effect_with_layering(
        "static_bg".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 1.0);
                params
            },
            duration: Duration::from_secs(10),
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

    engine.start_effect(static_bg).unwrap();
    engine.start_effect(dimmer_mg).unwrap();
    assert_eq!(engine.active_effects_count(), 2);

    // Adding another static in the same layer also coexists (no conflict resolution)
    let static_fg = create_effect_with_layering(
        "static_fg".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("blue".to_string(), 1.0);
                params
            },
            duration: Duration::from_secs(10),
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    engine.start_effect(static_fg).unwrap();
    assert_eq!(engine.active_effects_count(), 3);
    assert!(engine.has_effect("static_bg"));
    assert!(engine.has_effect("dimmer_mg"));
    assert!(engine.has_effect("static_fg"));

    // Pulse also coexists
    let pulse_mg = create_effect_with_layering(
        "pulse_mg".to_string(),
        EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.3,
            frequency: TempoAwareFrequency::Fixed(2.0),
            duration: Duration::from_secs(10),
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Midground,
        BlendMode::Multiply,
    );

    engine.start_effect(pulse_mg).unwrap();
    assert_eq!(engine.active_effects_count(), 4);
    assert!(engine.has_effect("dimmer_mg"));
    assert!(engine.has_effect("pulse_mg"));
}

#[test]
fn test_effects_coexist_on_different_fixtures() {
    let mut engine = EffectEngine::new();

    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);

    let fixture1 = FixtureInfo::new(
        "fixture1".to_string(),
        1,
        1,
        "RGB_Par".to_string(),
        channels.clone(),
        Some(20.0),
    );

    let fixture2 = FixtureInfo::new(
        "fixture2".to_string(),
        1,
        2,
        "RGB_Par".to_string(),
        channels.clone(),
        Some(20.0),
    );

    engine.register_fixture(fixture1);
    engine.register_fixture(fixture2);

    // Effects on different fixtures coexist
    let effect1 = create_effect_with_layering(
        "effect1".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 1.0);
                params
            },
            duration: Duration::from_secs(10),
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
            duration: Duration::from_secs(10),
        },
        vec!["fixture2".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    engine.start_effect(effect1).unwrap();
    engine.start_effect(effect2).unwrap();

    assert_eq!(engine.active_effects_count(), 2);
    assert!(engine.has_effect("effect1"));
    assert!(engine.has_effect("effect2"));
}

#[test]
fn test_all_effect_types_coexist() {
    let mut engine = EffectEngine::new();

    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    channels.insert("strobe".to_string(), 4);

    let fixture = FixtureInfo::new(
        "test_fixture".to_string(),
        1,
        1,
        "RGB_Par".to_string(),
        channels,
        Some(20.0),
    );
    engine.register_fixture(fixture);

    let static_effect = create_effect_with_layering(
        "static_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 1.0);
                params
            },
            duration: Duration::from_secs(10),
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
            duration: Duration::from_secs(10),
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    engine.start_effect(static_effect).unwrap();
    engine.start_effect(color_cycle_effect).unwrap();

    // All effects coexist (no conflict resolution)
    assert_eq!(engine.active_effects_count(), 2);
    assert!(engine.has_effect("static_effect"));
    assert!(engine.has_effect("color_cycle_effect"));

    let strobe1 = create_effect_with_layering(
        "strobe1".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(2.0),
            duration: Duration::from_secs(10),
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    let chase1 = create_effect_with_layering(
        "chase1".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::from_secs(10),
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

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
            duration: Duration::from_secs(10),
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Multiply,
    );

    engine.start_effect(strobe1).unwrap();
    engine.start_effect(chase1).unwrap();
    engine.start_effect(dimmer_effect).unwrap();
    engine.start_effect(pulse_effect).unwrap();

    // All 6 effects coexist
    assert_eq!(engine.active_effects_count(), 6);
    assert!(engine.has_effect("static_effect"));
    assert!(engine.has_effect("color_cycle_effect"));
    assert!(engine.has_effect("strobe1"));
    assert!(engine.has_effect("chase1"));
    assert!(engine.has_effect("dimmer_effect"));
    assert!(engine.has_effect("pulse_effect"));
}

#[test]
fn test_disabled_effects_coexist() {
    let mut engine = EffectEngine::new();

    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);

    let fixture = FixtureInfo::new(
        "test_fixture".to_string(),
        1,
        1,
        "RGB_Par".to_string(),
        channels,
        Some(20.0),
    );
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
            duration: Duration::from_secs(10),
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );
    disabled_effect.enabled = false;

    let active_effect = create_effect_with_layering(
        "active_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("blue".to_string(), 1.0);
                params
            },
            duration: Duration::from_secs(10),
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    engine.start_effect(disabled_effect).unwrap();
    engine.start_effect(active_effect).unwrap();

    // Both effects coexist
    assert_eq!(engine.active_effects_count(), 2);
    assert!(engine.has_effect("disabled_effect"));
    assert!(engine.has_effect("active_effect"));
}

#[test]
fn test_effects_coexist_in_different_layers() {
    let mut engine = EffectEngine::new();

    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);

    let fixture1 = FixtureInfo::new(
        "fixture1".to_string(),
        1,
        1,
        "RGB_Par".to_string(),
        channels.clone(),
        Some(20.0),
    );

    let fixture2 = FixtureInfo::new(
        "fixture2".to_string(),
        1,
        2,
        "RGB_Par".to_string(),
        channels.clone(),
        Some(20.0),
    );

    engine.register_fixture(fixture1);
    engine.register_fixture(fixture2);

    let effect1 = create_effect_with_layering(
        "effect1".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 1.0);
                params
            },
            duration: Duration::from_secs(10),
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
            duration: Duration::from_secs(10),
        },
        vec!["fixture2".to_string()],
        EffectLayer::Foreground,
        BlendMode::Replace,
    );

    engine.start_effect(effect1).unwrap();
    engine.start_effect(effect2).unwrap();

    // Effects in different layers coexist
    assert_eq!(engine.active_effects_count(), 2);
    assert!(engine.has_effect("effect1"));
    assert!(engine.has_effect("effect2"));

    // Adding another effect on same fixture and layer also coexists
    let effect3 = create_effect_with_layering(
        "effect3".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("green".to_string(), 1.0);
                params
            },
            duration: Duration::from_secs(10),
        },
        vec!["fixture1".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    engine.start_effect(effect3).unwrap();

    assert_eq!(engine.active_effects_count(), 3);
    assert!(engine.has_effect("effect1"));
    assert!(engine.has_effect("effect2"));
    assert!(engine.has_effect("effect3"));
}
