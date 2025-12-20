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

use crate::lighting::effects::*;
use crate::lighting::engine::tests::common::create_test_fixture;
use crate::lighting::engine::EffectEngine;
use std::collections::HashMap;
use std::time::Duration;

#[test]
fn test_get_active_effects_returns_correct_effects() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start multiple effects
    let effect1 = EffectInstance::new(
        "effect1".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 0.5);
                p
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    let mut effect2 = EffectInstance::new(
        "effect2".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("red".to_string(), 1.0);
                p
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    effect2.layer = EffectLayer::Foreground;

    engine.start_effect(effect1).unwrap();
    engine.start_effect(effect2).unwrap();

    let active_effects = engine.get_active_effects();
    assert_eq!(active_effects.len(), 2);
    assert!(active_effects.contains_key("effect1"));
    assert!(active_effects.contains_key("effect2"));

    // Verify we can read effect properties
    let e1 = active_effects.get("effect1").unwrap();
    assert_eq!(e1.layer, EffectLayer::Background);
    let e2 = active_effects.get("effect2").unwrap();
    assert_eq!(e2.layer, EffectLayer::Foreground);
}

#[test]
fn test_get_active_effects_empty_when_no_effects() {
    let engine = EffectEngine::new();
    let active_effects = engine.get_active_effects();
    assert!(active_effects.is_empty());
}

#[test]
fn test_get_active_effects_returns_reference() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 0.5);
                p
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    let active_effects1 = engine.get_active_effects();
    let active_effects2 = engine.get_active_effects();

    // Should return the same reference (or at least same contents)
    assert_eq!(active_effects1.len(), active_effects2.len());
    assert_eq!(active_effects1.len(), 1);
}

#[test]
fn test_get_fixture_states_returns_empty_initially() {
    let engine = EffectEngine::new();
    // Before any update, states should be empty
    let states = engine.get_fixture_states();
    assert!(states.is_empty());
}

#[test]
fn test_get_fixture_states_returns_states_after_update() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Before starting effects and updating, states should be empty
    let states_before = engine.get_fixture_states();
    assert!(states_before.is_empty());

    // Start an effect
    let mut parameters = HashMap::new();
    parameters.insert("dimmer".to_string(), 0.5);
    parameters.insert("red".to_string(), 1.0);
    parameters.insert("green".to_string(), 0.8);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Static {
            parameters: parameters.clone(),
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // Update the engine to process effects
    engine.update(Duration::from_millis(100), None).unwrap();

    // Now get_fixture_states should return the fixture states
    let states_after = engine.get_fixture_states();
    assert!(!states_after.is_empty());
    assert!(states_after.contains_key("test_fixture"));

    // Verify the fixture state has the expected channel values
    let fixture_state = states_after.get("test_fixture").unwrap();
    assert!(fixture_state.channels.contains_key("dimmer"));
    assert!(fixture_state.channels.contains_key("red"));
    assert!(fixture_state.channels.contains_key("green"));

    // Check values (0.5 for dimmer, 1.0 for red, 0.8 for green)
    let dimmer_value = fixture_state.channels.get("dimmer").unwrap().value;
    assert!((dimmer_value - 0.5).abs() < 0.01);

    let red_value = fixture_state.channels.get("red").unwrap().value;
    assert!((red_value - 1.0).abs() < 0.01);

    let green_value = fixture_state.channels.get("green").unwrap().value;
    assert!((green_value - 0.8).abs() < 0.01);
}

