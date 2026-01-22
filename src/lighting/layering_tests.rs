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

#[cfg(test)]
mod blend_mode_tests;
#[cfg(test)]
mod chase_tests;
#[cfg(test)]
mod common;
#[cfg(test)]
mod conflict_resolution_tests;
#[cfg(test)]
mod crossfade_tests;
#[cfg(test)]
mod dimmer_tests;
#[cfg(test)]
mod dsl_parsing_tests;
#[cfg(test)]
mod integration_tests;
#[cfg(test)]
mod static_effect_tests;
#[cfg(test)]
mod strobe_tests;

// Re-export the original modules that were already in the file
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
        let _ = engine.update(Duration::from_millis(1000), None).unwrap();

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
        let _ = engine.update(Duration::from_millis(1), None).unwrap();

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
        let _ = engine.update(Duration::from_millis(1), None).unwrap();

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
        let cmds = engine.update(Duration::from_millis(1), None).unwrap();

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
        let _ = engine.update(Duration::from_millis(1), None).unwrap();

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
        let _ = engine.update(Duration::from_millis(1), None).unwrap();

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
        let _ = engine.update(Duration::from_millis(1), None).unwrap();

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
        let cmds = engine.update(Duration::from_millis(1), None).unwrap();

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
        engine.register_fixture(pixelbrick("front_wash", 1, 1));

        // Background: static blue
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
        let _ = engine.update(Duration::from_millis(1), None).unwrap();

        // Midground: dimmer multiply 50%
        let mut mid_dim = EffectInstance::new(
            "mid_dim".to_string(),
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
        mid_dim.layer = EffectLayer::Midground;
        mid_dim.blend_mode = BlendMode::Multiply;
        engine.start_effect(mid_dim).unwrap();
        let cmds = engine.update(Duration::from_millis(1), None).unwrap();

        let blue_ch = 1 + 3 - 1;
        let blue = cmds
            .iter()
            .find(|c| c.universe == 1 && c.channel == blue_ch)
            .map(|c| c.value)
            .unwrap_or(0);
        assert!((120..=135).contains(&blue), "expected ~127, got {}", blue);
    }

    #[test]
    fn test_mid_replace_overrides_background_replace() {
        let mut engine = EffectEngine::new();
        engine.register_fixture(pixelbrick("front_wash", 1, 1));

        // Background: static blue
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
        let _ = engine.update(Duration::from_millis(1), None).unwrap();

        // Midground: static red (replace)
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
            vec!["front_wash".to_string()],
            None,
            None,
            None,
        );
        mid_red.layer = EffectLayer::Midground;
        mid_red.blend_mode = BlendMode::Replace;
        engine.start_effect(mid_red).unwrap();
        let cmds = engine.update(Duration::from_millis(1), None).unwrap();

        let red_ch = 1 + 1 - 1;
        let red = cmds
            .iter()
            .find(|c| c.universe == 1 && c.channel == red_ch)
            .map(|c| c.value)
            .unwrap_or(0);
        assert_eq!(red, 255);
    }

    #[test]
    fn test_foreground_replace_overrides_mid_replace() {
        let mut engine = EffectEngine::new();
        engine.register_fixture(pixelbrick("front_wash", 1, 1));

        // Background: static blue
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
        let _ = engine.update(Duration::from_millis(1), None).unwrap();

        // Midground: static red
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
            vec!["front_wash".to_string()],
            None,
            None,
            None,
        );
        mid_red.layer = EffectLayer::Midground;
        mid_red.blend_mode = BlendMode::Replace;
        engine.start_effect(mid_red).unwrap();
        let _ = engine.update(Duration::from_millis(1), None).unwrap();

        // Foreground: static green
        let mut fg_green = EffectInstance::new(
            "fg_green".to_string(),
            EffectType::Static {
                parameters: {
                    let mut p = HashMap::new();
                    p.insert("green".to_string(), 1.0);
                    p
                },
                duration: None,
            },
            vec!["front_wash".to_string()],
            None,
            None,
            None,
        );
        fg_green.layer = EffectLayer::Foreground;
        fg_green.blend_mode = BlendMode::Replace;
        engine.start_effect(fg_green).unwrap();
        let cmds = engine.update(Duration::from_millis(1), None).unwrap();

        let green_ch = 1 + 2 - 1;
        let green = cmds
            .iter()
            .find(|c| c.universe == 1 && c.channel == green_ch)
            .map(|c| c.value)
            .unwrap_or(0);
        assert_eq!(green, 255);
    }

    #[test]
    fn test_replace_affects_only_written_channels() {
        let mut engine = EffectEngine::new();
        engine.register_fixture(pixelbrick("front_wash", 1, 1));

        // Background: static blue
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
        let _ = engine.update(Duration::from_millis(1), None).unwrap();

        // Midground: static red (replace) - should only affect red channel
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
            vec!["front_wash".to_string()],
            None,
            None,
            None,
        );
        mid_red.layer = EffectLayer::Midground;
        mid_red.blend_mode = BlendMode::Replace;
        engine.start_effect(mid_red).unwrap();
        let cmds = engine.update(Duration::from_millis(1), None).unwrap();

        let blue_ch = 1 + 3 - 1;
        let blue = cmds
            .iter()
            .find(|c| c.universe == 1 && c.channel == blue_ch)
            .map(|c| c.value)
            .unwrap_or(0);
        assert_eq!(blue, 255, "blue should remain from background");
    }

    #[test]
    fn test_foreground_replace_blocks_mid_multiply_on_same_channel() {
        let mut engine = EffectEngine::new();
        engine.register_fixture(pixelbrick("front_wash", 1, 1));

        // Background: static blue
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
        let _ = engine.update(Duration::from_millis(1), None).unwrap();

        // Foreground: static red (replace)
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
            vec!["front_wash".to_string()],
            None,
            None,
            None,
        );
        fg_red.layer = EffectLayer::Foreground;
        fg_red.blend_mode = BlendMode::Replace;
        engine.start_effect(fg_red).unwrap();
        let _ = engine.update(Duration::from_millis(1), None).unwrap();

        // Midground: dimmer multiply 50% (should not affect red channel locked by foreground)
        let mut mid_dim = EffectInstance::new(
            "mid_dim".to_string(),
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
        mid_dim.layer = EffectLayer::Midground;
        mid_dim.blend_mode = BlendMode::Multiply;
        engine.start_effect(mid_dim).unwrap();
        let cmds = engine.update(Duration::from_millis(1), None).unwrap();

        let red_ch = 1 + 1 - 1;
        let red = cmds
            .iter()
            .find(|c| c.universe == 1 && c.channel == red_ch)
            .map(|c| c.value)
            .unwrap_or(0);
        assert_eq!(
            red, 255,
            "red should remain at 255 (not dimmed by midground)"
        );
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
            None,
        );
        static_blue.layer = EffectLayer::Background;
        static_blue.blend_mode = BlendMode::Replace;
        engine.start_effect(static_blue).unwrap();
        let _ = engine.update(Duration::from_millis(1), None).unwrap();

        // Midground dimmer to 50% (instant, permanent)
        let mut dimmer_50 = EffectInstance::new(
            "mid_dim_50".to_string(),
            EffectType::Dimmer {
                start_level: 0.5,
                end_level: 0.5,
                duration: Duration::from_millis(0),
                curve: DimmerCurve::Linear,
            },
            targets,
            None,
            None,
            None,
        );
        dimmer_50.layer = EffectLayer::Midground;
        dimmer_50.blend_mode = BlendMode::Multiply;
        engine.start_effect(dimmer_50).unwrap();
        let cmds = engine.update(Duration::from_millis(1), None).unwrap();

        // Expect blue = 127 (50% of 255) for all fixtures
        for i in 0..8 {
            let blue_ch = 1 + (i as u16) * 4 + 3 - 1;
            let blue = cmds
                .iter()
                .find(|c| c.universe == 1 && c.channel == blue_ch)
                .map(|c| c.value)
                .unwrap_or(0);
            assert!(
                (120..=135).contains(&blue),
                "fixture {} expected ~127, got {}",
                i + 1,
                blue
            );
        }
    }
}
