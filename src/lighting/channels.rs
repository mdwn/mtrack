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

//! What a channel carries, read off its name: a level or colour going dark
//! takes to 0, where a head points, or a setting. The engine's crossfades
//! and its fade to dark, the snapshots and the web UI's stage views all
//! read a channel by these.

/// How a channel crosses a crossfade between two engines: their fixture
/// states and their frames' bytes alike ([`CrossKind::cross`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CrossKind {
    /// Intensity, colour, zoom, focus: fade.
    Fade,
    /// Pan and tilt: fade, holding a side's position where the other side
    /// drives none.
    Position,
    /// Anything whose in-between values are other settings: switch at the
    /// midpoint.
    Snap,
}

/// Whether a channel named `channel` is a level or a colour, which going
/// dark takes to 0 -- not a position, an optic (zoom, focus, iris, frost)
/// or a setting, which it leaves as they are.
pub(crate) fn fades_to_dark(channel: &str) -> bool {
    matches!(CrossKind::of(channel), CrossKind::Fade)
        && !matches!(base_channel(channel), "zoom" | "focus" | "iris" | "frost")
}

/// A channel's name without its section (`dimmer:background`, as GDTF
/// profiles with several emitters name theirs) or its fine byte.
pub(crate) fn base_channel(channel: &str) -> &str {
    channel_parts(channel).0
}

/// A channel's name as its attribute and its section (`dimmer:2` is
/// `dimmer` of section `2`; the fixture's own section is `""`), either
/// without the `_fine` its fine byte carries (`dimmer_fine`,
/// `dimmer:2_fine`).
pub(crate) fn channel_parts(channel: &str) -> (&str, &str) {
    let (base, section) = channel.split_once(':').unwrap_or((channel, ""));
    (coarse(base), coarse(section))
}

/// A name without the `_fine` a fine byte's carries.
fn coarse(name: &str) -> &str {
    name.strip_suffix("_fine").unwrap_or(name)
}

/// Whether a channel carries where a head points -- pan or tilt, a section
/// (`pan:1`) or fine byte of either -- rather than what it shows.
pub(crate) fn is_position_channel(channel: &str) -> bool {
    matches!(CrossKind::of(channel), CrossKind::Position)
}

/// The value `t` of the way (0.0 to 1.0) from `a` to `b`: exactly `a` at
/// 0 and exactly `b` at 1, and exactly `a` throughout when the two are
/// equal.
pub fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t
}

impl CrossKind {
    /// A channel's value `t` of the way (0.0 to 1.0) through a crossfade
    /// from `old` to `new`, `None` where that side drives none, on any
    /// scale (a level, a byte, a 16-bit value): faded, from or to 0 for a
    /// side that drives none; a position held where only one side drives
    /// it; a setting switched at the midpoint, 0 for a side that drives
    /// none. The one blend the engine crosses states and DMX by.
    pub(crate) fn cross(self, old: Option<f64>, new: Option<f64>, t: f64) -> f64 {
        match self {
            CrossKind::Position => {
                let from = old.or(new).unwrap_or(0.0);
                lerp(from, new.unwrap_or(from), t)
            }
            CrossKind::Snap if t < 0.5 => old.unwrap_or(0.0),
            CrossKind::Snap => new.unwrap_or(0.0),
            CrossKind::Fade => lerp(old.unwrap_or(0.0), new.unwrap_or(0.0), t),
        }
    }

    pub(crate) fn of(channel: &str) -> CrossKind {
        match base_channel(channel) {
            "pan" | "tilt" => CrossKind::Position,
            "dimmer" | "red" | "green" | "blue" | "white" | "warm_white" | "cool_white" | "uv"
            | "amber" | "lime" | "cyan" | "magenta" | "yellow" | "indigo" | "zoom" | "focus"
            | "iris" | "frost" | "ct" | "cto" | "ctb" | "color_temp"
            // GDTF's amber and lime, as their attributes come through.
            | "coloradd_ry" | "coloradd_gy" => CrossKind::Fade,
            // GDTF's other additive and RGB colour attributes, likewise.
            base if base.starts_with("coloradd_") || base.starts_with("colorrgb_") => {
                CrossKind::Fade
            }
            _ => CrossKind::Snap,
        }
    }
}

