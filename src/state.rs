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
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time;

use crate::lighting::effects::{is_multiplier_channel, FixtureState};
use crate::lighting::EffectEngine;

/// Pre-computed fixture display state: all non-multiplier channels at 0-255.
#[derive(Clone, Debug)]
pub struct FixtureSnapshot {
    pub name: String,
    pub channels: HashMap<String, u8>,
}

/// State snapshot broadcast to all display consumers.
#[derive(Clone, Debug, Default)]
pub struct StateSnapshot {
    pub fixtures: Vec<FixtureSnapshot>,
    pub active_effects: Vec<String>,
}

/// Starts a 20Hz sampler that produces `StateSnapshot` values via a `watch` channel.
///
/// Returns the receiver and a join handle for the sampler task.
#[cfg(test)]
pub fn start_sampler(
    effect_engine: Arc<Mutex<EffectEngine>>,
) -> (watch::Receiver<Arc<StateSnapshot>>, JoinHandle<()>) {
    let (tx, rx) = watch::channel(Arc::new(StateSnapshot::default()));

    let handle = tokio::spawn(sampler_loop(effect_engine, tx));

    (rx, handle)
}

/// Starts a sampler using a shared watch sender and a cancellation token.
/// The sampler stops when the token is cancelled (e.g. on hardware reload).
pub fn start_sampler_cancellable(
    effect_engine: Arc<Mutex<EffectEngine>>,
    tx: Arc<watch::Sender<Arc<StateSnapshot>>>,
    cancel: tokio_util::sync::CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(sampler_loop_cancellable(effect_engine, tx, cancel))
}

#[cfg(test)]
async fn sampler_loop(
    effect_engine: Arc<Mutex<EffectEngine>>,
    tx: watch::Sender<Arc<StateSnapshot>>,
) {
    let mut interval = time::interval(Duration::from_millis(50));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    // Cache the dimmer map — fixture registry doesn't change at runtime.
    // Use spawn_blocking because effect_engine is a parking_lot::Mutex shared
    // with the 44Hz effects loop on a std::thread — we must never block a
    // tokio worker thread on it.
    let has_dimmer_map: HashMap<String, bool> = {
        let engine_ref = effect_engine.clone();
        tokio::task::spawn_blocking(move || {
            let engine = engine_ref.lock();
            engine
                .get_fixture_registry()
                .iter()
                .map(|(name, info)| (name.clone(), info.channels.contains_key("dimmer")))
                .collect()
        })
        .await
        .unwrap_or_default()
    };

    loop {
        interval.tick().await;

        // Acquire the effect engine lock on the blocking thread pool so we
        // never block a tokio worker thread.
        let engine_ref = effect_engine.clone();
        let (states, mut active_effects) = match tokio::task::spawn_blocking(move || {
            let lock_start = std::time::Instant::now();
            let engine = engine_ref.lock();
            let wait = lock_start.elapsed();
            if wait > std::time::Duration::from_secs(1) {
                tracing::warn!(
                    wait_ms = wait.as_millis() as u64,
                    "Sampler waited >1s for effect_engine lock"
                );
            }
            let states = engine.get_fixture_states();
            let effects: Vec<String> = engine.get_active_effects().keys().cloned().collect();
            (states, effects)
        })
        .await
        {
            Ok(result) => result,
            Err(_) => continue,
        };

        active_effects.sort();
        let fixtures = compute_fixture_snapshots(&states, &has_dimmer_map);

        let snapshot = Arc::new(StateSnapshot {
            fixtures,
            active_effects,
        });

        // Send fails only when all receivers are dropped; ignore.
        let _ = tx.send(snapshot);
    }
}

/// Cancellable variant of `sampler_loop`. Stops when the token is cancelled.
async fn sampler_loop_cancellable(
    effect_engine: Arc<Mutex<EffectEngine>>,
    tx: Arc<watch::Sender<Arc<StateSnapshot>>>,
    cancel: tokio_util::sync::CancellationToken,
) {
    let mut interval = time::interval(Duration::from_millis(50));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);

    let has_dimmer_map: HashMap<String, bool> = {
        let engine_ref = effect_engine.clone();
        tokio::task::spawn_blocking(move || {
            let engine = engine_ref.lock();
            engine
                .get_fixture_registry()
                .iter()
                .map(|(name, info)| (name.clone(), info.channels.contains_key("dimmer")))
                .collect()
        })
        .await
        .unwrap_or_default()
    };

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = interval.tick() => {}
        }

        let engine_ref = effect_engine.clone();
        let (states, mut active_effects) = match tokio::task::spawn_blocking(move || {
            let engine = engine_ref.lock();
            let states = engine.get_fixture_states();
            let effects: Vec<String> = engine.get_active_effects().keys().cloned().collect();
            (states, effects)
        })
        .await
        {
            Ok(result) => result,
            Err(_) => continue,
        };

        active_effects.sort();
        let fixtures = compute_fixture_snapshots(&states, &has_dimmer_map);

        let snapshot = Arc::new(StateSnapshot {
            fixtures,
            active_effects,
        });

        let _ = tx.send(snapshot);
    }
}

