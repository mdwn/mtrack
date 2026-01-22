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

use std::collections::HashMap;
use std::time::Duration;

use super::*;

#[test]
fn test_color_from_hsv() {
    let red = Color::from_hsv(0.0, 1.0, 1.0);
    assert_eq!(red.r, 255);
    assert_eq!(red.g, 0);
    assert_eq!(red.b, 0);

    let green = Color::from_hsv(120.0, 1.0, 1.0);
    assert_eq!(green.r, 0);
    assert_eq!(green.g, 255);
    assert_eq!(green.b, 0);

    let blue = Color::from_hsv(240.0, 1.0, 1.0);
    assert_eq!(blue.r, 0);
    assert_eq!(blue.g, 0);
    assert_eq!(blue.b, 255);
}

#[test]
fn test_fixture_capabilities() {
    // Test RGB fixture
    let mut rgb_channels = HashMap::new();
    rgb_channels.insert("red".to_string(), 1);
    rgb_channels.insert("green".to_string(), 2);
    rgb_channels.insert("blue".to_string(), 3);
    rgb_channels.insert("dimmer".to_string(), 4);

    let rgb_fixture = FixtureInfo {
        name: "RGB Fixture".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "RGB_Par".to_string(),
        channels: rgb_channels,
        max_strobe_frequency: None, // RGB_Par doesn't have strobe
    };

    assert!(rgb_fixture.has_capability(FixtureCapabilities::RGB_COLOR));
    assert!(rgb_fixture.has_capability(FixtureCapabilities::DIMMING));
    assert!(!rgb_fixture.has_capability(FixtureCapabilities::STROBING));

    // Test strobe fixture
    let mut strobe_channels = HashMap::new();
    strobe_channels.insert("strobe".to_string(), 1);
    strobe_channels.insert("dimmer".to_string(), 2);

    let strobe_fixture = FixtureInfo {
        name: "Strobe Fixture".to_string(),
        universe: 1,
        address: 5,
        fixture_type: "Strobe".to_string(),
        channels: strobe_channels,
        max_strobe_frequency: Some(20.0), // Test strobe fixture max frequency
    };

    assert!(strobe_fixture.has_capability(FixtureCapabilities::STROBING));
    assert!(strobe_fixture.has_capability(FixtureCapabilities::DIMMING));
    assert!(!strobe_fixture.has_capability(FixtureCapabilities::RGB_COLOR));

    // Test multiple capabilities
    assert!(
        rgb_fixture.has_capability(FixtureCapabilities::RGB_COLOR)
            && rgb_fixture.has_capability(FixtureCapabilities::DIMMING)
    );
    assert!(
        !(strobe_fixture.has_capability(FixtureCapabilities::RGB_COLOR)
            && strobe_fixture.has_capability(FixtureCapabilities::DIMMING))
    );

    // Test bitwise operations
    let capabilities = FixtureCapabilities::RGB_COLOR.with(FixtureCapabilities::DIMMING);
    assert!(capabilities.contains(FixtureCapabilities::RGB_COLOR));
    assert!(capabilities.contains(FixtureCapabilities::DIMMING));
    assert!(!capabilities.contains(FixtureCapabilities::STROBING));
    assert_eq!(capabilities.count(), 2);
}

#[test]
fn test_capabilities_performance() {
    // Create a fixture with multiple capabilities
    let mut channels = HashMap::new();
    channels.insert("red".to_string(), 1);
    channels.insert("green".to_string(), 2);
    channels.insert("blue".to_string(), 3);
    channels.insert("dimmer".to_string(), 4);
    channels.insert("strobe".to_string(), 5);
    channels.insert("pan".to_string(), 6);
    channels.insert("tilt".to_string(), 7);

    let fixture = FixtureInfo {
        name: "Multi-Capability Fixture".to_string(),
        universe: 1,
        address: 1,
        fixture_type: "Moving_Head".to_string(),
        channels,
        max_strobe_frequency: Some(15.0), // Moving head max strobe frequency
    };

    let capabilities = fixture.capabilities();

    // Test individual capability checks (very fast with bitwise operations)
    assert!(capabilities.contains(FixtureCapabilities::RGB_COLOR));
    assert!(capabilities.contains(FixtureCapabilities::DIMMING));
    assert!(capabilities.contains(FixtureCapabilities::STROBING));
    assert!(capabilities.contains(FixtureCapabilities::PANNING));
    assert!(capabilities.contains(FixtureCapabilities::TILTING));
    assert!(!capabilities.contains(FixtureCapabilities::ZOOMING));

    // Test multiple capability checks (single bitwise operation)
    let _required = FixtureCapabilities::RGB_COLOR
        .with(FixtureCapabilities::DIMMING)
        .with(FixtureCapabilities::STROBING);
    assert!(
        capabilities.contains(FixtureCapabilities::RGB_COLOR)
            && capabilities.contains(FixtureCapabilities::DIMMING)
            && capabilities.contains(FixtureCapabilities::STROBING)
    );

    // Test capability counting
    assert_eq!(capabilities.count(), 5);
}

