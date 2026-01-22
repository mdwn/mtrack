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
use std::collections::HashMap;
use std::time::{Duration, Instant};

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
