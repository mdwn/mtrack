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

mod pose;

pub use pose::PoseSnapshot;

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use tokio::time;

use crate::lighting::channels::{fine_name, mirror_name, CMY_OF_RGB};
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
    /// The rate the fixture is strobing at, in hertz, when it is: its
    /// strobe byte read back through its own strobe rule
    /// ([`FixtureInfo::strobe_hz`](crate::lighting::effects::FixtureInfo::strobe_hz)),
    /// which the bytes alone do not say, and — on a single-instant
    /// snapshot only ([`StrobeRates::Instant`]: an evaluation, the
    /// preview) — the rate of the software strobe flashing a fixture that
    /// has no strobe channel, whose bytes show only that instant's phase.
    /// The live stream ([`StrobeRates::Live`]) leaves a software strobe
    /// out: its bytes flip on and off on the stream itself, and a view
    /// that gated them again would flash twice over. `None` when the
    /// fixture is not strobing. The stage views flash at this.
    pub strobe_hz: Option<f64>,
}

/// Which strobe rates a snapshot's [`FixtureSnapshot::strobe_hz`] says:
/// the one rule for where a software strobe's rate is carried. The MCP
/// `get_fixture_state` tool and the evaluate CLI snapshot the engine the
/// `Instant` way (the same `evaluate::snapshot`); their JSON serialises
/// the bytes only, so the rate changes nothing they say. Mid-crossfade on
/// the `Instant` path a software strobe's rate snaps from the outgoing
/// song's to the incoming one's at half way, while its bytes lerp — a
/// rate cannot be halfway between two strobes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StrobeRates {
    /// The live state stream: a strobe channel's rate only. A software
    /// strobe's bytes flip on the stream frame by frame already.
    Live,
    /// A single instant (an evaluation, the preview): a strobe channel's
    /// rate and a software strobe's, since the bytes of one instant show
    /// only the phase.
    Instant,
}

/// State snapshot broadcast to all display consumers.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StateSnapshot {
    pub fixtures: Vec<FixtureSnapshot>,
    pub active_effects: Vec<String>,
    pub poses: Vec<PoseSnapshot>,
    /// Venue fixtures a fixture test's output covers (some or all of
    /// their channels), sorted: their `fixtures`, `cells` and `poses`
    /// show the test's bytes, which are what leaves for olad.
    pub under_test: Vec<String>,
}

/// A fixture test's output as the stage views see it: the bytes the
/// universe's output thread lays over the show's frame.
#[derive(Clone, Debug, PartialEq)]
pub struct TestOverlay {
    pub universe: u16,
    /// 1-based DMX channel and value.
    pub channels: Vec<(u16, u8)>,
}

/// The overlay in force, shared by the DMX engine (which sets it) and the
/// state sampler (which draws it).
pub type TestOverlayHandle = Arc<parking_lot::RwLock<Option<TestOverlay>>>;

/// Lays a fixture test's bytes over the snapshots of the venue fixtures
/// they cover, so the plot and the 3D view show what leaves for olad, not
/// only what the show computed: each covered channel (and fine byte,
/// mirror, cell channel) takes the test's byte, and a covered pan or tilt
/// moves the fixture's pose, decoded through its own channel definitions.
/// A fixture none of whose channels is covered is untouched. Answers the
/// fixtures it touched, sorted.
pub(crate) fn apply_test_overlay(
    fixtures: &mut Vec<FixtureSnapshot>,
    poses: &mut HashMap<String, crate::lighting::effects::Pose>,
    overlay: &TestOverlay,
    registry: &HashMap<String, crate::lighting::effects::FixtureInfo>,
) -> Vec<String> {
    let bytes: HashMap<u16, u8> = overlay.channels.iter().copied().collect();
    let read = |universe: u16, channel: u16| {
        (universe == overlay.universe)
            .then(|| bytes.get(&channel).copied())
            .flatten()
    };
    let touched = lay_bytes(fixtures, registry, read, Fill::Covered);
    // A covered pan or tilt turns the head.
    poses_from_bytes(poses, &touched, registry, read);
    touched
}

/// Turns each of `names` to the pan and tilt its bytes (`read`: universe,
/// 1-based channel) say, decoded through its own channel definitions: the
/// one reading of a pose off DMX bytes, for a fixture test's
/// ([`apply_test_overlay`]) and the rig's ([`shown_poses_on_wire`]). A
/// fixture neither of whose coarse pan and tilt bytes is read keeps the
/// pose it has (or has none); one with only one of them read keeps the
/// other axis, at rest where it had no pose.
fn poses_from_bytes<'a>(
    poses: &mut HashMap<String, crate::lighting::effects::Pose>,
    names: impl IntoIterator<Item = &'a String>,
    registry: &HashMap<String, crate::lighting::effects::FixtureInfo>,
    read: impl Fn(u16, u16) -> Option<u8>,
) {
    use crate::lighting::effects::{degrees_from_bytes, PhysicalParameter, Pose};
    for name in names {
        let Some(info) = registry.get(name) else {
            continue;
        };
        let at = |offset: u16| read(info.universe, info.dmx_channel(offset)?);
        let mut pose = poses.get(name).copied().unwrap_or(Pose {
            pan: 0.0,
            tilt: 0.0,
        });
        let mut moved = false;
        for parameter in [PhysicalParameter::Pan, PhysicalParameter::Tilt] {
            let Some(def) = info.channel_defs.get(parameter.channel()) else {
                continue;
            };
            let Some(coarse) = at(def.offset) else {
                continue;
            };
            let degrees = degrees_from_bytes(def, parameter, coarse, def.fine.and_then(at));
            match parameter {
                PhysicalParameter::Pan => pose.pan = degrees,
                PhysicalParameter::Tilt => pose.tilt = degrees,
            }
            moved = true;
        }
        if moved {
            poses.insert(name.clone(), pose);
        }
    }
}