/// Whether a snapshot channel carries a level that lights the rig -- one
/// going dark takes to 0, as the engine has it (`fades_to_dark`) -- rather
/// than where a head points, an optic or a setting. The channel's name is
/// read under the `#n` a snapshot names a ganged channel's mirrors by.
pub fn is_level_channel(name: &str) -> bool {
    let base = match name.split_once('#') {
        Some((base, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => base,
        _ => name,
    };
    fades_to_dark(base)
}

/// A channel's full value: what a DMX channel reaches.
const FULL: f64 = 255.0;

/// Each additive primary and the subtractive channel that removes it.
pub const CMY_OF_RGB: [(&str, &str); 3] =
    [("red", "cyan"), ("green", "magenta"), ("blue", "yellow")];

/// Whether a fixture mixes colour subtractively: cyan, magenta and yellow
/// channels and no red, green and blue of its own (`has` says which
/// channels it has). Its cyan, magenta and yellow are then flags filtering
/// its white; otherwise they are emitters like any other colour.
pub fn mixes_subtractively(has: impl Fn(&str) -> bool) -> bool {
    CMY_OF_RGB.iter().all(|(_, cmy)| has(cmy)) && !CMY_OF_RGB.iter().any(|(rgb, _)| has(rgb))
}

/// What one part of a fixture emits, read off its level channels: each
/// level by its attribute (`red`, `dimmer`), the brightest of the coarse
/// and fine bytes and of any mirrors.
#[derive(Default)]
struct Light {
    levels: std::collections::BTreeMap<String, f64>,
}

impl Light {
    /// Takes in a level channel's value, by its attribute.
    fn add(&mut self, base: &str, value: f64) {
        let level = self.levels.entry(base.to_string()).or_insert(value);
        *level = level.max(value);
    }

    fn dimmer(&self) -> Option<f64> {
        self.levels.get("dimmer").copied()
    }

    /// Its colour, each primary 0 to 255, given what it emits
    /// ([`Self::emitted`]): on a part mixing subtractively (`subtractive`)
    /// with a flag of its own, the white its flags pass (a flag it lacks
    /// being `flag`'s); else its red, green and blue where it has any; else
    /// none where it has a white, or a warm white at `emitted`; its white
    /// added.
    fn color(&self, subtractive: bool, flag: impl Fn(&str) -> f64, emitted: f64) -> [f64; 3] {
        let level = |name: &str| self.levels.get(name).copied();
        let has = |name: &str| self.levels.contains_key(name);
        let white = level("white");
        let base = if subtractive && CMY_OF_RGB.iter().any(|(_, cmy)| has(cmy)) {
            CMY_OF_RGB.map(|(_, cmy)| FULL - level(cmy).unwrap_or_else(|| flag(cmy)))
        } else if CMY_OF_RGB.iter().any(|(rgb, _)| has(rgb)) {
            CMY_OF_RGB.map(|(rgb, _)| level(rgb).unwrap_or(0.0))
        } else if white.is_some() {
            [0.0; 3]
        } else {
            WARM.map(|c| c * emitted)
        };
        base.map(|c| (c + white.unwrap_or(0.0)).min(FULL))
    }

    /// Its brightest emitter -- on a fixture mixing subtractively
    /// (`subtractive`), the white its flags pass counting as one when it
    /// has a flag of its own: all of it while any flag is open, none with
    /// all three closed, a flag it lacks being `flag`'s -- or `None` when
    /// it has no emitter. A colour temperature is a setting, not light.
    fn emitted(&self, subtractive: bool, flag: impl Fn(&str) -> f64) -> Option<f64> {
        let is_flag = |name: &str| subtractive && CMY_OF_RGB.iter().any(|(_, cmy)| *cmy == name);
        let passed = self.levels.keys().any(|name| is_flag(name)).then(|| {
            FULL - CMY_OF_RGB
                .iter()
                .map(|(_, cmy)| self.levels.get(*cmy).copied().unwrap_or_else(|| flag(cmy)))
                .fold(FULL, f64::min)
        });
        self.levels
            .iter()
            .filter(|(name, _)| {
                !is_flag(name)
                    && !matches!(
                        name.as_str(),
                        "dimmer" | "ct" | "cto" | "ctb" | "color_temp"
                    )
            })
            .map(|(_, &level)| level)
            .chain(passed)
            .reduce(f64::max)
    }
}

/// Whether a fixture whose channels hold `values` (DMX values, 0 to 255)
/// is lit: the light it emits is above 0. Each part of it -- the fixture
/// itself, a section of a GDTF profile's (`red:2`), a cell or a ganged
/// mirror (`red#2`, `red:2#3`) -- emits its brightest emitter (red, green,
/// blue, white, amber, UV, …, or, mixing subtractively, the white its
/// cyan, magenta and yellow flags pass) times its dimmer. A part takes the
/// dimmer, and a flag, it lacks from the part above it: its section's,
/// else the fixture's. Whether it mixes subtractively is its section's
/// (by the section's own channels) where the section has red, green or
/// blue or a full CMY set of its own, else the fixture's (by its own channels, as the engine's
/// `has_cmy` decides it). A
/// part with a dimmer and no emitter that no other part takes its dimmer
/// from is a light of its own (a ring of white on a dimmer), lit by that
/// dimmer; the fixture's own dimmer is one only when nothing in the
/// fixture has an emitter. The fixture is lit when any part is. Nothing
/// else lights it: a strobe, a colour wheel, where it points, an optic or
/// a colour temperature. The rig is dark when no fixture is lit.
pub fn is_lit<'a>(values: impl IntoIterator<Item = (&'a str, f64)>) -> bool {
    lamps(values)
        .iter()
        .any(|lamp| lamp.dimmer * lamp.emitted > 0.0)
}