#[test]
fn test_effect_instance_creation() {
    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Static {
            parameters: HashMap::new(),
            duration: Some(Duration::from_secs(5)),
        },
        vec!["fixture1".to_string(), "fixture2".to_string()],
        None,
        None,
        None,
    );

    assert_eq!(effect.id, "test_effect");
    assert_eq!(effect.target_fixtures.len(), 2);
    assert!(effect.enabled);
}

#[test]
fn test_tempo_aware_speed_zero_values() {
    // Test that zero/negative values don't cause divide-by-zero

    // Zero seconds should return 0.0 (stopped), not infinity
    let speed = TempoAwareSpeed::Seconds(0.0);
    let result = speed.to_cycles_per_second(None, Duration::ZERO);
    assert_eq!(result, 0.0, "Zero seconds should return 0.0");
    assert!(!result.is_infinite(), "Should not be infinite");

    // Negative seconds should also return 0.0
    let speed = TempoAwareSpeed::Seconds(-1.0);
    let result = speed.to_cycles_per_second(None, Duration::ZERO);
    assert_eq!(result, 0.0, "Negative seconds should return 0.0");

    // Zero measures should return 0.0
    let speed = TempoAwareSpeed::Measures(0.0);
    let result = speed.to_cycles_per_second(None, Duration::ZERO);
    assert_eq!(result, 0.0, "Zero measures should return 0.0");

    // Zero beats should return 0.0
    let speed = TempoAwareSpeed::Beats(0.0);
    let result = speed.to_cycles_per_second(None, Duration::ZERO);
    assert_eq!(result, 0.0, "Zero beats should return 0.0");

    // Positive values should still work normally
    let speed = TempoAwareSpeed::Seconds(2.0);
    let result = speed.to_cycles_per_second(None, Duration::ZERO);
    assert!(
        (result - 0.5).abs() < 0.001,
        "2 seconds should give 0.5 cycles/sec"
    );
}

#[test]
fn test_tempo_aware_frequency_zero_values() {
    // Test that zero/negative values don't cause divide-by-zero

    // Zero seconds should return 0.0 (stopped), not infinity
    let freq = TempoAwareFrequency::Seconds(0.0);
    let result = freq.to_hz(None, Duration::ZERO);
    assert_eq!(result, 0.0, "Zero seconds should return 0.0");
    assert!(!result.is_infinite(), "Should not be infinite");

    // Negative seconds should also return 0.0
    let freq = TempoAwareFrequency::Seconds(-1.0);
    let result = freq.to_hz(None, Duration::ZERO);
    assert_eq!(result, 0.0, "Negative seconds should return 0.0");

    // Zero measures should return 0.0
    let freq = TempoAwareFrequency::Measures(0.0);
    let result = freq.to_hz(None, Duration::ZERO);
    assert_eq!(result, 0.0, "Zero measures should return 0.0");

    // Zero beats should return 0.0
    let freq = TempoAwareFrequency::Beats(0.0);
    let result = freq.to_hz(None, Duration::ZERO);
    assert_eq!(result, 0.0, "Zero beats should return 0.0");

    // Positive values should still work normally
    let freq = TempoAwareFrequency::Seconds(0.5);
    let result = freq.to_hz(None, Duration::ZERO);
    assert!((result - 2.0).abs() < 0.001, "0.5 seconds should give 2 Hz");
}