#[test]
fn test_get_fixture_states_reflects_latest_update() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start with one effect
    let mut parameters1 = HashMap::new();
    parameters1.insert("red".to_string(), 1.0);
    let effect1 = EffectInstance::new(
        "effect1".to_string(),
        EffectType::Static {
            parameters: parameters1,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(effect1).unwrap();
    engine.update(Duration::from_millis(100), None).unwrap();

    let states1 = engine.get_fixture_states();
    let red1 = states1
        .get("test_fixture")
        .and_then(|s| s.channels.get("red"))
        .map(|c| c.value)
        .unwrap_or(0.0);
    assert!((red1 - 1.0).abs() < 0.01);

    // Start a new effect that changes the color
    let mut parameters2 = HashMap::new();
    parameters2.insert("red".to_string(), 0.0);
    parameters2.insert("blue".to_string(), 1.0);
    let effect2 = EffectInstance::new(
        "effect2".to_string(),
        EffectType::Static {
            parameters: parameters2,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(effect2).unwrap();
    engine.update(Duration::from_millis(100), None).unwrap();

    // States should reflect the new effect
    let states2 = engine.get_fixture_states();
    let red2 = states2
        .get("test_fixture")
        .and_then(|s| s.channels.get("red"))
        .map(|c| c.value)
        .unwrap_or(1.0);
    let blue2 = states2
        .get("test_fixture")
        .and_then(|s| s.channels.get("blue"))
        .map(|c| c.value)
        .unwrap_or(0.0);

    // Red should be 0, blue should be 1.0
    assert!((red2 - 0.0).abs() < 0.01);
    assert!((blue2 - 1.0).abs() < 0.01);
}

#[test]
fn test_get_fixture_states_cleared_on_stop_all() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start an effect and update
    let mut parameters = HashMap::new();
    parameters.insert("red".to_string(), 1.0);
    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Static {
            parameters,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(effect).unwrap();
    engine.update(Duration::from_millis(100), None).unwrap();

    // States should exist
    let states_before = engine.get_fixture_states();
    assert!(!states_before.is_empty());

    // Stop all effects
    engine.stop_all_effects();

    // States should be cleared
    let states_after = engine.get_fixture_states();
    assert!(states_after.is_empty());
}

#[test]
fn test_get_fixture_states_multiple_fixtures() {
    let mut engine = EffectEngine::new();
    let fixture1 = create_test_fixture("fixture1", 1, 1);
    let fixture2 = create_test_fixture("fixture2", 1, 10);
    engine.register_fixture(fixture1);
    engine.register_fixture(fixture2);

    // Start effect on fixture1
    let mut parameters1 = HashMap::new();
    parameters1.insert("red".to_string(), 1.0);
    let effect1 = EffectInstance::new(
        "effect1".to_string(),
        EffectType::Static {
            parameters: parameters1,
            duration: None,
        },
        vec!["fixture1".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(effect1).unwrap();

    // Start different effect on fixture2
    let mut parameters2 = HashMap::new();
    parameters2.insert("blue".to_string(), 1.0);
    let effect2 = EffectInstance::new(
        "effect2".to_string(),
        EffectType::Static {
            parameters: parameters2,
            duration: None,
        },
        vec!["fixture2".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(effect2).unwrap();

    engine.update(Duration::from_millis(100), None).unwrap();

    let states = engine.get_fixture_states();
    assert_eq!(states.len(), 2);
    assert!(states.contains_key("fixture1"));
    assert!(states.contains_key("fixture2"));

    // Verify fixture1 has red
    let state1 = states.get("fixture1").unwrap();
    assert!(state1.channels.contains_key("red"));
    let red = state1.channels.get("red").unwrap().value;
    assert!((red - 1.0).abs() < 0.01);

    // Verify fixture2 has blue
    let state2 = states.get("fixture2").unwrap();
    assert!(state2.channels.contains_key("blue"));
    let blue = state2.channels.get("blue").unwrap().value;
    assert!((blue - 1.0).abs() < 0.01);
}

#[test]
fn test_update_with_zero_duration() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 0.5);
                p
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // Update with zero duration should not panic
    let commands = engine.update(Duration::ZERO, None).unwrap();
    // Should still have commands (effect is still active)
    assert!(!commands.is_empty());
}

#[test]
fn test_update_with_very_large_duration() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 0.5);
                p
            },
            duration: Some(Duration::from_secs(1)),
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // Update with very large duration (longer than effect duration)
    let commands = engine.update(Duration::from_secs(100), None).unwrap();
    // Effect should have completed, so no commands
    assert!(commands.is_empty());
}

#[test]
fn test_update_with_no_active_effects() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Update with no effects should not panic
    let commands = engine.update(Duration::from_millis(100), None).unwrap();
    assert!(commands.is_empty());
}

#[test]
fn test_update_with_none_song_time() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 0.5);
                p
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // Update with None song_time should work fine
    let commands = engine.update(Duration::from_millis(100), None).unwrap();
    assert!(!commands.is_empty());
}

#[test]
fn test_multiple_fixture_registration() {
    let mut engine = EffectEngine::new();

    // Register multiple fixtures
    let fixture1 = create_test_fixture("fixture1", 1, 1);
    let fixture2 = create_test_fixture("fixture2", 1, 10);
    let fixture3 = create_test_fixture("fixture3", 2, 1);

    engine.register_fixture(fixture1);
    engine.register_fixture(fixture2);
    engine.register_fixture(fixture3);

    // All fixtures should be usable
    let effect1 = EffectInstance::new(
        "effect1".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 0.5);
                p
            },
            duration: None,
        },
        vec!["fixture1".to_string()],
        None,
        None,
        None,
    );

    let effect2 = EffectInstance::new(
        "effect2".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("red".to_string(), 1.0);
                p
            },
            duration: None,
        },
        vec!["fixture2".to_string()],
        None,
        None,
        None,
    );

    let effect3 = EffectInstance::new(
        "effect3".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("green".to_string(), 1.0);
                p
            },
            duration: None,
        },
        vec!["fixture3".to_string()],
        None,
        None,
        None,
    );

    assert!(engine.start_effect(effect1).is_ok());
    assert!(engine.start_effect(effect2).is_ok());
    assert!(engine.start_effect(effect3).is_ok());

    assert_eq!(engine.active_effects_count(), 3);
}