/// The colour a fixture whose channels hold `values` (DMX values, 0 to
/// 255) shows, red, green and blue each 0.0 to 1.0: what every view of
/// the rig (the web UI's stage views, the TUI) draws it in. Each part
/// that lights it by [`is_lit`]'s rule shows its colour times its dimmer,
/// and the fixture the brightest of them in each primary: a part's red,
/// green and blue -- on one mixing subtractively, the white its cyan,
/// magenta and yellow flags pass (full less each flag) -- with its white
/// added; a part with no colour of its own (a dimmer-only fixture, a
/// part lit by amber or UV alone) a warm white at the level it emits.
pub fn shown_color<'a>(values: impl IntoIterator<Item = (&'a str, f64)>) -> [f64; 3] {
    let mut shown = [0.0_f64; 3];
    for lamp in lamps(values) {
        for (shown, color) in shown.iter_mut().zip(lamp.color) {
            *shown = shown.max(color * lamp.dimmer / FULL / FULL);
        }
    }
    shown.map(|c| c.clamp(0.0, 1.0))
}

/// The warm white a part with no colour of its own shows, each primary
/// a share of its level.
const WARM: [f64; 3] = [1.0, 0.92, 0.8];

/// One part of a fixture that may light it ([`lamps`]): what it emits,
/// its colour (each primary 0 to 255) and the dimmer it takes.
struct Lamp {
    emitted: f64,
    color: [f64; 3],
    dimmer: f64,
}