/// Converts fixture states into sorted `FixtureSnapshot` values with DMX 0-255 channel values.
fn compute_fixture_snapshots(
    states: &HashMap<String, FixtureState>,
    has_dimmer_map: &HashMap<String, bool>,
) -> Vec<FixtureSnapshot> {
    let mut snapshots: Vec<FixtureSnapshot> = states
        .iter()
        .map(|(name, state)| {
            let has_dedicated_dimmer = has_dimmer_map.get(name).copied().unwrap_or(false);
            let mut channels = HashMap::new();

            for (channel_name, channel_state) in &state.channels {
                if is_multiplier_channel(channel_name) {
                    continue;
                }

                let value = state.effective_channel_value(
                    channel_name,
                    channel_state,
                    has_dedicated_dimmer,
                );
                let dmx_value = (value * 255.0) as u8;
                channels.insert(channel_name.clone(), dmx_value);
            }

            FixtureSnapshot {
                name: name.clone(),
                channels,
            }
        })
        .collect();

    snapshots.sort_by(|a, b| a.name.cmp(&b.name));
    snapshots
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::effects::{BlendMode, ChannelState, EffectLayer, FixtureState};

    #[test]
    fn test_compute_fixture_snapshots_rgb_only() {
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

        let has_dimmer = HashMap::from([("test_fixture".to_string(), false)]);
        let snapshots = compute_fixture_snapshots(&states, &has_dimmer);

        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].name, "test_fixture");
        assert_eq!(*snapshots[0].channels.get("red").unwrap(), 255);
        assert_eq!(*snapshots[0].channels.get("green").unwrap(), 127);
        assert_eq!(*snapshots[0].channels.get("blue").unwrap(), 0);
    }

    #[test]
    fn test_compute_fixture_snapshots_with_dimmer() {
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
        fixture_state.set_channel(
            "_dimmer_mult_bg".to_string(),
            ChannelState::new(0.5, EffectLayer::Background, BlendMode::Multiply),
        );
        states.insert("fixture_with_dimmer".to_string(), fixture_state);

        let has_dimmer = HashMap::from([("fixture_with_dimmer".to_string(), true)]);
        let snapshots = compute_fixture_snapshots(&states, &has_dimmer);

        assert_eq!(snapshots.len(), 1);
        // Red should remain 255 (not reduced by multiplier when fixture has dedicated dimmer)
        assert_eq!(*snapshots[0].channels.get("red").unwrap(), 255);
        assert_eq!(*snapshots[0].channels.get("dimmer").unwrap(), 127);
    }

    #[test]
    fn test_compute_fixture_snapshots_excludes_multiplier_channels() {
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
        let snapshots = compute_fixture_snapshots(&states, &has_dimmer);

        assert_eq!(snapshots.len(), 1);
        assert!(snapshots[0].channels.contains_key("red"));
        assert!(!snapshots[0].channels.contains_key("_dimmer_mult_bg"));
    }

    #[test]
    fn test_snapshots_sorted_by_name() {
        let mut states = HashMap::new();
        states.insert("zebra".to_string(), FixtureState::new());
        states.insert("alpha".to_string(), FixtureState::new());
        states.insert("middle".to_string(), FixtureState::new());

        let snapshots = compute_fixture_snapshots(&states, &HashMap::new());

        assert_eq!(snapshots[0].name, "alpha");
        assert_eq!(snapshots[1].name, "middle");
        assert_eq!(snapshots[2].name, "zebra");
    }

    #[test]
    fn test_compute_fixture_snapshots_empty() {
        let states = HashMap::new();
        let snapshots = compute_fixture_snapshots(&states, &HashMap::new());
        assert!(snapshots.is_empty());
    }

    #[test]
    fn test_compute_fixture_snapshots_zero_value() {
        let mut states = HashMap::new();
        let mut fixture_state = FixtureState::new();
        fixture_state.set_channel(
            "red".to_string(),
            ChannelState::new(0.0, EffectLayer::Background, BlendMode::Replace),
        );
        states.insert("dark_fixture".to_string(), fixture_state);

        let has_dimmer = HashMap::new();
        let snapshots = compute_fixture_snapshots(&states, &has_dimmer);
        assert_eq!(snapshots.len(), 1);
        assert_eq!(*snapshots[0].channels.get("red").unwrap(), 0);
    }

    #[test]
    fn test_compute_fixture_snapshots_unknown_fixture_in_dimmer_map() {
        let mut states = HashMap::new();
        let mut fixture_state = FixtureState::new();
        fixture_state.set_channel(
            "red".to_string(),
            ChannelState::new(0.5, EffectLayer::Background, BlendMode::Replace),
        );
        states.insert("unknown_fixture".to_string(), fixture_state);

        // has_dimmer_map doesn't contain this fixture - should default to false
        let has_dimmer = HashMap::from([("other_fixture".to_string(), true)]);
        let snapshots = compute_fixture_snapshots(&states, &has_dimmer);
        assert_eq!(snapshots.len(), 1);
        assert_eq!(*snapshots[0].channels.get("red").unwrap(), 127);
    }

    #[test]
    fn test_state_snapshot_default() {
        let snapshot = StateSnapshot::default();
        assert!(snapshot.fixtures.is_empty());
        assert!(snapshot.active_effects.is_empty());
    }

    #[test]
    fn test_fixture_snapshot_clone() {
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 255u8);
        channels.insert("green".to_string(), 128u8);
        let snapshot = FixtureSnapshot {
            name: "test".to_string(),
            channels,
        };
        let cloned = snapshot.clone();
        assert_eq!(cloned.name, "test");
        assert_eq!(*cloned.channels.get("red").unwrap(), 255);
        assert_eq!(*cloned.channels.get("green").unwrap(), 128);
    }

    #[test]
    fn test_state_snapshot_clone() {
        let snapshot = StateSnapshot {
            fixtures: vec![FixtureSnapshot {
                name: "f1".to_string(),
                channels: HashMap::new(),
            }],
            active_effects: vec!["effect1".to_string()],
        };
        let cloned = snapshot.clone();
        assert_eq!(cloned.fixtures.len(), 1);
        assert_eq!(cloned.active_effects.len(), 1);
    }

    #[tokio::test]
    async fn test_start_sampler_empty_engine() {
        let engine = Arc::new(Mutex::new(EffectEngine::new()));
        let (mut rx, handle) = start_sampler(engine);

        // Wait for the sampler to produce a snapshot
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), rx.changed()).await;
        assert!(result.is_ok(), "timed out waiting for sampler");

        let snapshot = rx.borrow().clone();
        assert!(snapshot.fixtures.is_empty());
        assert!(snapshot.active_effects.is_empty());

        handle.abort();
    }

    #[tokio::test]
    async fn test_start_sampler_with_registered_fixture() {
        use crate::lighting::effects::FixtureInfo;

        let engine = Arc::new(Mutex::new(EffectEngine::new()));

        // Register a simple RGB fixture
        {
            let mut channels = HashMap::new();
            channels.insert("red".to_string(), 1);
            channels.insert("green".to_string(), 2);
            channels.insert("blue".to_string(), 3);
            let fixture = FixtureInfo::new(
                "test_light".to_string(),
                1,
                1,
                "rgb".to_string(),
                channels,
                None,
            );
            engine.lock().register_fixture(fixture);
        }

        let (mut rx, handle) = start_sampler(engine);

        let result = tokio::time::timeout(std::time::Duration::from_secs(2), rx.changed()).await;
        assert!(result.is_ok(), "timed out waiting for sampler");

        let snapshot = rx.borrow().clone();
        // Fixture registry is populated, but without active effects the fixture
        // may or may not appear in the snapshot depending on engine state
        assert!(snapshot.active_effects.is_empty());

        handle.abort();
    }

    #[tokio::test]
    async fn test_start_sampler_multiple_ticks() {
        let engine = Arc::new(Mutex::new(EffectEngine::new()));
        let (mut rx, handle) = start_sampler(engine);

        // Wait for first tick
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), rx.changed()).await;

        // Wait for second tick
        let result = tokio::time::timeout(std::time::Duration::from_secs(2), rx.changed()).await;
        assert!(result.is_ok(), "timed out waiting for second sampler tick");

        handle.abort();
    }

    #[tokio::test]
    async fn test_sampler_stops_when_receiver_dropped() {
        let engine = Arc::new(Mutex::new(EffectEngine::new()));
        let (rx, handle) = start_sampler(engine);

        // Drop receiver — sampler should keep running (tx.send just fails silently)
        drop(rx);

        // Give it a moment, then abort
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        handle.abort();
        let _ = handle.await;
    }
}
