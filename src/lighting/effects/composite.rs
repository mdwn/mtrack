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

//! The brightness model (design 3.4): one rule for every fixture, however
//! its hardware takes a level.
//!
//! Effects do not write colour and level into channels. A colour effect
//! records its `red/green/blue/white` and a dimmer, pulse, chase or
//! software strobe records one level (under [`INTENSITY`]), each an [`Op`]
//! with its layer and blend mode. At the end of the frame the ops are
//! composited bottom layer up, as light, and written back as plain
//! channels: dimming on a layer applies to that layer and the layers
//! beneath it, never to the layers above.
//!
//! The composite keeps, per colour channel, the light so far and the same
//! light with every dimmer treated as full (what a dimmer `replace`
//! restores), and one scalar peak: the highest level among the layers that
//! contribute colour, which is what a dimmer channel is sent.

use std::collections::HashMap;

use super::fixture::{has_cmy, has_dimmer_channel, FixtureInfo};
use super::state::{blend_value, ChannelState, FixtureState};
use super::types::{BlendMode, EffectLayer};
use crate::lighting::channels::lerp;

/// The internal channel an effect's level is set under. Never a real
/// channel, never written out: [`FixtureState::set_channel`] turns it into
/// an op.
pub const INTENSITY: &str = "_intensity";

/// The colour channels that are composited, in the order of the arrays.
const COLOUR: [&str; 4] = ["red", "green", "blue", "white"];

/// What an [`Op`] sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OpKind {
    /// One of [`COLOUR`], by index.
    Colour(usize),
    /// The level.
    Intensity,
}

/// The op a channel name stands for, `None` for an ordinary channel.
pub(super) fn op_kind(name: &str) -> Option<OpKind> {
    if name == INTENSITY {
        return Some(OpKind::Intensity);
    }
    COLOUR.iter().position(|c| *c == name).map(OpKind::Colour)
}

/// One effect's ask of a fixture's colour or level.
#[derive(Debug, Clone, Copy)]
pub(super) struct Op {
    pub kind: OpKind,
    /// Value, layer, blend mode and mix (a crossfading replace's).
    pub state: ChannelState,
    /// The effect's state this came from, so its colour channels count as
    /// one ask when the peak is worked out.
    pub group: u32,
}

/// Plain channel values beneath the ops: a MIDI DMX show's, the bottom of
/// the stack.
#[derive(Default)]
struct Seed {
    colour: [Option<f64>; 4],
    dimmer: Option<f64>,
    /// A show set the flags of a CMY fixture directly: colour that is not
    /// red, green or blue, but colour all the same, not to be painted
    /// white over.
    flags: bool,
}

impl Seed {
    fn of(state: &FixtureState, dimmer: bool) -> Self {
        let value = |name: &str| state.channels.get(name).map(|c| c.value);
        Seed {
            colour: COLOUR.map(value),
            dimmer: if dimmer { value("dimmer") } else { None },
            flags: crate::lighting::channels::CMY_OF_RGB
                .iter()
                .any(|(_, flag)| state.channels.contains_key(*flag)),
        }
    }
}

/// How the level reaches the colour channels written.
enum Level {
    /// The state has a dimmer of its own: it gets the peak, and colour is
    /// the light relative to it.
    Own,
    /// A dimmer shared by cells, already the highest peak among them.
    Shared(f64),
    /// No dimmer: the level is carried in the colour.
    Plain,
}

/// The result of compositing one fixture's (or cell's) ops.
struct Composite {
    light: [f64; 4],
    undimmed: [f64; 4],
    driven: [bool; 4],
    /// `None` until a level exists, for a fixture with no colour at all:
    /// the first level there is the level, not a change to full white.
    peak: Option<f64>,
    /// No colour anywhere: white at the bottom (design rule 7).
    white_bottom: bool,
    top: EffectLayer,
}