/// The parts of a fixture whose channels hold `values` that may light it,
/// by [`is_lit`]'s rule: each part with an emitter, with the dimmer it
/// takes, and each dimmer that is a light of its own.
fn lamps<'a>(values: impl IntoIterator<Item = (&'a str, f64)>) -> Vec<Lamp> {
    use std::collections::{BTreeMap, BTreeSet};
    // Each part's light, by its section and its cell; the fixture's own
    // under ("", "").
    let mut parts: BTreeMap<(&str, &str), Light> = BTreeMap::new();
    for (name, value) in values {
        if !is_level_channel(name) {
            continue;
        }
        let (channel, cell) = name.split_once('#').unwrap_or((name, ""));
        let (attribute, section) = channel_parts(channel);
        parts
            .entry((section, cell))
            .or_default()
            .add(attribute, value);
    }
    // A part, then the parts above it it takes what it lacks from.
    let above = |key: (&'a str, &'a str)| [key, (key.0, ""), ("", "")];
    // Whether a scope (the fixture, a section) mixes subtractively, by its
    // own channels; `None` for one that does not decide for itself: with
    // no red, green or blue and no full CMY set of its own (a GDTF head
    // with two CMY units distils to `cyan:A` and `magenta:B`, `yellow:B`).
    let own_subtractive = |scope| {
        let levels = &parts.get(&scope)?.levels;
        let decides = CMY_OF_RGB.iter().any(|(rgb, _)| levels.contains_key(*rgb))
            || CMY_OF_RGB.iter().all(|(_, cmy)| levels.contains_key(*cmy));
        decides.then(|| mixes_subtractively(|c| levels.contains_key(c)))
    };
    // A part follows its section's decision, else the fixture's.
    let subtractive = |key: (&'a str, &'a str)| {
        own_subtractive((key.0, ""))
            .or_else(|| own_subtractive(("", "")))
            .unwrap_or(false)
    };
    // Each part's emitter and colour; `None` for a part with no emitter.
    let emitted: BTreeMap<(&str, &str), Option<(f64, [f64; 3])>> = parts
        .iter()
        .map(|(&key, light)| {
            let flag = |name: &str| {
                above(key)
                    .into_iter()
                    .find_map(|k| parts.get(&k)?.levels.get(name).copied())
                    // A flag the fixture lacks is open.
                    .unwrap_or(0.0)
            };
            let subtractive = subtractive(key);
            let emitted = light.emitted(subtractive, &flag);
            let color = emitted.map(|emitted| light.color(subtractive, &flag, emitted));
            (key, emitted.zip(color))
        })
        .collect();
    // The part a part takes its dimmer from.
    let dimmer_of = |key| {
        above(key)
            .into_iter()
            .find(|k| parts.get(k).and_then(Light::dimmer).is_some())
    };
    let mut used = BTreeSet::new();
    let mut lamps = Vec::new();
    for (&key, emitted) in &emitted {
        let Some((emitted, color)) = *emitted else {
            continue;
        };
        let dimmer = match dimmer_of(key) {
            Some(from) => {
                used.insert(from);
                parts[&from].dimmer().unwrap_or(FULL)
            }
            None => FULL,
        };
        lamps.push(Lamp {
            emitted,
            color,
            dimmer,
        });
    }
    let any_emitter = emitted.values().any(Option::is_some);
    // A dimmer no part with an emitter takes, a light of its own.
    lamps.extend(parts.iter().filter_map(|(key, light)| {
        let dimmer = light.dimmer()?;
        (emitted[key].is_none() && !used.contains(key) && (*key != ("", "") || !any_emitter)).then(
            || Lamp {
                emitted: FULL,
                color: WARM.map(|c| c * FULL),
                dimmer,
            },
        )
    }));
    lamps
}

#[cfg(test)]
mod tests {
    use super::{is_lit, shown_color};

    fn color(channels: &[(&str, f64)]) -> [u8; 3] {
        shown_color(channels.iter().copied()).map(|c| (c * 255.0).round() as u8)
    }

