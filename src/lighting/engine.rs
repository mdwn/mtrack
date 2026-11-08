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

mod layers;
pub(crate) mod processing;
mod validation;
mod wire;

#[cfg(test)]
mod tests;

pub use wire::Wire;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use std::sync::atomic::{AtomicU64, Ordering};

use super::effects::*;
use super::tempo::TempoMap;
use crate::lighting::timeline::Step;
use layers::{LayerState, Now};
use tracing::debug;

use crate::dmx::midi_dmx_store::MidiDmxStore;
pub(crate) use crate::lighting::channels::{base_channel, fades_to_dark, is_position_channel};
use crate::lighting::channels::{dmx_byte, dmx_word, CrossKind};

/// The next [`EffectInstance::start_order`]: process-wide, as a seek's
/// scratch engine hands its effects to the live one.
fn next_start_order() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Whether a layer speed master scales its effects' time: 1 does not, and
/// nor does 0, which freezes the layer instead.
pub(crate) fn speed_scales(speed: f64) -> bool {
    (speed - 1.0).abs() >= f64::EPSILON && speed != 0.0
}

/// `elapsed` at a layer speed master (see [`speed_scales`]).
pub(crate) fn scale_by_speed(elapsed: Duration, speed: f64) -> Duration {
    if speed_scales(speed) {
        Duration::try_from_secs_f64(elapsed.as_secs_f64() * speed).unwrap_or(Duration::MAX)
    } else {
        elapsed
    }
}

/// The song time `elapsed` of an effect's own time takes at a layer speed
/// master: the inverse of [`scale_by_speed`].
pub(crate) fn unscale_by_speed(elapsed: Duration, speed: f64) -> Duration {
    if speed_scales(speed) {
        Duration::try_from_secs_f64(elapsed.as_secs_f64() / speed).unwrap_or(Duration::MAX)
    } else {
        elapsed
    }
}

/// Registry of known fixtures and their DMX mappings.
#[derive(Clone)]
pub(crate) struct FixtureRegistry {
    fixtures: HashMap<String, FixtureInfo>,
    /// Each pixel fixture's sub-fixture names (`parent/cell`), in cell
    /// order (design §17.3).
    cells: HashMap<String, Vec<String>>,
    /// Reverse map from (universe_id, dmx_channel) to (fixture_name, channel_name).
    /// Only built during tests for validation; not needed in production.
    #[cfg(test)]
    dmx_to_fixture_map: HashMap<(u16, u16), (String, String)>,
}

impl FixtureRegistry {
    fn new() -> Self {
        Self {
            fixtures: HashMap::new(),
            cells: HashMap::new(),
            #[cfg(test)]
            dmx_to_fixture_map: HashMap::new(),
        }
    }

    /// The sub-fixture names of a pixel fixture, in cell order; empty for
    /// a fixture without cells.
    fn cells_of(&self, name: &str) -> &[String] {
        self.cells.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    fn register(&mut self, fixture: FixtureInfo) {
        // A pixel fixture's cells register beside it as sub-fixtures: the
        // parent's patch and rotation, the cell's channels, a position of
        // the parent's plus the rotated cell offset, no pan or tilt.
        if fixture.parent.is_none() && !fixture.cells.is_empty() {
            let mut names = Vec::with_capacity(fixture.cells.len());
            for cell in &fixture.cells {
                let name = cell_fixture_name(&fixture.name, &cell.name);
                let channels: HashMap<String, u16> = cell
                    .channels
                    .iter()
                    .map(|(n, d)| (n.clone(), d.offset))
                    .collect();
                let mut sub = FixtureInfo::new(
                    name.clone(),
                    fixture.universe,
                    fixture.address,
                    fixture.fixture_type.clone(),
                    channels,
                    fixture.max_strobe_frequency,
                );
                sub.min_strobe_frequency = fixture.min_strobe_frequency;
                sub.strobe_dmx_offset = fixture.strobe_dmx_offset;
                sub.channel_defs = cell.channels.clone();
                sub.rotation = fixture.rotation;
                sub.position = fixture.position.map(|p| {
                    let o = super::effects::out_of_frame(
                        fixture.rotation.unwrap_or([0.0; 3]),
                        cell.offset,
                    );
                    [p[0] + o[0], p[1] + o[1], p[2] + o[2]]
                });
                sub.parent = Some(fixture.name.clone());
                self.fixtures.insert(name.clone(), sub);
                names.push(name);
            }
            self.cells.insert(fixture.name.clone(), names);
        }
        #[cfg(test)]
        {
            for (channel_name, &offset) in &fixture.channels {
                if let Some(channel) = fixture.dmx_channel(offset) {
                    self.dmx_to_fixture_map.insert(
                        (fixture.universe, channel),
                        (fixture.name.clone(), channel_name.clone()),
                    );
                }
            }
        }
        self.fixtures.insert(fixture.name.clone(), fixture);
    }

    fn get(&self, name: &str) -> Option<&FixtureInfo> {
        self.fixtures.get(name)
    }

    fn contains(&self, name: &str) -> bool {
        self.fixtures.contains_key(name)
    }

    fn as_map(&self) -> &HashMap<String, FixtureInfo> {
        &self.fixtures
    }

    #[cfg(test)]
    fn lookup_dmx_channel(&self, universe: u16, dmx_channel: u16) -> Option<&(String, String)> {
        self.dmx_to_fixture_map.get(&(universe, dmx_channel))
    }
}

/// Caches the last computed DmxCommands so unchanged frames skip recomputation.
struct EffectCache {
    commands: Vec<DmxCommand>,
    /// Last-seen MIDI DMX store generation.
    last_store_generation: u32,
    /// True when engine state changed outside update() (e.g. clear_layer).
    dirty: bool,
}

impl EffectCache {
    fn new() -> Self {
        Self {
            commands: Vec::new(),
            last_store_generation: 0,
            dirty: false,
        }
    }

    fn invalidate(&mut self) {
        self.dirty = true;
    }

    fn get_cached(&self) -> &[DmxCommand] {
        &self.commands
    }

    fn update(&mut self, commands: Vec<DmxCommand>, generation: u32) {
        self.commands = commands;
        self.last_store_generation = generation;
        self.dirty = false;
    }
}

/// The main effects engine that manages and processes lighting effects
pub struct EffectEngine {
    active_effects: HashMap<String, EffectInstance>,
    fixtures: FixtureRegistry,
    current_time: Instant,
    /// Elapsed simulated time since engine start
    engine_elapsed: Duration,
    /// Optional tempo map for tempo-aware effects (measure/beat-based timing)
    tempo_map: Option<TempoMap>,
    layer_state: LayerState,
    /// Last computed merged fixture states (for preview/debugging)
    last_merged_states: HashMap<String, FixtureState>,
    /// While another engine's frame is crossed into this one's (a loop
    /// crossfade or a loop break's fade, #555): the blend of the two, which
    /// is what the rig shows -- fixture states and poses -- rather than this
    /// engine's own.
    shown: Option<(HashMap<String, FixtureState>, HashMap<String, Pose>)>,
    /// Last known song time (score-time) for tempo-aware speed lookups.
    last_song_time: Option<Duration>,
    /// Reference to the MIDI DMX store for reading interpolated values each frame.
    midi_dmx_store: Option<Arc<parking_lot::RwLock<MidiDmxStore>>>,
    cache: EffectCache,
    /// Sub-phase indicator for update() progress.
    update_subphase: Arc<AtomicU64>,
    /// The current venue's focus points, in stage coordinates.
    focus_points: HashMap<String, [f64; 3]>,
    /// Pose memory: where each mover actually is — the last pose emitted
    /// after the slew clamp. A `move` without `from` starts here.
    poses: HashMap<String, Pose>,
    /// Where each mover was last asked to be. A mover nobody is driving
    /// keeps travelling toward this, no faster than its limit, and holds
    /// it on arrival until the next move or a clear (design §15.4).
    goals: HashMap<String, Pose>,
    /// The last tick's length, for the slew clamp.
    last_dt: Duration,
    /// Fixtures whose strobe channel was driven on the last full frame.
    /// A hardware strobe latches: the fixture keeps flashing at the last
    /// rate it was sent until told otherwise, so a strobe nobody drives
    /// any more is sent off rather than left at its last byte. Survives
    /// clears and song changes on purpose — those are exactly when the
    /// strobe stops being driven.
    strobe_driven: HashSet<String>,
    /// A reckoner (see [`Self::reckoner`]): there is no rig, so effects
    /// start as they are, unvalidated.
    rigless: bool,
    /// While a reckoner reckons a show's ends: when each effect taken out
    /// ended, `None` for one that never does.
    ends: Option<HashMap<String, Option<Duration>>>,
    /// A scratch's scope (see [`Self::scratch`]).
    scope: StopScope,
    /// A `StopScope::Nothing` scratch's use of the pose memory it started
    /// from.
    pose_reads: Option<PoseReads>,
    /// The bytes the rig holds ([`Wire`]): what the frames sent through
    /// [`Self::send`] left there. A fresh engine swapped in for this one
    /// (a loop pass, a break) takes it over, as the universes do not
    /// change.
    wire: Wire,
    /// Whether a start from history sends the frames playing through it
    /// would have to the wire ([`Self::trace_wire`]).
    tracing_wire: bool,
}

impl Default for EffectEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl EffectEngine {
    pub fn new() -> Self {
        Self {
            active_effects: HashMap::new(),
            fixtures: FixtureRegistry::new(),
            current_time: Instant::now(),
            engine_elapsed: Duration::ZERO,
            tempo_map: None,
            layer_state: LayerState::default(),
            last_merged_states: HashMap::new(),
            shown: None,
            last_song_time: None,
            midi_dmx_store: None,
            cache: EffectCache::new(),
            update_subphase: Arc::new(AtomicU64::new(0)),
            focus_points: HashMap::new(),
            poses: HashMap::new(),
            goals: HashMap::new(),
            last_dt: Duration::ZERO,
            strobe_driven: HashSet::new(),
            rigless: false,
            ends: None,
            scope: StopScope::Nothing,
            pose_reads: None,
            wire: Wire::default(),
            tracing_wire: false,
        }
    }

    /// Returns the shared update sub-phase Arc for external monitoring.
    pub fn update_subphase(&self) -> Arc<AtomicU64> {
        self.update_subphase.clone()
    }

    /// Returns the current MIDI DMX store generation, or 0 if no store is set.
    fn store_generation(&self) -> u32 {
        self.midi_dmx_store
            .as_ref()
            .map(|s| s.read().generation())
            .unwrap_or(0)
    }

