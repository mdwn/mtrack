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

use std::time::{Duration, Instant};

#[test]
fn test_dimmer_effect() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
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
    )
    .with_timing(Some(Instant::now()), Some(Duration::from_secs(1)));

    engine.start_effect(effect).unwrap();

    // Update the engine after 500ms (half duration)
    let commands = engine.update(Duration::from_millis(500), None).unwrap();

    // Should have only dimmer command since fixture has dedicated dimmer channel
    // The fixture profile system uses DedicatedDimmer strategy for RGB+dimmer fixtures
    assert_eq!(commands.len(), 1);

    // Check dimmer command
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(dimmer_cmd.value, 127);
}
