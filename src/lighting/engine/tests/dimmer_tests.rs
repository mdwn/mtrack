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

#[test]
fn test_dimmer_effect_dim_down() {
    // Test dimming from higher to lower values (e.g., 1.0 -> 0.0)
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Dimmer {
            start_level: 1.0,
            end_level: 0.0,
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

    // At start (0ms), dimmer should be at start_level (1.0 = 255)
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    assert_eq!(commands.len(), 1);
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(dimmer_cmd.value, 255, "At start, dimmer should be at 100%");

    // At midpoint (500ms), dimmer should be at 50% (127)
    let commands = engine.update(Duration::from_millis(500), None).unwrap();
    assert_eq!(commands.len(), 1);
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(
        dimmer_cmd.value, 127,
        "At midpoint, dimmer should be at 50%"
    );

    // At end (1000ms), dimmer should be at end_level (0.0 = 0)
    let commands = engine.update(Duration::from_millis(500), None).unwrap();
    assert_eq!(commands.len(), 1);
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(dimmer_cmd.value, 0, "At end, dimmer should be at 0%");
}

#[test]
fn test_dimmer_effect_dim_down_partial() {
    // Test dimming from a partial level to another partial level (e.g., 0.8 -> 0.3)
    let mut engine = EffectEngine::new();
    let fixture = create_test_fixture("test_fixture", 1, 1);
    engine.register_fixture(fixture);

    let effect = EffectInstance::new(
        "test_effect".to_string(),
        EffectType::Dimmer {
            start_level: 0.8,
            end_level: 0.3,
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

    // At start (0ms), dimmer should be at start_level (0.8 = 204)
    let commands = engine.update(Duration::from_millis(0), None).unwrap();
    assert_eq!(commands.len(), 1);
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    assert_eq!(
        dimmer_cmd.value, 204,
        "At start, dimmer should be at 80% (204)"
    );

    // At midpoint (500ms), dimmer should be halfway between 0.8 and 0.3 = 0.55 (140)
    let commands = engine.update(Duration::from_millis(500), None).unwrap();
    assert_eq!(commands.len(), 1);
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    // 0.55 * 255 = 140.25, so expect 140 or 141
    assert!(
        (140..=141).contains(&dimmer_cmd.value),
        "At midpoint, dimmer should be at ~55% (140-141), got {}",
        dimmer_cmd.value
    );

    // At end (1000ms), dimmer should be at end_level (0.3 = 76)
    let commands = engine.update(Duration::from_millis(500), None).unwrap();
    assert_eq!(commands.len(), 1);
    let dimmer_cmd = commands.iter().find(|cmd| cmd.channel == 1).unwrap();
    // 0.3 * 255 = 76.5, so expect 76 or 77
    assert!(
        (76..=77).contains(&dimmer_cmd.value),
        "At end, dimmer should be at ~30% (76-77), got {}",
        dimmer_cmd.value
    );
}