    /// Set the tempo map for tempo-aware effects
    pub fn set_tempo_map(&mut self, tempo_map: Option<TempoMap>) {
        self.tempo_map = tempo_map;
    }

    /// Returns whether a tempo map is currently set
    #[cfg(test)]
    pub fn has_tempo_map(&self) -> bool {
        self.tempo_map.is_some()
    }

    /// Format effect type for logging
    fn format_effect_for_logging(effect: &EffectInstance) -> (&'static str, String) {
        match &effect.effect_type {
            EffectType::Static {
                parameters,
                duration,
            } => (
                "Static",
                format!("params={:?}, duration={:?}", parameters, duration),
            ),
            EffectType::ColorCycle {
                colors,
                speed,
                direction,
                transition,
                ..
            } => (
                "ColorCycle",
                format!(
                    "colors={:?}, speed={:?}, direction={:?}, transition={:?}",
                    colors, speed, direction, transition
                ),
            ),
            EffectType::Strobe {
                frequency,
                duration,
            } => (
                "Strobe",
                format!("frequency={:?}, duration={:?}", frequency, duration),
            ),
            EffectType::Move {
                to,
                from,
                easing,
                duration,
            } => (
                "Move",
                format!(
                    "to={:?}, from={:?}, easing={:?}, duration={:?}",
                    to, from, easing, duration
                ),
            ),
            EffectType::Dimmer {
                start_level,
                end_level,
                duration,
                curve,
            } => (
                "Dimmer",
                format!(
                    "start_level={:?}, end_level={:?}, duration={:?}, curve={:?}",
                    start_level, end_level, duration, curve
                ),
            ),
            EffectType::Chase {
                pattern,
                speed,
                direction,
                ..
            } => (
                "Chase",
                format!(
                    "pattern={:?}, speed={:?}, direction={:?}",
                    pattern, speed, direction
                ),
            ),
            EffectType::Rainbow {
                speed,
                saturation,
                brightness,
                ..
            } => (
                "Rainbow",
                format!(
                    "speed={:?}, saturation={:?}, brightness={:?}",
                    speed, saturation, brightness
                ),
            ),
            EffectType::Pulse {
                base_level,
                pulse_amplitude,
                frequency,
                duration,
            } => (
                "Pulse",
                format!(
                    "base_level={:?}, pulse_amplitude={:?}, frequency={:?}, duration={:?}",
                    base_level, pulse_amplitude, frequency, duration
                ),
            ),
        }
    }

    /// Register a fixture with the engine
    pub fn register_fixture(&mut self, fixture: FixtureInfo) {
        self.fixtures.register(fixture);
    }

    /// Replaces the registered fixtures with the venue's current set, so a
    /// fixture unhung since the last registration does not linger with its
    /// old patch. Registration runs at every song start and venue reload.
    pub fn replace_fixtures(&mut self, fixtures: impl IntoIterator<Item = FixtureInfo>) {
        self.fixtures = FixtureRegistry::new();
        for fixture in fixtures {
            self.register_fixture(fixture);
        }
        // Pose memory follows the registry: a mover the venue renamed or
        // unhung must not haunt the preview as a held pose.
        self.poses.retain(|name, _| self.fixtures.contains(name));
        self.goals.retain(|name, _| self.fixtures.contains(name));
        self.cache.invalidate();
    }

    /// Look up which fixture and channel a DMX address belongs to (test only).
    #[cfg(test)]
    pub fn lookup_dmx_channel(&self, universe: u16, dmx_channel: u16) -> Option<&(String, String)> {
        self.fixtures.lookup_dmx_channel(universe, dmx_channel)
    }

    /// Set the MIDI DMX store reference for reading interpolated MIDI values.
    pub fn set_midi_dmx_store(&mut self, store: Arc<parking_lot::RwLock<MidiDmxStore>>) {
        self.midi_dmx_store = Some(store);
    }

    /// Binds the venue's focus points, the stage points a `move` may aim at.
    pub fn set_focus_points(&mut self, focus_points: HashMap<String, [f64; 3]>) {
        self.focus_points = focus_points;
    }

    /// A fresh engine for the same rig: its fixtures, tempo map, MIDI DMX
    /// store and focus points, and where each mover is and is headed, with
    /// no effects running and its layers at rest. The next pass of a
    /// crossfading song loop starts in one while this one plays out.
    pub fn sibling(&self) -> EffectEngine {
        EffectEngine {
            fixtures: self.fixtures.clone(),
            tempo_map: self.tempo_map.clone(),
            midi_dmx_store: self.midi_dmx_store.clone(),
            focus_points: self.focus_points.clone(),
            poses: self.poses.clone(),
            goals: self.goals.clone(),
            // The effects loop's stall diagnostics watch this one counter.
            update_subphase: self.update_subphase.clone(),
            wire: self.wire.clone(),
            ..EffectEngine::new()
        }
    }

    /// Takes `other`'s rig -- its fixtures, MIDI DMX store and focus points
    /// -- keeping this engine's effects: how an engine running beside the
    /// main one (ambient light over a song) follows a venue registration.
    pub fn adopt_rig(&mut self, other: &EffectEngine) {
        self.fixtures = other.fixtures.clone();
        self.midi_dmx_store = other.midi_dmx_store.clone();
        self.focus_points = other.focus_points.clone();
        self.poses.retain(|name, _| other.fixtures.contains(name));
        self.goals.retain(|name, _| other.fixtures.contains(name));
        self.cache.invalidate();
    }

    /// The frame `t` of the way (0.0 to 1.0) through a crossfade from
    /// `outgoing` (another engine's frame for the same rig) to `incoming`
    /// (this engine's), channel by channel as a console crosses cues, by
    /// the blend the fixture states cross by ([`CrossKind::cross`]):
    /// intensity, colour and other continuous channels fade, a channel only
    /// one side drives fading from or to zero; pan and tilt fade too, but a
    /// mover only one side drives holds that side's position rather than
    /// swinging home; and discrete channels -- strobe, gobo, prism and the
    /// like, where a value in between is some other setting -- switch at
    /// the midpoint. A 16-bit channel's coarse and fine bytes cross as the
    /// one value they make, so it moves straight from one to the other.
    pub fn crossfade_commands(
        &self,
        outgoing: &[DmxCommand],
        incoming: &[DmxCommand],
        t: f64,
    ) -> Vec<DmxCommand> {
        use std::collections::BTreeMap;
        let t = t.clamp(0.0, 1.0);
        let places = self.cross_places();
        let mut pairs: BTreeMap<(u16, u16), (Option<u8>, Option<u8>)> = BTreeMap::new();
        for c in outgoing {
            pairs.entry((c.universe, c.channel)).or_default().0 = Some(c.value);
        }
        for c in incoming {
            pairs.entry((c.universe, c.channel)).or_default().1 = Some(c.value);
        }
        let byte = |kind: CrossKind, (old, new): (Option<u8>, Option<u8>)| {
            dmx_byte(kind.cross(old.map(f64::from), new.map(f64::from), t))
        };

        let mut out = BTreeMap::new();
        for (&(universe, channel), &values) in &pairs {
            let at = |channel| (universe, channel);
            match places.get(&(universe, channel)).copied() {
                // A 16-bit value: written with its coarse byte, when either
                // side writes its fine one.
                Some(CrossPlace::Coarse(kind, fine)) if pairs.contains_key(&at(fine)) => {
                    let (fine_old, fine_new) = pairs[&at(fine)];
                    let side = |coarse: Option<u8>, fine: Option<u8>| {
                        (coarse.is_some() || fine.is_some()).then(|| {
                            f64::from(u16::from_be_bytes([coarse.unwrap_or(0), fine.unwrap_or(0)]))
                        })
                    };
                    let value =
                        dmx_word(kind.cross(side(values.0, fine_old), side(values.1, fine_new), t));
                    let [high, low] = value.to_be_bytes();
                    out.insert(at(channel), high);
                    out.insert(at(fine), low);
                }
                Some(CrossPlace::Fine(kind, coarse)) => {
                    // Its coarse byte writes it, above; with none on either
                    // side, it crosses as a byte of its own.
                    if !pairs.contains_key(&at(coarse)) {
                        out.insert(at(channel), byte(kind, values));
                    }
                }
                Some(CrossPlace::Coarse(kind, _) | CrossPlace::Byte(kind)) => {
                    out.insert(at(channel), byte(kind, values));
                }
                None => {
                    out.insert(at(channel), byte(CrossKind::Fade, values));
                }
            }
        }
        out.into_iter()
            .map(|((universe, channel), value)| DmxCommand {
                universe,
                channel,
                value,
            })
            .collect()
    }

    /// How each DMX channel (universe, channel) of the patched fixtures
    /// crosses a crossfade: by its channel's kind, as a byte of its own or
    /// as the coarse or fine byte of a 16-bit value -- the channel's own
    /// bytes and each mirror's. Where channels overlap, inside one fixture
    /// or between two, the same whatever order the maps hold them in
    /// ([`CrossPlace`]'s order), and a coarse or fine byte whose partner
    /// another channel took crosses as a byte of its own, so no channel
    /// either side drives is dropped.
    fn cross_places(&self) -> HashMap<(u16, u16), CrossPlace> {
        let mut places: HashMap<(u16, u16), CrossPlace> = HashMap::new();
        // Of two places on one channel -- a fixture's own channels over
        // each other, or another fixture's -- the first by `CrossPlace`'s
        // order, whatever order the maps give them in.
        let mut claim = |address, place: CrossPlace| {
            places
                .entry(address)
                .and_modify(|held| *held = (*held).min(place))
                .or_insert(place);
        };
        for fixture in self.fixtures.as_map().values() {
            let at = |offset: u16| Some((fixture.universe, fixture.dmx_channel(offset)?));
            for (name, &offset) in &fixture.channels {
                if let Some(address) = at(offset) {
                    claim(address, CrossPlace::Byte(CrossKind::of(name)));
                }
            }
            for (name, def) in &fixture.channel_defs {
                let kind = CrossKind::of(name);
                for (coarse, fine) in
                    std::iter::once((def.offset, def.fine)).chain(def.mirrors.iter().copied())
                {
                    let (Some(coarse), fine) = (at(coarse), fine.and_then(at)) else {
                        continue;
                    };
                    match fine {
                        Some(fine) => {
                            claim(coarse, CrossPlace::Coarse(kind, fine.1));
                            claim(fine, CrossPlace::Fine(kind, coarse.1));
                        }
                        None => claim(coarse, CrossPlace::Byte(kind)),
                    }
                }
            }
        }
        // A coarse or fine byte whose partner is not its own.
        let partnered = |(universe, channel): (u16, u16), place: CrossPlace| match place {
            CrossPlace::Coarse(_, fine) => {
                matches!(places.get(&(universe, fine)), Some(CrossPlace::Fine(_, c)) if *c == channel)
            }
            CrossPlace::Fine(_, coarse) => matches!(
                places.get(&(universe, coarse)),
                Some(CrossPlace::Coarse(_, f)) if *f == channel
            ),
            CrossPlace::Byte(_) => true,
        };
        let alone: Vec<_> = places
            .iter()
            .filter(|&(&address, &place)| !partnered(address, place))
            .map(|(&address, place)| (address, CrossPlace::Byte(place.kind())))
            .collect();
        places.extend(alone);
        places
    }