/// The poses the rig shows ([`EffectEngine::shown_poses`]), with each
/// mover pose memory has no pose for -- a clear released it, its effect
/// long over -- where the pan and tilt bytes the wire holds point it: the
/// head stays where it was last sent, so its beam is drawn there, as its
/// channels say. A mover pose memory holds keeps that pose (a move's own,
/// at full precision); one no frame has sent a pan or tilt byte rests.
pub(crate) fn shown_poses_on_wire(
    engine: &EffectEngine,
) -> HashMap<String, crate::lighting::effects::Pose> {
    let mut poses = engine.shown_poses().clone();
    let registry = engine.get_fixture_registry();
    let wire = engine.wire();
    let unposed: Vec<String> = registry
        .iter()
        .filter(|(name, info)| info.parent.is_none() && !poses.contains_key(*name))
        .map(|(name, _)| name.clone())
        .collect();
    poses_from_bytes(&mut poses, &unposed, registry, |universe, channel| {
        wire.byte(universe, channel)
    });
    poses
}

/// Which channels of a fixture [`lay_bytes`] lays a byte on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fill {
    /// Those the bytes cover, under every name the wire carries them by:
    /// each channel's, its fine byte's, each mirror's and each cell's. A
    /// fixture test's.
    Covered,
    /// Once the bytes cover any of a fixture's channels, every one, under
    /// the names a snapshot shows -- a fine byte only for a position, as
    /// [`attach_pointing`] names it -- one they do not cover at 0: the
    /// wire's, which holds 0 on a channel no frame has written.
    Fixture,
}

/// Lays bytes read off DMX channels (`read`: universe, 1-based channel)
/// over the snapshots of the fixtures `registry` patches there, a fixture
/// with no snapshot yet getting one: the one reading of a fixture's
/// channels off the wire, for the bytes it holds ([`attach_wire`]) and a
/// fixture test's laid over them ([`apply_test_overlay`]). A pixel
/// fixture's cells are drawn on their own (`cells`) where one differs
/// from what its fixture's channels show, each from what it shows now.
/// Answers the fixtures it touched, sorted.
fn lay_bytes(
    fixtures: &mut Vec<FixtureSnapshot>,
    registry: &HashMap<String, crate::lighting::effects::FixtureInfo>,
    read: impl Fn(u16, u16) -> Option<u8>,
    fill: Fill,
) -> Vec<String> {
    use crate::lighting::channels::is_position_channel;
    let mut touched = Vec::new();
    for (name, info) in registry {
        if info.parent.is_some() {
            continue;
        }
        let at = |offset: u16| read(info.universe, info.dmx_channel(offset)?);
        // Each name with the byte it reads, `None` where none is read.
        let fine_named = |channel: &str| fill == Fill::Covered || is_position_channel(channel);
        let mut names: Vec<(String, Option<u8>)> = Vec::new();
        for (channel, def) in &info.channel_defs {
            names.push((channel.clone(), at(def.offset)));
            if let Some(fine) = def.fine.filter(|_| fine_named(channel)) {
                names.push((fine_name(channel), at(fine)));
            }
            for ((mirror, coarse), fine) in mirror_sections(channel, def) {
                names.push((mirror, at(coarse)));
                if let Some((mirror_fine, fine)) = fine.filter(|_| fine_named(channel)) {
                    names.push((mirror_fine, at(fine)));
                }
            }
        }
        let mut cell_bytes: Vec<(String, String, Option<u8>)> = Vec::new();
        for cell in &info.cells {
            for (channel, def) in &cell.channels {
                cell_bytes.push((cell.name.clone(), channel.clone(), at(def.offset)));
            }
        }
        let covered = names.iter().any(|(_, b)| b.is_some())
            || cell_bytes.iter().any(|(_, _, b)| b.is_some());
        if !covered {
            continue;
        }
        let unread = match fill {
            Fill::Covered => None,
            Fill::Fixture => Some(0),
        };
        let snapshot = match fixtures.iter().position(|f| &f.name == name) {
            Some(i) => &mut fixtures[i],
            None => {
                let at = fixtures.partition_point(|f| f.name.as_str() < name.as_str());
                fixtures.insert(
                    at,
                    FixtureSnapshot {
                        strobe_hz: None,
                        name: name.clone(),
                        channels: HashMap::new(),
                        cells: Default::default(),
                    },
                );
                &mut fixtures[at]
            }
        };
        let cell_bytes: Vec<(String, String, u8)> = cell_bytes
            .into_iter()
            .filter_map(|(cell, channel, b)| Some((cell, channel, b.or(unread)?)))
            .collect();
        // Every cell is drawn on its own once any differs from its
        // fixture, each starting from what it shows now.
        if snapshot.cells.is_empty()
            && cell_bytes
                .iter()
                .any(|(_, channel, b)| snapshot.channels.get(channel).copied().unwrap_or(0) != *b)
        {
            for cell in &info.cells {
                let values = cell
                    .channels
                    .keys()
                    .filter_map(|c| snapshot.channels.get(c).map(|v| (c.clone(), *v)))
                    .collect();
                snapshot.cells.insert(cell.name.clone(), values);
            }
        }
        if !snapshot.cells.is_empty() {
            for (cell, channel, b) in cell_bytes {
                if let Some(values) = snapshot.cells.get_mut(&cell) {
                    values.insert(channel, b);
                }
            }
        }
        for (channel, b) in names {
            if let Some(b) = b.or(unread) {
                snapshot.channels.insert(channel, b);
            }
        }
        touched.push(name.clone());
    }
    touched.sort();
    touched
}