    /// A fixture shows its colour times its dimmer, white added; a dimmer
    /// alone a warm white; nothing that lights it, black -- dark exactly
    /// where `is_lit` has it dark.
    #[test]
    fn a_fixture_shows_its_colour_times_its_dimmer() {
        let cases: &[(&[(&str, f64)], [u8; 3])] = &[
            (&[("red", 255.0), ("dimmer", 0.0)], [0, 0, 0]),
            (&[("red", 255.0), ("dimmer", 51.0)], [51, 0, 0]),
            (&[("red", 255.0), ("green", 128.0)], [255, 128, 0]),
            (&[("dimmer", 255.0)], [255, 235, 204]),
            (&[("dimmer", 0.0)], [0, 0, 0]),
            (
                &[("red", 0.0), ("white", 100.0), ("dimmer", 255.0)],
                [100, 100, 100],
            ),
            (&[("white", 255.0)], [255, 255, 255]),
            (&[("red", 0.0), ("green", 0.0), ("blue", 0.0)], [0, 0, 0]),
            (&[("pan", 128.0), ("gobo", 9.0)], [0, 0, 0]),
            (&[], [0, 0, 0]),
            // Flags filtering white: cyan closed takes the red out.
            (
                &[
                    ("cyan", 255.0),
                    ("magenta", 0.0),
                    ("yellow", 0.0),
                    ("dimmer", 255.0),
                ],
                [0, 255, 255],
            ),
            (
                &[("cyan", 255.0), ("magenta", 255.0), ("yellow", 255.0)],
                [0, 0, 0],
            ),
            // Each section by its own dimmer, the brightest of them shown.
            (
                &[
                    ("dimmer:1", 255.0),
                    ("red:1", 0.0),
                    ("dimmer:2", 0.0),
                    ("red:2", 255.0),
                ],
                [0, 0, 0],
            ),
            (
                &[
                    ("dimmer:1", 255.0),
                    ("blue:1", 255.0),
                    ("dimmer:2", 255.0),
                    ("red:2", 128.0),
                ],
                [128, 0, 255],
            ),
        ];
        for (channels, expected) in cases {
            let shown = color(channels);
            assert_eq!(shown, *expected, "{channels:?}");
            assert_eq!(
                shown != [0, 0, 0],
                is_lit(channels.iter().copied()),
                "{channels:?}"
            );
        }
    }

    fn lit(channels: &[(&str, f64)]) -> bool {
        is_lit(channels.iter().copied())
    }

    #[test]
    fn a_fixture_is_lit_by_its_light_its_dimmer_times_its_brightest_emitter() {
        assert!(!lit(&[
            ("red", 0.0),
            ("green", 0.0),
            ("blue", 0.0),
            ("dimmer", 255.0)
        ]));
        assert!(!lit(&[("red", 255.0), ("dimmer", 0.0)]));
        assert!(lit(&[
            ("red", 255.0),
            ("green", 255.0),
            ("blue", 255.0),
            ("dimmer", 255.0)
        ]));
        assert!(lit(&[("dimmer", 5.0)]));
        assert!(lit(&[("red", 5.0), ("dimmer", 255.0)]));
        assert!(lit(&[("white", 3.0)]));
        // A ganged mirror's or a cell's level is the fixture's.
        assert!(lit(&[("red#2", 9.0), ("dimmer", 255.0)]));
    }

    /// Each section or cell is lit by its own dimmer and emitters (or the
    /// fixture's dimmer), not the brightest dimmer times the brightest
    /// emitter across them.
    #[test]
    fn sections_and_cells_are_lit_each_by_their_own() {
        assert!(!lit(&[
            ("dimmer:1", 255.0),
            ("red:1", 0.0),
            ("dimmer:2", 0.0),
            ("red:2", 255.0),
        ]));
        assert!(lit(&[
            ("dimmer:1", 255.0),
            ("red:1", 9.0),
            ("dimmer:2", 0.0),
            ("red:2", 255.0),
        ]));
        // A section with no dimmer of its own takes the fixture's.
        assert!(!lit(&[("dimmer", 0.0), ("red:1", 255.0)]));
        assert!(lit(&[("dimmer", 255.0), ("red:1", 0.0), ("red:2", 3.0)]));
        // A cell, likewise.
        assert!(!lit(&[("dimmer", 0.0), ("red#2", 255.0)]));
        // Sectioned dimmers alone: any open.
        assert!(lit(&[("dimmer:1", 0.0), ("dimmer:2", 4.0)]));
        // A cell in a section takes the section's dimmer before the
        // fixture's.
        assert!(!lit(&[
            ("dimmer", 255.0),
            ("dimmer:1", 0.0),
            ("red:1#2", 255.0)
        ]));
        assert!(lit(&[
            ("dimmer", 0.0),
            ("dimmer:1", 255.0),
            ("red:1#2", 255.0)
        ]));
        // A section that is a dimmer alone is a light of its own.
        assert!(lit(&[
            ("red", 0.0),
            ("dimmer", 0.0),
            ("dimmer:ring", 255.0)
        ]));
        // But not one its cells take their dimmer from, nor the fixture's
        // master dimmer over black colours.
        assert!(!lit(&[("dimmer:1", 255.0), ("red:1#2", 0.0)]));
        assert!(!lit(&[("dimmer", 255.0), ("red:1", 0.0)]));
    }

