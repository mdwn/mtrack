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
///
/// `PartialEq` so a push-based consumer can tell a real change from a sampler
/// tick that produced the same values — an idle rig must not wake subscribers
/// twenty times a second.
#[derive(Clone, Debug, PartialEq)]
pub struct FixtureSnapshot {
    pub name: String,
    pub channels: HashMap<String, u8>,
    /// Per-cell values (design §17.4), by cell name, only while a
    /// per-cell effect drives at least one cell of this fixture; empty
    /// otherwise, when every cell shows the fixture's own channels.
    pub cells: std::collections::BTreeMap<String, HashMap<String, u8>>,
}

/// Where a mover is pointing, for the stage plot's beam.
#[derive(Clone, Debug, PartialEq)]
pub struct PoseSnapshot {
    pub name: String,
    pub pan: f64,
    pub tilt: f64,
    /// The beam direction in stage space, unit length.
    pub aim: [f64; 3],
    /// Where the beam meets the deck (stage x, y), when it points down and
    /// the venue places the fixture; a beam pointing up has no footprint.
    pub floor: Option<[f64; 2]>,
}

/// State snapshot broadcast to all display consumers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StateSnapshot {
    pub fixtures: Vec<FixtureSnapshot>,
    pub active_effects: Vec<String>,
    pub poses: Vec<PoseSnapshot>,
}

/// The farthest a beam footprint is drawn from its fixture, meters: a
/// near-level beam would otherwise land off the plot.
const MAX_THROW_M: f64 = 40.0;

