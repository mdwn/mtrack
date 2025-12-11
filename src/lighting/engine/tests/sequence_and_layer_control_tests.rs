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
fn test_stop_sequence_stops_all_effects_from_sequence() {
    let mut engine = EffectEngine::new();
    let fixture1 = create_test_fixture("test_fixture_1", 1, 1);
    let fixture2 = create_test_fixture("test_fixture_2", 1, 10);
    engine.register_fixture(fixture1);
    engine.register_fixture(fixture2);

    // Create effects with sequence IDs on different fixtures to avoid conflicts
    let mut seq1_effect1 = EffectInstance::new(
        "seq_intro_effect_1".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 0.5);
                p
            },
            duration: None,
        },
        vec!["test_fixture_1".to_string()],
        None,
        None,
        None,
    );
    seq1_effect1.id = "seq_intro_effect_1".to_string();
    seq1_effect1.layer = EffectLayer::Background;

    let mut seq1_effect2 = EffectInstance::new(
        "seq_intro_effect_2".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("red".to_string(), 1.0);
                p
            },
            duration: None,
        },
        vec!["test_fixture_1".to_string()],
        None,
        None,
        None,
    );
    seq1_effect2.id = "seq_intro_effect_2".to_string();
    seq1_effect2.layer = EffectLayer::Midground;

    // Create an effect from a different sequence (on different fixture to avoid conflict)
    let mut seq2_effect = EffectInstance::new(
        "seq_outro_effect_1".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("green".to_string(), 1.0);
                p
            },
            duration: None,
        },
        vec!["test_fixture_2".to_string()],
        None,
        None,
        None,
    );
    seq2_effect.id = "seq_outro_effect_1".to_string();
    seq2_effect.layer = EffectLayer::Background;

    // Create a non-sequence effect
    let mut regular_effect = EffectInstance::new(
        "regular_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("blue".to_string(), 1.0);
                p
            },
            duration: None,
        },
        vec!["test_fixture_2".to_string()],
        None,
        None,
        None,
    );
    regular_effect.id = "regular_effect".to_string();
    regular_effect.layer = EffectLayer::Foreground;

    engine.start_effect(seq1_effect1).unwrap();
    engine.start_effect(seq1_effect2).unwrap();
    engine.start_effect(seq2_effect).unwrap();
    engine.start_effect(regular_effect).unwrap();

    assert_eq!(engine.active_effects_count(), 4);

    // Stop the "intro" sequence
    engine.stop_sequence("intro");

    // Should have stopped both intro effects, but kept outro and regular
    assert_eq!(engine.active_effects_count(), 2);
    assert!(!engine.has_effect("seq_intro_effect_1"));
    assert!(!engine.has_effect("seq_intro_effect_2"));
    assert!(engine.has_effect("seq_outro_effect_1"));
    assert!(engine.has_effect("regular_effect"));
}

#[test]
fn test_stop_sequence_handles_releasing_effects() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Create an effect that will be releasing
    let mut seq_effect = EffectInstance::new(
        "seq_test_effect_1".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 1.0);
                p
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    seq_effect.id = "seq_test_effect_1".to_string();
    seq_effect.layer = EffectLayer::Foreground;

    engine.start_effect(seq_effect).unwrap();
    assert_eq!(engine.active_effects_count(), 1);

    // Start releasing the layer (adds effect to releasing_effects, but keeps it in active_effects)
    engine.release_layer(EffectLayer::Foreground);

    // Effect should still be in active_effects (release doesn't remove it)
    assert_eq!(engine.active_effects_count(), 1);
    engine.update(Duration::from_millis(100), None).unwrap();

    // Stop the sequence - should remove from both active_effects and releasing_effects
    // The effect ID is "seq_test_effect_1", so sequence name "test" should match
    engine.stop_sequence("test");

    // Effect should be completely gone (both from active and releasing)
    assert_eq!(engine.active_effects_count(), 0);
    // Verify the effect is not in the engine at all
    assert!(!engine.has_effect("seq_test_effect_1"));
}

#[test]
fn test_stop_sequence_with_no_matching_effects() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Create a regular effect
    let regular_effect = EffectInstance::new(
        "regular_effect".to_string(),
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

    engine.start_effect(regular_effect).unwrap();
    assert_eq!(engine.active_effects_count(), 1);

    // Stop a non-existent sequence - should not panic or affect existing effects
    engine.stop_sequence("nonexistent");

    assert_eq!(engine.active_effects_count(), 1);
    assert!(engine.has_effect("regular_effect"));
}

#[test]
fn test_stop_sequence_with_empty_engine() {
    let mut engine = EffectEngine::new();

    // Stop a sequence when engine is empty - should not panic
    engine.stop_sequence("any_sequence");

    assert_eq!(engine.active_effects_count(), 0);
}

