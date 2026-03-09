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
use super::common::create_test_fixture;
use crate::dmx::legacy_store::LegacyDmxStore;
use crate::lighting::effects::{EffectType, FixtureInfo};
use crate::lighting::engine::EffectEngine;
use crate::lighting::EffectInstance;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Helper: create a LegacyDmxStore with slots matching create_test_fixture("wash1", 1, 1).
/// The fixture has channels dimmer=1, red=2, green=3, blue=4, white=5, strobe=6
/// at universe 1, address 1, so DMX channels are 1–6.
fn create_store_for_wash1() -> Arc<parking_lot::RwLock<LegacyDmxStore>> {
    let mut store = LegacyDmxStore::new();
    store.register_slot(1, 1, "wash1", "dimmer"); // address 1 + offset 1 - 1 = 1
    store.register_slot(1, 2, "wash1", "red"); // address 1 + offset 2 - 1 = 2
    store.register_slot(1, 3, "wash1", "green"); // address 1 + offset 3 - 1 = 3
    store.register_slot(1, 4, "wash1", "blue"); // address 1 + offset 4 - 1 = 4
    store.register_slot(1, 5, "wash1", "white"); // address 1 + offset 5 - 1 = 5
    store.register_slot(1, 6, "wash1", "strobe"); // address 1 + offset 6 - 1 = 6
    store.register_universe(1);
    Arc::new(parking_lot::RwLock::new(store))
}

#[test]
fn test_reverse_map_construction() {
    let mut engine = EffectEngine::new();

    // Create fixture at universe 1, address 10, with channels dimmer=1, red=2, green=3, blue=4
    let mut channels = HashMap::new();
    channels.insert("dimmer".to_string(), 1);
    channels.insert("red".to_string(), 2);
    channels.insert("green".to_string(), 3);
    channels.insert("blue".to_string(), 4);

    let fixture = FixtureInfo::new(
        "wash1".to_string(),
        1,
        10,
        "RGBW_Par".to_string(),
        channels,
        None,
    );

    engine.register_fixture(fixture);

    // dmx_channel = address + offset - 1
    // dimmer: 10 + 1 - 1 = 10
    // red:    10 + 2 - 1 = 11
    // green:  10 + 3 - 1 = 12
    // blue:   10 + 4 - 1 = 13
    assert_eq!(
        engine.lookup_dmx_channel(1, 10),
        Some(&("wash1".to_string(), "dimmer".to_string()))
    );
    assert_eq!(
        engine.lookup_dmx_channel(1, 11),
        Some(&("wash1".to_string(), "red".to_string()))
    );
    assert_eq!(
        engine.lookup_dmx_channel(1, 12),
        Some(&("wash1".to_string(), "green".to_string()))
    );
    assert_eq!(
        engine.lookup_dmx_channel(1, 13),
        Some(&("wash1".to_string(), "blue".to_string()))
    );

    // Unmapped channel returns None
    assert_eq!(engine.lookup_dmx_channel(1, 14), None);
    // Wrong universe returns None
    assert_eq!(engine.lookup_dmx_channel(2, 10), None);
}

#[test]
fn test_legacy_store_values_in_merged_states() {
    let mut engine = EffectEngine::new();

    let fixture = create_test_fixture("wash1", 1, 1);
    engine.register_fixture(fixture);

    let store = create_store_for_wash1();
    engine.set_legacy_store(store.clone());

    // Write values via the store (simulating legacy MIDI writes)
    // Values are in 0–255 DMX scale
    {
        let s = store.read();
        s.write(1, 2, 255, false); // red = 1.0 normalized
        s.write(1, 3, 128, false); // green ≈ 0.502 normalized
        s.write(1, 4, 0, false); // blue = 0.0 normalized
        s.tick();
    }

    // Run update to compute merged states
    let _commands = engine.update(Duration::from_millis(23), None).unwrap();

    // Check that store values appear in merged states
    let states = engine.get_fixture_states();
    let wash1_state = states
        .get("wash1")
        .expect("wash1 should be in merged states");
    assert!(
        (wash1_state.channels.get("red").unwrap().value - 1.0).abs() < 0.01,
        "red should be ~1.0"
    );
    assert!(
        (wash1_state.channels.get("green").unwrap().value - 128.0 / 255.0).abs() < 0.01,
        "green should be ~0.502"
    );
    assert!(
        wash1_state.channels.get("blue").unwrap().value.abs() < f64::EPSILON,
        "blue should be 0.0"
    );
}