impl Composite {
    fn new(ops: &[Op], seed: &Seed) -> Self {
        let has_colour = seed.colour.iter().any(Option::is_some)
            || seed.flags
            || ops.iter().any(|op| matches!(op.kind, OpKind::Colour(_)));
        let mut c = if has_colour {
            let light = seed.colour.map(|v| v.unwrap_or(0.0));
            Composite {
                light,
                undimmed: light,
                driven: seed.colour.map(|v| v.is_some()),
                peak: Some(if seed.colour.iter().any(Option::is_some) || seed.flags {
                    1.0
                } else {
                    0.0
                }),
                white_bottom: false,
                top: EffectLayer::Background,
            }
        } else {
            Composite {
                light: [1.0; 4],
                undimmed: [1.0; 4],
                driven: [false; 4],
                peak: None,
                white_bottom: true,
                top: EffectLayer::Background,
            }
        };
        if let Some(dimmer) = seed.dimmer {
            c.intensity(BlendMode::Replace, dimmer, 1.0);
        }

        // Bottom layer up; within a layer the colour ops in their order,
        // then the level ops in theirs. The sort is stable.
        let mut order: Vec<&Op> = ops.iter().collect();
        order.sort_by_key(|op| op.state.layer);
        let mut i = 0;
        while i < order.len() {
            let layer = order[i].state.layer;
            let end = order[i..]
                .iter()
                .position(|op| op.state.layer != layer)
                .map_or(order.len(), |n| i + n);
            let mut group = None;
            for op in order[i..end]
                .iter()
                .filter(|op| matches!(op.kind, OpKind::Colour(_)))
            {
                if let OpKind::Colour(channel) = op.kind {
                    c.colour(channel, &op.state);
                    if group != Some(op.group) {
                        group = Some(op.group);
                        c.lift_peak(&op.state);
                    }
                }
            }
            for op in order[i..end]
                .iter()
                .filter(|op| op.kind == OpKind::Intensity)
            {
                c.intensity(op.state.blend_mode, op.state.value, op.state.mix);
            }
            c.top = c.top.max(layer);
            i = end;
        }
        c
    }

    /// One colour channel's op.
    fn colour(&mut self, channel: usize, state: &ChannelState) {
        let (v, m) = (state.value, state.mix);
        if !self.driven[channel] {
            // Nothing beneath this channel: the ask is the value, a
            // replace fading in from nothing.
            let first = if state.blend_mode == BlendMode::Replace {
                v * m
            } else {
                v
            };
            self.light[channel] = first;
            self.undimmed[channel] = first;
            self.driven[channel] = true;
            return;
        }
        let both = |value: f64| match state.blend_mode {
            BlendMode::Replace => lerp(value, v, m),
            mode => blend_value(value, v, mode, 1.0),
        };
        self.light[channel] = both(self.light[channel]);
        self.undimmed[channel] = both(self.undimmed[channel]);
    }

    /// What a colour ask does to the peak: a replace moves it toward full
    /// by its mix, a multiply only filters, anything else brings full.
    fn lift_peak(&mut self, state: &ChannelState) {
        match state.blend_mode {
            BlendMode::Multiply => {}
            BlendMode::Replace => {
                self.peak = Some(lerp(self.peak.unwrap_or(0.0), 1.0, state.mix));
            }
            _ => self.peak = Some(1.0),
        }
    }

    /// One level op.
    fn intensity(&mut self, mode: BlendMode, v: f64, m: f64) {
        let Some(peak) = self.peak else {
            // No colour and no level so far: this is the level, and the
            // light is white at it.
            let first = if mode == BlendMode::Replace { v * m } else { v }.clamp(0.0, 1.0);
            self.peak = Some(first);
            for i in 0..4 {
                self.light[i] = self.undimmed[i] * first;
            }
            return;
        };
        match mode {
            BlendMode::Multiply => {
                for l in &mut self.light {
                    *l *= v;
                }
                self.peak = Some(peak * v);
            }
            BlendMode::Replace => {
                for i in 0..4 {
                    self.light[i] = lerp(self.light[i], self.undimmed[i] * v, m);
                }
                self.peak = Some(lerp(peak, v, m));
            }
            _ => {
                let target = blend_value(peak, v, mode, 1.0).clamp(0.0, 1.0);
                for i in 0..4 {
                    self.light[i] = if peak > 0.0 {
                        self.light[i] * target / peak
                    } else {
                        self.undimmed[i] * target
                    };
                }
                self.peak = Some(target);
            }
        }
    }

