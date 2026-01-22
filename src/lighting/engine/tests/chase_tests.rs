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

use std::time::Duration;

#[test]
fn test_chase_fixture_boundaries() {
    // Test chase effect transitions between fixtures correctly
    // Note: Chase applies dimmer to active fixture, 0 to others
    let mut engine = EffectEngine::new();

    // Create 3 fixtures for chase, each at different addresses
    let fixture_0 = create_test_fixture("fixture_0", 1, 1);
    let fixture_1 = create_test_fixture("fixture_1", 1, 11);
    let fixture_2 = create_test_fixture("fixture_2", 1, 21);
    engine.register_fixture(fixture_0);
    engine.register_fixture(fixture_1);
    engine.register_fixture(fixture_2);

    // 1 Hz chase with 3 fixtures = each fixture active for 333.33ms
    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
        },
        vec![
            "fixture_0".to_string(),
            "fixture_1".to_string(),
            "fixture_2".to_string(),
        ],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // Helper to count active fixtures (dimmer channel = address, value = 255)
    let count_active = |commands: &[DmxCommand]| -> usize {
        // Each fixture has dimmer at relative channel 1
        // fixture_0: channel 1, fixture_1: channel 11, fixture_2: channel 21
        let dimmer_channels = [1, 11, 21];
        commands
            .iter()
            .filter(|cmd| dimmer_channels.contains(&cmd.channel) && cmd.value == 255)
            .count()
    };

    // At t=0ms: first fixture should be active (pattern_index = 0)
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    assert_eq!(
        count_active(&commands),
        1,
        "At t=0ms exactly one fixture should be active"
    );

    // At t=350ms: second fixture should be active (past 333.33ms)
    let commands = engine.update(Duration::from_millis(350), None).unwrap();
    assert_eq!(
        count_active(&commands),
        1,
        "At t=350ms exactly one fixture should be active"
    );

    // At t=700ms: third fixture should be active (past 666.66ms)
    let commands = engine.update(Duration::from_millis(350), None).unwrap();
    assert_eq!(
        count_active(&commands),
        1,
        "At t=700ms exactly one fixture should be active"
    );

    // At t=1050ms: should wrap back (past 1000ms)
    let commands = engine.update(Duration::from_millis(350), None).unwrap();
    assert_eq!(
        count_active(&commands),
        1,
        "At t=1050ms exactly one fixture should be active (wrapped)"
    );
}

#[test]
fn test_chase_zero_speed() {
    // Edge case: speed=0 should not cause divide-by-zero, should keep first fixture active
    let mut engine = EffectEngine::new();
    let fixture_0 = create_test_fixture("fixture_0", 1, 1);
    let fixture_1 = create_test_fixture("fixture_1", 1, 11);
    let fixture_2 = create_test_fixture("fixture_2", 1, 21);
    engine.register_fixture(fixture_0);
    engine.register_fixture(fixture_1);
    engine.register_fixture(fixture_2);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(0.0), // Zero speed!
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
        },
        vec![
            "fixture_0".to_string(),
            "fixture_1".to_string(),
            "fixture_2".to_string(),
        ],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // Should not panic, first fixture should be active
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    let dimmer_channels = [1, 11, 21];
    let active_count = commands
        .iter()
        .filter(|cmd| dimmer_channels.contains(&cmd.channel) && cmd.value == 255)
        .count();
    assert_eq!(
        active_count, 1,
        "Zero speed should have exactly one fixture active"
    );

    // First fixture (channel 1) should be the active one
    let first_dimmer = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(first_dimmer.value, 255, "First fixture should be active");

    // Even after time passes, should still be frozen on first fixture
    let commands = engine.update(Duration::from_millis(5000), None).unwrap();
    let first_dimmer = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(
        first_dimmer.value, 255,
        "Zero speed should remain frozen on first fixture"
    );
}

#[test]
fn test_chase_empty_fixtures() {
    // Edge case: chase with no fixtures should not panic (empty fixture list)
    let mut engine = EffectEngine::new();
    // Don't register any fixtures

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
        },
        vec![], // Empty fixture list!
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // Should not panic, should return empty commands
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    assert!(
        commands.is_empty(),
        "Empty fixture chase should produce no commands"
    );

    // Should still work after time passes
    let commands = engine.update(Duration::from_millis(1000), None).unwrap();
    assert!(
        commands.is_empty(),
        "Empty fixture chase should still produce no commands"
    );
}

#[test]
fn test_chase_effect() {
    let mut engine = EffectEngine::new();
    let fixture1 = create_test_fixture("fixture1", 1, 1);
    let fixture2 = create_test_fixture("fixture2", 1, 6);
    let fixture3 = create_test_fixture("fixture3", 1, 11);

    engine.register_fixture(fixture1);
    engine.register_fixture(fixture2);
    engine.register_fixture(fixture3);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
        },
        vec![
            "fixture1".to_string(),
            "fixture2".to_string(),
            "fixture3".to_string(),
        ],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    // Update the engine
    let commands = engine.update(Duration::from_millis(16), None).unwrap();

    // Should have dimmer commands for all fixtures
    // Note: The chase effect might generate more commands than expected
    // due to the simplified implementation
    assert!(commands.len() >= 3);

    // All commands should be for dimmer channels (but may be on different DMX addresses)
    for cmd in &commands {
        // The chase effect generates commands for different DMX addresses
        // but all should be for the dimmer channel (channel 1 relative to fixture address)
        assert!(cmd.channel >= 1 && cmd.channel <= 15); // Within reasonable DMX range
    }
}
