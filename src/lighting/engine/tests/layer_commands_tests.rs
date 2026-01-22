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
fn test_clear_layer() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start effects on different layers
    let bg_effect = EffectInstance::new(
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

    let mut fg_effect = EffectInstance::new(
        "fg_effect".to_string(),
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
    fg_effect.layer = EffectLayer::Foreground;

    engine.start_effect(bg_effect).unwrap();
    engine.start_effect(fg_effect).unwrap();
    assert_eq!(engine.active_effects_count(), 2);

    // Clear foreground layer
    engine.clear_layer(EffectLayer::Foreground);
    assert_eq!(engine.active_effects_count(), 1);
    assert!(engine.has_effect("bg_effect"));
    assert!(!engine.has_effect("fg_effect"));
}

#[test]
fn test_freeze_unfreeze_layer() {
    let mut engine = EffectEngine::new();

    // Create RGB fixture for rainbow test
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    let fixture = FixtureInfo {
        name: "rgb_fixture".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "RGB".to_string(),
        channels,
        max_strobe_frequency: None,
    };
    engine.register_fixture(fixture);

    // Start a rainbow effect - it cycles through colors over time
    let effect = EffectInstance::new(
        "bg_effect".to_string(),
        EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
            saturation: 1.0,
            brightness: 1.0,
        },
        vec!["rgb_fixture".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(effect).unwrap();

    // Let the effect run for a bit to get to an interesting state
    let _commands1 = engine.update(Duration::from_millis(250), None).unwrap();

    // Capture the state at this point
    let commands_before_freeze = engine.update(Duration::from_millis(10), None).unwrap();
    assert!(!commands_before_freeze.is_empty());

    // Freeze the background layer
    engine.freeze_layer(EffectLayer::Background);
    assert!(engine.is_layer_frozen(EffectLayer::Background));

    // Update multiple times - the values should stay the same while frozen
    let commands_frozen1 = engine.update(Duration::from_millis(100), None).unwrap();
    let commands_frozen2 = engine.update(Duration::from_millis(100), None).unwrap();
    let commands_frozen3 = engine.update(Duration::from_millis(500), None).unwrap();

    assert!(!commands_frozen1.is_empty());
    assert!(!commands_frozen2.is_empty());
    assert!(!commands_frozen3.is_empty());

    // All frozen commands should have the same values
    // Sort by channel to ensure consistent comparison
    let mut vals1: Vec<u8> = commands_frozen1.iter().map(|c| c.value).collect();
    let mut vals2: Vec<u8> = commands_frozen2.iter().map(|c| c.value).collect();
    let mut vals3: Vec<u8> = commands_frozen3.iter().map(|c| c.value).collect();
    vals1.sort();
    vals2.sort();
    vals3.sort();

    assert_eq!(
        vals1, vals2,
        "Frozen layer should produce same values: {:?} vs {:?}",
        vals1, vals2
    );
    assert_eq!(
        vals2, vals3,
        "Frozen layer should produce same values: {:?} vs {:?}",
        vals2, vals3
    );

    // Unfreeze the layer
    engine.unfreeze_layer(EffectLayer::Background);
    assert!(!engine.is_layer_frozen(EffectLayer::Background));

    // After unfreezing, the effect should resume and values should change
    let commands_after1 = engine.update(Duration::from_millis(100), None).unwrap();
    let commands_after2 = engine.update(Duration::from_millis(200), None).unwrap();

    assert!(!commands_after1.is_empty());
    assert!(!commands_after2.is_empty());

    // Values should be different after unfreezing and time passing
    let mut vals_after1: Vec<u8> = commands_after1.iter().map(|c| c.value).collect();
    let mut vals_after2: Vec<u8> = commands_after2.iter().map(|c| c.value).collect();
    vals_after1.sort();
    vals_after2.sort();

    // The effect should be animating, so values should differ
    // (with a 200ms gap at 1 cycle/sec, hue shifts about 72 degrees)
    assert_ne!(
        vals_after1, vals_after2,
        "After unfreezing, effect should animate: {:?} vs {:?}",
        vals_after1, vals_after2
    );
}

#[test]
fn test_release_frozen_layer_maintains_animation_continuity() {
    // Regression test: releasing a frozen layer should not cause animation discontinuity.
    // Before the fix, release_layer_with_time would call frozen_layers.remove() directly
    // instead of unfreeze_layer(), causing effects to jump forward in their animation.
    let mut engine = EffectEngine::new();

    // Create RGB fixture for rainbow test
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    let fixture = FixtureInfo {
        name: "rgb_fixture".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "RGB".to_string(),
        channels,
        max_strobe_frequency: None,
    };
    engine.register_fixture(fixture);

    // Start a rainbow effect - it cycles through colors over time
    let effect = EffectInstance::new(
        "rainbow".to_string(),
        EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
            saturation: 1.0,
            brightness: 1.0,
        },
        vec!["rgb_fixture".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(effect).unwrap();

    // Run effect to get to an interesting state (250ms into the cycle)
    engine.update(Duration::from_millis(250), None).unwrap();

    // Capture the current state (before freeze)
    let _commands_before_freeze = engine.update(Duration::from_millis(10), None).unwrap();

    // Freeze the layer
    engine.freeze_layer(EffectLayer::Background);

    // Let significant time pass while frozen (1 second = full cycle if not frozen)
    engine.update(Duration::from_millis(500), None).unwrap();
    engine.update(Duration::from_millis(500), None).unwrap();

    // Capture the frozen state (should be same as before freeze)
    let commands_frozen = engine.update(Duration::from_millis(10), None).unwrap();
    // Sort by channel for consistent comparison (DMX commands may be returned in any order)
    let mut frozen_sorted: Vec<_> = commands_frozen
        .iter()
        .map(|c| (c.channel, c.value))
        .collect();
    frozen_sorted.sort_by_key(|(ch, _)| *ch);
    let vals_frozen: Vec<u8> = frozen_sorted.iter().map(|(_, v)| *v).collect();

    // Now release the frozen layer with a fade time
    engine.release_layer_with_time(EffectLayer::Background, Some(Duration::from_secs(2)));

    // Immediately after release, the effect should continue from where it was frozen,
    // NOT jump forward by the 1 second that passed while frozen.
    let commands_after_release = engine.update(Duration::from_millis(10), None).unwrap();
    // Sort by channel for consistent comparison
    let mut after_release_sorted: Vec<_> = commands_after_release
        .iter()
        .map(|c| (c.channel, c.value))
        .collect();
    after_release_sorted.sort_by_key(|(ch, _)| *ch);
    let vals_after_release: Vec<u8> = after_release_sorted.iter().map(|(_, v)| *v).collect();

    // The values right after release should be very close to the frozen values
    // (only 10ms of animation has passed, not 1+ second)
    // We allow small differences due to the 10ms update and fade starting
    let max_diff: i16 = vals_frozen
        .iter()
        .zip(vals_after_release.iter())
        .map(|(a, b)| (*a as i16 - *b as i16).abs())
        .max()
        .unwrap_or(0);

    // If the bug exists (no start time adjustment), the rainbow would have jumped
    // forward by ~1 second in its cycle, causing a large color difference.
    // At 1 cycle/second, that's a 360 degree hue shift (back to same color)
    // but even 500ms would be 180 degrees (opposite color = huge difference).
    // With the fix, we should see only tiny differences from the 10ms elapsed.
    assert!(
        max_diff < 30,
        "Release of frozen layer caused animation discontinuity! \
     Frozen: {:?}, After release: {:?}, Max diff: {}. \
     Effect should continue from frozen state, not jump forward.",
        vals_frozen,
        vals_after_release,
        max_diff
    );

    // Also verify the effect is actually fading out over time
    engine.update(Duration::from_millis(1000), None).unwrap();
    let commands_mid_fade = engine.update(Duration::from_millis(10), None).unwrap();
    // Sort by channel for consistent comparison
    let mut mid_fade_sorted: Vec<_> = commands_mid_fade
        .iter()
        .map(|c| (c.channel, c.value))
        .collect();
    mid_fade_sorted.sort_by_key(|(ch, _)| *ch);
    let vals_mid_fade: Vec<u8> = mid_fade_sorted.iter().map(|(_, v)| *v).collect();

    // At 1 second into a 2 second fade, values should be roughly half
    let avg_mid: f64 =
        vals_mid_fade.iter().map(|v| *v as f64).sum::<f64>() / vals_mid_fade.len() as f64;
    let avg_frozen: f64 =
        vals_frozen.iter().map(|v| *v as f64).sum::<f64>() / vals_frozen.len() as f64;

    // Mid-fade average should be notably lower than frozen average
    assert!(
        avg_mid < avg_frozen * 0.8,
        "Effect should be fading: frozen avg={:.1}, mid-fade avg={:.1}",
        avg_frozen,
        avg_mid
    );
}

#[test]
fn test_layer_intensity_master() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start a static effect at 100% dimmer
    let effect = EffectInstance::new(
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
    engine.start_effect(effect).unwrap();

    // Get commands at full intensity
    let commands_full = engine.update(Duration::from_millis(16), None).unwrap();
    assert_eq!(commands_full.len(), 1);
    let full_value = commands_full[0].value;
    assert_eq!(full_value, 255); // Full intensity

    // Set layer intensity master to 50%
    engine.set_layer_intensity_master(EffectLayer::Background, 0.5);
    assert!((engine.get_layer_intensity_master(EffectLayer::Background) - 0.5).abs() < 0.01);

    // Get commands at 50% master
    let commands_half = engine.update(Duration::from_millis(16), None).unwrap();
    assert_eq!(commands_half.len(), 1);
    let half_value = commands_half[0].value;
    assert_eq!(half_value, 127); // 50% of 255
}

#[test]
fn test_layer_speed_master() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Test that speed master affects effect timing
    engine.set_layer_speed_master(EffectLayer::Background, 2.0);
    assert!((engine.get_layer_speed_master(EffectLayer::Background) - 2.0).abs() < 0.01);

    engine.set_layer_speed_master(EffectLayer::Background, 0.5);
    assert!((engine.get_layer_speed_master(EffectLayer::Background) - 0.5).abs() < 0.01);

    // Reset to default
    engine.set_layer_speed_master(EffectLayer::Background, 1.0);
    assert!((engine.get_layer_speed_master(EffectLayer::Background) - 1.0).abs() < 0.01);
}

#[test]
fn test_release_layer_fade_behavior() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start an effect on background layer
    let effect = EffectInstance::new(
        "bg_effect".to_string(),
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
    engine.start_effect(effect).unwrap();

    // Get initial commands at full brightness
    let commands_before = engine.update(Duration::from_millis(16), None).unwrap();
    assert_eq!(commands_before.len(), 1);
    assert_eq!(commands_before[0].value, 255);

    // Release the layer with a 1 second fade
    engine.release_layer_with_time(EffectLayer::Background, Some(Duration::from_secs(1)));

    // Immediately after release, should still be near full
    let commands_start = engine.update(Duration::from_millis(16), None).unwrap();
    assert!(!commands_start.is_empty());

    // Halfway through fade (500ms), should be around half brightness
    let commands_mid = engine.update(Duration::from_millis(500), None).unwrap();
    if !commands_mid.is_empty() {
        // Value should be less than full
        assert!(
            commands_mid[0].value < 200,
            "Should be fading: {}",
            commands_mid[0].value
        );
    }
}

#[test]
fn test_layer_commands_edge_cases() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Clear an empty layer - should not panic
    engine.clear_layer(EffectLayer::Foreground);
    assert_eq!(engine.active_effects_count(), 0);

    // Release an empty layer - should not panic
    engine.release_layer(EffectLayer::Midground);

    // Double freeze - should not panic
    engine.freeze_layer(EffectLayer::Background);
    engine.freeze_layer(EffectLayer::Background);
    assert!(engine.is_layer_frozen(EffectLayer::Background));

    // Unfreeze non-frozen layer - should not panic
    engine.unfreeze_layer(EffectLayer::Foreground);

    // Set intensity master multiple times
    engine.set_layer_intensity_master(EffectLayer::Background, 0.5);
    engine.set_layer_intensity_master(EffectLayer::Background, 0.75);
    assert!((engine.get_layer_intensity_master(EffectLayer::Background) - 0.75).abs() < 0.01);

    // Intensity clamping
    engine.set_layer_intensity_master(EffectLayer::Background, 1.5); // Should clamp to 1.0
    assert!((engine.get_layer_intensity_master(EffectLayer::Background) - 1.0).abs() < 0.01);

    engine.set_layer_intensity_master(EffectLayer::Background, -0.5); // Should clamp to 0.0
    assert!((engine.get_layer_intensity_master(EffectLayer::Background) - 0.0).abs() < 0.01);
}

#[test]
fn test_speed_master_affects_effect_progression() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start a pulse effect - easier to verify timing changes
    let effect = EffectInstance::new(
        "pulse".to_string(),
        EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.5,
            frequency: TempoAwareFrequency::Fixed(1.0), // 1 cycle per second
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(effect).unwrap();

    // Get initial value
    let cmd1 = engine.update(Duration::from_millis(100), None).unwrap();
    assert!(!cmd1.is_empty());
    let _initial_value = cmd1[0].value;

    // Now set speed master to 0 (effectively frozen via speed = 0)
    engine.set_layer_speed_master(EffectLayer::Background, 0.0);

    // With speed = 0, elapsed time stays at same effective position
    // So values should stay similar
    let cmd2 = engine.update(Duration::from_millis(500), None).unwrap();
    let cmd3 = engine.update(Duration::from_millis(500), None).unwrap();

    assert!(!cmd2.is_empty());
    assert!(!cmd3.is_empty());

    // With speed = 0, effect time doesn't progress, so values should be consistent
    // (allowing for small floating point differences)
    let val2 = cmd2[0].value;
    let val3 = cmd3[0].value;

    // Values should be the same when speed is 0
    assert_eq!(
        val2, val3,
        "Speed=0 should produce consistent values: {} vs {}",
        val2, val3
    );
}

#[test]
fn test_speed_master_resume_from_zero() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start a pulse effect
    let effect = EffectInstance::new(
        "pulse".to_string(),
        EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.5,
            frequency: TempoAwareFrequency::Fixed(1.0),
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(effect).unwrap();

    // Run for a bit to get to a known state
    engine.update(Duration::from_millis(250), None).unwrap();

    // Freeze with speed=0
    engine.set_layer_speed_master(EffectLayer::Background, 0.0);

    // Record frozen value
    let frozen_cmd = engine.update(Duration::from_millis(100), None).unwrap();
    let frozen_val = frozen_cmd[0].value;

    // Wait a bit while frozen
    engine.update(Duration::from_millis(500), None).unwrap();

    // Resume with speed=1
    engine.set_layer_speed_master(EffectLayer::Background, 1.0);

    // The effect should now progress from where it was frozen
    let resume_cmd1 = engine.update(Duration::from_millis(100), None).unwrap();
    let resume_cmd2 = engine.update(Duration::from_millis(100), None).unwrap();

    // After resuming, values should change (effect is running again)
    // We can't predict exact values due to sinusoidal pulse, but they should differ
    // over enough time
    let val1 = resume_cmd1[0].value;
    let val2 = resume_cmd2[0].value;

    // At least verify we got values (effect is running)
    assert!(!resume_cmd1.is_empty());
    assert!(!resume_cmd2.is_empty());

    // The frozen value should be different from at least one of the resumed values
    // (since we're now progressing through the pulse cycle)
    let changed = frozen_val != val1 || frozen_val != val2 || val1 != val2;
    assert!(
        changed,
        "Effect should progress after resume: frozen={}, val1={}, val2={}",
        frozen_val, val1, val2
    );
}

#[test]
fn test_multiple_layers_independent() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start effects on different layers
    let mut bg_effect = EffectInstance::new(
        "bg".to_string(),
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
    bg_effect.layer = EffectLayer::Background;

    let mut mid_effect = EffectInstance::new(
        "mid".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("dimmer".to_string(), 0.8);
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

    engine.start_effect(bg_effect).unwrap();
    engine.start_effect(mid_effect).unwrap();

    // Set different masters for each layer
    engine.set_layer_intensity_master(EffectLayer::Background, 0.5);
    engine.set_layer_intensity_master(EffectLayer::Midground, 1.0);

    // Freeze only background
    engine.freeze_layer(EffectLayer::Background);

    assert!(engine.is_layer_frozen(EffectLayer::Background));
    assert!(!engine.is_layer_frozen(EffectLayer::Midground));

    // Clear only midground
    engine.clear_layer(EffectLayer::Midground);

    assert_eq!(engine.active_effects_count(), 1);
    assert!(engine.has_effect("bg"));
    assert!(!engine.has_effect("mid"));
}