/// Each fixture's channels at the bytes the rig holds ([`Wire`]): a
/// channel nothing drives any more holds the last byte it was sent -- a
/// gobo or a zoom an effect over left set -- and the state says so, as
/// it does for one something drives. A fixture no frame has written a
/// byte of is left as the state has it; one patched on a universe the
/// rig has no output for reads 0 on every channel, as nothing reaches it.
///
/// [`Wire`]: crate::lighting::engine::Wire
fn attach_wire(
    fixtures: &mut Vec<FixtureSnapshot>,
    registry: &HashMap<String, crate::lighting::effects::FixtureInfo>,
    wire: &crate::lighting::engine::Wire,
) {
    lay_bytes(
        fixtures,
        registry,
        // A universe with no output holds nothing: the fixture is dark.
        |universe, channel| match wire.outputs_to(universe) {
            true => wire.byte(universe, channel),
            false => Some(0),
        },
        Fill::Fixture,
    );
}

/// Adds every fixture of `registry` that `fixtures` has no snapshot of,
/// each channel at 0, under the names [`attach_wire`] gives a fixture's
/// channels -- one reading, [`lay_bytes`], of a fixture whose every byte
/// is 0 -- so a fixture nothing touched names its channels as one the
/// rig wrote does. A cell's sub-fixture is its fixture's, not one of its
/// own.
pub(crate) fn fill_dark_fixtures(
    fixtures: &mut Vec<FixtureSnapshot>,
    registry: &HashMap<String, crate::lighting::effects::FixtureInfo>,
) {
    let mut dark = Vec::new();
    lay_bytes(&mut dark, registry, |_, _| Some(0), Fill::Fixture);
    // A fixture with no channels at all is still there, dark.
    for (name, info) in registry {
        if info.parent.is_none() && !dark.iter().any(|d: &FixtureSnapshot| &d.name == name) {
            dark.push(FixtureSnapshot {
                strobe_hz: None,
                name: name.clone(),
                channels: HashMap::new(),
                cells: Default::default(),
            });
        }
    }
    dark.retain(|d| !fixtures.iter().any(|f| f.name == d.name));
    fixtures.extend(dark);
    fixtures.sort_by(|a, b| a.name.cmp(&b.name));
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
            let calibration = fixture.calibration();
            let aim = calibration.direction(rotation, *pose);
            // The beam leaves the lens, a head's length from the mounting
            // point and moving with the pose.
            let lens = calibration.origin(rotation, *pose);
            let floor = fixture.position.and_then(|p| {
                let p = [p[0] + lens[0], p[1] + lens[1], p[2] + lens[2]];
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
    overlay: Option<TestOverlayHandle>,
    tx: Arc<watch::Sender<Arc<StateSnapshot>>>,
    cancel: tokio_util::sync::CancellationToken,
) -> JoinHandle<()> {
    tokio::spawn(sampler_loop_cancellable(effect_engine, overlay, tx, cancel))
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
            .map(|(name, info)| (name.clone(), info.has_dimmer()))
            .collect()
    })
    .await
    .unwrap_or_default()
}

/// Samples the current fixture state from the effect engine (on the blocking
/// thread pool) and returns a snapshot, or `None` if the blocking task panicked.
async fn sample_tick(
    effect_engine: &Arc<Mutex<EffectEngine>>,
    overlay: Option<&TestOverlayHandle>,
    has_dimmer_map: &HashMap<String, bool>,
) -> Option<Arc<StateSnapshot>> {
    let engine_ref = effect_engine.clone();
    let has_dimmer_map = has_dimmer_map.clone();
    let overlay = overlay.and_then(|o| o.read().clone());
    let snapshot = tokio::task::spawn_blocking(move || {
        let engine = engine_ref.lock();
        engine_snapshot(&engine, overlay.as_ref(), &has_dimmer_map)
    })
    .await
    .ok()?;
    Some(Arc::new(snapshot))
}

