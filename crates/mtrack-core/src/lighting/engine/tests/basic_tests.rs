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
use std::time::Duration;

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
            duration: Duration::from_secs(5),
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    // Should not error if fixture is registered
    assert!(engine.start_effect(effect).is_ok());
}

/// The channels that go to 0 once no frame writes them are worked out again
/// when a fixture is registered: one hung after a frame has gone out has
/// its level released as any other's is.
#[test]
fn a_fixture_registered_after_a_frame_has_its_level_released() {
    use crate::lighting::effects::DmxCommand;
    let on = |channel| DmxCommand {
        universe: 1,
        channel,
        value: 200,
    };
    let mut engine = EffectEngine::new();
    engine.register_fixture(create_test_fixture("first", 1, 1));
    let first: Vec<u16> = engine.level_channels().iter().map(|&(_, c)| c).collect();
    engine.send(&[on(first[0])]);

    engine.register_fixture(create_test_fixture("second", 1, 100));
    let second = engine
        .level_channels()
        .iter()
        .map(|&(_, c)| c)
        .find(|c| !first.contains(c))
        .expect("the second fixture has a level of its own");
    engine.send(&[on(second)]);
    assert_eq!(engine.wire().byte(1, second), Some(200));

    let released = engine.send(&[]);
    assert!(released.iter().any(|c| c.channel == second && c.value == 0));
    assert_eq!(engine.wire().byte(1, second), Some(0));
}