#[test]
fn test_legacy_store_values_generate_dmx_commands() {
    // With the new single-path architecture, legacy values flow through the
    // EffectEngine and generate DmxCommands (no suppression).
    let mut engine = EffectEngine::new();

    let fixture = create_test_fixture("wash1", 1, 1);
    engine.register_fixture(fixture);

    let store = create_store_for_wash1();
    engine.set_legacy_store(store.clone());

    // Write red value via the store
    {
        let s = store.read();
        s.write(1, 2, 255, false); // red channel, DMX channel 2
        s.tick();
    }

    let commands = engine.update(Duration::from_millis(23), None).unwrap();

    // The red channel's DMX command should now be present (no suppression)
    let red_cmd = commands
        .iter()
        .find(|cmd| cmd.universe == 1 && cmd.channel == 2);
    assert!(
        red_cmd.is_some(),
        "DMX command for legacy channel (universe 1, channel 2) should be generated"
    );
    assert_eq!(red_cmd.unwrap().value, 255);
}

#[test]
fn test_legacy_store_values_overridden_by_effects() {
    let mut engine = EffectEngine::new();

    let fixture = create_test_fixture("wash1", 1, 1);
    engine.register_fixture(fixture);

    let store = create_store_for_wash1();
    engine.set_legacy_store(store.clone());

    // Set legacy store value for red = 128 (0.502 normalized)
    {
        let s = store.read();
        s.write(1, 2, 128, false);
        s.tick();
    }

    // Start an effect that sets red = 1.0
    let mut parameters = HashMap::new();
    parameters.insert("red".to_string(), 1.0);

    let effect = EffectInstance::new(
        "override_test".to_string(),
        EffectType::Static {
            parameters,
            duration: None,
        },
        vec!["wash1".to_string()],
        None,
        None,
        None,
    );

    engine.start_effect(effect).unwrap();

    let _commands = engine.update(Duration::from_millis(23), None).unwrap();

    // The effect should override the legacy store value
    let states = engine.get_fixture_states();
    let wash1_state = states.get("wash1").unwrap();
    assert!(
        (wash1_state.channels.get("red").unwrap().value - 1.0).abs() < f64::EPSILON,
        "effect should override legacy store value: got {}",
        wash1_state.channels.get("red").unwrap().value
    );
}