#[test]
fn test_clear_all_layers_stops_all_effects() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start effects on all layers
    let mut bg_effect = EffectInstance::new(
        "bg_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 0.3);
                p
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    bg_effect.layer = EffectLayer::Background;

    let mut mid_effect = EffectInstance::new(
        "mid_effect".to_string(),
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
    mid_effect.layer = EffectLayer::Midground;

    let mut fg_effect = EffectInstance::new(
        "fg_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("green".to_string(), 1.0);
                p
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    fg_effect.layer = EffectLayer::Foreground;

    engine.start_effect(bg_effect).unwrap();
    engine.start_effect(mid_effect).unwrap();
    engine.start_effect(fg_effect).unwrap();

    assert_eq!(engine.active_effects_count(), 3);

    // Clear all layers
    engine.clear_all_layers();

    // All effects should be stopped
    assert_eq!(engine.active_effects_count(), 0);
    assert!(!engine.has_effect("bg_effect"));
    assert!(!engine.has_effect("mid_effect"));
    assert!(!engine.has_effect("fg_effect"));
}

#[test]
fn test_clear_all_layers_clears_frozen_layers() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start an effect and freeze it
    let mut bg_effect = EffectInstance::new(
        "bg_effect".to_string(),
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
    bg_effect.layer = EffectLayer::Background;

    engine.start_effect(bg_effect).unwrap();
    engine.freeze_layer(EffectLayer::Background);

    assert!(engine.is_layer_frozen(EffectLayer::Background));
    assert_eq!(engine.active_effects_count(), 1);

    // Clear all layers
    engine.clear_all_layers();

    // Layer should no longer be frozen and effect should be gone
    assert!(!engine.is_layer_frozen(EffectLayer::Background));
    assert_eq!(engine.active_effects_count(), 0);
}

#[test]
fn test_clear_all_layers_clears_releasing_effects() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start effects on different layers
    let mut bg_effect = EffectInstance::new(
        "bg_effect".to_string(),
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
    bg_effect.layer = EffectLayer::Background;

    let mut fg_effect = EffectInstance::new(
        "fg_effect".to_string(),
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
    fg_effect.layer = EffectLayer::Foreground;

    engine.start_effect(bg_effect).unwrap();
    engine.start_effect(fg_effect).unwrap();

    // Start releasing one layer
    engine.release_layer(EffectLayer::Foreground);

    // Update to move effect to releasing state
    engine.update(Duration::from_millis(100), None).unwrap();

    // Clear all layers - should stop both active and releasing effects
    engine.clear_all_layers();

    assert_eq!(engine.active_effects_count(), 0);
}

#[test]
fn test_clear_all_layers_clears_fixture_states_and_locks() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start a foreground Replace effect that will create channel locks
    // Use a permanent effect (no duration) so its state persists after completion
    let mut fg_effect = EffectInstance::new(
        "fg_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("red".to_string(), 1.0);
                p.insert("blue".to_string(), 1.0);
                p
            },
            duration: None, // Permanent effect
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    fg_effect.layer = EffectLayer::Foreground;
    fg_effect.blend_mode = BlendMode::Replace;

    engine.start_effect(fg_effect).unwrap();

    // Process the effect to create fixture states and channel locks
    let commands_before = engine.update(Duration::from_millis(100), None).unwrap();
    // Should have DMX commands for red and blue channels
    assert!(
        !commands_before.is_empty(),
        "Should have DMX commands after processing effect"
    );

    // Clear all layers - this should clear both fixture_states and channel_locks
    engine.clear_all_layers();

    // Verify the effect is stopped
    assert_eq!(engine.active_effects_count(), 0);

    // Verify that fixture states and locks are cleared by checking that
    // a new effect on a lower layer can now control the same channels
    let mut bg_effect = EffectInstance::new(
        "bg_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("green".to_string(), 1.0); // Different color to verify it works
                p
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    bg_effect.layer = EffectLayer::Background;
    bg_effect.blend_mode = BlendMode::Replace;

    // Should be able to start the effect (locks should be cleared)
    engine.start_effect(bg_effect).unwrap();
    let commands_after = engine.update(Duration::from_millis(100), None).unwrap();
    // Should have DMX commands for green (proving locks were cleared)
    assert!(
        !commands_after.is_empty(),
        "Should be able to control channels after clear_all_layers"
    );
}

#[test]
fn test_clear_all_layers_with_empty_engine() {
    let mut engine = EffectEngine::new();

    // Clear all layers when engine is empty - should not panic
    engine.clear_all_layers();

    assert_eq!(engine.active_effects_count(), 0);
}

#[test]
fn test_freeze_unfreeze_multiple_effects_same_layer() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Create RGB fixture for color effects
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    let rgb_fixture = FixtureInfo {
        name: "rgb_fixture".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "RGB".to_string(),
        channels,
        max_strobe_frequency: None,
    };
    engine.register_fixture(rgb_fixture);

    // Start multiple effects on the same layer
    let mut effect1 = EffectInstance::new(
        "effect1".to_string(),
        EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(1.0),
            saturation: 1.0,
            brightness: 1.0,
        },
        vec!["rgb_fixture".to_string()],
        None,
        None,
        None,
    );
    effect1.layer = EffectLayer::Background;

    let mut effect2 = EffectInstance::new(
        "effect2".to_string(),
        EffectType::ColorCycle {
            colors: vec![
                Color::new(255, 0, 0),
                Color::new(0, 255, 0),
                Color::new(0, 0, 255),
            ],
            speed: TempoAwareSpeed::Fixed(2.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
        },
        vec!["rgb_fixture".to_string()],
        None,
        None,
        None,
    );
    effect2.layer = EffectLayer::Background;

    engine.start_effect(effect1).unwrap();
    engine.start_effect(effect2).unwrap();

    // Let effects run
    let _commands1 = engine.update(Duration::from_millis(200), None).unwrap();
    let commands_before = engine.update(Duration::from_millis(10), None).unwrap();

    // Freeze the layer
    engine.freeze_layer(EffectLayer::Background);
    assert!(engine.is_layer_frozen(EffectLayer::Background));

    // Update multiple times - values should stay frozen
    let commands_frozen1 = engine.update(Duration::from_millis(500), None).unwrap();
    let commands_frozen2 = engine.update(Duration::from_millis(500), None).unwrap();

    // Frozen commands should match (or be very close due to rounding)
    // Sort by channel for comparison
    let mut before_sorted: Vec<_> = commands_before.iter().collect();
    before_sorted.sort_by_key(|c| c.channel);
    let mut frozen1_sorted: Vec<_> = commands_frozen1.iter().collect();
    frozen1_sorted.sort_by_key(|c| c.channel);
    let mut frozen2_sorted: Vec<_> = commands_frozen2.iter().collect();
    frozen2_sorted.sort_by_key(|c| c.channel);

    // Values should be the same (or very close)
    assert_eq!(frozen1_sorted.len(), frozen2_sorted.len());
    for (f1, f2) in frozen1_sorted.iter().zip(frozen2_sorted.iter()) {
        assert_eq!(f1.channel, f2.channel);
        // Allow small difference due to floating point
        assert!(
            (f1.value as i32 - f2.value as i32).abs() <= 1,
            "Frozen values should match: {} vs {}",
            f1.value,
            f2.value
        );
    }

    // Unfreeze
    engine.unfreeze_layer(EffectLayer::Background);
    assert!(!engine.is_layer_frozen(EffectLayer::Background));

    // Effects should resume and progress
    let commands_after = engine.update(Duration::from_millis(200), None).unwrap();
    // After unfreezing, values should have changed
    assert!(!commands_after.is_empty());
}

#[test]
fn test_freeze_unfreeze_different_layers_independently() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start effects on different layers
    let mut bg_effect = EffectInstance::new(
        "bg_effect".to_string(),
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
    bg_effect.layer = EffectLayer::Background;

    let mut fg_effect = EffectInstance::new(
        "fg_effect".to_string(),
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
    fg_effect.layer = EffectLayer::Foreground;

    engine.start_effect(bg_effect).unwrap();
    engine.start_effect(fg_effect).unwrap();

    // Freeze only background
    engine.freeze_layer(EffectLayer::Background);

    assert!(engine.is_layer_frozen(EffectLayer::Background));
    assert!(!engine.is_layer_frozen(EffectLayer::Foreground));

    // Unfreeze background, freeze foreground
    engine.unfreeze_layer(EffectLayer::Background);
    engine.freeze_layer(EffectLayer::Foreground);

    assert!(!engine.is_layer_frozen(EffectLayer::Background));
    assert!(engine.is_layer_frozen(EffectLayer::Foreground));

    // Both effects should still be active
    assert_eq!(engine.active_effects_count(), 2);
}

#[test]
fn test_freeze_unfreeze_with_releasing_effects() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start an effect
    let mut effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 1.0);
                p
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    effect.layer = EffectLayer::Background;

    engine.start_effect(effect).unwrap();

    // Freeze the layer
    engine.freeze_layer(EffectLayer::Background);
    assert!(engine.is_layer_frozen(EffectLayer::Background));

    // Start releasing - should unfreeze automatically
    engine.release_layer(EffectLayer::Background);

    // Layer should no longer be frozen (release clears freeze)
    assert!(!engine.is_layer_frozen(EffectLayer::Background));
}