#[test]
fn test_perpetual_effects_total_duration_is_none() {
    // Test that effects without explicit duration have total_duration() = None

    // ColorCycle without timing params is perpetual
    let color_cycle = EffectInstance::new(
        "color_cycle".to_string(),
        EffectType::ColorCycle {
            colors: vec![Color::new(255, 0, 0), Color::new(0, 0, 255)],
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Fade,
        },
        vec!["fixture".to_string()],
        None,
        None,
        None,
    );
    assert!(
        color_cycle.total_duration().is_none(),
        "ColorCycle without timing should be perpetual"
    );

    // Chase without timing params is perpetual
    let chase = EffectInstance::new(
        "chase".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
        },
        vec!["fixture".to_string()],
        None,
        None,
        None,
    );
    assert!(
        chase.total_duration().is_none(),
        "Chase without timing should be perpetual"
    );

    // Rainbow without timing params is perpetual
    let rainbow = EffectInstance::new(
        "rainbow".to_string(),
        EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(1.0),
            saturation: 1.0,
            brightness: 1.0,
        },
        vec!["fixture".to_string()],
        None,
        None,
        None,
    );
    assert!(
        rainbow.total_duration().is_none(),
        "Rainbow without timing should be perpetual"
    );

    // Strobe without duration is perpetual
    let strobe = EffectInstance::new(
        "strobe".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(10.0),
            duration: None,
        },
        vec!["fixture".to_string()],
        None,
        None,
        None,
    );
    assert!(
        strobe.total_duration().is_none(),
        "Strobe without duration should be perpetual"
    );

    // Pulse without duration is perpetual
    let pulse = EffectInstance::new(
        "pulse".to_string(),
        EffectType::Pulse {
            base_level: 0.2,
            pulse_amplitude: 0.8,
            frequency: TempoAwareFrequency::Fixed(2.0),
            duration: None,
        },
        vec!["fixture".to_string()],
        None,
        None,
        None,
    );
    assert!(
        pulse.total_duration().is_none(),
        "Pulse without duration should be perpetual"
    );

    // Static without duration is perpetual
    let static_effect = EffectInstance::new(
        "static".to_string(),
        EffectType::Static {
            parameters: HashMap::new(),
            duration: None,
        },
        vec!["fixture".to_string()],
        None,
        None,
        None,
    );
    assert!(
        static_effect.total_duration().is_none(),
        "Static without duration should be perpetual"
    );
}

#[test]
fn test_effects_with_duration_are_not_perpetual() {
    // Test that effects with explicit duration have a total_duration

    // Strobe with duration
    let strobe = EffectInstance::new(
        "strobe".to_string(),
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(10.0),
            duration: Some(Duration::from_secs(5)),
        },
        vec!["fixture".to_string()],
        None,
        None,
        None,
    );
    assert_eq!(
        strobe.total_duration(),
        Some(Duration::from_secs(5)),
        "Strobe with duration should not be perpetual"
    );

    // Pulse with duration
    let pulse = EffectInstance::new(
        "pulse".to_string(),
        EffectType::Pulse {
            base_level: 0.2,
            pulse_amplitude: 0.8,
            frequency: TempoAwareFrequency::Fixed(2.0),
            duration: Some(Duration::from_secs(10)),
        },
        vec!["fixture".to_string()],
        None,
        None,
        None,
    );
    assert_eq!(
        pulse.total_duration(),
        Some(Duration::from_secs(10)),
        "Pulse with duration should not be perpetual"
    );

    // Static with duration
    let static_effect = EffectInstance::new(
        "static".to_string(),
        EffectType::Static {
            parameters: HashMap::new(),
            duration: Some(Duration::from_secs(3)),
        },
        vec!["fixture".to_string()],
        None,
        None,
        None,
    );
    assert_eq!(
        static_effect.total_duration(),
        Some(Duration::from_secs(3)),
        "Static with duration should not be perpetual"
    );
}

#[test]
fn test_effects_with_timing_params_are_not_perpetual() {
    // Test that effects with timing parameters (up_time, hold_time, down_time)
    // are not perpetual even without explicit duration

    // ColorCycle with hold_time
    let color_cycle = EffectInstance::new(
        "color_cycle".to_string(),
        EffectType::ColorCycle {
            colors: vec![Color::new(255, 0, 0), Color::new(0, 0, 255)],
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Fade,
        },
        vec!["fixture".to_string()],
        None,
        Some(Duration::from_secs(30)), // hold_time
        None,
    );
    assert_eq!(
        color_cycle.total_duration(),
        Some(Duration::from_secs(30)),
        "ColorCycle with hold_time should not be perpetual"
    );

    // Rainbow with up_time and down_time
    let rainbow = EffectInstance::new(
        "rainbow".to_string(),
        EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(1.0),
            saturation: 1.0,
            brightness: 1.0,
        },
        vec!["fixture".to_string()],
        Some(Duration::from_secs(2)), // up_time
        None,
        Some(Duration::from_secs(2)), // down_time
    );
    assert_eq!(
        rainbow.total_duration(),
        Some(Duration::from_secs(4)), // up + hold(0) + down
        "Rainbow with timing params should not be perpetual"
    );
}