    /// Cyan, magenta and yellow are flags filtering white on a fixture that
    /// mixes subtractively (all three, no red, green or blue): open passes
    /// it, all three closed pass none. Elsewhere they are emitters.
    #[test]
    fn subtractive_flags_filter_white() {
        let cmy = |c, m, y| {
            [
                ("cyan", c),
                ("magenta", m),
                ("yellow", y),
                ("dimmer", 255.0),
            ]
        };
        assert!(lit(&cmy(0.0, 0.0, 0.0)));
        assert!(lit(&cmy(255.0, 0.0, 0.0)));
        assert!(!lit(&cmy(255.0, 255.0, 255.0)));
        // Under a closed dimmer, nothing passes.
        assert!(!lit(&[("cyan", 0.0), ("dimmer", 0.0)]));
        // With an emitter of its own beside them (white), the brighter.
        let mut closed_with_white = cmy(255.0, 255.0, 255.0).to_vec();
        closed_with_white.push(("white", 10.0));
        assert!(lit(&closed_with_white));
        let mut closed_dark_white = cmy(255.0, 255.0, 255.0).to_vec();
        closed_dark_white.push(("white", 0.0));
        assert!(!lit(&closed_dark_white));
    }

    /// A fine byte joins its coarse one (the brighter of the two), in
    /// whichever order they come: all three flags closed stays dark.
    /// Subtractive mixing is the fixture's, decided once; a part lacking a
    /// flag takes it from the part above it.
    #[test]
    fn a_part_takes_the_flags_it_lacks_from_above() {
        // A cell closing cyan, under the fixture's closed flags: dark.
        assert!(!lit(&[
            ("cyan", 255.0),
            ("magenta", 255.0),
            ("yellow", 255.0),
            ("cyan#2", 255.0),
            ("dimmer", 255.0),
        ]));
        // A cell opening cyan under them passes light.
        assert!(lit(&[
            ("cyan", 255.0),
            ("magenta", 255.0),
            ("yellow", 255.0),
            ("cyan#2", 0.0),
            ("dimmer", 255.0),
        ]));
        // A section closing all three of its own, under an open fixture.
        assert!(!lit(&[
            ("cyan", 0.0),
            ("magenta", 0.0),
            ("yellow", 0.0),
            ("dimmer", 0.0),
            ("cyan:2", 255.0),
            ("magenta:2", 255.0),
            ("yellow:2", 255.0),
            ("dimmer:2", 255.0),
        ]));
    }

    /// A section's red, green and blue are the section's: they do not turn
    /// the fixture's own CMY into emitters, as the engine's `has_cmy`,
    /// which reads the fixture's own channels, does not.
    #[test]
    fn a_sections_colour_decides_the_section_alone() {
        let rgb1 = [("red:1", 0.0), ("green:1", 0.0), ("blue:1", 0.0)];
        let with = |own: &[(&'static str, f64)], more: &[(&'static str, f64)]| {
            let mut all = own.to_vec();
            all.extend_from_slice(more);
            lit(&all)
        };
        let closed = [("cyan", 255.0), ("magenta", 255.0), ("yellow", 255.0)];
        let open = [("cyan", 0.0), ("magenta", 0.0), ("yellow", 0.0)];
        assert!(!with(
            &closed,
            &[rgb1[0], rgb1[1], rgb1[2], ("dimmer", 255.0)]
        ));
        assert!(with(&open, &rgb1));
        assert!(with(&open, &[("dimmer", 255.0), ("red:1", 0.0)]));
        // A section with a full CMY set of its own is subtractive alone.
        assert!(lit(&[
            ("cyan:1", 0.0),
            ("magenta:1", 255.0),
            ("yellow:1", 255.0),
            ("dimmer", 255.0),
        ]));
        assert!(!lit(&[
            ("cyan:1", 255.0),
            ("magenta:1", 255.0),
            ("yellow:1", 255.0),
            ("dimmer", 255.0),
        ]));
    }

