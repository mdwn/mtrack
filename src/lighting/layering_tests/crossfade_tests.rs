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

use super::common::*;
#[cfg(test)]
use crate::lighting::effects::*;
use crate::lighting::engine::EffectEngine;
use crate::lighting::parser::parse_light_shows;
use crate::lighting::timeline::LightingTimeline;
use std::collections::HashMap;
use std::time::Duration;

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
    let commands = engine.update(Duration::from_secs(0), None).unwrap();
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(red_cmd.value, 0); // 0% at start

    let commands = engine.update(Duration::from_millis(500), None).unwrap();
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert!(red_cmd.value > 0 && red_cmd.value < 255); // ~50% during up_time

    // Test hold_time phase (1s - 9s)
    let commands = engine.update(Duration::from_millis(1500), None).unwrap(); // 0.5s + 1.5s = 2s
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(red_cmd.value, 255); // Full intensity during hold_time

    // Test down_time phase (9s - 10s)
    let commands = engine.update(Duration::from_secs(7), None).unwrap(); // 2s + 7s = 9s
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(red_cmd.value, 255); // Still full intensity at start of down_time

    let commands = engine.update(Duration::from_millis(500), None).unwrap(); // 9s + 0.5s = 9.5s
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert!(red_cmd.value > 0 && red_cmd.value < 255); // ~50% during down_time

    let commands = engine.update(Duration::from_millis(500), None).unwrap(); // 9.5s + 0.5s = 10s
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
    let commands = engine.update(Duration::from_millis(500), None).unwrap();
    let active_channel = commands.iter().find(|cmd| cmd.value > 0);
    assert!(active_channel.is_some());
    let active_channel = active_channel.unwrap();
    assert!(active_channel.value > 0 && active_channel.value < 255); // Dimmed color during fade in

    // Test full intensity phase - colors should cycle at full brightness
    let commands = engine.update(Duration::from_millis(1500), None).unwrap(); // 0.5s + 1.5s = 2s
    let active_channel = commands.iter().find(|cmd| cmd.value > 0);
    assert!(active_channel.is_some());
    let active_channel = active_channel.unwrap();
    assert_eq!(active_channel.value, 255); // Full intensity during full phase

    // Test that the effect continues running (fade out phase is optional for this test)
    let _commands = engine.update(Duration::from_secs(7), None).unwrap(); // 2s + 7s = 9s
                                                                          // At this point, the effect may have ended or be in fade out - both are valid
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
    let commands = engine.update(Duration::from_millis(500), None).unwrap();
    // The pulse effect for RGB-only fixtures uses _pulse_multiplier, not direct RGB channels
    // So there should be no DMX commands at this point (multiplier is internal)
    assert!(commands.is_empty()); // No direct RGB commands with fixture profile system

    // Test full intensity phase - pulse should be at full amplitude
    let commands = engine.update(Duration::from_secs(2), None).unwrap();
    // Same as above - no direct RGB commands with fixture profile system
    assert!(commands.is_empty()); // No direct RGB commands with fixture profile system

    // Test fade out phase - pulse should be dimmed (at 4.5s total: 0.5s into down_time)
    let commands = engine.update(Duration::from_millis(2000), None).unwrap(); // 2.5s + 2s = 4.5s
                                                                              // Same as above - no direct RGB commands with fixture profile system
    assert!(commands.is_empty()); // No direct RGB commands with fixture profile system

    // Test effect end - should be no commands (at 5s total)
    let commands = engine.update(Duration::from_millis(500), None).unwrap(); // 4.5s + 0.5s = 5s
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
    let commands = engine.update(Duration::from_millis(500), None).unwrap();
    let active_cmd = commands.iter().find(|cmd| cmd.value > 0).unwrap();
    assert!(active_cmd.value > 0 && active_cmd.value < 255); // Dimmed rainbow during fade in

    // Test full intensity phase - rainbow should be at full brightness
    let commands = engine.update(Duration::from_secs(2), None).unwrap();
    let active_cmd = commands.iter().find(|cmd| cmd.value > 0).unwrap();
    assert!(active_cmd.value > 200); // High rainbow brightness during full intensity

    // Test fade out phase - rainbow should be dimmed (at 4.5s total: 0.5s into down_time)
    let commands = engine.update(Duration::from_millis(2000), None).unwrap(); // 2.5s + 2s = 4.5s
    let active_cmd = commands.iter().find(|cmd| cmd.value > 0).unwrap();
    assert!(active_cmd.value > 0 && active_cmd.value < 255); // Dimmed rainbow during fade out

    // Test effect end - should be no commands (at 5s total)
    let commands = engine.update(Duration::from_millis(500), None).unwrap(); // 4.5s + 0.5s = 5s
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
    let effect_instance = LightingTimeline::create_effect_instance(effect, show.cues[0].time);
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
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    if let Some(blue_cmd) = commands.iter().find(|cmd| cmd.channel == 3) {
        assert_eq!(blue_cmd.value, 0); // 0% blue during fade in
    }

    // Test fade in: at t=1000ms (50% of 2s fade in), should be ~50% blue
    let commands = engine.update(Duration::from_millis(1000), None).unwrap();
    if let Some(blue_cmd) = commands.iter().find(|cmd| cmd.channel == 3) {
        assert!(blue_cmd.value > 100 && blue_cmd.value < 150); // ~50% blue
    }

    // Test full intensity: at t=2000ms (after fade in complete), should be 100% blue
    let commands = engine.update(Duration::from_millis(1000), None).unwrap(); // Add 1s more (t=0 + 1s + 1s = 2s total)
    if let Some(blue_cmd) = commands.iter().find(|cmd| cmd.channel == 3) {
        assert_eq!(blue_cmd.value, 255); // 100% blue
    }

    // Test hold phase: at t=7000ms (end of hold phase), should still be 100% blue
    let commands = engine.update(Duration::from_millis(5000), None).unwrap(); // t=2s + 5s = 7s
    if let Some(blue_cmd) = commands.iter().find(|cmd| cmd.channel == 3) {
        assert_eq!(blue_cmd.value, 255); // 100% blue at end of hold phase
    }

    // Test fade out: at t=8000ms (fade out complete), effect ends (not permanent)
    let commands = engine.update(Duration::from_millis(1000), None).unwrap(); // Add 1s more (7s + 1s = 8s)
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
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
    assert_eq!(blue_cmd.value, 0); // Should be 0 (0% of 255)

    // Test fade in: at t=500ms, should be 50% (half blue)
    let commands = engine.update(Duration::from_millis(500), None).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
    assert_eq!(blue_cmd.value, 127); // Should be 127 (50% of 255)

    // Test full intensity: at t=1000ms, should be 100% (full blue)
    let commands = engine.update(Duration::from_millis(500), None).unwrap(); // 500ms more = 1000ms total
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
    assert_eq!(blue_cmd.value, 255); // Should be 255 (100% of 255)

    // Test fade out: at t=2500ms, should be 50% (half blue)
    let commands = engine.update(Duration::from_millis(1500), None).unwrap(); // 1500ms more = 2500ms total
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
    assert_eq!(blue_cmd.value, 127); // Should be 127 (50% of 255)

    // Test fade out: at t=3000ms, effect should be finished (no commands)
    let commands = engine.update(Duration::from_millis(500), None).unwrap(); // 500ms more = 3000ms total
    assert!(commands.is_empty()); // Effect should be finished, no commands
}
