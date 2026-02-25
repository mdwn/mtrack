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

use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time;

use crate::lighting::effects::{is_multiplier_channel, FixtureState};
use crate::lighting::EffectEngine;

/// Runs at 20Hz, sampling fixture states from the effect engine and broadcasting JSON.
pub async fn sampler_loop(effect_engine: Arc<Mutex<EffectEngine>>, tx: broadcast::Sender<String>) {
    let mut interval = time::interval(Duration::from_millis(50));
    // Cache the dimmer map — fixture registry doesn't change at runtime
    let has_dimmer_map: HashMap<String, bool> = {
        let engine = effect_engine.lock();
        engine
            .get_fixture_registry()
            .iter()
            .map(|(name, info)| (name.clone(), info.channels.contains_key("dimmer")))
            .collect()
    };
    loop {
        interval.tick().await;

        // Skip if no subscribers
        if tx.receiver_count() == 0 {
            continue;
        }

        let (fixture_states, active_effects) = {
            let engine = effect_engine.lock();
            let states = engine.get_fixture_states();
            let effects: Vec<String> = engine.get_active_effects().keys().cloned().collect();
            (states, effects)
        };

        let fixtures_json = fixture_states_to_json(&fixture_states, &has_dimmer_map);

        let msg = json!({
            "type": "state",
            "fixtures": fixtures_json,
            "active_effects": active_effects,
        });

        // Ignore send errors (no receivers)
        let _ = tx.send(msg.to_string());
    }
}

/// Converts a map of fixture states into JSON with RGB values (0-255 scale).
///
/// Uses `FixtureState::effective_channel_value` for RGB multiplier logic,
/// keeping the simulator in sync with the DMX output path.
fn fixture_states_to_json(
    states: &HashMap<String, FixtureState>,
    has_dimmer_map: &HashMap<String, bool>,
) -> Value {
    let mut fixtures = serde_json::Map::new();

    for (name, state) in states {
        let mut channels = serde_json::Map::new();
        let has_dedicated_dimmer = has_dimmer_map.get(name).copied().unwrap_or(false);

        for (channel_name, channel_state) in &state.channels {
            // Skip internal multiplier channels
            if is_multiplier_channel(channel_name) {
                continue;
            }

            let value =
                state.effective_channel_value(channel_name, channel_state, has_dedicated_dimmer);

            // Convert to 0-255 scale
            let dmx_value = (value * 255.0) as u8;
            channels.insert(
                channel_name.clone(),
                Value::Number(serde_json::Number::from(dmx_value)),
            );
        }

        fixtures.insert(name.clone(), Value::Object(channels));
    }

    Value::Object(fixtures)
}

/// Builds the initial metadata JSON from the lighting system.
pub fn build_metadata_json(
    lighting_system: Option<&Arc<Mutex<crate::lighting::system::LightingSystem>>>,
) -> String {
    let mut fixtures = serde_json::Map::new();

    if let Some(ls) = lighting_system {
        let system = ls.lock();
        if let Ok(fixture_infos) = system.get_current_venue_fixtures() {
            // Get venue fixtures for tag info
            let venue_fixtures = get_venue_fixture_tags(&system);

            for fi in &fixture_infos {
                let tags = venue_fixtures.get(&fi.name).cloned().unwrap_or_default();

                let fixture_meta = json!({
                    "tags": tags,
                    "type": fi.fixture_type,
                });
                fixtures.insert(fi.name.clone(), fixture_meta);
            }
        }
    }

    let msg = json!({
        "type": "metadata",
        "fixtures": fixtures,
    });
    msg.to_string()
}

/// Extracts fixture names → tags from the lighting system's current venue.
fn get_venue_fixture_tags(
    system: &crate::lighting::system::LightingSystem,
) -> HashMap<String, Vec<String>> {
    let mut result = HashMap::new();
    if let Some(venue) = system.get_current_venue() {
        for (name, fixture) in venue.fixtures() {
            result.insert(name.clone(), fixture.tags().to_vec());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::effects::{BlendMode, ChannelState, EffectLayer, FixtureState};

    #[test]
    fn test_fixture_states_to_json_rgb_only() {
        let mut states = HashMap::new();
        let mut fixture_state = FixtureState::new();
        fixture_state.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::Background, BlendMode::Replace),
        );
        fixture_state.set_channel(
            "green".to_string(),
            ChannelState::new(0.5, EffectLayer::Background, BlendMode::Replace),
        );
        fixture_state.set_channel(
            "blue".to_string(),
            ChannelState::new(0.0, EffectLayer::Background, BlendMode::Replace),
        );
        states.insert("test_fixture".to_string(), fixture_state);

        // No dedicated dimmer → multiplier applies (but default is 1.0)
        let has_dimmer = HashMap::from([("test_fixture".to_string(), false)]);
        let json = fixture_states_to_json(&states, &has_dimmer);
        let obj = json.as_object().unwrap();
        let fixture = obj.get("test_fixture").unwrap().as_object().unwrap();

        assert_eq!(fixture.get("red").unwrap().as_u64().unwrap(), 255);
        assert_eq!(fixture.get("green").unwrap().as_u64().unwrap(), 127);
        assert_eq!(fixture.get("blue").unwrap().as_u64().unwrap(), 0);
    }

    #[test]
    fn test_fixture_states_with_dimmer_skips_multiplier() {
        let mut states = HashMap::new();
        let mut fixture_state = FixtureState::new();
        fixture_state.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::Background, BlendMode::Replace),
        );
        fixture_state.set_channel(
            "dimmer".to_string(),
            ChannelState::new(0.5, EffectLayer::Background, BlendMode::Replace),
        );
        // Add a multiplier channel that would reduce RGB if applied
        fixture_state.set_channel(
            "_dimmer_mult_bg".to_string(),
            ChannelState::new(0.5, EffectLayer::Background, BlendMode::Multiply),
        );
        states.insert("fixture_with_dimmer".to_string(), fixture_state);

        // Has dedicated dimmer → multiplier should NOT be applied to RGB
        let has_dimmer = HashMap::from([("fixture_with_dimmer".to_string(), true)]);
        let json = fixture_states_to_json(&states, &has_dimmer);
        let fixture = json
            .as_object()
            .unwrap()
            .get("fixture_with_dimmer")
            .unwrap()
            .as_object()
            .unwrap();

        // Red should remain 255, not be reduced by the multiplier
        assert_eq!(fixture.get("red").unwrap().as_u64().unwrap(), 255);
        assert_eq!(fixture.get("dimmer").unwrap().as_u64().unwrap(), 127);
    }

    #[test]
    fn test_fixture_states_excludes_multiplier_channels() {
        let mut states = HashMap::new();
        let mut fixture_state = FixtureState::new();
        fixture_state.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::Background, BlendMode::Replace),
        );
        fixture_state.set_channel(
            "_dimmer_mult_bg".to_string(),
            ChannelState::new(0.5, EffectLayer::Background, BlendMode::Multiply),
        );
        states.insert("test_fixture".to_string(), fixture_state);

        let has_dimmer = HashMap::from([("test_fixture".to_string(), false)]);
        let json = fixture_states_to_json(&states, &has_dimmer);
        let fixture = json
            .as_object()
            .unwrap()
            .get("test_fixture")
            .unwrap()
            .as_object()
            .unwrap();

        // Should have "red" but not "_dimmer_mult_bg"
        assert!(fixture.contains_key("red"));
        assert!(!fixture.contains_key("_dimmer_mult_bg"));
    }
}
