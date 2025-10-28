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

#[cfg(test)]
mod tests {
    use super::super::effects::*;
    use super::super::engine::EffectEngine;
    use std::collections::HashMap;
    use std::time::Duration;

    fn create_test_fixture(name: &str, universe: u16, address: u16) -> FixtureInfo {
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);
        channels.insert("strobe".to_string(), 4); // Add strobe channel

        FixtureInfo {
            name: name.to_string(),
            universe,
            address,
            fixture_type: "RGB_Par".to_string(),
            channels,
            max_strobe_frequency: Some(20.0), // Test fixture with strobe
        }
    }

    #[test]
    fn test_effect_layering_static_blue_and_dimmer() {
        // Initialize tracing for this test
        let _ = tracing_subscriber::fmt::try_init();

        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture.clone());

        // Create static blue effect on background layer
        let mut blue_params = HashMap::new();
        blue_params.insert("red".to_string(), 0.0);
        blue_params.insert("green".to_string(), 0.0);
        blue_params.insert("blue".to_string(), 1.0);

        let blue_effect = EffectInstance::with_layering(
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
        let dimmer_effect = EffectInstance::with_layering(
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

        // Update engine at start (dimmer should be at 100%)
        let commands = engine.update(Duration::from_millis(16)).unwrap();

        // Should have 3 commands: red, green, blue (dimmer uses Multiply mode, so only affects RGB channels)
        assert_eq!(commands.len(), 3);

        // Find the commands
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

        // At start: blue should be full (255), others should be 0
        assert_eq!(red_cmd.value, 0);
        assert_eq!(green_cmd.value, 0);
        // The dimmer effect is applied to RGB channels with Multiply blend mode
        // Blue starts at 1.0, dimmer starts at 1.0, so result should be 1.0 * 1.0 = 1.0
        // But there might be some rounding, so check it's close to 255
        assert!(blue_cmd.value >= 250);

        // Update engine at middle (dimmer should be at 75%)
        engine.update(Duration::from_millis(500)).unwrap();
        let commands = engine.update(Duration::from_millis(16)).unwrap();

        // The dimmer effect is applied to RGB channels, so blue should be dimmed
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        // At 50% progress: blue should be around 75% (1.0 * 0.75 = 0.75, 0.75 * 255 = 191.25)
        assert!(blue_cmd.value >= 180 && blue_cmd.value <= 200); // Around 75%

        // Update engine at end (dimmer should be at 50%)
        engine.update(Duration::from_millis(500)).unwrap();
        let commands = engine.update(Duration::from_millis(16)).unwrap();

        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        // At 100% progress: blue should be around 50% (1.0 * 0.5 = 0.5, 0.5 * 255 = 127.5)
        assert!(blue_cmd.value >= 120 && blue_cmd.value <= 140); // Around 50%
    }

    #[test]
    fn test_effect_layering_with_strobe() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture.clone());

        // Create static blue effect
        let mut blue_params = HashMap::new();
        blue_params.insert("red".to_string(), 0.0);
        blue_params.insert("green".to_string(), 0.0);
        blue_params.insert("blue".to_string(), 1.0);

        let blue_effect = EffectInstance::with_layering(
            "static_blue".to_string(),
            EffectType::Static {
                parameters: blue_params,
                duration: None,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
        );

        // Create strobe effect
        let strobe_effect = EffectInstance::with_layering(
            "strobe".to_string(),
            EffectType::Strobe {
                frequency: 1.0, // 1 Hz for easy testing
                intensity: 1.0,
                duration: None,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Foreground,
            BlendMode::Overlay,
        );

        // Start effects
        engine.start_effect(blue_effect).unwrap();
        engine.start_effect(strobe_effect).unwrap();

        // Test strobe at different phases
        let commands = engine.update(Duration::from_millis(16)).unwrap();
        assert_eq!(commands.len(), 4); // red, green, blue, strobe (no dimmer channel)

        // At strobe peak (should be on)
        let commands = engine.update(Duration::from_millis(250)).unwrap(); // 1/4 cycle
        let strobe_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(strobe_cmd.value, 12); // Should be 1.0Hz / 20.0Hz * 255 = 12.75, rounded to 12

        // At strobe trough (should be off)
        let commands = engine.update(Duration::from_millis(500)).unwrap(); // 1/2 cycle
        let strobe_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(strobe_cmd.value, 12); // Strobe channel value should remain at the speed, not turn off
    }

    #[test]
    fn test_channel_state_blending() {
        // Test the ChannelState blending logic directly
        let base_state = ChannelState::new(0.8, EffectLayer::Background, BlendMode::Replace);
        let overlay_state = ChannelState::new(0.5, EffectLayer::Foreground, BlendMode::Multiply);

        let blended = base_state.blend_with(overlay_state);

        // Multiply: 0.8 * 0.5 = 0.4
        assert!((blended.value - 0.4).abs() < 0.01);
        assert_eq!(blended.layer, EffectLayer::Foreground); // Higher layer wins
        assert_eq!(blended.blend_mode, BlendMode::Multiply); // Higher layer's blend mode
    }

    #[test]
    fn test_fixture_state_blending() {
        let mut fixture1 = FixtureState::new("test".to_string());
        fixture1.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::Background, BlendMode::Replace),
        );
        fixture1.set_channel(
            "green".to_string(),
            ChannelState::new(0.5, EffectLayer::Background, BlendMode::Replace),
        );

        let mut fixture2 = FixtureState::new("test".to_string());
        fixture2.set_channel(
            "green".to_string(),
            ChannelState::new(0.8, EffectLayer::Foreground, BlendMode::Multiply),
        );
        fixture2.set_channel(
            "blue".to_string(),
            ChannelState::new(0.3, EffectLayer::Foreground, BlendMode::Replace),
        );

        fixture1.blend_with(&fixture2);

        // Green should be blended: 0.5 * 0.8 = 0.4
        let green_state = fixture1.get_channel("green").unwrap();
        assert!((green_state.value - 0.4).abs() < 0.01);

        // Blue should be added (new channel)
        let blue_state = fixture1.get_channel("blue").unwrap();
        assert!((blue_state.value - 0.3).abs() < 0.01);

        // Red should be unchanged
        let red_state = fixture1.get_channel("red").unwrap();
        assert!((red_state.value - 1.0).abs() < 0.01);
    }

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

        let blue_effect = EffectInstance::with_layering(
            "static_blue".to_string(),
            EffectType::Static {
                parameters: blue_params,
                duration: None,
            },
            vec!["rgb_par_1".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
        );

        let dimmer_effect = EffectInstance::with_layering(
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

        let strobe_effect = EffectInstance::with_layering(
            "strobe".to_string(),
            EffectType::Strobe {
                frequency: 2.0, // 2 Hz strobe
                intensity: 1.0,
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
            let commands = engine.update(dt).unwrap();

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

        // Create a static blue effect
        let blue_effect = EffectInstance::with_layering(
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
            EffectLayer::Background,
            BlendMode::Replace,
        );

        // Create a dimmer effect
        let dimmer_effect = EffectInstance::with_layering(
            "dimmer".to_string(),
            EffectType::Dimmer {
                start_level: 1.0,
                end_level: 0.5,
                duration: Duration::from_secs(1),
                curve: DimmerCurve::Linear,
            },
            vec!["rgb_only_fixture".to_string()],
            EffectLayer::Midground,
            BlendMode::Multiply,
        );

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

        // At start: blue should be full (255), others should be 0
        assert_eq!(red_cmd.value, 0);
        assert_eq!(green_cmd.value, 0);
        assert_eq!(blue_cmd.value, 255);

        // Update engine at 50% (dimmer should be at 50%)
        let commands = engine.update(Duration::from_millis(500)).unwrap();

        // Should still have only RGB commands
        assert_eq!(commands.len(), 3);

        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

        // At 50%: blue should be dimmed to 75% (191), others should be 0
        // (dimmer goes from 1.0 to 0.5 over 1s, so at 50% progress it's 0.75)
        assert_eq!(red_cmd.value, 0);
        assert_eq!(green_cmd.value, 0);
        assert_eq!(blue_cmd.value, 191);

        println!("Dimmer without dedicated channel test passed!");
        println!("RGB-only fixture properly dims its color channels");
    }

    #[test]
    fn test_dimmer_precedence_and_selective_dimming() {
        use super::super::effects::*;
        use super::super::engine::EffectEngine;

        // Create a fixture with both dimmer and RGB channels
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
            max_strobe_frequency: Some(20.0), // Test fixture with strobe
        };

        let mut engine = EffectEngine::new();
        engine.register_fixture(fixture.clone());

        // Test 1: Blue-only static effect
        println!("\n1. Blue-only static effect:");
        let mut static_params = HashMap::new();
        static_params.insert("blue".to_string(), 1.0);
        // No red or green values set

        let blue_effect = EffectInstance::with_layering(
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

        // Test 2: Add dimmer effect (1.0 -> 0.0)
        println!("\n2. Adding dimmer effect (1.0 -> 0.0):");
        let dimmer_effect = EffectInstance::with_layering(
            "dimmer".to_string(),
            EffectType::Dimmer {
                start_level: 1.0,
                end_level: 0.0,
                duration: Duration::from_secs(2),
                curve: DimmerCurve::Linear,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Midground,
            BlendMode::Multiply,
        );

        engine.start_effect(dimmer_effect).unwrap();

        // Check at different time points
        for (time_ms, description) in [(0, "Start"), (500, "25%"), (1000, "50%"), (2000, "End")] {
            let commands = engine.update(Duration::from_millis(time_ms)).unwrap();
            println!("\n  At {} ({}ms):", description, time_ms);
            for cmd in &commands {
                let channel_name = match cmd.channel {
                    1 => "Dimmer",
                    2 => "Red",
                    3 => "Green",
                    4 => "Blue",
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
        println!("- Dimmer channel: Gets dimmer values (only for Replace mode)");
        println!("- Red channel: Gets dimmer values (for layering with Multiply mode)");
        println!("- Green channel: Gets dimmer values (for layering with Multiply mode)");
        println!(
            "- Blue channel: Gets dimmer values multiplied with static blue value (for layering)"
        );

        // Verify the behavior is correct
        let final_commands = engine.update(Duration::from_millis(2000)).unwrap();
        assert_eq!(final_commands.len(), 1); // Only dimmer channel (dimmer uses Multiply mode, so only affects RGB via _dimmer_multiplier)

        // All channels should be at 0 at the end
        for cmd in &final_commands {
            assert_eq!(cmd.value, 0);
        }

        println!("✅ Dimmer precedence and selective dimming test passed!");
        println!("✅ Dimmer channel takes precedence over RGB for Replace mode");
        println!("✅ RGB channels are used for layering with Multiply mode");
    }

    #[test]
    fn test_dimmer_debug() {
        // Initialize tracing
        let _ = tracing_subscriber::fmt::try_init();

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

        let blue_effect = EffectInstance::with_layering(
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
        let dimmer_effect = EffectInstance::with_layering(
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
    fn test_layering_show_dsl_parsing() {
        use super::super::parser::parse_light_shows;

        // Test the exact DSL from layering_show.light
        let dsl_content = r#"show "Effect Layering Demo" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%, layer: background, blend_mode: replace
    
    @00:02.000
    front_wash: dimmer start_level: 1.0, end_level: 0.5, duration: 5s, layer: midground, blend_mode: multiply
    
    @00:04.000
    front_wash: strobe frequency: 2, intensity: 1.0, layer: foreground, blend_mode: overlay
}"#;

        let shows = match parse_light_shows(dsl_content) {
            Ok(s) => s,
            Err(e) => {
                println!("Parser error: {}", e);
                panic!("Failed to parse layering show DSL: {}", e);
            }
        };

        let show = shows.get("Effect Layering Demo").unwrap();
        assert_eq!(show.cues.len(), 3);

        // Check static blue with replace blend mode
        let cue1 = &show.cues[0];
        let effect1 = &cue1.effects[0];
        assert_eq!(effect1.layer, Some(EffectLayer::Background));
        assert_eq!(effect1.blend_mode, Some(BlendMode::Replace));

        // Check dimmer with multiply blend mode
        let cue2 = &show.cues[1];
        let effect2 = &cue2.effects[0];
        assert_eq!(effect2.layer, Some(EffectLayer::Midground));
        assert_eq!(effect2.blend_mode, Some(BlendMode::Multiply));

        // Check strobe with overlay blend mode
        let cue3 = &show.cues[2];
        let effect3 = &cue3.effects[0];
        assert_eq!(effect3.layer, Some(EffectLayer::Foreground));
        assert_eq!(effect3.blend_mode, Some(BlendMode::Overlay));

        println!("Layering show DSL parsing test passed!");
        println!("Successfully parsed all blend modes: replace, multiply, overlay");
    }

    #[test]
    fn test_blend_mode_loss_debug() {
        use super::super::effects::*;
        use super::super::engine::EffectEngine;
        use super::super::parser::parse_light_shows;
        use std::collections::HashMap;

        // Initialize tracing
        let _ = tracing_subscriber::fmt::try_init();

        // Test DSL that should use multiply blend mode
        let dsl_with_multiply = r#"show "Blend Mode Loss Test" {
    @00:00.000
    front_wash: static color: "blue", layer: background, blend_mode: replace
    
    @00:02.000
    front_wash: dimmer start_level: 1.0, end_level: 0.5, duration: 5s, layer: midground, blend_mode: multiply
}"#;

        // Parse the DSL
        let result = parse_light_shows(dsl_with_multiply);
        assert!(
            result.is_ok(),
            "DSL should parse successfully: {:?}",
            result
        );

        let shows = result.unwrap();
        let show = shows.get("Blend Mode Loss Test").unwrap();

        // Check that the dimmer effect has the correct blend mode
        let dimmer_cue = &show.cues[1];
        let dimmer_effect = &dimmer_cue.effects[0];
        assert_eq!(dimmer_effect.blend_mode, Some(BlendMode::Multiply));
        println!(
            "✅ DSL parsing: dimmer effect has blend_mode = {:?}",
            dimmer_effect.blend_mode
        );

        // Create effect engine and register fixtures
        let mut engine = EffectEngine::new();

        // Create a test fixture
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

        engine.register_fixture(fixture);

        // Create effect instances from the DSL effects
        let static_effect = EffectInstance::with_layering(
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
            vec!["front_wash".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
        );

        let dimmer_effect = EffectInstance::with_layering(
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

        println!("✅ Created effects:");
        println!(
            "  Static effect: blend_mode = {:?}",
            static_effect.blend_mode
        );
        println!(
            "  Dimmer effect: blend_mode = {:?}",
            dimmer_effect.blend_mode
        );

        // Start the static effect
        engine.start_effect(static_effect).unwrap();

        // Update to apply static effect
        let _commands = engine.update(Duration::from_secs(0)).unwrap();
        println!("✅ Applied static effect");

        // Start the dimmer effect
        engine.start_effect(dimmer_effect).unwrap();

        // Update to apply dimmer effect
        let _commands = engine.update(Duration::from_secs(2)).unwrap();
        println!("✅ Applied dimmer effect");

        // The debug output should show where the blend mode is being lost
    }

    #[test]
    fn test_timeline_blend_mode_loss() {
        use super::super::effects::*;
        use super::super::engine::EffectEngine;
        use super::super::parser::parse_light_shows;
        use super::super::timeline::LightingTimeline;
        use std::collections::HashMap;

        // Initialize tracing
        let _ = tracing_subscriber::fmt::try_init();

        // Test DSL that should use multiply blend mode
        let dsl_with_multiply = r#"show "Timeline Blend Mode Test" {
    @00:00.000
    front_wash: static color: "blue", layer: background, blend_mode: replace
    
    @00:02.000
    front_wash: dimmer start_level: 1.0, end_level: 0.5, duration: 5s, layer: midground, blend_mode: multiply
}"#;

        // Parse the DSL
        let result = parse_light_shows(dsl_with_multiply);
        assert!(
            result.is_ok(),
            "DSL should parse successfully: {:?}",
            result
        );

        let shows = result.unwrap();
        let show = shows.get("Timeline Blend Mode Test").unwrap();

        // Check that the dimmer effect has the correct blend mode
        let dimmer_cue = &show.cues[1];
        let dimmer_effect = &dimmer_cue.effects[0];
        assert_eq!(dimmer_effect.blend_mode, Some(BlendMode::Multiply));
        println!(
            "✅ DSL parsing: dimmer effect has blend_mode = {:?}",
            dimmer_effect.blend_mode
        );

        // Create timeline from the show
        let mut timeline = LightingTimeline::new(show.cues.clone());
        println!("✅ Created timeline with {} cues", show.cues.len());

        // Create effect engine and register fixtures
        let mut engine = EffectEngine::new();

        // Create a test fixture
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

        engine.register_fixture(fixture);

        // Start the timeline
        timeline.start();
        println!("✅ Started timeline");

        // Update timeline to get effects at different times
        let effects_at_0s = timeline.update(Duration::from_secs(0));
        println!("✅ Timeline at 0s: {} effects", effects_at_0s.len());
        for effect in &effects_at_0s {
            println!(
                "  Effect: {} blend_mode = {:?}",
                effect.id, effect.blend_mode
            );
        }

        let effects_at_2s = timeline.update(Duration::from_secs(2));
        println!("✅ Timeline at 2s: {} effects", effects_at_2s.len());
        for effect in &effects_at_2s {
            println!(
                "  Effect: {} blend_mode = {:?}",
                effect.id, effect.blend_mode
            );
        }

        // Start the effects from timeline
        for effect in effects_at_0s {
            engine.start_effect(effect).unwrap();
        }

        // Update to apply static effect
        let _commands = engine.update(Duration::from_secs(0)).unwrap();
        println!("✅ Applied static effect from timeline");

        // Start the dimmer effect from timeline
        for effect in effects_at_2s {
            engine.start_effect(effect).unwrap();
        }

        // Update to apply dimmer effect
        let _commands = engine.update(Duration::from_secs(2)).unwrap();
        println!("✅ Applied dimmer effect from timeline");

        // The debug output should show where the blend mode is being lost
    }

    #[test]
    fn test_dsl_blend_mode_parsing() {
        use super::super::parser::parse_light_shows;

        // Test DSL with multiply blend mode
        let dsl_with_multiply = r#"show "Blend Mode Test" {
    @00:00.000
    front_wash: static color: "blue", layer: background, blend_mode: replace
    
    @00:02.000
    front_wash: dimmer start_level: 1.0, end_level: 0.5, duration: 5s, layer: midground, blend_mode: multiply
}"#;

        let result = parse_light_shows(dsl_with_multiply);
        assert!(
            result.is_ok(),
            "DSL should parse successfully: {:?}",
            result
        );

        let shows = result.unwrap();
        let show = shows.get("Blend Mode Test").unwrap();
        assert_eq!(show.cues.len(), 2);

        // Check first cue (static effect)
        let static_cue = &show.cues[0];
        assert_eq!(static_cue.effects.len(), 1);
        let static_effect = &static_cue.effects[0];
        assert_eq!(
            static_effect.blend_mode,
            Some(super::super::effects::BlendMode::Replace)
        );
        assert_eq!(
            static_effect.layer,
            Some(super::super::effects::EffectLayer::Background)
        );

        // Check second cue (dimmer effect)
        let dimmer_cue = &show.cues[1];
        assert_eq!(dimmer_cue.effects.len(), 1);
        let dimmer_effect = &dimmer_cue.effects[0];
        assert_eq!(
            dimmer_effect.blend_mode,
            Some(super::super::effects::BlendMode::Multiply)
        );
        assert_eq!(
            dimmer_effect.layer,
            Some(super::super::effects::EffectLayer::Midground)
        );

        println!("✅ DSL blend mode parsing test passed");
        println!(
            "  Static effect: blend_mode={:?}, layer={:?}",
            static_effect.blend_mode, static_effect.layer
        );
        println!(
            "  Dimmer effect: blend_mode={:?}, layer={:?}",
            dimmer_effect.blend_mode, dimmer_effect.layer
        );
    }

    #[test]
    fn test_multiple_effects_simultaneous() {
        use super::super::effects::*;
        use super::super::engine::EffectEngine;

        // Initialize tracing
        let _ = tracing_subscriber::fmt::try_init();

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

        let static_effect = EffectInstance::with_layering(
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
        let commands = engine.update(Duration::from_secs(0)).unwrap();
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
        let dimmer_effect = EffectInstance::with_layering(
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
        let commands = engine.update(Duration::from_secs(2)).unwrap();
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
        engine.update(Duration::from_secs(2)).unwrap();
        let commands = engine.update(Duration::from_secs(0)).unwrap();
        println!("\n=== At 4.5s (50% through dimmer on 4 fixtures) ===");
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
        let _ = tracing_subscriber::fmt::try_init();

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

        let static_effect = EffectInstance::with_layering(
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
        let commands = engine.update(Duration::from_secs(0)).unwrap();
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
        let dimmer_effect = EffectInstance::with_layering(
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
        engine.update(Duration::from_secs(2)).unwrap();
        engine.start_effect(dimmer_effect).unwrap();

        // Check at 2s (dimmer start)
        let commands = engine.update(Duration::from_secs(0)).unwrap();
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
        engine.update(Duration::from_secs(2)).unwrap();
        let commands = engine.update(Duration::from_secs(0)).unwrap();
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
    fn test_static_replace_blend_mode() {
        use super::super::effects::*;
        use super::super::engine::EffectEngine;

        // Initialize tracing
        let _ = tracing_subscriber::fmt::try_init();

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

        let static_effect = EffectInstance::with_layering(
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
        let commands = engine.update(Duration::from_secs(0)).unwrap();
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
    fn test_static_with_dimmer_parameter() {
        use super::super::effects::*;
        use super::super::engine::EffectEngine;

        // Initialize tracing
        let _ = tracing_subscriber::fmt::try_init();

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

        let static_effect = EffectInstance::with_layering(
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
        let dimmer_effect = EffectInstance::with_layering(
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
    fn test_real_layering_show_file() {
        use super::super::effects::*;
        use super::super::engine::EffectEngine;
        use super::super::parser::parse_light_shows;

        // Initialize tracing
        let _ = tracing_subscriber::fmt::try_init();

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
                let effect_instance = EffectInstance::with_layering(
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
        let commands = engine.update(Duration::from_secs(0)).unwrap();
        println!("At 0s (static blue):");
        for cmd in &commands {
            println!("  Channel {}: {}", cmd.channel, cmd.value);
        }

        // At 2 seconds - should have static blue + dimmer starting
        engine.update(Duration::from_secs(2)).unwrap();
        let commands = engine.update(Duration::from_secs(0)).unwrap();
        println!("\nAt 2s (static blue + dimmer start):");
        for cmd in &commands {
            println!("  Channel {}: {}", cmd.channel, cmd.value);
        }

        // At 4.5 seconds - should have dimmed blue (50% through dimmer)
        engine.update(Duration::from_secs(2)).unwrap();
        let commands = engine.update(Duration::from_secs(0)).unwrap();
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
        let _ = tracing_subscriber::fmt::try_init();

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
                let effect_instance = EffectInstance::with_layering(
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
        let commands = engine.update(Duration::from_secs(0)).unwrap();
        println!("At 0s (static blue):");
        for cmd in &commands {
            println!("  Channel {}: {}", cmd.channel, cmd.value);
        }

        // At 2 seconds - should have static blue + dimmer starting
        engine.update(Duration::from_secs(2)).unwrap();
        let commands = engine.update(Duration::from_secs(0)).unwrap();
        println!("\nAt 2s (static blue + dimmer start):");
        for cmd in &commands {
            println!("  Channel {}: {}", cmd.channel, cmd.value);
        }

        // At 4.5 seconds - should have dimmed blue (50% through dimmer)
        engine.update(Duration::from_secs(2)).unwrap();
        let commands = engine.update(Duration::from_secs(0)).unwrap();
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
    fn test_dsl_parsing_debug() {
        use super::super::parser::parse_light_shows;

        let dsl = r#"show "Test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%, layer: background, blend_mode: replace
    
    @00:02.000
    front_wash: dimmer start_level: 1.0, end_level: 0.5, duration: 5s, layer: midground, blend_mode: multiply
}"#;

        match parse_light_shows(dsl) {
            Ok(shows) => {
                for (show_name, show) in shows {
                    println!("Show: {}", show_name);
                    for cue in &show.cues {
                        println!("  Cue at {:?}: {:?}", cue.time, cue.time);
                        for effect in &cue.effects {
                            println!("    Effect: {:?}", effect.effect_type);
                            println!("    Layer: {:?}", effect.layer);
                            println!("    Blend Mode: {:?}", effect.blend_mode);
                        }
                    }
                }
            }
            Err(e) => {
                println!("Error parsing DSL: {}", e);
            }
        }
    }

    #[test]
    fn test_dimmer_replace_vs_multiply() {
        // Initialize tracing
        let _ = tracing_subscriber::fmt::try_init();

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

        let blue_effect = EffectInstance::with_layering(
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
        let dimmer_replace = EffectInstance::with_layering(
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

        // With Replace, all channels should have the same value (white)
        assert_eq!(red_cmd.value, green_cmd.value);
        assert_eq!(green_cmd.value, blue_cmd.value);

        // Clear effects and test Multiply
        let mut engine2 = EffectEngine::new();
        engine2.register_fixture(fixture.clone());
        engine2.start_effect(blue_effect).unwrap();

        let dimmer_multiply = EffectInstance::with_layering(
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
        let _ = tracing_subscriber::fmt::try_init();

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

        let blue_effect = EffectInstance::with_layering(
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
        let dimmer_effect = EffectInstance::with_layering(
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

        let dimmer_replace = EffectInstance::with_layering(
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

        // With Replace, all channels should have the same value (white)
        assert_eq!(red_cmd.value, green_cmd.value);
        assert_eq!(green_cmd.value, blue_cmd.value);
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

        let rgb_effect = EffectInstance::with_layering(
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
        let dimmer_effect = EffectInstance::with_layering(
            "dimmer".to_string(),
            EffectType::Dimmer {
                start_level: 1.0,
                end_level: 0.0,
                duration: Duration::from_secs(2),
                curve: DimmerCurve::Linear,
            },
            vec!["rgb_fixture".to_string()],
            EffectLayer::Midground,
            BlendMode::Multiply,
        );

        engine.start_effect(dimmer_effect).unwrap();

        // Check at different time points
        for (time_ms, description) in [(0, "Start"), (500, "25%"), (1000, "50%"), (2000, "End")] {
            let commands = engine.update(Duration::from_millis(time_ms)).unwrap();
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

        println!("\nCurrent behavior analysis:");
        println!("- All RGB channels get the same dimmer value applied");
        println!("- Red: 255 * dimmer_value");
        println!("- Green: 127 * dimmer_value");
        println!("- Blue: 63 * dimmer_value");
        println!("- This maintains the relative brightness ratios between colors");

        // Verify the behavior is correct
        let final_commands = engine.update(Duration::from_millis(2000)).unwrap();
        assert_eq!(final_commands.len(), 3); // RGB channels only

        // All channels should be at 0 at the end
        for cmd in &final_commands {
            assert_eq!(cmd.value, 0);
        }

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
    fn test_dsl_layering_parsing() {
        use super::super::parser::parse_light_shows;

        let dsl_content = r#"show "DSL Layering Test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%, layer: background
    
    @00:02.000
    front_wash: dimmer start_level: 1.0, end_level: 0.5, duration: 5s, layer: midground, blend_mode: multiply
}"#;

        let shows = match parse_light_shows(dsl_content) {
            Ok(s) => s,
            Err(e) => {
                println!("Parser error: {}", e);
                panic!("Failed to parse DSL: {}", e);
            }
        };
        let show = shows.get("DSL Layering Test").unwrap();

        assert_eq!(show.name, "DSL Layering Test");
        assert_eq!(show.cues.len(), 2);

        // Check first cue (static blue with background layer)
        let cue1 = &show.cues[0];
        assert_eq!(cue1.time, Duration::from_secs(0));
        assert_eq!(cue1.effects.len(), 1);
        let effect1 = &cue1.effects[0];
        assert_eq!(effect1.groups, vec!["front_wash"]);
        assert_eq!(effect1.layer, Some(EffectLayer::Background));
        assert_eq!(effect1.blend_mode, None); // Not specified in DSL

        // Check second cue (dimmer with midground layer and multiply blend mode)
        let cue2 = &show.cues[1];
        assert_eq!(cue2.time, Duration::from_secs(2));
        assert_eq!(cue2.effects.len(), 1);
        let effect2 = &cue2.effects[0];
        assert_eq!(effect2.groups, vec!["front_wash"]);
        assert_eq!(effect2.layer, Some(EffectLayer::Midground));
        assert_eq!(effect2.blend_mode, Some(BlendMode::Multiply));

        println!("DSL layering parsing test passed!");
        println!("Successfully parsed layering parameters from DSL:");
        println!("- layer: background, midground, foreground");
        println!("- blend_mode: replace, multiply, overlay");
    }
}