    /// The highest level among what contributes colour.
    fn level(&self) -> f64 {
        self.peak.unwrap_or(1.0).clamp(0.0, 1.0)
    }

    /// The colour channels to write and their values. `takes` is which of
    /// the channels the target has, for the white a colour-less fixture
    /// shows.
    fn colours(&self, level: &Level, takes: [bool; 4]) -> Vec<(&'static str, f64)> {
        let relative = |i: usize, by: f64| {
            if by > 0.0 {
                self.light[i] / by
            } else {
                self.undimmed[i]
            }
        };
        (0..4)
            .filter(|&i| {
                if self.white_bottom {
                    takes[i]
                } else {
                    self.driven[i]
                }
            })
            .map(|i| {
                let value = match level {
                    Level::Own => relative(i, self.level()),
                    Level::Shared(by) => relative(i, *by),
                    Level::Plain => self.light[i],
                };
                (COLOUR[i], value.clamp(0.0, 1.0))
            })
            .collect()
    }
}

/// Which colour channels a fixture or cell takes: red, green and blue also
/// on a CMY fixture, which turns them into its flags at output.
fn takes<V>(channels: &HashMap<String, V>) -> [bool; 4] {
    let cmy = has_cmy(channels);
    COLOUR.map(|c| channels.contains_key(c) || (c != "white" && cmy))
}

fn put(state: &mut FixtureState, layer: EffectLayer, name: &str, value: f64) {
    state.channels.insert(
        name.to_string(),
        ChannelState::new(value, layer, BlendMode::Replace),
    );
}

/// Composites `state`'s ops into plain channels for a fixture with
/// `channels`, and drops the ops.
pub(super) fn resolve_plain<V>(state: &mut FixtureState, channels: &HashMap<String, V>) {
    if state.ops.is_empty() {
        return;
    }
    let ops = std::mem::take(&mut state.ops);
    state.groups = 0;
    let has_dimmer = has_dimmer_channel(channels);
    let seed = Seed::of(state, has_dimmer);
    let composite = Composite::new(&ops, &seed);
    let level = if has_dimmer { Level::Own } else { Level::Plain };
    for (name, value) in composite.colours(&level, takes(channels)) {
        put(state, composite.top, name, value);
    }
    if has_dimmer {
        put(state, composite.top, "dimmer", composite.level());
    }
}

