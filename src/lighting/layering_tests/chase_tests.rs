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
use std::collections::HashMap;
use std::time::Duration;

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
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    let fixture1_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(fixture1_cmd.value, 255); // Should be ON

    // At t=125ms (1/4 cycle) - fixture_2 should be ON
    let commands = engine.update(Duration::from_millis(125), None).unwrap();
    let fixture2_cmd = commands.iter().find(|cmd| cmd.channel == 5).unwrap(); // fixture_2 dimmer
    assert_eq!(fixture2_cmd.value, 255); // Should be ON

    // At t=250ms (1/2 cycle) - fixture_3 should be ON
    let commands = engine.update(Duration::from_millis(125), None).unwrap(); // 125ms more = 250ms total
    let fixture3_cmd = commands.iter().find(|cmd| cmd.channel == 9).unwrap(); // fixture_3 dimmer
    assert_eq!(fixture3_cmd.value, 255); // Should be ON

    // At t=375ms (3/4 cycle) - fixture_4 should be ON
    let commands = engine.update(Duration::from_millis(125), None).unwrap(); // 125ms more = 375ms total
    let fixture4_cmd = commands.iter().find(|cmd| cmd.channel == 13).unwrap(); // fixture_4 dimmer
    assert_eq!(fixture4_cmd.value, 255); // Should be ON

    // At t=500ms (full cycle) - fixture_1 should be ON again
    let commands = engine.update(Duration::from_millis(125), None).unwrap(); // 125ms more = 500ms total
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
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    let fixture4_cmd = commands.iter().find(|cmd| cmd.channel == 13).unwrap(); // fixture_4 dimmer
    assert_eq!(fixture4_cmd.value, 255); // Should be ON

    // At t=125ms (1/4 cycle) - fixture_3 should be ON
    let commands = engine.update(Duration::from_millis(125), None).unwrap();
    let fixture3_cmd = commands.iter().find(|cmd| cmd.channel == 9).unwrap(); // fixture_3 dimmer
    assert_eq!(fixture3_cmd.value, 255); // Should be ON

    // At t=250ms (1/2 cycle) - fixture_2 should be ON
    let commands = engine.update(Duration::from_millis(125), None).unwrap(); // 125ms more = 250ms total
    let fixture2_cmd = commands.iter().find(|cmd| cmd.channel == 5).unwrap(); // fixture_2 dimmer
    assert_eq!(fixture2_cmd.value, 255); // Should be ON

    // At t=375ms (3/4 cycle) - fixture_1 should be ON
    let commands = engine.update(Duration::from_millis(125), None).unwrap(); // 125ms more = 375ms total
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
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    let fixture1_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(fixture1_cmd.value, 255); // Should be ON

    // At t=125ms (1/6 cycle) - fixture_2 should be ON
    let commands = engine.update(Duration::from_millis(125), None).unwrap();
    let fixture2_cmd = commands.iter().find(|cmd| cmd.channel == 5).unwrap();
    assert_eq!(fixture2_cmd.value, 255); // Should be ON

    // At t=250ms (2/6 cycle) - fixture_3 should be ON
    let commands = engine.update(Duration::from_millis(125), None).unwrap(); // 125+125=250ms total
    let fixture3_cmd = commands.iter().find(|cmd| cmd.channel == 9).unwrap();
    assert_eq!(fixture3_cmd.value, 255); // Should be ON

    // At t=375ms (3/6 cycle) - fixture_4 should be ON
    let commands = engine.update(Duration::from_millis(125), None).unwrap(); // 250+125=375ms total
    let fixture4_cmd = commands.iter().find(|cmd| cmd.channel == 13).unwrap();
    assert_eq!(fixture4_cmd.value, 255); // Should be ON

    // At t=500ms (4/6 cycle) - fixture_3 should be ON (snake back)
    let commands = engine.update(Duration::from_millis(125), None).unwrap(); // 375+125=500ms total
    let fixture3_cmd = commands.iter().find(|cmd| cmd.channel == 9).unwrap();
    assert_eq!(fixture3_cmd.value, 255); // Should be ON

    // At t=625ms (5/6 cycle) - fixture_2 should be ON (snake back)
    let commands = engine.update(Duration::from_millis(125), None).unwrap(); // 500+125=625ms total
    let fixture2_cmd = commands.iter().find(|cmd| cmd.channel == 5).unwrap();
    assert_eq!(fixture2_cmd.value, 255); // Should be ON

    // At t=750ms (6/6 cycle) - fixture_1 should be ON again
    let commands = engine.update(Duration::from_millis(125), None).unwrap(); // 625+125=750ms total
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
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    let on_fixtures: Vec<_> = commands.iter().filter(|cmd| cmd.value == 255).collect();
    assert_eq!(on_fixtures.len(), 1); // Exactly one fixture should be ON

    // At t=125ms - some fixture should be ON
    let commands = engine.update(Duration::from_millis(125), None).unwrap();
    let on_fixtures: Vec<_> = commands.iter().filter(|cmd| cmd.value == 255).collect();
    assert_eq!(on_fixtures.len(), 1); // Exactly one fixture should be ON

    // At t=250ms - some fixture should be ON
    let commands = engine.update(Duration::from_millis(125), None).unwrap(); // 125ms more = 250ms total
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
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
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
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
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
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    let fixture1_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(fixture1_cmd.value, 255); // Should be ON

    // At t=600ms (1/3 cycle) - fixture_1 should still be ON
    let commands = engine.update(Duration::from_millis(600), None).unwrap();
    let fixture1_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(fixture1_cmd.value, 255); // Should still be ON

    // At t=1200ms (2/3 cycle) - fixture_2 should be ON
    let commands = engine.update(Duration::from_millis(600), None).unwrap(); // 600ms more = 1200ms total
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
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    let fixture_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(fixture_cmd.value, 255); // Should be ON

    // At any time, single fixture should be ON
    let commands = engine.update(Duration::from_millis(500), None).unwrap();
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
    let commands = engine.update(Duration::from_millis(0), None).unwrap();

    // fixture_1 should have all RGB channels ON
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 3).unwrap();
    assert_eq!(red_cmd.value, 255);
    assert_eq!(green_cmd.value, 255);
    assert_eq!(blue_cmd.value, 255);

    // At t=167ms (1/3 cycle) - fixture_2 should be ON
    let commands = engine.update(Duration::from_millis(167), None).unwrap();
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 4).unwrap(); // fixture_2 red
    let green_cmd = commands.iter().find(|cmd| cmd.channel == 5).unwrap(); // fixture_2 green
    let blue_cmd = commands.iter().find(|cmd| cmd.channel == 6).unwrap(); // fixture_2 blue
    assert_eq!(red_cmd.value, 255);
    assert_eq!(green_cmd.value, 255);
    assert_eq!(blue_cmd.value, 255);
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
    let commands = engine.update(Duration::from_millis(500), None).unwrap();
    let active_fixture = commands.iter().find(|cmd| cmd.value > 0);
    assert!(active_fixture.is_some());
    if let Some(cmd) = active_fixture {
        assert!(cmd.value > 0 && cmd.value < 255); // Dimmed chase during fade in
    }

    // Test full intensity phase - chase should be at full brightness
    let commands = engine.update(Duration::from_secs(2), None).unwrap();
    let active_fixture = commands.iter().find(|cmd| cmd.value > 0);
    assert!(active_fixture.is_some());
    if let Some(cmd) = active_fixture {
        assert_eq!(cmd.value, 255); // Full brightness during full intensity
    }

    // Test fade out phase - chase should be dimmed (at 3.5s total: 0.5s into down_time)
    let commands = engine.update(Duration::from_millis(1000), None).unwrap(); // 2.5s + 1s = 3.5s
    let active_fixture = commands.iter().find(|cmd| cmd.value > 0);
    assert!(active_fixture.is_some());
    if let Some(cmd) = active_fixture {
        assert!(cmd.value > 0 && cmd.value < 255); // Dimmed chase during fade out
    }

    // Test effect end - should be no commands (at 4s total)
    let commands = engine.update(Duration::from_millis(500), None).unwrap(); // 3.5s + 0.5s = 4s
    assert!(commands.is_empty()); // Effect should be finished
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
    let mut active_fixtures: std::collections::HashSet<usize> = std::collections::HashSet::new();

    // Check over multiple time points to see if pattern is advancing
    for _step in 0..20 {
        let cmds = engine.update(Duration::from_millis(50), None).unwrap();
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
