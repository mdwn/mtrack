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

//! Physical values and their resolution into DMX (venue-exchange design
//! §15.2).
//!
//! Every channel the engine has driven so far is a normalized 0..1 value
//! written to one byte. Movement breaks that: a show says *degrees*, and
//! what those degrees are in bytes depends on the fixture — its pan range,
//! whether it has a fine byte, which sub-range of the channel is the
//! function that moves it. So a [`PhysicalState`] rides beside the
//! normalized channels, in the units the show spoke, and is resolved here,
//! at the point DMX is produced, through the fixture's [`ChannelDef`]s.
//!
//! Physical intents merge by replacement: the highest layer wins per
//! parameter, and within a layer the later writer. There is no meaningful
//! sum of two pan angles, and no layer master scales one.

use std::collections::HashMap;

use super::types::EffectLayer;
use crate::lighting::types::{ChannelDef, PhysicalUnit};

/// The full 16-bit DMX scale a resolved value is expressed on.
const FULL_SCALE_16: f64 = 65535.0;

/// A physical value an effect asked for, in degrees, with the layer it
/// came from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Intent {
    pub degrees: f64,
    pub layer: EffectLayer,
}

/// The physical part of a fixture's state: what the show asked for in
/// physical units, not yet a byte.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PhysicalState {
    pub pan: Option<Intent>,
    pub tilt: Option<Intent>,
}

impl PhysicalState {
    /// Whether any physical intent is set.
    pub fn is_empty(&self) -> bool {
        self.pan.is_none() && self.tilt.is_none()
    }

    /// Sets an intent, keeping the higher layer if one is already set.
    pub fn set(&mut self, parameter: PhysicalParameter, intent: Intent) {
        let slot = match parameter {
            PhysicalParameter::Pan => &mut self.pan,
            PhysicalParameter::Tilt => &mut self.tilt,
        };
        *slot = Some(match *slot {
            Some(existing) if existing.layer > intent.layer => existing,
            _ => intent,
        });
    }

    /// Merges another state's intents in: replace-by-layer, later writer
    /// wins within a layer — the same order the channel blend runs in.
    pub fn blend_with(&mut self, other: &PhysicalState) {
        if let Some(pan) = other.pan {
            self.set(PhysicalParameter::Pan, pan);
        }
        if let Some(tilt) = other.tilt {
            self.set(PhysicalParameter::Tilt, tilt);
        }
    }
}

/// The physical parameters the engine resolves, and the channel each
/// lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhysicalParameter {
    Pan,
    Tilt,
}

impl PhysicalParameter {
    /// The canonical channel name (the distiller's, and the v1 map's).
    pub fn channel(self) -> &'static str {
        match self {
            PhysicalParameter::Pan => "pan",
            PhysicalParameter::Tilt => "tilt",
        }
    }

    /// The travel assumed for a channel with no physical range: enough
    /// that any real fixture's degrees land inside it, so a thin fixture
    /// definition still moves, less precisely. Lint says so (§15.5).
    pub fn fallback_range(self) -> (f64, f64) {
        match self {
            PhysicalParameter::Pan => (0.0, 540.0),
            PhysicalParameter::Tilt => (0.0, 270.0),
        }
    }
}

/// A resolved channel: which bytes to write, and whether the physical
/// value had to be clamped to reach the fixture's range.
#[derive(Clone, Debug, PartialEq)]
pub struct Resolved {
    /// `(1-based offset within the fixture, byte)`; one entry for an 8-bit
    /// channel, coarse then fine for a 16-bit one.
    pub bytes: Vec<(u16, u8)>,
    /// Whether the value lay outside the range it resolved through.
    pub clamped: bool,
}

/// Fans a 16-bit value out over a channel's bytes: coarse from the high
/// byte, fine from the low; just the high byte for an 8-bit channel.
/// Monotonic by construction (property-tested below), so a slow sweep
/// never steps the coarse byte backwards across a fine rollover.
pub fn fanout(def: &ChannelDef, value16: u16) -> Vec<(u16, u8)> {
    let [high, low] = value16.to_be_bytes();
    let mut out = Vec::with_capacity(2 + 2 * def.mirrors.len());
    for (coarse, fine) in std::iter::once((def.offset, def.fine)).chain(def.mirrors.iter().copied())
    {
        out.push((coarse, high));
        if let Some(fine) = fine {
            out.push((fine, low));
        }
    }
    out
}

/// Resolves a normalized 0..1 value onto a channel. An 8-bit channel gets
/// exactly the byte the engine always wrote (`(value × 255) as u8`); a
/// 16-bit channel gets the same coarse byte and a fine byte the engine
/// used to leave untouched.
pub fn resolve_normalized(def: &ChannelDef, value: f64) -> Vec<(u16, u8)> {
    match def.fine {
        None => {
            let byte = (value * 255.0) as u8;
            std::iter::once(def.offset)
                .chain(def.mirrors.iter().map(|(coarse, _)| *coarse))
                .map(|offset| (offset, byte))
                .collect()
        }
        Some(_) => fanout(def, (value.clamp(0.0, 1.0) * FULL_SCALE_16).round() as u16),
    }
}

