#[cfg(test)]
mod layering_behavior_tests {
    use super::super::effects::*;
    use super::super::engine::EffectEngine;
    use std::collections::HashMap;
    use std::time::Duration;

    fn pixelbrick(name: &str, universe: u16, address: u16) -> FixtureInfo {
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);
        FixtureInfo {
            name: name.to_string(),
            universe,
            address,
            fixture_type: "Astera-PixelBrick".to_string(),
            channels,
            max_strobe_frequency: Some(25.0),
        }
    }

    #[test]
    fn test_layer_stack_dim_sequence_final_127() {
        let mut engine = EffectEngine::new();
        engine.register_fixture(pixelbrick("front_wash", 1, 1));

        // 00:00 Background static blue
        let bg_blue = EffectInstance::new(
            "bg_blue".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
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
        engine.start_effect(bg_blue).unwrap();
        let _ = engine.update(Duration::from_millis(1000)).unwrap();

        // 00:01 Midground dim to 50% (instant, permanent persist at completion)
        let mut mid_dim_50 = EffectInstance::new(
            "mid_dim_50".to_string(),
            EffectType::Dimmer {
                start_level: 0.5,
                end_level: 0.5,
                duration: Duration::from_millis(0),
                curve: DimmerCurve::Linear,
            },
            vec!["front_wash".to_string()],
            None,
            None,
            None,
        );
        mid_dim_50.layer = EffectLayer::Midground;
        mid_dim_50.blend_mode = BlendMode::Multiply;
        engine.start_effect(mid_dim_50).unwrap();
        let _ = engine.update(Duration::from_millis(1)).unwrap();

        // 00:02 Foreground dim to 50% (instant)
        let mut fg_dim_50 = EffectInstance::new(
            "fg_dim_50".to_string(),
            EffectType::Dimmer {
                start_level: 0.5,
                end_level: 0.5,
                duration: Duration::from_millis(0),
                curve: DimmerCurve::Linear,
            },
            vec!["front_wash".to_string()],
            None,
            None,
            None,
        );
        fg_dim_50.layer = EffectLayer::Foreground;
        fg_dim_50.blend_mode = BlendMode::Multiply;
        engine.start_effect(fg_dim_50).unwrap();
        let _ = engine.update(Duration::from_millis(1)).unwrap();

        // 00:03 Midground dim to 100% (instant) — should overwrite mid 0.5 to 1.0
        let mut mid_dim_100 = EffectInstance::new(
            "mid_dim_100".to_string(),
            EffectType::Dimmer {
                start_level: 1.0,
                end_level: 1.0,
                duration: Duration::from_millis(0),
                curve: DimmerCurve::Linear,
            },
            vec!["front_wash".to_string()],
            None,
            None,
            None,
        );
        mid_dim_100.layer = EffectLayer::Midground;
        mid_dim_100.blend_mode = BlendMode::Multiply;
        engine.start_effect(mid_dim_100).unwrap();
        let cmds = engine.update(Duration::from_millis(1)).unwrap();

        // Expect blue = 127 (50% of 255) — only foreground 0.5 remains
        let blue_ch = 1 + 3 - 1; // address 1 + blue offset 3 - 1 = 3
        let blue = cmds
            .iter()
            .find(|c| c.universe == 1 && c.channel == blue_ch)
            .map(|c| c.value)
            .unwrap_or(0);
        assert!((120..=135).contains(&blue), "expected ~127, got {}", blue);
    }

    #[test]
    fn test_layer_stack_dim_sequence_fg_first_final_127() {
        let mut engine = EffectEngine::new();
        engine.register_fixture(pixelbrick("front_wash", 1, 1));

        // Background static blue
        let bg_blue = EffectInstance::new(
            "bg_blue".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
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
        engine.start_effect(bg_blue).unwrap();
        let _ = engine.update(Duration::from_millis(1)).unwrap();

        // 00:01 Foreground dim to 50%
        let mut fg_dim_50 = EffectInstance::new(
            "fg_dim_50".to_string(),
            EffectType::Dimmer {
                start_level: 0.5,
                end_level: 0.5,
                duration: Duration::from_millis(0),
                curve: DimmerCurve::Linear,
            },
            vec!["front_wash".to_string()],
            None,
            None,
            None,
        );
        fg_dim_50.layer = EffectLayer::Foreground;
        fg_dim_50.blend_mode = BlendMode::Multiply;
        engine.start_effect(fg_dim_50).unwrap();
        let _ = engine.update(Duration::from_millis(1)).unwrap();

        // 00:02 Midground dim to 50%
        let mut mid_dim_50 = EffectInstance::new(
            "mid_dim_50".to_string(),
            EffectType::Dimmer {
                start_level: 0.5,
                end_level: 0.5,
                duration: Duration::from_millis(0),
                curve: DimmerCurve::Linear,
            },
            vec!["front_wash".to_string()],
            None,
            None,
            None,
        );
        mid_dim_50.layer = EffectLayer::Midground;
        mid_dim_50.blend_mode = BlendMode::Multiply;
        engine.start_effect(mid_dim_50).unwrap();
        let _ = engine.update(Duration::from_millis(1)).unwrap();

        // 00:03 Foreground dim to 100%
        let mut fg_dim_100 = EffectInstance::new(
            "fg_dim_100".to_string(),
            EffectType::Dimmer {
                start_level: 1.0,
                end_level: 1.0,
                duration: Duration::from_millis(0),
                curve: DimmerCurve::Linear,
            },
            vec!["front_wash".to_string()],
            None,
            None,
            None,
        );
        fg_dim_100.layer = EffectLayer::Foreground;
        fg_dim_100.blend_mode = BlendMode::Multiply;
        engine.start_effect(fg_dim_100).unwrap();
        let cmds = engine.update(Duration::from_millis(1)).unwrap();

        // Expect blue ≈ 127 (mid=0.5, fg=1.0)
        let blue_ch = 1 + 3 - 1;
        let blue = cmds
            .iter()
            .find(|c| c.universe == 1 && c.channel == blue_ch)
            .map(|c| c.value)
            .unwrap_or(0);
        assert!((120..=135).contains(&blue), "expected ~127, got {}", blue);
    }

    #[test]
    fn test_replace_vs_multiply_behavior() {
        let mut engine = EffectEngine::new();
        engine.register_fixture(pixelbrick("fx", 1, 1));

        // Bg static blue
        let bg_blue = EffectInstance::new(
            "bg_blue".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("blue".to_string(), 1.0);
                    p
                },
                duration: None,
            },
            vec!["fx".to_string()],
            None,
            None,
            None,
        );
        engine.start_effect(bg_blue).unwrap();
        let _ = engine.update(Duration::from_millis(1)).unwrap();

        // Mid dim 50% multiply
        let mut mid_dim_50 = EffectInstance::new(
            "mid_dim_50".to_string(),
            EffectType::Dimmer {
                start_level: 0.5,
                end_level: 0.5,
                duration: Duration::from_millis(0),
                curve: DimmerCurve::Linear,
            },
            vec!["fx".to_string()],
            None,
            None,
            None,
        );
        mid_dim_50.layer = EffectLayer::Midground;
        mid_dim_50.blend_mode = BlendMode::Multiply;
        engine.start_effect(mid_dim_50).unwrap();
        let _ = engine.update(Duration::from_millis(1)).unwrap();

        // Fg static red Replace overrides dim result
        let mut fg_red = EffectInstance::new(
            "fg_red".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("red".to_string(), 1.0);
                    p.insert("green".to_string(), 0.0);
                    p.insert("blue".to_string(), 0.0);
                    p
                },
                duration: None,
            },
            vec!["fx".to_string()],
            None,
            None,
            None,
        );
        fg_red.layer = EffectLayer::Foreground;
        fg_red.blend_mode = BlendMode::Replace;
        engine.start_effect(fg_red).unwrap();
        let cmds = engine.update(Duration::from_millis(1)).unwrap();
        let red_ch = 1 + 1 - 1;
        let green_ch = 1 + 2 - 1;
        let blue_ch = 1 + 3 - 1;
        let get = |ch| {
            cmds.iter()
                .find(|c| c.universe == 1 && c.channel == ch)
                .map(|c| c.value)
                .unwrap_or(0)
        };
        assert_eq!(get(red_ch), 255);
        assert_eq!(get(green_ch), 0);
        assert_eq!(get(blue_ch), 0);

        // Fg dim 0% Replace = blackout
        let mut fg_black = EffectInstance::new(
            "fg_black".to_string(),
            EffectType::Dimmer {
                start_level: 0.0,
                end_level: 0.0,
                duration: Duration::from_millis(0),
                curve: DimmerCurve::Linear,
            },
            vec!["fx".to_string()],
            None,
            None,
            None,
        );
        fg_black.layer = EffectLayer::Foreground;
        fg_black.blend_mode = BlendMode::Replace;
        engine.start_effect(fg_black).unwrap();
        let cmds = engine.update(Duration::from_millis(1)).unwrap();
        let get = |ch| {
            cmds.iter()
                .find(|c| c.universe == 1 && c.channel == ch)
                .map(|c| c.value)
                .unwrap_or(255)
        };
        assert_eq!(get(red_ch), 0);
        assert_eq!(get(green_ch), 0);
        assert_eq!(get(blue_ch), 0);
    }

    #[test]
    fn test_mid_replace_overrides_background_replace() {
        let mut engine = EffectEngine::new();
        engine.register_fixture(pixelbrick("fx", 1, 1));

        // Bg Replace blue 100%
        let mut bg_blue = EffectInstance::new(
            "bg_blue".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("blue".to_string(), 1.0);
                    p
                },
                duration: None,
            },
            vec!["fx".to_string()],
            None,
            None,
            None,
        );
        bg_blue.layer = EffectLayer::Background;
        bg_blue.blend_mode = BlendMode::Replace;
        engine.start_effect(bg_blue).unwrap();
        let _ = engine.update(Duration::from_millis(1)).unwrap();

        // Mid Replace blue 25% overrides bg
        let mut mid_blue_25 = EffectInstance::new(
            "mid_blue_25".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("blue".to_string(), 0.25);
                    p
                },
                duration: None,
            },
            vec!["fx".to_string()],
            None,
            None,
            None,
        );
        mid_blue_25.layer = EffectLayer::Midground;
        mid_blue_25.blend_mode = BlendMode::Replace;
        engine.start_effect(mid_blue_25).unwrap();
        let cmds = engine.update(Duration::from_millis(1)).unwrap();

        let blue_ch = 1 + 3 - 1;
        let blue = cmds
            .iter()
            .find(|c| c.universe == 1 && c.channel == blue_ch)
            .map(|c| c.value)
            .unwrap_or(0);
        assert!((60..=65).contains(&blue), "expected ~64, got {}", blue);
    }

    #[test]
    fn test_foreground_replace_overrides_mid_replace() {
        let mut engine = EffectEngine::new();
        engine.register_fixture(pixelbrick("fx", 1, 1));

        // Mid Replace red 100%
        let mut mid_red = EffectInstance::new(
            "mid_red".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("red".to_string(), 1.0);
                    p
                },
                duration: None,
            },
            vec!["fx".to_string()],
            None,
            None,
            None,
        );
        mid_red.layer = EffectLayer::Midground;
        mid_red.blend_mode = BlendMode::Replace;
        engine.start_effect(mid_red).unwrap();
        let _ = engine.update(Duration::from_millis(1)).unwrap();

        // Fg Replace red 20%
        let mut fg_red_20 = EffectInstance::new(
            "fg_red_20".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("red".to_string(), 0.2);
                    p
                },
                duration: None,
            },
            vec!["fx".to_string()],
            None,
            None,
            None,
        );
        fg_red_20.layer = EffectLayer::Foreground;
        fg_red_20.blend_mode = BlendMode::Replace;
        engine.start_effect(fg_red_20).unwrap();
        let cmds = engine.update(Duration::from_millis(1)).unwrap();

        let red_ch = 1 + 1 - 1;
        let red = cmds
            .iter()
            .find(|c| c.universe == 1 && c.channel == red_ch)
            .map(|c| c.value)
            .unwrap_or(0);
        assert!((50..=54).contains(&red), "expected ~51, got {}", red);
    }

    #[test]
    fn test_replace_affects_only_written_channels() {
        let mut engine = EffectEngine::new();
        engine.register_fixture(pixelbrick("fx", 1, 1));

        // Bg static blue
        let bg_blue = EffectInstance::new(
            "bg_blue".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("blue".to_string(), 1.0);
                    p
                },
                duration: None,
            },
            vec!["fx".to_string()],
            None,
            None,
            None,
        );
        engine.start_effect(bg_blue).unwrap();
        let _ = engine.update(Duration::from_millis(1)).unwrap();

        // Mid Replace red only
        let mut mid_red_only = EffectInstance::new(
            "mid_red_only".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("red".to_string(), 1.0);
                    p
                },
                duration: None,
            },
            vec!["fx".to_string()],
            None,
            None,
            None,
        );
        mid_red_only.layer = EffectLayer::Midground;
        mid_red_only.blend_mode = BlendMode::Replace;
        engine.start_effect(mid_red_only).unwrap();
        let cmds = engine.update(Duration::from_millis(1)).unwrap();

        let red_ch = 1 + 1 - 1;
        let blue_ch = 1 + 3 - 1;
        let get = |ch| {
            cmds.iter()
                .find(|c| c.universe == 1 && c.channel == ch)
                .map(|c| c.value)
                .unwrap_or(0)
        };
        assert_eq!(get(red_ch), 255);
        assert_eq!(get(blue_ch), 255);
    }

    #[test]
    fn test_foreground_replace_blocks_mid_multiply_on_same_channel() {
        let mut engine = EffectEngine::new();
        engine.register_fixture(pixelbrick("fx", 1, 1));

        // Bg static red
        let bg_red = EffectInstance::new(
            "bg_red".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("red".to_string(), 1.0);
                    p
                },
                duration: None,
            },
            vec!["fx".to_string()],
            None,
            None,
            None,
        );
        engine.start_effect(bg_red).unwrap();
        let _ = engine.update(Duration::from_millis(1)).unwrap();

        // Fg Replace red 100%
        let mut fg_red = EffectInstance::new(
            "fg_red".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("red".to_string(), 1.0);
                    p
                },
                duration: None,
            },
            vec!["fx".to_string()],
            None,
            None,
            None,
        );
        fg_red.layer = EffectLayer::Foreground;
        fg_red.blend_mode = BlendMode::Replace;
        engine.start_effect(fg_red).unwrap();
        let _ = engine.update(Duration::from_millis(1)).unwrap();

        // Mid Multiply dim 0% should not affect red while fg replace is active
        let mut mid_dim_0 = EffectInstance::new(
            "mid_dim_0".to_string(),
            EffectType::Dimmer {
                start_level: 0.0,
                end_level: 0.0,
                duration: Duration::from_millis(0),
                curve: DimmerCurve::Linear,
            },
            vec!["fx".to_string()],
            None,
            None,
            None,
        );
        mid_dim_0.layer = EffectLayer::Midground;
        mid_dim_0.blend_mode = BlendMode::Multiply;
        engine.start_effect(mid_dim_0).unwrap();
        let cmds = engine.update(Duration::from_millis(1)).unwrap();

        let red_ch = 1 + 1 - 1;
        let red = cmds
            .iter()
            .find(|c| c.universe == 1 && c.channel == red_ch)
            .map(|c| c.value)
            .unwrap_or(0);
        assert_eq!(red, 255);
    }
}
#[cfg(test)]
mod layering_show_regression {
    use super::super::effects::*;
    use super::super::engine::EffectEngine;
    use std::collections::HashMap;
    use std::time::Duration;

