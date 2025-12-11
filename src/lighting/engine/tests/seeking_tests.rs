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
use crate::lighting::effects::*;
use crate::lighting::engine::tests::common::create_test_fixture;
use crate::lighting::engine::EffectEngine;
use std::collections::HashMap;
use std::time::Duration;

#[test]
fn test_start_effect_with_elapsed_static_effect() {
    // Test starting a static effect with pre-calculated elapsed time
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let mut params = HashMap::new();
    params.insert("dimmer".to_string(), 1.0);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Static {
            parameters: params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    // Start with 500ms elapsed - effect should appear as if it's been running for 500ms
    engine
        .start_effect_with_elapsed(effect, Duration::from_millis(500))
        .unwrap();

    // Update immediately - should get full value (static effects don't change over time)
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].value, 255);
}

#[test]
fn test_start_effect_with_elapsed_dimmer_midpoint() {
    // Test starting a dimmer effect at its midpoint
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let effect = EffectInstance::new(
        "dimmer".to_string(),
        EffectType::Dimmer {
            start_level: 0.0,
            end_level: 1.0,
            duration: Duration::from_secs(2), // 2 second fade
            curve: DimmerCurve::Linear,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    // Start with 1 second elapsed (halfway through 2 second fade)
    engine
        .start_effect_with_elapsed(effect, Duration::from_secs(1))
        .unwrap();

    // Update immediately - should be at ~50%
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    assert_eq!(commands.len(), 1);
    // Should be approximately 50% (127-128)
    assert!(
        (120..=135).contains(&commands[0].value),
        "Expected ~50% dimmer value, got {}",
        commands[0].value
    );
}

#[test]
fn test_start_effect_with_elapsed_dimmer_complete() {
    // Test starting a dimmer effect that has already completed
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let effect = EffectInstance::new(
        "dimmer".to_string(),
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
    );

    // Start with 2 seconds elapsed (beyond the 1 second duration)
    engine
        .start_effect_with_elapsed(effect, Duration::from_secs(2))
        .unwrap();

    // Update immediately - should be at 100% (completed)
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].value, 255);
}