/// The degree span a channel resolves over, and the DMX sub-range it maps
/// onto (inclusive, on the 8-bit coarse scale): the channel's own range
/// first, then a function carrying a degree range, then the parameter's
/// fallback travel. The one place this is decided, so the pointing math
/// and the byte resolution never disagree about a fixture's travel.
pub fn degree_span(def: &ChannelDef, parameter: PhysicalParameter) -> (f64, f64, u8, u8) {
    if let Some(range) = def.range.filter(|r| r.unit == PhysicalUnit::Degrees) {
        (range.from, range.to, 0u8, 255u8)
    } else if let Some(function) = def
        .functions
        .iter()
        .find(|f| f.physical.is_some_and(|p| p.unit == PhysicalUnit::Degrees))
    {
        let physical = function.physical.expect("filtered");
        (
            physical.from,
            physical.to,
            function.dmx_from,
            function.dmx_to,
        )
    } else {
        let (from, to) = parameter.fallback_range();
        (from, to, 0, 255)
    }
}

/// Resolves degrees onto a pan or tilt channel through [`degree_span`]:
/// linear interpolation over the span's DMX sub-range, clamped and
/// flagged when the value lies outside.
pub fn resolve_degrees(def: &ChannelDef, parameter: PhysicalParameter, degrees: f64) -> Resolved {
    let (from, to, dmx_lo, dmx_hi) = degree_span(def, parameter);

    let (low, high) = if from <= to { (from, to) } else { (to, from) };
    let clamped = degrees < low || degrees > high;
    let bounded = degrees.clamp(low, high);
    let fraction = if (to - from).abs() < f64::EPSILON {
        0.0
    } else {
        (bounded - from) / (to - from)
    };
    // The DMX sub-range on the 16-bit scale: a function over 64..255 spans
    // 64·257 .. 255·257, so its coarse byte stays inside the function.
    let lo16 = f64::from(dmx_lo) * 257.0;
    let hi16 = f64::from(dmx_hi) * 257.0;
    let value16 = (lo16 + fraction * (hi16 - lo16))
        .round()
        .clamp(0.0, FULL_SCALE_16) as u16;
    Resolved {
        bytes: fanout(def, value16),
        clamped,
    }
}