    /// The channels (universe, DMX channel) of the patched fixtures that
    /// carry a position (pan, tilt): a crossfade holds those rather than
    /// fading them, so no fade takes one dark.
    pub fn position_channels(&self) -> HashSet<(u16, u16)> {
        self.channels_where(is_position_channel)
    }

    /// Every DMX channel (universe, channel) a patched fixture drives for a
    /// channel whose name passes `keep`: its coarse byte, its fine byte, and
    /// those of each section mirrored to it, as the frame writes them.
    fn channels_where(&self, keep: impl Fn(&str) -> bool) -> HashSet<(u16, u16)> {
        let mut out = HashSet::new();
        for fixture in self.fixtures.as_map().values() {
            let at = |offset: u16| Some((fixture.universe, fixture.dmx_channel(offset)?));
            for (name, def) in &fixture.channel_defs {
                if !keep(name) {
                    continue;
                }
                out.extend(at(def.offset));
                out.extend(def.fine.and_then(at));
                for &(coarse, fine) in &def.mirrors {
                    out.extend(at(coarse));
                    out.extend(fine.and_then(at));
                }
            }
        }
        out
    }

    /// The channels (universe, DMX channel) of the patched fixtures that
    /// carry a level or colour ([`fades_to_dark`]).
    pub fn level_channels(&self) -> HashSet<(u16, u16)> {
        self.channels_where(fades_to_dark)
    }

    /// The frame this engine last rendered.
    pub fn last_commands(&self) -> &[DmxCommand] {
        self.cache.get_cached()
    }

    /// The bytes the rig holds, as the frames sent through [`Self::send`]
    /// left them.
    pub fn wire(&self) -> &Wire {
        &self.wire
    }

    /// Sets the universes the rig has an output for ([`Wire::set_outputs`]):
    /// the live output's profile's. A fixture patched on any other holds
    /// nothing and reads dark. `None`, as a fresh engine has it, takes
    /// every universe as one: an offline evaluation's rig.
    pub fn set_outputs(&mut self, outputs: Option<HashSet<u16>>) {
        self.wire.set_outputs(outputs);
    }

    /// Sends `frame` -- what goes out to the universes, this engine's own
    /// or one crossed with others -- onto the wire ([`Wire::send`]), the
    /// patched levels and colours the last frame wrote and this one does
    /// not going to 0. Answers those, to send after the frame.
    pub fn send(&mut self, frame: &[DmxCommand]) -> Vec<DmxCommand> {
        let levels = self.level_channels();
        self.wire.send(frame, &levels)
    }

    /// From now on a start from history (see [`Self::start_from`]) sends
    /// the wire the frames playing the history through would have: one
    /// just before each effect in it ends and one at each cue time, so a
    /// gobo or zoom an effect over before the start point left on the rig
    /// is still there. For an offline evaluation, which has no rig of its
    /// own; live, the rig already holds what it was sent.
    ///
    /// The frame before an end is the effect at its end: a setting still
    /// moving then (a gobo or zoom fading down to its end, a head mid-move
    /// at a cut) reads where it got to. Live, the last frame lands up to a
    /// frame period earlier -- about 23 ms on a Pi, longer under load -- so
    /// the rig can hold it one frame short of that.
    pub(crate) fn trace_wire(&mut self) {
        self.tracing_wire = true;
    }

    /// Renders the frame `dt` on, at `song_time` ([`Self::update`]), and
    /// sends it to the wire ([`Self::send`]) as the live output sends an
    /// engine's own frame: for an engine with no output of its own -- an
    /// offline evaluation, a test playing a show through. A frame that
    /// fails to render sends nothing.
    pub(crate) fn render_and_send(&mut self, dt: Duration, song_time: Duration) {
        if self.update(dt, Some(song_time)).is_ok() {
            let frame = self.cache.get_cached().to_vec();
            self.send(&frame);
        }
    }

    /// Renders the frame at `song_time`, sends it to the wire and hands the
    /// engine to `read`, then puts pose memory back as the history being
    /// replayed has it, so the replay carries on as if no frame had been
    /// rendered: a traced frame's and a sampled one's.
    fn send_replayed_frame(&mut self, song_time: Duration, read: impl FnOnce(&EffectEngine)) {
        let (poses, goals) = (self.poses.clone(), self.goals.clone());
        self.cache.invalidate();
        self.render_and_send(Duration::ZERO, song_time);
        read(self);
        self.poses = poses;
        self.goals = goals;
    }

    /// When each effect running now that may leave a setting on the rig
    /// ([`leaves_settings`]) ends, from now on, up to `time`, in order: the
    /// instants a replay to `time` sends a frame just before. One that
    /// writes only levels and colours leaves nothing: they go to 0 as it
    /// ends, whatever its last frame sent.
    fn ends_until(&self, time: Duration) -> Vec<Duration> {
        let now = self.now();
        let from = now.song.unwrap_or_default();
        let mut ends: Vec<Duration> = self
            .active_effects
            .values()
            .filter(|effect| effect.start_time.is_some() && leaves_settings(effect))
            .filter_map(|effect| self.layer_state.clock(effect.layer).end_of(effect, now))
            .filter(|&end| end > from && end <= time)
            .collect();
        ends.sort();
        ends.dedup();
        ends
    }

    /// The pose memory: where each mover was last driven to.
    pub fn poses(&self) -> &HashMap<String, Pose> {
        &self.poses
    }

    /// The fixture states `t` of the way through a crossfade from
    /// `outgoing`'s last frame to this engine's, and the poses likewise: the
    /// states [`Self::crossfade_commands`] makes of the two frames' DMX, by
    /// the same blend ([`CrossKind::cross`]), for what shows the rig rather
    /// than drives it.
    pub fn crossfade_states(
        &self,
        outgoing: &EffectEngine,
        t: f64,
    ) -> (HashMap<String, FixtureState>, HashMap<String, Pose>) {
        self.crossfade_states_onto(outgoing, (&self.last_merged_states, &self.poses), t)
    }

    /// The same, crossing `outgoing` onto `onto` -- states and poses that
    /// another crossfade already blended -- rather than this engine's own.
    pub fn crossfade_states_onto(
        &self,
        outgoing: &EffectEngine,
        onto: (&HashMap<String, FixtureState>, &HashMap<String, Pose>),
        t: f64,
    ) -> (HashMap<String, FixtureState>, HashMap<String, Pose>) {
        let (onto_states, onto_poses) = onto;
        let t = t.clamp(0.0, 1.0);
        let lerp = |a: f64, b: f64| crate::lighting::channels::lerp(a, b, t);
        // Each side as it shows, multipliers applied and cells resolved, as
        // the DMX of each frame is: a multiplier one side has and the other
        // not is neutral there, not a level crossing from zero.
        let (old_resolved, new_resolved) = (
            // What the outgoing engine shows: mid-crossfade itself (a loop
            // break during a pass crossfade, #560), its own blend.
            self.resolved_states(outgoing.shown_fixture_states()),
            self.resolved_states(onto_states),
        );
        let (old_states, new_states) = (&old_resolved, &new_resolved);
        let names: HashSet<&String> = old_states.keys().chain(new_states.keys()).collect();
        let mut states = HashMap::new();
        for name in names {
            let empty = FixtureState::new();
            let old = old_states.get(name).unwrap_or(&empty);
            let new = new_states.get(name).unwrap_or(&empty);
            let mut state = FixtureState::new();
            let channels: HashSet<&String> =
                old.channels.keys().chain(new.channels.keys()).collect();
            for channel in channels {
                let (o, n) = (old.channels.get(channel), new.channels.get(channel));
                let blended =
                    CrossKind::of(channel).cross(o.map(|c| c.value), n.map(|c| c.value), t);
                let like = n.or(o).expect("a channel one side has");
                state.channels.insert(
                    channel.clone(),
                    ChannelState::new(blended, like.layer, like.blend_mode),
                );
            }
            let intent = |o: Option<Intent>, n: Option<Intent>| match (o, n) {
                (Some(o), Some(n)) => Some(Intent {
                    degrees: lerp(o.degrees, n.degrees),
                    layer: n.layer,
                }),
                (o, n) => n.or(o),
            };
            state.physical.pan = intent(old.physical.pan, new.physical.pan);
            state.physical.tilt = intent(old.physical.tilt, new.physical.tilt);
            // A software strobe's rate snaps over, as its bytes do.
            state.strobe_hz = if t < 0.5 {
                old.strobe_hz
            } else {
                new.strobe_hz
            };
            states.insert(name.clone(), state);
        }
        let mut poses = outgoing.shown_poses().clone();
        for (name, pose) in onto_poses {
            let blended = match outgoing.shown_poses().get(name) {
                Some(old) => Pose {
                    pan: lerp(old.pan, pose.pan),
                    tilt: lerp(old.tilt, pose.tilt),
                },
                None => *pose,
            };
            poses.insert(name.clone(), blended);
        }
        (states, poses)
    }

    /// `states` as the rig shows them: each channel at its effective value
    /// (layer multipliers applied, the multiplier channels gone), and every
    /// cell of a pixel fixture resolved through its fixture as the DMX
    /// path writes it. Every value is a plain replace, so the snapshot
    /// built from it resolves it to itself.
    fn resolved_states(
        &self,
        states: &HashMap<String, FixtureState>,
    ) -> HashMap<String, FixtureState> {
        let plain =
            |value: f64| ChannelState::new(value, EffectLayer::Background, BlendMode::Replace);
        let cell_state = |values: Vec<(String, f64)>| {
            let mut state = FixtureState::new();
            for (channel, value) in values {
                state.channels.insert(channel, plain(value));
            }
            state
        };
        let mut out = HashMap::new();
        for (name, state) in states {
            let info = self.fixtures.get(name);
            if let Some(parent) = info.and_then(|i| i.parent.as_ref()) {
                // A cell comes with its fixture, below; on its own only when
                // its fixture has no state of its own.
                if states.contains_key(parent) {
                    continue;
                }
                let cell = self
                    .fixtures
                    .get(parent)
                    .and_then(|p| p.cells.iter().find(|c| c.name == cell_name(name)));
                if let Some(cell) = cell {
                    out.insert(
                        name.clone(),
                        cell_state(FixtureState::new().cell_values(cell, Some(state))),
                    );
                }
                continue;
            }
            let has_dimmer = info.is_some_and(FixtureInfo::has_dimmer);
            let mut resolved = FixtureState::new();
            resolved.physical = state.physical.clone();
            resolved.strobe_hz = state.strobe_hz;
            for (channel, channel_state) in &state.channels {
                if is_multiplier_channel(channel) {
                    continue;
                }
                let value = state.effective_channel_value(channel, channel_state, has_dimmer);
                resolved.channels.insert(channel.clone(), plain(value));
            }
            out.insert(name.clone(), resolved);
            for cell in info.map(|i| i.cells.as_slice()).unwrap_or_default() {
                let sub = cell_fixture_name(name, &cell.name);
                let own = states.get(&sub);
                out.insert(sub, cell_state(state.cell_values(cell, own)));
            }
        }
        out
    }

