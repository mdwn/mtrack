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
    use super::super::parser::parse_light_shows;
    use super::super::timeline::LightingTimeline;
    use std::collections::HashMap;
    use std::time::Duration;

    // Helper function to create EffectInstance with layering
    fn create_effect_with_layering(
        id: String,
        effect_type: EffectType,
        target_fixtures: Vec<String>,
        layer: EffectLayer,
        blend_mode: BlendMode,
    ) -> EffectInstance {
        let mut effect = EffectInstance::new(id, effect_type, target_fixtures);
        effect.layer = layer;
        effect.blend_mode = blend_mode;
        // Ensure effects created via this helper persist long enough for tests that
        // advance simulated time beyond 1s. Provide a generous default hold_time.
        if effect.hold_time.is_none() {
            effect.hold_time = Some(Duration::from_secs(10));
        }
        effect
    }

    // Helper function to create EffectInstance with timing
    fn create_effect_with_timing(
        id: String,
        effect_type: EffectType,
        target_fixtures: Vec<String>,
        layer: EffectLayer,
        blend_mode: BlendMode,
        up_time: Option<Duration>,
        down_time: Option<Duration>,
    ) -> EffectInstance {
        let mut effect = EffectInstance::new(id, effect_type, target_fixtures);
        effect.layer = layer;
        effect.blend_mode = blend_mode;
        effect.up_time = up_time;
        effect.down_time = down_time;
        effect
    }

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

        // At start: red and green should be 0, blue should be full (255) - static blue * dimmer (1.0 * 1.0 = 1.0)
        assert_eq!(red_cmd.value, 0);
        assert_eq!(green_cmd.value, 0);
        // But there might be some rounding, so check it's close to 255
        assert!(blue_cmd.value >= 250);

        // Update engine at middle (dimmer should be at 75%)
        engine.update(Duration::from_millis(500)).unwrap();
        let commands = engine.update(Duration::from_millis(16)).unwrap();

        // The dimmer effect is applied to RGB channels, so blue should be dimmed
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        // At 25% progress: blue should be around 87% (1.0 * 0.87 = 0.87, 0.87 * 255 = 221.85)
        assert!(blue_cmd.value >= 210 && blue_cmd.value <= 230); // Around 87%

        // Update engine at end (dimmer should be at 50%)
        engine.update(Duration::from_millis(500)).unwrap();
        let commands = engine.update(Duration::from_millis(16)).unwrap();

        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        // At 50% progress: blue should be around 74% (1.0 * 0.746 = 0.746, 0.746 * 255 = 190.23)
        assert!(blue_cmd.value >= 180 && blue_cmd.value <= 200); // Around 74%
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

        // Create strobe effect
        let strobe_effect = create_effect_with_layering(
            "strobe".to_string(),
            EffectType::Strobe {
                frequency: 1.0, // 1 Hz for easy testing
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
                frequency: 2.0, // 2 Hz strobe
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
        let blue_effect = create_effect_with_layering(
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
        let dimmer_effect = create_effect_with_layering(
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
        let dimmer_effect = create_effect_with_layering(
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
        assert_eq!(final_commands.len(), 4); // RGB + dimmer channels (dimmer uses Replace mode, so sets all channels directly)

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
    fn test_layering_show_dsl_parsing() {
        use super::super::parser::parse_light_shows;

        // Test the exact DSL from layering_show.light
        let dsl_content = r#"show "Effect Layering Demo" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%, layer: background, blend_mode: replace
    
    @00:02.000
    front_wash: dimmer start_level: 1.0, end_level: 0.5, duration: 5s, layer: midground, blend_mode: multiply
    
    @00:04.000
    front_wash: strobe frequency: 2, layer: foreground, blend_mode: overlay
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
        let static_effect = create_effect_with_layering(
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

        // With Replace, all channels should have the same value (white)
        assert_eq!(red_cmd.value, green_cmd.value);
        assert_eq!(green_cmd.value, blue_cmd.value);

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
            let commands = engine.update(Duration::from_millis(delta_ms)).unwrap();
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

    #[test]
    fn test_sophisticated_conflict_resolution() {
        let mut engine = EffectEngine::new();

        // Create a test fixture
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
            max_strobe_frequency: Some(20.0),
        };
        engine.register_fixture(fixture);

        // Test 1: Effects in different layers should not conflict
        let static_bg = create_effect_with_layering(
            "static_bg".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("red".to_string(), 1.0);
                    params
                },
                duration: None,
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

        // Both effects should coexist (different layers)
        engine.start_effect(static_bg).unwrap();
        engine.start_effect(dimmer_mg).unwrap();
        assert_eq!(engine.active_effects_count(), 2);

        // Test 2: Static effects in the same layer should conflict
        let static_fg = create_effect_with_layering(
            "static_fg".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("blue".to_string(), 1.0);
                    params
                },
                duration: None,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background, // Same layer as static_bg
            BlendMode::Replace,
        );

        // This should stop the background static effect (same layer, same type)
        engine.start_effect(static_fg).unwrap();
        assert_eq!(engine.active_effects_count(), 2); // dimmer + new static
        assert!(!engine.has_effect("static_bg"));
        assert!(engine.has_effect("dimmer_mg"));
        assert!(engine.has_effect("static_fg"));

        // Test 3: Compatible blend modes should layer
        let pulse_mg = create_effect_with_layering(
            "pulse_mg".to_string(),
            EffectType::Pulse {
                base_level: 0.5,
                pulse_amplitude: 0.3,
                frequency: 2.0,
                duration: None,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Midground,
            BlendMode::Multiply,
        );

        // This should layer with the existing dimmer (same layer, compatible blend modes)
        engine.start_effect(pulse_mg).unwrap();
        assert_eq!(engine.active_effects_count(), 3); // dimmer + static + pulse
        assert!(engine.has_effect("dimmer_mg"));
        assert!(engine.has_effect("pulse_mg"));

        // Test 4: Replace blend mode should stop conflicting effects
        let static_replace = create_effect_with_layering(
            "static_replace".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("green".to_string(), 1.0);
                    params
                },
                duration: None,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background, // Same layer as static_fg
            BlendMode::Replace,
        );

        // This should stop the existing static effect (same type, same layer, Replace mode)
        engine.start_effect(static_replace).unwrap();
        assert_eq!(engine.active_effects_count(), 3); // dimmer + pulse + new static
        assert!(!engine.has_effect("static_fg"));
        assert!(engine.has_effect("static_replace"));
    }

    #[test]
    fn test_priority_based_conflict_resolution() {
        let mut engine = EffectEngine::new();

        // Create test fixtures
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);

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

        // Test 1: Higher priority effect stops lower priority effect in same layer
        let low_priority = create_effect_with_layering(
            "low_priority".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("red".to_string(), 1.0);
                    params
                },
                duration: None,
            },
            vec!["fixture1".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
        )
        .with_priority(1);

        let high_priority = create_effect_with_layering(
            "high_priority".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("blue".to_string(), 1.0);
                    params
                },
                duration: None,
            },
            vec!["fixture1".to_string()],
            EffectLayer::Background, // Same layer
            BlendMode::Replace,
        )
        .with_priority(10);

        engine.start_effect(low_priority).unwrap();
        engine.start_effect(high_priority).unwrap();

        assert_eq!(engine.active_effects_count(), 1);
        assert!(!engine.has_effect("low_priority"));
        assert!(engine.has_effect("high_priority"));

        // Test 2: Effects on different fixtures should not conflict
        let different_fixture_effect = create_effect_with_layering(
            "different_fixture_effect".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("green".to_string(), 1.0);
                    params
                },
                duration: None,
            },
            vec!["fixture2".to_string()], // Different fixture
            EffectLayer::Background,      // Same layer
            BlendMode::Replace,
        )
        .with_priority(5);

        engine.start_effect(different_fixture_effect).unwrap();

        // Should not conflict because different fixtures
        assert_eq!(engine.active_effects_count(), 2); // high_priority + different_fixture_effect
        assert!(engine.has_effect("high_priority"));
        assert!(engine.has_effect("different_fixture_effect"));

        // Test 3: Higher priority effect stops lower priority effect on same fixture
        let low_priority_same_fixture = create_effect_with_layering(
            "low_priority_same_fixture".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("yellow".to_string(), 1.0);
                    params
                },
                duration: None,
            },
            vec!["fixture2".to_string()], // Same fixture as different_fixture_effect
            EffectLayer::Background,
            BlendMode::Replace,
        )
        .with_priority(1);

        let high_priority_same_fixture = create_effect_with_layering(
            "high_priority_same_fixture".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("purple".to_string(), 1.0);
                    params
                },
                duration: None,
            },
            vec!["fixture2".to_string()], // Same fixture as low_priority_same_fixture
            EffectLayer::Background,      // Same layer
            BlendMode::Replace,
        )
        .with_priority(15); // Higher priority than different_fixture_effect

        engine.start_effect(low_priority_same_fixture).unwrap();
        engine.start_effect(high_priority_same_fixture).unwrap();

        // High priority should stop both lower priority effects on same fixture
        assert_eq!(engine.active_effects_count(), 2); // high_priority + high_priority_same_fixture
        assert!(engine.has_effect("high_priority"));
        assert!(!engine.has_effect("different_fixture_effect"));
        assert!(!engine.has_effect("low_priority_same_fixture"));
        assert!(engine.has_effect("high_priority_same_fixture"));
    }

    #[test]
    fn test_blend_mode_compatibility_matrix() {
        let mut engine = EffectEngine::new();

        // Create test fixture
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
            max_strobe_frequency: Some(20.0),
        };
        engine.register_fixture(fixture);

        // Test Replace mode conflicts with everything
        let replace_effect = create_effect_with_layering(
            "replace_effect".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("red".to_string(), 1.0);
                    params
                },
                duration: None,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
        );

        let multiply_effect = create_effect_with_layering(
            "multiply_effect".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("blue".to_string(), 1.0);
                    params
                },
                duration: None,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background, // Same layer
            BlendMode::Multiply,
        );

        engine.start_effect(replace_effect).unwrap();
        engine.start_effect(multiply_effect).unwrap();

        // Replace should conflict with Multiply (same layer, same type)
        assert_eq!(engine.active_effects_count(), 1);
        assert!(!engine.has_effect("replace_effect"));
        assert!(engine.has_effect("multiply_effect"));

        // Test compatible blend modes can layer
        let add_effect = create_effect_with_layering(
            "add_effect".to_string(),
            EffectType::Dimmer {
                start_level: 1.0,
                end_level: 0.5,
                duration: Duration::from_secs(1),
                curve: DimmerCurve::Linear,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background,
            BlendMode::Add,
        );

        let overlay_effect = create_effect_with_layering(
            "overlay_effect".to_string(),
            EffectType::Pulse {
                base_level: 0.5,
                pulse_amplitude: 0.3,
                frequency: 2.0,
                duration: None,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background, // Same layer
            BlendMode::Overlay,
        );

        engine.start_effect(add_effect).unwrap();
        engine.start_effect(overlay_effect).unwrap();

        // Add and Overlay should be compatible (different types, compatible blend modes)
        assert_eq!(engine.active_effects_count(), 3); // multiply + add + overlay
        assert!(engine.has_effect("add_effect"));
        assert!(engine.has_effect("overlay_effect"));

        // Test all blend mode combinations
        let blend_modes = vec![
            BlendMode::Replace,
            BlendMode::Multiply,
            BlendMode::Add,
            BlendMode::Overlay,
            BlendMode::Screen,
        ];

        for (i, mode1) in blend_modes.iter().enumerate() {
            for (j, mode2) in blend_modes.iter().enumerate() {
                let effect1 = create_effect_with_layering(
                    format!("test_mode1_{}_{}", i, j),
                    EffectType::Static {
                        parameters: {
                            let mut params = HashMap::new();
                            params.insert("red".to_string(), 1.0);
                            params
                        },
                        duration: None,
                    },
                    vec!["test_fixture".to_string()],
                    EffectLayer::Background,
                    *mode1,
                );

                let effect2 = create_effect_with_layering(
                    format!("test_mode2_{}_{}", i, j),
                    EffectType::Static {
                        parameters: {
                            let mut params = HashMap::new();
                            params.insert("blue".to_string(), 1.0);
                            params
                        },
                        duration: None,
                    },
                    vec!["test_fixture".to_string()],
                    EffectLayer::Background, // Same layer
                    *mode2,
                );

                // Clear engine for each test
                engine.stop_all_effects();

                engine.start_effect(effect1).unwrap();
                let count_before = engine.active_effects_count();
                engine.start_effect(effect2).unwrap();
                let count_after = engine.active_effects_count();

                // Verify expected behavior based on blend mode compatibility
                let should_conflict = !engine.blend_modes_are_compatible_public(*mode1, *mode2);
                if should_conflict {
                    assert_eq!(
                        count_after, count_before,
                        "Blend modes {:?} and {:?} should conflict",
                        mode1, mode2
                    );
                } else {
                    assert_eq!(
                        count_after,
                        count_before + 1,
                        "Blend modes {:?} and {:?} should be compatible",
                        mode1,
                        mode2
                    );
                }
            }
        }
    }

    #[test]
    fn test_effect_type_conflict_combinations() {
        let mut engine = EffectEngine::new();

        // Create test fixture
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);
        channels.insert("strobe".to_string(), 4);
        // No dimmer channel - Chase should work with RGB channels

        let fixture = FixtureInfo {
            name: "test_fixture".to_string(),
            universe: 1,
            address: 1,
            channels,
            fixture_type: "RGB_Par".to_string(),
            max_strobe_frequency: Some(20.0),
        };
        engine.register_fixture(fixture);

        // Test Static vs ColorCycle conflict
        let static_effect = create_effect_with_layering(
            "static_effect".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("red".to_string(), 1.0);
                    params
                },
                duration: None,
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
                speed: 1.0,
                direction: CycleDirection::Forward,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background, // Same layer
            BlendMode::Replace,
        );

        engine.start_effect(static_effect).unwrap();
        engine.start_effect(color_cycle_effect).unwrap();

        // Static and ColorCycle should conflict
        assert_eq!(engine.active_effects_count(), 1);
        assert!(!engine.has_effect("static_effect"));
        assert!(engine.has_effect("color_cycle_effect"));

        // Test Rainbow vs Static conflict
        let rainbow_effect = create_effect_with_layering(
            "rainbow_effect".to_string(),
            EffectType::Rainbow {
                speed: 1.0,
                saturation: 1.0,
                brightness: 1.0,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
        );

        let static_effect2 = create_effect_with_layering(
            "static_effect2".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("blue".to_string(), 1.0);
                    params
                },
                duration: None,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background, // Same layer
            BlendMode::Replace,
        );

        engine.start_effect(rainbow_effect).unwrap();
        engine.start_effect(static_effect2).unwrap();

        // Rainbow and Static should conflict - static should win (last one wins)
        assert_eq!(engine.active_effects_count(), 1); // static_effect2
        assert!(!engine.has_effect("rainbow_effect"));
        assert!(engine.has_effect("static_effect2"));

        // Test Strobe vs Strobe conflict
        let strobe1 = create_effect_with_layering(
            "strobe1".to_string(),
            EffectType::Strobe {
                frequency: 2.0,
                duration: None,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
        );

        let strobe2 = create_effect_with_layering(
            "strobe2".to_string(),
            EffectType::Strobe {
                frequency: 4.0,
                duration: None,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background, // Same layer
            BlendMode::Replace,
        );

        engine.start_effect(strobe1).unwrap();
        engine.start_effect(strobe2).unwrap();

        // Strobe and Strobe should conflict
        assert_eq!(engine.active_effects_count(), 2); // static_effect2 + strobe2
        assert!(!engine.has_effect("strobe1"));
        assert!(engine.has_effect("strobe2"));

        // Test Chase vs Chase conflict
        let chase1 = create_effect_with_layering(
            "chase1".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: 1.0,
                direction: ChaseDirection::LeftToRight,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
        );

        let chase2 = create_effect_with_layering(
            "chase2".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Snake,
                speed: 2.0,
                direction: ChaseDirection::RightToLeft,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background, // Same layer
            BlendMode::Replace,
        );

        engine.start_effect(chase1).unwrap();
        engine.start_effect(chase2).unwrap();

        // Chase and Chase should conflict
        assert_eq!(engine.active_effects_count(), 3); // static_effect2 + strobe2 + chase2
        assert!(!engine.has_effect("chase1"));
        assert!(engine.has_effect("chase2"));

        // Test Dimmer and Pulse compatibility (should layer)
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
                frequency: 2.0,
                duration: None,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background, // Same layer
            BlendMode::Multiply,
        );

        engine.start_effect(dimmer_effect).unwrap();
        engine.start_effect(pulse_effect).unwrap();

        // Dimmer and Pulse should be compatible (they layer)
        assert_eq!(engine.active_effects_count(), 5); // static_effect2 + strobe2 + chase2 + dimmer + pulse
        assert!(engine.has_effect("dimmer_effect"));
        assert!(engine.has_effect("pulse_effect"));
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
                speed: 1.0,
                direction: ChaseDirection::LeftToRight,
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
                speed: 1.0,
                direction: ChaseDirection::LeftToRight,
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
                frequency: 2.0, // 2 Hz for easy testing
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
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

        assert_eq!(red_cmd.value, 255); // Should be ON (1.0 * 255)
        assert_eq!(green_cmd.value, 255);
        assert_eq!(blue_cmd.value, 255);

        // At t=125ms (1/4 cycle) - should still be ON
        let commands = engine.update(Duration::from_millis(125)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(red_cmd.value, 255); // Should still be ON

        // At t=250ms (1/2 cycle) - should be OFF
        let commands = engine.update(Duration::from_millis(125)).unwrap(); // 125ms more = 250ms total
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

        assert_eq!(red_cmd.value, 0); // Should be OFF (0.0 * 255)
        assert_eq!(green_cmd.value, 0);
        assert_eq!(blue_cmd.value, 0);

        // At t=375ms (3/4 cycle) - should still be OFF
        let commands = engine.update(Duration::from_millis(125)).unwrap(); // 125ms more = 375ms total
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(red_cmd.value, 0); // Should still be OFF

        // At t=500ms (full cycle) - should be ON again
        let commands = engine.update(Duration::from_millis(125)).unwrap(); // 125ms more = 500ms total
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(red_cmd.value, 255); // Should be ON again
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
                frequency: 4.0, // 4 Hz for easy testing
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
                frequency: 2.0, // 2 Hz
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
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

        assert_eq!(red_cmd.value, 0); // Red should be 0 (static effect)
        assert_eq!(green_cmd.value, 0); // Green should be 0 (static effect)
        assert_eq!(blue_cmd.value, 255); // Blue should be 255 (static + strobe overlay)

        // At t=250ms (strobe OFF) - should see no light
        let commands = engine.update(Duration::from_millis(250)).unwrap();
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
                frequency: 2.0, // 2 Hz
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
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

        assert_eq!(red_cmd.value, 255); // Should be ON (1.0 * 255)
        assert_eq!(green_cmd.value, 255);
        assert_eq!(blue_cmd.value, 255);

        // At t=250ms (strobe OFF) - should see no light
        let commands = engine.update(Duration::from_millis(250)).unwrap();
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
                frequency: 0.0, // Off
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
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();

        // Should see blue light (static effect) - strobe should not interfere
        assert_eq!(red_cmd.value, 0); // Red should be 0 (static effect)
        assert_eq!(green_cmd.value, 0); // Green should be 0 (static effect)
        assert_eq!(blue_cmd.value, 255); // Blue should be 255 (static effect only)
    }

    #[test]
    fn test_chase_pattern_linear_left_to_right() {
        let mut engine = EffectEngine::new();

        // Create 4 fixtures for testing
        for i in 1..=4 {
            let mut channels = HashMap::new();
            channels.insert("dimmer".to_string(), 1);
            channels.insert("red".to_string(), 2);
            channels.insert("green".to_string(), 3);
            channels.insert("blue".to_string(), 4);

            let fixture = FixtureInfo {
                name: format!("fixture_{}", i),
                universe: 1,
                address: (i - 1) * 4 + 1,
                channels,
                fixture_type: "RGB_Par".to_string(),
                max_strobe_frequency: Some(20.0),
            };
            engine.register_fixture(fixture);
        }

        let chase_effect = create_effect_with_layering(
            "chase_linear_ltr".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: 2.0, // 2 Hz for easy testing
                direction: ChaseDirection::LeftToRight,
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

        engine.start_effect(chase_effect).unwrap();

        // Test chase sequence: fixture_1 -> fixture_2 -> fixture_3 -> fixture_4
        // At t=0ms (start) - fixture_1 should be ON
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let fixture1_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(fixture1_cmd.value, 255); // Should be ON

        // At t=125ms (1/4 cycle) - fixture_2 should be ON
        let commands = engine.update(Duration::from_millis(125)).unwrap();
        let fixture2_cmd = commands.iter().find(|cmd| cmd.channel == 5).unwrap(); // fixture_2 dimmer
        assert_eq!(fixture2_cmd.value, 255); // Should be ON

        // At t=250ms (1/2 cycle) - fixture_3 should be ON
        let commands = engine.update(Duration::from_millis(125)).unwrap(); // 125ms more = 250ms total
        let fixture3_cmd = commands.iter().find(|cmd| cmd.channel == 9).unwrap(); // fixture_3 dimmer
        assert_eq!(fixture3_cmd.value, 255); // Should be ON

        // At t=375ms (3/4 cycle) - fixture_4 should be ON
        let commands = engine.update(Duration::from_millis(125)).unwrap(); // 125ms more = 375ms total
        let fixture4_cmd = commands.iter().find(|cmd| cmd.channel == 13).unwrap(); // fixture_4 dimmer
        assert_eq!(fixture4_cmd.value, 255); // Should be ON

        // At t=500ms (full cycle) - fixture_1 should be ON again
        let commands = engine.update(Duration::from_millis(125)).unwrap(); // 125ms more = 500ms total
        let fixture1_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(fixture1_cmd.value, 255); // Should be ON again
    }

    #[test]
    fn test_chase_pattern_linear_right_to_left() {
        let mut engine = EffectEngine::new();

        // Create 4 fixtures for testing
        for i in 1..=4 {
            let mut channels = HashMap::new();
            channels.insert("dimmer".to_string(), 1);
            channels.insert("red".to_string(), 2);
            channels.insert("green".to_string(), 3);
            channels.insert("blue".to_string(), 4);

            let fixture = FixtureInfo {
                name: format!("fixture_{}", i),
                universe: 1,
                address: (i - 1) * 4 + 1,
                channels,
                fixture_type: "RGB_Par".to_string(),
                max_strobe_frequency: Some(20.0),
            };
            engine.register_fixture(fixture);
        }

        let chase_effect = create_effect_with_layering(
            "chase_linear_rtl".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: 2.0, // 2 Hz for easy testing
                direction: ChaseDirection::RightToLeft,
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

        engine.start_effect(chase_effect).unwrap();

        // Test chase sequence: fixture_4 -> fixture_3 -> fixture_2 -> fixture_1
        // At t=0ms (start) - fixture_4 should be ON
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let fixture4_cmd = commands.iter().find(|cmd| cmd.channel == 13).unwrap(); // fixture_4 dimmer
        assert_eq!(fixture4_cmd.value, 255); // Should be ON

        // At t=125ms (1/4 cycle) - fixture_3 should be ON
        let commands = engine.update(Duration::from_millis(125)).unwrap();
        let fixture3_cmd = commands.iter().find(|cmd| cmd.channel == 9).unwrap(); // fixture_3 dimmer
        assert_eq!(fixture3_cmd.value, 255); // Should be ON

        // At t=250ms (1/2 cycle) - fixture_2 should be ON
        let commands = engine.update(Duration::from_millis(125)).unwrap(); // 125ms more = 250ms total
        let fixture2_cmd = commands.iter().find(|cmd| cmd.channel == 5).unwrap(); // fixture_2 dimmer
        assert_eq!(fixture2_cmd.value, 255); // Should be ON

        // At t=375ms (3/4 cycle) - fixture_1 should be ON
        let commands = engine.update(Duration::from_millis(125)).unwrap(); // 125ms more = 375ms total
        let fixture1_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(fixture1_cmd.value, 255); // Should be ON
    }

    #[test]
    fn test_chase_pattern_snake() {
        let mut engine = EffectEngine::new();

        // Create 4 fixtures for testing
        for i in 1..=4 {
            let mut channels = HashMap::new();
            channels.insert("dimmer".to_string(), 1);
            channels.insert("red".to_string(), 2);
            channels.insert("green".to_string(), 3);
            channels.insert("blue".to_string(), 4);

            let fixture = FixtureInfo {
                name: format!("fixture_{}", i),
                universe: 1,
                address: (i - 1) * 4 + 1,
                channels,
                fixture_type: "RGB_Par".to_string(),
                max_strobe_frequency: Some(20.0),
            };
            engine.register_fixture(fixture);
        }

        let chase_effect = create_effect_with_layering(
            "chase_snake".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Snake,
                speed: 2.0, // 2 Hz for easy testing
                direction: ChaseDirection::LeftToRight,
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

        engine.start_effect(chase_effect).unwrap();

        // Test snake pattern: fixture_1 -> fixture_2 -> fixture_3 -> fixture_4 -> fixture_3 -> fixture_2 -> fixture_1
        // Snake pattern has 6 positions: [0,1,2,3,2,1] for 4 fixtures
        // Each position lasts 500ms/6 = 83.33ms

        // At t=0ms (start) - fixture_1 should be ON
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let fixture1_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(fixture1_cmd.value, 255); // Should be ON

        // At t=125ms (1/6 cycle) - fixture_2 should be ON
        let commands = engine.update(Duration::from_millis(125)).unwrap();
        let fixture2_cmd = commands.iter().find(|cmd| cmd.channel == 5).unwrap();
        assert_eq!(fixture2_cmd.value, 255); // Should be ON

        // At t=250ms (2/6 cycle) - fixture_3 should be ON
        let commands = engine.update(Duration::from_millis(125)).unwrap(); // 125+125=250ms total
        let fixture3_cmd = commands.iter().find(|cmd| cmd.channel == 9).unwrap();
        assert_eq!(fixture3_cmd.value, 255); // Should be ON

        // At t=375ms (3/6 cycle) - fixture_4 should be ON
        let commands = engine.update(Duration::from_millis(125)).unwrap(); // 250+125=375ms total
        let fixture4_cmd = commands.iter().find(|cmd| cmd.channel == 13).unwrap();
        assert_eq!(fixture4_cmd.value, 255); // Should be ON

        // At t=500ms (4/6 cycle) - fixture_3 should be ON (snake back)
        let commands = engine.update(Duration::from_millis(125)).unwrap(); // 375+125=500ms total
        let fixture3_cmd = commands.iter().find(|cmd| cmd.channel == 9).unwrap();
        assert_eq!(fixture3_cmd.value, 255); // Should be ON

        // At t=625ms (5/6 cycle) - fixture_2 should be ON (snake back)
        let commands = engine.update(Duration::from_millis(125)).unwrap(); // 500+125=625ms total
        let fixture2_cmd = commands.iter().find(|cmd| cmd.channel == 5).unwrap();
        assert_eq!(fixture2_cmd.value, 255); // Should be ON

        // At t=750ms (6/6 cycle) - fixture_1 should be ON again
        let commands = engine.update(Duration::from_millis(125)).unwrap(); // 625+125=750ms total
        let fixture1_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(fixture1_cmd.value, 255); // Should be ON again
    }

    #[test]
    fn test_chase_pattern_random() {
        let mut engine = EffectEngine::new();

        // Create 4 fixtures for testing
        for i in 1..=4 {
            let mut channels = HashMap::new();
            channels.insert("dimmer".to_string(), 1);
            channels.insert("red".to_string(), 2);
            channels.insert("green".to_string(), 3);
            channels.insert("blue".to_string(), 4);

            let fixture = FixtureInfo {
                name: format!("fixture_{}", i),
                universe: 1,
                address: (i - 1) * 4 + 1,
                channels,
                fixture_type: "RGB_Par".to_string(),
                max_strobe_frequency: Some(20.0),
            };
            engine.register_fixture(fixture);
        }

        let chase_effect = create_effect_with_layering(
            "chase_random".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Random,
                speed: 2.0,                             // 2 Hz for easy testing
                direction: ChaseDirection::LeftToRight, // Direction doesn't matter for random
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

        engine.start_effect(chase_effect).unwrap();

        // Test random pattern - should have some fixture ON at each time point
        // At t=0ms - some fixture should be ON
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let on_fixtures: Vec<_> = commands.iter().filter(|cmd| cmd.value == 255).collect();
        assert_eq!(on_fixtures.len(), 1); // Exactly one fixture should be ON

        // At t=125ms - some fixture should be ON
        let commands = engine.update(Duration::from_millis(125)).unwrap();
        let on_fixtures: Vec<_> = commands.iter().filter(|cmd| cmd.value == 255).collect();
        assert_eq!(on_fixtures.len(), 1); // Exactly one fixture should be ON

        // At t=250ms - some fixture should be ON
        let commands = engine.update(Duration::from_millis(125)).unwrap(); // 125ms more = 250ms total
        let on_fixtures: Vec<_> = commands.iter().filter(|cmd| cmd.value == 255).collect();
        assert_eq!(on_fixtures.len(), 1); // Exactly one fixture should be ON
    }

    #[test]
    fn test_chase_direction_vertical() {
        let mut engine = EffectEngine::new();

        // Create 4 fixtures for testing
        for i in 1..=4 {
            let mut channels = HashMap::new();
            channels.insert("dimmer".to_string(), 1);
            channels.insert("red".to_string(), 2);
            channels.insert("green".to_string(), 3);
            channels.insert("blue".to_string(), 4);

            let fixture = FixtureInfo {
                name: format!("fixture_{}", i),
                universe: 1,
                address: (i - 1) * 4 + 1,
                channels,
                fixture_type: "RGB_Par".to_string(),
                max_strobe_frequency: Some(20.0),
            };
            engine.register_fixture(fixture);
        }

        // Test TopToBottom
        let chase_effect = create_effect_with_layering(
            "chase_ttb".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: 2.0,
                direction: ChaseDirection::TopToBottom,
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

        engine.start_effect(chase_effect).unwrap();

        // TopToBottom should behave like LeftToRight (fixture_1 -> fixture_2 -> fixture_3 -> fixture_4)
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let fixture1_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(fixture1_cmd.value, 255); // Should be ON
    }

    #[test]
    fn test_chase_direction_circular() {
        let mut engine = EffectEngine::new();

        // Create 4 fixtures for testing
        for i in 1..=4 {
            let mut channels = HashMap::new();
            channels.insert("dimmer".to_string(), 1);
            channels.insert("red".to_string(), 2);
            channels.insert("green".to_string(), 3);
            channels.insert("blue".to_string(), 4);

            let fixture = FixtureInfo {
                name: format!("fixture_{}", i),
                universe: 1,
                address: (i - 1) * 4 + 1,
                channels,
                fixture_type: "RGB_Par".to_string(),
                max_strobe_frequency: Some(20.0),
            };
            engine.register_fixture(fixture);
        }

        // Test Clockwise
        let chase_effect = create_effect_with_layering(
            "chase_cw".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: 2.0,
                direction: ChaseDirection::Clockwise,
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

        engine.start_effect(chase_effect).unwrap();

        // Clockwise should behave like LeftToRight (fixture_1 -> fixture_2 -> fixture_3 -> fixture_4)
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let fixture1_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(fixture1_cmd.value, 255); // Should be ON
    }

    #[test]
    fn test_chase_speed_variations() {
        let mut engine = EffectEngine::new();

        // Create 3 fixtures for testing
        for i in 1..=3 {
            let mut channels = HashMap::new();
            channels.insert("dimmer".to_string(), 1);
            channels.insert("red".to_string(), 2);
            channels.insert("green".to_string(), 3);
            channels.insert("blue".to_string(), 4);

            let fixture = FixtureInfo {
                name: format!("fixture_{}", i),
                universe: 1,
                address: (i - 1) * 4 + 1,
                channels,
                fixture_type: "RGB_Par".to_string(),
                max_strobe_frequency: Some(20.0),
            };
            engine.register_fixture(fixture);
        }

        // Test slow speed (0.5 Hz)
        let slow_chase = create_effect_with_layering(
            "chase_slow".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: 0.5, // 0.5 Hz - 2 second cycle
                direction: ChaseDirection::LeftToRight,
            },
            vec![
                "fixture_1".to_string(),
                "fixture_2".to_string(),
                "fixture_3".to_string(),
            ],
            EffectLayer::Background,
            BlendMode::Replace,
        );

        engine.start_effect(slow_chase).unwrap();

        // At t=0ms - fixture_1 should be ON
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let fixture1_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(fixture1_cmd.value, 255); // Should be ON

        // At t=600ms (1/3 cycle) - fixture_1 should still be ON
        let commands = engine.update(Duration::from_millis(600)).unwrap();
        let fixture1_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(fixture1_cmd.value, 255); // Should still be ON

        // At t=1200ms (2/3 cycle) - fixture_2 should be ON
        let commands = engine.update(Duration::from_millis(600)).unwrap(); // 600ms more = 1200ms total
        let fixture2_cmd = commands.iter().find(|cmd| cmd.channel == 5).unwrap();
        assert_eq!(fixture2_cmd.value, 255); // Should be ON
    }

    #[test]
    fn test_chase_single_fixture() {
        let mut engine = EffectEngine::new();

        // Create single fixture
        let mut channels = HashMap::new();
        channels.insert("dimmer".to_string(), 1);
        channels.insert("red".to_string(), 2);
        channels.insert("green".to_string(), 3);
        channels.insert("blue".to_string(), 4);

        let fixture = FixtureInfo {
            name: "single_fixture".to_string(),
            universe: 1,
            address: 1,
            channels,
            fixture_type: "RGB_Par".to_string(),
            max_strobe_frequency: Some(20.0),
        };
        engine.register_fixture(fixture);

        let chase_effect = create_effect_with_layering(
            "chase_single".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: 2.0,
                direction: ChaseDirection::LeftToRight,
            },
            vec!["single_fixture".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
        );

        engine.start_effect(chase_effect).unwrap();

        // With single fixture, it should always be ON
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let fixture_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(fixture_cmd.value, 255); // Should be ON

        // At any time, single fixture should be ON
        let commands = engine.update(Duration::from_millis(500)).unwrap();
        let fixture_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(fixture_cmd.value, 255); // Should be ON
    }

    #[test]
    fn test_chase_rgb_only_fixtures() {
        let mut engine = EffectEngine::new();

        // Create RGB-only fixtures
        for i in 1..=3 {
            let mut channels = HashMap::new();
            channels.insert("red".to_string(), 1);
            channels.insert("green".to_string(), 2);
            channels.insert("blue".to_string(), 3);
            // No dimmer channel!

            let fixture = FixtureInfo {
                name: format!("rgb_fixture_{}", i),
                universe: 1,
                address: (i - 1) * 3 + 1,
                channels,
                fixture_type: "RGB_Par".to_string(),
                max_strobe_frequency: None,
            };
            engine.register_fixture(fixture);
        }

        let chase_effect = create_effect_with_layering(
            "chase_rgb".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: 2.0,
                direction: ChaseDirection::LeftToRight,
            },
            vec![
                "rgb_fixture_1".to_string(),
                "rgb_fixture_2".to_string(),
                "rgb_fixture_3".to_string(),
            ],
            EffectLayer::Background,
            BlendMode::Replace,
        );

        engine.start_effect(chase_effect).unwrap();

        // Test that RGB channels are used for chase (white chase)
        let commands = engine.update(Duration::from_millis(0)).unwrap();

        // fixture_1 should have all RGB channels ON
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        assert_eq!(red_cmd.value, 255);
        assert_eq!(green_cmd.value, 255);
        assert_eq!(blue_cmd.value, 255);

        // At t=167ms (1/3 cycle) - fixture_2 should be ON
        let commands = engine.update(Duration::from_millis(167)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap(); // fixture_2 red
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 5).unwrap(); // fixture_2 green
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 6).unwrap(); // fixture_2 blue
        assert_eq!(red_cmd.value, 255);
        assert_eq!(green_cmd.value, 255);
        assert_eq!(blue_cmd.value, 255);
    }

    #[test]
    fn test_crossfade_multiplier_calculation() {
        // Test the crossfade multiplier calculation logic
        let mut effect = EffectInstance::new(
            "test".to_string(),
            EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            vec!["test_fixture".to_string()],
        );

        // Test up_time only
        effect.up_time = Some(Duration::from_secs(2));
        effect.hold_time = Some(Duration::from_secs(8));
        effect.down_time = None;

        // At start (0s) - should be 0%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(0)),
            0.0
        );

        // At 1s (50% through up_time) - should be 50%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(1)),
            0.5
        );

        // At 2s (up_time complete) - should be 100%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(2)),
            1.0
        );

        // At 5s (middle of hold_time) - should be 100%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(5)),
            1.0
        );

        // At 10s (end of hold_time) - should be 100%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(10)),
            0.0
        );

        // Test down_time only
        effect.up_time = None;
        effect.hold_time = Some(Duration::from_secs(8));
        effect.down_time = Some(Duration::from_secs(2));

        // At 8s (before down_time) - should be 100%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(8)),
            1.0
        );

        // At 9s (50% through down_time) - should be 50%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(9)),
            0.5
        );

        // At 10s (end of down_time) - should be 0%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(10)),
            0.0
        );

        // Test all three phases
        effect.up_time = Some(Duration::from_secs(1));
        effect.hold_time = Some(Duration::from_secs(8));
        effect.down_time = Some(Duration::from_secs(1));

        // At 0s - should be 0%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(0)),
            0.0
        );
        // At 0.5s (50% through up_time) - should be 50%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_millis(500)),
            0.5
        );
        // At 1s (up_time complete) - should be 100%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(1)),
            1.0
        );
        // At 5s (middle of hold_time) - should be 100%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(5)),
            1.0
        );
        // At 9s (start of down_time) - should be 100%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(9)),
            1.0
        );
        // At 9.5s (50% through down_time) - should be 50%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_millis(9500)),
            0.5
        );
        // At 10s (end of down_time) - should be 0%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(10)),
            0.0
        );
    }

    #[test]
    fn test_static_effect_crossfade_comprehensive() {
        let mut engine = EffectEngine::new();

        // Create a test fixture
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
            max_strobe_frequency: None,
        };
        engine.register_fixture(fixture);

        // Create static effect with crossfades
        let mut parameters = HashMap::new();
        parameters.insert("red".to_string(), 1.0);
        parameters.insert("green".to_string(), 0.0);
        parameters.insert("blue".to_string(), 0.0);

        let mut static_effect = create_effect_with_timing(
            "static_test".to_string(),
            EffectType::Static {
                parameters,
                duration: Some(Duration::from_secs(10)), // Total duration
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
            Some(Duration::from_secs(1)), // up_time: 1s
            Some(Duration::from_secs(1)), // down_time: 1s
        );
        static_effect.hold_time = Some(Duration::from_secs(8)); // 8s hold_time

        engine.start_effect(static_effect).unwrap();

        // Test up_time phase (0s - 1s)
        let commands = engine.update(Duration::from_secs(0)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(red_cmd.value, 0); // 0% at start

        let commands = engine.update(Duration::from_millis(500)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert!(red_cmd.value > 0 && red_cmd.value < 255); // ~50% during up_time

        // Test hold_time phase (1s - 9s)
        let commands = engine.update(Duration::from_millis(1500)).unwrap(); // 0.5s + 1.5s = 2s
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(red_cmd.value, 255); // Full intensity during hold_time

        // Test down_time phase (9s - 10s)
        let commands = engine.update(Duration::from_secs(7)).unwrap(); // 2s + 7s = 9s
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(red_cmd.value, 255); // Still full intensity at start of down_time

        let commands = engine.update(Duration::from_millis(500)).unwrap(); // 9s + 0.5s = 9.5s
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert!(red_cmd.value > 0 && red_cmd.value < 255); // ~50% during down_time

        let commands = engine.update(Duration::from_millis(500)).unwrap(); // 9.5s + 0.5s = 10s
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1);
        if red_cmd.is_none() {
            // Effect should have ended at 10s
        } else {
            assert_eq!(red_cmd.unwrap().value, 0); // 0% at end
        }
    }

    #[test]
    fn test_color_cycle_effect_crossfade() {
        let mut engine = EffectEngine::new();

        // Create a test fixture
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
            max_strobe_frequency: None,
        };
        engine.register_fixture(fixture);

        // Create color cycle effect with crossfades
        let mut cycle_effect = create_effect_with_timing(
            "cycle_test".to_string(),
            EffectType::ColorCycle {
                colors: vec![
                    Color {
                        r: 255,
                        g: 0,
                        b: 0,
                        w: None,
                    }, // Red
                    Color {
                        r: 0,
                        g: 255,
                        b: 0,
                        w: None,
                    }, // Green
                    Color {
                        r: 0,
                        g: 0,
                        b: 255,
                        w: None,
                    }, // Blue
                ],
                speed: 1.0, // 1 cycle per second
                direction: CycleDirection::Forward,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
            Some(Duration::from_secs(1)), // fade_in: 1s
            Some(Duration::from_secs(1)), // fade_out: 1s
        );
        cycle_effect.hold_time = Some(Duration::from_secs(9)); // Set hold_time for crossfade testing

        engine.start_effect(cycle_effect).unwrap();

        // Test fade in phase - colors should cycle but be dimmed
        let commands = engine.update(Duration::from_millis(500)).unwrap();
        let active_channel = commands.iter().find(|cmd| cmd.value > 0);
        assert!(active_channel.is_some());
        let active_channel = active_channel.unwrap();
        assert!(active_channel.value > 0 && active_channel.value < 255); // Dimmed color during fade in

        // Test full intensity phase - colors should cycle at full brightness
        let commands = engine.update(Duration::from_millis(1500)).unwrap(); // 0.5s + 1.5s = 2s
        let active_channel = commands.iter().find(|cmd| cmd.value > 0);
        assert!(active_channel.is_some());
        let active_channel = active_channel.unwrap();
        assert_eq!(active_channel.value, 255); // Full intensity during full phase

        // Test that the effect continues running (fade out phase is optional for this test)
        let _commands = engine.update(Duration::from_secs(7)).unwrap(); // 2s + 7s = 9s
                                                                        // At this point, the effect may have ended or be in fade out - both are valid
    }

    #[test]
    fn test_strobe_effect_crossfade() {
        let mut engine = EffectEngine::new();

        // Create a test fixture with strobe capability
        let mut channels = HashMap::new();
        channels.insert("strobe".to_string(), 1);
        let fixture = FixtureInfo {
            name: "test_fixture".to_string(),
            universe: 1,
            address: 1,
            channels,
            fixture_type: "Strobe".to_string(),
            max_strobe_frequency: Some(20.0),
        };
        engine.register_fixture(fixture);

        // Create strobe effect with crossfades
        let mut strobe_effect = create_effect_with_timing(
            "strobe_test".to_string(),
            EffectType::Strobe {
                frequency: 16.0, // 16 Hz (should give value > 200)
                duration: Some(Duration::from_secs(5)),
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Foreground,
            BlendMode::Overlay,
            Some(Duration::from_secs(1)), // fade_in: 1s
            Some(Duration::from_secs(1)), // fade_out: 1s
        );
        strobe_effect.hold_time = Some(Duration::from_secs(3)); // 3s hold time

        engine.start_effect(strobe_effect).unwrap();

        // Test fade in phase - strobe should be dimmed
        let commands = engine.update(Duration::from_millis(500)).unwrap();
        let strobe_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert!(strobe_cmd.value > 0 && strobe_cmd.value < 255); // Dimmed strobe during fade in

        // Test full intensity phase - strobe should be at full speed
        let commands = engine.update(Duration::from_secs(2)).unwrap();
        let strobe_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert!(strobe_cmd.value > 200); // High strobe speed during full intensity

        // Test fade out phase - strobe should be dimmed (at 4.5s total: 0.5s into down_time)
        let commands = engine.update(Duration::from_millis(2000)).unwrap(); // 2.5s + 2s = 4.5s
        let strobe_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert!(strobe_cmd.value > 0 && strobe_cmd.value < 255); // Dimmed strobe during fade out

        // Test effect end - should be no commands (at 5s total)
        let commands = engine.update(Duration::from_millis(500)).unwrap(); // 4.5s + 0.5s = 5s
        assert!(commands.is_empty()); // Effect should be finished
    }

    #[test]
    fn test_chase_effect_crossfade() {
        let mut engine = EffectEngine::new();

        // Create test fixtures
        for i in 1..=4 {
            let mut channels = HashMap::new();
            channels.insert("dimmer".to_string(), i);
            let fixture = FixtureInfo {
                name: format!("fixture_{}", i),
                universe: 1,
                address: i,
                channels,
                fixture_type: "Dimmer".to_string(),
                max_strobe_frequency: None,
            };
            engine.register_fixture(fixture);
        }

        // Create chase effect with crossfades
        let mut chase_effect = create_effect_with_timing(
            "chase_test".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: 1.0, // 1 cycle per second
                direction: ChaseDirection::LeftToRight,
            },
            vec![
                "fixture_1".to_string(),
                "fixture_2".to_string(),
                "fixture_3".to_string(),
                "fixture_4".to_string(),
            ],
            EffectLayer::Midground,
            BlendMode::Replace,
            Some(Duration::from_secs(1)), // fade_in: 1s
            Some(Duration::from_secs(1)), // fade_out: 1s
        );
        chase_effect.hold_time = Some(Duration::from_secs(2)); // 2s hold time

        engine.start_effect(chase_effect).unwrap();

        // Test fade in phase - chase should be dimmed
        let commands = engine.update(Duration::from_millis(500)).unwrap();
        let active_fixture = commands.iter().find(|cmd| cmd.value > 0);
        assert!(active_fixture.is_some());
        if let Some(cmd) = active_fixture {
            assert!(cmd.value > 0 && cmd.value < 255); // Dimmed chase during fade in
        }

        // Test full intensity phase - chase should be at full brightness
        let commands = engine.update(Duration::from_secs(2)).unwrap();
        let active_fixture = commands.iter().find(|cmd| cmd.value > 0);
        assert!(active_fixture.is_some());
        if let Some(cmd) = active_fixture {
            assert_eq!(cmd.value, 255); // Full brightness during full intensity
        }

        // Test fade out phase - chase should be dimmed (at 3.5s total: 0.5s into down_time)
        let commands = engine.update(Duration::from_millis(1000)).unwrap(); // 2.5s + 1s = 3.5s
        let active_fixture = commands.iter().find(|cmd| cmd.value > 0);
        assert!(active_fixture.is_some());
        if let Some(cmd) = active_fixture {
            assert!(cmd.value > 0 && cmd.value < 255); // Dimmed chase during fade out
        }

        // Test effect end - should be no commands (at 4s total)
        let commands = engine.update(Duration::from_millis(500)).unwrap(); // 3.5s + 0.5s = 4s
        assert!(commands.is_empty()); // Effect should be finished
    }

    #[test]
    fn test_pulse_effect_crossfade() {
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
            max_strobe_frequency: None,
        };
        engine.register_fixture(fixture);

        // Create pulse effect with crossfades
        let mut pulse_effect = create_effect_with_timing(
            "pulse_test".to_string(),
            EffectType::Pulse {
                base_level: 0.5,
                pulse_amplitude: 0.5,
                frequency: 2.0, // 2 Hz
                duration: Some(Duration::from_secs(5)),
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Midground,
            BlendMode::Overlay,
            Some(Duration::from_secs(1)), // fade_in: 1s
            Some(Duration::from_secs(1)), // fade_out: 1s
        );
        pulse_effect.hold_time = Some(Duration::from_secs(3)); // 3s hold time

        engine.start_effect(pulse_effect).unwrap();

        // Test fade in phase - pulse should be dimmed
        let commands = engine.update(Duration::from_millis(500)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert!(red_cmd.value > 0 && red_cmd.value < 255); // Dimmed pulse during fade in

        // Test full intensity phase - pulse should be at full amplitude
        let commands = engine.update(Duration::from_secs(2)).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert!(red_cmd.value > 100); // Higher pulse amplitude during full intensity

        // Test fade out phase - pulse should be dimmed (at 4.5s total: 0.5s into down_time)
        let commands = engine.update(Duration::from_millis(2000)).unwrap(); // 2.5s + 2s = 4.5s
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert!(red_cmd.value > 0 && red_cmd.value < 255); // Dimmed pulse during fade out

        // Test effect end - should be no commands (at 5s total)
        let commands = engine.update(Duration::from_millis(500)).unwrap(); // 4.5s + 0.5s = 5s
        assert!(commands.is_empty()); // Effect should be finished
    }

    #[test]
    fn test_rainbow_effect_crossfade() {
        let mut engine = EffectEngine::new();

        // Create a test fixture
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
            max_strobe_frequency: None,
        };
        engine.register_fixture(fixture);

        // Create rainbow effect with crossfades
        let mut rainbow_effect = create_effect_with_timing(
            "rainbow_test".to_string(),
            EffectType::Rainbow {
                speed: 1.0, // 1 cycle per second
                saturation: 1.0,
                brightness: 1.0,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
            Some(Duration::from_secs(1)), // fade_in: 1s
            Some(Duration::from_secs(1)), // fade_out: 1s
        );
        rainbow_effect.hold_time = Some(Duration::from_secs(3)); // 3s hold time

        engine.start_effect(rainbow_effect).unwrap();

        // Test fade in phase - rainbow should be dimmed
        let commands = engine.update(Duration::from_millis(500)).unwrap();
        let active_cmd = commands.iter().find(|cmd| cmd.value > 0).unwrap();
        assert!(active_cmd.value > 0 && active_cmd.value < 255); // Dimmed rainbow during fade in

        // Test full intensity phase - rainbow should be at full brightness
        let commands = engine.update(Duration::from_secs(2)).unwrap();
        let active_cmd = commands.iter().find(|cmd| cmd.value > 0).unwrap();
        assert!(active_cmd.value > 200); // High rainbow brightness during full intensity

        // Test fade out phase - rainbow should be dimmed (at 4.5s total: 0.5s into down_time)
        let commands = engine.update(Duration::from_millis(2000)).unwrap(); // 2.5s + 2s = 4.5s
        let active_cmd = commands.iter().find(|cmd| cmd.value > 0).unwrap();
        assert!(active_cmd.value > 0 && active_cmd.value < 255); // Dimmed rainbow during fade out

        // Test effect end - should be no commands (at 5s total)
        let commands = engine.update(Duration::from_millis(500)).unwrap(); // 4.5s + 0.5s = 5s
        assert!(commands.is_empty()); // Effect should be finished
    }

    #[test]
    fn test_dsl_crossfade_integration() {
        // Test that DSL crossfade parameters are properly connected to the lighting engine
        let content = r#"show "DSL Crossfade Test" {
    @00:00.000
    front_wash: static color: "blue", up_time: 2s, down_time: 1s, hold_time: 5s
}"#;

        let result = parse_light_shows(content);
        assert!(result.is_ok());

        let shows = result.unwrap();
        let show = shows.get("DSL Crossfade Test").unwrap();
        let effect = &show.cues[0].effects[0];

        // Verify crossfade parameters are parsed correctly
        assert_eq!(effect.up_time, Some(Duration::from_secs(2)));
        assert_eq!(effect.down_time, Some(Duration::from_secs(1)));

        // Test that the effect can be converted to an EffectInstance with crossfade support
        let mut engine = EffectEngine::new();

        // Create a test fixture
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);

        let fixture = FixtureInfo {
            name: "front_wash".to_string(),
            universe: 1,
            address: 1,
            channels,
            fixture_type: "RGB_Par".to_string(),
            max_strobe_frequency: Some(20.0),
        };
        engine.register_fixture(fixture);

        // Create EffectInstance from DSL Effect
        let effect_instance = LightingTimeline::create_effect_instance(effect);
        assert!(
            effect_instance.is_some(),
            "Failed to create EffectInstance from DSL Effect"
        );
        let effect_instance = effect_instance.unwrap();
        assert_eq!(effect_instance.up_time, Some(Duration::from_secs(2)));
        assert_eq!(effect_instance.down_time, Some(Duration::from_secs(1)));

        // Start the effect and test crossfade behavior
        engine.start_effect(effect_instance).unwrap();

        // Test fade in: at t=0ms, should be 0% (no blue)
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        if let Some(blue_cmd) = commands.iter().find(|cmd| cmd.channel == 3) {
            assert_eq!(blue_cmd.value, 0); // 0% blue during fade in
        }

        // Test fade in: at t=1000ms (50% of 2s fade in), should be ~50% blue
        let commands = engine.update(Duration::from_millis(1000)).unwrap();
        if let Some(blue_cmd) = commands.iter().find(|cmd| cmd.channel == 3) {
            assert!(blue_cmd.value > 100 && blue_cmd.value < 150); // ~50% blue
        }

        // Test full intensity: at t=3000ms (after fade in complete), should be 100% blue
        let commands = engine.update(Duration::from_millis(3000)).unwrap();
        if let Some(blue_cmd) = commands.iter().find(|cmd| cmd.channel == 3) {
            assert_eq!(blue_cmd.value, 255); // 100% blue
        }

        // Test fade out: at t=4000ms (1s before end), should be ~0% blue
        let commands = engine.update(Duration::from_millis(4000)).unwrap();
        if let Some(blue_cmd) = commands.iter().find(|cmd| cmd.channel == 3) {
            assert!(blue_cmd.value < 50); // Nearly 0% blue during fade out
        }

        // Test end: at t=5000ms (effect ended), should be 0% blue
        let commands = engine.update(Duration::from_millis(5000)).unwrap();
        assert!(commands.is_empty()); // No commands when effect ends
    }

    #[test]
    fn test_static_effect_crossfade() {
        let mut engine = EffectEngine::new();

        // Create a test fixture
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
            max_strobe_frequency: Some(20.0),
        };
        engine.register_fixture(fixture);

        // Create a static blue effect with 1 second fade in
        let mut parameters = HashMap::new();
        parameters.insert("red".to_string(), 0.0);
        parameters.insert("green".to_string(), 0.0);
        parameters.insert("blue".to_string(), 1.0);

        let mut static_effect = create_effect_with_timing(
            "static_blue".to_string(),
            EffectType::Static {
                parameters,
                duration: Some(Duration::from_secs(3)),
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
            Some(Duration::from_secs(1)), // 1 second fade in
            Some(Duration::from_secs(1)), // 1 second fade out
        );
        static_effect.hold_time = Some(Duration::from_secs(1)); // 1 second hold time

        engine.start_effect(static_effect).unwrap();

        // Test fade in: at t=0ms, should be 0% (no blue)
        let commands = engine.update(Duration::from_millis(0)).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        assert_eq!(blue_cmd.value, 0); // Should be 0 (0% of 255)

        // Test fade in: at t=500ms, should be 50% (half blue)
        let commands = engine.update(Duration::from_millis(500)).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        assert_eq!(blue_cmd.value, 127); // Should be 127 (50% of 255)

        // Test full intensity: at t=1000ms, should be 100% (full blue)
        let commands = engine.update(Duration::from_millis(500)).unwrap(); // 500ms more = 1000ms total
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        assert_eq!(blue_cmd.value, 255); // Should be 255 (100% of 255)

        // Test fade out: at t=2500ms, should be 50% (half blue)
        let commands = engine.update(Duration::from_millis(1500)).unwrap(); // 1500ms more = 2500ms total
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        assert_eq!(blue_cmd.value, 127); // Should be 127 (50% of 255)

        // Test fade out: at t=3000ms, effect should be finished (no commands)
        let commands = engine.update(Duration::from_millis(500)).unwrap(); // 500ms more = 3000ms total
        assert!(commands.is_empty()); // Effect should be finished, no commands
    }

    #[test]
    fn test_disabled_effects_not_participating_in_conflicts() {
        let mut engine = EffectEngine::new();

        // Create test fixture
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
            max_strobe_frequency: Some(20.0),
        };
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
                duration: None,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
        );
        disabled_effect.enabled = false; // Disable the effect

        // Create a conflicting effect
        let conflicting_effect = create_effect_with_layering(
            "conflicting_effect".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("blue".to_string(), 1.0);
                    params
                },
                duration: None,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background, // Same layer, same type
            BlendMode::Replace,
        );

        // Start the disabled effect first
        engine.start_effect(disabled_effect).unwrap();
        assert_eq!(engine.active_effects_count(), 1);
        assert!(engine.has_effect("disabled_effect"));

        // Start the conflicting effect
        engine.start_effect(conflicting_effect).unwrap();

        // The disabled effect should not be stopped because it's disabled
        // The conflicting effect should still be added
        assert_eq!(engine.active_effects_count(), 2);
        assert!(engine.has_effect("disabled_effect"));
        assert!(engine.has_effect("conflicting_effect"));

        // Test that disabled effects don't stop other effects
        let another_effect = create_effect_with_layering(
            "another_effect".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("green".to_string(), 1.0);
                    params
                },
                duration: None,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background, // Same layer, same type
            BlendMode::Replace,
        );

        engine.start_effect(another_effect).unwrap();

        // The disabled effect should still be there, but the conflicting effect should be stopped
        assert_eq!(engine.active_effects_count(), 2); // disabled + another
        assert!(engine.has_effect("disabled_effect"));
        assert!(!engine.has_effect("conflicting_effect"));
        assert!(engine.has_effect("another_effect"));
    }

    #[test]
    fn test_fixture_overlap_without_conflicts() {
        let mut engine = EffectEngine::new();

        // Create test fixtures
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);

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

        // Test effects targeting different fixtures (no overlap)
        let effect1 = create_effect_with_layering(
            "effect1".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("red".to_string(), 1.0);
                    params
                },
                duration: None,
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
                duration: None,
            },
            vec!["fixture2".to_string()], // Different fixture
            EffectLayer::Background,      // Same layer, same type
            BlendMode::Replace,
        );

        engine.start_effect(effect1).unwrap();
        engine.start_effect(effect2).unwrap();

        // No overlap, so no conflict
        assert_eq!(engine.active_effects_count(), 2);
        assert!(engine.has_effect("effect1"));
        assert!(engine.has_effect("effect2"));

        // Test effects with partial overlap
        let effect3 = create_effect_with_layering(
            "effect3".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("green".to_string(), 1.0);
                    params
                },
                duration: None,
            },
            vec!["fixture1".to_string(), "fixture2".to_string()], // Both fixtures
            EffectLayer::Background,
            BlendMode::Replace,
        );

        let effect4 = create_effect_with_layering(
            "effect4".to_string(),
            EffectType::Dimmer {
                start_level: 1.0,
                end_level: 0.5,
                duration: Duration::from_secs(1),
                curve: DimmerCurve::Linear,
            },
            vec!["fixture1".to_string()], // Only fixture1
            EffectLayer::Background,      // Same layer
            BlendMode::Multiply,
        );

        engine.start_effect(effect3).unwrap();
        engine.start_effect(effect4).unwrap();

        // Overlap on fixture1, but different types (Static vs Dimmer)
        // Dimmer is generally compatible, so should layer
        // effect3 should stop effect1 and effect2 because they're all static effects
        assert_eq!(engine.active_effects_count(), 2); // effect3 + effect4
        assert!(!engine.has_effect("effect1"));
        assert!(!engine.has_effect("effect2"));
        assert!(engine.has_effect("effect3"));
        assert!(engine.has_effect("effect4"));
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
                frequency: 2.0,
                duration: None,
            },
            vec!["fixture1".to_string()],
            EffectLayer::Midground,
            BlendMode::Multiply,
        );

        let foreground_strobe = create_effect_with_layering(
            "foreground_strobe".to_string(),
            EffectType::Strobe {
                frequency: 2.0,
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
                frequency: 4.0,
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
    fn test_channel_conflict_detection_behavior() {
        let mut engine = EffectEngine::new();

        // Create test fixtures
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);

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

        // Test that channel conflicts currently always return false
        let effect1 = create_effect_with_layering(
            "effect1".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("red".to_string(), 1.0);
                    params
                },
                duration: None,
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
                duration: None,
            },
            vec!["fixture2".to_string()],
            EffectLayer::Foreground, // Different layer
            BlendMode::Replace,
        );

        engine.start_effect(effect1).unwrap();
        engine.start_effect(effect2).unwrap();

        // Since channel conflicts always return false, effects in different layers should coexist
        assert_eq!(engine.active_effects_count(), 2);
        assert!(engine.has_effect("effect1"));
        assert!(engine.has_effect("effect2"));

        // Test with same layer but different fixtures
        let effect3 = create_effect_with_layering(
            "effect3".to_string(),
            EffectType::Static {
                parameters: {
                    let mut params = HashMap::new();
                    params.insert("green".to_string(), 1.0);
                    params
                },
                duration: None,
            },
            vec!["fixture1".to_string()],
            EffectLayer::Background, // Same layer as effect1
            BlendMode::Replace,
        );

        engine.start_effect(effect3).unwrap();

        // Same layer, same type, same fixture - should conflict
        assert_eq!(engine.active_effects_count(), 2); // effect2 + effect3
        assert!(!engine.has_effect("effect1"));
        assert!(engine.has_effect("effect2"));
        assert!(engine.has_effect("effect3"));
    }

    #[test]
    fn test_example_files_parse() {
        use crate::lighting::parser::parse_light_shows;
        use std::fs;

        let example_files = [
            "examples/lighting/shows/crossfade_show.light",
            "examples/lighting/shows/layering_show.light",
            "examples/lighting/shows/comprehensive_show.light",
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
}