#[test]
fn test_perpetual_effects_never_reach_terminal_state() {
    // Perpetual effects should never reach terminal state

    let color_cycle = EffectInstance::new(
        "color_cycle".to_string(),
        EffectType::ColorCycle {
            colors: vec![Color::new(255, 0, 0), Color::new(0, 0, 255)],
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Fade,
        },
        vec!["fixture".to_string()],
        None,
        None,
        None,
    );

    // Test at various time points - should never be terminal
    assert!(
        !color_cycle.has_reached_terminal_state(Duration::from_secs(0)),
        "Perpetual effect should not be terminal at t=0"
    );
    assert!(
        !color_cycle.has_reached_terminal_state(Duration::from_secs(60)),
        "Perpetual effect should not be terminal at t=60s"
    );
    assert!(
        !color_cycle.has_reached_terminal_state(Duration::from_secs(3600)),
        "Perpetual effect should not be terminal at t=1hr"
    );
    assert!(
        !color_cycle.has_reached_terminal_state(Duration::from_secs(86400)),
        "Perpetual effect should not be terminal at t=1day"
    );
}

#[test]
fn test_perpetual_effects_crossfade_multiplier() {
    // Perpetual effects should have crossfade multiplier of 1.0 indefinitely

    let rainbow = EffectInstance::new(
        "rainbow".to_string(),
        EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(1.0),
            saturation: 1.0,
            brightness: 1.0,
        },
        vec!["fixture".to_string()],
        None,
        None,
        None,
    );

    // Should always be at full intensity
    assert!(
        (rainbow.calculate_crossfade_multiplier(Duration::from_secs(0)) - 1.0).abs() < 0.001,
        "Perpetual effect should be at full intensity at t=0"
    );
    assert!(
        (rainbow.calculate_crossfade_multiplier(Duration::from_secs(60)) - 1.0).abs() < 0.001,
        "Perpetual effect should be at full intensity at t=60s"
    );
    assert!(
        (rainbow.calculate_crossfade_multiplier(Duration::from_secs(3600)) - 1.0).abs() < 0.001,
        "Perpetual effect should be at full intensity at t=1hr"
    );
}

#[test]
fn test_perpetual_effect_with_up_time_fades_in_then_stays() {
    // A perpetual effect with only up_time should fade in and stay at full intensity

    let chase = EffectInstance::new(
        "chase".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
        },
        vec!["fixture".to_string()],
        Some(Duration::from_secs(2)), // up_time only
        None,
        None,
    );

    // During fade-in (0 to 2 seconds)
    let mult_at_0 = chase.calculate_crossfade_multiplier(Duration::from_secs(0));
    let mult_at_1 = chase.calculate_crossfade_multiplier(Duration::from_secs(1));
    let mult_at_2 = chase.calculate_crossfade_multiplier(Duration::from_secs(2));

    assert!(mult_at_0 < 0.1, "Should start near 0");
    assert!(
        (mult_at_1 - 0.5).abs() < 0.1,
        "Should be around 50% at midpoint"
    );
    assert!((mult_at_2 - 1.0).abs() < 0.1, "Should reach full intensity");

    // After fade-in, should stay at full intensity indefinitely
    let mult_at_10 = chase.calculate_crossfade_multiplier(Duration::from_secs(10));
    let mult_at_100 = chase.calculate_crossfade_multiplier(Duration::from_secs(100));
    assert!(
        (mult_at_10 - 1.0).abs() < 0.001,
        "Should stay at full intensity after fade-in"
    );
    assert!(
        (mult_at_100 - 1.0).abs() < 0.001,
        "Should stay at full intensity long after fade-in"
    );
}

#[test]
fn test_perpetual_effect_with_up_time_never_reaches_terminal_state() {
    // This test ensures consistency between total_duration() and has_reached_terminal_state()
    // for effects with only up_time set (no hold_time or down_time).
    // Such effects should be treated as perpetual after the fade-in completes.

    let chase = EffectInstance::new(
        "chase".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
        },
        vec!["fixture".to_string()],
        Some(Duration::from_secs(2)), // up_time only - fade in over 2 seconds
        None,                         // no hold_time
        None,                         // no down_time
    );

    // total_duration() should return None (perpetual)
    assert!(
        chase.total_duration().is_none(),
        "Effect with only up_time should have total_duration() = None (perpetual)"
    );

    // Should never reach terminal state, even after fade-in completes
    assert!(
        !chase.has_reached_terminal_state(Duration::from_secs(0)),
        "Should not be terminal at t=0"
    );
    assert!(
        !chase.has_reached_terminal_state(Duration::from_secs(2)),
        "Should not be terminal at t=2s (fade-in complete)"
    );
    assert!(
        !chase.has_reached_terminal_state(Duration::from_secs(10)),
        "Should not be terminal at t=10s (well after fade-in)"
    );
    assert!(
        !chase.has_reached_terminal_state(Duration::from_secs(3600)),
        "Should not be terminal at t=1hr"
    );
}