    /// Sets what the rig shows while another engine's frame is crossed into
    /// this one's (see [`Self::crossfade_states`]), until the next
    /// [`Self::update`].
    pub fn set_shown(
        &mut self,
        shown: Option<(HashMap<String, FixtureState>, HashMap<String, Pose>)>,
    ) {
        self.shown = shown;
    }

    /// The fixture states the rig shows: this engine's, or mid-crossfade the
    /// blend going out (#555).
    pub fn shown_fixture_states(&self) -> &HashMap<String, FixtureState> {
        self.shown
            .as_ref()
            .map_or(&self.last_merged_states, |(states, _)| states)
    }

    /// The poses the rig shows, likewise.
    pub fn shown_poses(&self) -> &HashMap<String, Pose> {
        self.shown.as_ref().map_or(&self.poses, |(_, poses)| poses)
    }

    /// A `move` starts where the fixture is now unless `from` says
    /// otherwise: capture that from pose memory before the effect's first
    /// tick, for every move, since a `from` naming an unbound focus point
    /// falls back to it too. A fixture with no memory yet starts at its
    /// target — the show's first cue is a snap, not a sweep from nowhere.
    fn capture_start_poses(&mut self, effect: &mut EffectInstance) {
        let EffectType::Move { .. } = &effect.effect_type else {
            return;
        };
        if let Some(reads) = self.pose_reads.as_mut().filter(|r| !r.released) {
            for name in &effect.target_fixtures {
                if !reads.written.contains(name) {
                    reads.read.insert(name.clone());
                }
            }
        }
        effect.start_poses = effect
            .target_fixtures
            .iter()
            .filter_map(|name| self.poses.get(name).map(|pose| (name.clone(), *pose)))
            .collect();
    }

    /// Start an effect
    /// `per: cell`: each target with cells becomes its cells, in cell
    /// order (spatial order is the effect's to apply); a target without
    /// cells stays itself, so a mixed group still works.
    fn expand_cells(&self, effect: &mut EffectInstance) {
        // A cell has no pan or tilt: a move stays on the fixtures (the
        // lint says `per: cell` does nothing there) rather than failing
        // validation for the whole cue.
        if !effect.per_cell || matches!(effect.effect_type, EffectType::Move { .. }) {
            return;
        }
        let mut targets = Vec::with_capacity(effect.target_fixtures.len());
        for name in &effect.target_fixtures {
            let cells = self.fixtures.cells_of(name);
            if cells.is_empty() {
                targets.push(name.clone());
            } else {
                targets.extend(cells.iter().cloned());
            }
        }
        effect.target_fixtures = targets;
    }

    /// A move takes its heads from any move already on them, on any layer:
    /// the latest move wins (#599). The head carries on from where it is,
    /// since the new move starts from pose memory; the move it took them
    /// from runs on its other fixtures, and ends if it has none left.
    fn take_heads(&mut self, effect: &EffectInstance) {
        if !matches!(effect.effect_type, EffectType::Move { .. }) {
            return;
        }
        let mut took = false;
        let at = self.last_song_time;
        let ends = &mut self.ends;
        self.active_effects.retain(|id, other| {
            if *id == effect.id || !matches!(other.effect_type, EffectType::Move { .. }) {
                return true;
            }
            let before = other.target_fixtures.len();
            other
                .target_fixtures
                .retain(|name| !effect.target_fixtures.contains(name));
            took |= other.target_fixtures.len() != before;
            let left = !other.target_fixtures.is_empty();
            if !left {
                if let Some(ends) = ends.as_mut() {
                    ends.insert(id.clone(), at);
                }
            }
            left
        });
        if took {
            self.cache.invalidate();
        }
    }

    /// Takes the effects `out` picks out of the running set: a clear, a
    /// stopped sequence. Where a show's ends are reckoned, each ends now.
    fn take_out(&mut self, mut out: impl FnMut(&EffectInstance) -> bool) {
        let at = self.last_song_time;
        let ends = &mut self.ends;
        self.active_effects.retain(|id, effect| {
            if !out(effect) {
                return true;
            }
            if let Some(ends) = ends.as_mut() {
                ends.insert(id.clone(), at);
            }
            false
        });
    }

    pub fn start_effect(&mut self, mut effect: EffectInstance) -> Result<(), EffectError> {
        self.expand_cells(&mut effect);
        // Validate effect: against the rig, where there is one.
        if !self.rigless {
            validation::validate_effect(self.fixtures.as_map(), &effect)?;
        }
        self.capture_start_poses(&mut effect);
        self.take_heads(&effect);

        // Log effect parameters
        if tracing::enabled!(tracing::Level::DEBUG) {
            let (effect_kind, effect_params) = Self::format_effect_for_logging(&effect);
            debug!(
                effect_id = %effect.id,
                effect_kind,
                effect_params = %effect_params,
                layer = ?effect.layer,
                blend_mode = ?effect.blend_mode,
                priority = effect.priority,
                up_time = ?effect.up_time,
                hold_time = ?effect.hold_time,
                down_time = ?effect.down_time,
                targets = ?effect.target_fixtures,
                "Starting lighting effect"
            );
        }

        // Set the start time to the current engine time
        effect.start_time = Some(self.current_time);
        effect.start_order = next_start_order();
        self.active_effects.insert(effect.id.clone(), effect);
        self.cache.invalidate();
        Ok(())
    }

    /// Whether every mover has reached its goal. While one is still
    /// travelling under its slew limit, the frame must be computed even
    /// with no effect active, or the fast path would freeze it mid-sweep.
    fn movers_settled(&self) -> bool {
        self.goals.iter().all(|(name, goal)| {
            self.poses.get(name).is_some_and(|pose| {
                (pose.pan - goal.pan).abs() < 1e-9 && (pose.tilt - goal.tilt).abs() < 1e-9
            })
        })
    }

    /// Applies pose memory and the slew clamp to this frame's states.
    fn settle_poses(&mut self, states: &mut HashMap<String, FixtureState>) {
        use crate::lighting::effects::{Intent, PhysicalParameter};
        let dt = self.last_dt.as_secs_f64();
        let mut next_poses = self.poses.clone();
        // Held poses: every remembered mover nobody drove this frame keeps
        // heading for (or sitting at) its last goal.
        for (name, pose) in &self.goals {
            if !self.fixtures.contains(name) {
                continue;
            }
            let state = states.entry(name.clone()).or_default();
            if state.physical.pan.is_none() {
                state.physical.set(
                    PhysicalParameter::Pan,
                    Intent {
                        degrees: pose.pan,
                        layer: EffectLayer::Background,
                    },
                );
            }
            if state.physical.tilt.is_none() {
                state.physical.set(
                    PhysicalParameter::Tilt,
                    Intent {
                        degrees: pose.tilt,
                        layer: EffectLayer::Background,
                    },
                );
            }
        }
        for (name, state) in states.iter_mut() {
            if state.physical.is_empty() {
                continue;
            }
            let previous = self.poses.get(name).copied();
            let asked_pan = state.physical.pan.map(|i| i.degrees);
            let asked_tilt = state.physical.tilt.map(|i| i.degrees);
            let limits = self.fixtures.get(name).map(|f| f.movement);
            let clamp = |asked: f64, was: Option<f64>, max_speed: Option<f64>| -> f64 {
                match (was, max_speed) {
                    (Some(was), Some(max_speed)) if max_speed > 0.0 && dt > 0.0 => {
                        let step = max_speed * dt;
                        was + (asked - was).clamp(-step, step)
                    }
                    _ => asked,
                }
            };
            let pan = state.physical.pan.map(|intent| {
                clamp(
                    intent.degrees,
                    previous.map(|p| p.pan),
                    limits.and_then(|l| l.max_pan_speed),
                )
            });
            let tilt = state.physical.tilt.map(|intent| {
                clamp(
                    intent.degrees,
                    previous.map(|p| p.tilt),
                    limits.and_then(|l| l.max_tilt_speed),
                )
            });
            if let (Some(pan), Some(intent)) = (pan, state.physical.pan.as_mut()) {
                intent.degrees = pan;
            }
            if let (Some(tilt), Some(intent)) = (tilt, state.physical.tilt.as_mut()) {
                intent.degrees = tilt;
            }
            next_poses.insert(
                name.clone(),
                Pose {
                    pan: pan.or(previous.map(|p| p.pan)).unwrap_or(0.0),
                    tilt: tilt.or(previous.map(|p| p.tilt)).unwrap_or(0.0),
                },
            );
            let goal = self.goals.get(name).copied();
            self.goals.insert(
                name.clone(),
                Pose {
                    pan: asked_pan.or(goal.map(|g| g.pan)).unwrap_or(0.0),
                    tilt: asked_tilt.or(goal.map(|g| g.tilt)).unwrap_or(0.0),
                },
            );
        }
        self.poses = next_poses;
    }

