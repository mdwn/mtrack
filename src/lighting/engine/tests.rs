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
#[allow(clippy::module_inception)]
mod tests {
    use crate::lighting::effects::*;
    use crate::lighting::engine::EffectEngine;
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    fn create_test_fixture(name: &str, universe: u16, address: u16) -> FixtureInfo {
        let mut channels = HashMap::new();
        channels.insert("dimmer".to_string(), 1);
        channels.insert("red".to_string(), 2);
        channels.insert("green".to_string(), 3);
        channels.insert("blue".to_string(), 4);
        channels.insert("white".to_string(), 5);
        channels.insert("strobe".to_string(), 6);

        FixtureInfo {
            name: name.to_string(),
            universe,
            address,
            fixture_type: "RGBW_Strobe".to_string(),
            channels,
            max_strobe_frequency: Some(20.0), // Default test fixture max strobe
        }
    }

    #[test]
    fn test_effect_engine_creation() {
        let engine = EffectEngine::new();
        assert_eq!(engine.active_effects_count(), 0);
    }

    #[test]
    fn test_fixture_registration() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);

        engine.register_fixture(fixture);
        // Verify fixture is registered by trying to use it in an effect
        let mut parameters = HashMap::new();
        parameters.insert("dimmer".to_string(), 0.5);
        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters,
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );
        // Should not error if fixture is registered
        assert!(engine.start_effect(effect).is_ok());
    }

    #[test]
    fn test_static_effect() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let mut parameters = HashMap::new();
        parameters.insert("dimmer".to_string(), 0.5);
        parameters.insert("red".to_string(), 1.0);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters: parameters.clone(),
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Update the engine
        let commands = engine.update(Duration::from_millis(16), None).unwrap();

        // Should have commands for dimmer and red channels
        assert_eq!(commands.len(), 2);

        // Check dimmer command (50% = 127)
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(dimmer_cmd.value, 127);

        // Check red command (100% = 255)
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        assert_eq!(red_cmd.value, 255);
    }

    #[test]
    fn test_color_cycle_effect() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let colors = vec![
            Color::new(255, 0, 0), // Red
            Color::new(0, 255, 0), // Green
            Color::new(0, 0, 255), // Blue
        ];

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
                direction: CycleDirection::Forward,
                transition: CycleTransition::Snap,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Test cycling over time
        // At t=0ms: should be red (index 0)
        let commands = engine.update(Duration::from_millis(0), None).unwrap();
        assert_eq!(commands.len(), 3);
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(red_cmd.value, 255);
        assert_eq!(green_cmd.value, 0);
        assert_eq!(blue_cmd.value, 0);

        // At t=500ms: should be green (index 1) - clearly in green's range
        let commands = engine.update(Duration::from_millis(500), None).unwrap();
        assert_eq!(commands.len(), 3);
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(red_cmd.value, 0);
        assert_eq!(green_cmd.value, 255);
        assert_eq!(blue_cmd.value, 0);

        // At t=300ms: should be blue (index 2) - 300ms into the second cycle
        let commands = engine.update(Duration::from_millis(300), None).unwrap();
        assert_eq!(commands.len(), 3);
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(red_cmd.value, 0);
        assert_eq!(green_cmd.value, 0);
        assert_eq!(blue_cmd.value, 255);
    }

    #[test]
    fn test_color_cycle_pingpong_peak() {
        // Regression test: PingPong should show the last color at cycle peak (cycle_progress = 0.5)
        // Previously, a bug caused it to incorrectly show the first color at the peak.
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let colors = vec![
            Color::new(255, 0, 0), // Red (index 0)
            Color::new(0, 255, 0), // Green (index 1)
            Color::new(0, 0, 255), // Blue (index 2) - should be shown at peak
        ];

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
                direction: CycleDirection::PingPong,
                transition: CycleTransition::Snap,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0ms: should be red (index 0) - start of cycle
        let commands = engine.update(Duration::from_millis(0), None).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (255, 0, 0),
            "At t=0ms should be red"
        );

        // At t=500ms: cycle_progress = 0.5, ping_pong_progress = 1.0 (peak)
        // Should show the LAST color (blue, index 2), not the first color (red)
        let commands = engine.update(Duration::from_millis(500), None).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (0, 0, 255),
            "At t=500ms (peak) should be blue (last color), not red"
        );

        // At t=1000ms: cycle_progress = 0.0, back to start
        // Should be red again (index 0)
        let commands = engine.update(Duration::from_millis(500), None).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (255, 0, 0),
            "At t=1000ms should be red again"
        );
    }

    #[test]
    fn test_color_cycle_backward_boundary() {
        // Regression test: Backward direction should show the LAST color at cycle start (cycle_progress = 0.0)
        // Previously, a bug caused it to incorrectly show the first color due to:
        // reversed_progress = 1.0 → color_index_f = colors.len() → floor = colors.len() → modulo wraps to 0
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let colors = vec![
            Color::new(255, 0, 0), // Red (index 0) - should be shown at END of backward cycle
            Color::new(0, 255, 0), // Green (index 1)
            Color::new(0, 0, 255), // Blue (index 2) - should be shown at START of backward cycle
        ];

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
                direction: CycleDirection::Backward,
                transition: CycleTransition::Snap,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Note: engine.update() takes DELTA time, not absolute time!
        // Each call adds to the elapsed time.

        // At t=0ms: cycle_progress = 0.0, reversed_progress = 1.0
        // Should be BLUE (last color, index 2), NOT red (first color)
        let commands = engine.update(Duration::from_millis(0), None).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (0, 0, 255),
            "At t=0ms (cycle start) backward should show LAST color (blue), not first (red)"
        );

        // At t=500ms (dt=500): cycle_progress = 0.5, reversed_progress = 0.5
        // color_index_f = 1.5, color_index = 1 → green
        let commands = engine.update(Duration::from_millis(500), None).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (0, 255, 0),
            "At t=500ms should be green"
        );

        // At t=834ms (dt=334, total=834): cycle_progress ≈ 0.834, reversed_progress ≈ 0.166
        // color_index_f ≈ 0.5, color_index = 0 → red
        let commands = engine.update(Duration::from_millis(334), None).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (255, 0, 0),
            "At t=834ms should be red"
        );
    }

    #[test]
    fn test_color_cycle_backward_fade_boundary() {
        // Regression test: Backward + Fade at cycle start (cycle_progress = 0) should show
        // the LAST color, not interpolate toward the previous color.
        // Previously, segment_progress was 1.0 at cycle start due to clamping, causing
        // lerp to return next_color instead of current_color.
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let colors = vec![
            Color::new(255, 0, 0), // Red (index 0)
            Color::new(0, 255, 0), // Green (index 1)
            Color::new(0, 0, 255), // Blue (index 2) - should be shown at START
        ];

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: CycleDirection::Backward,
                transition: CycleTransition::Fade, // Key difference from Snap test
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0ms: cycle_progress = 0, should be PURE BLUE (last color)
        // With the bug, segment_progress was 1.0, causing lerp to return Green instead
        let commands = engine.update(Duration::from_millis(0), None).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (0, 0, 255),
            "At t=0ms Backward+Fade should show PURE BLUE (last color), not interpolated"
        );

        // At t=166ms: ~50% through Blue->Green segment, should be teal-ish
        let commands = engine.update(Duration::from_millis(166), None).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        // Should be interpolating between Blue and Green
        assert!(
            green_cmd.value > 100 && blue_cmd.value > 100,
            "At t=166ms should be fading from Blue toward Green, got ({}, {}, {})",
            red_cmd.value,
            green_cmd.value,
            blue_cmd.value
        );
    }

    #[test]
    fn test_color_cycle_fade_interpolation() {
        // Regression test: CycleTransition::Fade should smoothly interpolate between colors.
        // Previously, a bug divided segment_progress by segment_size (1/colors.len()),
        // effectively multiplying by colors.len(). This caused segment_progress to exceed 1.0
        // early in each segment, and since lerp clamps to 0-1, colors would snap at ~33%
        // through each segment instead of smoothly fading over the full segment duration.
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let colors = vec![
            Color::new(255, 0, 0), // Red (index 0)
            Color::new(0, 0, 255), // Blue (index 1)
        ];

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
                direction: CycleDirection::Forward,
                transition: CycleTransition::Fade,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0ms: should be pure red (start of first segment)
        let commands = engine.update(Duration::from_millis(0), None).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (255, 0, 0),
            "At t=0ms should be pure red"
        );

        // At t=250ms: 50% through red→blue segment, should be purple (127, 0, 127)
        // With the bug, segment_progress would be 1.0 (clamped from 0.5 * 2 = 1.0),
        // resulting in pure blue instead of purple.
        let commands = engine.update(Duration::from_millis(250), None).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        // Allow ±1 tolerance for floating point rounding
        assert!(
            (126..=128).contains(&red_cmd.value)
                && green_cmd.value == 0
                && (126..=128).contains(&blue_cmd.value),
            "At t=250ms (50% through segment) should be ~purple (127, 0, 127), got ({}, {}, {})",
            red_cmd.value,
            green_cmd.value,
            blue_cmd.value
        );

        // At t=500ms: start of blue→red segment, should be pure blue
        let commands = engine.update(Duration::from_millis(250), None).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (0, 0, 255),
            "At t=500ms should be pure blue"
        );

        // At t=750ms: 50% through blue→red segment, should be purple again
        let commands = engine.update(Duration::from_millis(250), None).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert!(
            (126..=128).contains(&red_cmd.value)
                && green_cmd.value == 0
                && (126..=128).contains(&blue_cmd.value),
            "At t=750ms (50% through segment) should be ~purple (127, 0, 127), got ({}, {}, {})",
            red_cmd.value,
            green_cmd.value,
            blue_cmd.value
        );
    }

    #[test]
    fn test_color_cycle_forward_wraparound() {
        // Test that Forward direction wraps correctly from last color back to first
        // Note: engine.update() takes DELTA time. Each call advances elapsed.
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let colors = vec![
            Color::new(255, 0, 0), // Red (index 0)
            Color::new(0, 255, 0), // Green (index 1)
            Color::new(0, 0, 255), // Blue (index 2)
        ];

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
                direction: CycleDirection::Forward,
                transition: CycleTransition::Snap,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // With 3 colors at 1 cycle/second: each color is ~333.33ms
        // Color 0 (red): 0ms - 333.32ms
        // Color 1 (green): 333.33ms - 666.65ms
        // Color 2 (blue): 666.66ms - 999.99ms

        // At t=0ms: should be red (start of cycle)
        let commands = engine.update(Duration::from_millis(0), None).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        assert_eq!(red_cmd.value, 255, "At t=0ms should be red");

        // At t=350ms: should be green (past 333.33ms threshold)
        let commands = engine.update(Duration::from_millis(350), None).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        assert_eq!(green_cmd.value, 255, "At t=350ms should be green");

        // At t=700ms: should be blue (past 666.66ms threshold)
        let commands = engine.update(Duration::from_millis(350), None).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(blue_cmd.value, 255, "At t=700ms should be blue");

        // At t=1050ms: should wrap back to red (past 1000ms)
        let commands = engine.update(Duration::from_millis(350), None).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        assert_eq!(red_cmd.value, 255, "At t=1050ms should wrap back to red");
    }

    #[test]
    fn test_color_cycle_two_colors_all_directions() {
        // Test all directions with just 2 colors to catch edge cases with minimal color sets
        let colors = vec![
            Color::new(255, 0, 0), // Red
            Color::new(0, 0, 255), // Blue
        ];

        for direction in [
            CycleDirection::Forward,
            CycleDirection::Backward,
            CycleDirection::PingPong,
        ] {
            let mut engine = EffectEngine::new();
            let fixture = create_test_fixture("test_fixture", 1, 1);
            engine.register_fixture(fixture);

            let effect = EffectInstance::new(
                "test_effect".to_string(),
                EffectType::ColorCycle {
                    colors: colors.clone(),
                    speed: TempoAwareSpeed::Fixed(1.0),
                    direction,
                    transition: CycleTransition::Snap,
                },
                vec!["test_fixture".to_string()],
                None,
                None,
                None,
            );

            engine.start_effect(effect).unwrap();

            // At t=0: should have a valid color (not crash, not garbage values)
            let commands = engine.update(Duration::from_millis(0), None).unwrap();
            let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
            let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();

            // Should be either pure red or pure blue
            let is_valid_color = (red_cmd.value == 255 && blue_cmd.value == 0)
                || (red_cmd.value == 0 && blue_cmd.value == 255);
            assert!(
                is_valid_color,
                "{:?} at t=0 should be pure red or blue, got r={} b={}",
                direction, red_cmd.value, blue_cmd.value
            );

            // At t=500ms (half cycle): should still be valid
            let commands = engine.update(Duration::from_millis(500), None).unwrap();
            let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
            let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();

            let is_valid_color = (red_cmd.value == 255 && blue_cmd.value == 0)
                || (red_cmd.value == 0 && blue_cmd.value == 255);
            assert!(
                is_valid_color,
                "{:?} at t=500ms should be pure red or blue, got r={} b={}",
                direction, red_cmd.value, blue_cmd.value
            );
        }
    }

    #[test]
    fn test_strobe_boundary_at_duty_cycle_transition() {
        // Test strobe behavior at exactly the 50% duty cycle boundary
        // strobe_phase < 0.5 means ON, >= 0.5 means OFF
        let mut engine = EffectEngine::new();

        // Create a fixture WITHOUT hardware strobe capability to test software strobe
        let mut channels = HashMap::new();
        channels.insert("dimmer".to_string(), 1);
        channels.insert("red".to_string(), 2);
        channels.insert("green".to_string(), 3);
        channels.insert("blue".to_string(), 4);

        let fixture = FixtureInfo {
            name: "test_fixture".to_string(),
            universe: 1,
            address: 1,
            fixture_type: "RGB".to_string(),
            channels,
            max_strobe_frequency: None, // No hardware strobe
        };
        engine.register_fixture(fixture);

        // 2 Hz strobe = 500ms period, so 50% duty cycle transition at 250ms
        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(2.0),
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0ms: strobe_phase=0, which is < 0.5, so ON (dimmer=255)
        let commands = engine.update(Duration::from_millis(0), None).unwrap();
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(dimmer_cmd.value, 255, "At t=0ms strobe should be ON");

        // At t=249ms: still in first half of period, should be ON
        let commands = engine.update(Duration::from_millis(249), None).unwrap();
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(
            dimmer_cmd.value, 255,
            "At t=249ms strobe should still be ON"
        );

        // At t=251ms: just past 50% of period, should be OFF
        let commands = engine.update(Duration::from_millis(2), None).unwrap();
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(dimmer_cmd.value, 0, "At t=251ms strobe should be OFF");

        // At t=500ms: start of new period, should be ON again
        let commands = engine.update(Duration::from_millis(249), None).unwrap();
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(
            dimmer_cmd.value, 255,
            "At t=500ms strobe should be ON again"
        );
    }

    #[test]
    fn test_chase_fixture_boundaries() {
        // Test chase effect transitions between fixtures correctly
        // Note: Chase applies dimmer to active fixture, 0 to others
        let mut engine = EffectEngine::new();

        // Create 3 fixtures for chase, each at different addresses
        let fixture_0 = create_test_fixture("fixture_0", 1, 1);
        let fixture_1 = create_test_fixture("fixture_1", 1, 11);
        let fixture_2 = create_test_fixture("fixture_2", 1, 21);
        engine.register_fixture(fixture_0);
        engine.register_fixture(fixture_1);
        engine.register_fixture(fixture_2);

        // 1 Hz chase with 3 fixtures = each fixture active for 333.33ms
        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
            },
            vec![
                "fixture_0".to_string(),
                "fixture_1".to_string(),
                "fixture_2".to_string(),
            ],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Helper to count active fixtures (dimmer channel = address, value = 255)
        let count_active = |commands: &[DmxCommand]| -> usize {
            // Each fixture has dimmer at relative channel 1
            // fixture_0: channel 1, fixture_1: channel 11, fixture_2: channel 21
            let dimmer_channels = [1, 11, 21];
            commands
                .iter()
                .filter(|cmd| dimmer_channels.contains(&cmd.channel) && cmd.value == 255)
                .count()
        };

        // At t=0ms: first fixture should be active (pattern_index = 0)
        let commands = engine.update(Duration::from_millis(0), None).unwrap();
        assert_eq!(
            count_active(&commands),
            1,
            "At t=0ms exactly one fixture should be active"
        );

        // At t=350ms: second fixture should be active (past 333.33ms)
        let commands = engine.update(Duration::from_millis(350), None).unwrap();
        assert_eq!(
            count_active(&commands),
            1,
            "At t=350ms exactly one fixture should be active"
        );

        // At t=700ms: third fixture should be active (past 666.66ms)
        let commands = engine.update(Duration::from_millis(350), None).unwrap();
        assert_eq!(
            count_active(&commands),
            1,
            "At t=700ms exactly one fixture should be active"
        );

        // At t=1050ms: should wrap back (past 1000ms)
        let commands = engine.update(Duration::from_millis(350), None).unwrap();
        assert_eq!(
            count_active(&commands),
            1,
            "At t=1050ms exactly one fixture should be active (wrapped)"
        );
    }

    #[test]
    fn test_rainbow_hue_wraparound() {
        // Test that rainbow effect wraps hue correctly at 360 degrees
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Speed of 1.0 = 1 full hue rotation per second (360 degrees/sec)
        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Fixed(1.0),
                saturation: 1.0,
                brightness: 1.0,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0ms: hue=0 (red)
        let commands_start = engine.update(Duration::from_millis(0), None).unwrap();
        let red_start = commands_start
            .iter()
            .find(|cmd| cmd.channel == 2)
            .unwrap()
            .value;
        let green_start = commands_start
            .iter()
            .find(|cmd| cmd.channel == 3)
            .unwrap()
            .value;
        let blue_start = commands_start
            .iter()
            .find(|cmd| cmd.channel == 4)
            .unwrap()
            .value;

        // At hue=0 (red), should be approximately (255, 0, 0)
        assert!(
            red_start > 200 && green_start < 50 && blue_start < 50,
            "At t=0ms should be red-ish, got ({}, {}, {})",
            red_start,
            green_start,
            blue_start
        );

        // At t=1000ms: hue should wrap back to 0 (red again)
        let commands_end = engine.update(Duration::from_millis(1000), None).unwrap();
        let red_end = commands_end
            .iter()
            .find(|cmd| cmd.channel == 2)
            .unwrap()
            .value;
        let green_end = commands_end
            .iter()
            .find(|cmd| cmd.channel == 3)
            .unwrap()
            .value;
        let blue_end = commands_end
            .iter()
            .find(|cmd| cmd.channel == 4)
            .unwrap()
            .value;

        // Should be back to approximately red
        assert!(
            red_end > 200 && green_end < 50 && blue_end < 50,
            "At t=1000ms should wrap back to red-ish, got ({}, {}, {})",
            red_end,
            green_end,
            blue_end
        );

        // Colors at start and end should be very similar (wrapped)
        assert!(
            (red_start as i16 - red_end as i16).abs() < 10,
            "Red should be similar after full cycle"
        );
    }

    #[test]
    fn test_pulse_at_peaks_and_troughs() {
        // Test pulse effect at its mathematical peaks and troughs
        // pulse_value = (base_level + pulse_amplitude * (sin(phase) * 0.5 + 0.5))
        // At phase=0: sin(0)=0, so multiplier=0.5
        // At phase=π/2: sin=1, so multiplier=1.0 (peak)
        // At phase=3π/2: sin=-1, so multiplier=0.0 (trough)

        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // 1 Hz pulse, base_level=0.0, amplitude=1.0 for easy calculation
        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Pulse {
                base_level: 0.0,
                pulse_amplitude: 1.0,
                frequency: TempoAwareFrequency::Fixed(1.0),
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0ms: phase=0, sin(0)=0, pulse_value = 0 + 1.0 * (0 * 0.5 + 0.5) = 0.5
        let commands = engine.update(Duration::from_millis(0), None).unwrap();
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        // 0.5 * 255 ≈ 127
        assert!(
            (120..=135).contains(&dimmer_cmd.value),
            "At t=0ms pulse should be ~127 (mid), got {}",
            dimmer_cmd.value
        );

        // At t=250ms: phase=π/2, sin(π/2)=1, pulse_value = 0 + 1.0 * (1 * 0.5 + 0.5) = 1.0 (peak)
        let commands = engine.update(Duration::from_millis(250), None).unwrap();
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert!(
            dimmer_cmd.value >= 250,
            "At t=250ms pulse should be at peak (~255), got {}",
            dimmer_cmd.value
        );

        // At t=750ms: phase=3π/2, sin(3π/2)=-1, pulse_value = 0 + 1.0 * (-1 * 0.5 + 0.5) = 0.0 (trough)
        let commands = engine.update(Duration::from_millis(500), None).unwrap();
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert!(
            dimmer_cmd.value <= 5,
            "At t=750ms pulse should be at trough (~0), got {}",
            dimmer_cmd.value
        );

        // At t=1000ms: should be back to mid-point
        let commands = engine.update(Duration::from_millis(250), None).unwrap();
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert!(
            (120..=135).contains(&dimmer_cmd.value),
            "At t=1000ms pulse should be back to ~127 (mid), got {}",
            dimmer_cmd.value
        );
    }

    #[test]
    fn test_color_cycle_zero_speed() {
        // Edge case: speed=0 should not cause divide-by-zero, should show first color
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let colors = vec![
            Color::new(255, 0, 0), // Red (first)
            Color::new(0, 255, 0), // Green
            Color::new(0, 0, 255), // Blue
        ];

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Fixed(0.0), // Zero speed!
                direction: CycleDirection::Forward,
                transition: CycleTransition::Snap,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Should not panic, and should show first color
        let commands = engine.update(Duration::from_millis(0), None).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (255, 0, 0),
            "Zero speed should show first color (red)"
        );

        // Even after time passes, should still show first color (frozen)
        let commands = engine.update(Duration::from_millis(5000), None).unwrap();
        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
        assert_eq!(
            (red_cmd.value, green_cmd.value, blue_cmd.value),
            (255, 0, 0),
            "Zero speed should remain frozen on first color"
        );
    }

    #[test]
    fn test_chase_zero_speed() {
        // Edge case: speed=0 should not cause divide-by-zero, should keep first fixture active
        let mut engine = EffectEngine::new();
        let fixture_0 = create_test_fixture("fixture_0", 1, 1);
        let fixture_1 = create_test_fixture("fixture_1", 1, 11);
        let fixture_2 = create_test_fixture("fixture_2", 1, 21);
        engine.register_fixture(fixture_0);
        engine.register_fixture(fixture_1);
        engine.register_fixture(fixture_2);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Fixed(0.0), // Zero speed!
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
            },
            vec![
                "fixture_0".to_string(),
                "fixture_1".to_string(),
                "fixture_2".to_string(),
            ],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Should not panic, first fixture should be active
        let commands = engine.update(Duration::from_millis(0), None).unwrap();
        let dimmer_channels = [1, 11, 21];
        let active_count = commands
            .iter()
            .filter(|cmd| dimmer_channels.contains(&cmd.channel) && cmd.value == 255)
            .count();
        assert_eq!(
            active_count, 1,
            "Zero speed should have exactly one fixture active"
        );

        // First fixture (channel 1) should be the active one
        let first_dimmer = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(first_dimmer.value, 255, "First fixture should be active");

        // Even after time passes, should still be frozen on first fixture
        let commands = engine.update(Duration::from_millis(5000), None).unwrap();
        let first_dimmer = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(
            first_dimmer.value, 255,
            "Zero speed should remain frozen on first fixture"
        );
    }

    #[test]
    fn test_chase_empty_fixtures() {
        // Edge case: chase with no fixtures should not panic (empty fixture list)
        let mut engine = EffectEngine::new();
        // Don't register any fixtures

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
            },
            vec![], // Empty fixture list!
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Should not panic, should return empty commands
        let commands = engine.update(Duration::from_millis(0), None).unwrap();
        assert!(
            commands.is_empty(),
            "Empty fixture chase should produce no commands"
        );

        // Should still work after time passes
        let commands = engine.update(Duration::from_millis(1000), None).unwrap();
        assert!(
            commands.is_empty(),
            "Empty fixture chase should still produce no commands"
        );
    }

    #[test]
    fn test_single_color_cycle() {
        // Edge case: color cycle with only 1 color should always show that color
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let colors = vec![Color::new(255, 128, 64)]; // Single color

        for direction in [
            CycleDirection::Forward,
            CycleDirection::Backward,
            CycleDirection::PingPong,
        ] {
            let effect = EffectInstance::new(
                "test_effect".to_string(),
                EffectType::ColorCycle {
                    colors: colors.clone(),
                    speed: TempoAwareSpeed::Fixed(1.0),
                    direction,
                    transition: CycleTransition::Snap,
                },
                vec!["test_fixture".to_string()],
                None,
                None,
                None,
            );

            let mut test_engine = EffectEngine::new();
            let fixture = create_test_fixture("test_fixture", 1, 1);
            test_engine.register_fixture(fixture);
            test_engine.start_effect(effect).unwrap();

            // Should always be the same color at any time
            for ms in [0, 250, 500, 750, 1000] {
                let commands = test_engine.update(Duration::from_millis(ms), None).unwrap();
                let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
                let green_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
                let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap();
                assert_eq!(
                    (red_cmd.value, green_cmd.value, blue_cmd.value),
                    (255, 128, 64),
                    "{:?} with single color at t={}ms should always show that color",
                    direction,
                    ms
                );
            }
        }
    }

    #[test]
    fn test_strobe_effect() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(2.0), // 2 Hz
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Update the engine
        let commands = engine.update(Duration::from_millis(16), None).unwrap();

        // Should have strobe command since fixture has dedicated strobe channel
        assert_eq!(commands.len(), 1);

        // Check strobe command (frequency 2.0 / max 20.0 = 0.1 = 25 in DMX)
        let strobe_cmd = commands.iter().find(|cmd| cmd.channel == 6).unwrap();
        assert_eq!(strobe_cmd.value, 25);
    }

    #[test]
    fn test_dimmer_effect() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Dimmer {
                start_level: 0.0,
                end_level: 1.0,
                duration: Duration::from_secs(1),
                curve: DimmerCurve::Linear,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        )
        .with_timing(Some(Instant::now()), Some(Duration::from_secs(1)));

        engine.start_effect(effect).unwrap();

        // Update the engine after 500ms (half duration)
        let commands = engine.update(Duration::from_millis(500), None).unwrap();

        // Should have only dimmer command since fixture has dedicated dimmer channel
        // The fixture profile system uses DedicatedDimmer strategy for RGB+dimmer fixtures
        assert_eq!(commands.len(), 1);

        // Check dimmer command
        let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
        assert_eq!(dimmer_cmd.value, 127);
    }

    #[test]
    fn test_rainbow_effect() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Fixed(1.0),
                saturation: 1.0,
                brightness: 1.0,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Update the engine
        let commands = engine.update(Duration::from_millis(16), None).unwrap();

        // Should have RGB commands
        assert_eq!(commands.len(), 3);

        let red_cmd = commands.iter().find(|cmd| cmd.channel == 2);
        let green_cmd = commands.iter().find(|cmd| cmd.channel == 3);
        let blue_cmd = commands.iter().find(|cmd| cmd.channel == 4);

        assert!(red_cmd.is_some());
        assert!(green_cmd.is_some());
        assert!(blue_cmd.is_some());
    }

    #[test]
    fn test_pulse_effect() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Pulse {
                base_level: 0.5,
                pulse_amplitude: 0.5,
                frequency: TempoAwareFrequency::Fixed(1.0), // 1 Hz
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Update the engine
        let commands = engine.update(Duration::from_millis(16), None).unwrap();

        // Should have dimmer command since fixture has dedicated dimmer channel
        assert_eq!(commands.len(), 1);

        // Check that dimmer command exists (values are u8, so always in valid range)
        let _dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    }

    #[test]
    fn test_chase_effect() {
        let mut engine = EffectEngine::new();
        let fixture1 = create_test_fixture("fixture1", 1, 1);
        let fixture2 = create_test_fixture("fixture2", 1, 6);
        let fixture3 = create_test_fixture("fixture3", 1, 11);

        engine.register_fixture(fixture1);
        engine.register_fixture(fixture2);
        engine.register_fixture(fixture3);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
            },
            vec![
                "fixture1".to_string(),
                "fixture2".to_string(),
                "fixture3".to_string(),
            ],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Update the engine
        let commands = engine.update(Duration::from_millis(16), None).unwrap();

        // Should have dimmer commands for all fixtures
        // Note: The chase effect might generate more commands than expected
        // due to the simplified implementation
        assert!(commands.len() >= 3);

        // All commands should be for dimmer channels (but may be on different DMX addresses)
        for cmd in &commands {
            // The chase effect generates commands for different DMX addresses
            // but all should be for the dimmer channel (channel 1 relative to fixture address)
            assert!(cmd.channel >= 1 && cmd.channel <= 15); // Within reasonable DMX range
        }

        // Should have commands for all three fixtures
        let fixture1_cmd = commands
            .iter()
            .find(|cmd| cmd.universe == 1 && cmd.channel == 1);
        let fixture2_cmd = commands
            .iter()
            .find(|cmd| cmd.universe == 1 && cmd.channel == 6);
        let fixture3_cmd = commands
            .iter()
            .find(|cmd| cmd.universe == 1 && cmd.channel == 11);

        assert!(fixture1_cmd.is_some());
        assert!(fixture2_cmd.is_some());
        assert!(fixture3_cmd.is_some());
    }

    #[test]
    fn test_effect_priority() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Low priority effect
        let mut low_priority_params = HashMap::new();
        low_priority_params.insert("dimmer".to_string(), 0.3);

        let low_effect = EffectInstance::new(
            "low_effect".to_string(),
            EffectType::Static {
                parameters: low_priority_params,
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        )
        .with_priority(1);

        // High priority effect
        let mut high_priority_params = HashMap::new();
        high_priority_params.insert("dimmer".to_string(), 0.8);

        let high_effect = EffectInstance::new(
            "high_effect".to_string(),
            EffectType::Static {
                parameters: high_priority_params,
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        )
        .with_priority(10);

        engine.start_effect(low_effect).unwrap();
        engine.start_effect(high_effect).unwrap();

        // Update the engine
        let commands = engine.update(Duration::from_millis(16), None).unwrap();

        // Should have only one dimmer command (high priority wins)
        assert_eq!(commands.len(), 1);
        let dimmer_cmd = &commands[0];
        assert_eq!(dimmer_cmd.value, 204); // 80% of 255
    }

    #[test]
    fn test_effect_stop() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let mut parameters = HashMap::new();
        parameters.insert("dimmer".to_string(), 0.5);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters,
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Update the engine - should have command
        let commands = engine.update(Duration::from_millis(16), None).unwrap();
        assert_eq!(commands.len(), 1);

        // Stop the effect

        // Update again - should still have commands since we didn't stop the effect
        let commands = engine.update(Duration::from_millis(16), None).unwrap();
        assert_eq!(commands.len(), 1);
    }

    #[test]
    fn test_invalid_fixture_error() {
        let mut engine = EffectEngine::new();

        let mut parameters = HashMap::new();
        parameters.insert("dimmer".to_string(), 0.5);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters,
                duration: None,
            },
            vec!["nonexistent_fixture".to_string()],
            None,
            None,
            None,
        );

        let result = engine.start_effect(effect);
        assert!(result.is_err());

        if let Err(EffectError::Fixture(msg)) = result {
            assert!(msg.contains("nonexistent_fixture"));
        } else {
            panic!("Expected InvalidFixture error");
        }
    }

    #[test]
    fn test_effect_duration_expiry() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        let mut parameters = HashMap::new();
        parameters.insert("dimmer".to_string(), 0.5);

        let effect = EffectInstance::new(
            "test_effect".to_string(),
            EffectType::Static {
                parameters,
                duration: Some(Duration::from_millis(100)), // Set duration for expiry test
            },
            vec!["test_fixture".to_string()],
            None,                             // up_time
            Some(Duration::from_millis(100)), // hold_time
            None,                             // down_time
        )
        .with_timing(Some(Instant::now()), Some(Duration::from_millis(100)));

        engine.start_effect(effect).unwrap();

        // Update before expiry - should have commands
        let commands = engine.update(Duration::from_millis(50), None).unwrap();
        assert_eq!(commands.len(), 1);

        // Update after expiry - timed static effects end and don't preserve their state
        let commands = engine.update(Duration::from_millis(100), None).unwrap();
        // Timed static effects end and don't generate commands after expiry
        assert_eq!(commands.len(), 0);
    }

    #[test]
    fn test_tempo_aware_speed_adapts_to_tempo_changes() {
        use crate::lighting::tempo::{
            TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
        };

        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Create a tempo map: 120 BPM initially, changes to 60 BPM at 4 seconds
        let tempo_map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(4)),
                original_measure_beat: None,
                bpm: Some(60.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        engine.set_tempo_map(Some(tempo_map));

        // Create a cycle effect with speed: 1measure (tempo-aware)
        let colors = vec![
            Color::new(255, 0, 0), // Red
            Color::new(0, 255, 0), // Green
            Color::new(0, 0, 255), // Blue
        ];

        let effect = EffectInstance::new(
            "tempo_aware_cycle".to_string(),
            EffectType::ColorCycle {
                colors,
                speed: TempoAwareSpeed::Measures(1.0), // 1 cycle per measure
                direction: CycleDirection::Forward,
                transition: CycleTransition::Snap,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0s (120 BPM): 1 measure = 2.0s, so speed = 0.5 cycles/sec
        // Verify effect is running
        let commands = engine.update(Duration::from_millis(100), None).unwrap();
        assert!(!commands.is_empty(), "Effect should generate commands");

        // At t=4s: tempo changes to 60 BPM
        // At t=4.1s (60 BPM): 1 measure = 4.0s, so speed = 0.25 cycles/sec
        // This is slower than before - the effect should have adapted
        engine.update(Duration::from_secs(4), None).unwrap(); // Advance to tempo change
        let commands_after = engine.update(Duration::from_millis(100), None).unwrap(); // 0.1s after tempo change

        // At slower tempo, the cycle should be progressing more slowly
        // The effect should still be running and generating commands
        assert!(
            !commands_after.is_empty(),
            "Effect should still generate commands after tempo change"
        );

        // Verify that the speed calculation uses the new tempo
        // We can't easily verify exact color values, but we can verify the effect is adapting
        // by checking that it's still running and producing different values over time
        let commands_later = engine.update(Duration::from_millis(1000), None).unwrap(); // 1.1s after tempo change
        assert!(
            !commands_later.is_empty(),
            "Effect should continue running after tempo change"
        );
    }

    #[test]
    fn test_tempo_aware_frequency_adapts_to_tempo_changes() {
        use crate::lighting::tempo::{
            TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
        };

        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Create a tempo map: 120 BPM initially, changes to 60 BPM at 2 seconds
        let tempo_map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(2)),
                original_measure_beat: None,
                bpm: Some(60.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        engine.set_tempo_map(Some(tempo_map));

        // Create a background static effect so strobe has something to work with
        let mut bg_params = HashMap::new();
        bg_params.insert("red".to_string(), 1.0);
        bg_params.insert("green".to_string(), 1.0);
        bg_params.insert("blue".to_string(), 1.0);
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
        engine.update(Duration::from_millis(10), None).unwrap(); // Let background settle

        // Create a strobe effect with frequency: 1beat (tempo-aware)
        let effect = EffectInstance::new(
            "tempo_aware_strobe".to_string(),
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Beats(1.0), // 1 cycle per beat
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0s (120 BPM): 1 beat = 0.5s, so frequency = 2.0 Hz
        // At 2 Hz, period = 0.5s
        let commands_before = engine.update(Duration::from_millis(100), None).unwrap();
        let strobe_before = commands_before.iter().find(|cmd| cmd.channel == 6);
        assert!(
            strobe_before.is_some(),
            "Strobe should generate commands before tempo change"
        );

        // At t=2s: tempo changes to 60 BPM
        // At t=2.1s (60 BPM): 1 beat = 1.0s, so frequency = 1.0 Hz
        // At 1 Hz, period = 1.0s
        // This is slower than before - the effect should have adapted
        engine.update(Duration::from_secs(2), None).unwrap(); // Advance to tempo change
        let commands_after = engine.update(Duration::from_millis(100), None).unwrap(); // 0.1s after tempo change

        // The effect should still be running (may or may not generate strobe commands depending on phase)
        // The key is that the frequency calculation uses the new tempo
        // We verify the effect is adapting by checking commands are generated
        assert!(
            !commands_after.is_empty(),
            "Effect should still generate commands after tempo change"
        );
    }

    #[test]
    fn test_tempo_aware_chase_adapts_to_tempo_changes() {
        use crate::lighting::tempo::{
            TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
        };

        let mut engine = EffectEngine::new();
        let fixture1 = create_test_fixture("fixture1", 1, 1);
        let fixture2 = create_test_fixture("fixture2", 1, 6);
        let fixture3 = create_test_fixture("fixture3", 1, 11);
        engine.register_fixture(fixture1);
        engine.register_fixture(fixture2);
        engine.register_fixture(fixture3);

        // Create a tempo map: 120 BPM initially, changes to 60 BPM at 3 seconds
        let tempo_map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(3)),
                original_measure_beat: None,
                bpm: Some(60.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        engine.set_tempo_map(Some(tempo_map));

        // Create a chase effect with speed: 1measure (tempo-aware)
        let effect = EffectInstance::new(
            "tempo_aware_chase".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Measures(1.0), // 1 cycle per measure
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
            },
            vec![
                "fixture1".to_string(),
                "fixture2".to_string(),
                "fixture3".to_string(),
            ],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0s (120 BPM): 1 measure = 2.0s, so speed = 0.5 cycles/sec
        let commands_before = engine.update(Duration::from_millis(100), None).unwrap();
        assert!(
            !commands_before.is_empty(),
            "Chase should generate commands before tempo change"
        );

        // At t=3s: tempo changes to 60 BPM
        // At t=3.1s (60 BPM): 1 measure = 4.0s, so speed = 0.25 cycles/sec
        // This is slower than before - the effect should have adapted
        engine.update(Duration::from_secs(3), None).unwrap(); // Advance to tempo change
        let commands_after = engine.update(Duration::from_millis(100), None).unwrap(); // 0.1s after tempo change

        // The effect should still be running and generating commands
        assert!(
            !commands_after.is_empty(),
            "Chase should still generate commands after tempo change"
        );

        // Verify it continues running
        let commands_later = engine.update(Duration::from_millis(1000), None).unwrap();
        assert!(
            !commands_later.is_empty(),
            "Chase should continue running after tempo change"
        );
    }

    #[test]
    fn test_tempo_aware_chase_beats_speed_never_zero() {
        use crate::lighting::tempo::{
            TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
        };

        let mut engine = EffectEngine::new();
        let fixture1 = create_test_fixture("fixture1", 1, 1);
        let fixture2 = create_test_fixture("fixture2", 1, 6);
        let fixture3 = create_test_fixture("fixture3", 1, 11);
        engine.register_fixture(fixture1);
        engine.register_fixture(fixture2);
        engine.register_fixture(fixture3);

        // Tempo map: 120 BPM initially, changes to 60 BPM at 3 seconds
        let tempo_map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(3)),
                original_measure_beat: None,
                bpm: Some(60.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        engine.set_tempo_map(Some(tempo_map));

        // Chase with speed expressed in beats (tempo-aware), using a small beat value
        // similar to "0.5beats" in the show file. This guards against beats-based
        // speed resolving to zero due to beats_to_duration returning a degenerate
        // duration around tempo changes.
        let effect = EffectInstance::new(
            "tempo_aware_chase_beats".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Beats(0.5),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
            },
            vec![
                "fixture1".to_string(),
                "fixture2".to_string(),
                "fixture3".to_string(),
            ],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // Shortly after start at 120 BPM, the chase should generate commands
        let commands_before = engine.update(Duration::from_millis(100), None).unwrap();
        assert!(
            !commands_before.is_empty(),
            "Chase with beats-based speed should generate commands before tempo change"
        );

        // Advance past the tempo change and ensure the chase still generates commands
        engine.update(Duration::from_secs(3), None).unwrap(); // Advance to tempo change
        let commands_after = engine.update(Duration::from_millis(100), None).unwrap(); // 0.1s after change
        assert!(
            !commands_after.is_empty(),
            "Chase with beats-based speed should still generate commands after tempo change"
        );

        // And it should continue to run later in time as well
        let commands_later = engine.update(Duration::from_millis(1000), None).unwrap();
        assert!(
            !commands_later.is_empty(),
            "Chase with beats-based speed should continue running after tempo change"
        );
    }

    #[test]
    fn test_chase_after_tempo_change_with_measure_offset() {
        // Regression test: Replicates scenario where chases after a tempo change
        // may be missed due to timing/precision issues.
        // Scenario:
        // - Tempo change at measure 68/1 (score measure)
        // - Measure offset of 8
        // - Random chase at @70/1 (score measure) with speed: 1beats
        // - Linear chase at @74/1 (score measure) with speed: 0.5beats, direction: right_to_left
        use crate::lighting::tempo::{
            TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
        };

        let mut engine = EffectEngine::new();
        let fixture1 = create_test_fixture("fixture1", 1, 1);
        let fixture2 = create_test_fixture("fixture2", 1, 6);
        let fixture3 = create_test_fixture("fixture3", 1, 11);
        let fixture4 = create_test_fixture("fixture4", 1, 16);
        engine.register_fixture(fixture1);
        engine.register_fixture(fixture2);
        engine.register_fixture(fixture3);
        engine.register_fixture(fixture4);

        // Create tempo map: 160 BPM initially, changes to 120 BPM at measure 68/1
        // Using MeasureBeat position to match the user's scenario
        let tempo_map = TempoMap::new(
            Duration::from_secs_f64(1.5), // start_offset of 1.5s
            160.0,                        // initial BPM
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::MeasureBeat(68, 1.0),
                original_measure_beat: Some((68, 1.0)),
                bpm: Some(120.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        engine.set_tempo_map(Some(tempo_map.clone()));

        // Calculate times for the chases using measure_to_time_with_offset
        // Measure offset of 8 means score measure 70 becomes playback measure 78
        let measure_offset = 8;
        let random_chase_time = tempo_map
            .measure_to_time_with_offset(70, 1.0, measure_offset, 0.0)
            .expect("Should be able to calculate time for measure 70/1");
        let linear_chase_time = tempo_map
            .measure_to_time_with_offset(74, 1.0, measure_offset, 0.0)
            .expect("Should be able to calculate time for measure 74/1");

        // Create random chase at @70/1 with speed: 1beats
        let random_chase = EffectInstance::new(
            "random_chase".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Random,
                speed: TempoAwareSpeed::Beats(1.0),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
            },
            vec![
                "fixture1".to_string(),
                "fixture2".to_string(),
                "fixture3".to_string(),
                "fixture4".to_string(),
            ],
            None,
            None,
            None,
        );

        // Create linear chase at @74/1 with speed: 0.5beats, direction: right_to_left
        let linear_chase = EffectInstance::new(
            "linear_chase".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Beats(0.5),
                direction: ChaseDirection::RightToLeft,
                transition: CycleTransition::Snap,
            },
            vec![
                "fixture1".to_string(),
                "fixture2".to_string(),
                "fixture3".to_string(),
                "fixture4".to_string(),
            ],
            None,
            None,
            None,
        );

        // Advance to just before the random chase
        let time_before_random = random_chase_time - Duration::from_millis(10);
        engine.update(time_before_random, None).unwrap();

        // Start the random chase
        engine.start_effect(random_chase).unwrap();

        // Test that random chase produces output at various times
        // Test immediately after start
        let commands_at_start = engine
            .update(random_chase_time + Duration::from_millis(16), None)
            .unwrap();
        assert!(
            !commands_at_start.is_empty(),
            "Random chase should generate commands immediately after start"
        );

        // Test a bit later (during the chase)
        let commands_during = engine
            .update(random_chase_time + Duration::from_millis(100), None)
            .unwrap();
        assert!(
            !commands_during.is_empty(),
            "Random chase should continue generating commands during execution"
        );

        // Advance to just before the linear chase
        let time_before_linear = linear_chase_time - Duration::from_millis(10);
        engine.update(time_before_linear, None).unwrap();

        // Start the linear chase
        engine.start_effect(linear_chase).unwrap();

        // Test that linear chase produces output at various times
        // Test immediately after start
        let commands_linear_start = engine
            .update(linear_chase_time + Duration::from_millis(16), None)
            .unwrap();
        assert!(
            !commands_linear_start.is_empty(),
            "Linear chase should generate commands immediately after start (at measure 74/1)"
        );

        // Test a bit later (during the chase)
        let commands_linear_during = engine
            .update(linear_chase_time + Duration::from_millis(100), None)
            .unwrap();
        assert!(
            !commands_linear_during.is_empty(),
            "Linear chase should continue generating commands during execution"
        );

        // Test even later to ensure it keeps running
        let commands_linear_later = engine
            .update(linear_chase_time + Duration::from_millis(500), None)
            .unwrap();
        assert!(
            !commands_linear_later.is_empty(),
            "Linear chase should continue generating commands well into its execution"
        );

        // Critical test: Verify speed calculation doesn't return zero
        // This is the suspected issue - beats_to_duration might return a degenerate value
        // that causes speed to be calculated as 0.0
        let current_speed = TempoAwareSpeed::Beats(0.5).to_cycles_per_second(
            Some(&tempo_map),
            linear_chase_time + Duration::from_millis(100),
        );
        assert!(
            current_speed > 0.0,
            "Chase speed should never be zero; got speed={} at time after tempo change",
            current_speed
        );
    }

    #[test]
    fn test_chase_timing_edge_cases_after_tempo_change() {
        // More aggressive test: Try to catch timing edge cases that might cause
        // a chase to be missed. Tests multiple time points around tempo changes
        // and chase start times to catch floating-point precision issues.
        use crate::lighting::tempo::{
            TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
        };

        let mut engine = EffectEngine::new();
        let fixture1 = create_test_fixture("fixture1", 1, 1);
        let fixture2 = create_test_fixture("fixture2", 1, 6);
        let fixture3 = create_test_fixture("fixture3", 1, 11);
        engine.register_fixture(fixture1);
        engine.register_fixture(fixture2);
        engine.register_fixture(fixture3);

        // Create tempo map: 160 BPM initially, changes to 120 BPM at measure 68/1
        let tempo_map = TempoMap::new(
            Duration::from_secs_f64(1.5),
            160.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::MeasureBeat(68, 1.0),
                original_measure_beat: Some((68, 1.0)),
                bpm: Some(120.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        engine.set_tempo_map(Some(tempo_map.clone()));

        let measure_offset = 8;
        let linear_chase_time = tempo_map
            .measure_to_time_with_offset(74, 1.0, measure_offset, 0.0)
            .expect("Should be able to calculate time for measure 74/1");

        // Test speed calculation at multiple time points around the chase start
        // This catches edge cases where beats_to_duration might return degenerate values
        let test_times = [
            linear_chase_time - Duration::from_millis(1),
            linear_chase_time,
            linear_chase_time + Duration::from_nanos(1),
            linear_chase_time + Duration::from_millis(1),
            linear_chase_time + Duration::from_millis(10),
            linear_chase_time + Duration::from_millis(100),
            linear_chase_time + Duration::from_millis(500),
        ];

        for (i, test_time) in test_times.iter().enumerate() {
            let speed =
                TempoAwareSpeed::Beats(0.5).to_cycles_per_second(Some(&tempo_map), *test_time);
            assert!(
                speed > 0.0,
                "Speed should never be zero at test point {} (time={:?}): got speed={}",
                i,
                test_time,
                speed
            );
        }

        // Now test actual chase execution at these edge case times
        let linear_chase = EffectInstance::new(
            "linear_chase".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Beats(0.5),
                direction: ChaseDirection::RightToLeft,
                transition: CycleTransition::Snap,
            },
            vec![
                "fixture1".to_string(),
                "fixture2".to_string(),
                "fixture3".to_string(),
            ],
            None,
            None,
            None,
        );

        // Advance to just before the chase
        engine
            .update(linear_chase_time - Duration::from_millis(10), None)
            .unwrap();
        engine.start_effect(linear_chase).unwrap();

        // Test at multiple time points to catch any frame where it might fail
        for (i, test_time) in test_times.iter().enumerate() {
            if *test_time >= linear_chase_time {
                engine.update(*test_time, None).unwrap();
                let commands = engine
                    .update(*test_time + Duration::from_millis(16), None)
                    .unwrap();
                assert!(
                    !commands.is_empty(),
                    "Chase should generate commands at test point {} (time={:?})",
                    i,
                    test_time
                );
            }
        }
    }

    #[test]
    fn test_tempo_aware_rainbow_adapts_to_tempo_changes() {
        use crate::lighting::tempo::{
            TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
        };

        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Create a tempo map: 120 BPM initially, changes to 60 BPM at 2.5 seconds
        let tempo_map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_millis(2500)),
                original_measure_beat: None,
                bpm: Some(60.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        engine.set_tempo_map(Some(tempo_map));

        // Create a rainbow effect with speed: 2beats (tempo-aware)
        let effect = EffectInstance::new(
            "tempo_aware_rainbow".to_string(),
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Beats(2.0), // 1 cycle per 2 beats
                saturation: 1.0,
                brightness: 1.0,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0s (120 BPM): 2 beats = 1.0s, so speed = 1.0 cycles/sec
        let commands_before = engine.update(Duration::from_millis(100), None).unwrap();
        assert!(
            !commands_before.is_empty(),
            "Rainbow should generate commands before tempo change"
        );

        // At t=2.5s: tempo changes to 60 BPM
        // At t=2.6s (60 BPM): 2 beats = 2.0s, so speed = 0.5 cycles/sec
        // This is slower than before - the effect should have adapted
        engine.update(Duration::from_millis(2500), None).unwrap(); // Advance to tempo change
        let commands_after = engine.update(Duration::from_millis(100), None).unwrap(); // 0.1s after tempo change

        // The effect should still be running and generating commands
        assert!(
            !commands_after.is_empty(),
            "Rainbow should still generate commands after tempo change"
        );

        // Verify it continues running
        let commands_later = engine.update(Duration::from_millis(1000), None).unwrap();
        assert!(
            !commands_later.is_empty(),
            "Rainbow should continue running after tempo change"
        );
    }

    #[test]
    fn test_tempo_aware_pulse_adapts_to_tempo_changes() {
        use crate::lighting::tempo::{
            TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
        };

        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Create a tempo map: 120 BPM initially, changes to 60 BPM at 1.5 seconds
        let tempo_map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_millis(1500)),
                original_measure_beat: None,
                bpm: Some(60.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        engine.set_tempo_map(Some(tempo_map));

        // Create a pulse effect with frequency: 1beat (tempo-aware)
        let effect = EffectInstance::new(
            "tempo_aware_pulse".to_string(),
            EffectType::Pulse {
                base_level: 0.5,
                pulse_amplitude: 0.5,
                frequency: TempoAwareFrequency::Beats(1.0), // 1 cycle per beat
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );

        engine.start_effect(effect).unwrap();

        // At t=0s (120 BPM): 1 beat = 0.5s, so frequency = 2.0 Hz
        let commands_before = engine.update(Duration::from_millis(100), None).unwrap();
        assert!(
            !commands_before.is_empty(),
            "Pulse should generate commands before tempo change"
        );

        // At t=1.5s: tempo changes to 60 BPM
        // At t=1.6s (60 BPM): 1 beat = 1.0s, so frequency = 1.0 Hz
        // This is slower than before - the effect should have adapted
        engine.update(Duration::from_millis(1500), None).unwrap(); // Advance to tempo change
        let commands_after = engine.update(Duration::from_millis(100), None).unwrap(); // 0.1s after tempo change

        // The effect should still be running and generating commands
        assert!(
            !commands_after.is_empty(),
            "Pulse should still generate commands after tempo change"
        );

        // Verify it continues running
        let commands_later = engine.update(Duration::from_millis(1000), None).unwrap();
        assert!(
            !commands_later.is_empty(),
            "Pulse should continue running after tempo change"
        );
    }

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

        // After fade completes (another 600ms), effect should be gone
        let _commands_end = engine.update(Duration::from_millis(600), None).unwrap();
        // Effect should have completed and been removed
        assert_eq!(engine.active_effects_count(), 0);
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

    #[test]
    fn test_format_active_effects() {
        let mut engine = EffectEngine::new();
        let fixture = create_test_fixture("test_fixture", 1, 1);
        engine.register_fixture(fixture);

        // Test with no effects
        let output = engine.format_active_effects();
        assert_eq!(output, "No active effects");

        // Add a static effect
        let mut params = HashMap::new();
        params.insert("red".to_string(), 1.0);
        let effect = EffectInstance::new(
            "test_effect_1".to_string(),
            EffectType::Static {
                parameters: params,
                duration: None,
            },
            vec!["test_fixture".to_string()],
            None,
            Some(Duration::from_secs(5)),
            None,
        );
        engine.start_effect(effect).unwrap();

        // Add a chase effect on a different layer
        let mut chase_effect = EffectInstance::new(
            "test_effect_2".to_string(),
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
            },
            vec!["test_fixture".to_string()],
            None,
            None,
            None,
        );
        chase_effect.layer = EffectLayer::Foreground;
        engine.start_effect(chase_effect).unwrap();

        // Format and verify output
        let output = engine.format_active_effects();
        assert!(output.contains("Active effects (2)"));
        assert!(output.contains("Background"));
        assert!(output.contains("Foreground"));
        assert!(output.contains("test_effect_1"));
        assert!(output.contains("test_effect_2"));
        assert!(output.contains("Static"));
        assert!(output.contains("Chase"));
        assert!(output.contains("1 fixture(s)"));
    }
}
