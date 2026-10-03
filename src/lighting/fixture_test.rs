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

//! Testing one fixture by hand (the fixture page's "Test this fixture"):
//! what a colour, a level, a strobe rate or a pose is in DMX for a fixture
//! of a given type and mode at a given address.
//!
//! Nothing here maps a value to a byte. The controls become the effects a
//! show would start — a static look (colour, dimmer, extra colour levels),
//! a strobe at a rate, a move to angles — on a one-fixture effects engine
//! holding the fixture exactly as a venue registers it
//! ([`super::system::fixture_info_for`]), and one frame of that engine is
//! the answer. So what the test sends is what a show would send: dimmer-less
//! fixtures fold the level into their colour, strobe rates land in the strobe
//! function's range, pan and tilt in degrees resolve through the channel's
//! physical range and fine byte (design §15.2), CMY fixtures get the
//! complement. Raw per-channel values then override single bytes.

use std::collections::{BTreeMap, HashMap};
use std::time::Duration;

use super::effects::{
    degree_span, Color, Easing, EffectInstance, EffectLayer, EffectType, FixtureInfo, MoveTarget,
    PhysicalParameter, TempoAwareFrequency,
};
use super::engine::EffectEngine;
use super::types::ChannelDef;

/// Colour channels a test offers as their own levels (beyond the colour
/// itself), when the mode has them.
const LEVEL_CHANNELS: &[&str] = &[
    "white",
    "warm_white",
    "cool_white",
    "amber",
    "uv",
    "lime",
    "cyan",
    "magenta",
    "yellow",
    "indigo",
];

/// What the person asked the fixture to do. Absent means "not touched".
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TestControls {
    /// Additive colour, 0..255 each.
    pub color: Option<[u8; 3]>,
    /// Overall level, 0..1.
    pub dimmer: Option<f64>,
    /// Strobe: `Some(None)` is off, `Some(Some(hz))` a rate.
    pub strobe: Option<Option<f64>>,
    /// Pan and tilt, in degrees.
    pub pan: Option<f64>,
    pub tilt: Option<f64>,
    /// Further channel levels by channel name (white, amber, …), 0..1.
    pub levels: BTreeMap<String, f64>,
}

/// What a mode lets a test do.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct ControlSupport {
    /// The mode mixes colour (RGB or CMY).
    pub color: bool,
    /// `channel` (a dimmer channel), `folded` (none: the level scales the
    /// colour), or none at all.
    pub dimmer: Option<&'static str>,
    /// The strobe's rate range in Hz, when the mode has a strobe channel.
    pub strobe: Option<HzRange>,
    pub pan: Option<DegreeRange>,
    pub tilt: Option<DegreeRange>,
    /// Extra colour levels the mode has.
    pub levels: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct HzRange {
    pub min_hz: f64,
    pub max_hz: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize)]
pub struct DegreeRange {
    pub min: f64,
    pub max: f64,
}

/// One channel of the fixture as a test frame sets it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct FrameChannel {
    /// 1-based offset within the fixture.
    pub offset: u16,
    /// The DMX address it lands on.
    pub address: u16,
    pub name: String,
    pub value: u8,
    /// Set by a raw value rather than a control.
    pub raw: bool,
}

/// What the mode can do, read from the fixture as the engine sees it.
pub fn control_support(info: &FixtureInfo) -> ControlSupport {
    let channels = &info.channels;
    let color = info.takes_channel("red") && info.takes_channel("green");
    let dimmer = if channels.contains_key("dimmer") {
        Some("channel")
    } else if color {
        Some("folded")
    } else {
        None
    };
    let strobe = channels.contains_key("strobe").then(|| HzRange {
        min_hz: info.min_strobe_frequency.unwrap_or(0.0),
        max_hz: info.max_strobe_frequency.unwrap_or(20.0),
    });
    let range = |parameter: PhysicalParameter| {
        info.channel_defs.get(parameter.channel()).map(|def| {
            let (min, max, _, _) = degree_span(def, parameter);
            DegreeRange { min, max }
        })
    };
    let levels = LEVEL_CHANNELS
        .iter()
        .filter(|name| channels.contains_key(**name))
        // A CMY fixture's flags are its colour, not extra levels.
        .filter(|name| {
            !(color
                && !channels.contains_key("red")
                && ["cyan", "magenta", "yellow"].contains(name))
        })
        .map(|name| name.to_string())
        .collect();
    ControlSupport {
        color,
        dimmer,
        strobe,
        pan: range(PhysicalParameter::Pan),
        tilt: range(PhysicalParameter::Tilt),
        levels,
    }
}