    /// Update the engine and return DMX commands
    /// song_time is the current song time (score-time) for tempo-aware effect completion checks
    pub fn update(
        &mut self,
        dt: Duration,
        song_time: Option<Duration>,
    ) -> Result<&[DmxCommand], EffectError> {
        self.update_subphase.store(10, Ordering::Relaxed);
        // What the rig shows is this frame from here, unless the caller
        // crossing it with another engine's sets the blend after.
        self.shown = None;
        self.current_time += dt;
        self.engine_elapsed += dt;
        self.last_song_time = song_time;
        self.last_dt = dt;

        // Fast path for MIDI-DMX-only frames: when no DSL effects are running,
        // generate DmxCommands directly from the store. This skips all HashMap
        // cloning, fixture state rebuilding, and the full pipeline.
        if !self.cache.dirty && self.active_effects.is_empty() && self.movers_settled() {
            let store_gen = self
                .midi_dmx_store
                .as_ref()
                .map(|s| s.read().generation())
                .unwrap_or(0);

            // Unchanged since last frame — return cached commands.
            if store_gen == self.cache.last_store_generation {
                self.update_subphase.store(0, Ordering::Relaxed);
                return Ok(self.cache.get_cached());
            }

            // Store changed — rebuild commands directly from store (no intermediate
            // active effects pipeline).
            let mut commands = Vec::new();
            self.last_merged_states.clear();

            if let Some(ref store) = self.midi_dmx_store {
                let store = store.read();
                for (slot_idx, normalized_value) in store.iter_active() {
                    let (fixture_name, channel_name) = store.fixture_info(slot_idx);
                    if let Some(fixture_info) = self.fixtures.get(fixture_name) {
                        if let Some(def) = fixture_info.channel_defs.get(channel_name) {
                            for (offset, byte) in resolve_normalized(def, normalized_value) {
                                if let Some(channel) = fixture_info.dmx_channel(offset) {
                                    commands.push(DmxCommand {
                                        universe: fixture_info.universe,
                                        channel,
                                        value: byte,
                                    });
                                }
                            }
                            // Update merged state for simulator visibility.
                            self.last_merged_states
                                .entry(fixture_name.clone())
                                .or_default()
                                .set_channel(
                                    channel_name.clone(),
                                    ChannelState::new(
                                        normalized_value,
                                        EffectLayer::Background,
                                        BlendMode::Replace,
                                    ),
                                );
                        }
                    }
                }
            }

            // A settled mover still holds its pose: the states the live
            // views read must say so even on this path, or a head that has
            // arrived drops out of `get_fixture_state` the moment an
            // unrelated MIDI fader moves.
            let mut held = HashMap::new();
            self.settle_poses(&mut held);
            for (name, state) in held {
                self.last_merged_states.entry(name).or_default().physical = state.physical;
            }

            self.cache.update(commands, store_gen);
            self.update_subphase.store(0, Ordering::Relaxed);
            return Ok(self.cache.get_cached());
        }

        self.update_subphase.store(20, Ordering::Relaxed);

        // Use song_time for tempo-aware speed lookups (BPM at current position).
        // Falls back to engine_elapsed when no song is playing.
        let absolute_time = song_time.unwrap_or(self.engine_elapsed);

        // Start with empty fixture states — no persistent state carried between frames
        let mut current_fixture_states = HashMap::new();

        self.update_subphase.store(30, Ordering::Relaxed);

        // Inject MIDI DMX values from the lockless store (lowest priority).
        if let Some(ref store) = self.midi_dmx_store {
            let store = store.read();
            for (slot_idx, normalized_value) in store.iter_active() {
                let (fixture_name, channel_name) = store.fixture_info(slot_idx);
                if self.fixtures.contains(fixture_name) {
                    let state = current_fixture_states
                        .entry(fixture_name.clone())
                        .or_insert_with(FixtureState::new);
                    if !state.channels.contains_key(channel_name) {
                        state.set_channel(
                            channel_name.clone(),
                            ChannelState::new(
                                normalized_value,
                                EffectLayer::Background,
                                BlendMode::Replace,
                            ),
                        );
                    }
                }
            }
        }

        self.update_subphase.store(40, Ordering::Relaxed);

        // Group effects by layer - collect effect IDs first to avoid borrowing conflicts
        // Within each layer, we will sort effects deterministically so that:
        // - Higher priority effects are processed after lower priority ones
        // - For equal priority, later-started effects are processed after earlier ones
        // This ensures consistent layering behavior between runs and avoids
        // HashMap iteration order affecting visual output.
        let mut effects_by_layer: std::collections::BTreeMap<EffectLayer, Vec<String>> =
            std::collections::BTreeMap::new();

        for (effect_id, effect) in &self.active_effects {
            if effect.enabled {
                effects_by_layer
                    .entry(effect.layer)
                    .or_default()
                    .push(effect_id.clone());
            }
        }

        // Sort effect IDs within each layer by (priority, start_time,
        // cue_time, start_order). Using cue_time ensures deterministic
        // ordering when multiple effects start at the same time; the start
        // order then puts the one started later on top. Not the id: compared
        // as strings, `song_effect_10000` sorted before `song_effect_9999`,
        // so same-time cues swapped when the id counter crossed a power of
        // ten (#669).
        for (_layer, effect_ids) in effects_by_layer.iter_mut() {
            effect_ids.sort_by(|a, b| {
                let ea = self.active_effects.get(a).unwrap();
                let eb = self.active_effects.get(b).unwrap();

                ea.priority
                    .cmp(&eb.priority)
                    .then_with(|| {
                        // Effects without a start_time are treated as earliest
                        let sa = ea.start_time;
                        let sb = eb.start_time;
                        match (sa, sb) {
                            (Some(ta), Some(tb)) => ta.cmp(&tb),
                            (None, Some(_)) => std::cmp::Ordering::Less,
                            (Some(_), None) => std::cmp::Ordering::Greater,
                            (None, None) => std::cmp::Ordering::Equal,
                        }
                    })
                    .then_with(|| {
                        // Use cue_time for deterministic ordering when effects have same priority and start_time
                        // This ensures effects at the same time are always processed in the same order
                        match (ea.cue_time, eb.cue_time) {
                            (Some(ca), Some(cb)) => ca.cmp(&cb),
                            (None, Some(_)) => std::cmp::Ordering::Less,
                            (Some(_), None) => std::cmp::Ordering::Greater,
                            (None, None) => std::cmp::Ordering::Equal,
                        }
                    })
                    .then_with(|| ea.start_order.cmp(&eb.start_order))
            });
        }

        self.update_subphase.store(50, Ordering::Relaxed);

        // Track effects that have just completed to preserve their final state
        let mut completed_effects = Vec::new();

        // Process each layer in order
        for (layer, effect_ids) in effects_by_layer {
            // Get layer masters
            let layer_intensity = self.get_layer_intensity_master(layer);

            for effect_id in effect_ids {
                // Get reference to effect to avoid unnecessary clone
                let effect = self.active_effects.get(&effect_id).unwrap();

                let elapsed = self.elapsed_at(effect, song_time);

                // Expire on the time the effect renders on: score time, less
                // its time frozen, for a cued effect at normal speed, so it
                // ends exactly on its musical end (#536); a frozen or slowed
                // one on its own elapsed, as song time runs on through a
                // freeze and ignores the speed master (#586).
                let is_expired =
                    effect.start_time.is_some() && effect.has_reached_terminal_state(elapsed);

                if is_expired {
                    // Effect has completed — no state persists, so don't blend a final
                    // frame. Queue for removal after this frame.
                    completed_effects.push(effect_id.clone());
                    continue;
                }

                // Process the effect and get fixture states
                if let Some(mut effect_states) = processing::process_effect(
                    self.fixtures.as_map(),
                    &self.focus_points,
                    effect,
                    elapsed,
                    absolute_time,
                    self.tempo_map.as_ref(),
                )? {
                    // Apply the layer intensity master to effect states if not 1.0
                    if (layer_intensity - 1.0).abs() > f64::EPSILON {
                        for fixture_state in effect_states.values_mut() {
                            for channel_state in fixture_state.channels.values_mut() {
                                channel_state.value *= layer_intensity;
                            }
                        }
                    }

                    // Blend the effect states into the current fixture states
                    for (fixture_name, effect_state) in effect_states {
                        if self.fixtures.contains(&fixture_name) {
                            current_fixture_states
                                .entry(fixture_name.clone())
                                .or_insert_with(FixtureState::new)
                                .blend_with(&effect_state);
                        }
                    }
                }
            }
        }

        self.update_subphase.store(60, Ordering::Relaxed);

        // Handle completed effects — simply remove them. No state persists after completion.
        for effect_id in completed_effects {
            if let Some(effect) = self.active_effects.remove(&effect_id) {
                // Except a move's destination: the frame it completes on is
                // not processed, so the pose it was travelling to is written
                // now, and pose memory holds it from here (design §15.4).
                for (name, pose) in processing::move_final_poses(
                    self.fixtures.as_map(),
                    &self.focus_points,
                    &effect,
                ) {
                    let state = current_fixture_states.entry(name).or_default();
                    state.physical.set(
                        PhysicalParameter::Pan,
                        Intent {
                            degrees: pose.pan,
                            layer: effect.layer,
                        },
                    );
                    state.physical.set(
                        PhysicalParameter::Tilt,
                        Intent {
                            degrees: pose.tilt,
                            layer: effect.layer,
                        },
                    );
                }
                // Clean up per-layer multipliers for completed effects
                let multiplier_keys: Vec<String> = MULTIPLIER_PREFIXES
                    .iter()
                    .map(|prefix| multiplier_key(prefix, effect.layer))
                    .collect();

                for fixture_name in &effect.target_fixtures {
                    if let Some(current_state) = current_fixture_states.get_mut(fixture_name) {
                        for key in &multiplier_keys {
                            current_state.channels.remove(key);
                        }
                    }
                }
            }
        }

        self.update_subphase.store(70, Ordering::Relaxed);

        // Movers: a fixture no effect is driving this frame holds the pose
        // it was last driven to; one that is being driven is slewed from
        // where it was toward where it is asked to be, no faster than its
        // declared limit; and the pose memory records where it ends up.
        self.settle_poses(&mut current_fixture_states);

        self.release_undriven_strobes(&mut current_fixture_states);

        self.update_subphase.store(80, Ordering::Relaxed);

        // Store states for preview/debugging (before converting to DMX)
        self.last_merged_states = current_fixture_states.clone();

        // Convert fixture states to DMX commands. A sub-fixture (a cell)
        // writes nothing of its own: its parent writes every cell, merging
        // the cell's state over its own (design §17.3). A parent no effect
        // touched this frame still writes its cells when one of them was.
        let mut commands = Vec::new();
        let mut parents_with_cell_states: Vec<String> = Vec::new();
        for name in current_fixture_states.keys() {
            if let Some(parent) = self.fixtures.get(name).and_then(|f| f.parent.clone()) {
                if !current_fixture_states.contains_key(&parent)
                    && !parents_with_cell_states.contains(&parent)
                {
                    parents_with_cell_states.push(parent);
                }
            }
        }
        for parent in parents_with_cell_states {
            current_fixture_states.insert(parent, FixtureState::new());
        }
        for (fixture_name, fixture_state) in &current_fixture_states {
            let Some(fixture_info) = self.fixtures.get(fixture_name) else {
                continue;
            };
            if fixture_info.parent.is_some() {
                continue;
            }
            if fixture_info.cells.is_empty() {
                commands.extend(fixture_state.to_dmx_commands(fixture_info));
            } else {
                commands.extend(
                    fixture_state.to_dmx_commands_with_cells(fixture_info, |cell| {
                        current_fixture_states.get(&cell_fixture_name(fixture_name, cell))
                    }),
                );
            }
        }

        // Cache commands and store generation for fast-path short-circuit on
        // subsequent frames where nothing changes.
        let gen = self.store_generation();
        self.cache.update(commands, gen);

        self.update_subphase.store(0, Ordering::Relaxed);
        Ok(self.cache.get_cached())
    }