/// Resolves a fixture's physical intents against its channel definitions:
/// `(channel name, resolved)` for each parameter both sides know about.
pub fn resolve_physical(
    physical: &PhysicalState,
    channel_defs: &HashMap<String, ChannelDef>,
) -> Vec<(&'static str, Resolved)> {
    let mut out = Vec::new();
    for (parameter, intent) in [
        (PhysicalParameter::Pan, physical.pan),
        (PhysicalParameter::Tilt, physical.tilt),
    ] {
        let Some(intent) = intent else { continue };
        let Some(def) = channel_defs.get(parameter.channel()) else {
            continue;
        };
        out.push((
            parameter.channel(),
            resolve_degrees(def, parameter, intent.degrees),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::types::{ChannelFunction, PhysicalRange};

    fn def16(offset: u16, fine: u16) -> ChannelDef {
        ChannelDef {
            offset,
            fine: Some(fine),
            ..ChannelDef::at(offset)
        }
    }

    fn degrees(from: f64, to: f64) -> PhysicalRange {
        PhysicalRange {
            from,
            to,
            unit: PhysicalUnit::Degrees,
        }
    }

    #[test]
    fn eight_bit_normalized_is_the_byte_the_engine_always_wrote() {
        let def = ChannelDef::at(3);
        for value in [0.0, 0.25, 0.5, 0.999, 1.0] {
            assert_eq!(
                resolve_normalized(&def, value),
                vec![(3, (value * 255.0) as u8)]
            );
        }
    }

    #[test]
    fn sixteen_bit_normalized_keeps_the_coarse_byte_and_adds_the_fine() {
        let def = def16(2, 3);
        assert_eq!(resolve_normalized(&def, 0.0), vec![(2, 0), (3, 0)]);
        assert_eq!(resolve_normalized(&def, 1.0), vec![(2, 255), (3, 255)]);
        let half = resolve_normalized(&def, 0.5);
        assert_eq!(half[0], (2, 128), "32768 = 0x8000");
        assert_eq!(half[1].0, 3);
    }

    #[test]
    fn fanout_is_monotonic_across_every_fine_rollover() {
        let def = def16(1, 2);
        let mut previous = (0u8, 0u8);
        for value in 0..=u16::MAX {
            let bytes = fanout(&def, value);
            let pair = (bytes[0].1, bytes[1].1);
            assert!(pair >= previous, "{value}: {pair:?} after {previous:?}");
            previous = pair;
        }
    }

    #[test]
    fn a_ganged_channel_writes_every_mirror() {
        let mut def = ChannelDef::at(1);
        def.mirrors = vec![(4, None), (7, None)];
        assert_eq!(
            resolve_normalized(&def, 1.0),
            vec![(1, 255), (4, 255), (7, 255)]
        );
        let mut wide = def16(1, 2);
        wide.mirrors = vec![(3, Some(4))];
        assert_eq!(
            fanout(&wide, 0x1234),
            vec![(1, 0x12), (2, 0x34), (3, 0x12), (4, 0x34)]
        );
    }

    #[test]
    fn degrees_resolve_through_the_channel_range() {
        let mut def = def16(1, 2);
        def.range = Some(degrees(-270.0, 270.0));
        let center = resolve_degrees(&def, PhysicalParameter::Pan, 0.0);
        assert!(!center.clamped);
        assert_eq!(
            center.bytes[0],
            (1, 0x80),
            "0° is mid-travel: 32768 = 0x8000"
        );
        assert_eq!(center.bytes[1], (2, 0x00));
        let left = resolve_degrees(&def, PhysicalParameter::Pan, -270.0);
        assert_eq!(left.bytes, vec![(1, 0), (2, 0)]);
        let right = resolve_degrees(&def, PhysicalParameter::Pan, 270.0);
        assert_eq!(right.bytes, vec![(1, 255), (2, 255)]);
        // Out of range clamps and says so.
        let beyond = resolve_degrees(&def, PhysicalParameter::Pan, 300.0);
        assert!(beyond.clamped);
        assert_eq!(beyond.bytes, right.bytes);
    }

    #[test]
    fn a_descending_range_maps_the_other_way() {
        let mut def = ChannelDef::at(4);
        def.range = Some(degrees(135.0, -135.0));
        let up = resolve_degrees(&def, PhysicalParameter::Tilt, 135.0);
        assert_eq!(up.bytes, vec![(4, 0)]);
        let down = resolve_degrees(&def, PhysicalParameter::Tilt, -135.0);
        assert_eq!(down.bytes, vec![(4, 255)]);
        assert!(!resolve_degrees(&def, PhysicalParameter::Tilt, 0.0).clamped);
    }

    #[test]
    fn a_function_sub_range_keeps_the_coarse_byte_inside_the_function() {
        let mut def = ChannelDef::at(1);
        def.functions.push(ChannelFunction {
            name: "pan".to_string(),
            dmx_from: 64,
            dmx_to: 200,
            physical: Some(degrees(0.0, 360.0)),
        });
        let start = resolve_degrees(&def, PhysicalParameter::Pan, 0.0);
        assert_eq!(start.bytes, vec![(1, 64)]);
        let end = resolve_degrees(&def, PhysicalParameter::Pan, 360.0);
        assert_eq!(end.bytes, vec![(1, 200)]);
        let mid = resolve_degrees(&def, PhysicalParameter::Pan, 180.0);
        assert_eq!(mid.bytes, vec![(1, 132)]);
    }

    #[test]
    fn a_channel_with_no_range_uses_the_fallback_travel() {
        let def = ChannelDef::at(2);
        let full = resolve_degrees(&def, PhysicalParameter::Pan, 540.0);
        assert_eq!(full.bytes, vec![(2, 255)]);
        assert!(!full.clamped);
        let half = resolve_degrees(&def, PhysicalParameter::Tilt, 135.0);
        assert_eq!(half.bytes, vec![(2, 128)]);
        assert!(resolve_degrees(&def, PhysicalParameter::Pan, -10.0).clamped);
    }

    #[test]
    fn intents_replace_by_layer_and_later_writer_within_a_layer() {
        let mut state = PhysicalState::default();
        state.set(
            PhysicalParameter::Pan,
            Intent {
                degrees: 10.0,
                layer: EffectLayer::Foreground,
            },
        );
        let mut other = PhysicalState::default();
        other.set(
            PhysicalParameter::Pan,
            Intent {
                degrees: 90.0,
                layer: EffectLayer::Background,
            },
        );
        other.set(
            PhysicalParameter::Tilt,
            Intent {
                degrees: -20.0,
                layer: EffectLayer::Background,
            },
        );
        state.blend_with(&other);
        assert_eq!(state.pan.unwrap().degrees, 10.0, "the foreground pan holds");
        assert_eq!(
            state.tilt.unwrap().degrees,
            -20.0,
            "an unset parameter takes any layer"
        );

        let later = PhysicalState {
            pan: Some(Intent {
                degrees: 45.0,
                layer: EffectLayer::Foreground,
            }),
            tilt: None,
        };
        state.blend_with(&later);
        assert_eq!(
            state.pan.unwrap().degrees,
            45.0,
            "same layer: later writer wins"
        );
    }

    #[test]
    fn resolution_skips_parameters_the_fixture_has_no_channel_for() {
        let mut defs = HashMap::new();
        defs.insert("pan".to_string(), ChannelDef::at(1));
        let physical = PhysicalState {
            pan: Some(Intent {
                degrees: 270.0,
                layer: EffectLayer::Background,
            }),
            tilt: Some(Intent {
                degrees: 0.0,
                layer: EffectLayer::Background,
            }),
        };
        let resolved = resolve_physical(&physical, &defs);
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].0, "pan");
        assert_eq!(resolved[0].1.bytes, vec![(1, 128)]);
    }
}
