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

use std::time::Duration;

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
