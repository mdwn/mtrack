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

/// How a channel crosses a crossfade between two engines' frames
/// (the engine's `crossfade_commands`).
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
    let base = channel.split(':').next().unwrap_or(channel);
    base.strip_suffix("_fine").unwrap_or(base)
}

/// Whether a channel carries where a head points -- pan or tilt, a section
/// (`pan:1`) or fine byte of either -- rather than what it shows.
pub(crate) fn is_position_channel(channel: &str) -> bool {
    matches!(CrossKind::of(channel), CrossKind::Position)
}

impl CrossKind {
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

/// What one fixture emits, read off its level channels: its dimmer, its
/// brightest additive emitter, and its subtractive flags (cyan, magenta,
/// yellow), which filter its white.
#[derive(Default)]
struct Light {
    dimmer: Option<f64>,
    emitted: Option<f64>,
    /// Each flag's value; one the fixture lacks stays open.
    flags: Option<[f64; 3]>,
}

impl Light {
    /// Takes in a level channel's value, by its base name.
    fn add(&mut self, base: &str, value: f64) {
        let max = |level: &mut Option<f64>| *level = Some(level.map_or(value, |l| l.max(value)));
        let flag =
            |i: usize, flags: &mut Option<[f64; 3]>| flags.get_or_insert([0.0; 3])[i] = value;
        match base {
            "dimmer" => max(&mut self.dimmer),
            "cyan" => flag(0, &mut self.flags),
            "magenta" => flag(1, &mut self.flags),
            "yellow" => flag(2, &mut self.flags),
            // A colour temperature is a setting, not light.
            "ct" | "cto" | "ctb" | "color_temp" => {}
            _ => max(&mut self.emitted),
        }
    }

    /// Its brightest emitter -- the white its flags pass counting as one,
    /// all of it while any flag is open and none with all three closed --
    /// or `None` when it has no emitter.
    fn emitted(&self) -> Option<f64> {
        let passed = self
            .flags
            .map(|f| FULL - f.into_iter().fold(FULL, f64::min));
        match (self.emitted, passed) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }
}

/// Whether a fixture whose channels hold `values` (DMX values, 0 to 255)
/// is lit: the light it emits is above 0. A part of it -- the fixture
/// itself, a section of a GDTF profile's (`red:2`), a cell or a ganged
/// mirror (`red#2`) -- emits its brightest emitter (red, green, blue,
/// white, amber, UV, …, or the white its cyan, magenta and yellow flags
/// pass) times its dimmer, or the fixture's where it has none of its own;
/// the fixture is lit when any part with an emitter is. A fixture with no
/// emitter at all follows its dimmers. Nothing else lights it: a strobe,
/// a colour wheel, where it points, an optic or a colour temperature. The
/// rig is dark when no fixture is lit.
pub fn is_lit<'a>(values: impl IntoIterator<Item = (&'a str, f64)>) -> bool {
    // Each part's light, by what its channels name it after the attribute
    // (`:section`, `#cell`); the fixture's own under "#".
    let mut parts: std::collections::BTreeMap<String, Light> = Default::default();
    for (name, value) in values {
        if !is_level_channel(name) {
            continue;
        }
        let (channel, cell) = name.split_once('#').unwrap_or((name, ""));
        let section = channel.split_once(':').map_or("", |(_, section)| section);
        parts
            .entry(format!("{section}#{cell}"))
            .or_default()
            .add(base_channel(channel), value);
    }
    let own_dimmer = parts.get("#").and_then(|light| light.dimmer);
    let emitting: Vec<(&Light, f64)> = parts
        .values()
        .filter_map(|light| Some((light, light.emitted()?)))
        .collect();
    if emitting.is_empty() {
        return parts
            .values()
            .any(|light| light.dimmer.is_some_and(|d| d > 0.0));
    }
    emitting
        .iter()
        .any(|(light, emitted)| light.dimmer.or(own_dimmer).unwrap_or(FULL) * emitted > 0.0)
}

#[cfg(test)]
mod tests {
    use super::is_lit;

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
    }

    /// Cyan, magenta and yellow are flags filtering white: open passes it,
    /// all three closed pass none.
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
        // With emitters of its own too, the brighter of the two.
        let mut closed_with_red = cmy(255.0, 255.0, 255.0).to_vec();
        closed_with_red.push(("red", 10.0));
        assert!(lit(&closed_with_red));
        let mut closed_dark_red = cmy(255.0, 255.0, 255.0).to_vec();
        closed_dark_red.push(("red", 0.0));
        assert!(!lit(&closed_dark_red));
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