/// The controls given that this mode has nothing to apply them to, by the
/// name the request used.
pub fn unsupported(support: &ControlSupport, controls: &TestControls) -> Vec<String> {
    let mut out = Vec::new();
    if controls.color.is_some() && !support.color {
        out.push("color".to_string());
    }
    if controls.dimmer.is_some() && support.dimmer.is_none() {
        out.push("dimmer".to_string());
    }
    if matches!(controls.strobe, Some(Some(_))) && support.strobe.is_none() {
        out.push("strobe".to_string());
    }
    if controls.pan.is_some() && support.pan.is_none() {
        out.push("pan".to_string());
    }
    if controls.tilt.is_some() && support.tilt.is_none() {
        out.push("tilt".to_string());
    }
    for name in controls.levels.keys() {
        if !support.levels.contains(name) {
            out.push(name.clone());
        }
    }
    out
}

/// Every byte of the fixture, by 1-based offset, with the name a person
/// would recognise: the channel, `<name> fine` for a fine byte, and
/// `<cell>: <name>` for a pixel cell's own channels.
pub fn channel_names(info: &FixtureInfo) -> BTreeMap<u16, String> {
    let mut names = BTreeMap::new();
    let mut put = |name: &str, def: &ChannelDef| {
        names.entry(def.offset).or_insert_with(|| name.to_string());
        if let Some(fine) = def.fine {
            names.entry(fine).or_insert_with(|| format!("{name} fine"));
        }
        for (i, (coarse, fine)) in def.mirrors.iter().enumerate() {
            names
                .entry(*coarse)
                .or_insert_with(|| format!("{name} ({})", i + 2));
            if let Some(fine) = fine {
                names
                    .entry(*fine)
                    .or_insert_with(|| format!("{name} fine ({})", i + 2));
            }
        }
    };
    for cell in &info.cells {
        for (name, def) in &cell.channels {
            put(&format!("{}: {name}", cell.name), def);
        }
    }
    for (name, def) in &info.channel_defs {
        put(name, def);
    }
    for (name, &offset) in &info.channels {
        names.entry(offset).or_insert_with(|| name.clone());
    }
    names
}

/// The fixture's span, in channels: its highest byte offset.
pub fn footprint(info: &FixtureInfo) -> u16 {
    channel_names(info).keys().next_back().copied().unwrap_or(0)
}

/// The fixture's whole frame for `controls`, then `raw` (by offset) over
/// it: every byte of its span, those nothing sets at 0.
pub fn resolve_frame(
    info: &FixtureInfo,
    controls: &TestControls,
    raw: &BTreeMap<u16, u8>,
) -> Vec<FrameChannel> {
    let values = resolve_controls(info, controls);
    channel_names(info)
        .into_iter()
        .map(|(offset, name)| {
            let (value, raw) = match raw.get(&offset) {
                Some(&v) => (v, true),
                None => (values.get(&offset).copied().unwrap_or(0), false),
            };
            FrameChannel {
                offset,
                address: info.address + offset - 1,
                name,
                value,
                raw,
            }
        })
        .collect()
}