    /// Sends strobe off to every fixture whose strobe channel was driven
    /// last frame and is not driven this one — an effect that expired, a
    /// clear, a stopped sequence, a new song. Without it the DMX universe
    /// holds the last strobe byte and the fixture strobes on in hardware
    /// under whatever comes next.
    fn release_undriven_strobes(&mut self, states: &mut HashMap<String, FixtureState>) {
        let driven: HashSet<String> = states
            .iter()
            .filter(|(_, state)| state.channels.contains_key("strobe"))
            .map(|(name, _)| name.clone())
            .collect();
        for name in self.strobe_driven.difference(&driven) {
            if !self.fixtures.contains(name) {
                continue;
            }
            states.entry(name.clone()).or_default().set_channel(
                "strobe".to_string(),
                ChannelState::new(0.0, EffectLayer::Background, BlendMode::Replace),
            );
        }
        self.strobe_driven = driven;
    }

    /// Stop all active effects and reset per-song layer state
    pub fn stop_all_effects(&mut self) {
        self.stop_show_effects();
        // Clear MIDI DMX values so they don't bleed into the next song.
        // Uses a read lock because MidiDmxStore uses interior mutability
        // (atomics) — no write lock needed.
        if let Some(ref store) = self.midi_dmx_store {
            store.read().clear();
        }
    }

    /// [`Self::stop_all_effects`] but for the MIDI DMX store, which every
    /// engine shares: for one engine's show starting over (ambient's, at
    /// its loop) while the others -- and MIDI -- go on.
    pub fn stop_show_effects(&mut self) {
        // The rig holds its last frame until the next one, so the view does
        // too rather than going black: what it already shows (a loop
        // break's fade starts the next song's show under it), else this
        // engine's last frame, until the next frame's `update`.
        if self.shown.is_none() {
            self.shown = Some((self.last_merged_states.clone(), self.poses.clone()));
        }
        // A clear releases pose memory: the next move starts from its target.
        self.poses.clear();
        self.goals.clear();
        self.active_effects.clear();
        self.last_merged_states.clear();
        // Layer masters and freezes belong to the song that set them. The engine is
        // reused across songs, so without this a show stopped mid-duck leaves the next
        // song mastered down while its own source looks perfectly correct. Both callers
        // replay the new song's layer commands from the timeline afterwards, so a seek
        // rebuilds masters from show history rather than inheriting them.
        self.layer_state.reset();
        self.cache.invalidate();
    }

    /// Stop all effects from a specific sequence: those cued through it by
    /// name, inside another sequence or not.
    pub fn stop_sequence(&mut self, sequence_name: &str) {
        self.take_out(|effect| effect.is_from_sequence(sequence_name));
        self.cache.invalidate();
    }

    // ===== Layer Control Methods (grandMA-inspired) =====

    /// The engine's now, on both its time bases: what a layer command
    /// applies at.
    fn now(&self) -> Now {
        Now {
            instant: self.current_time,
            song: self.last_song_time,
        }
    }

    /// Clear a layer - immediately stops all effects on the specified layer
    /// This is equivalent to a "kill" or panic button for a layer
    pub fn clear_layer(&mut self, layer: EffectLayer) {
        self.take_out(|effect| effect.layer == layer);
        // Including the layer's masters — killing a layer that stays mastered down
        // silently dims whatever is started on it next.
        self.layer_state.reset_layer(layer);
        self.last_merged_states.clear();
        self.cache.invalidate();
    }

    /// Clear all layers - immediately stops all effects on all layers
    /// This is equivalent to a "kill all" or panic button for everything
    pub fn clear_all_layers(&mut self) {
        // A clear releases pose memory: the next move starts from its target.
        self.poses.clear();
        self.goals.clear();
        if let Some(reads) = self.pose_reads.as_mut() {
            reads.released = true;
        }
        self.take_out(|_| true);
        // Including the layer masters — a panic button that leaves the rig mastered
        // down is not a panic button.
        self.layer_state.reset();
        self.last_merged_states.clear();
        self.cache.invalidate();
    }

    /// Freeze a layer - pauses all effects on the layer at their current state
    /// Effects maintain their current output values but don't advance in time
    pub fn freeze_layer(&mut self, layer: EffectLayer) {
        let now = self.now();
        self.layer_state.clock_mut(layer).freeze(now);
        self.cache.invalidate();
    }

    /// How far into itself `effect` is at `song_time`: what it renders and
    /// expires on (see `LayerClock::elapsed`). A cued effect renders on
    /// score time, the elapsed it expires on, so the two agree: engine time
    /// runs up to a frame ahead of it (the effect is stamped before an
    /// update adds `dt`, and its cue lands part way through a tick), and an
    /// effect rendered past its end on engine time before it expired on
    /// score time went dark for a frame at a hand-over (#551). Engine time,
    /// which steps a fixed frame a tick, falls behind the song where frames
    /// run long, and an effect timed on it after an unfreeze ended late.
    pub fn elapsed_at(&self, effect: &EffectInstance, song_time: Option<Duration>) -> Duration {
        self.layer_state.clock(effect.layer).elapsed(
            effect,
            Now {
                instant: self.current_time,
                song: song_time,
            },
        )
    }

    /// Sets the song time a layer command applied before the next frame
    /// takes as now.
    pub fn set_song_time(&mut self, song_time: Duration) {
        self.last_song_time = Some(song_time);
    }

    /// Unfreeze a layer - resumes effects on the layer from where they left off
    pub fn unfreeze_layer(&mut self, layer: EffectLayer) {
        let now = self.now();
        self.layer_state
            .clock_mut(layer)
            .unfreeze(layer, &mut self.active_effects, now);
        self.cache.invalidate();
    }

    /// Check if a layer is frozen
    #[cfg(test)]
    pub fn is_layer_frozen(&self, layer: EffectLayer) -> bool {
        self.layer_state.clock(layer).is_frozen()
    }

    // ===== Layer Master Methods =====

    /// Set the intensity master for a layer (0.0 to 1.0)
    /// This multiplies with all effect outputs on the layer
    pub fn set_layer_intensity_master(&mut self, layer: EffectLayer, intensity: f64) {
        self.layer_state.set_intensity_master(layer, intensity);
        self.cache.invalidate();
    }

    /// Get the intensity master for a layer (defaults to 1.0)
    pub fn get_layer_intensity_master(&self, layer: EffectLayer) -> f64 {
        self.layer_state.intensity_master(layer)
    }

    /// Set the speed master for a layer (0.0+ where 1.0 = normal speed)
    /// This multiplies with effect speeds on the layer
    /// 0.5 = half speed, 2.0 = double speed, 0.0 = frozen at current state
    pub fn set_layer_speed_master(&mut self, layer: EffectLayer, speed: f64) {
        let now = self.now();
        self.layer_state
            .clock_mut(layer)
            .set_speed(layer, &mut self.active_effects, speed, now);
        self.cache.invalidate();
    }

    /// Get the speed master for a layer (defaults to 1.0)
    pub fn get_layer_speed_master(&self, layer: EffectLayer) -> f64 {
        self.layer_state.clock(layer).speed()
    }

    /// Applies what a timeline fired as the song plays, its groups already
    /// resolved: step by step in the order the timeline gives them (see
    /// `TimelineUpdate`), each layer command at its cue's song time -- a
    /// freeze holds from its cue, and an unfreeze resumes from its, rather
    /// than from the frame they land in. Returns the errors of the effects
    /// that would not start.
    pub fn apply_timeline_update(
        &mut self,
        update: crate::lighting::timeline::TimelineUpdate,
    ) -> Vec<EffectError> {
        let frame = self.last_song_time;
        let mut errors = Vec::new();
        for (time, step) in update.steps {
            let command = matches!(step, Step::Command(_));
            if command {
                self.last_song_time = Some(time);
            }
            if let Err(e) = self.apply_step(step) {
                errors.push(e);
            }
            if command {
                self.last_song_time = frame;
            }
        }
        errors
    }

    /// Starts the show part way, from `history` (a seek's, from
    /// `LightingTimeline::start_at`, its groups resolved): stops what
    /// `scope` reaches, then replays the history in this engine by the live
    /// rules, cue time by cue time with no frame rendered (see `Replay`), so
    /// the state at the start point is what playing through would have left
    /// -- and holds the song time there for the cues from it on. With
    /// `StopScope::Nothing` the history replays over what this engine has
    /// (a fresh engine's rest, or in a test a layer already slowed). Returns
    /// the errors of the effects that would not start.
    ///
    /// For a caller that holds this engine under a lock the effects loop
    /// also takes, the same replay in three steps: [`Self::scratch`] (cheap:
    /// the rig) and [`Self::install`] (as many steps as effects running)
    /// under the lock, and [`Self::prepare`], the replay itself, with it
    /// released.
    pub fn start_from(
        &mut self,
        history: crate::lighting::timeline::SeekHistory,
        scope: StopScope,
    ) -> Vec<EffectError> {
        self.stop_by_scope(scope);
        self.replay(history)
    }

    /// Stops what `scope` says a replay starts over.
    fn stop_by_scope(&mut self, scope: StopScope) {
        match scope {
            StopScope::Nothing => {}
            StopScope::Show => self.stop_show_effects(),
            StopScope::All => self.stop_all_effects(),
        }
    }

    /// Replays `history` -- every cue up to and at its start point, its
    /// groups resolved -- in this engine, as [`Self::start_from`] with
    /// `StopScope::Nothing` does, and at each of `times` (ascending, none
    /// past the start point), once every cue up to and at it is in, renders
    /// the frame there and hands the engine to `sample`: one replay read at
    /// many instants, each read as a start there reads it. For an offline
    /// evaluation, with the wire traced ([`Self::trace_wire`]). Returns the
    /// errors of the effects that would not start.
    pub(crate) fn replay_sampled(
        &mut self,
        history: crate::lighting::timeline::SeekHistory,
        times: &[Duration],
        mut sample: impl FnMut(&EffectEngine, Duration),
    ) -> Vec<EffectError> {
        let mut replay = Replay::new(self, history.at);
        replay.play(history.update.steps, times, &mut sample);
        replay.errors
    }

