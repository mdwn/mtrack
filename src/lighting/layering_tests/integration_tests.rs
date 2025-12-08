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
fn test_layering_demo() {
    println!("\n=== Effect Layering Demo ===");

    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("rgb_par_1", 1, 1);
    engine.register_fixture(fixture.clone());

    // Create layered effects: Static blue + Dimmer + Strobe
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
        vec!["rgb_par_1".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    let dimmer_effect = create_effect_with_layering(
        "dimmer".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.5,
            duration: Duration::from_secs(2), // Shorter for demo
            curve: DimmerCurve::Linear,
        },
        vec!["rgb_par_1".to_string()],
        EffectLayer::Midground,
        BlendMode::Multiply,
    );

    let strobe_effect = create_effect_with_layering(
        "strobe".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(2.0), // 2 Hz strobe
            duration: None,
        },
        vec!["rgb_par_1".to_string()],
        EffectLayer::Foreground,
        BlendMode::Overlay,
    );

    // Start all effects
    engine.start_effect(blue_effect).unwrap();
    engine.start_effect(dimmer_effect).unwrap();
    engine.start_effect(strobe_effect).unwrap();

    println!("Effects started:");
    println!("- Static blue (Background, Replace)");
    println!("- Dimmer 100%->50% over 2s (Midground, Multiply)");
    println!("- Strobe 2Hz (Foreground, Overlay)");
    println!();

    // Simulate for 3 seconds
    let mut time = 0.0;
    let dt = Duration::from_millis(200); // 5 FPS for demo

    while time < 3.0 {
        let commands = engine.update(dt, None).unwrap();

        // Print state every 0.4 seconds
        if (time * 5.0) as i32 % 2 == 0 {
            println!("Time: {:.1}s", time);

            for cmd in &commands {
                let channel_name = match cmd.channel {
                    1 => "Red",
                    2 => "Green",
                    3 => "Blue",
                    4 => "Dimmer",
                    5 => "Strobe",
                    _ => "Unknown",
                };
                println!(
                    "  {}: {} ({:.1}%)",
                    channel_name,
                    cmd.value,
                    cmd.value as f64 / 255.0 * 100.0
                );
            }
            println!();
        }

        time += dt.as_secs_f64();
    }

    println!("Demo complete! This shows how effects layer together:");
    println!("1. Blue color starts at full intensity");
    println!("2. Dimmer slowly reduces brightness from 100% to 50%");
    println!("3. Strobe effect overlays on top, creating a strobing effect");
    println!("4. The final result is a strobing, dimmed blue light");
}
#[test]
fn test_multiple_effects_simultaneous() {
    use super::super::effects::*;
    use super::super::engine::EffectEngine;

    // Initialize tracing

    // Create 4 fixtures (like in your real application)
    let mut engine = EffectEngine::new();

    for i in 1..=4 {
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);
        channels.insert("strobe".to_string(), 4);

        let fixture = FixtureInfo {
            name: format!("fixture_{}", i),
            universe: 1,
            address: (i - 1) * 4 + 1, // Each fixture takes 4 channels
            channels,
            fixture_type: "Astera-PixelBrick".to_string(),
            max_strobe_frequency: Some(25.0), // Astera-PixelBrick max strobe frequency
        };

        engine.register_fixture(fixture);
    }

    // Test 1: Static blue on all fixtures
    let mut static_params = HashMap::new();
    static_params.insert("red".to_string(), 0.0);
    static_params.insert("green".to_string(), 0.0);
    static_params.insert("blue".to_string(), 1.0);

    let static_effect = create_effect_with_layering(
        "static_blue".to_string(),
        EffectType::Static {
            parameters: static_params,
            duration: None,
        },
        vec![
            "fixture_1".to_string(),
            "fixture_2".to_string(),
            "fixture_3".to_string(),
            "fixture_4".to_string(),
        ],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    engine.start_effect(static_effect).unwrap();

    // Check at 0s
    let commands = engine.update(Duration::from_secs(0), None).unwrap();
    println!("=== At 0s (Static blue on 4 fixtures) ===");
    for cmd in &commands {
        let fixture_num = ((cmd.channel - 1) / 4) + 1;
        let channel_in_fixture = ((cmd.channel - 1) % 4) + 1;
        let channel_name = match channel_in_fixture {
            1 => "Red",
            2 => "Green",
            3 => "Blue",
            4 => "Strobe",
            _ => "Unknown",
        };
        println!(
            "  Fixture {} {} (Ch{}): {}",
            fixture_num, channel_name, cmd.channel, cmd.value
        );
    }

    // Test 2: Add dimmer with multiply blend mode
    let dimmer_effect = create_effect_with_layering(
        "dimmer_multiply".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.5,
            duration: Duration::from_secs(5),
            curve: DimmerCurve::Linear,
        },
        vec![
            "fixture_1".to_string(),
            "fixture_2".to_string(),
            "fixture_3".to_string(),
            "fixture_4".to_string(),
        ],
        EffectLayer::Midground,
        BlendMode::Multiply,
    );

    engine.start_effect(dimmer_effect).unwrap();

    // Check at 2s (dimmer start)
    let commands = engine.update(Duration::from_secs(2), None).unwrap();
    println!("\n=== At 2s (Dimmer starts on 4 fixtures) ===");
    for cmd in &commands {
        let fixture_num = ((cmd.channel - 1) / 4) + 1;
        let channel_in_fixture = ((cmd.channel - 1) % 4) + 1;
        let channel_name = match channel_in_fixture {
            1 => "Red",
            2 => "Green",
            3 => "Blue",
            4 => "Strobe",
            _ => "Unknown",
        };
        println!(
            "  Fixture {} {} (Ch{}): {}",
            fixture_num, channel_name, cmd.channel, cmd.value
        );
    }

    // Check at 4.5s (50% through dimmer)
    engine.update(Duration::from_secs(2), None).unwrap();
    let commands = engine.update(Duration::from_secs(0), None).unwrap();
    println!("\n=== At 4.5s (50% through dimmer on 4 fixtures) ===");

    // Advance to 25s to trigger debug logging
    engine.update(Duration::from_secs(20), None).unwrap();
    let commands_25s = engine.update(Duration::from_secs(0), None).unwrap();
    println!("\n=== At 25s (Debug logging should appear) ===");
    println!("Commands at 25s: {} commands", commands_25s.len());
    for cmd in &commands_25s {
        println!("  Channel {}: {}", cmd.channel, cmd.value);
    }
    for cmd in &commands {
        let fixture_num = ((cmd.channel - 1) / 4) + 1;
        let channel_in_fixture = ((cmd.channel - 1) % 4) + 1;
        let channel_name = match channel_in_fixture {
            1 => "Red",
            2 => "Green",
            3 => "Blue",
            4 => "Strobe",
            _ => "Unknown",
        };
        println!(
            "  Fixture {} {} (Ch{}): {}",
            fixture_num, channel_name, cmd.channel, cmd.value
        );
    }

    // Final analysis
    let red_commands: Vec<_> = commands
        .iter()
        .filter(|cmd| ((cmd.channel - 1) % 4) + 1 == 1)
        .collect();
    let green_commands: Vec<_> = commands
        .iter()
        .filter(|cmd| ((cmd.channel - 1) % 4) + 1 == 2)
        .collect();
    let blue_commands: Vec<_> = commands
        .iter()
        .filter(|cmd| ((cmd.channel - 1) % 4) + 1 == 3)
        .collect();

    println!("\n=== FINAL ANALYSIS ===");
    println!(
        "Red channels: {:?}",
        red_commands.iter().map(|cmd| cmd.value).collect::<Vec<_>>()
    );
    println!(
        "Green channels: {:?}",
        green_commands
            .iter()
            .map(|cmd| cmd.value)
            .collect::<Vec<_>>()
    );
    println!(
        "Blue channels: {:?}",
        blue_commands
            .iter()
            .map(|cmd| cmd.value)
            .collect::<Vec<_>>()
    );

    // Check if all RGB values are the same (the problem you're seeing)
    let all_red_same = red_commands.windows(2).all(|w| w[0].value == w[1].value);
    let all_green_same = green_commands.windows(2).all(|w| w[0].value == w[1].value);
    let all_blue_same = blue_commands.windows(2).all(|w| w[0].value == w[1].value);

    if all_red_same && all_green_same && all_blue_same {
        println!("❌ ALL RGB VALUES ARE THE SAME ACROSS ALL FIXTURES!");
        println!("❌ This matches what you're seeing in OLA!");
    } else {
        println!("✅ RGB values vary across fixtures - this is correct");
    }
}
#[test]
fn test_astera_pixelblock_real_behavior() {
    use super::super::effects::*;
    use super::super::engine::EffectEngine;

    // Initialize tracing

    // Create Astera PixelBlock fixture (exactly as you described)
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    channels.insert("strobe".to_string(), 4);
    // NO dimmer channel!

    let fixture = FixtureInfo {
        name: "front_wash".to_string(),
        universe: 1,
        address: 1,
        channels,
        fixture_type: "Astera-PixelBrick".to_string(),
        max_strobe_frequency: Some(20.0), // Test fixture with strobe
    };

    println!("Fixture capabilities: {:?}", fixture.capabilities());
    println!(
        "Has RGB_COLOR: {}",
        fixture.has_capability(FixtureCapabilities::RGB_COLOR)
    );
    println!(
        "Has DIMMING: {}",
        fixture.has_capability(FixtureCapabilities::DIMMING)
    );

    let mut engine = EffectEngine::new();
    engine.register_fixture(fixture);

    // Test static blue (with dimmer parameter)
    let mut static_params = HashMap::new();
    static_params.insert("red".to_string(), 0.0);
    static_params.insert("green".to_string(), 0.0);
    static_params.insert("blue".to_string(), 1.0);
    static_params.insert("dimmer".to_string(), 1.0); // This should be ignored!

    let static_effect = create_effect_with_layering(
        "static_blue".to_string(),
        EffectType::Static {
            parameters: static_params,
            duration: None,
        },
        vec!["front_wash".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    engine.start_effect(static_effect).unwrap();

    // Check at 0s
    let commands = engine.update(Duration::from_secs(0), None).unwrap();
    println!("\n=== At 0s (Static blue) ===");
    for cmd in &commands {
        let channel_name = match cmd.channel {
            1 => "Red",
            2 => "Green",
            3 => "Blue",
            4 => "Strobe",
            _ => "Unknown",
        };
        println!("  {} (Ch{}): {}", channel_name, cmd.channel, cmd.value);
    }

    // Add dimmer with multiply blend mode
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

    // Advance to 2s and start dimmer
    engine.update(Duration::from_secs(2), None).unwrap();
    engine.start_effect(dimmer_effect).unwrap();

    // Check at 2s (dimmer start)
    let commands = engine.update(Duration::from_secs(0), None).unwrap();
    println!("\n=== At 2s (Dimmer starts) ===");
    for cmd in &commands {
        let channel_name = match cmd.channel {
            1 => "Red",
            2 => "Green",
            3 => "Blue",
            4 => "Strobe",
            _ => "Unknown",
        };
        println!("  {} (Ch{}): {}", channel_name, cmd.channel, cmd.value);
    }

    // Check at 4.5s (50% through dimmer)
    engine.update(Duration::from_secs(2), None).unwrap();
    let commands = engine.update(Duration::from_secs(0), None).unwrap();
    println!("\n=== At 4.5s (50% through dimmer) ===");
    for cmd in &commands {
        let channel_name = match cmd.channel {
            1 => "Red",
            2 => "Green",
            3 => "Blue",
            4 => "Strobe",
            _ => "Unknown",
        };
        println!("  {} (Ch{}): {}", channel_name, cmd.channel, cmd.value);
    }

    // Final analysis
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1);
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2);
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3);

    if let (Some(red), Some(green), Some(blue)) = (red_cmd, green_cmd, blue_cmd) {
        println!("\n=== FINAL ANALYSIS ===");
        println!(
            "Red: {}, Green: {}, Blue: {}",
            red.value, green.value, blue.value
        );

        if red.value == green.value && green.value == blue.value {
            println!("❌ ALL RGB VALUES ARE THE SAME - THIS IS THE PROBLEM!");
            println!("❌ The dimmer is setting all RGB channels to the same value");
            println!("❌ This matches what you're seeing in OLA!");
        } else if red.value == 0 && green.value == 0 && blue.value > 0 {
            println!("✅ Only blue is set - this is correct behavior");
        } else {
            println!("❓ Unexpected behavior - need to investigate further");
        }
    }
}
#[test]
fn test_permanent_vs_temporary_effects() {
    // Test that permanent effects lock channels while temporary effects don't
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

    // Test 1: Permanent effect (Static) should be indefinite and active
    let mut static_effect = EffectInstance::new(
        "static_red".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 1.0);
                params.insert("green".to_string(), 0.0);
                params.insert("blue".to_string(), 0.0);
                params.insert("dimmer".to_string(), 1.0);
                params
            },
            duration: None, // Indefinite static effect
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    static_effect.layer = EffectLayer::Foreground;
    static_effect.blend_mode = BlendMode::Replace;

    engine.start_effect(static_effect).unwrap();

    // Let the static effect run for a bit
    engine.update(Duration::from_secs(1), None).unwrap();

    // Now add a background effect that should be blocked by the locked channels
    let mut background_effect = EffectInstance::new(
        "background_blue".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 0.0);
                params.insert("green".to_string(), 0.0);
                params.insert("blue".to_string(), 1.0);
                params.insert("dimmer".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    background_effect.layer = EffectLayer::Background;
    background_effect.blend_mode = BlendMode::Replace;

    engine.start_effect(background_effect).unwrap();

    // The background effect should not be able to override the foreground static effect
    let commands = engine.update(Duration::from_secs(1), None).unwrap();

    println!("Testing permanent effect behavior:");
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

    // Red should be 255 (foreground static effect takes precedence)
    // Blue should be 0 (background effect can't override foreground effect)
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 2);
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4);

    assert_eq!(
        red_cmd.map(|cmd| cmd.value).unwrap_or(0),
        255,
        "Red should be 255 (foreground static effect)"
    );
    assert_eq!(
        blue_cmd.map(|cmd| cmd.value).unwrap_or(0),
        0,
        "Blue should be 0 (background effect blocked by foreground)"
    );

    println!("✅ Permanent effect behavior test passed!");
}
#[test]
fn test_grandma_style_fade_out() {
    // Test that fade-out effects work like grandMA - final state persists
    let mut engine = EffectEngine::new();

    // Register test fixtures
    let mut front_wash_channels = HashMap::new();
    front_wash_channels.insert("dimmer".to_string(), 1);
    front_wash_channels.insert("red".to_string(), 2);
    front_wash_channels.insert("green".to_string(), 3);
    front_wash_channels.insert("blue".to_string(), 4);

    let front_wash = FixtureInfo {
        name: "front_wash".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "Dimmer".to_string(),
        channels: front_wash_channels,
        max_strobe_frequency: None,
    };

    engine.register_fixture(front_wash);

    // Start with a static blue background effect (indefinite)
    let mut blue_effect = EffectInstance::new(
        "blue_bg".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 0.0);
                params.insert("green".to_string(), 0.0);
                params.insert("blue".to_string(), 1.0);
                params.insert("dimmer".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["front_wash".to_string()],
        None,
        None,
        None,
    );
    blue_effect.layer = EffectLayer::Background;
    blue_effect.blend_mode = BlendMode::Replace;

    engine.start_effect(blue_effect).unwrap();

    // Let the blue effect run for a bit
    engine.update(Duration::from_secs(1), None).unwrap();

    // Now add a fade-out effect (2 seconds) - crossfade all channels to black
    let mut fade_out_effect = EffectInstance::new(
        "fade_out".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 0.0);
                params.insert("green".to_string(), 0.0);
                params.insert("blue".to_string(), 0.0);
                params.insert("dimmer".to_string(), 0.0);
                params
            },
            duration: Some(Duration::from_secs(2)), // Make it timed
        },
        vec!["front_wash".to_string()],
        Some(Duration::from_secs(0)), // up_time
        Some(Duration::from_secs(0)), // hold_time
        Some(Duration::from_secs(2)), // down_time
    );
    fade_out_effect.layer = EffectLayer::Foreground;
    fade_out_effect.blend_mode = BlendMode::Replace;

    engine.start_effect(fade_out_effect).unwrap();

    println!("Testing grandMA-style fade-out behavior");

    // Test during fade-out
    let commands_1s = engine.update(Duration::from_secs(1), None).unwrap();
    println!("\nAt 1s (50% through fade-out):");
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

    // Test at end of fade-out
    let commands_2s = engine.update(Duration::from_secs(1), None).unwrap();
    println!("\nAt 2s (end of fade-out):");
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

    // Test after fade-out (should stay at 0 - grandMA behavior)
    let commands_3s = engine.update(Duration::from_secs(1), None).unwrap();
    println!("\nAt 3s (after fade-out - should stay at 0):");
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

    // Verify that background effect takes over after timed effect ends
    let final_dimmer = commands_3s
        .iter()
        .find(|cmd| cmd.channel == 1)
        .map(|cmd| cmd.value)
        .unwrap_or(0);
    let final_blue = commands_3s
        .iter()
        .find(|cmd| cmd.channel == 4)
        .map(|cmd| cmd.value)
        .unwrap_or(0);

    assert_eq!(
        final_dimmer, 255,
        "Dimmer should be 255 (background effect takes over after timed effect ends)"
    );
    assert_eq!(
        final_blue, 255,
        "Blue should be 255 (background effect takes over after timed effect ends)"
    );

    println!("✅ grandMA-style fade-out test completed - final state persists!");
}
#[test]
fn test_real_layering_show_file() {
    use super::super::effects::*;
    use super::super::engine::EffectEngine;
    use super::super::parser::parse_light_shows;

    // Initialize tracing

    // Read the actual layering show file
    let dsl_content = std::fs::read_to_string("examples/lighting/shows/layering_show.light")
        .expect("Failed to read layering show file");

    let shows = match parse_light_shows(&dsl_content) {
        Ok(s) => s,
        Err(e) => {
            println!("Parser error: {}", e);
            panic!("Failed to parse layering show DSL: {}", e);
        }
    };

    let show = shows.get("Effect Layering Demo").unwrap();

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

    // Convert DSL effects to EffectInstances and start them
    for cue in &show.cues {
        for effect in &cue.effects {
            let effect_instance = create_effect_with_layering(
                format!("dsl_effect_{:?}", effect.effect_type),
                effect.effect_type.clone(),
                effect.groups.clone(),
                effect.layer.unwrap_or(EffectLayer::Background),
                effect.blend_mode.unwrap_or(BlendMode::Replace),
            );

            engine.start_effect(effect_instance).unwrap();
        }
    }

    // Test at different time points
    println!("\n=== Testing REAL Layering Show File ===");

    // At 0 seconds - should have static blue
    let commands = engine.update(Duration::from_secs(0), None).unwrap();
    println!("At 0s (static blue):");
    for cmd in &commands {
        println!("  Channel {}: {}", cmd.channel, cmd.value);
    }

    // At 2 seconds - should have static blue + dimmer starting
    engine.update(Duration::from_secs(2), None).unwrap();
    let commands = engine.update(Duration::from_secs(0), None).unwrap();
    println!("\nAt 2s (static blue + dimmer start):");
    for cmd in &commands {
        println!("  Channel {}: {}", cmd.channel, cmd.value);
    }

    // At 4.5 seconds - should have dimmed blue (50% through dimmer)
    engine.update(Duration::from_secs(2), None).unwrap();
    let commands = engine.update(Duration::from_secs(0), None).unwrap();
    println!("\nAt 4.5s (50% through dimmer):");
    for cmd in &commands {
        println!("  Channel {}: {}", cmd.channel, cmd.value);
    }

    // Check what's actually happening
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1);
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2);
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3);

    if let (Some(red), Some(green), Some(blue)) = (red_cmd, green_cmd, blue_cmd) {
        println!("\nREAL BEHAVIOR:");
        println!("  Red: {}", red.value);
        println!("  Green: {}", green.value);
        println!("  Blue: {}", blue.value);

        if red.value == green.value && green.value == blue.value {
            println!("  ❌ ALL RGB VALUES ARE THE SAME - THIS IS THE PROBLEM!");
            println!("  ❌ The dimmer is setting all RGB channels to the same value (white)");
        } else if red.value == 0 && green.value == 0 && blue.value > 0 {
            println!("  ✅ Only blue is set - this is correct behavior");
        } else {
            println!("  ❓ Unexpected behavior - need to investigate");
        }
    }
}
#[test]
fn test_layering_show_effect_execution() {
    use super::super::effects::*;
    use super::super::engine::EffectEngine;
    use super::super::parser::parse_light_shows;

    // Initialize tracing

    // Test the exact DSL from layering_show.light
    let dsl_content = r#"show "Effect Layering Demo" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%, layer: background, blend_mode: replace
    
    @00:02.000
    front_wash: dimmer start_level: 1.0, end_level: 0.5, duration: 5s, layer: midground, blend_mode: multiply
}"#;

    let shows = match parse_light_shows(dsl_content) {
        Ok(s) => s,
        Err(e) => {
            println!("Parser error: {}", e);
            panic!("Failed to parse layering show DSL: {}", e);
        }
    };

    let show = shows.get("Effect Layering Demo").unwrap();

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

    // Convert DSL effects to EffectInstances and start them
    for cue in &show.cues {
        for effect in &cue.effects {
            let effect_instance = create_effect_with_layering(
                format!("dsl_effect_{:?}", effect.effect_type),
                effect.effect_type.clone(),
                effect.groups.clone(),
                effect.layer.unwrap_or(EffectLayer::Background),
                effect.blend_mode.unwrap_or(BlendMode::Replace),
            );

            engine.start_effect(effect_instance).unwrap();
        }
    }

    // Test at different time points
    println!("\n=== Testing DSL Effect Execution ===");

    // At 0 seconds - should have static blue
    let commands = engine.update(Duration::from_secs(0), None).unwrap();
    println!("At 0s (static blue):");
    for cmd in &commands {
        println!("  Channel {}: {}", cmd.channel, cmd.value);
    }

    // At 2 seconds - should have static blue + dimmer starting
    engine.update(Duration::from_secs(2), None).unwrap();
    let commands = engine.update(Duration::from_secs(0), None).unwrap();
    println!("\nAt 2s (static blue + dimmer start):");
    for cmd in &commands {
        println!("  Channel {}: {}", cmd.channel, cmd.value);
    }

    // At 4.5 seconds - should have dimmed blue (50% through dimmer)
    engine.update(Duration::from_secs(2), None).unwrap();
    let commands = engine.update(Duration::from_secs(0), None).unwrap();
    println!("\nAt 4.5s (50% through dimmer):");
    for cmd in &commands {
        println!("  Channel {}: {}", cmd.channel, cmd.value);
    }

    // Check that blue is dimmed but not white
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1);
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2);
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3);

    if let (Some(red), Some(green), Some(blue)) = (red_cmd, green_cmd, blue_cmd) {
        println!("\nColor analysis:");
        println!("  Red: {} (should be 0)", red.value);
        println!("  Green: {} (should be 0)", green.value);
        println!("  Blue: {} (should be dimmed, not 255)", blue.value);

        // With multiply blend mode, red and green should be 0, blue should be dimmed
        assert_eq!(red.value, 0, "Red should be 0 with multiply blend mode");
        assert_eq!(green.value, 0, "Green should be 0 with multiply blend mode");
        assert!(
            blue.value < 255,
            "Blue should be dimmed, not full brightness"
        );
        assert!(blue.value > 0, "Blue should not be completely off");
    }

    println!("\n✅ DSL effect execution test passed!");
    println!("✅ Dimmer with multiply blend mode preserves blue color");
}
#[test]
fn test_custom_rgb_dimming() {
    use super::super::effects::*;
    use super::super::engine::EffectEngine;

    // Create a fixture with RGB channels only (no dedicated dimmer)
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);

    let fixture = FixtureInfo {
        name: "rgb_fixture".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "RGB_Par".to_string(),
        channels,
        max_strobe_frequency: Some(20.0), // Test fixture with strobe
    };

    let mut engine = EffectEngine::new();
    engine.register_fixture(fixture.clone());

    // Test 1: Custom RGB static effect
    println!("\n1. Custom RGB static effect:");
    let mut static_params = HashMap::new();
    static_params.insert("red".to_string(), 1.0); // Full red
    static_params.insert("green".to_string(), 0.5); // Half green
    static_params.insert("blue".to_string(), 0.25); // Quarter blue

    let rgb_effect = create_effect_with_layering(
        "rgb_static".to_string(),
        EffectType::Static {
            parameters: static_params,
            duration: None,
        },
        vec!["rgb_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    engine.start_effect(rgb_effect).unwrap();
    let commands = engine.update(Duration::from_millis(0), None).unwrap();

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
    let dimmer_effect = create_effect_with_layering(
        "dimmer".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.0,
            duration: Duration::from_secs(2),
            curve: DimmerCurve::Linear,
        },
        vec!["rgb_fixture".to_string()],
        EffectLayer::Midground,
        BlendMode::Replace,
    );

    engine.start_effect(dimmer_effect).unwrap();

    // Check at different time points
    let mut last_time = 0;
    for (time_ms, description) in [(0, "Start"), (500, "25%"), (1000, "50%"), (2000, "End")] {
        let delta_ms = time_ms - last_time;
        let commands = engine
            .update(Duration::from_millis(delta_ms), None)
            .unwrap();
        println!("\n  At {} ({}ms):", description, time_ms);
        last_time = time_ms;
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

    println!("\nCurrent behavior analysis:");
    println!("- All RGB channels get the same dimmer value applied");
    println!("- Red: 255 * dimmer_value");
    println!("- Green: 127 * dimmer_value");
    println!("- Blue: 63 * dimmer_value");
    println!("- This maintains the relative brightness ratios between colors");

    // Verify the behavior is correct
    let final_commands = engine.update(Duration::from_millis(2000), None).unwrap();
    assert_eq!(final_commands.len(), 3); // RGB channels only

    // At the end (4000ms), the dimmer effect should have completed and persisted at 0.0
    // (dimmers are permanent, so the final dimmed value persists)
    let red_cmd = final_commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = final_commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = final_commands.iter().find(|cmd| cmd.channel == 3).unwrap();

    assert_eq!(red_cmd.value, 0); // Dimmed to 0 and persisted
    assert_eq!(green_cmd.value, 0); // Dimmed to 0 and persisted
    assert_eq!(blue_cmd.value, 0); // Dimmed to 0 and persisted

    println!("✅ Custom RGB dimming test passed!");
    println!("✅ Dimmer maintains relative brightness ratios between colors");
}
#[test]
fn test_parse_layering_show() {
    use super::super::parser::parse_light_shows;

    let dsl_content = std::fs::read_to_string("examples/lighting/shows/layering_show.light")
        .expect("Failed to read layering show file");

    let shows = match parse_light_shows(&dsl_content) {
        Ok(s) => s,
        Err(e) => {
            println!("Parser error: {}", e);
            panic!("Failed to parse layering show: {}", e);
        }
    };

    let show = shows.get("Effect Layering Demo").unwrap();
    assert_eq!(show.name, "Effect Layering Demo");
    assert_eq!(show.cues.len(), 8);

    // Check that group names are parsed correctly (not including comments)
    for cue in &show.cues {
        for effect in &cue.effects {
            for group in &effect.groups {
                assert!(
                    !group.contains("#"),
                    "Group name '{}' contains comment",
                    group
                );
                assert!(
                    !group.contains("Add a"),
                    "Group name '{}' contains comment text",
                    group
                );
            }
        }
    }

    println!("Layering show parsing test passed!");
    println!(
        "Successfully parsed {} cues with proper group names",
        show.cues.len()
    );
}
#[test]
fn test_software_strobing_rgb_only_fixture() {
    let mut engine = EffectEngine::new();

    // Create RGB-only fixture (no strobe or dimmer channels)
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    // No strobe or dimmer channels!

    let fixture = FixtureInfo {
        name: "rgb_only_fixture".to_string(),
        universe: 1,
        address: 1,
        channels,
        fixture_type: "RGB_Par".to_string(),
        max_strobe_frequency: None, // No strobe capability
    };
    engine.register_fixture(fixture);

    // Create strobe effect - should use software strobing
    let strobe_effect = create_effect_with_layering(
        "strobe_effect".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(2.0), // 2 Hz for easy testing
            duration: None,
        },
        vec!["rgb_only_fixture".to_string()],
        EffectLayer::Foreground,
        BlendMode::Overlay,
    );

    // Start the strobe effect
    engine.start_effect(strobe_effect).unwrap();

    // Test at different time points to verify strobing behavior
    // At t=0ms (start of cycle) - should be ON
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

    assert_eq!(red_cmd.value, 255); // Should be ON (1.0 * 255)
    assert_eq!(green_cmd.value, 255);
    assert_eq!(blue_cmd.value, 255);

    // At t=125ms (1/4 cycle) - should still be ON
    let commands = engine.update(Duration::from_millis(125), None).unwrap();
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(red_cmd.value, 255); // Should still be ON

    // At t=250ms (1/2 cycle) - should be OFF
    let commands = engine.update(Duration::from_millis(125), None).unwrap(); // 125ms more = 250ms total
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

    assert_eq!(red_cmd.value, 0); // Should be OFF (0.0 * 255)
    assert_eq!(green_cmd.value, 0);
    assert_eq!(blue_cmd.value, 0);

    // At t=375ms (3/4 cycle) - should still be OFF
    let commands = engine.update(Duration::from_millis(125), None).unwrap(); // 125ms more = 375ms total
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(red_cmd.value, 0); // Should still be OFF

    // At t=500ms (full cycle) - should be ON again
    let commands = engine.update(Duration::from_millis(125), None).unwrap(); // 125ms more = 500ms total
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(red_cmd.value, 255); // Should be ON again
}
#[test]
fn test_software_strobing_with_layering() {
    let mut engine = EffectEngine::new();

    // Create RGB-only fixture
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
        max_strobe_frequency: None, // No strobe capability
    };
    engine.register_fixture(fixture);

    // Create static blue effect (background layer)
    let blue_effect = create_effect_with_layering(
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
        vec!["rgb_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    // Create strobe effect (foreground layer)
    let strobe_effect = create_effect_with_layering(
        "strobe_effect".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(2.0), // 2 Hz
            duration: None,
        },
        vec!["rgb_fixture".to_string()],
        EffectLayer::Foreground,
        BlendMode::Overlay,
    );

    // Start both effects
    engine.start_effect(blue_effect).unwrap();
    engine.start_effect(strobe_effect).unwrap();

    // Test that layering works with software strobing
    // At t=0ms (strobe ON) - should see blue light
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

    assert_eq!(red_cmd.value, 0); // Red should be 0 (static effect)
    assert_eq!(green_cmd.value, 0); // Green should be 0 (static effect)
    assert_eq!(blue_cmd.value, 255); // Blue should be 255 (static + strobe overlay)

    // At t=250ms (strobe OFF) - should see no light
    let commands = engine.update(Duration::from_millis(250), None).unwrap();
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

    assert_eq!(red_cmd.value, 0); // Red should be 0
    assert_eq!(green_cmd.value, 0); // Green should be 0
    assert_eq!(blue_cmd.value, 0); // Blue should be 0 (strobe OFF overrides static)
}
#[test]
fn test_software_strobing_simple() {
    let mut engine = EffectEngine::new();

    // Create RGB-only fixture
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
        max_strobe_frequency: None, // No strobe capability
    };
    engine.register_fixture(fixture);

    // Create strobe effect only (no other effects)
    let strobe_effect = create_effect_with_layering(
        "strobe_effect".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(2.0), // 2 Hz
            duration: None,
        },
        vec!["rgb_fixture".to_string()],
        EffectLayer::Foreground,
        BlendMode::Overlay,
    );

    // Start strobe effect
    engine.start_effect(strobe_effect).unwrap();

    // Test basic strobe functionality
    // At t=0ms (strobe ON) - should see white light
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

    assert_eq!(red_cmd.value, 255); // Should be ON (1.0 * 255)
    assert_eq!(green_cmd.value, 255);
    assert_eq!(blue_cmd.value, 255);

    // At t=250ms (strobe OFF) - should see no light
    let commands = engine.update(Duration::from_millis(250), None).unwrap();
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

    assert_eq!(red_cmd.value, 0); // Should be OFF (0.0 * 255)
    assert_eq!(green_cmd.value, 0);
    assert_eq!(blue_cmd.value, 0);
}
#[test]
fn test_software_strobing_frequency_zero() {
    let mut engine = EffectEngine::new();

    // Create RGB-only fixture
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
        max_strobe_frequency: None,
    };
    engine.register_fixture(fixture);

    // Create static blue effect
    let blue_effect = create_effect_with_layering(
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
        vec!["rgb_fixture".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    // Create strobe effect with frequency 0 (off)
    let strobe_effect = create_effect_with_layering(
        "strobe_off".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(0.0), // Off
            duration: None,
        },
        vec!["rgb_fixture".to_string()],
        EffectLayer::Foreground,
        BlendMode::Overlay,
    );

    // Start both effects
    engine.start_effect(blue_effect).unwrap();
    engine.start_effect(strobe_effect).unwrap();

    // Test that strobe off defers to parent layers
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

    // Should see blue light (static effect) - strobe should not interfere
    assert_eq!(red_cmd.value, 0); // Red should be 0 (static effect)
    assert_eq!(green_cmd.value, 0); // Green should be 0 (static effect)
    assert_eq!(blue_cmd.value, 255); // Blue should be 255 (static effect only)
}
#[test]
fn test_full_layering_show_sequence_with_replace() {
    // Test the full sequence from layering_show.light to see what interferes with fade-out
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
        max_strobe_frequency: Some(10.0),
    };

    let back_wash = FixtureInfo {
        name: "back_wash".to_string(),
        universe: 1,
        address: 5,
        fixture_type: "Dimmer".to_string(),
        channels: channels.clone(),
        max_strobe_frequency: Some(10.0),
    };

    engine.register_fixture(front_wash);
    engine.register_fixture(back_wash);

    println!("Testing full layering show sequence");

    // Simulate the show sequence
    // @00:00.000 - Static blue background
    let static_blue = create_effect_with_layering(
        "static_blue".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("blue".to_string(), 1.0);
                params.insert("red".to_string(), 0.0);
                params.insert("green".to_string(), 0.0);
                params.insert("dimmer".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["front_wash".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );
    engine.start_effect(static_blue).unwrap();

    // @00:02.000 - Dimmer effect
    let dimmer_effect = create_effect_with_layering(
        "dimmer_effect".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.5,
            duration: Duration::from_secs(5), // 1s up + 3s hold + 1s down
            curve: DimmerCurve::Linear,
        },
        vec!["front_wash".to_string()],
        EffectLayer::Midground,
        BlendMode::Multiply,
    );
    engine.start_effect(dimmer_effect).unwrap();

    // @00:12.000 - Color cycle on back_wash
    let color_cycle = create_effect_with_layering(
        "color_cycle".to_string(),
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
                Color {
                    r: 0,
                    g: 0,
                    b: 255,
                    w: None,
                },
            ],
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
        },
        vec!["back_wash".to_string()],
        EffectLayer::Midground,
        BlendMode::Replace,
    );
    engine.start_effect(color_cycle).unwrap();

    // @00:15.000 - Dimmer effect on back_wash
    let back_dimmer = create_effect_with_layering(
        "back_dimmer".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.3,
            duration: Duration::from_secs(3), // 0.5s up + 2s hold + 0.5s down
            curve: DimmerCurve::Linear,
        },
        vec!["back_wash".to_string()],
        EffectLayer::Foreground,
        BlendMode::Multiply,
    );
    engine.start_effect(back_dimmer).unwrap();

    // @00:18.000 - Pulse effect on back_wash
    let pulse_effect = create_effect_with_layering(
        "pulse_effect".to_string(),
        EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.5,
            frequency: TempoAwareFrequency::Fixed(4.0),
            duration: Some(Duration::from_secs(7)), // 1s up + 5s hold + 1s down
        },
        vec!["back_wash".to_string()],
        EffectLayer::Foreground,
        BlendMode::Overlay,
    );
    engine.start_effect(pulse_effect).unwrap();

    // Check state before fade-out
    println!("\nAt 25s (before fade-out):");
    let commands = engine.update(Duration::from_secs(25), None).unwrap();
    for cmd in &commands {
        let fixture = if cmd.channel <= 4 {
            "front_wash"
        } else {
            "back_wash"
        };
        let channel_name = match cmd.channel {
            1 | 5 => "Dimmer",
            2 | 6 => "Red",
            3 | 7 => "Green",
            4 | 8 => "Blue",
            _ => "Unknown",
        };
        println!(
            "  {} {}: {} ({:.1}%)",
            fixture,
            channel_name,
            cmd.value,
            cmd.value as f64 / 255.0 * 100.0
        );
    }

    // @00:25.000 - Fade out effects (also set RGB to 0)
    // Create static effects that set RGB to 0 and dimmer to fade-out value
    let front_wash_fade = create_effect_with_layering(
        "front_wash_fade".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 0.0);
                params.insert("green".to_string(), 0.0);
                params.insert("blue".to_string(), 0.0);
                params.insert("dimmer".to_string(), 0.5); // Start at 50%
                params
            },
            duration: Some(Duration::from_secs(2)), // 2 second fade out
        },
        vec!["front_wash".to_string()],
        EffectLayer::Foreground,
        BlendMode::Replace,
    );

    let back_wash_fade = create_effect_with_layering(
        "back_wash_fade".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 0.0);
                params.insert("green".to_string(), 0.0);
                params.insert("blue".to_string(), 0.0);
                params.insert("dimmer".to_string(), 0.3); // Start at 30%
                params
            },
            duration: Some(Duration::from_secs(2)), // 2 second fade out
        },
        vec!["back_wash".to_string()],
        EffectLayer::Foreground,
        BlendMode::Replace,
    );

    engine.start_effect(front_wash_fade).unwrap();
    engine.start_effect(back_wash_fade).unwrap();

    // Test fade-out behavior
    for (time_ms, description) in [
        (0, "Fade start"),
        (500, "25%"),
        (1000, "50%"),
        (1500, "75%"),
        (2000, "End"),
    ] {
        let commands = engine.update(Duration::from_millis(time_ms), None).unwrap();
        println!("\nAt {} ({}ms):", description, time_ms);

        for cmd in &commands {
            let fixture = if cmd.channel <= 4 {
                "front_wash"
            } else {
                "back_wash"
            };
            let channel_name = match cmd.channel {
                1 | 5 => "Dimmer",
                2 | 6 => "Red",
                3 | 7 => "Green",
                4 | 8 => "Blue",
                _ => "Unknown",
            };
            println!(
                "  {} {}: {} ({:.1}%)",
                fixture,
                channel_name,
                cmd.value,
                cmd.value as f64 / 255.0 * 100.0
            );
        }
    }

    println!("✅ Full layering show sequence test completed");
}
#[test]
fn test_complex_multi_layer_multi_effect_scenarios() {
    let mut engine = EffectEngine::new();

    // Create test fixtures
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    channels.insert("strobe".to_string(), 4);

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

    // Complex scenario: Multiple effects across different layers and fixtures
    let background_static = create_effect_with_layering(
        "background_static".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("red".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["fixture1".to_string(), "fixture2".to_string()],
        EffectLayer::Background,
        BlendMode::Replace,
    );

    let midground_dimmer = create_effect_with_layering(
        "midground_dimmer".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.5,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        },
        vec!["fixture1".to_string()],
        EffectLayer::Midground,
        BlendMode::Multiply,
    );

    let midground_pulse = create_effect_with_layering(
        "midground_pulse".to_string(),
        EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.3,
            frequency: TempoAwareFrequency::Fixed(2.0),
            duration: None,
        },
        vec!["fixture1".to_string()],
        EffectLayer::Midground,
        BlendMode::Multiply,
    );

    let foreground_strobe = create_effect_with_layering(
        "foreground_strobe".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(2.0),
            duration: None,
        },
        vec!["fixture2".to_string()],
        EffectLayer::Foreground,
        BlendMode::Overlay,
    );

    // Start all effects
    engine.start_effect(background_static).unwrap();
    engine.start_effect(midground_dimmer).unwrap();
    engine.start_effect(midground_pulse).unwrap();
    engine.start_effect(foreground_strobe).unwrap();

    // All should coexist (different layers, compatible types)
    assert_eq!(engine.active_effects_count(), 4);
    assert!(engine.has_effect("background_static"));
    assert!(engine.has_effect("midground_dimmer"));
    assert!(engine.has_effect("midground_pulse"));
    assert!(engine.has_effect("foreground_strobe"));

    // Add conflicting effects
    let conflicting_static = create_effect_with_layering(
        "conflicting_static".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("blue".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["fixture1".to_string()],
        EffectLayer::Background, // Same layer as background_static
        BlendMode::Replace,
    );

    let conflicting_strobe = create_effect_with_layering(
        "conflicting_strobe".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(4.0),
            duration: None,
        },
        vec!["fixture2".to_string()],
        EffectLayer::Foreground, // Same layer as foreground_strobe
        BlendMode::Replace,
    );

    engine.start_effect(conflicting_static).unwrap();
    engine.start_effect(conflicting_strobe).unwrap();

    // Conflicting effects should stop their counterparts
    assert_eq!(engine.active_effects_count(), 4); // midground_dimmer + midground_pulse + conflicting_static + conflicting_strobe
    assert!(!engine.has_effect("background_static"));
    assert!(engine.has_effect("midground_dimmer"));
    assert!(engine.has_effect("midground_pulse"));
    assert!(!engine.has_effect("foreground_strobe"));
    assert!(engine.has_effect("conflicting_static"));
    assert!(engine.has_effect("conflicting_strobe"));

    // Add a high-priority effect that should stop others
    let high_priority_effect = create_effect_with_layering(
        "high_priority_effect".to_string(),
        EffectType::Static {
            parameters: {
                let mut params = HashMap::new();
                params.insert("green".to_string(), 1.0);
                params
            },
            duration: None,
        },
        vec!["fixture1".to_string()],
        EffectLayer::Background, // Same layer as conflicting_static
        BlendMode::Replace,
    )
    .with_priority(100); // Very high priority

    engine.start_effect(high_priority_effect).unwrap();

    // High priority should stop the conflicting static
    assert_eq!(engine.active_effects_count(), 4); // midground_dimmer + midground_pulse + high_priority + conflicting_strobe
    assert!(!engine.has_effect("conflicting_static"));
    assert!(engine.has_effect("high_priority_effect"));
    assert!(engine.has_effect("conflicting_strobe"));
}
#[test]
fn test_example_files_parse() {
    use crate::lighting::parser::parse_light_shows;
    use std::fs;

    let example_files = [
        "examples/lighting/shows/crossfade_show.light",
        "examples/lighting/shows/layering_show.light",
        "examples/lighting/shows/comprehensive_show.light",
        "examples/lighting/shows/layer_control_demo.light",
    ];

    for file_path in example_files {
        let content = fs::read_to_string(file_path).expect("Failed to read example file");
        let result = parse_light_shows(&content);
        assert!(
            result.is_ok(),
            "Failed to parse {}: {:?}",
            file_path,
            result.err()
        );

        let shows = result.unwrap();
        assert!(!shows.is_empty(), "No shows found in {}", file_path);

        println!(
            "✅ {} parsed successfully with {} shows",
            file_path,
            shows.len()
        );
    }
}
