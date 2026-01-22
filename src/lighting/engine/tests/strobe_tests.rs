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
use crate::lighting::effects::*;
use crate::lighting::engine::tests::common::create_test_fixture;
use crate::lighting::engine::EffectEngine;
use std::collections::HashMap;
use std::time::Duration;

#[test]
fn test_strobe_boundary_at_duty_cycle_transition() {
    // Test strobe behavior at exactly the 50% duty cycle boundary
    // strobe_phase < 0.5 means ON, >= 0.5 means OFF
    let mut engine = EffectEngine::new();

    // Create a fixture WITHOUT hardware strobe capability to test software strobe
    let mut channels = HashMap::new();
    channels.insert("dimmer".to_string(), 1);
    channels.insert("red".to_string(), 2);
    channels.insert("green".to_string(), 3);
    channels.insert("blue".to_string(), 4);

    let fixture = FixtureInfo {
        name: "test_fixture".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "RGB".to_string(),
        channels,
        max_strobe_frequency: None, // No hardware strobe
    };
    engine.register_fixture(fixture);

    // 2 Hz strobe = 500ms period, so 50% duty cycle transition at 250ms
    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(2.0),
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // At t=0ms: strobe_phase=0, which is < 0.5, so ON (dimmer=255)
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(dimmer_cmd.value, 255, "At t=0ms strobe should be ON");

    // At t=249ms: still in first half of period, should be ON
    let commands = engine.update(Duration::from_millis(249), None).unwrap();
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(
        dimmer_cmd.value, 255,
        "At t=249ms strobe should still be ON"
    );

    // At t=251ms: just past 50% of period, should be OFF
    let commands = engine.update(Duration::from_millis(2), None).unwrap();
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(dimmer_cmd.value, 0, "At t=251ms strobe should be OFF");

    // At t=500ms: start of new period, should be ON again
    let commands = engine.update(Duration::from_millis(249), None).unwrap();
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(
        dimmer_cmd.value, 255,
        "At t=500ms strobe should be ON again"
    );
}

#[test]
fn test_strobe_effect() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(2.0), // 2 Hz
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // Update the engine
    let commands = engine.update(Duration::from_millis(16), None).unwrap();

    // Should have strobe command since fixture has dedicated strobe channel
    assert_eq!(commands.len(), 1);

    // Check strobe command (frequency 2.0 / max 20.0 = 0.1 = 25 in DMX)
    let strobe_cmd = commands.iter().find(|cmd| cmd.channel == 6).unwrap();
    assert_eq!(strobe_cmd.value, 25);
}

#[test]
fn test_clear_layer_resets_strobe_channel() {
    // Test that clearing a layer with a strobe effect resets the strobe channel to 0
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start a strobe effect on foreground layer
    let mut strobe_effect = EffectInstance::new(
        "strobe_effect".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(5.0), // 5 Hz
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    strobe_effect.layer = EffectLayer::Foreground;

    engine.start_effect(strobe_effect).unwrap();

    // Update to process the strobe effect
    let commands_before = engine.update(Duration::from_millis(16), None).unwrap();

    // Verify strobe channel has a non-zero value
    let strobe_cmd_before = commands_before.iter().find(|cmd| cmd.channel == 6);
    assert!(
        strobe_cmd_before.is_some(),
        "Should have strobe command before clear"
    );
    let strobe_value_before = strobe_cmd_before.unwrap().value;
    assert!(
        strobe_value_before > 0,
        "Strobe channel should be non-zero before clear: {}",
        strobe_value_before
    );

    // Clear the foreground layer
    engine.clear_layer(EffectLayer::Foreground);

    // Verify the effect is stopped
    assert_eq!(engine.active_effects_count(), 0);
    assert!(!engine.has_effect("strobe_effect"));

    // Update again - strobe channel should now be 0
    let commands_after = engine.update(Duration::from_millis(16), None).unwrap();

    // Find the strobe command
    let strobe_cmd_after = commands_after.iter().find(|cmd| cmd.channel == 6);
    assert!(
        strobe_cmd_after.is_some(),
        "Should have strobe command after clear (to reset it to 0)"
    );
    assert_eq!(
        strobe_cmd_after.unwrap().value,
        0,
        "Strobe channel should be reset to 0 after clear"
    );
}

#[test]
fn test_clear_all_layers_resets_strobe_channel() {
    // Test that clearing all layers with strobe effects resets strobe channels to 0
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start strobe effects on multiple layers
    let mut bg_strobe = EffectInstance::new(
        "bg_strobe".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(3.0),
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    bg_strobe.layer = EffectLayer::Background;

    let mut fg_strobe = EffectInstance::new(
        "fg_strobe".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(4.0),
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    fg_strobe.layer = EffectLayer::Foreground;

    engine.start_effect(bg_strobe).unwrap();
    engine.start_effect(fg_strobe).unwrap();

    // Update to process the strobe effects
    let commands_before = engine.update(Duration::from_millis(16), None).unwrap();

    // Verify strobe channel has a non-zero value
    let strobe_cmd_before = commands_before.iter().find(|cmd| cmd.channel == 6);
    assert!(
        strobe_cmd_before.is_some(),
        "Should have strobe command before clear"
    );
    let strobe_value_before = strobe_cmd_before.unwrap().value;
    assert!(
        strobe_value_before > 0,
        "Strobe channel should be non-zero before clear: {}",
        strobe_value_before
    );

    // Clear all layers
    engine.clear_all_layers();

    // Verify all effects are stopped
    assert_eq!(engine.active_effects_count(), 0);

    // Update again - strobe channel should now be 0
    let commands_after = engine.update(Duration::from_millis(16), None).unwrap();

    // Find the strobe command
    let strobe_cmd_after = commands_after.iter().find(|cmd| cmd.channel == 6);
    assert!(
        strobe_cmd_after.is_some(),
        "Should have strobe command after clear (to reset it to 0)"
    );
    assert_eq!(
        strobe_cmd_after.unwrap().value,
        0,
        "Strobe channel should be reset to 0 after clear_all_layers"
    );
}
