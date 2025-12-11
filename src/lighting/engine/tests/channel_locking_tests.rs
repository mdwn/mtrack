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

/// Helper to get RGB values from DMX commands
/// The fixture has dimmer at address, red at address+1, green at address+2, blue at address+3
fn get_rgb(universe: u16, address: u16, commands: &[DmxCommand]) -> (u8, u8, u8) {
    let mut red = 0;
    let mut green = 0;
    let mut blue = 0;

    for cmd in commands {
        if cmd.universe == universe && cmd.channel == address + 1 {
            red = cmd.value;
        } else if cmd.universe == universe && cmd.channel == address + 2 {
            green = cmd.value;
        } else if cmd.universe == universe && cmd.channel == address + 3 {
            blue = cmd.value;
        }
    }

    (red, green, blue)
}

#[test]
fn test_channel_locking_foreground_replace_completes() {
    // Test that when a foreground Replace effect completes, channels are locked
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start a background effect with red color
    let mut bg_params = HashMap::new();
    bg_params.insert("red".to_string(), 1.0);
    bg_params.insert("green".to_string(), 0.0);
    bg_params.insert("blue".to_string(), 0.0);

    let bg_effect = EffectInstance::new(
        "bg_effect".to_string(),
        EffectType::Static {
            parameters: bg_params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(bg_effect).unwrap();

    // Update to apply background effect
    let _commands = engine.update(Duration::from_millis(16), None).unwrap();

    // Start a foreground Replace effect with blue color (permanent)
    let mut fg_params = HashMap::new();
    fg_params.insert("red".to_string(), 0.0);
    fg_params.insert("green".to_string(), 0.0);
    fg_params.insert("blue".to_string(), 1.0);

    let mut fg_effect = EffectInstance::new(
        "fg_effect".to_string(),
        EffectType::Static {
            parameters: fg_params,
            duration: None, // Permanent
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    fg_effect.layer = EffectLayer::Foreground;
    fg_effect.blend_mode = BlendMode::Replace;
    engine.start_effect(fg_effect).unwrap();

    // Update to apply foreground effect - should see blue
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    let (_r, _g, b) = get_rgb(1, 1, &commands);
    assert_eq!(b, 255, "Foreground effect should show blue");

    // Clear the old background effect first to avoid interference
    engine.clear_layer(EffectLayer::Background);
    engine.update(Duration::from_millis(1), None).unwrap();

    // Release the foreground effect - this will fade it out and complete it
    // When it completes, channels should be locked
    engine.release_layer_with_time(EffectLayer::Foreground, Some(Duration::from_millis(100)));

    // Wait for release to complete (fade time) and then one more update to process completion
    engine.update(Duration::from_millis(100), None).unwrap();
    engine.update(Duration::from_millis(1), None).unwrap();

    // Try to start a new background effect with red - should be blocked by locks
    let mut new_bg_params = HashMap::new();
    new_bg_params.insert("red".to_string(), 1.0);
    new_bg_params.insert("green".to_string(), 0.0);
    new_bg_params.insert("blue".to_string(), 0.0);

    let new_bg_effect = EffectInstance::new(
        "new_bg_effect".to_string(),
        EffectType::Static {
            parameters: new_bg_params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(new_bg_effect).unwrap();

    // Update - red should be blocked, blue should persist
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    let (r, _g, b) = get_rgb(1, 1, &commands);
    assert_eq!(r, 0, "Red channel should be locked");
    assert_eq!(b, 255, "Blue should persist from locked foreground effect");
}

#[test]
fn test_channel_locking_only_foreground_replace() {
    // Test that only foreground Replace effects lock channels
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start a foreground Replace effect
    let mut fg_params = HashMap::new();
    fg_params.insert("red".to_string(), 1.0);
    fg_params.insert("green".to_string(), 0.0);
    fg_params.insert("blue".to_string(), 0.0);

    let mut fg_replace = EffectInstance::new(
        "fg_replace".to_string(),
        EffectType::Static {
            parameters: fg_params.clone(),
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    fg_replace.layer = EffectLayer::Foreground;
    fg_replace.blend_mode = BlendMode::Replace;
    engine.start_effect(fg_replace).unwrap();
    engine.update(Duration::from_millis(16), None).unwrap();
    // Release to complete the effect and create locks
    engine.release_layer_with_time(EffectLayer::Foreground, Some(Duration::from_millis(100)));
    engine.update(Duration::from_millis(100), None).unwrap();

    // Start background effect - should be blocked
    let bg_effect = EffectInstance::new(
        "bg".to_string(),
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
    engine.start_effect(bg_effect).unwrap();
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    let (r, g, _b) = get_rgb(1, 1, &commands);
    assert_eq!(r, 255, "Red should persist (locked)");
    assert_eq!(g, 0, "Green should be blocked by lock");

    // Now test with foreground Multiply - should NOT lock
    engine.stop_all_effects();
    let mut fg_mult = EffectInstance::new(
        "fg_mult".to_string(),
        EffectType::Static {
            parameters: fg_params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    fg_mult.layer = EffectLayer::Foreground;
    fg_mult.blend_mode = BlendMode::Multiply;
    engine.start_effect(fg_mult).unwrap();
    engine.update(Duration::from_millis(16), None).unwrap();
    engine.clear_layer(EffectLayer::Foreground);

    // Background effect should work now (no locks)
    let bg_effect2 = EffectInstance::new(
        "bg2".to_string(),
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
    engine.start_effect(bg_effect2).unwrap();
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    let (_r, g, _b) = get_rgb(1, 1, &commands);
    assert_eq!(g, 255, "Green should work (no locks from Multiply)");
}

#[test]
fn test_channel_locking_dimmer_passthrough() {
    // Test that dimmer and multiplier channels pass through locks
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start foreground Replace effect to lock RGB channels
    let mut fg_params = HashMap::new();
    fg_params.insert("red".to_string(), 1.0);
    fg_params.insert("green".to_string(), 0.0);
    fg_params.insert("blue".to_string(), 0.0);

    let mut fg_effect = EffectInstance::new(
        "fg_lock".to_string(),
        EffectType::Static {
            parameters: fg_params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    fg_effect.layer = EffectLayer::Foreground;
    fg_effect.blend_mode = BlendMode::Replace;
    engine.start_effect(fg_effect).unwrap();
    engine.update(Duration::from_millis(16), None).unwrap();
    // Release to complete the effect and create locks
    engine.release_layer_with_time(EffectLayer::Foreground, Some(Duration::from_millis(100)));
    engine.update(Duration::from_millis(100), None).unwrap();
    // One more update to ensure completion is processed and locks are created
    engine.update(Duration::from_millis(1), None).unwrap();

    // Verify locks are in place - check what value red is locked at
    // Should be 1.0 (255) from the foreground effect that set red=1.0
    let check_commands = engine.update(Duration::from_millis(1), None).unwrap();
    let (check_r, check_g, check_b) = get_rgb(1, 1, &check_commands);
    // Red should be locked at 255 (from the foreground effect that set red=1.0)
    assert_eq!(check_r, 255, "Red should be locked at 255 before dimmer");
    assert_eq!(check_g, 0, "Green should be locked at 0");
    assert_eq!(check_b, 0, "Blue should be locked at 0");

    // Verify locks work - try to set green, should be blocked
    let test_bg = EffectInstance::new(
        "test_bg".to_string(),
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
    engine.start_effect(test_bg).unwrap();
    let test_commands = engine.update(Duration::from_millis(16), None).unwrap();
    let (_tr, tg, _tb) = get_rgb(1, 1, &test_commands);
    // Green should be blocked if locks are working
    assert_eq!(tg, 0, "Green should be locked");
    // Don't clear the background layer - clear_layer() clears ALL locks, which would break the test

    // Start a dimmer effect on background - dimmer should pass through
    // For fixtures with dedicated dimmer, the dimmer channel affects all RGB at hardware level
    let dimmer_effect = EffectInstance::new(
        "dimmer".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.0,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(dimmer_effect).unwrap();

    // At midpoint (500ms into 1s fade), dimmer should be at 0.5
    // For dedicated dimmer fixtures, this dims all channels at hardware level
    let commands = engine.update(Duration::from_millis(500), None).unwrap();

    // Find the dimmer channel value (channel 1, address 1)
    let mut dimmer_value = 255;
    for cmd in &commands {
        if cmd.universe == 1 && cmd.channel == 1 {
            dimmer_value = cmd.value;
            break;
        }
    }
    // Dimmer should be at ~50% (127-128)
    assert!(
        (120..=135).contains(&dimmer_value),
        "Dimmer channel should be at ~50%, got {}",
        dimmer_value
    );

    // For dedicated dimmer fixtures, the RGB channels maintain their locked values
    // but the hardware dimmer dims the output. The test fixture has a dedicated dimmer,
    // so RGB channels stay at locked values, and dimmer channel controls brightness.
    let (r, g, b) = get_rgb(1, 1, &commands);
    // RGB channels should still be at their locked values
    assert_eq!(
        r, 255,
        "Red should remain at locked value (dimmer is separate channel)"
    );
    assert_eq!(g, 0, "Green should remain 0");
    assert_eq!(b, 0, "Blue should remain 0");
}

#[test]
fn test_channel_locking_multiple_channels() {
    // Test that all channels from a foreground Replace effect are locked
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start foreground Replace with all RGB channels
    let mut fg_params = HashMap::new();
    fg_params.insert("red".to_string(), 1.0);
    fg_params.insert("green".to_string(), 1.0);
    fg_params.insert("blue".to_string(), 1.0);
    fg_params.insert("white".to_string(), 0.5);

    let mut fg_effect = EffectInstance::new(
        "fg_all".to_string(),
        EffectType::Static {
            parameters: fg_params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    fg_effect.layer = EffectLayer::Foreground;
    fg_effect.blend_mode = BlendMode::Replace;
    engine.start_effect(fg_effect).unwrap();
    engine.update(Duration::from_millis(16), None).unwrap();
    // Release to complete the effect and create locks
    engine.release_layer_with_time(EffectLayer::Foreground, Some(Duration::from_millis(100)));
    engine.update(Duration::from_millis(100), None).unwrap();

    // Try to change RGBW from background - all should be blocked
    let mut bg_params = HashMap::new();
    bg_params.insert("red".to_string(), 0.0);
    bg_params.insert("green".to_string(), 0.0);
    bg_params.insert("blue".to_string(), 0.0);
    bg_params.insert("white".to_string(), 1.0);

    let bg_effect = EffectInstance::new(
        "bg_change".to_string(),
        EffectType::Static {
            parameters: bg_params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(bg_effect).unwrap();
    let commands = engine.update(Duration::from_millis(16), None).unwrap();

    // All locked channels should persist
    let (r, g, b) = get_rgb(1, 1, &commands);
    assert_eq!(r, 255, "Red should be locked");
    assert_eq!(g, 255, "Green should be locked");
    assert_eq!(b, 255, "Blue should be locked");

    // Find white channel (address 5)
    let mut white = 0;
    for cmd in &commands {
        if cmd.universe == 1 && cmd.channel == 5 {
            white = cmd.value;
        }
    }
    assert_eq!(white, 127, "White should be locked at 0.5 (127)");
}

#[test]
fn test_channel_locking_temporary_effect_no_lock() {
    // Test that temporary effects don't lock channels even if foreground Replace
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start a temporary foreground Replace effect
    let mut fg_params = HashMap::new();
    fg_params.insert("red".to_string(), 1.0);
    fg_params.insert("green".to_string(), 0.0);
    fg_params.insert("blue".to_string(), 0.0);

    let mut fg_effect = EffectInstance::new(
        "fg_temp".to_string(),
        EffectType::Static {
            parameters: fg_params,
            duration: Some(Duration::from_millis(100)), // Temporary
        },
        vec!["test_fixture".to_string()],
        None,
        Some(Duration::from_millis(100)),
        None,
    );
    fg_effect.layer = EffectLayer::Foreground;
    fg_effect.blend_mode = BlendMode::Replace;
    engine.start_effect(fg_effect).unwrap();

    // Let it complete
    engine.update(Duration::from_millis(50), None).unwrap();
    engine.update(Duration::from_millis(100), None).unwrap();

    // Background effect should work (no locks from temporary effect)
    let bg_params = HashMap::from([("green".to_string(), 1.0), ("blue".to_string(), 1.0)]);

    let bg_effect = EffectInstance::new(
        "bg".to_string(),
        EffectType::Static {
            parameters: bg_params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(bg_effect).unwrap();
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    let (r, g, b) = get_rgb(1, 1, &commands);
    assert_eq!(r, 0, "Red should not be locked (temporary effect)");
    assert_eq!(g, 255, "Green should work");
    assert_eq!(b, 255, "Blue should work");
}

#[test]
fn test_channel_locking_partial_channel_locks() {
    // Test that only channels affected by the foreground Replace are locked
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start foreground Replace with only red
    let mut fg_params = HashMap::new();
    fg_params.insert("red".to_string(), 1.0);

    let mut fg_effect = EffectInstance::new(
        "fg_red".to_string(),
        EffectType::Static {
            parameters: fg_params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    fg_effect.layer = EffectLayer::Foreground;
    fg_effect.blend_mode = BlendMode::Replace;
    engine.start_effect(fg_effect).unwrap();
    engine.update(Duration::from_millis(16), None).unwrap();
    // Release to complete the effect and create locks
    engine.release_layer_with_time(EffectLayer::Foreground, Some(Duration::from_millis(100)));
    engine.update(Duration::from_millis(100), None).unwrap();

    // Background effect with green and blue should work
    let bg_params = HashMap::from([("green".to_string(), 1.0), ("blue".to_string(), 1.0)]);

    let bg_effect = EffectInstance::new(
        "bg".to_string(),
        EffectType::Static {
            parameters: bg_params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(bg_effect).unwrap();
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    let (r, g, b) = get_rgb(1, 1, &commands);
    assert_eq!(r, 255, "Red should be locked");
    assert_eq!(g, 255, "Green should work (not locked)");
    assert_eq!(b, 255, "Blue should work (not locked)");
}
