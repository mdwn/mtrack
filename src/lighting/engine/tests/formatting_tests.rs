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
fn test_format_active_effects() {
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    // Test with no effects
    let output = engine.format_active_effects();
    assert_eq!(output, "No active effects");

    // Add a static effect
    let mut params = HashMap::new();
    params.insert("red".to_string(), 1.0);
    let effect = EffectInstance::new(
        "test_effect_1".to_string(),
        EffectType::Static {
            parameters: params,
            duration: None,
        },
        vec!["test_fixture".to_string()],
        None,
        Some(Duration::from_secs(5)),
        None,
    );
    engine.start_effect(effect).unwrap();

    // Add a chase effect on a different layer
    let mut chase_effect = EffectInstance::new(
        "test_effect_2".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
        },
        vec!["test_fixture".to_string()],
        None,
        None,
        None,
    );
    chase_effect.layer = EffectLayer::Foreground;
    engine.start_effect(chase_effect).unwrap();

    // Format and verify output
    let output = engine.format_active_effects();
    assert!(output.contains("Active effects (2)"));
    assert!(output.contains("Background"));
    assert!(output.contains("Foreground"));
    assert!(output.contains("test_effect_1"));
    assert!(output.contains("test_effect_2"));
    assert!(output.contains("Static"));
    assert!(output.contains("Chase"));
    assert!(output.contains("1 fixture(s)"));
}