/// Resolves a fixture, and the states of its cells (by cell name) if it is
/// a pixel fixture (design 17.3): a cell's ops are merged with the
/// fixture's by layer, the fixture's first within a layer, and composited
/// per cell. A dimmer the cells share gets the highest peak among them and
/// each cell's colour is scaled against it, so every cell's light is exact.
pub(crate) fn resolve_fixture(
    parent: &mut FixtureState,
    cells: &mut HashMap<String, FixtureState>,
    info: &FixtureInfo,
) {
    if info.cells.is_empty() {
        resolve_plain(parent, &info.channels);
        return;
    }
    let parent_ops = std::mem::take(&mut parent.ops);
    let parent_groups = std::mem::take(&mut parent.groups);
    let own: Vec<bool> = info
        .cells
        .iter()
        .map(|c| cells.get(&c.name).is_some_and(|s| !s.ops.is_empty()))
        .collect();
    if parent_ops.is_empty() && !own.iter().any(|o| *o) {
        return;
    }
    let cell_dimmer = info.cells[0].has_dimmer();
    let shared_dimmer = !cell_dimmer && info.has_dimmer();
    let seed = Seed::of(parent, cell_dimmer || shared_dimmer);

    // The fixture's ops alone, for the cells with none of their own.
    let ganged = (!parent_ops.is_empty() && own.iter().any(|o| !*o))
        .then(|| Composite::new(&parent_ops, &seed));
    let mut per_cell = Vec::new();
    for (cell, _) in info.cells.iter().zip(&own).filter(|(_, o)| **o) {
        let Some(state) = cells.get_mut(&cell.name) else {
            continue;
        };
        let mut ops = parent_ops.clone();
        ops.extend(state.ops.drain(..).map(|op| Op {
            group: op.group + parent_groups,
            ..op
        }));
        state.groups = 0;
        per_cell.push((cell, Composite::new(&ops, &seed)));
    }

    let shared = ganged
        .iter()
        .map(Composite::level)
        .chain(per_cell.iter().map(|(_, c)| c.level()))
        .fold(0.0, f64::max);
    let level = |c: &Composite| match (cell_dimmer, shared_dimmer) {
        (true, _) => (Level::Own, Some(c.level())),
        (false, true) => (Level::Shared(shared), None),
        (false, false) => (Level::Plain, None),
    };

    if let Some(g) = &ganged {
        let (level, dimmer) = level(g);
        for (name, value) in g.colours(&level, takes(&info.cells[0].channels)) {
            put(parent, g.top, name, value);
        }
        if let Some(dimmer) = dimmer {
            put(parent, g.top, "dimmer", dimmer);
        }
    }
    let mut top = ganged.as_ref().map(|g| g.top);
    for (cell, c) in &per_cell {
        let state = cells.get_mut(&cell.name).expect("a cell with ops");
        let (level, dimmer) = level(c);
        for (name, value) in c.colours(&level, takes(&cell.channels)) {
            put(state, c.top, name, value);
        }
        if let Some(dimmer) = dimmer {
            put(state, c.top, "dimmer", dimmer);
        }
        top = top.max(Some(c.top));
    }
    if shared_dimmer {
        put(
            parent,
            top.unwrap_or(EffectLayer::Background),
            "dimmer",
            shared,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(name: &str, value: f64, layer: EffectLayer, mode: BlendMode) -> (String, ChannelState) {
        (name.to_string(), ChannelState::new(value, layer, mode))
    }

    fn red(layer: EffectLayer, mode: BlendMode) -> Vec<(String, ChannelState)> {
        vec![
            op("red", 1.0, layer, mode),
            op("green", 0.0, layer, mode),
            op("blue", 0.0, layer, mode),
        ]
    }

    fn blue(layer: EffectLayer, mode: BlendMode) -> Vec<(String, ChannelState)> {
        vec![
            op("red", 0.0, layer, mode),
            op("green", 0.0, layer, mode),
            op("blue", 1.0, layer, mode),
        ]
    }

    fn level(v: f64, layer: EffectLayer, mode: BlendMode) -> Vec<(String, ChannelState)> {
        vec![op(INTENSITY, v, layer, mode)]
    }

    fn channels(names: &[&str]) -> HashMap<String, u16> {
        names
            .iter()
            .enumerate()
            .map(|(i, n)| (n.to_string(), i as u16 + 1))
            .collect()
    }

    /// Effects' states blended in the order given, then resolved.
    fn resolved(effects: Vec<Vec<(String, ChannelState)>>, fixture: &[&str]) -> FixtureState {
        let mut state = FixtureState::new();
        for effect in effects {
            state.blend_with(&FixtureState::from_channels(effect));
        }
        state.resolve(&channels(fixture));
        state
    }

    fn value(state: &FixtureState, name: &str) -> f64 {
        state.channels[name].value
    }

    const DIMMER_RGB: [&str; 4] = ["dimmer", "red", "green", "blue"];
    const RGB: [&str; 3] = ["red", "green", "blue"];
    use EffectLayer::{Background as A, Foreground as C, Midground as B};

    #[test]
    fn a_dimmed_layer_dims_itself_on_both_kinds() {
        let effects = || {
            vec![
                red(A, BlendMode::Replace),
                level(0.5, A, BlendMode::Replace),
            ]
        };
        let s = resolved(effects(), &DIMMER_RGB);
        assert_eq!((value(&s, "dimmer"), value(&s, "red")), (0.5, 1.0));
        let s = resolved(effects(), &RGB);
        assert_eq!((value(&s, "red"), value(&s, "blue")), (0.5, 0.0));
        assert!(!s.channels.contains_key("dimmer"));
    }

    #[test]
    fn the_dimming_does_not_reach_the_layers_above() {
        for fixture in [&DIMMER_RGB[..], &RGB[..]] {
            let s = resolved(
                vec![
                    red(A, BlendMode::Replace),
                    level(0.5, A, BlendMode::Replace),
                    blue(C, BlendMode::Replace),
                ],
                fixture,
            );
            assert_eq!(value(&s, "blue"), 1.0, "{fixture:?}");
            assert_eq!(value(&s, "red"), 0.0, "{fixture:?}");
            if fixture.contains(&"dimmer") {
                assert_eq!(value(&s, "dimmer"), 1.0);
            }
        }
    }

    #[test]
    fn a_midground_colour_replace_is_not_dimmed_by_the_background() {
        for fixture in [&DIMMER_RGB[..], &RGB[..]] {
            let s = resolved(
                vec![
                    red(A, BlendMode::Replace),
                    level(0.5, A, BlendMode::Replace),
                    blue(B, BlendMode::Replace),
                ],
                fixture,
            );
            assert_eq!(value(&s, "blue"), 1.0, "{fixture:?}");
        }
    }

    #[test]
    fn a_layers_own_dimmer_dims_its_colour() {
        let effects = || {
            vec![
                red(A, BlendMode::Replace),
                blue(C, BlendMode::Replace),
                level(0.5, C, BlendMode::Replace),
            ]
        };
        let s = resolved(effects(), &DIMMER_RGB);
        assert_eq!((value(&s, "dimmer"), value(&s, "blue")), (0.5, 1.0));
        let s = resolved(effects(), &RGB);
        assert_eq!(value(&s, "blue"), 0.5);
    }

    #[test]
    fn a_multiply_colour_only_filters_the_dimmed_light() {
        let white = vec![
            op("red", 1.0, C, BlendMode::Multiply),
            op("green", 1.0, C, BlendMode::Multiply),
            op("blue", 1.0, C, BlendMode::Multiply),
        ];
        for fixture in [&DIMMER_RGB[..], &RGB[..]] {
            let s = resolved(
                vec![
                    red(A, BlendMode::Replace),
                    level(0.5, A, BlendMode::Replace),
                    white.clone(),
                ],
                fixture,
            );
            if fixture.contains(&"dimmer") {
                assert_eq!((value(&s, "dimmer"), value(&s, "red")), (0.5, 1.0));
            } else {
                assert_eq!(value(&s, "red"), 0.5);
            }
        }
    }

    #[test]
    fn an_added_colour_is_not_dimmed_by_the_dimmer_beneath() {
        let s = resolved(
            vec![
                red(A, BlendMode::Replace),
                level(0.5, A, BlendMode::Replace),
                blue(C, BlendMode::Add),
            ],
            &RGB,
        );
        assert_eq!((value(&s, "red"), value(&s, "blue")), (0.5, 1.0));
        let s = resolved(
            vec![
                red(A, BlendMode::Replace),
                level(0.5, A, BlendMode::Replace),
                blue(C, BlendMode::Add),
            ],
            &DIMMER_RGB,
        );
        assert_eq!(value(&s, "dimmer"), 1.0);
        assert_eq!((value(&s, "red"), value(&s, "blue")), (0.5, 1.0));
    }

    #[test]
    fn a_dimmer_replace_above_a_lower_dimmer_sets_the_level() {
        for fixture in [&DIMMER_RGB[..], &RGB[..]] {
            let s = resolved(
                vec![
                    red(A, BlendMode::Replace),
                    level(0.2, A, BlendMode::Replace),
                    level(0.8, B, BlendMode::Replace),
                ],
                fixture,
            );
            let got = if fixture.contains(&"dimmer") {
                value(&s, "dimmer")
            } else {
                value(&s, "red")
            };
            assert!((got - 0.8).abs() < 1e-9, "{fixture:?}: {got}");
        }
    }

    #[test]
    fn a_dimmer_multiply_scales_the_level_beneath() {
        let s = resolved(
            vec![
                red(A, BlendMode::Replace),
                level(0.5, A, BlendMode::Replace),
                level(0.5, B, BlendMode::Multiply),
            ],
            &RGB,
        );
        assert!((value(&s, "red") - 0.25).abs() < 1e-9);
    }

    #[test]
    fn a_crossfading_colour_replace_has_no_brightness_jump() {
        // Red at half dimmer, blue crossfading in over it.
        let mut previous: Option<f64> = None;
        for step in 0..=10 {
            let m = step as f64 / 10.0;
            let fade: Vec<_> = blue(C, BlendMode::Replace)
                .into_iter()
                .map(|(n, s)| (n, s.with_mix(m)))
                .collect();
            let s = resolved(
                vec![
                    red(A, BlendMode::Replace),
                    level(0.5, A, BlendMode::Replace),
                    fade,
                ],
                &RGB,
            );
            let total = value(&s, "red") + value(&s, "blue");
            if let Some(p) = previous {
                assert!((total - p).abs() <= 0.5 / 10.0 + 1e-9, "{p} -> {total}");
            }
            previous = Some(total);
        }
        assert_eq!(previous, Some(1.0));
    }

    #[test]
    fn colour_alone_on_a_dimmer_fixture_is_full() {
        let s = resolved(vec![red(A, BlendMode::Replace)], &DIMMER_RGB);
        assert_eq!((value(&s, "dimmer"), value(&s, "red")), (1.0, 1.0));
    }

    #[test]
    fn a_level_with_no_colour_is_white_at_that_level() {
        let s = resolved(vec![level(0.4, A, BlendMode::Replace)], &DIMMER_RGB);
        assert_eq!(value(&s, "dimmer"), 0.4);
        assert_eq!(
            (value(&s, "red"), value(&s, "green"), value(&s, "blue")),
            (1.0, 1.0, 1.0)
        );
        let s = resolved(vec![level(0.4, A, BlendMode::Replace)], &RGB);
        assert_eq!(
            (value(&s, "red"), value(&s, "green"), value(&s, "blue")),
            (0.4, 0.4, 0.4)
        );
    }

    #[test]
    fn a_dimmer_only_fixture_just_blends_its_levels() {
        let s = resolved(
            vec![
                level(0.5, A, BlendMode::Replace),
                level(0.5, B, BlendMode::Add),
            ],
            &["dimmer"],
        );
        assert_eq!(s.channels.len(), 1);
        assert_eq!(value(&s, "dimmer"), 1.0);
        let s = resolved(vec![level(0.3, A, BlendMode::Add)], &["dimmer"]);
        assert_eq!(value(&s, "dimmer"), 0.3);
    }

    #[test]
    fn a_strobe_over_red_flashes_red() {
        for on in [1.0, 0.0] {
            let s = resolved(
                vec![
                    red(A, BlendMode::Replace),
                    level(
                        on,
                        C,
                        if on == 0.0 {
                            BlendMode::Replace
                        } else {
                            BlendMode::Multiply
                        },
                    ),
                ],
                &RGB,
            );
            assert_eq!((value(&s, "red"), value(&s, "blue")), (on, 0.0));
        }
    }

    #[test]
    fn midi_values_are_the_bottom_of_the_stack() {
        let mut state = FixtureState::new();
        state.channels.insert(
            "red".to_string(),
            ChannelState::new(1.0, A, BlendMode::Replace),
        );
        state.channels.insert(
            "dimmer".to_string(),
            ChannelState::new(0.5, A, BlendMode::Replace),
        );
        state.blend_with(&FixtureState::from_channels(level(
            0.5,
            B,
            BlendMode::Multiply,
        )));
        state.resolve(&channels(&DIMMER_RGB));
        assert_eq!((value(&state, "dimmer"), value(&state, "red")), (0.25, 1.0));
    }

    #[test]
    fn a_state_with_no_ops_is_left_alone() {
        let mut state = FixtureState::new();
        state.channels.insert(
            "red".to_string(),
            ChannelState::new(0.3, A, BlendMode::Replace),
        );
        state.resolve(&channels(&DIMMER_RGB));
        assert_eq!(state.channels.len(), 1);
        assert_eq!(value(&state, "red"), 0.3);
    }

    #[test]
    fn the_colour_channels_of_one_effect_lift_the_peak_once() {
        // A half-faded replace of a three-channel colour over nothing: the
        // peak is the mix, not the mix compounded per channel.
        let fade: Vec<_> = blue(A, BlendMode::Replace)
            .into_iter()
            .map(|(n, s)| (n, s.with_mix(0.5)))
            .collect();
        let s = resolved(vec![fade], &DIMMER_RGB);
        assert_eq!(value(&s, "dimmer"), 0.5);
        assert_eq!(value(&s, "blue"), 1.0);
    }

    fn bar(parent: &[&str], cell: &[&str]) -> FixtureInfo {
        let mut info = FixtureInfo::new(
            "bar".to_string(),
            1,
            1,
            "t".to_string(),
            channels(parent),
            None,
        );
        info.cells = ["1", "2"]
            .iter()
            .map(|name| crate::lighting::types::Cell {
                name: name.to_string(),
                channels: cell
                    .iter()
                    .enumerate()
                    .map(|(i, c)| {
                        (
                            c.to_string(),
                            crate::lighting::types::ChannelDef::at(10 + i as u16),
                        )
                    })
                    .collect(),
                offset: [0.0; 3],
            })
            .collect();
        info
    }

    #[test]
    fn a_pixel_bar_with_a_master_dimmer_scales_every_cell_against_its_highest() {
        let info = bar(&["dimmer"], &["red", "green", "blue"]);
        let mut parent = FixtureState::from_channels(red(A, BlendMode::Replace));
        parent.blend_with(&FixtureState::from_channels(level(
            0.5,
            A,
            BlendMode::Replace,
        )));
        let cell_2 = FixtureState::from_channels(level(1.0, B, BlendMode::Replace));
        let mut cells = HashMap::from([("2".to_string(), cell_2)]);
        resolve_fixture(&mut parent, &mut cells, &info);
        // Cell 2 is red at full; cell 1 is red at half.
        assert_eq!(value(&parent, "dimmer"), 1.0);
        assert_eq!(value(&parent, "red"), 0.5, "the ganged cell");
        assert_eq!(value(&cells["2"], "red"), 1.0);
        assert!(parent.ops.is_empty() && cells["2"].ops.is_empty());
    }

    #[test]
    fn cells_with_a_dimmer_of_their_own_use_their_own_peak() {
        let info = bar(&[], &["dimmer", "red", "green", "blue"]);
        let mut parent = FixtureState::from_channels(red(A, BlendMode::Replace));
        let mut cells = HashMap::from([(
            "1".to_string(),
            FixtureState::from_channels(level(0.25, B, BlendMode::Replace)),
        )]);
        resolve_fixture(&mut parent, &mut cells, &info);
        assert_eq!(value(&parent, "dimmer"), 1.0);
        assert_eq!(value(&cells["1"], "dimmer"), 0.25);
        assert_eq!(value(&cells["1"], "red"), 1.0);
    }

    #[test]
    fn cells_without_a_dimmer_carry_the_level_in_their_colour() {
        let info = bar(&[], &["red", "green", "blue"]);
        let mut parent = FixtureState::from_channels(red(A, BlendMode::Replace));
        let mut cells = HashMap::from([(
            "1".to_string(),
            FixtureState::from_channels(level(0.25, B, BlendMode::Replace)),
        )]);
        resolve_fixture(&mut parent, &mut cells, &info);
        assert_eq!(value(&parent, "red"), 1.0);
        assert_eq!(value(&cells["1"], "red"), 0.25);
    }
}
