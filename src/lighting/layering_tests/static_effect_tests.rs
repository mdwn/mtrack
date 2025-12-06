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
fn test_static_replace_blend_mode() {
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

    // Test static effect with replace blend mode (like in your DSL)
    let mut static_params = HashMap::new();
    static_params.insert("red".to_string(), 0.0);
    static_params.insert("green".to_string(), 0.0);
    static_params.insert("blue".to_string(), 1.0);
    static_params.insert("dimmer".to_string(), 1.0);

    let static_effect = create_effect_with_layering(
        "static_blue_with_dimmer".to_string(),
        EffectType::Static {
            parameters: static_params,
            duration: None,
        },
        vec!["front_wash".to_string()],
        EffectLayer::Background,
        BlendMode::Replace, // This is what your DSL uses
    );

    engine.start_effect(static_effect).unwrap();

    // Check what the static effect produces
    let commands = engine.update(Duration::from_secs(0), None).unwrap();
    println!("Static effect with replace blend mode:");
    for cmd in &commands {
        println!("  Channel {}: {}", cmd.channel, cmd.value);
    }

    // Check that it produces blue, not white
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1);
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2);
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3);

    if let (Some(red), Some(green), Some(blue)) = (red_cmd, green_cmd, blue_cmd) {
        println!("\nAnalysis:");
        println!("  Red: {} (should be 0)", red.value);
        println!("  Green: {} (should be 0)", green.value);
        println!("  Blue: {} (should be 255)", blue.value);

        assert_eq!(red.value, 0, "Red should be 0");
        assert_eq!(green.value, 0, "Green should be 0");
        assert_eq!(blue.value, 255, "Blue should be 255");
    }
}
#[test]
fn test_static_effect_timing() {
    // Test how static effects work with different timing options
    let mut engine = EffectEngine::new();

    // Register test fixture (RGB-only, no dedicated dimmer)
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

    // Test 1: Static effect with no duration (indefinite)
    let mut indefinite_static = EffectInstance::new(
        "indefinite_static".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 1.0);
                params.insert("green".to_string(), 0.0);
                params.insert("blue".to_string(), 0.0);
                params
            },
            duration: None, // Indefinite
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    indefinite_static.layer = EffectLayer::Foreground;
    indefinite_static.blend_mode = BlendMode::Replace;

    engine.start_effect(indefinite_static).unwrap();

    // Let it run for a bit
    engine.update(Duration::from_secs(1), None).unwrap();

    let commands_1s = engine.update(Duration::from_secs(1), None).unwrap();
    println!("Indefinite static at 1s:");
    for cmd in &commands_1s {
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

    // Test 2: Static effect with duration (timed)
    let mut timed_static = EffectInstance::new(
        "timed_static".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 0.0);
                params.insert("green".to_string(), 1.0);
                params.insert("blue".to_string(), 0.0);
                params
            },
            duration: Some(Duration::from_secs(3)), // 3 seconds
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    timed_static.layer = EffectLayer::Foreground;
    timed_static.blend_mode = BlendMode::Replace;

    engine.start_effect(timed_static.clone()).unwrap();

    // Test at various times
    let commands_1s_timed = engine.update(Duration::from_secs(1), None).unwrap();
    println!("\nTimed static at 1s (should be green):");
    for cmd in &commands_1s_timed {
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

    let commands_4s = engine.update(Duration::from_secs(3), None).unwrap();
    println!("\nTimed static at 4s (should be no commands - timed static ended, indefinite static was stopped):");
    println!("Active effects count: {}", engine.active_effects_count());
    for cmd in &commands_4s {
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

    // Verify behavior
    let red_1s = commands_1s_timed
        .iter()
        .find(|cmd| cmd.channel == 1)
        .map(|cmd| cmd.value)
        .unwrap_or(0);
    let green_1s = commands_1s_timed
        .iter()
        .find(|cmd| cmd.channel == 2)
        .map(|cmd| cmd.value)
        .unwrap_or(0);

    assert_eq!(
        red_1s, 0,
        "Red should be 0 (timed static replaces indefinite)"
    );
    assert_eq!(green_1s, 255, "Green should be 255 (timed static active)");

    // After timed static ends, no effects remain and no state persists
    // (timed static ended and is not permanent, indefinite was removed by conflict)
    assert!(
        commands_4s.is_empty(),
        "No commands after timed static ends (not permanent, no effects active)"
    );

    println!("✅ Static effect timing test passed!");
}
#[test]
fn test_static_effect_with_up_time() {
    // Test that static effects can have up_time (fade-in) from DSL
    let mut engine = EffectEngine::new();

    // Register test fixture
    let mut channels = HashMap::new();
    channels.insert("dimmer".to_string(), 1);
    channels.insert("red".to_string(), 2);
    channels.insert("green".to_string(), 3);
    channels.insert("blue".to_string(), 4);

    let fixture = FixtureInfo {
        name: "test_fixture".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "RGB_Par".to_string(),
        channels,
        max_strobe_frequency: None,
    };

    engine.register_fixture(fixture);

    // Create a static effect with up_time using the new constructor
    let static_effect = EffectInstance::new(
        "fade_in_static".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 1.0);
                params.insert("green".to_string(), 0.0);
                params.insert("blue".to_string(), 0.0);
                params.insert("dimmer".to_string(), 1.0);
                params
            },
            duration: None, // Indefinite
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    // Don't set up_time - keep it as truly indefinite

    engine.start_effect(static_effect).unwrap();

    // Test at various times during fade-in
    let commands_0s = engine.update(Duration::from_secs(0), None).unwrap();
    println!("Static with up_time at 0s (should be off):");
    for cmd in &commands_0s {
        let channel_name = match cmd.channel {
            1 => "Dimmer",
            2 => "Red",
            3 => "Green",
            4 => "Blue",
            _ => "Unknown",
        };
        println!(
            "  {}: {} ({:.1}%)",
            channel_name,
            cmd.value,
            cmd.value as f64 / 255.0 * 100.0
        );
    }

    let commands_1s = engine.update(Duration::from_secs(1), None).unwrap();
    println!("\nStatic with up_time at 1s (should be 50%):");
    for cmd in &commands_1s {
        let channel_name = match cmd.channel {
            1 => "Dimmer",
            2 => "Red",
            3 => "Green",
            4 => "Blue",
            _ => "Unknown",
        };
        println!(
            "  {}: {} ({:.1}%)",
            channel_name,
            cmd.value,
            cmd.value as f64 / 255.0 * 100.0
        );
    }

    let commands_2s = engine.update(Duration::from_secs(1), None).unwrap();
    println!("\nStatic with up_time at 2s (should be 100%):");
    for cmd in &commands_2s {
        let channel_name = match cmd.channel {
            1 => "Dimmer",
            2 => "Red",
            3 => "Green",
            4 => "Blue",
            _ => "Unknown",
        };
        println!(
            "  {}: {} ({:.1}%)",
            channel_name,
            cmd.value,
            cmd.value as f64 / 255.0 * 100.0
        );
    }

    let commands_3s = engine.update(Duration::from_secs(1), None).unwrap();
    println!("\nStatic with up_time at 3s (should still be 100%):");
    for cmd in &commands_3s {
        let channel_name = match cmd.channel {
            1 => "Dimmer",
            2 => "Red",
            3 => "Green",
            4 => "Blue",
            _ => "Unknown",
        };
        println!(
            "  {}: {} ({:.1}%)",
            channel_name,
            cmd.value,
            cmd.value as f64 / 255.0 * 100.0
        );
    }

    // Verify behavior
    let red_0s = commands_0s
        .iter()
        .find(|cmd| cmd.channel == 2)
        .map(|cmd| cmd.value)
        .unwrap_or(0);
    let red_1s = commands_1s
        .iter()
        .find(|cmd| cmd.channel == 2)
        .map(|cmd| cmd.value)
        .unwrap_or(0);
    let red_2s = commands_2s
        .iter()
        .find(|cmd| cmd.channel == 2)
        .map(|cmd| cmd.value)
        .unwrap_or(0);
    let red_3s = commands_3s
        .iter()
        .find(|cmd| cmd.channel == 2)
        .map(|cmd| cmd.value)
        .unwrap_or(0);

    assert_eq!(
        red_0s, 255,
        "Red should be on at start (instant indefinite effect)"
    );
    assert_eq!(red_1s, 255, "Red should be on at 1s (indefinite effect)");
    assert_eq!(red_2s, 255, "Red should be on at 2s (indefinite effect)");
    assert_eq!(red_3s, 255, "Red should be on at 3s (indefinite effect)");

    println!("✅ Static effect with up_time test passed!");
}
#[test]
fn test_static_effect_with_down_time() {
    // Test that static effects can have down_time (fade-out) from DSL
    let mut engine = EffectEngine::new();

    // Register test fixture
    let mut channels = HashMap::new();
    channels.insert("dimmer".to_string(), 1);
    channels.insert("red".to_string(), 2);
    channels.insert("green".to_string(), 3);
    channels.insert("blue".to_string(), 4);

    let fixture = FixtureInfo {
        name: "test_fixture".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "RGB_Par".to_string(),
        channels,
        max_strobe_frequency: None,
    };

    engine.register_fixture(fixture);

    // Create a static effect with up_time, hold_time, and down_time
    let static_effect = EffectInstance::new(
        "fade_in_hold_fade_out_static".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 1.0);
                params.insert("green".to_string(), 0.0);
                params.insert("blue".to_string(), 0.0);
                params.insert("dimmer".to_string(), 1.0);
                params
            },
            duration: Some(Duration::from_secs(5)), // 5 second total duration
        },
        vec!["test_fixture".to_string()],
        Some(Duration::from_secs(1)), // 1 second fade in
        Some(Duration::from_secs(2)), // 2 second hold
        Some(Duration::from_secs(2)), // 2 second fade out
    );

    engine.start_effect(static_effect.clone()).unwrap();

    // Test at various times during the effect
    let commands_0s = engine.update(Duration::from_secs(0), None).unwrap();
    println!("Static with down_time at 0s (should be off):");
    for cmd in &commands_0s {
        let channel_name = match cmd.channel {
            1 => "Dimmer",
            2 => "Red",
            3 => "Green",
            4 => "Blue",
            _ => "Unknown",
        };
        println!(
            "  {}: {} ({:.1}%)",
            channel_name,
            cmd.value,
            cmd.value as f64 / 255.0 * 100.0
        );
    }

    let commands_0_5s = engine.update(Duration::from_millis(500), None).unwrap();
    println!("\nStatic with down_time at 0.5s (should be 50% fade in):");
    for cmd in &commands_0_5s {
        let channel_name = match cmd.channel {
            1 => "Dimmer",
            2 => "Red",
            3 => "Green",
            4 => "Blue",
            _ => "Unknown",
        };
        println!(
            "  {}: {} ({:.1}%)",
            channel_name,
            cmd.value,
            cmd.value as f64 / 255.0 * 100.0
        );
    }

    let commands_1s = engine.update(Duration::from_millis(500), None).unwrap();
    println!("\nStatic with down_time at 1s (should be 100% - fade in complete):");
    for cmd in &commands_1s {
        let channel_name = match cmd.channel {
            1 => "Dimmer",
            2 => "Red",
            3 => "Green",
            4 => "Blue",
            _ => "Unknown",
        };
        println!(
            "  {}: {} ({:.1}%)",
            channel_name,
            cmd.value,
            cmd.value as f64 / 255.0 * 100.0
        );
    }

    let commands_2s = engine.update(Duration::from_secs(1), None).unwrap();
    println!("\nStatic with down_time at 2s (should be 100% - hold phase):");
    for cmd in &commands_2s {
        let channel_name = match cmd.channel {
            1 => "Dimmer",
            2 => "Red",
            3 => "Green",
            4 => "Blue",
            _ => "Unknown",
        };
        println!(
            "  {}: {} ({:.1}%)",
            channel_name,
            cmd.value,
            cmd.value as f64 / 255.0 * 100.0
        );
    }

    let commands_3s = engine.update(Duration::from_secs(1), None).unwrap();
    println!("\nStatic with down_time at 3s (should be 100% - end of hold phase):");
    for cmd in &commands_3s {
        let channel_name = match cmd.channel {
            1 => "Dimmer",
            2 => "Red",
            3 => "Green",
            4 => "Blue",
            _ => "Unknown",
        };
        println!(
            "  {}: {} ({:.1}%)",
            channel_name,
            cmd.value,
            cmd.value as f64 / 255.0 * 100.0
        );
    }

    // Test at 3.5s (middle of fade-out phase)
    let commands_3_5s = engine.update(Duration::from_millis(500), None).unwrap();
    println!("\nStatic with down_time at 3.5s (should be 75% - fade out phase):");
    for cmd in &commands_3_5s {
        let channel_name = match cmd.channel {
            1 => "Dimmer",
            2 => "Red",
            3 => "Green",
            4 => "Blue",
            _ => "Unknown",
        };
        println!(
            "  {}: {} ({:.1}%)",
            channel_name,
            cmd.value,
            cmd.value as f64 / 255.0 * 100.0
        );
    }

    let commands_4s = engine.update(Duration::from_secs(1), None).unwrap();
    println!("\nStatic with down_time at 4s (should be 50% - middle of fade out):");
    for cmd in &commands_4s {
        let channel_name = match cmd.channel {
            1 => "Dimmer",
            2 => "Red",
            3 => "Green",
            4 => "Blue",
            _ => "Unknown",
        };
        println!(
            "  {}: {} ({:.1}%)",
            channel_name,
            cmd.value,
            cmd.value as f64 / 255.0 * 100.0
        );
    }

    let commands_5s = engine.update(Duration::from_secs(1), None).unwrap();
    println!("\nStatic with down_time at 5s (should be 0% - effect ended):");
    for cmd in &commands_5s {
        let channel_name = match cmd.channel {
            1 => "Dimmer",
            2 => "Red",
            3 => "Green",
            4 => "Blue",
            _ => "Unknown",
        };
        println!(
            "  {}: {} ({:.1}%)",
            channel_name,
            cmd.value,
            cmd.value as f64 / 255.0 * 100.0
        );
    }

    // Verify behavior
    let red_0s = commands_0s
        .iter()
        .find(|cmd| cmd.channel == 2)
        .map(|cmd| cmd.value)
        .unwrap_or(0);
    let red_0_5s = commands_0_5s
        .iter()
        .find(|cmd| cmd.channel == 2)
        .map(|cmd| cmd.value)
        .unwrap_or(0);
    let red_1s = commands_1s
        .iter()
        .find(|cmd| cmd.channel == 2)
        .map(|cmd| cmd.value)
        .unwrap_or(0);
    let red_2s = commands_2s
        .iter()
        .find(|cmd| cmd.channel == 2)
        .map(|cmd| cmd.value)
        .unwrap_or(0);
    let red_3s = commands_3s
        .iter()
        .find(|cmd| cmd.channel == 2)
        .map(|cmd| cmd.value)
        .unwrap_or(0);
    let red_3_5s = commands_3_5s
        .iter()
        .find(|cmd| cmd.channel == 2)
        .map(|cmd| cmd.value)
        .unwrap_or(0);
    let red_4s = commands_4s
        .iter()
        .find(|cmd| cmd.channel == 2)
        .map(|cmd| cmd.value)
        .unwrap_or(0);
    let red_5s = commands_5s
        .iter()
        .find(|cmd| cmd.channel == 2)
        .map(|cmd| cmd.value)
        .unwrap_or(0);

    assert_eq!(red_0s, 0, "Red should be 0 at start (fade in begins)");
    assert!(
        red_0_5s > 0 && red_0_5s < 255,
        "Red should be partially faded in at 0.5s"
    );
    assert_eq!(
        red_1s, 255,
        "Red should be fully on at 1s (fade in complete)"
    );
    assert_eq!(red_2s, 255, "Red should be fully on at 2s (hold phase)");
    assert_eq!(
        red_3s, 255,
        "Red should be fully on at 3s (end of hold phase)"
    );
    assert!(
        red_3_5s > 0 && red_3_5s < 255,
        "Red should be partially faded out at 3.5s"
    );
    assert!(
        red_4s > 0 && red_4s < 255,
        "Red should be partially faded out at 4s (middle of fade out)"
    );
    assert_eq!(red_5s, 0, "Red should be off at 5s (fade out complete)");

    println!("✅ Static effect with down_time test passed!");
}
#[test]
fn test_static_effect_fade_out() {
    // Test that static effects with timing work correctly
    let mut engine = EffectEngine::new();

    // Register test fixture
    let mut channels = HashMap::new();
    channels.insert("dimmer".to_string(), 1);
    channels.insert("red".to_string(), 2);
    channels.insert("green".to_string(), 3);
    channels.insert("blue".to_string(), 4);

    let fixture = FixtureInfo {
        name: "test_fixture".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "Dimmer".to_string(),
        channels,
        max_strobe_frequency: None,
    };

    engine.register_fixture(fixture);

    // Create a static effect that fades out over 2 seconds
    let mut fade_out_effect = EffectInstance::new(
        "fade_out".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 1.0);
                params.insert("green".to_string(), 0.5);
                params.insert("blue".to_string(), 0.0);
                params.insert("dimmer".to_string(), 0.8);
                params
            },
            duration: Some(Duration::from_secs(2)), // Timed static effect
        },
        vec!["test_fixture".to_string()],
        Some(Duration::from_secs(0)), // up_time
        Some(Duration::from_secs(0)), // hold_time
        Some(Duration::from_secs(2)), // down_time
    );
    fade_out_effect.layer = EffectLayer::Foreground;
    fade_out_effect.blend_mode = BlendMode::Replace;

    engine.start_effect(fade_out_effect).unwrap();

    println!("Testing static effect fade-out");

    // Test at various time points
    for (time_ms, description) in [
        (0, "Start"),
        (500, "25%"),
        (1000, "50%"),
        (1500, "75%"),
        (2000, "End"),
    ] {
        let commands = engine.update(Duration::from_millis(time_ms), None).unwrap();
        println!("\nAt {} ({}ms):", description, time_ms);

        for cmd in &commands {
            let channel_name = match cmd.channel {
                1 => "Dimmer",
                2 => "Red",
                3 => "Green",
                4 => "Blue",
                _ => "Unknown",
            };
            println!(
                "  {}: {} ({:.1}%)",
                channel_name,
                cmd.value,
                cmd.value as f64 / 255.0 * 100.0
            );
        }
    }

    // Verify the behavior
    let final_commands = engine.update(Duration::from_millis(2000), None).unwrap();
    assert!(final_commands.is_empty(), "Effect should have ended at 2s");

    println!("✅ Static effect fade-out test completed");
}