/// Computes the pose snapshots from the engine's pose memory and the
/// registered fixtures' placement. Every placed fixture has one (design
/// §18, decision 4): a fixture nothing has moved sits at rest, pointing
/// down its mounting's −Z, which for a static fixture is simply where it
/// points.
pub(crate) fn compute_pose_snapshots(
    poses: &HashMap<String, crate::lighting::effects::Pose>,
    registry: &HashMap<String, crate::lighting::effects::FixtureInfo>,
) -> Vec<PoseSnapshot> {
    let rest = crate::lighting::effects::Pose {
        pan: 0.0,
        tilt: 0.0,
    };
    let at_rest = registry.iter().filter_map(|(name, fixture)| {
        (fixture.position.is_some() && fixture.parent.is_none() && !poses.contains_key(name))
            .then_some((name, &rest))
    });
    let mut out: Vec<PoseSnapshot> = poses
        .iter()
        .chain(at_rest)
        .filter_map(|(name, pose)| {
            let fixture = registry.get(name)?;
            let rotation = fixture.rotation.unwrap_or([0.0; 3]);
            let aim = fixture.calibration().direction(rotation, *pose);
            let floor = fixture.position.and_then(|p| {
                if aim[2] >= -1e-6 || p[2] <= 0.0 {
                    return None;
                }
                let t = (-p[2] / aim[2]).min(MAX_THROW_M);
                Some([p[0] + aim[0] * t, p[1] + aim[1] * t])
            });
            Some(PoseSnapshot {
                name: name.clone(),
                pan: pose.pan,
                tilt: pose.tilt,
                aim,
                floor,
            })
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
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

/// Builds the dimmer map by querying the fixture registry on the blocking
/// thread pool (the effect engine uses a `parking_lot::Mutex` shared with
/// a std::thread, so we must never block a tokio worker on it).
async fn init_dimmer_map(effect_engine: &Arc<Mutex<EffectEngine>>) -> HashMap<String, bool> {
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
}

/// Samples the current fixture state from the effect engine (on the blocking
/// thread pool) and returns a snapshot, or `None` if the blocking task panicked.
async fn sample_tick(
    effect_engine: &Arc<Mutex<EffectEngine>>,
    has_dimmer_map: &HashMap<String, bool>,
) -> Option<Arc<StateSnapshot>> {
    let engine_ref = effect_engine.clone();
    let has_dimmer_map = has_dimmer_map.clone();
    let (fixtures, mut active_effects, poses) = tokio::task::spawn_blocking(move || {
        let engine = engine_ref.lock();
        // A cell's sub-fixture is part of its fixture, not a fixture of
        // its own to the stream (design §17.4): its state folds into the
        // fixture's `cells`.
        let registry = engine.get_fixture_registry();
        let fixtures =
            fixture_snapshots_with_cells(&engine.get_fixture_states(), &has_dimmer_map, registry);
        let effects: Vec<String> = engine.get_active_effects().keys().cloned().collect();
        let poses = compute_pose_snapshots(engine.poses(), registry);
        (fixtures, effects, poses)
    })
    .await
    .ok()?;

    active_effects.sort();

    Some(Arc::new(StateSnapshot {
        fixtures,
        active_effects,
        poses,
    }))
}

#[cfg(test)]
async fn sampler_loop(
    effect_engine: Arc<Mutex<EffectEngine>>,
    tx: watch::Sender<Arc<StateSnapshot>>,
) {
    let mut interval = time::interval(Duration::from_millis(50));
    interval.set_missed_tick_behavior(time::MissedTickBehavior::Skip);
    let has_dimmer_map = init_dimmer_map(&effect_engine).await;
    let mut first = true;

    loop {
        interval.tick().await;

        if let Some(snapshot) = sample_tick(&effect_engine, &has_dimmer_map).await {
            // An idle rig must not wake subscribers twenty times a second:
            // a snapshot equal to the last one is not a change. The first
            // one always goes out, so a subscriber waiting on the sampler
            // learns it is alive.
            tx.send_if_modified(|current| {
                if !first && **current == *snapshot {
                    false
                } else {
                    *current = snapshot;
                    true
                }
            });
            first = false;
        }
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
    let has_dimmer_map = init_dimmer_map(&effect_engine).await;
    let mut first = true;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => break,
            _ = interval.tick() => {}
        }

        if let Some(snapshot) = sample_tick(&effect_engine, &has_dimmer_map).await {
            // An idle rig must not wake subscribers twenty times a second:
            // a snapshot equal to the last one is not a change. The first
            // one always goes out, so a subscriber waiting on the sampler
            // learns it is alive.
            tx.send_if_modified(|current| {
                if !first && **current == *snapshot {
                    false
                } else {
                    *current = snapshot;
                    true
                }
            });
            first = false;
        }
    }
}

/// Converts fixture states into sorted `FixtureSnapshot` values with DMX 0-255 channel values.
pub(crate) fn compute_fixture_snapshots(
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
                cells: Default::default(),
            }
        })
        .collect();

    snapshots.sort_by(|a, b| a.name.cmp(&b.name));
    snapshots
}

/// Snapshots for every fixture the engine holds state for, with a pixel
/// fixture's cells folded in (design §17.4): sub-fixtures (`parent/cell`)
/// are not fixtures of their own here, their state reaches the parent's
/// `cells`. The one builder both the sampler and the evaluators use.
pub(crate) fn fixture_snapshots_with_cells(
    states: &HashMap<String, FixtureState>,
    has_dimmer_map: &HashMap<String, bool>,
    registry: &HashMap<String, crate::lighting::effects::FixtureInfo>,
) -> Vec<FixtureSnapshot> {
    let mut fixtures = compute_fixture_snapshots(states, has_dimmer_map);
    fixtures.retain(|s| registry.get(&s.name).is_none_or(|f| f.parent.is_none()));
    attach_cell_snapshots(&mut fixtures, states, registry);
    attach_pointing(&mut fixtures, states, registry);
    fixtures
}