#[test]
fn test_register_same_fixture_name_twice() {
    let mut engine = EffectEngine::new();

    let fixture1 = create_test_fixture("test_fixture", 1, 1);
    let fixture2 = create_test_fixture("test_fixture", 1, 10); // Same name, different address

    engine.register_fixture(fixture1);
    engine.register_fixture(fixture2); // Should overwrite the first one

    // Should be able to use the fixture (with the second registration's properties)
    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 0.5);
                p
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    assert!(engine.start_effect(effect).is_ok());
}

#[test]
fn test_get_layer_masters_default_to_one() {
    let engine = EffectEngine::new();

    // All layers should default to 1.0
    assert!((engine.get_layer_intensity_master(EffectLayer::Background) - 1.0).abs() < 0.01);
    assert!((engine.get_layer_intensity_master(EffectLayer::Midground) - 1.0).abs() < 0.01);
    assert!((engine.get_layer_intensity_master(EffectLayer::Foreground) - 1.0).abs() < 0.01);

    assert!((engine.get_layer_speed_master(EffectLayer::Background) - 1.0).abs() < 0.01);
    assert!((engine.get_layer_speed_master(EffectLayer::Midground) - 1.0).abs() < 0.01);
    assert!((engine.get_layer_speed_master(EffectLayer::Foreground) - 1.0).abs() < 0.01);
}

#[test]
fn test_get_layer_masters_after_setting() {
    let mut engine = EffectEngine::new();

    engine.set_layer_intensity_master(EffectLayer::Background, 0.5);
    engine.set_layer_speed_master(EffectLayer::Midground, 2.0);

    assert!((engine.get_layer_intensity_master(EffectLayer::Background) - 0.5).abs() < 0.01);
    assert!((engine.get_layer_speed_master(EffectLayer::Midground) - 2.0).abs() < 0.01);

    // Other layers should still be default
    assert!((engine.get_layer_intensity_master(EffectLayer::Foreground) - 1.0).abs() < 0.01);
    assert!((engine.get_layer_speed_master(EffectLayer::Background) - 1.0).abs() < 0.01);
}

#[test]
fn test_set_tempo_map_none() {
    let mut engine = EffectEngine::new();

    // Setting tempo map to None should work
    engine.set_tempo_map(None);

    // Should be able to start effects without tempo map
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 0.5);
                p
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    assert!(engine.start_effect(effect).is_ok());
}

#[test]
fn test_effect_with_empty_target_fixtures() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Effect with empty target fixtures - validation doesn't reject it,
    // but it won't produce any DMX commands
    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 0.5);
                p
            },
            duration: None,
        },
        vec![], // Empty target fixtures
        None,
        None,
        None,
    );

    // Validation passes (empty list just means no fixtures to check)
    let result = engine.start_effect(effect);
    assert!(result.is_ok());

    // But update should produce no commands since there are no target fixtures
    let commands = engine.update(Duration::from_millis(100), None).unwrap();
    assert!(commands.is_empty());
}

#[test]
fn test_update_multiple_times_sequential() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 0.5);
                p
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // Multiple sequential updates should work
    let commands1 = engine.update(Duration::from_millis(16), None).unwrap();
    let commands2 = engine.update(Duration::from_millis(16), None).unwrap();
    let commands3 = engine.update(Duration::from_millis(16), None).unwrap();

    // All should produce commands
    assert!(!commands1.is_empty());
    assert!(!commands2.is_empty());
    assert!(!commands3.is_empty());
}

#[test]
fn test_active_effects_count_matches_get_active_effects() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let mut effect1 = EffectInstance::new(
        "effect1".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 0.5);
                p
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    effect1.layer = EffectLayer::Background;

    let mut effect2 = EffectInstance::new(
        "effect2".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("red".to_string(), 1.0);
                p
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    effect2.layer = EffectLayer::Midground;

    engine.start_effect(effect1).unwrap();
    assert_eq!(engine.active_effects_count(), 1);
    assert_eq!(engine.get_active_effects().len(), 1);

    engine.start_effect(effect2).unwrap();
    assert_eq!(engine.active_effects_count(), 2);
    assert_eq!(engine.get_active_effects().len(), 2);
}