    /// Replays `history` in this engine, up to its start point.
    fn replay(&mut self, history: crate::lighting::timeline::SeekHistory) -> Vec<EffectError> {
        let mut replay = Replay::new(self, history.at);
        replay.play(history.update.steps, &[], &mut |_, _| {});
        replay.finish()
    }

    /// The engine a split start from history (see [`Self::start_from`])
    /// replays in, apart from this one, for [`Self::install`] with the same
    /// `scope`. It has this engine's rig; the history replays from rest --
    /// no effects running, layers as nothing set them -- whatever this
    /// engine runs now, since what it runs by the install is not known yet.
    /// With `StopScope::Show` or `StopScope::All` that is what the stop
    /// leaves. With `StopScope::Nothing` -- an engine fresh for the start, a
    /// pass crossfade's -- it starts from this engine's pose memory, and
    /// the install puts the history's state in beside whatever the engine it
    /// goes in has started since and takes pose memory as of then where the
    /// history did not set it.
    pub fn scratch(&self, scope: StopScope) -> EffectEngine {
        let mut scratch = EffectEngine {
            current_time: self.current_time,
            rigless: self.rigless,
            scope,
            // A replay sends nothing to this engine's rig: the wire stays
            // with the engine the history is installed in.
            wire: Wire::default(),
            ..self.sibling()
        };
        match scope {
            StopScope::Nothing => {
                scratch.pose_reads = Some(PoseReads {
                    initial: self.poses.clone(),
                    ..Default::default()
                });
            }
            // A stop releases pose memory.
            StopScope::Show | StopScope::All => {
                scratch.poses.clear();
                scratch.goals.clear();
            }
        }
        scratch
    }

    /// Replays `history` in this engine, a [`Self::scratch`]: the state it
    /// leaves, for [`Self::install`] to put in the engine it came from.
    pub fn prepare(mut self, history: crate::lighting::timeline::SeekHistory) -> Prepared {
        // Kept to replay again at the install should the pose memory the
        // replay started from have moved on by then (see `PoseReads`).
        let again = self.pose_reads.is_some().then(|| history.clone());
        let at = history.at;
        let errors = self.replay(history);
        Prepared {
            at,
            scope: self.scope,
            instant: self.current_time,
            effects: self.active_effects,
            layer_state: self.layer_state,
            poses: self.poses,
            goals: self.goals,
            pose_reads: self.pose_reads,
            again,
            errors,
        }
    }

    /// Stops what the scratch's scope reaches and puts in what
    /// [`Self::prepare`] left: its effects, layers and pose memory, their
    /// instants moved on by the time this engine has run since the scratch
    /// was taken, and the song time at the start point. With
    /// `StopScope::Nothing` (see [`Self::scratch`]), nothing this engine runs
    /// goes: its effects stay as they are beside the history's, its layers
    /// over the history's, and its pose memory where the history set none --
    /// and should a pose the history started a move from have moved on since
    /// the scratch, the history replays again here, from it. Every rendered
    /// frame rewrites a moving head's pose, so while movers move that is
    /// most such installs, a history move reading one: the replay then runs
    /// under the caller's lock after all, at a plain replay's cost. Returns
    /// the errors of the effects that would not start.
    pub fn install(&mut self, prepared: Prepared) -> Vec<EffectError> {
        let Prepared {
            at,
            scope,
            instant,
            mut effects,
            mut layer_state,
            poses,
            goals,
            pose_reads,
            again,
            errors,
        } = prepared;
        self.stop_by_scope(scope);
        if let (Some(reads), Some(history)) = (&pose_reads, again) {
            if reads
                .read
                .iter()
                .any(|name| self.poses.get(name) != reads.initial.get(name))
            {
                return self.replay(history);
            }
        }
        let rebase = |then: Instant| match self.current_time.checked_duration_since(instant) {
            Some(later) => then + later,
            None => then
                .checked_sub(instant.duration_since(self.current_time))
                .unwrap_or(then),
        };
        for effect in effects.values_mut() {
            effect.start_time = effect.start_time.map(rebase);
        }
        layer_state.rebase(rebase);
        match pose_reads {
            // Beside what runs here, which is later than the history.
            Some(reads) => {
                effects.extend(std::mem::take(&mut self.active_effects));
                layer_state.overlay(&self.layer_state);
                if reads.released {
                    self.poses = poses;
                    self.goals = goals;
                } else {
                    for name in &reads.written {
                        if let Some(pose) = poses.get(name) {
                            self.poses.insert(name.clone(), *pose);
                        }
                        if let Some(goal) = goals.get(name) {
                            self.goals.insert(name.clone(), *goal);
                        }
                    }
                }
            }
            None => {
                self.poses = poses;
                self.goals = goals;
            }
        }
        self.active_effects = effects;
        self.layer_state = layer_state;
        self.last_song_time = Some(at);
        self.cache.invalidate();
        errors
    }

    /// Applies one step of what a timeline fired.
    fn apply_step(&mut self, step: Step) -> Result<(), EffectError> {
        match step {
            Step::Command(cmd) => {
                self.apply_layer_command(&cmd);
                Ok(())
            }
            Step::Stop(name) => {
                self.stop_sequence(&name);
                Ok(())
            }
            Step::Start(effect) => self.start_effect(effect),
        }
    }

    /// Moves a replay's time on to `time` as a frame there would, without
    /// rendering one: the effects run their course by then go (a move's
    /// destination committed to pose memory, as the frame it completes on
    /// writes it), and each move still running holds its heads where it has
    /// them then, as the frame would leave pose memory -- where a clear, a
    /// stop or a later move at `time` finds them.
    fn advance(&mut self, time: Duration) {
        self.last_song_time = Some(time);
        let now = self.now();
        let expired: Vec<String> = self
            .active_effects
            .iter()
            .filter(|(_, effect)| {
                effect.start_time.is_some()
                    && effect.has_reached_terminal_state(self.elapsed_at(effect, now.song))
            })
            .map(|(id, _)| id.clone())
            .collect();
        if !expired.is_empty() {
            // The cached frame has them in it.
            self.cache.invalidate();
        }
        for id in expired {
            let Some(effect) = self.active_effects.remove(&id) else {
                continue;
            };
            if let Some(ends) = self.ends.as_mut() {
                let end = self.layer_state.clock(effect.layer).end_of(&effect, now);
                ends.insert(id, end);
            }
            for (name, pose) in
                processing::move_final_poses(self.fixtures.as_map(), &self.focus_points, &effect)
            {
                self.hold_pose(name, pose);
            }
        }
        let mut held = Vec::new();
        for effect in self.active_effects.values() {
            if matches!(effect.effect_type, EffectType::Move { .. }) {
                let elapsed = self.elapsed_at(effect, now.song);
                held.extend(processing::move_poses_at(
                    self.fixtures.as_map(),
                    &self.focus_points,
                    effect,
                    elapsed,
                ));
            }
        }
        for (name, pose) in held {
            self.hold_pose(name, pose);
        }
    }

    /// Holds `name` at `pose` in pose memory, as a replay's frame leaves it.
    fn hold_pose(&mut self, name: String, pose: Pose) {
        if let Some(reads) = self.pose_reads.as_mut() {
            reads.written.insert(name.clone());
        }
        self.goals.insert(name.clone(), pose);
        self.poses.insert(name, pose);
    }

    /// An engine to reckon a show's ends by, with no rig: effects start as
    /// they are, nothing renders, and each effect's end is kept as it goes
    /// (see `lighting::timeline::effect_ends`).
    pub(crate) fn reckoner() -> Self {
        Self {
            rigless: true,
            ends: Some(HashMap::new()),
            ..Self::new()
        }
    }

    /// A reckoner's ends: when each effect taken out ended, and when each
    /// still running ends from the song time last moved on to, its layer as
    /// it is then. `None` for one that never ends.
    pub(crate) fn take_ends(&mut self) -> HashMap<String, Option<Duration>> {
        let now = self.now();
        let mut ends = self.ends.take().unwrap_or_default();
        for (id, effect) in &self.active_effects {
            let end = self.layer_state.clock(effect.layer).end_of(effect, now);
            ends.insert(id.clone(), end);
        }
        ends
    }