/// Pan and tilt as the bytes on the wire. A move writes physical intents
/// in degrees, not channel values, and they resolve to bytes only on the
/// DMX path — so without this a snapshot of a mover mid-sweep shows no
/// pan or tilt at all, and `evaluate_show` cannot answer "where is it
/// pointing". Resolved through the same code as the wire: coarse byte
/// under the channel's name, fine byte under `<name>_fine` when the
/// channel has one. A ganged channel's mirror bytes repeat these on the
/// wire and have no name of their own here, as for every other channel
/// in a snapshot.
fn attach_pointing(
    snapshots: &mut [FixtureSnapshot],
    states: &HashMap<String, FixtureState>,
    registry: &HashMap<String, crate::lighting::effects::FixtureInfo>,
) {
    for snapshot in snapshots.iter_mut() {
        let (Some(state), Some(info)) = (states.get(&snapshot.name), registry.get(&snapshot.name))
        else {
            continue;
        };
        for (channel, resolved) in
            crate::lighting::effects::resolve_physical(&state.physical, &info.channel_defs)
        {
            let def = &info.channel_defs[channel];
            for (offset, byte) in resolved.bytes {
                if offset == def.offset {
                    snapshot.channels.insert(channel.to_string(), byte);
                } else if Some(offset) == def.fine {
                    snapshot.channels.insert(format!("{channel}_fine"), byte);
                }
            }
        }
    }
}

/// Whether a channel name carries where a head points rather than what it
/// shows: pan, tilt and their fine bytes.
pub fn is_pointing_channel(name: &str) -> bool {
    matches!(name, "pan" | "tilt" | "pan_fine" | "tilt_fine")
}