/// The state snapshot of an effect engine, with a fixture test's bytes
/// laid over the fixtures they cover.
pub(crate) fn engine_snapshot(
    engine: &EffectEngine,
    overlay: Option<&TestOverlay>,
    has_dimmer_map: &HashMap<String, bool>,
) -> StateSnapshot {
    // A cell's sub-fixture is part of its fixture, not a fixture of
    // its own to the stream (design §17.4): its state folds into the
    // fixture's `cells`.
    let registry = engine.get_fixture_registry();
    // What the rig shows: mid-crossfade, the blend going out (#555).
    let mut fixtures = fixture_snapshots_with_cells(
        engine.shown_fixture_states(),
        has_dimmer_map,
        registry,
        engine.wire(),
        StrobeRates::Live,
    );
    let mut active_effects: Vec<String> = engine.get_active_effects().keys().cloned().collect();
    active_effects.sort();
    let (poses, under_test) = match overlay {
        None => (
            compute_pose_snapshots(&shown_poses_on_wire(engine), registry),
            Vec::new(),
        ),
        Some(overlay) => {
            let mut poses = shown_poses_on_wire(engine);
            let under_test = apply_test_overlay(&mut fixtures, &mut poses, overlay, registry);
            attach_strobe_rates(&mut fixtures, registry, StrobeRates::Live);
            (compute_pose_snapshots(&poses, registry), under_test)
        }
    };
    StateSnapshot {
        fixtures,
        active_effects,
        poses,
        under_test,
    }
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

        if let Some(snapshot) = sample_tick(&effect_engine, None, &has_dimmer_map).await {
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
    overlay: Option<TestOverlayHandle>,
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

        if let Some(snapshot) = sample_tick(&effect_engine, overlay.as_ref(), &has_dimmer_map).await
        {
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
/// Each channel shows the byte it goes out as ([`wire_byte`]) where
/// `registry` knows the fixture's channel.
pub(crate) fn compute_fixture_snapshots(
    states: &HashMap<String, FixtureState>,
    has_dimmer_map: &HashMap<String, bool>,
    registry: &HashMap<String, crate::lighting::effects::FixtureInfo>,
) -> Vec<FixtureSnapshot> {
    let mut snapshots: Vec<FixtureSnapshot> = states
        .iter()
        .map(|(name, state)| {
            let has_dedicated_dimmer = has_dimmer_map.get(name).copied().unwrap_or(false);
            let info = registry.get(name);
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
                channels.insert(channel_name.clone(), wire_byte(info, channel_name, value));
            }

            FixtureSnapshot {
                // A software strobe's rate; a strobe channel's is read
                // from its byte once the registry is at hand.
                strobe_hz: state.strobe_hz,
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
    wire: &crate::lighting::engine::Wire,
    rates: StrobeRates,
) -> Vec<FixtureSnapshot> {
    let mut fixtures = compute_fixture_snapshots(states, has_dimmer_map, registry);
    fixtures.retain(|s| registry.get(&s.name).is_none_or(|f| f.parent.is_none()));
    drop_unsent_primaries(&mut fixtures, registry);
    attach_cell_snapshots(&mut fixtures, states, registry);
    attach_pointing(&mut fixtures, states, registry);
    attach_mirrors(&mut fixtures, registry);
    attach_wire(&mut fixtures, registry, wire);
    attach_strobe_rates(&mut fixtures, registry, rates);
    fixtures
}

/// The byte a snapshot shows for `channel` at `value` (0 to 1): the
/// coarse byte it goes out as on the fixture's channel (a 16-bit
/// channel's high byte, [`coarse_byte`](crate::lighting::effects::coarse_byte)),
/// or an 8-bit channel's byte for a name the fixture has no channel by.
fn wire_byte(
    info: Option<&crate::lighting::effects::FixtureInfo>,
    channel: &str,
    value: f64,
) -> u8 {
    crate::lighting::effects::coarse_byte(
        info.and_then(|info| info.channel_defs.get(channel)),
        value,
    )
}

/// A CMY fixture's colour as the head is sent it (#661): the engine keeps
/// colour in red, green and blue, which such a fixture has no channels
/// for, and sends each driven primary as the flag that removes it
/// ([`FixtureState::cmy_flags`], the one conversion). The snapshot shows
/// the flags as the wire holds them ([`attach_wire`]) and no red, green or
/// blue, so every reading of it -- whether the rig is dark, the colour a
/// stage view draws -- judges the fixture by what it emits.
fn drop_unsent_primaries(
    snapshots: &mut [FixtureSnapshot],
    registry: &HashMap<String, crate::lighting::effects::FixtureInfo>,
) {
    for snapshot in snapshots.iter_mut() {
        let Some(info) = registry.get(&snapshot.name) else {
            continue;
        };
        if crate::lighting::effects::has_cmy(&info.channel_defs) {
            for (rgb, _) in CMY_OF_RGB {
                snapshot.channels.remove(rgb);
            }
        }
    }
}

/// Reads each fixture's strobe byte back into the rate it means through
/// the fixture's own strobe rule, for a fixture with a strobe channel; a
/// software strobe's rate (no strobe channel) came with the engine's
/// state and stays only where `rates` says so ([`StrobeRates`]). Run
/// again after a test overlay changes the bytes.
pub(crate) fn attach_strobe_rates(
    fixtures: &mut [FixtureSnapshot],
    registry: &HashMap<String, crate::lighting::effects::FixtureInfo>,
    rates: StrobeRates,
) {
    for snapshot in fixtures {
        let Some(info) = registry.get(&snapshot.name) else {
            continue;
        };
        match snapshot.channels.get("strobe") {
            Some(byte) => snapshot.strobe_hz = info.strobe_hz(*byte),
            None if rates == StrobeRates::Live => snapshot.strobe_hz = None,
            None => {}
        }
    }
}

/// The sections mirrored to `channel`, from the second: each one's name
/// in a snapshot and offset, `{channel}#{n}`, and its fine byte's,
/// `{channel}_fine#{n}`, where it has one.
fn mirror_sections<'a>(
    channel: &'a str,
    def: &'a crate::lighting::types::ChannelDef,
) -> impl Iterator<Item = ((String, u16), Option<(String, u16)>)> + 'a {
    def.mirrors
        .iter()
        .enumerate()
        .map(move |(i, &(coarse, fine))| {
            let n = i + 2;
            (
                (mirror_name(channel, n), coarse),
                fine.map(|fine| (mirror_name(&fine_name(channel), n), fine)),
            )
        })
}

/// The bytes a ganged channel repeats on the wire, named: a channel with
/// mirrors (an LED bar's sections ganged to one colour, a wash's zones)
/// writes every mirror the same byte, and the snapshot
/// says so under `<channel>#2`, `<channel>#3`, ... (and `_fine` likewise),
/// so a state reader sees what the wire carries. A pixel fixture's cells
/// are their own entries already and are not repeated here.
fn attach_mirrors(
    snapshots: &mut [FixtureSnapshot],
    registry: &HashMap<String, crate::lighting::effects::FixtureInfo>,
) {
    for snapshot in snapshots.iter_mut() {
        let Some(info) = registry.get(&snapshot.name) else {
            continue;
        };
        let cell_owned = |channel: &str| {
            info.cells
                .first()
                .is_some_and(|c| c.channels.contains_key(channel))
        };
        let mut extra = Vec::new();
        for (name, def) in &info.channel_defs {
            if def.mirrors.is_empty() || cell_owned(name) {
                continue;
            }
            let Some(&coarse) = snapshot.channels.get(name) else {
                continue;
            };
            let fine = snapshot.channels.get(&fine_name(name)).copied();
            for ((mirror, _), mirror_fine) in mirror_sections(name, def) {
                extra.push((mirror, coarse));
                if let (Some(fine), Some((mirror_fine, _))) = (fine, mirror_fine) {
                    extra.push((mirror_fine, fine));
                }
            }
        }
        for (name, value) in extra {
            // Never over a channel of the fixture's own; `#` is refused in
            // channel names by the parser, so this is belt and braces.
            snapshot.channels.entry(name).or_insert(value);
        }
    }
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
                    snapshot.channels.insert(fine_name(channel), byte);
                }
            }
        }
    }
}

