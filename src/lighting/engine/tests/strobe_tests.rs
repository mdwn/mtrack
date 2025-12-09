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