    /// A section with part of a CMY set (no red, green or blue) follows the
    /// fixture: on a subtractive fixture its flags are flags, the rest
    /// borrowed from the fixture.
    #[test]
    fn a_section_with_part_of_a_cmy_set_follows_the_fixture() {
        let closed = [("cyan", 255.0), ("magenta", 255.0), ("yellow", 255.0)];
        let with = |more: &[(&'static str, f64)]| {
            let mut all = closed.to_vec();
            all.extend_from_slice(more);
            lit(&all)
        };
        assert!(!with(&[("dimmer", 255.0), ("cyan:1", 255.0)]));
        assert!(!with(&[
            ("cyan:A", 255.0),
            ("magenta:B", 255.0),
            ("yellow:B", 255.0),
            ("dimmer", 255.0),
        ]));
        assert!(with(&[("cyan:1", 0.0)]));
    }

    /// A sectioned fine byte is its section's, not a section of its own.
    #[test]
    fn a_sectioned_fine_byte_is_its_sections() {
        assert!(!lit(&[
            ("red", 0.0),
            ("dimmer", 0.0),
            ("red:2", 0.0),
            ("dimmer:2", 255.0),
            ("dimmer:2_fine", 255.0),
        ]));
        assert!(!lit(&[
            ("cyan:2", 255.0),
            ("cyan:2_fine", 0.0),
            ("magenta:2", 255.0),
            ("magenta:2_fine", 0.0),
            ("yellow:2", 255.0),
            ("yellow:2_fine", 0.0),
            ("dimmer", 255.0),
        ]));
        assert!(!lit(&[
            ("dimmer", 255.0),
            ("red:2", 0.0),
            ("red:2_fine", 255.0),
            ("dimmer:2", 0.0),
        ]));
    }

    #[test]
    fn a_fine_byte_does_not_replace_its_coarse_one() {
        let closed = [
            ("cyan", 255.0),
            ("cyan_fine", 0.0),
            ("magenta", 255.0),
            ("yellow", 255.0),
            ("dimmer", 255.0),
        ];
        assert!(!lit(&closed));
        let mut reversed = closed;
        reversed.reverse();
        assert!(!lit(&reversed));
    }

    #[test]
    fn cyan_magenta_and_yellow_are_emitters_unless_the_fixture_mixes_subtractively() {
        // Cyan alone, or two of three, are emitters: lit by their own level.
        assert!(lit(&[("cyan", 255.0), ("dimmer", 255.0)]));
        assert!(!lit(&[("cyan", 0.0), ("dimmer", 255.0)]));
        assert!(lit(&[
            ("cyan", 255.0),
            ("magenta", 255.0),
            ("dimmer", 255.0)
        ]));
        // Beside red, green and blue, cyan is one more emitter.
        assert!(!lit(&[
            ("red", 0.0),
            ("green", 0.0),
            ("blue", 0.0),
            ("cyan", 0.0),
            ("dimmer", 255.0),
        ]));
    }

    #[test]
    fn nothing_but_light_makes_a_fixture_lit() {
        assert!(!lit(&[("strobe", 200.0)]));
        assert!(!lit(&[("color_wheel", 30.0)]));
        assert!(!lit(&[("pan", 128.0), ("tilt", 90.0)]));
        assert!(!lit(&[("zoom", 128.0), ("cto", 255.0), ("dimmer", 0.0)]));
        assert!(!lit(&[]));
    }
}
