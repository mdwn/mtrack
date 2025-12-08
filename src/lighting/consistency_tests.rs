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
// Consistency tests validating dimmer parity across fixture types and blend modes,
// and timing behavior.

#[cfg(test)]
mod tests {
    use crate::lighting::effects::*;
    use crate::lighting::effects::{CycleTransition, TempoAwareFrequency, TempoAwareSpeed};
    use crate::lighting::engine::EffectEngine;
    use std::collections::HashMap;
    use std::time::Duration;

    fn register_rgb_only_fixture(engine: &mut EffectEngine, name: &str, base_address: u16) {
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), base_address);
        channels.insert("green".to_string(), base_address + 1);
        channels.insert("blue".to_string(), base_address + 2);
        let fixture = FixtureInfo {
            name: name.to_string(),
            universe: 1,
            address: base_address,
            channels,
            fixture_type: "RGB_Par".to_string(),
            max_strobe_frequency: None,
        };
        engine.register_fixture(fixture);
    }

    fn register_dedicated_dimmer_fixture(engine: &mut EffectEngine, name: &str, base_address: u16) {
        let mut channels = HashMap::new();
        channels.insert("dimmer".to_string(), base_address);
        channels.insert("red".to_string(), base_address + 1);
        channels.insert("green".to_string(), base_address + 2);
        channels.insert("blue".to_string(), base_address + 3);
        let fixture = FixtureInfo {
            name: name.to_string(),
            universe: 1,
            address: base_address,
            channels,
            fixture_type: "RGB_Par_Dimmer".to_string(),
            max_strobe_frequency: None,
        };
        engine.register_fixture(fixture);
    }

    fn start_static_rgb(
        engine: &mut EffectEngine,
        name: &str,
        r: f64,
        g: f64,
        b: f64,
        layer: EffectLayer,
    ) {
        let mut params = HashMap::new();
        params.insert("red".to_string(), r);
        params.insert("green".to_string(), g);
        params.insert("blue".to_string(), b);
        let mut eff = EffectInstance::new(
            format!("static_{}", name),
            EffectType::Static {
                parameters: params,
                duration: None,
            },
            vec![name.to_string()],
            None,
            None,
            None,
        );
        eff.layer = layer;
        eff.blend_mode = BlendMode::Replace;
        engine.start_effect(eff).unwrap();
    }

    fn start_dimmer(
        engine: &mut EffectEngine,
        name: &str,
        start: f64,
        end: f64,
        duration: Duration,
        layer: EffectLayer,
        blend: BlendMode,
    ) {
        let mut eff = EffectInstance::new(
            format!("dimmer_{}", name),
            EffectType::Dimmer {
                start_level: start,
                end_level: end,
                duration,
                curve: DimmerCurve::Linear,
            },
            vec![name.to_string()],
            None,
            None,
            None,
        );
        eff.layer = layer;
        eff.blend_mode = blend;
        engine.start_effect(eff).unwrap();
    }

    fn get_rgb(universe: u16, base: u16, cmds: &[DmxCommand]) -> (u8, u8, u8) {
        let r = cmds
            .iter()
            .find(|c| c.universe == universe && c.channel == base)
            .map(|c| c.value)
            .unwrap_or(0);
        let g = cmds
            .iter()
            .find(|c| c.universe == universe && c.channel == base + 1)
            .map(|c| c.value)
            .unwrap_or(0);
        let b = cmds
            .iter()
            .find(|c| c.universe == universe && c.channel == base + 2)
            .map(|c| c.value)
            .unwrap_or(0);
        (r, g, b)
    }

    fn get_dimmer(universe: u16, ch: u16, cmds: &[DmxCommand]) -> u8 {
        cmds.iter()
            .find(|c| c.universe == universe && c.channel == ch)
            .map(|c| c.value)
            .unwrap_or(255)
    }

    #[test]
    fn test_dimmer_parity_rgb_only_vs_dedicated() {
        // Static color with non-trivial ratios
        // RGB-only fixture
        let mut eng_rgb = EffectEngine::new();
        register_rgb_only_fixture(&mut eng_rgb, "fx_rgb", 1);
        start_static_rgb(
            &mut eng_rgb,
            "fx_rgb",
            1.0,
            0.5,
            0.25,
            EffectLayer::Background,
        );
        eng_rgb.update(Duration::from_millis(10), None).unwrap();

        // Dedicated dimmer fixture
        let mut eng_dim = EffectEngine::new();
        register_dedicated_dimmer_fixture(&mut eng_dim, "fx_dim", 1);
        start_static_rgb(
            &mut eng_dim,
            "fx_dim",
            1.0,
            0.5,
            0.25,
            EffectLayer::Background,
        );
        eng_dim.update(Duration::from_millis(10), None).unwrap();

        // Case 1: Multiply blend dimmer 1.0 -> 0.0 over 2s (midpoint = 1s)
        start_dimmer(
            &mut eng_rgb,
            "fx_rgb",
            1.0,
            0.0,
            Duration::from_secs(2),
            EffectLayer::Foreground,
            BlendMode::Multiply,
        );
        start_dimmer(
            &mut eng_dim,
            "fx_dim",
            1.0,
            0.0,
            Duration::from_secs(2),
            EffectLayer::Foreground,
            BlendMode::Multiply,
        );

        let cmds_rgb_1s = eng_rgb.update(Duration::from_secs(1), None).unwrap();
        let cmds_dim_1s = eng_dim.update(Duration::from_secs(1), None).unwrap();

        // For Multiply, both implementations should yield identical effective brightness
        // RGB-only fixture bakes multiplier into RGB; dedicated uses dimmer channel
        let (r_rgb, g_rgb, b_rgb) = get_rgb(1, 1, &cmds_rgb_1s);
        let (r_dim, g_dim, b_dim) = get_rgb(1, 2, &cmds_dim_1s); // base+1 because dimmer occupies ch1
        let d_dim = get_dimmer(1, 1, &cmds_dim_1s) as f32 / 255.0;
        let r_dim_eff = ((r_dim as f32) * d_dim).round() as u8;
        let g_dim_eff = ((g_dim as f32) * d_dim).round() as u8;
        let b_dim_eff = ((b_dim as f32) * d_dim).round() as u8;
        assert_eq!((r_rgb, g_rgb, b_rgb), (r_dim_eff, g_dim_eff, b_dim_eff));

        // Case 2: Replace blend dimmer 1.0 -> 0.0 over 2s (midpoint = 1s)
        let mut eng_rgb_r = EffectEngine::new();
        register_rgb_only_fixture(&mut eng_rgb_r, "fx_rgb", 1);
        start_static_rgb(
            &mut eng_rgb_r,
            "fx_rgb",
            1.0,
            0.5,
            0.25,
            EffectLayer::Background,
        );
        eng_rgb_r.update(Duration::from_millis(10), None).unwrap();
        start_dimmer(
            &mut eng_rgb_r,
            "fx_rgb",
            1.0,
            0.0,
            Duration::from_secs(2),
            EffectLayer::Foreground,
            BlendMode::Replace,
        );

        let mut eng_dim_r = EffectEngine::new();
        register_dedicated_dimmer_fixture(&mut eng_dim_r, "fx_dim", 1);
        start_static_rgb(
            &mut eng_dim_r,
            "fx_dim",
            1.0,
            0.5,
            0.25,
            EffectLayer::Background,
        );
        eng_dim_r.update(Duration::from_millis(10), None).unwrap();
        start_dimmer(
            &mut eng_dim_r,
            "fx_dim",
            1.0,
            0.0,
            Duration::from_secs(2),
            EffectLayer::Foreground,
            BlendMode::Replace,
        );

        let cmds_rgb_r_1s = eng_rgb_r.update(Duration::from_secs(1), None).unwrap();
        let cmds_dim_r_1s = eng_dim_r.update(Duration::from_secs(1), None).unwrap();

        // Visual equivalence: effective RGB should match between the two strategies
        let (r1, g1, b1) = get_rgb(1, 1, &cmds_rgb_r_1s);
        let (r2_raw, g2_raw, b2_raw) = get_rgb(1, 2, &cmds_dim_r_1s);
        let d2 = get_dimmer(1, 1, &cmds_dim_r_1s) as f32 / 255.0;
        let r2 = ((r2_raw as f32) * d2).round() as u8;
        let g2 = ((g2_raw as f32) * d2).round() as u8;
        let b2 = ((b2_raw as f32) * d2).round() as u8;
        assert_eq!((r1, g1, b1), (r2, g2, b2));
    }

    #[test]
    fn test_dimmer_monotonic_and_timing() {
        let mut eng = EffectEngine::new();
        register_rgb_only_fixture(&mut eng, "fx", 1);
        start_static_rgb(&mut eng, "fx", 0.0, 0.0, 1.0, EffectLayer::Background);
        eng.update(Duration::from_millis(10), None).unwrap();

        // 2s fade to black - dimmer effects are permanent, so final value persists
        start_dimmer(
            &mut eng,
            "fx",
            1.0,
            0.0,
            Duration::from_secs(2),
            EffectLayer::Foreground,
            BlendMode::Multiply,
        );

        // t=0
        let cmds_0 = eng.update(Duration::from_millis(0), None).unwrap();
        let (_, _, b0) = get_rgb(1, 1, &cmds_0);
        assert_eq!(b0, 255);

        // t=1.0s -> ~50%
        let cmds_1 = eng.update(Duration::from_secs(1), None).unwrap();
        let (_, _, b1) = get_rgb(1, 1, &cmds_1);
        assert!((120..=135).contains(&b1));

        // t=1.5s -> ~25%
        let cmds_15 = eng.update(Duration::from_millis(500), None).unwrap();
        let (_, _, b15) = get_rgb(1, 1, &cmds_15);
        assert!((50..=75).contains(&b15));

        // effect ends at 2.0s; dimmer effects are permanent so final dimmed value (0) persists
        let _cmds_2 = eng.update(Duration::from_millis(500), None).unwrap();
        let cmds_after = eng.update(Duration::from_millis(10), None).unwrap();
        let (_, _, b_after) = get_rgb(1, 1, &cmds_after);
        assert_eq!(b_after, 0, "Dimmer should persist at 0.0 after completion");
    }

    #[test]
    fn test_color_cycle_parity() {
        let mut eng_rgb = EffectEngine::new();
        register_rgb_only_fixture(&mut eng_rgb, "fx_rgb", 1);
        eng_rgb.update(Duration::from_millis(1), None).unwrap();

        let mut eng_dim = EffectEngine::new();
        register_dedicated_dimmer_fixture(&mut eng_dim, "fx_dim", 1);
        eng_dim.update(Duration::from_millis(1), None).unwrap();

        // Start identical color cycles (3 colors)
        let colors = vec![
            Color::new(255, 0, 0),
            Color::new(0, 255, 0),
            Color::new(0, 0, 255),
        ];
        let mut e1 = EffectInstance::new(
            "cc_rgb".to_string(),
            EffectType::ColorCycle {
                colors: colors.clone(),
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: CycleDirection::Forward,
                transition: CycleTransition::Snap,
            },
            vec!["fx_rgb".to_string()],
            None,
            None,
            None,
        );
        e1.layer = EffectLayer::Foreground;
        e1.blend_mode = BlendMode::Replace;
        eng_rgb.start_effect(e1).unwrap();

        let mut e2 = EffectInstance::new(
            "cc_dim".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: CycleDirection::Forward,
                transition: CycleTransition::Snap,
            },
            vec!["fx_dim".to_string()],
            None,
            None,
            None,
        );
        e2.layer = EffectLayer::Foreground;
        e2.blend_mode = BlendMode::Replace;
        eng_dim.start_effect(e2).unwrap();

        // Sample a few points in time and compare effective RGBs
        for dt in [0u64, 500, 1000, 1500, 2000] {
            let cmds_rgb = eng_rgb.update(Duration::from_millis(dt), None).unwrap();
            let cmds_dim = eng_dim.update(Duration::from_millis(dt), None).unwrap();
            let (r1, g1, b1) = get_rgb(1, 1, &cmds_rgb);
            let (r2, g2, b2) = get_rgb(1, 2, &cmds_dim);
            let d2 = get_dimmer(1, 1, &cmds_dim) as f32 / 255.0;
            let r2e = ((r2 as f32) * d2).round() as u8;
            let g2e = ((g2 as f32) * d2).round() as u8;
            let b2e = ((b2 as f32) * d2).round() as u8;
            assert_eq!((r1, g1, b1), (r2e, g2e, b2e));
        }
    }

    #[test]
    fn test_strobe_parity() {
        let mut eng_rgb = EffectEngine::new();
        register_rgb_only_fixture(&mut eng_rgb, "fx_rgb", 1);
        start_static_rgb(
            &mut eng_rgb,
            "fx_rgb",
            1.0,
            1.0,
            1.0,
            EffectLayer::Background,
        );
        eng_rgb.update(Duration::from_millis(10), None).unwrap();

        let mut eng_dim = EffectEngine::new();
        register_dedicated_dimmer_fixture(&mut eng_dim, "fx_dim", 1);
        start_static_rgb(
            &mut eng_dim,
            "fx_dim",
            1.0,
            1.0,
            1.0,
            EffectLayer::Background,
        );
        eng_dim.update(Duration::from_millis(10), None).unwrap();

        let mut s1 = EffectInstance::new(
            "strobe_rgb".to_string(),
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(10.0),
                duration: Some(Duration::from_secs(2)),
            },
            vec!["fx_rgb".to_string()],
            None,
            None,
            None,
        );
        s1.layer = EffectLayer::Foreground;
        s1.blend_mode = BlendMode::Replace;
        eng_rgb.start_effect(s1).unwrap();

        let mut s2 = EffectInstance::new(
            "strobe_dim".to_string(),
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(10.0),
                duration: Some(Duration::from_secs(2)),
            },
            vec!["fx_dim".to_string()],
            None,
            None,
            None,
        );
        s2.layer = EffectLayer::Foreground;
        s2.blend_mode = BlendMode::Replace;
        eng_dim.start_effect(s2).unwrap();

        // Sample a few points and compare effective RGBs allowing small mismatches
        for dt in [0u64, 50, 100, 150, 200, 500] {
            let cmds_rgb = eng_rgb.update(Duration::from_millis(dt), None).unwrap();
            let cmds_dim = eng_dim.update(Duration::from_millis(dt), None).unwrap();
            let (r1, g1, b1) = get_rgb(1, 1, &cmds_rgb);
            let (r2, g2, b2) = get_rgb(1, 2, &cmds_dim);
            let d2 = get_dimmer(1, 1, &cmds_dim) as f32 / 255.0;
            let r2e = ((r2 as f32) * d2).round() as u8;
            let g2e = ((g2 as f32) * d2).round() as u8;
            let b2e = ((b2 as f32) * d2).round() as u8;
            assert_eq!((r1, g1, b1), (r2e, g2e, b2e));
        }
    }

    #[test]
    fn test_pulse_parity() {
        let mut eng_rgb = EffectEngine::new();
        register_rgb_only_fixture(&mut eng_rgb, "fx_rgb", 1);
        start_static_rgb(
            &mut eng_rgb,
            "fx_rgb",
            0.0,
            0.0,
            1.0,
            EffectLayer::Background,
        );
        eng_rgb.update(Duration::from_millis(10), None).unwrap();

        let mut eng_dim = EffectEngine::new();
        register_dedicated_dimmer_fixture(&mut eng_dim, "fx_dim", 1);
        start_static_rgb(
            &mut eng_dim,
            "fx_dim",
            0.0,
            0.0,
            1.0,
            EffectLayer::Background,
        );
        eng_dim.update(Duration::from_millis(10), None).unwrap();

        let mut p1 = EffectInstance::new(
            "pulse_rgb".to_string(),
            EffectType::Pulse {
                base_level: 0.2,
                pulse_amplitude: 0.8,
                frequency: TempoAwareFrequency::Fixed(1.0),
                duration: Some(Duration::from_secs(2)),
            },
            vec!["fx_rgb".to_string()],
            None,
            None,
            None,
        );
        p1.layer = EffectLayer::Foreground;
        p1.blend_mode = BlendMode::Multiply;
        eng_rgb.start_effect(p1).unwrap();

        let mut p2 = EffectInstance::new(
            "pulse_dim".to_string(),
            EffectType::Pulse {
                base_level: 0.2,
                pulse_amplitude: 0.8,
                frequency: TempoAwareFrequency::Fixed(1.0),
                duration: Some(Duration::from_secs(2)),
            },
            vec!["fx_dim".to_string()],
            None,
            None,
            None,
        );
        p2.layer = EffectLayer::Foreground;
        p2.blend_mode = BlendMode::Multiply;
        eng_dim.start_effect(p2).unwrap();

        for dt in [0u64, 250, 500, 750, 1000] {
            let cmds_rgb = eng_rgb.update(Duration::from_millis(dt), None).unwrap();
            let cmds_dim = eng_dim.update(Duration::from_millis(dt), None).unwrap();
            let (r1, g1, b1) = get_rgb(1, 1, &cmds_rgb);
            let (r2, g2, b2) = get_rgb(1, 2, &cmds_dim);
            let d2 = get_dimmer(1, 1, &cmds_dim) as f32 / 255.0;
            let r2e = ((r2 as f32) * d2).round() as u8;
            let g2e = ((g2 as f32) * d2).round() as u8;
            let b2e = ((b2 as f32) * d2).round() as u8;
            assert_eq!((r1, g1, b1), (r2e, g2e, b2e));
        }
    }

    #[test]
    fn test_chase_parity_basic() {
        let mut eng_rgb = EffectEngine::new();
        register_rgb_only_fixture(&mut eng_rgb, "fx_rgb", 1);
        start_static_rgb(
            &mut eng_rgb,
            "fx_rgb",
            1.0,
            0.0,
            0.0,
            EffectLayer::Background,
        );
        eng_rgb.update(Duration::from_millis(10), None).unwrap();

        let mut eng_dim = EffectEngine::new();
        register_dedicated_dimmer_fixture(&mut eng_dim, "fx_dim", 1);
        start_static_rgb(
            &mut eng_dim,
            "fx_dim",
            1.0,
            0.0,
            0.0,
            EffectLayer::Background,
        );
        eng_dim.update(Duration::from_millis(10), None).unwrap();

        let pattern = ChasePattern::Linear;
        let mut c1 = EffectInstance::new(
            "chase_rgb".to_string(),
            EffectType::Chase {
                pattern: pattern.clone(),
                speed: TempoAwareSpeed::Fixed(2.0),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
            },
            vec!["fx_rgb".to_string()],
            None,
            None,
            None,
        );
        c1.layer = EffectLayer::Foreground;
        c1.blend_mode = BlendMode::Multiply;
        eng_rgb.start_effect(c1).unwrap();

        let mut c2 = EffectInstance::new(
            "chase_dim".to_string(),
            EffectType::Chase {
                pattern,
                speed: TempoAwareSpeed::Fixed(2.0),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
            },
            vec!["fx_dim".to_string()],
            None,
            None,
            None,
        );
        c2.layer = EffectLayer::Foreground;
        c2.blend_mode = BlendMode::Multiply;
        eng_dim.start_effect(c2).unwrap();

        for dt in [0u64, 250, 500, 750, 1000] {
            let cmds_rgb = eng_rgb.update(Duration::from_millis(dt), None).unwrap();
            let cmds_dim = eng_dim.update(Duration::from_millis(dt), None).unwrap();
            let (r1, g1, b1) = get_rgb(1, 1, &cmds_rgb);
            let (r2, g2, b2) = get_rgb(1, 2, &cmds_dim);
            let d2 = get_dimmer(1, 1, &cmds_dim) as f32 / 255.0;
            let r2e = ((r2 as f32) * d2).round() as u8;
            let g2e = ((g2 as f32) * d2).round() as u8;
            let b2e = ((b2 as f32) * d2).round() as u8;
            assert_eq!((r1, g1, b1), (r2e, g2e, b2e));
        }
    }

    #[test]
    fn test_rainbow_parity_basic() {
        let mut eng_rgb = EffectEngine::new();
        register_rgb_only_fixture(&mut eng_rgb, "fx_rgb", 1);
        eng_rgb.update(Duration::from_millis(5), None).unwrap();

        let mut eng_dim = EffectEngine::new();
        register_dedicated_dimmer_fixture(&mut eng_dim, "fx_dim", 1);
        eng_dim.update(Duration::from_millis(5), None).unwrap();

        let mut r1 = EffectInstance::new(
            "rainbow_rgb".to_string(),
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Fixed(1.0),
                saturation: 1.0,
                brightness: 1.0,
            },
            vec!["fx_rgb".to_string()],
            None,
            None,
            None,
        );
        r1.layer = EffectLayer::Foreground;
        r1.blend_mode = BlendMode::Replace;
        eng_rgb.start_effect(r1).unwrap();

        let mut r2 = EffectInstance::new(
            "rainbow_dim".to_string(),
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Fixed(1.0),
                saturation: 1.0,
                brightness: 1.0,
            },
            vec!["fx_dim".to_string()],
            None,
            None,
            None,
        );
        r2.layer = EffectLayer::Foreground;
        r2.blend_mode = BlendMode::Replace;
        eng_dim.start_effect(r2).unwrap();

        for dt in [0u64, 250, 500, 750, 1000] {
            let cmds_rgb = eng_rgb.update(Duration::from_millis(dt), None).unwrap();
            let cmds_dim = eng_dim.update(Duration::from_millis(dt), None).unwrap();
            let (r1, g1, b1) = get_rgb(1, 1, &cmds_rgb);
            let (r2, g2, b2) = get_rgb(1, 2, &cmds_dim);
            let d2 = get_dimmer(1, 1, &cmds_dim) as f32 / 255.0;
            let r2e = ((r2 as f32) * d2).round() as u8;
            let g2e = ((g2 as f32) * d2).round() as u8;
            let b2e = ((b2 as f32) * d2).round() as u8;
            assert_eq!((r1, g1, b1), (r2e, g2e, b2e));
        }
    }

    // Parameterized parity suites (bounded to keep runtime reasonable)
    #[test]
    fn test_param_color_cycle_parity_matrix() {
        let color_sets: Vec<Vec<Color>> = vec![
            vec![Color::new(255, 0, 0), Color::new(0, 255, 0)],
            vec![
                Color::new(255, 0, 0),
                Color::new(0, 255, 0),
                Color::new(0, 0, 255),
            ],
        ];
        let speeds = [0.5, 1.0, 2.0];
        let dirs = [
            CycleDirection::Forward,
            CycleDirection::Backward,
            CycleDirection::PingPong,
        ];
        // Include 500ms to catch PingPong peak edge case (cycle_progress = 0.5)
        let sample_ms = [0u64, 250, 333, 500, 666, 750, 1000];

        for colors in color_sets {
            for speed in speeds {
                for direction in dirs {
                    let mut eng_rgb = EffectEngine::new();
                    register_rgb_only_fixture(&mut eng_rgb, "fx_rgb", 1);
                    eng_rgb.update(Duration::from_millis(1), None).unwrap();
                    let mut eng_dim = EffectEngine::new();
                    register_dedicated_dimmer_fixture(&mut eng_dim, "fx_dim", 1);
                    eng_dim.update(Duration::from_millis(1), None).unwrap();

                    let mut e1 = EffectInstance::new(
                        "cc_rgb_param".to_string(),
                        EffectType::ColorCycle {
                            colors: colors.clone(),
                            speed: TempoAwareSpeed::Fixed(speed),
                            direction,
                            transition: CycleTransition::Snap,
                        },
                        vec!["fx_rgb".to_string()],
                        None,
                        None,
                        None,
                    );
                    e1.layer = EffectLayer::Foreground;
                    e1.blend_mode = BlendMode::Replace;
                    eng_rgb.start_effect(e1).unwrap();

                    let mut e2 = EffectInstance::new(
                        "cc_dim_param".to_string(),
                        EffectType::ColorCycle {
                            colors: colors.clone(),
                            speed: TempoAwareSpeed::Fixed(speed),
                            direction,
                            transition: CycleTransition::Snap,
                        },
                        vec!["fx_dim".to_string()],
                        None,
                        None,
                        None,
                    );
                    e2.layer = EffectLayer::Foreground;
                    e2.blend_mode = BlendMode::Replace;
                    eng_dim.start_effect(e2).unwrap();

                    for dt in sample_ms {
                        let cmds_rgb = eng_rgb.update(Duration::from_millis(dt), None).unwrap();
                        let cmds_dim = eng_dim.update(Duration::from_millis(dt), None).unwrap();
                        let (r1, g1, b1) = get_rgb(1, 1, &cmds_rgb);
                        let (r2, g2, b2) = get_rgb(1, 2, &cmds_dim);
                        let d2 = get_dimmer(1, 1, &cmds_dim) as f32 / 255.0;
                        let r2e = ((r2 as f32) * d2).round() as u8;
                        let g2e = ((g2 as f32) * d2).round() as u8;
                        let b2e = ((b2 as f32) * d2).round() as u8;
                        assert_eq!((r1, g1, b1), (r2e, g2e, b2e));
                    }
                }
            }
        }
    }

    #[test]
    fn test_param_strobe_parity_matrix() {
        let freqs = [1.0, 5.0, 10.0];
        // duty_cycle not supported in effect type; validate across frequencies only
        let sample_ms = [0u64, 50, 100, 250, 500];
        for f in freqs {
            let mut eng_rgb = EffectEngine::new();
            register_rgb_only_fixture(&mut eng_rgb, "fx_rgb", 1);
            start_static_rgb(
                &mut eng_rgb,
                "fx_rgb",
                1.0,
                1.0,
                1.0,
                EffectLayer::Background,
            );
            eng_rgb.update(Duration::from_millis(1), None).unwrap();
            let mut eng_dim = EffectEngine::new();
            register_dedicated_dimmer_fixture(&mut eng_dim, "fx_dim", 1);
            start_static_rgb(
                &mut eng_dim,
                "fx_dim",
                1.0,
                1.0,
                1.0,
                EffectLayer::Background,
            );
            eng_dim.update(Duration::from_millis(1), None).unwrap();
            let mut s1 = EffectInstance::new(
                "strobe_rgb_param".to_string(),
                EffectType::Strobe {
                    frequency: TempoAwareFrequency::Fixed(f),
                    duration: Some(Duration::from_secs(1)),
                },
                vec!["fx_rgb".to_string()],
                None,
                None,
                None,
            );
            s1.layer = EffectLayer::Foreground;
            s1.blend_mode = BlendMode::Replace;
            eng_rgb.start_effect(s1).unwrap();
            let mut s2 = EffectInstance::new(
                "strobe_dim_param".to_string(),
                EffectType::Strobe {
                    frequency: TempoAwareFrequency::Fixed(f),
                    duration: Some(Duration::from_secs(1)),
                },
                vec!["fx_dim".to_string()],
                None,
                None,
                None,
            );
            s2.layer = EffectLayer::Foreground;
            s2.blend_mode = BlendMode::Replace;
            eng_dim.start_effect(s2).unwrap();
            for dt in sample_ms {
                let cmds_rgb = eng_rgb.update(Duration::from_millis(dt), None).unwrap();
                let cmds_dim = eng_dim.update(Duration::from_millis(dt), None).unwrap();
                let (r1, g1, b1) = get_rgb(1, 1, &cmds_rgb);
                let (r2, g2, b2) = get_rgb(1, 2, &cmds_dim);
                let d2 = get_dimmer(1, 1, &cmds_dim) as f32 / 255.0;
                let r2e = ((r2 as f32) * d2).round() as u8;
                let g2e = ((g2 as f32) * d2).round() as u8;
                let b2e = ((b2 as f32) * d2).round() as u8;
                assert_eq!((r1, g1, b1), (r2e, g2e, b2e));
            }
        }
    }

    #[test]
    fn test_param_pulse_parity_matrix() {
        let bases = [0.0, 0.2, 0.5];
        let pulses = [0.5, 1.0];
        let freqs = [0.5, 1.0, 2.0];
        let sample_ms = [0u64, 250, 500, 750, 1000];
        for base in bases {
            for pulse in pulses {
                for freq in freqs {
                    let mut eng_rgb = EffectEngine::new();
                    register_rgb_only_fixture(&mut eng_rgb, "fx_rgb", 1);
                    start_static_rgb(
                        &mut eng_rgb,
                        "fx_rgb",
                        0.0,
                        0.0,
                        1.0,
                        EffectLayer::Background,
                    );
                    eng_rgb.update(Duration::from_millis(1), None).unwrap();
                    let mut eng_dim = EffectEngine::new();
                    register_dedicated_dimmer_fixture(&mut eng_dim, "fx_dim", 1);
                    start_static_rgb(
                        &mut eng_dim,
                        "fx_dim",
                        0.0,
                        0.0,
                        1.0,
                        EffectLayer::Background,
                    );
                    eng_dim.update(Duration::from_millis(1), None).unwrap();
                    let mut p1 = EffectInstance::new(
                        "pulse_rgb_param".to_string(),
                        EffectType::Pulse {
                            base_level: base,
                            pulse_amplitude: pulse,
                            frequency: TempoAwareFrequency::Fixed(freq),
                            duration: Some(Duration::from_secs(1)),
                        },
                        vec!["fx_rgb".to_string()],
                        None,
                        None,
                        None,
                    );
                    p1.layer = EffectLayer::Foreground;
                    p1.blend_mode = BlendMode::Multiply;
                    eng_rgb.start_effect(p1).unwrap();
                    let mut p2 = EffectInstance::new(
                        "pulse_dim_param".to_string(),
                        EffectType::Pulse {
                            base_level: base,
                            pulse_amplitude: pulse,
                            frequency: TempoAwareFrequency::Fixed(freq),
                            duration: Some(Duration::from_secs(1)),
                        },
                        vec!["fx_dim".to_string()],
                        None,
                        None,
                        None,
                    );
                    p2.layer = EffectLayer::Foreground;
                    p2.blend_mode = BlendMode::Multiply;
                    eng_dim.start_effect(p2).unwrap();
                    for dt in sample_ms {
                        let cmds_rgb = eng_rgb.update(Duration::from_millis(dt), None).unwrap();
                        let cmds_dim = eng_dim.update(Duration::from_millis(dt), None).unwrap();
                        let (r1, g1, b1) = get_rgb(1, 1, &cmds_rgb);
                        let (r2, g2, b2) = get_rgb(1, 2, &cmds_dim);
                        let d2 = get_dimmer(1, 1, &cmds_dim) as f32 / 255.0;
                        let r2e = ((r2 as f32) * d2).round() as u8;
                        let g2e = ((g2 as f32) * d2).round() as u8;
                        let b2e = ((b2 as f32) * d2).round() as u8;
                        assert_eq!((r1, g1, b1), (r2e, g2e, b2e));
                    }
                }
            }
        }
    }

    #[test]
    fn test_param_chase_parity_matrix() {
        let patterns: Vec<ChasePattern> = vec![
            ChasePattern::Linear,
            ChasePattern::Snake,
            ChasePattern::Random,
        ];
        let speeds = [0.5, 1.0, 2.0];
        let dirs = [ChaseDirection::LeftToRight, ChaseDirection::RightToLeft];
        let sample_ms = [0u64, 250, 500, 750, 1000];
        for pat in patterns {
            for speed in speeds {
                for dir in dirs {
                    let mut eng_rgb = EffectEngine::new();
                    register_rgb_only_fixture(&mut eng_rgb, "fx_rgb", 1);
                    start_static_rgb(
                        &mut eng_rgb,
                        "fx_rgb",
                        1.0,
                        0.0,
                        0.0,
                        EffectLayer::Background,
                    );
                    eng_rgb.update(Duration::from_millis(1), None).unwrap();
                    let mut eng_dim = EffectEngine::new();
                    register_dedicated_dimmer_fixture(&mut eng_dim, "fx_dim", 1);
                    start_static_rgb(
                        &mut eng_dim,
                        "fx_dim",
                        1.0,
                        0.0,
                        0.0,
                        EffectLayer::Background,
                    );
                    eng_dim.update(Duration::from_millis(1), None).unwrap();
                    let mut c1 = EffectInstance::new(
                        "chase_rgb_param".to_string(),
                        EffectType::Chase {
                            pattern: pat.clone(),
                            speed: TempoAwareSpeed::Fixed(speed),
                            direction: dir,
                            transition: CycleTransition::Snap,
                        },
                        vec!["fx_rgb".to_string()],
                        None,
                        None,
                        None,
                    );
                    c1.layer = EffectLayer::Foreground;
                    c1.blend_mode = BlendMode::Multiply;
                    eng_rgb.start_effect(c1).unwrap();
                    let mut c2 = EffectInstance::new(
                        "chase_dim_param".to_string(),
                        EffectType::Chase {
                            pattern: pat.clone(),
                            speed: TempoAwareSpeed::Fixed(speed),
                            direction: dir,
                            transition: CycleTransition::Snap,
                        },
                        vec!["fx_dim".to_string()],
                        None,
                        None,
                        None,
                    );
                    c2.layer = EffectLayer::Foreground;
                    c2.blend_mode = BlendMode::Multiply;
                    eng_dim.start_effect(c2).unwrap();
                    for dt in sample_ms {
                        let cmds_rgb = eng_rgb.update(Duration::from_millis(dt), None).unwrap();
                        let cmds_dim = eng_dim.update(Duration::from_millis(dt), None).unwrap();
                        let (r1, g1, b1) = get_rgb(1, 1, &cmds_rgb);
                        let (r2, g2, b2) = get_rgb(1, 2, &cmds_dim);
                        let d2 = get_dimmer(1, 1, &cmds_dim) as f32 / 255.0;
                        let r2e = ((r2 as f32) * d2).round() as u8;
                        let g2e = ((g2 as f32) * d2).round() as u8;
                        let b2e = ((b2 as f32) * d2).round() as u8;
                        assert_eq!((r1, g1, b1), (r2e, g2e, b2e));
                    }
                }
            }
        }
    }

    #[test]
    fn test_param_rainbow_parity_matrix() {
        let speeds = [0.5, 1.0, 2.0];
        let saturations = [0.5, 1.0];
        let brightnesses = [0.5, 1.0];
        let sample_ms = [0u64, 250, 500, 750, 1000];
        for speed in speeds {
            for sat in saturations {
                for bri in brightnesses {
                    let mut eng_rgb = EffectEngine::new();
                    register_rgb_only_fixture(&mut eng_rgb, "fx_rgb", 1);
                    eng_rgb.update(Duration::from_millis(1), None).unwrap();
                    let mut eng_dim = EffectEngine::new();
                    register_dedicated_dimmer_fixture(&mut eng_dim, "fx_dim", 1);
                    eng_dim.update(Duration::from_millis(1), None).unwrap();
                    let mut r1 = EffectInstance::new(
                        "rainbow_rgb_param".to_string(),
                        EffectType::Rainbow {
                            speed: TempoAwareSpeed::Fixed(speed),
                            saturation: sat,
                            brightness: bri,
                        },
                        vec!["fx_rgb".to_string()],
                        None,
                        None,
                        None,
                    );
                    r1.layer = EffectLayer::Foreground;
                    r1.blend_mode = BlendMode::Replace;
                    eng_rgb.start_effect(r1).unwrap();
                    let mut r2 = EffectInstance::new(
                        "rainbow_dim_param".to_string(),
                        EffectType::Rainbow {
                            speed: TempoAwareSpeed::Fixed(speed),
                            saturation: sat,
                            brightness: bri,
                        },
                        vec!["fx_dim".to_string()],
                        None,
                        None,
                        None,
                    );
                    r2.layer = EffectLayer::Foreground;
                    r2.blend_mode = BlendMode::Replace;
                    eng_dim.start_effect(r2).unwrap();
                    for dt in sample_ms {
                        let cmds_rgb = eng_rgb.update(Duration::from_millis(dt), None).unwrap();
                        let cmds_dim = eng_dim.update(Duration::from_millis(dt), None).unwrap();
                        let (r1, g1, b1) = get_rgb(1, 1, &cmds_rgb);
                        let (r2, g2, b2) = get_rgb(1, 2, &cmds_dim);
                        let d2 = get_dimmer(1, 1, &cmds_dim) as f32 / 255.0;
                        let r2e = ((r2 as f32) * d2).round() as u8;
                        let g2e = ((g2 as f32) * d2).round() as u8;
                        let b2e = ((b2 as f32) * d2).round() as u8;
                        assert_eq!((r1, g1, b1), (r2e, g2e, b2e));
                    }
                }
            }
        }
    }

    #[test]
    fn test_locks_foreground_replace_with_dimmer_multiply_passthrough() {
        let mut eng = EffectEngine::new();
        register_rgb_only_fixture(&mut eng, "fx", 1);

        // Background blue
        start_static_rgb(&mut eng, "fx", 0.0, 0.0, 1.0, EffectLayer::Background);
        eng.update(Duration::from_millis(10), None).unwrap();

        // Foreground replace static red locks RGB
        let mut fg = EffectInstance::new(
            "fg_lock".to_string(),
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
        fg.layer = EffectLayer::Foreground;
        fg.blend_mode = BlendMode::Replace;
        eng.start_effect(fg).unwrap();

        // Let lock engage
        eng.update(Duration::from_millis(50), None).unwrap();

        // Foreground multiply dimmer - multipliers pass through channel locks
        start_dimmer(
            &mut eng,
            "fx",
            1.0,
            0.0,
            Duration::from_millis(500),
            EffectLayer::Foreground,
            BlendMode::Multiply,
        );

        let cmds_mid = eng.update(Duration::from_millis(250), None).unwrap();
        let (r_mid, _, _) = get_rgb(1, 1, &cmds_mid);
        // At midpoint, red should be dimmed to ~50% (multiplier passes through lock)
        assert!(
            (120..=135).contains(&r_mid),
            "Expected red ~127 at midpoint, got {}",
            r_mid
        );

        // After dimmer completes, final dimmed value (0) persists (dimmers are permanent)
        let cmds_end = eng.update(Duration::from_millis(300), None).unwrap();
        let (r_end, g_end, b_end) = get_rgb(1, 1, &cmds_end);
        assert_eq!(r_end, 0);
        assert_eq!(g_end, 0);
        assert_eq!(b_end, 0);
    }

    #[test]
    fn test_extremes_zero_duration_instant_cut_and_full_duration() {
        let mut eng = EffectEngine::new();
        register_rgb_only_fixture(&mut eng, "fx", 1);
        start_static_rgb(&mut eng, "fx", 0.0, 0.0, 1.0, EffectLayer::Background);
        eng.update(Duration::from_millis(10), None).unwrap();

        // Use near-instant dimmer; engine expires zero/near-zero quickly. Ensure no panic and subsequent long fade behaves.
        start_dimmer(
            &mut eng,
            "fx",
            1.0,
            0.0,
            Duration::from_millis(1),
            EffectLayer::Foreground,
            BlendMode::Multiply,
        );
        let _ = eng.update(Duration::from_millis(2), None).unwrap();

        // Long duration minimal change early on
        start_static_rgb(&mut eng, "fx", 0.0, 0.0, 1.0, EffectLayer::Background);
        eng.update(Duration::from_millis(10), None).unwrap();
        start_dimmer(
            &mut eng,
            "fx",
            1.0,
            0.0,
            Duration::from_secs(60),
            EffectLayer::Foreground,
            BlendMode::Multiply,
        );
        let cmds_100ms = eng.update(Duration::from_millis(100), None).unwrap();
        let (_, _, b_100) = get_rgb(1, 1, &cmds_100ms);
        assert!(b_100 > 240);
    }

    #[test]
    fn test_rainbow_extreme_speeds() {
        let mut eng = EffectEngine::new();
        register_rgb_only_fixture(&mut eng, "fx", 1);
        eng.update(Duration::from_millis(1), None).unwrap();

        // Low speed snapshot
        let mut r_low = EffectInstance::new(
            "r_low".to_string(),
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Fixed(0.1),
                saturation: 1.0,
                brightness: 1.0,
            },
            vec!["fx".to_string()],
            None,
            None,
            None,
        );
        r_low.layer = EffectLayer::Foreground;
        r_low.blend_mode = BlendMode::Replace;
        eng.start_effect(r_low).unwrap();
        let c0 = eng.update(Duration::from_millis(0), None).unwrap();
        let (r0, g0, b0) = get_rgb(1, 1, &c0);

        // High speed snapshot at non-integer multiple of the cycle (period=100ms at 10Hz); use 125ms
        let mut r_high = EffectInstance::new(
            "r_high".to_string(),
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Fixed(10.0),
                saturation: 1.0,
                brightness: 1.0,
            },
            vec!["fx".to_string()],
            None,
            None,
            None,
        );
        r_high.layer = EffectLayer::Foreground;
        r_high.blend_mode = BlendMode::Replace;
        eng.start_effect(r_high).unwrap();
        let c1 = eng.update(Duration::from_millis(125), None).unwrap();
        let (r1, g1, b1) = get_rgb(1, 1, &c1);

        assert_ne!((r0, g0, b0), (r1, g1, b1));
    }
}