#[test]
fn test_start_effect_with_elapsed_color_cycle() {
    // Test starting a color cycle effect partway through
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let colors = vec![
        Color::new(255, 0, 0), // Red
        Color::new(0, 255, 0), // Green
        Color::new(0, 0, 255), // Blue
    ];

    let effect = EffectInstance::new(
        "cycle".to_string(),
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

    // Start with 0.5 seconds elapsed (halfway through first cycle)
    // At 1 cycle/sec with 3 colors, each color gets 1/3 of the cycle
    // At 0.5s progress, we're at color index 1 (green) with Snap transition
    engine
        .start_effect_with_elapsed(effect, Duration::from_millis(500))
        .unwrap();

    // Update immediately - with Snap, should be pure green (no transition)
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    assert_eq!(commands.len(), 3); // RGB channels

    // Find RGB values (channels: dimmer=1, red=2, green=3, blue=4)
    let mut red = 0;
    let mut green = 0;
    let mut blue = 0;
    for cmd in &commands {
        if cmd.channel == 2 {
            red = cmd.value;
        } else if cmd.channel == 3 {
            green = cmd.value;
        } else if cmd.channel == 4 {
            blue = cmd.value;
        }
    }

    // With Snap transition at 0.5s, should be pure green
    assert_eq!(red, 0, "Red should be 0 at this point in cycle");
    assert_eq!(green, 255, "Green should be full at this point in cycle");
    assert_eq!(blue, 0, "Blue should be 0");
}

#[test]
fn test_start_effect_with_elapsed_pulse() {
    // Test starting a pulse effect partway through its cycle
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let effect = EffectInstance::new(
        "pulse".to_string(),
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

    // Start with 0.25 seconds elapsed (quarter cycle at 1Hz = 90 degrees)
    // At 90 degrees, sin = 1.0, so pulse should be at max (base + amplitude = 1.0)
    engine
        .start_effect_with_elapsed(effect, Duration::from_millis(250))
        .unwrap();

    // Update immediately - should be near maximum
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    assert_eq!(commands.len(), 1);
    // Should be near 100% (allowing for small timing differences)
    assert!(
        commands[0].value > 240,
        "Pulse should be near max at 90 degrees, got {}",
        commands[0].value
    );
}

#[test]
fn test_start_effect_with_elapsed_rainbow() {
    // Test starting a rainbow effect partway through
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let effect = EffectInstance::new(
        "rainbow".to_string(),
        EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(1.0), // 1 cycle per second
            saturation: 1.0,
            brightness: 1.0,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    // Start with 0.33 seconds elapsed (1/3 through cycle = 120 degrees hue)
    // At 120 degrees, we should see green-cyan transition
    engine
        .start_effect_with_elapsed(effect, Duration::from_millis(333))
        .unwrap();

    // Update immediately - should show appropriate color for that hue
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    assert_eq!(commands.len(), 3); // RGB channels

    // Find RGB values
    let mut red = 0;
    let mut green = 0;
    let mut blue = 0;
    for cmd in &commands {
        if cmd.channel == 2 {
            red = cmd.value;
        } else if cmd.channel == 3 {
            green = cmd.value;
        } else if cmd.channel == 4 {
            blue = cmd.value;
        }
    }

    // At 120 degrees hue, green should be high, red low, blue medium
    assert!(green > red, "Green should dominate at 120 degrees");
    assert!(green > blue, "Green should be highest");
}

#[test]
fn test_start_effect_with_elapsed_timed_effect_completion() {
    // Test that timed effects complete correctly when started with elapsed time
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let mut params = HashMap::new();
    params.insert("dimmer".to_string(), 1.0);

    let effect = EffectInstance::new(
        "timed".to_string(),
        EffectType::Static {
            parameters: params,
            duration: Some(Duration::from_secs(1)),
        },
        vec!["test_fixture".to_string()],
        None,
        Some(Duration::from_secs(1)),
        None,
    );

    // Start with 1.5 seconds elapsed (beyond the 1 second duration)
    engine
        .start_effect_with_elapsed(effect, Duration::from_millis(1500))
        .unwrap();

    // Update immediately - effect should already be expired
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    // Effect should have completed immediately
    assert_eq!(commands.len(), 0);
}

#[test]
fn test_start_effect_with_elapsed_zero_elapsed() {
    // Test that zero elapsed time works the same as start_effect
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let mut params = HashMap::new();
    params.insert("dimmer".to_string(), 0.5);

    let effect1 = EffectInstance::new(
        "effect1".to_string(),
        EffectType::Static {
            parameters: params.clone(),
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect1).unwrap();
    let commands1 = engine.update(Duration::from_millis(16), None).unwrap();

    // Reset and try with zero elapsed
    engine.stop_all_effects();

    let effect2 = EffectInstance::new(
        "effect2".to_string(),
        EffectType::Static {
            parameters: params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );

    engine
        .start_effect_with_elapsed(effect2, Duration::ZERO)
        .unwrap();
    let commands2 = engine.update(Duration::from_millis(16), None).unwrap();

    // Should produce same results
    assert_eq!(commands1.len(), commands2.len());
    if !commands1.is_empty() && !commands2.is_empty() {
        assert_eq!(commands1[0].value, commands2[0].value);
    }
}

#[test]
fn test_start_effect_with_elapsed_conflict_resolution() {
    // Test that conflict resolution works correctly with elapsed time
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Start a background effect
    let mut bg_params = HashMap::new();
    bg_params.insert("dimmer".to_string(), 0.3);

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
    engine.update(Duration::from_millis(16), None).unwrap();

    // Start a foreground effect with elapsed time - should conflict and replace
    let mut fg_params = HashMap::new();
    fg_params.insert("dimmer".to_string(), 0.8);

    let mut fg_effect = EffectInstance::new(
        "fg".to_string(),
        EffectType::Static {
            parameters: fg_params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    fg_effect.layer = EffectLayer::Foreground;
    fg_effect.priority = 10;

    engine
        .start_effect_with_elapsed(fg_effect, Duration::from_millis(500))
        .unwrap();

    // Update - should see foreground effect
    let commands = engine.update(Duration::from_millis(16), None).unwrap();
    assert_eq!(commands.len(), 1);
    assert_eq!(commands[0].value, 204); // 80% of 255
}
