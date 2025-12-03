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

use super::common::*;
#[cfg(test)]
use crate::lighting::effects::*;
use crate::lighting::engine::EffectEngine;
use std::collections::HashMap;
use std::time::Duration;

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
