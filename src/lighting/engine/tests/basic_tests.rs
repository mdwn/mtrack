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
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    // Should not error if fixture is registered
    assert!(engine.start_effect(effect).is_ok());
}
