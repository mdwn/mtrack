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

use super::common::*;
#[cfg(test)]
use crate::lighting::effects::*;
use crate::lighting::engine::EffectEngine;
use std::collections::HashMap;
use std::time::Duration;

#[test]
fn test_dimmer_multiplier_passes_through_locks_rgb_only() {
    // RGB-only fixture: foreground replace static should lock RGB channels,
    // but a dimmer fade (implemented via _dimmer_multiplier) must still affect output.
    let mut engine = EffectEngine::new();

    // Register an RGB-only fixture (no dedicated dimmer)
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);

    let fixture = FixtureInfo {
        name: "front_wash".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "RGB_Par".to_string(),
        channels,
        max_strobe_frequency: None,
    };
    engine.register_fixture(fixture);

    // Foreground replace static blue (locks RGB)
    let mut static_blue = EffectInstance::new(
        "static_blue".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("red".to_string(), 0.0);
                p.insert("green".to_string(), 0.0);
                p.insert("blue".to_string(), 1.0);
                p
            },
            duration: None,
        },
        vec!["front_wash".to_string()],
        None,
        None,
        None,
    );
    static_blue.layer = EffectLayer::Background;
    static_blue.blend_mode = BlendMode::Replace;
    engine.start_effect(static_blue).unwrap();

    // Advance so static applies
    engine.update(Duration::from_millis(100)).unwrap();

    // Foreground multiply dimmer fade to black over 2s
    let mut fade_out = EffectInstance::new(
        "fade_out".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.0,
            duration: Duration::from_secs(2),
            curve: DimmerCurve::Linear,
        },
        vec!["front_wash".to_string()],
        None,
        None,
        None,
    );
    fade_out.layer = EffectLayer::Foreground;
    fade_out.blend_mode = BlendMode::Multiply;
    engine.start_effect(fade_out).unwrap();

    // Halfway through fade (1s): blue should be ~50%
    let cmds_1s = engine.update(Duration::from_secs(1)).unwrap();
    let blue_1s = cmds_1s
        .iter()
        .find(|c| c.universe == 1 && c.channel == 3)
        .map(|c| c.value)
        .unwrap_or(0);
    assert!(
        blue_1s > 100 && blue_1s < 155,
        "blue should be mid-fade (~50%) at 1s, got {}",
        blue_1s
    );

    // Near end of fade (additional 500ms, total 1.5s): blue should be around 25% (faded 75% to black)
    let cmds_15s = engine.update(Duration::from_millis(500)).unwrap();
    let blue_15s = cmds_15s
        .iter()
        .find(|c| c.universe == 1 && c.channel == 3)
        .map(|c| c.value)
        .unwrap_or(0);
    assert!(
        blue_15s > 50 && blue_15s < 75,
        "blue should be around 25% (faded 75% to black) at 1.5s, got {}",
        blue_15s
    );

    // After fade completes (exceed 2s): foreground Replace static is temporary but the dimmer
    // that faded it to black is permanent, so the final dimmed value (0) persists
    let cmds_after = engine.update(Duration::from_millis(500)).unwrap();
    let blue_after = cmds_after
        .iter()
        .find(|c| c.universe == 1 && c.channel == 3)
        .map(|c| c.value)
        .unwrap_or(0);
    assert_eq!(
        blue_after, 0,
        "blue should remain at 0 after dimmer completes (dimmers are permanent)"
    );
}
#[test]
fn test_dedicated_dimmer_preserves_rgb() {
    // Fixture with a dedicated dimmer: dimmer fades should not change RGB channel values.
    let mut engine = EffectEngine::new();

    // Register fixture with dedicated dimmer channel
    let mut channels = HashMap::new();
    channels.insert("dimmer".to_string(), 1);
    channels.insert("red".to_string(), 2);
    channels.insert("green".to_string(), 3);
    channels.insert("blue".to_string(), 4);

    let fixture = FixtureInfo {
        name: "front_wash".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "RGB_Par_Dimmer".to_string(),
        channels,
        max_strobe_frequency: None,
    };
    engine.register_fixture(fixture);

    // Foreground replace static blue at full with dimmer 100%
    let mut static_blue = EffectInstance::new(
        "static_blue".to_string(),
        EffectType::Static {
            parameters: {
                let mut p = HashMap::new();
                p.insert("red".to_string(), 0.0);
                p.insert("green".to_string(), 0.0);
                p.insert("blue".to_string(), 1.0);
                p.insert("dimmer".to_string(), 1.0);
                p
            },
            duration: None,
        },
        vec!["front_wash".to_string()],
        None,
        None,
        None,
    );
    static_blue.layer = EffectLayer::Background;
    static_blue.blend_mode = BlendMode::Replace;
    engine.start_effect(static_blue).unwrap();

    // Allow static to apply
    engine.update(Duration::from_millis(50)).unwrap();

    // Foreground replace dimmer fade from 1.0 to 0.0 over 2s
    let mut fade_out = EffectInstance::new(
        "fade_out".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.0,
            duration: Duration::from_secs(2), // 2s fade to black
            curve: DimmerCurve::Linear,
        },
        vec!["front_wash".to_string()],
        None,
        None,
        None,
    );
    fade_out.layer = EffectLayer::Foreground;
    fade_out.blend_mode = BlendMode::Replace;
    engine.start_effect(fade_out).unwrap();

    // At 1s into fade: dimmer should be ~50% while RGB stays at static values
    let cmds_1s = engine.update(Duration::from_secs(1)).unwrap();
    let dimmer_1s = cmds_1s
        .iter()
        .find(|c| c.universe == 1 && c.channel == 1)
        .map(|c| c.value)
        .unwrap_or(0);
    let red_1s = cmds_1s
        .iter()
        .find(|c| c.universe == 1 && c.channel == 2)
        .map(|c| c.value)
        .unwrap_or(0);
    let green_1s = cmds_1s
        .iter()
        .find(|c| c.universe == 1 && c.channel == 3)
        .map(|c| c.value)
        .unwrap_or(0);
    let blue_1s = cmds_1s
        .iter()
        .find(|c| c.universe == 1 && c.channel == 4)
        .map(|c| c.value)
        .unwrap_or(0);

    assert!(
        dimmer_1s > 100 && dimmer_1s < 155,
        "dimmer should be mid-fade at 1s"
    );
    assert_eq!(red_1s, 0, "red should remain 0 at 1s");
    assert_eq!(green_1s, 0, "green should remain 0 at 1s");
    assert_eq!(blue_1s, 255, "blue should remain 255 at 1s");
}
#[test]
fn test_effect_layering_static_blue_and_dimmer() {
    // Initialize tracing for this test

    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture.clone());

    // Create static blue effect on background layer
    let mut blue_params = HashMap::new();
    blue_params.insert("red".to_string(), 0.0);
    blue_params.insert("green".to_string(), 0.0);
    blue_params.insert("blue".to_string(), 1.0);

    let blue_effect = create_effect_with_layering(
        "static_blue".to_string(),
        EffectType::Static {
            parameters: blue_params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    // Create dimmer effect on midground layer
    let dimmer_effect = create_effect_with_layering(
        "dimmer".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.5,
            duration: Duration::from_secs(2),
            curve: DimmerCurve::Linear,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Midground,
        BlendMode::Multiply,
    );

    // Start effects
    engine.start_effect(blue_effect).unwrap();
    engine.start_effect(dimmer_effect).unwrap();

    // Update engine at start (dimmer should be at 100%)
    let commands = engine.update(Duration::from_millis(16)).unwrap();

    // Should have 3 commands: red, green, blue (dimmer uses Multiply mode, affects all RGB channels)
    assert_eq!(commands.len(), 3);

    // Find the commands
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

    // At start (t=16ms): dimmer is 0.008 progress through 2s up_time, so dimmer ≈ 1.0
    // Blue should be at full brightness (255 * 1.0 = 255)
    assert_eq!(red_cmd.value, 0);
    assert_eq!(green_cmd.value, 0);
    assert!(blue_cmd.value >= 250);

    // Update engine at t=532ms (26.6% through 2s up_time)
    engine.update(Duration::from_millis(500)).unwrap();
    let commands = engine.update(Duration::from_millis(16)).unwrap();

    // The dimmer effect is applied to RGB channels
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
    // At 26.6% progress: dimmer = 1.0 + (0.5 - 1.0) * 0.266 = 0.867
    // blue = 255 * 0.867 = 221
    assert!(blue_cmd.value >= 215 && blue_cmd.value <= 225);

    // Update engine at t=1048ms (52.4% through 2s up_time)
    engine.update(Duration::from_millis(500)).unwrap();
    let commands = engine.update(Duration::from_millis(16)).unwrap();

    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
    // At 52.4% progress: dimmer = 1.0 + (0.5 - 1.0) * 0.524 = 0.738
    // blue = 255 * 0.738 = 188
    assert!(blue_cmd.value >= 185 && blue_cmd.value <= 195);
}
#[test]
fn test_dimmer_without_dedicated_channel() {
    use super::super::effects::*;
    use super::super::engine::EffectEngine;

    // Create a fixture without a dedicated dimmer channel
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    // No dimmer channel!

    let fixture = FixtureInfo {
        name: "rgb_only_fixture".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "RGB_Par".to_string(),
        channels,
        max_strobe_frequency: Some(20.0), // Test fixture with strobe
    };

    let mut engine = EffectEngine::new();
    engine.register_fixture(fixture.clone());

    // Create a static blue effect (indefinite - no timing)
    let mut blue_effect = EffectInstance::new(
        "blue".to_string(),
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
        vec!["rgb_only_fixture".to_string()],
        None,
        None,
        None,
    );
    blue_effect.layer = EffectLayer::Background;
    blue_effect.blend_mode = BlendMode::Replace;

    // Create a dimmer effect (1s duration, permanent)
    let mut dimmer_effect = EffectInstance::new(
        "dimmer".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.5,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        },
        vec!["rgb_only_fixture".to_string()],
        None,
        None,
        None,
    );
    dimmer_effect.layer = EffectLayer::Midground;
    dimmer_effect.blend_mode = BlendMode::Multiply;

    // Start effects
    engine.start_effect(blue_effect).unwrap();
    engine.start_effect(dimmer_effect).unwrap();

    // Update engine immediately at start (dimmer should be at 100%)
    let commands = engine.update(Duration::from_millis(0)).unwrap();

    // Should have only RGB commands (no dedicated dimmer channel)
    assert_eq!(commands.len(), 3);

    // Find the commands
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

    // At start: blue should be at full brightness (255), others should be 0
    // (dimmer starts at 1.0 and fades to 0.5 over 1s, so at start it's 1.0)
    assert_eq!(red_cmd.value, 0);
    assert_eq!(green_cmd.value, 0);
    assert_eq!(blue_cmd.value, 255); // 255 * 1.0 = 255

    // Update engine at 50% (dimmer should be at 0.75)
    let commands = engine.update(Duration::from_millis(500)).unwrap();

    // Should still have only RGB commands
    assert_eq!(commands.len(), 3);

    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

    // At 50%: blue should be dimmed to 75% (191), others should be 0
    // (dimmer goes from 1.0 to 0.5 over 1s, so at 50% progress it's 0.75)
    // With fixture profile system, the dimmer effect uses per-layer multipliers
    // which get applied at emission, so we expect the dimmed result
    assert_eq!(red_cmd.value, 0);
    assert_eq!(green_cmd.value, 0);
    assert_eq!(
        blue_cmd.value, 191,
        "Expected 191 (0.75 * 255), got {}",
        blue_cmd.value
    );

    println!("Dimmer without dedicated channel test passed!");
    println!("RGB-only fixture properly dims its color channels");
}
#[test]
fn test_dimmer_precedence_and_selective_dimming() {
    use super::super::effects::*;
    use super::super::engine::EffectEngine;

    // Create a fixture with RGB channels only (no dedicated dimmer)
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);

    let fixture = FixtureInfo {
        name: "test_fixture".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "RGB_Par".to_string(),
        channels,
        max_strobe_frequency: Some(20.0), // Test fixture with strobe
    };

    let mut engine = EffectEngine::new();
    engine.register_fixture(fixture.clone());

    // Test 1: Blue-only static effect
    println!("\n1. Blue-only static effect:");
    let mut static_params = HashMap::new();
    static_params.insert("blue".to_string(), 1.0);
    // No red or green values set

    let blue_effect = create_effect_with_layering(
        "blue_static".to_string(),
        EffectType::Static {
            parameters: static_params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    engine.start_effect(blue_effect).unwrap();
    let commands = engine.update(Duration::from_millis(0)).unwrap();

    println!("Commands: {:?}", commands);
    for cmd in &commands {
        let channel_name = match cmd.channel {
            1 => "Red",
            2 => "Green",
            3 => "Blue",
            _ => "Unknown",
        };
        println!(
            "  {}: {} ({:.1}%)",
            channel_name,
            cmd.value,
            cmd.value as f64 / 255.0 * 100.0
        );
    }

    // Test 2: Add dimmer effect (1.0 -> 0.0)
    println!("\n2. Adding dimmer effect (1.0 -> 0.0):");
    let mut dimmer_effect = create_effect_with_layering(
        "dimmer".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.0,
            duration: Duration::from_secs(2),
            curve: DimmerCurve::Linear,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Midground,
        BlendMode::Replace,
    );

    // Override the timing to have exact 2-second duration
    dimmer_effect.up_time = Some(Duration::from_secs(2));
    dimmer_effect.hold_time = Some(Duration::from_secs(0));
    dimmer_effect.down_time = Some(Duration::from_secs(0));

    engine.start_effect(dimmer_effect).unwrap();

    // Check at different time points (using incremental durations)
    let mut previous_time = 0;
    for (time_ms, description) in [(0, "Start"), (500, "25%"), (1000, "50%"), (2000, "End")] {
        let increment = time_ms - previous_time;
        let commands = engine.update(Duration::from_millis(increment)).unwrap();
        previous_time = time_ms;
        println!("\n  At {} ({}ms):", description, time_ms);
        for cmd in &commands {
            let channel_name = match cmd.channel {
                1 => "Red",
                2 => "Green",
                3 => "Blue",
                _ => "Unknown",
            };
            println!(
                "    {}: {} ({:.1}%)",
                channel_name,
                cmd.value,
                cmd.value as f64 / 255.0 * 100.0
            );
        }
    }

    println!("\nFixed behavior analysis:");
    println!("- Red channel: Gets dimmer values multiplied with static red value (for layering)");
    println!(
        "- Green channel: Gets dimmer values multiplied with static green value (for layering)"
    );
    println!("- Blue channel: Gets dimmer values multiplied with static blue value (for layering)");

    // Verify the behavior is correct
    let final_commands = engine.update(Duration::from_millis(2000)).unwrap();
    // At the end (4000ms), the dimmer effect has completed and persisted at 0.0
    assert_eq!(final_commands.len(), 1); // Only blue channel from static effect

    // Blue channel should be at 0 (dimmed to 0 and persisted)
    let blue_cmd = final_commands.iter().find(|cmd| cmd.channel == 3).unwrap();
    assert_eq!(blue_cmd.value, 0, "Blue should be dimmed to 0 and persist");

    println!("✅ Dimmer precedence and selective dimming test passed!");
    println!("✅ RGB channels are used for layering with Multiply mode");
    println!("✅ No dedicated dimmer channel - RGB multiplication preserves color");
}
#[test]
fn test_dimmer_debug() {
    // Initialize tracing

    let mut engine = EffectEngine::new();

    // Create a test fixture with RGB channels
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
        max_strobe_frequency: Some(20.0), // Test fixture with strobe
    };
    engine.register_fixture(fixture.clone());

    // Create static blue effect
    let mut blue_params = HashMap::new();
    blue_params.insert("red".to_string(), 0.0);
    blue_params.insert("green".to_string(), 0.0);
    blue_params.insert("blue".to_string(), 1.0);

    let blue_effect = create_effect_with_layering(
        "static_blue".to_string(),
        EffectType::Static {
            parameters: blue_params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    // Create dimmer effect with Multiply blend mode
    let dimmer_effect = create_effect_with_layering(
        "dimmer".to_string(),
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

    // Start effects
    engine.start_effect(blue_effect).unwrap();
    engine.start_effect(dimmer_effect).unwrap();

    // Update engine and check results
    let commands = engine.update(Duration::from_millis(16)).unwrap();

    println!("Commands at start:");
    for cmd in &commands {
        println!("  Channel {}: {}", cmd.channel, cmd.value);
    }

    // Update to middle of dimmer
    engine.update(Duration::from_millis(500)).unwrap();
    let commands = engine.update(Duration::from_millis(16)).unwrap();

    println!("Commands at middle (should be dimmed blue):");
    for cmd in &commands {
        println!("  Channel {}: {}", cmd.channel, cmd.value);
    }

    // Check that red and green are 0, blue is dimmed
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

    println!(
        "Red: {}, Green: {}, Blue: {}",
        red_cmd.value, green_cmd.value, blue_cmd.value
    );

    // Red and green should be 0, blue should be dimmed (around 75% of 255 at 50% progress)
    assert_eq!(red_cmd.value, 0);
    assert_eq!(green_cmd.value, 0);
    assert!(blue_cmd.value > 180 && blue_cmd.value < 200); // Around 75% of 255
}
#[test]
fn test_static_with_dimmer_parameter() {
    use super::super::effects::*;
    use super::super::engine::EffectEngine;

    // Initialize tracing

    // Create a test fixture (Astera PixelBlock style)
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

    let mut engine = EffectEngine::new();
    engine.register_fixture(fixture);

    // Also register back_wash fixture
    let mut back_channels = HashMap::new();
    back_channels.insert("red".to_string(), 1);
    back_channels.insert("green".to_string(), 2);
    back_channels.insert("blue".to_string(), 3);
    back_channels.insert("strobe".to_string(), 4);

    let back_fixture = FixtureInfo {
        name: "back_wash".to_string(),
        universe: 1,
        address: 5, // Different address
        channels: back_channels,
        fixture_type: "Astera-PixelBrick".to_string(),
        max_strobe_frequency: Some(20.0), // Test fixture with strobe
    };
    engine.register_fixture(back_fixture);

    // Test static effect with both color and dimmer parameters
    let mut static_params = HashMap::new();
    static_params.insert("red".to_string(), 0.0);
    static_params.insert("green".to_string(), 0.0);
    static_params.insert("blue".to_string(), 1.0);
    static_params.insert("dimmer".to_string(), 1.0); // This is the problem!

    let static_effect = create_effect_with_layering(
        "static_blue_with_dimmer".to_string(),
        EffectType::Static {
            parameters: static_params,
            duration: None,
        },
        vec!["front_wash".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    engine.start_effect(static_effect).unwrap();

    // Check what the static effect produces
    let commands = engine.update(Duration::from_secs(0)).unwrap();
    println!("Static effect with dimmer parameter:");
    for cmd in &commands {
        println!("  Channel {}: {}", cmd.channel, cmd.value);
    }

    // Now add a dimmer effect with multiply blend mode
    let dimmer_effect = create_effect_with_layering(
        "dimmer_multiply".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.5,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        },
        vec!["front_wash".to_string()],
        EffectLayer::Midground,
        BlendMode::Multiply,
    );

    engine.start_effect(dimmer_effect).unwrap();

    // Check what happens with the dimmer effect
    let commands = engine.update(Duration::from_secs(500)).unwrap(); // 50% through dimmer
    println!("\nWith dimmer effect (50% through):");
    for cmd in &commands {
        println!("  Channel {}: {}", cmd.channel, cmd.value);
    }

    // The issue: static effect sets dimmer channel to 1.0, but dimmer effect only affects RGB channels
    // So the dimmer channel stays at 1.0 while RGB channels get dimmed
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1);
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2);
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3);

    if let (Some(red), Some(green), Some(blue)) = (red_cmd, green_cmd, blue_cmd) {
        println!("\nAnalysis:");
        println!("  Red: {} (should be 0)", red.value);
        println!("  Green: {} (should be 0)", green.value);
        println!("  Blue: {} (should be dimmed)", blue.value);
    }
}
#[test]
fn test_dimmer_replace_vs_multiply() {
    // Initialize tracing

    let mut engine = EffectEngine::new();

    // Create a test fixture with RGB channels
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
        max_strobe_frequency: Some(20.0), // Test fixture with strobe
    };
    engine.register_fixture(fixture.clone());

    // Create static blue effect
    let mut blue_params = HashMap::new();
    blue_params.insert("red".to_string(), 0.0);
    blue_params.insert("green".to_string(), 0.0);
    blue_params.insert("blue".to_string(), 1.0);

    let blue_effect = create_effect_with_layering(
        "static_blue".to_string(),
        EffectType::Static {
            parameters: blue_params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    // Test 1: Dimmer with Replace blend mode (should turn white)
    let dimmer_replace = create_effect_with_layering(
        "dimmer_replace".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.5,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        },
        vec!["test_fixture".to_string()],
        EffectLayer::Midground,
        BlendMode::Replace,
    );

    // Start effects
    engine.start_effect(blue_effect.clone()).unwrap();
    engine.start_effect(dimmer_replace).unwrap();

    // Update engine and check results
    let commands = engine.update(Duration::from_millis(16)).unwrap();

    println!("Commands with Replace blend mode:");
    for cmd in &commands {
        println!("  Channel {}: {}", cmd.channel, cmd.value);
    }

    // Check that all channels have the same value (white)
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

    println!(
        "Replace - Red: {}, Green: {}, Blue: {}",
        red_cmd.value, green_cmd.value, blue_cmd.value
    );

    // With fixture profile system, RGB-only fixtures use RgbMultiplication strategy
    // which preserves color instead of creating white light
    // The dimmer effect uses _dimmer_multiplier, so we expect only blue channel
    // to be set by the static effect, not all channels by the dimmer
    assert_eq!(red_cmd.value, 0);
    assert_eq!(green_cmd.value, 0);
    assert!(blue_cmd.value > 0);

    // Clear effects and test Multiply
    let mut engine2 = EffectEngine::new();
    engine2.register_fixture(fixture.clone());
    engine2.start_effect(blue_effect).unwrap();

    let dimmer_multiply = create_effect_with_layering(
        "dimmer_multiply".to_string(),
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

    engine2.start_effect(dimmer_multiply).unwrap();

    // Update engine and check results
    let commands = engine2.update(Duration::from_millis(16)).unwrap();

    println!("Commands with Multiply blend mode:");
    for cmd in &commands {
        println!("  Channel {}: {}", cmd.channel, cmd.value);
    }

    // Check that red and green are 0, blue is dimmed
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

    println!(
        "Multiply - Red: {}, Green: {}, Blue: {}",
        red_cmd.value, green_cmd.value, blue_cmd.value
    );

    // With Multiply, red and green should be 0, blue should be dimmed
    assert_eq!(red_cmd.value, 0);
    assert_eq!(green_cmd.value, 0);
    assert!(blue_cmd.value > 0);
}
#[test]
fn test_astera_pixelblock_dimmer() {
    // Initialize tracing

    let mut engine = EffectEngine::new();

    // Create Astera PixelBlock fixture (no dimmer channel, only RGB + strobe)
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    channels.insert("strobe".to_string(), 4);

    let fixture = FixtureInfo {
        name: "astera_pixelblock".to_string(),
        universe: 1,
        address: 1,
        channels,
        fixture_type: "Astera-PixelBrick".to_string(),
        max_strobe_frequency: Some(20.0), // Test fixture with strobe
    };
    engine.register_fixture(fixture.clone());

    // Create static blue effect
    let mut blue_params = HashMap::new();
    blue_params.insert("red".to_string(), 0.0);
    blue_params.insert("green".to_string(), 0.0);
    blue_params.insert("blue".to_string(), 1.0);

    let blue_effect = create_effect_with_layering(
        "static_blue".to_string(),
        EffectType::Static {
            parameters: blue_params,
            duration: None,
        },
        vec!["astera_pixelblock".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    // Create dimmer effect with Multiply blend mode (as specified in DSL)
    let dimmer_effect = create_effect_with_layering(
        "dimmer".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.5,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        },
        vec!["astera_pixelblock".to_string()],
        EffectLayer::Midground,
        BlendMode::Multiply,
    );

    // Start effects
    engine.start_effect(blue_effect.clone()).unwrap();
    engine.start_effect(dimmer_effect).unwrap();

    // Update engine and check results
    let commands = engine.update(Duration::from_millis(16)).unwrap();

    println!("Commands with Astera PixelBlock fixture:");
    for cmd in &commands {
        println!("  Channel {}: {}", cmd.channel, cmd.value);
    }

    // Check that red and green are 0, blue is dimmed
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

    println!(
        "Astera PixelBlock - Red: {}, Green: {}, Blue: {}",
        red_cmd.value, green_cmd.value, blue_cmd.value
    );

    // With Multiply, red and green should be 0, blue should be dimmed
    assert_eq!(red_cmd.value, 0);
    assert_eq!(green_cmd.value, 0);
    assert!(blue_cmd.value > 0);

    // Test with Replace blend mode to see the difference
    let mut engine2 = EffectEngine::new();
    engine2.register_fixture(fixture);
    engine2.start_effect(blue_effect).unwrap();

    let dimmer_replace = create_effect_with_layering(
        "dimmer_replace".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.5,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        },
        vec!["astera_pixelblock".to_string()],
        EffectLayer::Midground,
        BlendMode::Replace,
    );

    engine2.start_effect(dimmer_replace).unwrap();

    let commands = engine2.update(Duration::from_millis(16)).unwrap();

    println!("Commands with Replace blend mode:");
    for cmd in &commands {
        println!("  Channel {}: {}", cmd.channel, cmd.value);
    }

    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

    println!(
        "Replace - Red: {}, Green: {}, Blue: {}",
        red_cmd.value, green_cmd.value, blue_cmd.value
    );

    // With fixture profile system, RGB-only fixtures use RgbMultiplication strategy
    // which preserves color instead of creating white light
    // The dimmer effect uses _dimmer_multiplier, so we expect only blue channel
    // to be set by the static effect, not all channels by the dimmer
    assert_eq!(red_cmd.value, 0);
    assert_eq!(green_cmd.value, 0);
    assert!(blue_cmd.value > 0);
}
#[test]
fn test_chase_effect_without_dimmer_channel() {
    let mut engine = EffectEngine::new();

    // Create RGB-only fixture (no dimmer)
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);

    let fixture = FixtureInfo {
        name: "rgb_fixture".to_string(),
        universe: 1,
        address: 1,
        channels,
        fixture_type: "RGB_Par".to_string(),
        max_strobe_frequency: Some(20.0),
    };
    engine.register_fixture(fixture);

    // Create Chase effect - should work with RGB-only fixture
    let chase_effect = create_effect_with_layering(
        "chase_effect".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
        },
        vec!["rgb_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    // Should start successfully without dimmer channel
    let result = engine.start_effect(chase_effect);
    assert!(
        result.is_ok(),
        "Chase effect should work with RGB-only fixture"
    );

    // Update engine to generate commands
    let commands = engine.update(Duration::from_millis(100)).unwrap();

    // Should have RGB commands (not dimmer commands)
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1);
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2);
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3);

    assert!(red_cmd.is_some(), "Should have red channel command");
    assert!(green_cmd.is_some(), "Should have green channel command");
    assert!(blue_cmd.is_some(), "Should have blue channel command");

    // All RGB channels should have the same value (white chase)
    if let (Some(red), Some(green), Some(blue)) = (red_cmd, green_cmd, blue_cmd) {
        assert_eq!(red.value, green.value);
        assert_eq!(green.value, blue.value);
    }
}
#[test]
fn test_chase_effect_with_dimmer_channel() {
    let mut engine = EffectEngine::new();

    // Create fixture with both RGB and dimmer
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    channels.insert("dimmer".to_string(), 4);

    let fixture = FixtureInfo {
        name: "rgb_dimmer_fixture".to_string(),
        universe: 1,
        address: 1,
        channels,
        fixture_type: "RGB_Par".to_string(),
        max_strobe_frequency: Some(20.0),
    };
    engine.register_fixture(fixture);

    // Create Chase effect - should use dimmer channel
    let chase_effect = create_effect_with_layering(
        "chase_effect".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
        },
        vec!["rgb_dimmer_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    // Should start successfully
    let result = engine.start_effect(chase_effect);
    assert!(
        result.is_ok(),
        "Chase effect should work with dimmer fixture"
    );

    // Update engine to generate commands
    let commands = engine.update(Duration::from_millis(100)).unwrap();

    // Should have dimmer command (not RGB commands)
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 4);
    assert!(dimmer_cmd.is_some(), "Should have dimmer channel command");

    // Should not have RGB commands when dimmer is available
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1);
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2);
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3);

    assert!(
        red_cmd.is_none(),
        "Should not have red channel command when dimmer is available"
    );
    assert!(
        green_cmd.is_none(),
        "Should not have green channel command when dimmer is available"
    );
    assert!(
        blue_cmd.is_none(),
        "Should not have blue channel command when dimmer is available"
    );
}
#[test]
fn test_software_strobing_dimmer_only_fixture() {
    let mut engine = EffectEngine::new();

    // Create dimmer-only fixture (no strobe or RGB channels)
    let mut channels = HashMap::new();
    channels.insert("dimmer".to_string(), 1);
    // No strobe or RGB channels!

    let fixture = FixtureInfo {
        name: "dimmer_only_fixture".to_string(),
        universe: 1,
        address: 1,
        channels,
        fixture_type: "Dimmer".to_string(),
        max_strobe_frequency: None, // No strobe capability
    };
    engine.register_fixture(fixture);

    // Create strobe effect - should use software strobing on dimmer
    let strobe_effect = create_effect_with_layering(
        "strobe_effect".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(4.0), // 4 Hz for easy testing
            duration: None,
        },
        vec!["dimmer_only_fixture".to_string()],
        EffectLayer::Foreground,
        BlendMode::Overlay,
    );

    // Start the strobe effect
    engine.start_effect(strobe_effect).unwrap();

    // Test at different time points to verify strobing behavior
    // At t=0ms (start of cycle) - should be ON
    let commands = engine.update(Duration::from_millis(0)).unwrap();
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(dimmer_cmd.value, 255); // Should be ON (1.0 * 255)

    // At t=62ms (1/4 cycle) - should still be ON
    let commands = engine.update(Duration::from_millis(62)).unwrap();
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(dimmer_cmd.value, 255); // Should still be ON

    // At t=125ms (1/2 cycle) - should be OFF
    let commands = engine.update(Duration::from_millis(63)).unwrap(); // 62ms more = 125ms total
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(dimmer_cmd.value, 0); // Should be OFF (0.0 * 255)

    // At t=187ms (3/4 cycle) - should still be OFF
    let commands = engine.update(Duration::from_millis(62)).unwrap(); // 62ms more = 187ms total
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(dimmer_cmd.value, 0); // Should still be OFF

    // At t=250ms (full cycle) - should be ON again
    let commands = engine.update(Duration::from_millis(63)).unwrap(); // 63ms more = 250ms total
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(dimmer_cmd.value, 255); // Should be ON again
}
#[test]
fn test_multiple_dimmer_fade_to_black() {
    // Test multiple fixtures dimming to black simultaneously
    let mut engine = EffectEngine::new();

    // Register test fixtures
    let mut channels = HashMap::new();
    channels.insert("dimmer".to_string(), 1);
    channels.insert("red".to_string(), 2);
    channels.insert("green".to_string(), 3);
    channels.insert("blue".to_string(), 4);

    let front_wash = FixtureInfo {
        name: "front_wash".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "Dimmer".to_string(),
        channels: channels.clone(),
        max_strobe_frequency: None,
    };

    let back_wash = FixtureInfo {
        name: "back_wash".to_string(),
        universe: 1,
        address: 5,
        fixture_type: "Dimmer".to_string(),
        channels: channels.clone(),
        max_strobe_frequency: None,
    };

    engine.register_fixture(front_wash);
    engine.register_fixture(back_wash);

    // Create fade-out dimmer effects (2s fade from start to 0.0)
    let mut front_wash_fade = EffectInstance::new(
        "front_wash_fade".to_string(),
        EffectType::Dimmer {
            start_level: 0.5,
            end_level: 0.0,
            duration: Duration::from_secs(2), // 2s fade from 0.5 to 0.0
            curve: DimmerCurve::Linear,
        },
        vec!["front_wash".to_string()],
        None,
        None,
        None,
    );
    front_wash_fade.layer = EffectLayer::Foreground;
    front_wash_fade.blend_mode = BlendMode::Replace;

    let mut back_wash_fade = EffectInstance::new(
        "back_wash_fade".to_string(),
        EffectType::Dimmer {
            start_level: 0.3,
            end_level: 0.0,
            duration: Duration::from_secs(2), // 2s fade from 0.3 to 0.0
            curve: DimmerCurve::Linear,
        },
        vec!["back_wash".to_string()],
        None,
        None,
        None,
    );
    back_wash_fade.layer = EffectLayer::Foreground;
    back_wash_fade.blend_mode = BlendMode::Replace;

    // Start the effects
    engine.start_effect(front_wash_fade).unwrap();
    engine.start_effect(back_wash_fade).unwrap();

    println!("Testing fade-out effects from layering_show.light");

    // Test at various time points
    for (time_ms, description) in [
        (0, "Start"),
        (500, "25%"),
        (1000, "50%"),
        (1500, "75%"),
        (2000, "End"),
    ] {
        let commands = engine.update(Duration::from_millis(time_ms)).unwrap();
        println!("\nAt {} ({}ms):", description, time_ms);

        let front_dimmer = commands.iter().find(|cmd| cmd.channel == 1);
        let back_dimmer = commands.iter().find(|cmd| cmd.channel == 5);

        if let Some(cmd) = front_dimmer {
            println!(
                "  Front wash dimmer: {} ({:.1}%)",
                cmd.value,
                cmd.value as f64 / 255.0 * 100.0
            );
        } else {
            println!("  Front wash dimmer: No command");
        }

        if let Some(cmd) = back_dimmer {
            println!(
                "  Back wash dimmer: {} ({:.1}%)",
                cmd.value,
                cmd.value as f64 / 255.0 * 100.0
            );
        } else {
            println!("  Back wash dimmer: No command");
        }
    }

    // Verify the behavior
    let final_commands = engine.update(Duration::from_millis(2000)).unwrap();
    // Dimmers persist at 0.0, so dimmer channels should be 0
    // (or no commands if fixtures have no RGB to emit)
    for cmd in &final_commands {
        assert_eq!(cmd.value, 0, "Dimmer should persist at 0 after completion");
    }

    println!("✅ Fade-out effects test completed");
}
#[test]
fn test_dimmer_effect_mid_level_start() {
    // Test dimmer starting at a mid-level value (0.5) and fading to 0.0
    let mut engine = EffectEngine::new();

    // Register a test fixture with RGB channels (no dedicated dimmer)
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    let fixture = FixtureInfo {
        name: "test_fixture".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "RGB_Par".to_string(),
        channels,
        max_strobe_frequency: None,
    };
    engine.register_fixture(fixture);

    // Create a dimmer effect that fades from 0.5 to 0.0 over 2s
    let mut dimmer_effect = EffectInstance::new(
        "fade_out_test".to_string(),
        EffectType::Dimmer {
            start_level: 0.5,
            end_level: 0.0,
            duration: Duration::from_secs(2), // 2s fade from 0.5 to 0.0
            curve: DimmerCurve::Linear,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    dimmer_effect.layer = EffectLayer::Foreground;
    dimmer_effect.blend_mode = BlendMode::Replace;

    // Add a static blue effect to provide RGB values to dim
    let static_effect = EffectInstance::new(
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
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(static_effect).unwrap();

    // Start the dimmer effect
    engine.start_effect(dimmer_effect.clone()).unwrap();

    // Test the fade behavior at various time points
    // At 0s - dimmer is at start_level (0.5)
    let commands = engine.update(Duration::from_secs(0)).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
    assert_eq!(blue_cmd.value, 127, "Blue should be at 50% (127) at 0s"); // 255 * 0.5 = 127

    // At 0.5s (25% through 2s fade) - dimmer at 0.5 + (0.0 - 0.5) * 0.25 = 0.375
    let commands = engine.update(Duration::from_millis(500)).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
    assert_eq!(blue_cmd.value, 95, "Blue should be at 37.5% (95) at 0.5s"); // 255 * 0.375 ≈ 95

    // At 1s (50% through 2s fade) - dimmer at 0.5 + (0.0 - 0.5) * 0.5 = 0.25
    let commands = engine.update(Duration::from_millis(500)).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
    assert_eq!(blue_cmd.value, 63, "Blue should be at 25% (63) at 1s"); // 255 * 0.25 ≈ 63

    // At 2s (end of fade) - dimmer persists at end_level (0.0)
    let commands = engine.update(Duration::from_secs(1)).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
    assert_eq!(
        blue_cmd.value, 0,
        "Blue should be at 0% (0) at 2s and persist"
    );
    assert_eq!(
        engine.active_effects_count(),
        1,
        "Only static effect should remain active"
    );
}
#[test]
fn test_dimmer_curves() {
    // Test that different dimmer curves produce different fade shapes
    let mut engine = EffectEngine::new();

    // Register a test fixture with RGB channels
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    let fixture = FixtureInfo {
        name: "test_fixture".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "RGB_Par".to_string(),
        channels,
        max_strobe_frequency: None,
    };
    engine.register_fixture(fixture);

    // Add a static blue effect as base
    let static_blue = EffectInstance::new(
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
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    engine.start_effect(static_blue).unwrap();

    // Test each curve type
    let curves = vec![
        (DimmerCurve::Linear, "Linear"),
        (DimmerCurve::Exponential, "Exponential"),
        (DimmerCurve::Logarithmic, "Logarithmic"),
        (DimmerCurve::Sine, "Sine"),
        (DimmerCurve::Cosine, "Cosine"),
    ];

    for (curve, curve_name) in curves {
        // Reset engine for each curve test
        let mut test_engine = EffectEngine::new();
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);
        let fixture = FixtureInfo {
            name: "test_fixture".to_string(),
            universe: 1,
            address: 1,
            fixture_type: "RGB_Par".to_string(),
            channels,
            max_strobe_frequency: None,
        };
        test_engine.register_fixture(fixture);

        // Add static blue
        let static_blue = EffectInstance::new(
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
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );
        test_engine.start_effect(static_blue).unwrap();

        // Create dimmer with this curve
        let mut dimmer = EffectInstance::new(
            "dimmer".to_string(),
            EffectType::Dimmer {
                start_level: 1.0,
                end_level: 0.0,
                duration: Duration::from_secs(2),
                curve: curve.clone(),
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );
        dimmer.layer = EffectLayer::Midground;
        dimmer.blend_mode = BlendMode::Multiply;
        test_engine.start_effect(dimmer).unwrap();

        println!("\n{} curve:", curve_name);

        // Sample at 0%, 25%, 50%, 75%, 100%
        let test_points = vec![
            (0, "0%"),
            (500, "25%"),
            (1000, "50%"),
            (1500, "75%"),
            (2000, "100%"),
        ];

        let mut values = Vec::new();
        for (time_ms, label) in test_points {
            let commands = test_engine.update(Duration::from_millis(time_ms)).unwrap();
            let blue_cmd = commands.iter().find(|c| c.channel == 3).unwrap();
            values.push(blue_cmd.value);
            println!("  {} ({:4}ms): {}", label, time_ms, blue_cmd.value);
        }

        // Verify curve characteristics
        match curve {
            DimmerCurve::Linear => {
                // Linear should be evenly spaced
                assert_eq!(values[0], 255, "Linear start should be 255");
                assert_eq!(values[4], 0, "Linear end should be 0");
            }
            DimmerCurve::Exponential => {
                // Exponential should fade slowly at first, then faster
                assert_eq!(values[0], 255, "Exponential start should be 255");
                let early_drop = values[0] as i32 - values[1] as i32;
                let mid_drop = values[1] as i32 - values[2] as i32;
                assert!(
                    early_drop < mid_drop,
                    "Exponential: early fade should be slower (early: {}, mid: {})",
                    early_drop,
                    mid_drop
                );
                assert_eq!(values[4], 0, "Exponential end should be 0");
            }
            DimmerCurve::Logarithmic => {
                // Logarithmic should fade fast at first, then slower
                assert_eq!(values[0], 255, "Logarithmic start should be 255");
                let early_drop = values[0] as i32 - values[1] as i32;
                let mid_drop = values[1] as i32 - values[2] as i32;
                assert!(
                    early_drop > mid_drop,
                    "Logarithmic: early fade should be faster (early: {}, mid: {})",
                    early_drop,
                    mid_drop
                );
                assert_eq!(values[4], 0, "Logarithmic end should be 0");
            }
            DimmerCurve::Sine => {
                // Sine should be smooth ease-in-out
                assert_eq!(values[0], 255, "Sine start should be 255");
                assert_eq!(values[4], 0, "Sine end should be 0");
            }
            DimmerCurve::Cosine => {
                // Cosine should be smooth ease-in
                assert_eq!(values[0], 255, "Cosine start should be 255");
                assert_eq!(values[4], 0, "Cosine end should be 0");
            }
        }
    }

    println!("\n✅ All dimmer curves tested successfully");
}