pub use crate::lighting::channels::is_level_channel;

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
            let own = states.get(&crate::lighting::effects::cell_fixture_name(
                parent, &cell.name,
            ));
            // Each cell channel at the coarse byte it goes out as on the
            // cell's own channel, as the DMX output resolves it.
            let values: HashMap<String, u8> = parent_state
                .cell_values(cell, own)
                .into_iter()
                .map(|(name, value)| {
                    let byte =
                        crate::lighting::effects::coarse_byte(Some(&cell.channels[&name]), value);
                    (name, byte)
                })
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
                        strobe_hz: None,
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
        let snapshots = compute_fixture_snapshots(&states, &has_dimmer, &HashMap::new());

        assert_eq!(snapshots.len(), 1);
        assert_eq!(snapshots[0].name, "test_fixture");
        assert_eq!(*snapshots[0].channels.get("red").unwrap(), 255);
        assert_eq!(*snapshots[0].channels.get("green").unwrap(), 128);
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
        let snapshots = compute_fixture_snapshots(&states, &has_dimmer, &HashMap::new());

        assert_eq!(snapshots.len(), 1);
        // Red should remain 255 (not reduced by multiplier when fixture has dedicated dimmer)
        assert_eq!(*snapshots[0].channels.get("red").unwrap(), 255);
        assert_eq!(*snapshots[0].channels.get("dimmer").unwrap(), 128);
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
        let snapshots = compute_fixture_snapshots(&states, &has_dimmer, &HashMap::new());

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

        let snapshots = compute_fixture_snapshots(&states, &HashMap::new(), &HashMap::new());

        assert_eq!(snapshots[0].name, "alpha");
        assert_eq!(snapshots[1].name, "middle");
        assert_eq!(snapshots[2].name, "zebra");
    }

    #[test]
    fn test_compute_fixture_snapshots_empty() {
        let states = HashMap::new();
        let snapshots = compute_fixture_snapshots(&states, &HashMap::new(), &HashMap::new());
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
        let snapshots = compute_fixture_snapshots(&states, &has_dimmer, &HashMap::new());
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
        let snapshots = compute_fixture_snapshots(&states, &has_dimmer, &HashMap::new());
        assert_eq!(snapshots.len(), 1);
        assert_eq!(*snapshots[0].channels.get("red").unwrap(), 128);
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
            strobe_hz: None,
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
                strobe_hz: None,
                name: "f1".to_string(),
                channels: HashMap::new(),
                cells: Default::default(),
            }],
            active_effects: vec!["effect1".to_string()],
            poses: Vec::new(),
            under_test: Vec::new(),
        };
        let cloned = snapshot.clone();
        assert_eq!(cloned.fixtures.len(), 1);
        assert_eq!(cloned.active_effects.len(), 1);
    }

    /// A rig on universe 1: "front" (dimmer, red, green, blue at 11-14),
    /// "side" (the same at 13-16), "far" at 100, "other" at 11 of
    /// universe 2, and "mover" at 30 with a 16-bit pan over -270..270
    /// (30, 31) and an 8-bit tilt (32). The show holds front and side blue.
    fn overlay_rig() -> EffectEngine {
        use crate::lighting::effects::{EffectInstance, EffectType};
        use crate::lighting::types::{PhysicalRange, PhysicalUnit};
        let par = |name: &str, universe: u16, address: u16| {
            FixtureInfo::new(
                name.to_string(),
                universe,
                address,
                "Par".to_string(),
                [("dimmer", 1), ("red", 2), ("green", 3), ("blue", 4)]
                    .into_iter()
                    .map(|(n, o)| (n.to_string(), o))
                    .collect(),
                None,
            )
        };
        let mut engine = EffectEngine::new();
        engine.register_fixture(par("front", 1, 11));
        engine.register_fixture(par("side", 1, 13));
        engine.register_fixture(par("far", 1, 100));
        engine.register_fixture(par("other", 2, 11));
        let mut mover = FixtureInfo::new(
            "mover".to_string(),
            1,
            30,
            "Mover".to_string(),
            [("pan", 1), ("tilt", 3)]
                .into_iter()
                .map(|(n, o)| (n.to_string(), o))
                .collect(),
            None,
        );
        mover.channel_defs.insert(
            "pan".to_string(),
            ChannelDef {
                fine: Some(2),
                range: Some(PhysicalRange {
                    from: -270.0,
                    to: 270.0,
                    unit: PhysicalUnit::Degrees,
                }),
                ..ChannelDef::at(1)
            },
        );
        mover.position = Some([0.0, 0.0, 5.0]);
        engine.register_fixture(mover);
        engine
            .start_effect(EffectInstance::new(
                "blue".into(),
                EffectType::Static {
                    parameters: [("blue".to_string(), 1.0), ("dimmer".to_string(), 1.0)]
                        .into_iter()
                        .collect(),
                    duration: std::time::Duration::from_secs(60),
                },
                vec!["front".into(), "side".into(), "far".into()],
                None,
                None,
                None,
            ))
            .unwrap();
        engine
            .update(std::time::Duration::from_millis(23), None)
            .unwrap();
        engine
    }

    fn dimmers() -> HashMap<String, bool> {
        ["front", "side", "far", "other"]
            .into_iter()
            .map(|n| (n.to_string(), true))
            .collect()
    }

    fn rgb(snapshot: &StateSnapshot, name: &str) -> [u8; 4] {
        let f = snapshot
            .fixtures
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("{name} has no snapshot"));
        ["dimmer", "red", "green", "blue"].map(|c| f.channels.get(c).copied().unwrap_or(0))
    }

    #[test]
    fn a_fixture_test_shows_on_the_venue_fixtures_it_covers() {
        let engine = overlay_rig();
        let show = engine_snapshot(&engine, None, &dimmers());
        assert_eq!(rgb(&show, "front"), [255, 0, 0, 255]);
        assert!(show.under_test.is_empty());

        // The test: red, full, at 11-14.
        let overlay = TestOverlay {
            universe: 1,
            channels: vec![(11, 255), (12, 255), (13, 0), (14, 0)],
        };
        let tested = engine_snapshot(&engine, Some(&overlay), &dimmers());
        // Fully covered.
        assert_eq!(rgb(&tested, "front"), [255, 255, 0, 0]);
        // Partly covered (13, 14 are its dimmer and red): only those.
        assert_eq!(rgb(&tested, "side"), [0, 0, 0, 255]);
        assert_eq!(tested.under_test, vec!["front", "side"]);
        // Outside the span, or on another universe: untouched.
        assert_eq!(rgb(&tested, "far"), rgb(&show, "far"));
        assert!(tested.fixtures.iter().all(|f| f.name != "other"));

        // Released: the show again, nothing marked.
        assert_eq!(engine_snapshot(&engine, None, &dimmers()), show);
    }

    #[test]
    fn a_covered_pan_turns_the_head() {
        let engine = overlay_rig();
        let rest = engine_snapshot(&engine, None, &dimmers());
        let rest_pose = rest.poses.iter().find(|p| p.name == "mover").unwrap();
        assert_eq!(rest_pose.pan, 0.0);

        // 45° on a 16-bit -270..270 pan, through the engine's own bytes.
        let def = &engine.get_fixture_registry()["mover"].channel_defs["pan"];
        let bytes = crate::lighting::effects::resolve_degrees(
            def,
            crate::lighting::effects::PhysicalParameter::Pan,
            45.0,
        )
        .bytes;
        let overlay = TestOverlay {
            universe: 1,
            channels: bytes.iter().map(|(o, b)| (29 + o, *b)).collect(),
        };
        let tested = engine_snapshot(&engine, Some(&overlay), &dimmers());
        let pose = tested.poses.iter().find(|p| p.name == "mover").unwrap();
        assert!((pose.pan - 45.0).abs() < 0.01, "{}", pose.pan);
        assert_eq!(pose.tilt, 0.0, "tilt is not covered");

        // With the tilt byte too (8-bit, the fallback travel), the beam
        // leaves straight down.
        let tilt = crate::lighting::effects::resolve_degrees(
            &engine.get_fixture_registry()["mover"].channel_defs["tilt"],
            crate::lighting::effects::PhysicalParameter::Tilt,
            30.0,
        )
        .bytes[0]
            .1;
        let mut both = overlay.clone();
        both.channels.push((32, tilt));
        let tilted = engine_snapshot(&engine, Some(&both), &dimmers());
        let pose = tilted.poses.iter().find(|p| p.name == "mover").unwrap();
        assert!((pose.tilt - 30.0).abs() < 1.1, "{}", pose.tilt);
        assert_ne!(pose.aim, rest_pose.aim);
        assert_eq!(tested.under_test, vec!["mover"]);
        let mover = tested.fixtures.iter().find(|f| f.name == "mover").unwrap();
        assert_eq!(mover.channels["pan"], bytes[0].1);
        assert_eq!(mover.channels["pan_fine"], bytes[1].1);
    }

    #[tokio::test]
    async fn test_start_sampler_empty_engine() {
        let engine = Arc::new(Mutex::new(EffectEngine::new()));
        let (mut rx, handle) = start_sampler(engine);

        // Wait for the sampler to produce a snapshot
        let result = tokio::time::timeout(crate::testutil::PATIENCE, rx.changed()).await;
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

        let result = tokio::time::timeout(crate::testutil::PATIENCE, rx.changed()).await;
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
            let channels: HashMap<String, u16> = [("red", 1u16), ("green", 2), ("blue", 3)]
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
        let first = tokio::time::timeout(crate::testutil::PATIENCE, rx.changed()).await;
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
        let changed = tokio::time::timeout(crate::testutil::PATIENCE, rx.changed()).await;
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
        let name = crate::lighting::effects::cell_fixture_name(parent, &cell.name);
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
        let mut snapshots = compute_fixture_snapshots(&bar_only, &has_dimmer, &HashMap::new());
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

    /// A cell's snapshot is the bytes the head is sent: a 16-bit cell
    /// channel shows its coarse byte on the wire, the 16-bit value's high
    /// byte.
    #[test]
    fn a_cell_snapshot_is_what_the_head_is_sent() {
        use crate::lighting::effects::{EffectInstance, EffectType};
        let mut info = bar("Bar");
        // Cell 2's red is 16-bit, its fine byte at 11.
        info.cells[1].channels.get_mut("red").unwrap().fine = Some(11);
        let red_2 = info.cells[1].channels["red"].offset;
        let mut engine = EffectEngine::new();
        engine.register_fixture(info);
        engine
            .start_effect(EffectInstance::new(
                "cell".to_string(),
                EffectType::Static {
                    parameters: HashMap::from([("red".to_string(), 0.3)]),
                    duration: std::time::Duration::from_secs(5),
                },
                vec!["Bar/2".to_string()],
                None,
                None,
                None,
            ))
            .unwrap();
        let sent: HashMap<u16, u8> = engine
            .update(std::time::Duration::from_millis(10), None)
            .unwrap()
            .iter()
            .map(|c| (c.channel, c.value))
            .collect();
        let snapshot = engine_snapshot(&engine, None, &HashMap::from([("Bar".to_string(), true)]));
        let cell = &snapshot.fixtures[0].cells["2"];
        assert_eq!(cell.get("red"), sent.get(&red_2));
        assert_eq!(cell["red"], 76, "0.3 of 65535 is 0x4CCD: coarse 76");
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
        let mut snapshots = compute_fixture_snapshots(&states, &has_dimmer, &HashMap::new());
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
                strobe_hz: None,
                name: "Aaa".to_string(),
                channels: HashMap::new(),
                cells: Default::default(),
            },
            FixtureSnapshot {
                strobe_hz: None,
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

        let snapshots = fixture_snapshots_with_cells(
            &states,
            &HashMap::new(),
            &registry,
            &Default::default(),
            StrobeRates::Live,
        );
        let m = &snapshots[0].channels;
        // Pan 0° over ±270° is 32768: 0x80 0x00. Tilt at the top of its
        // range is 65535: 0xFF 0xFF.
        assert_eq!((m["pan"], m["pan_fine"]), (0x80, 0x00));
        assert_eq!((m["tilt"], m["tilt_fine"]), (0xFF, 0xFF));
        assert!(!m.contains_key("dimmer"), "nothing wrote the dimmer");
    }

    /// A ganged channel's mirror bytes are in the snapshot under numbered
    /// names, so a linked head or a ganged bar section reads as the wire
    /// carries it; an unganged channel gets none.
    #[test]
    fn fixture_snapshots_name_a_ganged_channels_mirrors() {
        use crate::lighting::effects::{BlendMode, ChannelState, EffectLayer};
        use crate::lighting::types::ChannelDef;

        let channels: HashMap<String, u16> = [("red", 1), ("dimmer", 4)]
            .into_iter()
            .map(|(n, o)| (n.to_string(), o))
            .collect();
        let mut info = crate::lighting::effects::FixtureInfo::new(
            "bar".to_string(),
            1,
            1,
            "Bar".to_string(),
            channels,
            None,
        );
        let mut defs = HashMap::new();
        defs.insert(
            "red".to_string(),
            ChannelDef {
                offset: 1,
                fine: None,
                range: None,
                functions: Vec::new(),
                mirrors: vec![(2, None), (3, None)],
            },
        );
        defs.insert("dimmer".to_string(), ChannelDef::at(4));
        info = info.with_channel_defs(defs);
        let registry: HashMap<String, _> = [("bar".to_string(), info)].into_iter().collect();

        let mut state = FixtureState::new();
        state.set_channel(
            "red".to_string(),
            ChannelState::new(1.0, EffectLayer::Background, BlendMode::Replace),
        );
        let states: HashMap<String, FixtureState> =
            [("bar".to_string(), state)].into_iter().collect();
        let snapshots = fixture_snapshots_with_cells(
            &states,
            &HashMap::new(),
            &registry,
            &Default::default(),
            StrobeRates::Live,
        );
        let bar = &snapshots[0].channels;
        assert_eq!((bar["red"], bar["red#2"], bar["red#3"]), (255, 255, 255));
        assert!(
            !bar.contains_key("dimmer#2"),
            "an unganged channel has no mirrors"
        );
        assert!(is_level_channel("red#2") && !is_level_channel("pan#2"));
        assert!(!is_level_channel("zoom#2") && !is_level_channel("gobo"));
    }

    /// A CMY fixture's snapshot is the bytes the head is sent (#661): its
    /// colour driven through red, green and blue reads as the flags the
    /// engine writes, over a `static` naming a flag directly, and the
    /// stage views and `is_dark` judge it by those.
    #[test]
    fn a_cmy_snapshot_is_what_the_head_is_sent() {
        use crate::lighting::effects::{EffectInstance, EffectType};
        let channels: HashMap<String, u16> =
            [("dimmer", 1), ("cyan", 2), ("magenta", 3), ("yellow", 4)]
                .into_iter()
                .map(|(n, o)| (n.to_string(), o))
                .collect();
        let mut info = FixtureInfo::new("f".to_string(), 1, 1, "T".to_string(), channels, None);
        // A 16-bit dimmer, its fine byte at 5: the snapshot shows the
        // coarse byte on the wire, the high byte of the 16-bit value.
        info.channel_defs.get_mut("dimmer").unwrap().fine = Some(5);
        let mut engine = EffectEngine::new();
        engine.register_fixture(info);
        let stat = |id: &str, parameters: &[(&str, f64)]| {
            EffectInstance::new(
                id.to_string(),
                EffectType::Static {
                    parameters: parameters
                        .iter()
                        .map(|&(n, v)| (n.to_string(), v))
                        .collect(),
                    duration: std::time::Duration::from_secs(5),
                },
                vec!["f".to_string()],
                None,
                None,
                None,
            )
        };
        engine
            .start_effect(stat(
                "white",
                &[
                    ("dimmer", 0.3),
                    ("cyan", 0.0),
                    ("magenta", 0.0),
                    ("yellow", 0.0),
                ],
            ))
            .unwrap();
        engine.start_effect(stat("red", &[("red", 0.25)])).unwrap();
        let frame = engine
            .update(std::time::Duration::from_millis(10), None)
            .unwrap()
            .to_vec();
        // Out to the rig, whose bytes the snapshot reads.
        engine.send(&frame);
        let sent: HashMap<u16, u8> = frame.iter().map(|c| (c.channel, c.value)).collect();
        let snapshot = engine_snapshot(&engine, None, &HashMap::from([("f".to_string(), true)]));
        let f = &snapshot.fixtures[0];
        for (name, address) in [("dimmer", 1), ("cyan", 2), ("magenta", 3), ("yellow", 4)] {
            assert_eq!(f.channels.get(name), sent.get(&address), "{name}");
        }
        assert_eq!(f.channels["cyan"], 191, "red at a quarter: cyan at three");
        assert_eq!(
            f.channels["dimmer"], 76,
            "0.3 of 65535 is 0x4CCD: coarse 76"
        );
        assert!(!f.channels.contains_key("red"), "{:?}", f.channels);
    }
}
