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
use tokio::sync::{broadcast, watch};

use crate::state::StateSnapshot;

/// Bridges the shared `watch` channel to the simulator's `broadcast` channel.
///
/// Each time the watch receiver sees a new `StateSnapshot`, it converts it to JSON
/// and sends it via the broadcast sender to all connected WebSocket clients.
pub async fn sampler_loop(
    mut state_rx: watch::Receiver<Arc<StateSnapshot>>,
    tx: broadcast::Sender<String>,
) {
    loop {
        // Wait for new state
        if state_rx.changed().await.is_err() {
            // Sender dropped, sampler is shutting down
            break;
        }

        // Skip if no WebSocket subscribers
        if tx.receiver_count() == 0 {
            continue;
        }

        let snapshot = state_rx.borrow_and_update().clone();
        let json = snapshot_to_json(&snapshot);

        // Ignore send errors (no receivers)
        let _ = tx.send(json);
    }
}

/// Converts a `StateSnapshot` into the JSON format expected by WebSocket clients.
fn snapshot_to_json(snapshot: &StateSnapshot) -> String {
    let mut fixtures = serde_json::Map::new();

    for fixture in &snapshot.fixtures {
        let mut channels = serde_json::Map::new();
        for (channel_name, &value) in &fixture.channels {
            channels.insert(
                channel_name.clone(),
                Value::Number(serde_json::Number::from(value)),
            );
        }
        fixtures.insert(fixture.name.clone(), Value::Object(channels));
    }

    let msg = json!({
        "type": "state",
        "fixtures": Value::Object(fixtures),
        "active_effects": snapshot.active_effects,
    });

    msg.to_string()
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
    use crate::state::FixtureSnapshot;

    #[test]
    fn test_snapshot_to_json_rgb_only() {
        let snapshot = StateSnapshot {
            fixtures: vec![FixtureSnapshot {
                name: "test_fixture".to_string(),
                channels: HashMap::from([
                    ("red".to_string(), 255),
                    ("green".to_string(), 127),
                    ("blue".to_string(), 0),
                ]),
            }],
            active_effects: vec![],
        };

        let json_str = snapshot_to_json(&snapshot);
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let fixture = parsed["fixtures"]["test_fixture"].as_object().unwrap();
        assert_eq!(fixture.get("red").unwrap().as_u64().unwrap(), 255);
        assert_eq!(fixture.get("green").unwrap().as_u64().unwrap(), 127);
        assert_eq!(fixture.get("blue").unwrap().as_u64().unwrap(), 0);
    }

    #[test]
    fn test_snapshot_to_json_with_active_effects() {
        let snapshot = StateSnapshot {
            fixtures: vec![],
            active_effects: vec!["chase_1".to_string(), "static_blue".to_string()],
        };

        let json_str = snapshot_to_json(&snapshot);
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let effects = parsed["active_effects"].as_array().unwrap();
        assert_eq!(effects.len(), 2);
        assert_eq!(effects[0].as_str().unwrap(), "chase_1");
        assert_eq!(effects[1].as_str().unwrap(), "static_blue");
    }

    #[test]
    fn test_snapshot_to_json_with_dimmer() {
        let snapshot = StateSnapshot {
            fixtures: vec![FixtureSnapshot {
                name: "fixture_with_dimmer".to_string(),
                channels: HashMap::from([("red".to_string(), 255), ("dimmer".to_string(), 127)]),
            }],
            active_effects: vec![],
        };

        let json_str = snapshot_to_json(&snapshot);
        let parsed: serde_json::Value = serde_json::from_str(&json_str).unwrap();

        let fixture = parsed["fixtures"]["fixture_with_dimmer"]
            .as_object()
            .unwrap();
        assert_eq!(fixture.get("red").unwrap().as_u64().unwrap(), 255);
        assert_eq!(fixture.get("dimmer").unwrap().as_u64().unwrap(), 127);
    }
}