#[test]
fn test_legacy_store_values_update_across_frames() {
    // Regression test: legacy values must reflect new MIDI writes on subsequent frames.
    let mut engine = EffectEngine::new();

    let fixture = create_test_fixture("wash1", 1, 1);
    engine.register_fixture(fixture);

    let store = create_store_for_wash1();
    engine.set_legacy_store(store.clone());

    // Frame 1: set red = 128 (~0.502)
    {
        let s = store.read();
        s.write(1, 2, 128, false);
        s.tick();
    }
    let _commands = engine.update(Duration::from_millis(23), None).unwrap();
    let states = engine.get_fixture_states();
    let red_val = states
        .get("wash1")
        .unwrap()
        .channels
        .get("red")
        .unwrap()
        .value;
    assert!(
        (red_val - 128.0 / 255.0).abs() < 0.01,
        "frame 1: red should be ~0.502, got {}",
        red_val
    );

    // Frame 2: update red to 255 (1.0)
    {
        let s = store.read();
        s.write(1, 2, 255, false);
        s.tick();
    }
    let _commands = engine.update(Duration::from_millis(23), None).unwrap();
    let states = engine.get_fixture_states();
    let red_val = states
        .get("wash1")
        .unwrap()
        .channels
        .get("red")
        .unwrap()
        .value;
    assert!(
        (red_val - 1.0).abs() < 0.01,
        "frame 2: red should be ~1.0, got {}",
        red_val
    );

    // Frame 3: update red to 0 (0.0)
    {
        let s = store.read();
        s.write(1, 2, 0, false);
        s.tick();
    }
    let _commands = engine.update(Duration::from_millis(23), None).unwrap();
    let states = engine.get_fixture_states();
    let red_val = states
        .get("wash1")
        .unwrap()
        .channels
        .get("red")
        .unwrap()
        .value;
    assert!(
        red_val.abs() < f64::EPSILON,
        "frame 3: red should be 0.0, got {}",
        red_val
    );
}

#[test]
fn test_legacy_store_values_not_persisted_as_permanent() {
    // Legacy values should NOT be saved into fixture_states (permanent storage).
    // They must be re-injected fresh each frame from the store.
    let mut engine = EffectEngine::new();

    let fixture = create_test_fixture("wash1", 1, 1);
    engine.register_fixture(fixture);

    let store = create_store_for_wash1();
    engine.set_legacy_store(store.clone());

    // Set and process one frame
    {
        let s = store.read();
        s.write(1, 2, 204, false); // red ≈ 0.8
        s.tick();
    }
    let _commands = engine.update(Duration::from_millis(23), None).unwrap();

    // Clear the store (simulating song transition)
    store.read().clear();

    // Next frame should NOT have the old values
    let _commands = engine.update(Duration::from_millis(23), None).unwrap();
    let states = engine.get_fixture_states();
    if let Some(wash1_state) = states.get("wash1") {
        assert!(
            !wash1_state.channels.contains_key("red"),
            "red should not persist after clearing store"
        );
    }
}

#[test]
fn test_clear_legacy_store() {
    let mut engine = EffectEngine::new();

    let fixture = create_test_fixture("wash1", 1, 1);
    engine.register_fixture(fixture);

    let store = create_store_for_wash1();
    engine.set_legacy_store(store.clone());

    // Write values
    {
        let s = store.read();
        s.write(1, 2, 255, false);
        s.write(1, 3, 128, false);
        s.tick();
    }

    // Clear the store
    store.read().clear();

    // After update, no legacy values should appear
    let _commands = engine.update(Duration::from_millis(23), None).unwrap();
    let states = engine.get_fixture_states();

    // wash1 should either not be present or have no channels
    if let Some(wash1_state) = states.get("wash1") {
        assert!(
            wash1_state.channels.is_empty(),
            "channels should be empty after clearing store"
        );
    }
}

#[test]
fn test_stop_all_effects_clears_legacy_store() {
    let mut engine = EffectEngine::new();

    let fixture = create_test_fixture("wash1", 1, 1);
    engine.register_fixture(fixture);

    let store = create_store_for_wash1();
    engine.set_legacy_store(store.clone());

    // Write values
    {
        let s = store.read();
        s.write(1, 2, 255, false);
        s.tick();
    }

    // Verify they're present
    let _commands = engine.update(Duration::from_millis(23), None).unwrap();
    let states = engine.get_fixture_states();
    assert!(states.contains_key("wash1"), "wash1 should have state");

    // stop_all_effects should clear the legacy store
    engine.stop_all_effects();

    let _commands = engine.update(Duration::from_millis(23), None).unwrap();
    let states = engine.get_fixture_states();
    if let Some(wash1_state) = states.get("wash1") {
        assert!(
            wash1_state.channels.is_empty(),
            "channels should be empty after stop_all_effects"
        );
    }
}