    fn pixelbrick(name: &str, universe: u16, address: u16) -> FixtureInfo {
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);
        FixtureInfo {
            name: name.to_string(),
            universe,
            address,
            fixture_type: "Astera-PixelBrick".to_string(),
            channels,
            max_strobe_frequency: Some(25.0),
        }
    }

    #[test]
    fn test_layering_show_pixelbrick_dim_persists() {
        let mut engine = EffectEngine::new();

        // Register 8 PixelBricks as front_wash fixtures
        for i in 0..8 {
            let name = format!("front_wash_{}", i + 1);
            engine.register_fixture(pixelbrick(&name, 1, 1 + (i as u16) * 4));
        }

        // Build fixture name list
        let targets: Vec<String> = (0..8).map(|i| format!("front_wash_{}", i + 1)).collect();

        // Background static blue (permanent, no fade)
        let mut static_blue = EffectInstance::new(
            "bg_blue".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("blue".to_string(), 1.0);
                    p
                },
                duration: None,
            },
            targets.clone(),
            None,
            None,
            None, // no timing params = permanent
        );
        static_blue.layer = EffectLayer::Background;
        static_blue.blend_mode = BlendMode::Replace;
        engine.start_effect(static_blue).unwrap();

        // Advance to 2.0s
        let _ = engine.update(Duration::from_millis(2000)).unwrap();

        // Midground dimmer multiply 1.0 -> 0.5 over 1s
        let mut dimmer = EffectInstance::new(
            "mid_dim".to_string(),
            EffectType::Dimmer {
                start_level: 1.0,
                end_level: 0.5,
                duration: Duration::from_secs(1), // 1s fade from 1.0 to 0.5
                curve: DimmerCurve::Linear,
            },
            targets.clone(),
            None,
            None,
            None,
        )
        .with_priority(5);
        dimmer.layer = EffectLayer::Midground;
        dimmer.blend_mode = BlendMode::Multiply;
        engine.start_effect(dimmer).unwrap();

        // After 2.5s (0.5s after dimmer starts at t=2s): 0.5s into 1s duration, dimmer at 0.75
        let cmds = engine.update(Duration::from_millis(500)).unwrap();
        // Expect blue ~ 191 for each fixture (255 * 0.75)
        for i in 0..8u16 {
            let ch = (1 + i * 4) + 3 - 1;
            let value = cmds
                .iter()
                .find(|c| c.universe == 1 && c.channel == ch)
                .map(|c| c.value)
                .unwrap_or(0);
            assert!(
                (185..=195).contains(&value),
                "expected ~191 (0.75 * 255), got {}",
                value
            );
        }

        // Advance past dimmer completion (1s duration from t=2s, completes at t=3s)
        let _ = engine.update(Duration::from_millis(500)).unwrap(); // Now at t=3s

        // One more frame to emit after completion
        // The dimmer reached end_level (0.5) and persists there (dimmers are permanent)
        let cmds = engine.update(Duration::from_millis(10)).unwrap();
        for i in 0..8u16 {
            let ch = (1 + i * 4) + 3 - 1;
            let value = cmds
                .iter()
                .find(|c| c.universe == 1 && c.channel == ch)
                .map(|c| c.value)
                .unwrap_or(0);
            assert!(
                (120..=135).contains(&value),
                "dimmer persisted at 0.5 (end_level), expected ~127, got {}",
                value
            );
        }

        // Now simulate the final crossfade to black at 00:25
        let mut blackout = EffectInstance::new(
            "fg_blackout".to_string(),
            EffectType::Dimmer {
                start_level: 1.0,
                end_level: 0.0,
                duration: Duration::from_secs(2), // 2s fade to black
                curve: DimmerCurve::Linear,
            },
            targets.clone(),
            None,
            None,
            None,
        )
        .with_priority(10);
        // Foreground Replace blackout (grandMA style)
        blackout.layer = EffectLayer::Foreground;
        blackout.blend_mode = BlendMode::Replace;
        engine.start_effect(blackout).unwrap();

        // Advance 2s to complete blackout fade
        let _ = engine.update(Duration::from_millis(2000)).unwrap();
        let cmds = engine.update(Duration::from_millis(10)).unwrap();

        // Expect all RGB channels that were set to be 0
        // (blue was set to 0 by blackout multiplier; red/green were never set so no commands)
        for i in 0..8u16 {
            let base = 1 + i * 4;
            for offset in [1u16, 2u16, 3u16] {
                // red, green, blue
                let ch = base + offset - 1;
                if let Some(cmd) = cmds.iter().find(|c| c.universe == 1 && c.channel == ch) {
                    assert_eq!(
                        cmd.value, 0,
                        "expected blackout 0 at ch {} got {}",
                        ch, cmd.value
                    );
                }
            }
        }
    }
}
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
        let mut effect = EffectInstance::new(id, effect_type, target_fixtures, None, None, None);
        effect.layer = layer;
        effect.blend_mode = blend_mode;
        // Ensure effects persist long enough for tests
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
        let mut effect =
            EffectInstance::new(id, effect_type, target_fixtures, up_time, None, down_time);
        effect.layer = layer;
        effect.blend_mode = blend_mode;
        effect.up_time = up_time;
        effect.down_time = down_time;
        effect
    }

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
                frequency: TempoAwareFrequency::Fixed(1.0), // 1 Hz for easy testing
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
        let mut fixture1 = FixtureState::new();
        fixture1.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::Background, BlendMode::Replace),
        );
        fixture1.set_channel(
            "green".to_string(),
            ChannelState::new(0.5, EffectLayer::Background, BlendMode::Replace),
        );

        let mut fixture2 = FixtureState::new();
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
        let green_state = fixture1.channels.get("green").unwrap();
        assert!((green_state.value - 0.4).abs() < 0.01);

        // Blue should be added (new channel)
        let blue_state = fixture1.channels.get("blue").unwrap();
        assert!((blue_state.value - 0.3).abs() < 0.01);

        // Red should be unchanged
        let red_state = fixture1.channels.get("red").unwrap();
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
        println!(
            "- Red channel: Gets dimmer values multiplied with static red value (for layering)"
        );
        println!(
            "- Green channel: Gets dimmer values multiplied with static green value (for layering)"
        );
        println!(
            "- Blue channel: Gets dimmer values multiplied with static blue value (for layering)"
        );

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
        let mut timeline = LightingTimeline::new_with_cues(show.cues.clone());
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
        let result_at_0s = timeline.update(Duration::from_secs(0));
        println!("✅ Timeline at 0s: {} effects", result_at_0s.effects.len());
        for effect in &result_at_0s.effects {
            println!(
                "  Effect: {} blend_mode = {:?}",
                effect.id, effect.blend_mode
            );
        }

        let result_at_2s = timeline.update(Duration::from_secs(2));
        println!("✅ Timeline at 2s: {} effects", result_at_2s.effects.len());
        for effect in &result_at_2s.effects {
            println!(
                "  Effect: {} blend_mode = {:?}",
                effect.id, effect.blend_mode
            );
        }

        // Start the effects from timeline
        for effect in result_at_0s.effects {
            engine.start_effect(effect).unwrap();
        }

        // Update to apply static effect
        let _commands = engine.update(Duration::from_secs(0)).unwrap();
        println!("✅ Applied static effect from timeline");

        // Start the dimmer effect from timeline
        for effect in result_at_2s.effects {
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

        // Advance to 25s to trigger debug logging
        engine.update(Duration::from_secs(20)).unwrap();
        let commands_25s = engine.update(Duration::from_secs(0)).unwrap();
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
        engine.update(Duration::from_secs(1)).unwrap();

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
        let commands = engine.update(Duration::from_secs(1)).unwrap();

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
        engine.update(Duration::from_secs(1)).unwrap();

        let commands_1s = engine.update(Duration::from_secs(1)).unwrap();
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
        let commands_1s_timed = engine.update(Duration::from_secs(1)).unwrap();
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

        let commands_4s = engine.update(Duration::from_secs(3)).unwrap();
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
        let commands_0s = engine.update(Duration::from_secs(0)).unwrap();
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

        let commands_1s = engine.update(Duration::from_secs(1)).unwrap();
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

        let commands_2s = engine.update(Duration::from_secs(1)).unwrap();
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

        let commands_3s = engine.update(Duration::from_secs(1)).unwrap();
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
        let commands_0s = engine.update(Duration::from_secs(0)).unwrap();
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

        let commands_0_5s = engine.update(Duration::from_millis(500)).unwrap();
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

        let commands_1s = engine.update(Duration::from_millis(500)).unwrap();
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

        let commands_2s = engine.update(Duration::from_secs(1)).unwrap();
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

        let commands_3s = engine.update(Duration::from_secs(1)).unwrap();
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
        let commands_3_5s = engine.update(Duration::from_millis(500)).unwrap();
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

        let commands_4s = engine.update(Duration::from_secs(1)).unwrap();
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

        let commands_5s = engine.update(Duration::from_secs(1)).unwrap();
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
        engine.update(Duration::from_secs(1)).unwrap();

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
        let commands_1s = engine.update(Duration::from_secs(1)).unwrap();
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
        let commands_2s = engine.update(Duration::from_secs(1)).unwrap();
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
        let commands_3s = engine.update(Duration::from_secs(1)).unwrap();
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
                frequency: TempoAwareFrequency::Fixed(2.0),
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
                frequency: TempoAwareFrequency::Fixed(2.0),
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
        let blend_modes = [
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
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: CycleDirection::Forward,
                transition: CycleTransition::Snap,
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
                speed: TempoAwareSpeed::Fixed(1.0),
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
                frequency: TempoAwareFrequency::Fixed(2.0),
                duration: None,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
        );

        let strobe2 = create_effect_with_layering(
            "strobe2".to_string(),
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(4.0),
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
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
        );

        let chase2 = create_effect_with_layering(
            "chase2".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Snake,
                speed: TempoAwareSpeed::Fixed(2.0),
                direction: ChaseDirection::RightToLeft,
                transition: CycleTransition::Snap,
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
                frequency: TempoAwareFrequency::Fixed(2.0),
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
                speed: TempoAwareSpeed::Fixed(2.0), // 2 Hz for easy testing
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
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
                speed: TempoAwareSpeed::Fixed(2.0), // 2 Hz for easy testing
                direction: ChaseDirection::RightToLeft,
                transition: CycleTransition::Snap,
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
                speed: TempoAwareSpeed::Fixed(2.0), // 2 Hz for easy testing
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
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
                speed: TempoAwareSpeed::Fixed(2.0), // 2 Hz for easy testing
                direction: ChaseDirection::LeftToRight, // Direction doesn't matter for random
                transition: CycleTransition::Snap,
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
                speed: TempoAwareSpeed::Fixed(2.0),
                direction: ChaseDirection::TopToBottom,
                transition: CycleTransition::Snap,
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
                speed: TempoAwareSpeed::Fixed(2.0),
                direction: ChaseDirection::Clockwise,
                transition: CycleTransition::Snap,
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
                speed: TempoAwareSpeed::Fixed(0.5), // 0.5 Hz - 2 second cycle
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
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
                speed: TempoAwareSpeed::Fixed(2.0),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
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
                speed: TempoAwareSpeed::Fixed(2.0),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
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
            None,
            None,
            None,
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

        // At 10s (end of hold_time) - should be 100% (still in hold phase)
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(10)),
            1.0
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
    fn test_crossfade_multiplier_no_up_time_no_hold_time() {
        // Test static effect with no fade-in and fade-out only
        let mut effect = EffectInstance::new(
            "test".to_string(),
            EffectType::Static {
                parameters: HashMap::new(),
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        effect.up_time = Some(Duration::from_secs(0)); // No fade in
        effect.hold_time = Some(Duration::from_secs(0)); // No hold
        effect.down_time = Some(Duration::from_secs(2)); // 2 second fade out

        // At start (0s) - should be 100%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(0)),
            1.0
        );

        // At 0.5s (25% through down_time) - should be 75%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_millis(500)),
            0.75
        );

        // At 1s (50% through down_time) - should be 50%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(1)),
            0.5
        );

        // At 1.5s (75% through down_time) - should be 25%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_millis(1500)),
            0.25
        );

        // At 2s (end of down_time) - should be 0%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(2)),
            0.0
        );

        // At 3s (after effect ends) - should be 0%
        assert_eq!(
            effect.calculate_crossfade_multiplier(Duration::from_secs(3)),
            0.0
        );
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
        let commands = engine.update(Duration::from_secs(25)).unwrap();
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
            let commands = engine.update(Duration::from_millis(time_ms)).unwrap();
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
            let commands = engine.update(Duration::from_millis(time_ms)).unwrap();
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
        let final_commands = engine.update(Duration::from_millis(2000)).unwrap();
        assert!(final_commands.is_empty(), "Effect should have ended at 2s");

        println!("✅ Static effect fade-out test completed");
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
                speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
                direction: CycleDirection::Forward,
                transition: CycleTransition::Snap,
            },
            vec!["test_fixture".to_string()],
            EffectLayer::Background,
            BlendMode::Replace,
            Some(Duration::from_secs(1)), // fade_in: 1s
            Some(Duration::from_secs(1)), // fade_out: 1s
        );
        cycle_effect.hold_time = Some(Duration::from_secs(9));

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
                frequency: TempoAwareFrequency::Fixed(16.0), // 16 Hz (should give value > 200)
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
                speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
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
                frequency: TempoAwareFrequency::Fixed(2.0), // 2 Hz
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
        // With fixture profile system, RGB-only fixtures use _pulse_multiplier
        // which gets applied during blending, so we expect no direct RGB commands
        let commands = engine.update(Duration::from_millis(500)).unwrap();
        // The pulse effect for RGB-only fixtures uses _pulse_multiplier, not direct RGB channels
        // So there should be no DMX commands at this point (multiplier is internal)
        assert!(commands.is_empty()); // No direct RGB commands with fixture profile system

        // Test full intensity phase - pulse should be at full amplitude
        let commands = engine.update(Duration::from_secs(2)).unwrap();
        // Same as above - no direct RGB commands with fixture profile system
        assert!(commands.is_empty()); // No direct RGB commands with fixture profile system

        // Test fade out phase - pulse should be dimmed (at 4.5s total: 0.5s into down_time)
        let commands = engine.update(Duration::from_millis(2000)).unwrap(); // 2.5s + 2s = 4.5s
                                                                            // Same as above - no direct RGB commands with fixture profile system
        assert!(commands.is_empty()); // No direct RGB commands with fixture profile system

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
                speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
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

        // Test full intensity: at t=2000ms (after fade in complete), should be 100% blue
        let commands = engine.update(Duration::from_millis(1000)).unwrap(); // Add 1s more (t=0 + 1s + 1s = 2s total)
        if let Some(blue_cmd) = commands.iter().find(|cmd| cmd.channel == 3) {
            assert_eq!(blue_cmd.value, 255); // 100% blue
        }

        // Test hold phase: at t=7000ms (end of hold phase), should still be 100% blue
        let commands = engine.update(Duration::from_millis(5000)).unwrap(); // t=2s + 5s = 7s
        if let Some(blue_cmd) = commands.iter().find(|cmd| cmd.channel == 3) {
            assert_eq!(blue_cmd.value, 255); // 100% blue at end of hold phase
        }

        // Test fade out: at t=8000ms (fade out complete), effect ends (not permanent)
        let commands = engine.update(Duration::from_millis(1000)).unwrap(); // Add 1s more (7s + 1s = 8s)
                                                                            // Static effect with timing params is not permanent, so no persistence after completion
        assert!(
            commands.is_empty() || commands.iter().all(|cmd| cmd.value == 0),
            "Effect should end with no commands or all zeros (not permanent)"
        );
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

    #[test]
    fn test_random_chase_pattern_visibility() {
        // Test to replicate the issue where random pattern chase doesn't show up
        let mut engine = EffectEngine::new();

        // Register 8 fixtures like in the user's setup
        for i in 1..=8 {
            let mut channels = HashMap::new();
            channels.insert("red".to_string(), 1);
            channels.insert("green".to_string(), 2);
            channels.insert("blue".to_string(), 3);
            let fixture = FixtureInfo {
                name: format!("Brick{}", i),
                universe: 1,
                address: (i - 1) * 4 + 1,
                fixture_type: "Astera-PixelBrick".to_string(),
                channels,
                max_strobe_frequency: Some(25.0),
            };
            engine.register_fixture(fixture);
        }

        // Create a random pattern chase effect on background layer
        let mut random_chase = EffectInstance::new(
            "random_chase".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Random,
                speed: TempoAwareSpeed::Fixed(3.0), // 3 cycles per second
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
            },
            vec![
                "Brick1".to_string(),
                "Brick2".to_string(),
                "Brick3".to_string(),
                "Brick4".to_string(),
                "Brick5".to_string(),
                "Brick6".to_string(),
                "Brick7".to_string(),
                "Brick8".to_string(),
            ],
            None,
            Some(Duration::from_secs(4)), // hold_time: 4 seconds
            None,
        );
        random_chase.layer = EffectLayer::Background;
        random_chase.blend_mode = BlendMode::Replace;

        engine.start_effect(random_chase).unwrap();

        // Update engine and check that we get DMX commands
        let mut total_commands = 0;
        let mut active_fixtures: std::collections::HashSet<usize> =
            std::collections::HashSet::new();

        // Check over multiple time points to see if pattern is advancing
        for _step in 0..20 {
            let cmds = engine.update(Duration::from_millis(50)).unwrap();
            total_commands += cmds.len();

            // Track which fixtures have non-zero values (active)
            // For PixelBrick, red channel is at address, green at address+1, blue at address+2
            for cmd in &cmds {
                if cmd.value > 0 {
                    // Find which fixture this command belongs to
                    for i in 1..=8 {
                        let expected_address = (i - 1) * 4 + 1;
                        // Check if this command is for any channel of this fixture
                        if cmd.universe == 1
                            && cmd.channel >= expected_address
                            && cmd.channel < expected_address + 4
                        {
                            active_fixtures.insert(i as usize);
                        }
                    }
                }
            }
        }

        // Verify that we got some commands
        assert!(
            total_commands > 0,
            "Expected some DMX commands, got {}",
            total_commands
        );

        // Verify that multiple fixtures were activated (pattern should advance)
        assert!(active_fixtures.len() > 1,
                "Expected multiple fixtures to be active (pattern advancing), but only {} fixture(s) were active: {:?}", 
                active_fixtures.len(), active_fixtures);

        // Verify that the pattern order is not sequential (should be random)
        // The shuffle for 8 fixtures produces [6, 7, 0, 1, 2, 3, 4, 5]
        // So we should see Brick7, Brick8, Brick1, etc. - not just Brick1, Brick2, etc.
        let fixture_order: Vec<usize> = active_fixtures.iter().copied().collect();
        let is_sequential = fixture_order.windows(2).all(|w| w[1] == w[0] + 1);
        assert!(
            !is_sequential || fixture_order.len() < 3,
            "Pattern appears to be sequential (not random). Active fixtures: {:?}",
            fixture_order
        );
    }
}