    /// Starts `effect` `elapsed` into itself, as a seek to that far past its
    /// cue would: for tests of effects started part way.
    #[cfg(test)]
    pub fn start_effect_part_way(
        &mut self,
        mut effect: EffectInstance,
        elapsed: Duration,
    ) -> Result<(), EffectError> {
        let cue = effect.cue_time.unwrap_or(Duration::ZERO);
        effect.cue_time = Some(cue);
        let mut history = crate::lighting::timeline::SeekHistory {
            at: cue + elapsed,
            ..Default::default()
        };
        history.update.steps.push((cue, Step::Start(effect)));
        match self.start_from(history, StopScope::Nothing).pop() {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    /// Dispatch a single layer command to the appropriate engine method.
    pub fn apply_layer_command(&mut self, cmd: &crate::lighting::parser::LayerCommand) {
        use crate::lighting::parser::LayerCommandType;
        match cmd.command_type {
            LayerCommandType::Clear => {
                if let Some(layer) = cmd.layer {
                    self.clear_layer(layer);
                } else {
                    self.clear_all_layers();
                }
            }
            LayerCommandType::Freeze => {
                if let Some(layer) = cmd.layer {
                    self.freeze_layer(layer);
                }
            }
            LayerCommandType::Unfreeze => {
                if let Some(layer) = cmd.layer {
                    self.unfreeze_layer(layer);
                }
            }
            LayerCommandType::Master => {
                if let Some(layer) = cmd.layer {
                    if let Some(intensity) = cmd.intensity {
                        self.set_layer_intensity_master(layer, intensity);
                    }
                    if let Some(speed) = cmd.speed {
                        self.set_layer_speed_master(layer, speed);
                    }
                }
            }
        }
    }

    /// Get the number of active effects
    #[cfg(test)]
    pub fn active_effects_count(&self) -> usize {
        self.active_effects.len()
    }

    /// Check if a specific effect is active
    #[cfg(test)]
    pub fn has_effect(&self, effect_id: &str) -> bool {
        self.active_effects.contains_key(effect_id)
    }

    /// Get all active effects (for debugging/simulation)
    #[allow(dead_code)]
    pub fn get_active_effects(&self) -> &HashMap<String, EffectInstance> {
        &self.active_effects
    }

    #[allow(dead_code)]
    /// Get current fixture states (for debugging/simulation/preview)
    /// Returns the merged fixture states from the last update() call.
    /// This provides a snapshot of all fixture states without generating DMX commands.
    pub fn get_fixture_states(&self) -> HashMap<String, FixtureState> {
        self.last_merged_states.clone()
    }

    /// Get the fixture registry (for simulator to determine fixture capabilities).
    #[allow(dead_code)]
    pub fn get_fixture_registry(&self) -> &HashMap<String, FixtureInfo> {
        self.fixtures.as_map()
    }

    /// How far into itself an effect was at the last frame, as it rendered
    /// it (see [`Self::elapsed_at`]). Zero for an effect that has not
    /// started.
    pub fn effect_elapsed(&self, effect: &EffectInstance) -> Duration {
        self.elapsed_at(effect, self.last_song_time)
    }

    /// Get a formatted string listing all active effects
    pub fn format_active_effects(&self) -> String {
        use std::fmt::Write;
        let mut output = String::new();

        if self.active_effects.is_empty() {
            return "No active effects".to_string();
        }

        writeln!(output, "Active effects ({}):", self.active_effects.len()).unwrap();

        // Group effects by layer for better readability
        let mut effects_by_layer: std::collections::HashMap<EffectLayer, Vec<&EffectInstance>> =
            std::collections::HashMap::new();

        for effect in self.active_effects.values() {
            effects_by_layer
                .entry(effect.layer)
                .or_default()
                .push(effect);
        }

        // Sort layers for consistent output
        let mut layers: Vec<_> = effects_by_layer.keys().collect();
        layers.sort();

        for layer in layers {
            let effects = &effects_by_layer[layer];
            // Print layer name
            let layer_name = match layer {
                EffectLayer::Background => "Background",
                EffectLayer::Midground => "Midground",
                EffectLayer::Foreground => "Foreground",
            };
            writeln!(output, "  {}:", layer_name).unwrap();
            for effect in effects {
                let elapsed = self.effect_elapsed(effect);
                let effect_type_str = effect.effect_type.name();

                let total = effect.total_duration();
                let duration_str = format!(" (duration: {:.2}s)", total.as_secs_f64());

                writeln!(
                    output,
                    "    - {} [{}] - {} fixture(s) - elapsed: {:.2}s{}",
                    effect.id,
                    effect_type_str,
                    effect.target_fixtures.len(),
                    elapsed.as_secs_f64(),
                    duration_str
                )
                .unwrap();
            }
        }

        output
    }
}

/// The state a start from history leaves, replayed apart from the engine it
/// goes in (see [`EffectEngine::prepare`]).
pub struct Prepared {
    at: Duration,
    scope: StopScope,
    /// The scratch engine's instant once replayed: that of `at`.
    instant: Instant,
    effects: HashMap<String, EffectInstance>,
    layer_state: LayerState,
    poses: HashMap<String, Pose>,
    goals: HashMap<String, Pose>,
    pose_reads: Option<PoseReads>,
    /// The history, to replay again at the install (see `PoseReads`).
    again: Option<crate::lighting::timeline::SeekHistory>,
    errors: Vec<EffectError>,
}

/// How a DMX channel crosses a crossfade ([`EffectEngine::crossfade_commands`]).
/// Ordered by which of two patched over one channel takes it: a 16-bit
/// value's byte before a byte of its own, then by kind and partner.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum CrossPlace {
    /// The coarse byte of a 16-bit value, with the DMX channel of its fine
    /// byte.
    Coarse(CrossKind, u16),
    /// The fine byte of a 16-bit value, with the DMX channel of its coarse
    /// byte.
    Fine(CrossKind, u16),
    /// A byte of its own.
    Byte(CrossKind),
}

impl CrossPlace {
    fn kind(self) -> CrossKind {
        match self {
            CrossPlace::Coarse(kind, _) | CrossPlace::Fine(kind, _) | CrossPlace::Byte(kind) => {
                kind
            }
        }
    }
}

/// How a replay from another engine's pose memory used it (see
/// [`EffectEngine::scratch`]): what it started from, the heads whose pose a
/// move started from before the replay set one there, the heads it set, and
/// whether a full clear let it all go. The replay's poses then hold for the
/// heads it set (all of them, after a full clear) and the memory as of the
/// install for the rest -- unless a pose a move started from has moved on,
/// when it replays again from the memory as it is.
#[derive(Default)]
struct PoseReads {
    initial: HashMap<String, Pose>,
    read: HashSet<String>,
    written: HashSet<String>,
    released: bool,
}

/// Where the stop before a start from history reaches (see
/// [`EffectEngine::start_from`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StopScope {
    /// Nothing: the engine is fresh for the start.
    Nothing,
    /// This engine's show -- its effects, layers and pose memory -- but not
    /// the MIDI DMX store every engine shares.
    Show,
    /// The show and the MIDI DMX store.
    All,
}

/// A replay of cue history through an engine, cue time by cue time up to the
/// time it starts from, no frame rendered. The engine's instant is set back
/// to each time's place before that one, so what it stamps there -- an
/// effect's start, a freeze -- is where playing through would have put it.
pub(crate) struct Replay<'a> {
    engine: &'a mut EffectEngine,
    /// The engine's instant: that of `to`.
    now: Instant,
    /// The song time started from.
    to: Duration,
    errors: Vec<EffectError>,
}

impl<'a> Replay<'a> {
    pub(crate) fn new(engine: &'a mut EffectEngine, to: Duration) -> Self {
        let now = engine.current_time;
        Self {
            engine,
            now,
            to,
            errors: Vec::new(),
        }
    }

    /// Applies `steps`, what one time's cues fired, at `time`: the time moved
    /// on to there first (see [`EffectEngine::advance`]). A time that fired
    /// nothing changes nothing.
    pub(crate) fn at(&mut self, time: Duration, steps: impl IntoIterator<Item = Step>) {
        let steps: Vec<Step> = steps.into_iter().collect();
        if steps.is_empty() {
            return;
        }
        // While tracing, the frame here goes to the wire only where what
        // fires can stop a byte being written: a layer command (a clear, a
        // master), a stopped sequence, or a move, which may take heads from
        // another (`take_heads`). An effect starting stops nothing: every
        // byte written before it is still written after, until a traced
        // end, cue or sample. A move taking a head writes the same pan and
        // tilt from the same instant, so its frame is a margin rather than
        // a need today; it is kept so that skipping plain starts does not
        // rest on pose memory moving on without a frame for every future
        // move or pose rule, and it costs one frame per move start.
        let stops = steps.iter().any(|step| match step {
            Step::Start(effect) => matches!(effect.effect_type, EffectType::Move { .. }),
            Step::Command(_) | Step::Stop(_) => true,
        });
        self.trace_until(time, stops);
        for step in steps {
            if let Err(e) = self.engine.apply_step(step) {
                self.errors.push(e);
            }
        }
    }

    /// Applies `steps` (in time order) cue time by cue time ([`Self::at`]),
    /// and at each of `times` (ascending), once every step up to and at it
    /// is in, [`Self::sample`]s the engine there: the one walk through a
    /// replay's steps.
    fn play(
        &mut self,
        steps: Vec<(Duration, Step)>,
        times: &[Duration],
        sample: &mut impl FnMut(&EffectEngine, Duration),
    ) {
        let mut steps = steps.into_iter().peekable();
        let mut times = times.iter().copied().peekable();
        loop {
            let next_step = steps.peek().map(|&(time, _)| time);
            let next_time = times.peek().copied();
            // A step at a sampled time goes in before the sample.
            match (next_step, next_time) {
                (Some(time), at) if at.is_none_or(|at| time <= at) => {
                    let group: Vec<Step> =
                        std::iter::from_fn(|| steps.next_if(|(t, _)| *t == time))
                            .map(|(_, step)| step)
                            .collect();
                    self.at(time, group);
                }
                (_, Some(at)) => {
                    times.next();
                    self.sample(at, sample);
                }
                (_, None) => break,
            }
        }
    }

    /// Moves the time on to the start point, and leaves the engine there.
    pub(crate) fn finish(mut self) -> Vec<EffectError> {
        self.trace_until(self.to, true);
        self.engine.cache.invalidate();
        self.errors
    }

    /// Moves the time on to `time` -- every cue up to and at it already
    /// applied -- and renders the frame there, sends it to the wire and
    /// hands the engine to `sample`, as a start at `time` would leave it.
    /// Pose memory is then put back as the replay has it, so the replay
    /// carries on as if no frame had been rendered.
    fn sample(&mut self, time: Duration, sample: &mut impl FnMut(&EffectEngine, Duration)) {
        self.trace_until(time, false);
        self.engine
            .send_replayed_frame(time, |engine| sample(engine, time));
    }

    /// Moves the time on to `time` (see [`EffectEngine::advance`]), the
    /// engine's instant with it.
    fn move_to(&mut self, time: Duration) {
        self.engine.current_time = self
            .now
            .checked_sub(self.to.saturating_sub(time))
            .unwrap_or(self.now);
        self.engine.advance(time);
    }

    /// Moves the time on to `time`; while the engine traces the wire
    /// ([`EffectEngine::trace_wire`]), sending it the frame just before each
    /// effect ending on the way goes, and -- with `frame` -- the frame at
    /// `time` before what fires there.
    fn trace_until(&mut self, time: Duration, frame: bool) {
        if self.engine.tracing_wire {
            for end in self.engine.ends_until(time) {
                let from = self.engine.last_song_time.unwrap_or_default();
                let before = end.saturating_sub(JUST_BEFORE).max(from);
                self.move_to(before);
                self.engine.send_replayed_frame(before, |_| {});
                self.move_to(end);
            }
        }
        self.move_to(time);
        if frame && self.engine.tracing_wire {
            self.engine.send_replayed_frame(time, |_| {});
        }
    }
}

/// Whether `effect` may write a channel that is not a level or colour
/// ([`fades_to_dark`]) -- a position, a strobe, a gobo -- which the rig
/// holds at its last byte once the effect ends. A static setting only
/// levels and colours, a colour cycle, rainbow, dimmer, pulse or chase
/// writes none.
fn leaves_settings(effect: &EffectInstance) -> bool {
    match &effect.effect_type {
        EffectType::Static { parameters, .. } => {
            parameters.keys().any(|channel| !fades_to_dark(channel))
        }
        EffectType::Move { .. } | EffectType::Strobe { .. } => true,
        EffectType::ColorCycle { .. }
        | EffectType::Rainbow { .. }
        | EffectType::Dimmer { .. }
        | EffectType::Pulse { .. }
        | EffectType::Chase { .. } => false,
    }
}

/// How long before an effect ends a traced replay renders its last frame
/// (see [`Replay::trace_until`]): within the live frame before its end.
const JUST_BEFORE: Duration = Duration::from_micros(1);
