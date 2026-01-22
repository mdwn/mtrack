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
use crate::lighting::effects::*;
use crate::lighting::engine::tests::common::create_test_fixture;
use crate::lighting::engine::EffectEngine;

use std::time::Duration;

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