/// The bytes the controls come to, by 1-based offset, from one frame of a
/// one-fixture effects engine.
fn resolve_controls(info: &FixtureInfo, controls: &TestControls) -> HashMap<u16, u8> {
    let support = control_support(info);
    let mut fixture = info.clone();
    // A test snaps to its pose: movement limits slew a show's moves over
    // frames, and a test is one frame.
    fixture.movement = Default::default();
    let name = fixture.name.clone();
    let mut engine = EffectEngine::new();
    engine.register_fixture(fixture);
    let hold = Duration::from_secs(3600);

    let mut parameters: HashMap<String, f64> = HashMap::new();
    if let Some([r, g, b]) = controls.color.filter(|_| support.color) {
        let color = Color::new(r, g, b);
        parameters.insert("red".into(), f64::from(color.r) / 255.0);
        parameters.insert("green".into(), f64::from(color.g) / 255.0);
        parameters.insert("blue".into(), f64::from(color.b) / 255.0);
    }
    if let Some(level) = controls.dimmer.filter(|_| support.dimmer.is_some()) {
        parameters.insert("dimmer".into(), level.clamp(0.0, 1.0));
    }
    for (channel, level) in &controls.levels {
        if support.levels.contains(channel) {
            parameters.insert(channel.clone(), level.clamp(0.0, 1.0));
        }
    }
    let mut start = |id: &str, effect_type, layer| {
        let mut effect = EffectInstance::new(
            id.to_string(),
            effect_type,
            vec![name.clone()],
            None,
            None,
            None,
        );
        effect.layer = layer;
        // An effect the fixture cannot take is simply not applied.
        let _ = engine.start_effect(effect);
    };
    if !parameters.is_empty() {
        start(
            "test-look",
            EffectType::Static {
                parameters,
                duration: hold,
            },
            EffectLayer::Background,
        );
    }
    if let Some(rate) = controls.strobe.filter(|_| support.strobe.is_some()) {
        start(
            "test-strobe",
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(rate.unwrap_or(0.0).max(0.0)),
                duration: hold,
            },
            EffectLayer::Foreground,
        );
    }
    let pan = controls.pan.filter(|_| support.pan.is_some());
    let tilt = controls.tilt.filter(|_| support.tilt.is_some());
    if pan.is_some() || tilt.is_some() {
        start(
            "test-pose",
            EffectType::Move {
                to: MoveTarget::Angles { pan, tilt },
                from: None,
                easing: Easing::Linear,
                duration: Duration::ZERO,
            },
            EffectLayer::Background,
        );
    }

    let mut out = HashMap::new();
    if let Ok(commands) = engine.update(Duration::from_millis(1), None) {
        for command in commands {
            if command.universe == info.universe && command.channel >= info.address {
                out.insert(command.channel - info.address + 1, command.value);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::types::{ChannelFunction, PhysicalRange, PhysicalUnit};

    fn fixture(channels: &[(&str, u16)]) -> FixtureInfo {
        FixtureInfo::new(
            "f".into(),
            1,
            10,
            "T".into(),
            channels.iter().map(|(n, o)| (n.to_string(), *o)).collect(),
            Some(25.0),
        )
    }

    fn values(frame: &[FrameChannel]) -> Vec<(String, u8)> {
        frame.iter().map(|c| (c.name.clone(), c.value)).collect()
    }

    fn white(dimmer: f64) -> TestControls {
        TestControls {
            color: Some([255, 255, 255]),
            dimmer: Some(dimmer),
            ..Default::default()
        }
    }

    #[test]
    fn rgb_with_a_dimmer_channel_puts_the_level_on_the_dimmer() {
        let info = fixture(&[("dimmer", 1), ("red", 2), ("green", 3), ("blue", 4)]);
        let frame = resolve_frame(
            &info,
            &TestControls {
                color: Some([255, 0, 128]),
                dimmer: Some(0.5),
                ..Default::default()
            },
            &BTreeMap::new(),
        );
        // The dimmer byte is the engine's own for 0.5 (it truncates), as
        // `resolve_normalized` writes it for any show.
        let half = crate::lighting::effects::resolve_normalized(&ChannelDef::at(1), 0.5)[0].1;
        assert_eq!(half, 127);
        assert_eq!(
            values(&frame),
            vec![
                ("dimmer".into(), half),
                ("red".into(), 255),
                ("green".into(), 0),
                ("blue".into(), 128)
            ]
        );
        // Addresses are the fixture's.
        assert_eq!(frame[0].address, 10);
        assert_eq!(frame[3].address, 13);
    }

    #[test]
    fn rgb_without_a_dimmer_folds_the_level_into_the_colour() {
        let info = fixture(&[("red", 1), ("green", 2), ("blue", 3), ("strobe", 4)]);
        assert_eq!(control_support(&info).dimmer, Some("folded"));
        let frame = resolve_frame(&info, &white(0.5), &BTreeMap::new());
        let v = values(&frame);
        // The engine's own rounding of 0.5 × 255, on every primary.
        let half = v[0].1;
        assert!((127..=128).contains(&half), "{v:?}");
        assert_eq!(v[1].1, half);
        assert_eq!(v[2].1, half);
        assert_eq!(v[3], ("strobe".into(), 0));
    }

    #[test]
    fn full_white_is_full_on_every_primary() {
        let info = fixture(&[("red", 1), ("green", 2), ("blue", 3)]);
        let frame = resolve_frame(&info, &white(1.0), &BTreeMap::new());
        assert!(frame.iter().all(|c| c.value == 255), "{frame:?}");
    }

    #[test]
    fn a_strobe_rate_lands_in_the_strobe_function_s_range() {
        let mut info = fixture(&[("red", 1), ("green", 2), ("blue", 3), ("strobe", 4)]);
        info.min_strobe_frequency = Some(0.4);
        info.max_strobe_frequency = Some(25.0);
        info.strobe_dmx_offset = Some(7);
        let at = |hz: Option<f64>| {
            let frame = resolve_frame(
                &info,
                &TestControls {
                    strobe: Some(hz),
                    ..Default::default()
                },
                &BTreeMap::new(),
            );
            frame[3].value
        };
        // Off is the closed-to-open bottom of the channel; the slowest rate
        // starts at the function's own start, the fastest is full.
        assert_eq!(at(None), 0);
        assert_eq!(at(Some(0.4)), 7);
        assert_eq!(at(Some(25.0)), 255);
        let mid = at(Some(5.0));
        assert!(mid > 7 && mid < 255, "{mid}");
    }

    #[test]
    fn pan_in_degrees_resolves_through_the_range_and_fine_byte() {
        let mut info = fixture(&[("pan", 1), ("pan_fine", 2), ("tilt", 3)]);
        let mut defs = info.channel_defs.clone();
        defs.insert(
            "pan".into(),
            ChannelDef {
                offset: 1,
                fine: Some(2),
                range: Some(PhysicalRange {
                    from: -270.0,
                    to: 270.0,
                    unit: PhysicalUnit::Degrees,
                }),
                functions: Vec::<ChannelFunction>::new(),
                mirrors: Vec::new(),
            },
        );
        defs.remove("pan_fine");
        info = info.with_channel_defs(defs);
        let frame = resolve_frame(
            &info,
            &TestControls {
                pan: Some(45.0),
                ..Default::default()
            },
            &BTreeMap::new(),
        );
        // 45° of −270..270 is 315/540 of 65535 = 38229 = 0x9555.
        let pan: Vec<_> = frame.iter().filter(|c| c.offset <= 2).collect();
        assert_eq!(pan[0].name, "pan");
        assert_eq!(pan[1].name, "pan fine");
        assert_eq!(
            u16::from(pan[0].value) << 8 | u16::from(pan[1].value),
            38229
        );
    }

    #[test]
    fn raw_values_override_single_bytes() {
        let info = fixture(&[("red", 1), ("green", 2), ("blue", 3)]);
        let raw = BTreeMap::from([(2, 42)]);
        let frame = resolve_frame(&info, &white(1.0), &raw);
        assert_eq!(frame[1].value, 42);
        assert!(frame[1].raw);
        assert!(!frame[0].raw);
        assert_eq!(frame[0].value, 255);
    }

    #[test]
    fn what_a_mode_cannot_do_is_named() {
        let info = fixture(&[("red", 1), ("green", 2), ("blue", 3)]);
        let support = control_support(&info);
        assert!(support.strobe.is_none());
        let asked = TestControls {
            strobe: Some(Some(5.0)),
            pan: Some(0.0),
            levels: BTreeMap::from([("white".to_string(), 1.0)]),
            ..white(1.0)
        };
        assert_eq!(
            unsupported(&support, &asked),
            vec!["strobe", "pan", "white"]
        );
        // Off is never a request the mode has to answer.
        let off = TestControls {
            strobe: Some(None),
            ..Default::default()
        };
        assert!(unsupported(&support, &off).is_empty());
    }

    #[test]
    fn untouched_channels_are_zero_and_named() {
        let info = fixture(&[("dimmer", 1), ("zoom", 2)]);
        let frame = resolve_frame(&info, &TestControls::default(), &BTreeMap::new());
        assert_eq!(
            values(&frame),
            vec![("dimmer".into(), 0), ("zoom".into(), 0)]
        );
        assert_eq!(footprint(&info), 2);
    }
}
