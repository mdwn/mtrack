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
fn test_static_effect() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let mut parameters = HashMap::new();
    parameters.insert("dimmer".to_string(), 0.5);
    parameters.insert("red".to_string(), 1.0);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Static {
            parameters: parameters.clone(),
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

    // Should have commands for dimmer and red channels
    assert_eq!(commands.len(), 2);

    // Check dimmer command (50% = 127)
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(dimmer_cmd.value, 127);

    // Check red command (100% = 255)
    let red_cmd = commands.iter().find(|cmd| cmd.channel == 2).unwrap();
    assert_eq!(red_cmd.value, 255);
}