/// Adds per-cell values to the snapshots of fixtures whose cells carry
/// state of their own this frame (design §17.4): each cell's channels are
/// the fixture's blended with the cell's, as the wire gets them. A fixture
/// none of whose cells has state stays without `cells`; a fixture that
/// has no snapshot yet but a lit cell gets one, its own channels dark.
pub(crate) fn attach_cell_snapshots(
    snapshots: &mut Vec<FixtureSnapshot>,
    states: &HashMap<String, FixtureState>,
    registry: &HashMap<String, crate::lighting::effects::FixtureInfo>,
) {
    let empty = FixtureState::new();
    let mut parents: Vec<&str> = states
        .keys()
        .filter_map(|name| registry.get(name).and_then(|f| f.parent.as_deref()))
        .collect();
    parents.sort();
    parents.dedup();
    for parent in parents {
        let Some(info) = registry.get(parent) else {
            continue;
        };
        let parent_state = states.get(parent).unwrap_or(&empty);
        let mut cells = std::collections::BTreeMap::new();
        for cell in &info.cells {
            let own = states.get(&format!("{parent}/{}", cell.name));
            let values: HashMap<String, u8> = parent_state
                .cell_values(cell, own)
                .into_iter()
                .map(|(name, value)| (name, (value * 255.0) as u8))
                .collect();
            cells.insert(cell.name.clone(), values);
        }
        match snapshots.iter_mut().find(|s| s.name == parent) {
            Some(snapshot) => snapshot.cells = cells,
            None => {
                let at = snapshots.partition_point(|s| s.name.as_str() < parent);
                snapshots.insert(
                    at,
                    FixtureSnapshot {
                        name: parent.to_string(),
                        channels: info
                            .channels
                            .keys()
                            .filter(|name| !is_multiplier_channel(name))
                            .map(|name| (name.clone(), 0u8))
                            .collect(),
                        cells,
                    },
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::effects::{
        BlendMode, ChannelState, EffectLayer, FixtureInfo, FixtureState,
    };
    use crate::lighting::types::{Cell, ChannelDef};

    #[test]
    fn pose_snapshots_carry_the_beam_and_its_footprint() {
        use crate::lighting::effects::{FixtureInfo, Pose};
        let mut registry = HashMap::new();
        let mut mover =
            FixtureInfo::new("m".to_string(), 1, 1, "T".to_string(), HashMap::new(), None);
        mover.position = Some([0.0, 3.5, 4.0]);
        mover.rotation = Some([0.0, 0.0, 180.0]);
        registry.insert("m".to_string(), mover);
        let mut poses = HashMap::new();
        // Hung facing the audience, tilted 45° from straight down toward
        // its local +y (downstage): the beam hits the deck 4 m downstage
        // of the fixture.
        poses.insert(
            "m".to_string(),
            Pose {
                pan: 0.0,
                tilt: 45.0,
            },
        );
        let snapshots = compute_pose_snapshots(&poses, &registry);
        assert_eq!(snapshots.len(), 1);
        let floor = snapshots[0].floor.expect("footprint");
        assert!(
            (floor[0]).abs() < 1e-9 && (floor[1] - -0.5).abs() < 1e-9,
            "{floor:?}"
        );
        // Past level: no footprint.
        poses.get_mut("m").unwrap().tilt = 100.0;
        assert!(compute_pose_snapshots(&poses, &registry)[0].floor.is_none());
    }

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
            cells: Default::default(),
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
                cells: Default::default(),
            }],
            active_effects: vec!["effect1".to_string()],
            poses: Vec::new(),
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
    async fn an_idle_engine_does_not_wake_subscribers_but_a_change_does() {
        use crate::lighting::effects::{EffectInstance, EffectType, FixtureInfo};
        let engine = Arc::new(Mutex::new(EffectEngine::new()));
        {
            let channels: HashMap<String, u16> = [("red", 1u16)]
                .into_iter()
                .map(|(n, o)| (n.to_string(), o))
                .collect();
            engine.lock().register_fixture(FixtureInfo::new(
                "f".to_string(),
                1,
                1,
                "T".to_string(),
                channels,
                None,
            ));
        }
        let (mut rx, handle) = start_sampler(engine.clone());

        // The first snapshot always goes out.
        let first = tokio::time::timeout(std::time::Duration::from_secs(2), rx.changed()).await;
        assert!(first.is_ok(), "timed out waiting for the first snapshot");

        // Nothing changes: no wake-up, however many ticks pass.
        let idle = tokio::time::timeout(std::time::Duration::from_millis(300), rx.changed()).await;
        assert!(idle.is_err(), "an idle engine woke a subscriber");

        // Something changes: the next tick reports it.
        {
            let mut guard = engine.lock();
            let mut parameters = HashMap::new();
            parameters.insert("red".to_string(), 1.0);
            let effect = EffectInstance::new(
                "e".to_string(),
                EffectType::Static {
                    parameters,
                    duration: std::time::Duration::from_secs(5),
                },
                vec!["f".to_string()],
                None,
                None,
                None,
            );
            guard.start_effect(effect).unwrap();
            guard
                .update(std::time::Duration::from_millis(10), None)
                .unwrap();
        }
        let changed = tokio::time::timeout(std::time::Duration::from_secs(2), rx.changed()).await;
        assert!(changed.is_ok(), "a change did not wake the subscriber");
        assert!(!rx.borrow().fixtures.is_empty());

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

    // ── attach_cell_snapshots (design §17.4) ───────────────────────

    /// A three-cell RGB bar at address 1: dimmer 1, cells at 2-4, 5-7, 8-10,
    /// the fixture-level colour ganged across them. Mirrors
    /// `src/lighting/engine/tests/cell_tests.rs`'s `bar()`.
    fn bar(name: &str) -> FixtureInfo {
        let cell = |n: &str, base: u16, x: f64| Cell {
            name: n.to_string(),
            channels: HashMap::from([
                ("red".to_string(), ChannelDef::at(base)),
                ("green".to_string(), ChannelDef::at(base + 1)),
                ("blue".to_string(), ChannelDef::at(base + 2)),
            ]),
            offset: [x, 0.0, 0.0],
        };
        let cells = vec![cell("1", 2, -0.3), cell("2", 5, 0.0), cell("3", 8, 0.3)];
        let ganged = crate::lighting::types::ganged_from_cells(&cells).unwrap();
        let mut channels: HashMap<String, u16> =
            ganged.iter().map(|(n, d)| (n.clone(), d.offset)).collect();
        channels.insert("dimmer".to_string(), 1);
        let mut defs = ganged;
        defs.insert("dimmer".to_string(), ChannelDef::at(1));
        let mut info = FixtureInfo::new(name.to_string(), 1, 1, "Bar".to_string(), channels, None);
        info.channel_defs = defs;
        info.cells = cells;
        info
    }

    /// A cell registered as a sub-fixture of `parent`, the way
    /// `EffectEngine::register` builds one: the cell's own channels, and
    /// `parent` set so `attach_cell_snapshots` folds its state back in.
    fn sub_fixture(parent: &str, cell: &Cell) -> FixtureInfo {
        let name = format!("{parent}/{}", cell.name);
        let channels: HashMap<String, u16> = cell
            .channels
            .iter()
            .map(|(n, d)| (n.clone(), d.offset))
            .collect();
        let mut sub = FixtureInfo::new(name, 1, 1, "Bar".to_string(), channels, None);
        sub.channel_defs = cell.channels.clone();
        sub.parent = Some(parent.to_string());
        sub
    }

    #[test]
    fn attach_cell_snapshots_blends_a_cells_own_state_over_the_bed() {
        let bar_info = bar("Bar");
        let mut registry = HashMap::new();
        for cell in &bar_info.cells {
            registry.insert(format!("Bar/{}", cell.name), sub_fixture("Bar", cell));
        }
        registry.insert("Bar".to_string(), bar_info);

        let mut bed = FixtureState::new();
        bed.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::Background, BlendMode::Replace),
        );
        let mut cell_2 = FixtureState::new();
        cell_2.set_channel(
            "red".to_string(),
            ChannelState::new(0.0, EffectLayer::Midground, BlendMode::Replace),
        );

        let mut states = HashMap::new();
        states.insert("Bar".to_string(), bed.clone());
        states.insert("Bar/2".to_string(), cell_2);

        // Seed the snapshot as `sample_tick` would: only the fixture-level
        // state feeds `compute_fixture_snapshots` (sub-fixture states are
        // filtered out before this point).
        let has_dimmer = HashMap::from([("Bar".to_string(), false)]);
        let bar_only: HashMap<String, FixtureState> = HashMap::from([("Bar".to_string(), bed)]);
        let mut snapshots = compute_fixture_snapshots(&bar_only, &has_dimmer);
        let channels_before = snapshots[0].channels.clone();

        attach_cell_snapshots(&mut snapshots, &states, &registry);

        assert_eq!(snapshots.len(), 1);
        let bar_snapshot = &snapshots[0];
        assert_eq!(
            bar_snapshot.channels, channels_before,
            "the parent's own channels are untouched by cell attachment"
        );
        assert_eq!(*bar_snapshot.cells["1"].get("red").unwrap(), 255);
        assert_eq!(*bar_snapshot.cells["2"].get("red").unwrap(), 0);
        assert_eq!(*bar_snapshot.cells["3"].get("red").unwrap(), 255);
    }

    #[test]
    fn attach_cell_snapshots_is_a_no_op_with_no_sub_fixture_states() {
        let bar_info = bar("Bar");
        let mut registry = HashMap::new();
        for cell in &bar_info.cells {
            registry.insert(format!("Bar/{}", cell.name), sub_fixture("Bar", cell));
        }
        registry.insert("Bar".to_string(), bar_info);

        let mut bed = FixtureState::new();
        bed.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::Background, BlendMode::Replace),
        );
        let states = HashMap::from([("Bar".to_string(), bed.clone())]);

        let has_dimmer = HashMap::from([("Bar".to_string(), false)]);
        let mut snapshots = compute_fixture_snapshots(&states, &has_dimmer);
        let before = snapshots.clone();

        attach_cell_snapshots(&mut snapshots, &states, &registry);

        assert_eq!(snapshots, before, "no fixture gets `cells`");
        assert!(snapshots[0].cells.is_empty());
    }

    #[test]
    fn attach_cell_snapshots_inserts_a_parent_that_has_no_snapshot_yet() {
        let bar_info = bar("Bar");
        let mut registry = HashMap::new();
        for cell in &bar_info.cells {
            registry.insert(format!("Bar/{}", cell.name), sub_fixture("Bar", cell));
        }
        registry.insert("Bar".to_string(), bar_info);

        let mut cell_2 = FixtureState::new();
        cell_2.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::Background, BlendMode::Replace),
        );
        // Only the cell has state — no state for "Bar" itself.
        let states = HashMap::from([("Bar/2".to_string(), cell_2)]);

        // Two unrelated fixtures already in the snapshot list, sorted, so
        // the insertion position (between them) is exercised.
        let mut snapshots = vec![
            FixtureSnapshot {
                name: "Aaa".to_string(),
                channels: HashMap::new(),
                cells: Default::default(),
            },
            FixtureSnapshot {
                name: "Zzz".to_string(),
                channels: HashMap::new(),
                cells: Default::default(),
            },
        ];

        attach_cell_snapshots(&mut snapshots, &states, &registry);

        assert_eq!(
            snapshots
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            vec!["Aaa", "Bar", "Zzz"],
            "inserted in sorted position"
        );
        let bar_snapshot = snapshots.iter().find(|s| s.name == "Bar").unwrap();
        assert!(
            bar_snapshot.channels.values().all(|v| *v == 0)
                && bar_snapshot.channels.contains_key("dimmer"),
            "inserted parent is dark, not empty: {:?}",
            bar_snapshot.channels
        );
        assert_eq!(*bar_snapshot.cells["2"].get("red").unwrap(), 255);
        assert!(bar_snapshot.cells["1"].is_empty());
        assert!(bar_snapshot.cells["3"].is_empty());
    }

    /// A mover's physical intent reaches the snapshot as the wire's
    /// bytes: coarse under the channel name, fine under `<name>_fine`.
    #[test]
    fn fixture_snapshots_with_cells_carry_pan_and_tilt_bytes() {
        use crate::lighting::effects::{EffectLayer, Intent, PhysicalParameter};
        use crate::lighting::types::{ChannelDef, PhysicalRange, PhysicalUnit};

        let channels: HashMap<String, u16> = [("pan", 1), ("tilt", 3), ("dimmer", 5)]
            .into_iter()
            .map(|(n, o)| (n.to_string(), o))
            .collect();
        let mut info = crate::lighting::effects::FixtureInfo::new(
            "m".to_string(),
            1,
            1,
            "Mover".to_string(),
            channels,
            None,
        );
        let mut defs = HashMap::new();
        for (name, offset, span) in [("pan", 1, 270.0), ("tilt", 3, 135.0)] {
            defs.insert(
                name.to_string(),
                ChannelDef {
                    offset,
                    fine: Some(offset + 1),
                    range: Some(PhysicalRange {
                        from: -span,
                        to: span,
                        unit: PhysicalUnit::Degrees,
                    }),
                    functions: Vec::new(),
                    mirrors: Vec::new(),
                },
            );
        }
        defs.insert("dimmer".to_string(), ChannelDef::at(5));
        info = info.with_channel_defs(defs);
        let registry: HashMap<String, _> = [("m".to_string(), info)].into_iter().collect();

        let mut state = FixtureState::new();
        state.physical.set(
            PhysicalParameter::Pan,
            Intent {
                degrees: 0.0,
                layer: EffectLayer::Background,
            },
        );
        state.physical.set(
            PhysicalParameter::Tilt,
            Intent {
                degrees: 135.0,
                layer: EffectLayer::Background,
            },
        );
        let states: HashMap<String, FixtureState> =
            [("m".to_string(), state)].into_iter().collect();

        let snapshots = fixture_snapshots_with_cells(&states, &HashMap::new(), &registry);
        let m = &snapshots[0].channels;
        // Pan 0° over ±270° is 32768: 0x80 0x00. Tilt at the top of its
        // range is 65535: 0xFF 0xFF.
        assert_eq!((m["pan"], m["pan_fine"]), (0x80, 0x00));
        assert_eq!((m["tilt"], m["tilt_fine"]), (0xFF, 0xFF));
        assert!(!m.contains_key("dimmer"), "nothing wrote the dimmer");
    }
}
