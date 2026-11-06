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

//! Fixture types, fixtures and venues as plain data: what the files say and
//! the web UI is sent. Light, so the UI's WebAssembly module compiles it as
//! it is; the rules that read and write it are `types`' and the engine's.

use std::collections::{BTreeMap, HashMap};

use serde::{Deserialize, Serialize};

use super::geometry::Vec3;

/// The widest beam angle a venue fixture is given, in degrees.
pub const MAX_BEAM_ANGLE: f64 = 180.0;

/// Whether `degrees` is a venue fixture's beam angle: greater than 0 and
/// at most [`MAX_BEAM_ANGLE`]. The one check, for the venue file, the API
/// and the web UI.
pub fn beam_angle_valid(degrees: f64) -> bool {
    degrees > 0.0 && degrees <= MAX_BEAM_ANGLE
}

/// The physical unit a channel value or range is expressed in.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhysicalUnit {
    /// Degrees (pan/tilt angles).
    Degrees,
    /// Hertz (strobe frequencies).
    Hertz,
}

/// A physical value range (e.g. -270°..270°, 0.3 Hz..25 Hz).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhysicalRange {
    /// The value at the low end of the DMX range.
    pub from: f64,
    /// The value at the high end of the DMX range.
    pub to: f64,
    /// The unit both endpoints are expressed in.
    pub unit: PhysicalUnit,
}

/// A function of a channel: a named DMX sub-range, optionally mapped to a
/// physical value range (e.g. "strobe" over DMX 64..255 as 0.3 Hz..25 Hz).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelFunction {
    /// The function name.
    pub name: String,
    /// First DMX value of the function's range.
    pub dmx_from: u8,
    /// Last DMX value of the function's range.
    pub dmx_to: u8,
    /// The physical values the DMX range maps onto, if any.
    pub physical: Option<PhysicalRange>,
    /// The function's own table of steps (a GDTF's ChannelSets), in DMX
    /// order, when the file states one: what `strobe_curve: declared`
    /// follows. Empty when only the endpoints are known.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<FunctionStep>,
}

/// One step of a function's table: DMX `dmx_from..=dmx_to` runs linearly
/// over the physical values `from..to` (equal when the step holds a value).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct FunctionStep {
    pub dmx_from: u8,
    pub dmx_to: u8,
    pub from: f64,
    pub to: f64,
}

/// One cell of a pixel fixture (design §17.2): its own channel offsets and
/// where it sits in the fixture's frame. The fixture-level channels stay
/// ganged (the first cell's, mirroring the rest); a show that says
/// `per: cell` reaches these.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Cell {
    /// The cell's name — the GDTF geometry name, or the `cell` block's.
    pub name: String,
    /// The cell's channels by canonical name, offsets without mirrors.
    pub channels: HashMap<String, ChannelDef>,
    /// The cell's position in the fixture's frame, meters.
    pub offset: Vec3,
}

/// A structured channel definition.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChannelDef {
    /// 1-based offset of the (coarse) byte within the fixture.
    pub offset: u16,
    /// 1-based offset of the fine byte for 16-bit channels, if any.
    pub fine: Option<u16>,
    /// The physical range the full DMX range maps onto, if any.
    pub range: Option<PhysicalRange>,
    /// DMX sub-range functions of this channel.
    pub functions: Vec<ChannelFunction>,
    /// Further `(coarse, fine)` byte offsets that receive the same value —
    /// the other cells of a pixel fixture or the other identical sections
    /// of an LED bar, ganged to this channel so the whole fixture shows one
    /// color until per-cell control exists. GDTF-sourced only.
    #[serde(default)]
    pub mirrors: Vec<(u16, Option<u16>)>,
}

/// A reference to the GDTF archive a fixture type is distilled from.
///
/// The type is the whole archive, not one of its modes (venue-exchange
/// design §22): every venue fixture of it states its own mode.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GdtfSource {
    /// Path to the GDTF archive, relative to the config directory.
    pub path: String,
}

/// How a strobe rate in hertz maps onto the strobe function's DMX range.
/// The GDTF spec's rule is the default for a GDTF fixture: its strobe
/// function's own table, linear in hertz where it has none. A unit whose
/// firmware does otherwise (the Astera PixelBrick runs on a period curve)
/// says so in its record; see [`StrobeCurve::automatic`].
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, strum::VariantArray,
)]
#[serde(rename_all = "lowercase")]
pub enum StrobeCurve {
    /// Linear in the flash period (1/Hz) across the DMX range: what
    /// measured units (the Astera PixelBrick) do. The default.
    #[default]
    Period,
    /// Linear in hertz across the DMX range, between the endpoints.
    Linear,
    /// The function's own step table (GDTF ChannelSets), linear inside
    /// each step; [`StrobeCurve::Linear`] when there is no table.
    Declared,
}

/// Movement limits — not part of GDTF; measured or configured per fixture.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MovementLimits {
    /// Maximum pan speed in degrees per second.
    pub max_pan_speed: Option<f64>,
    /// Maximum tilt speed in degrees per second.
    pub max_tilt_speed: Option<f64>,
}

/// A fixture type definition — the internal model.
///
/// The model itself is unversioned: DSL generations and GDTF are *source*
/// formats that convert into this. The v1 view (the `channels()` offset map
/// and the strobe getters) is derived from the structured channel
/// definitions, so consumers of either view always agree.
///
/// The structured fields are internal-only for now and deliberately skipped
/// from serialization: `FixtureType` serializes into webui/MCP responses,
/// and that surface must not change until the features consuming the rich
/// model ship. The distill cache stores its own representation.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FixtureType {
    /// The name of the fixture type.
    pub(super) name: String,

    /// Structured channel definitions, keyed by canonical channel name.
    #[serde(skip)]
    pub(super) channel_defs: HashMap<String, ChannelDef>,

    /// Coarse channel offsets, derived from `channel_defs`. Kept as a
    /// separate map so `channels()` can hand out the v1 view by reference.
    pub(super) channels: HashMap<String, u16>,

    /// The GDTF archive + mode this type is distilled from, if referential.
    #[serde(skip)]
    pub(super) source: Option<GdtfSource>,

    /// Movement limits, if configured.
    #[serde(skip)]
    pub(super) movement: MovementLimits,

    /// How a strobe rate maps onto the strobe channel, when the type says;
    /// `None` is automatic ([`StrobeCurve::automatic`]).
    #[serde(skip)]
    pub(super) strobe_curve: Option<StrobeCurve>,

    /// The cells of a pixel fixture, in the manufacturer's order; empty for
    /// a fixture with one colour.
    #[serde(skip)]
    pub(super) cells: Vec<Cell>,

    /// The rig model's path in the project's asset store
    /// (`lighting/.cache/assets/`), when the type has one — the 3D view's
    /// picture of it (design §16.2). Never serialized: it is a cache
    /// location, not part of the type.
    #[serde(skip)]
    pub(super) rig: Option<String>,

    /// How the rig's joints sit in the mounting frame (design §18.6),
    /// read from the rig when the type has one. Absent means the plain
    /// convention; a cache location's derivative, not part of the type.
    #[serde(skip)]
    pub(super) aim: Option<AimCalibration>,

    /// Maximum strobe frequency in Hz (if supported). Derived from the
    /// strobe channel's function when one exists; private so a fixture type
    /// can only be built through the normalizing constructors.
    pub(super) max_strobe_frequency: Option<f64>,

    /// Minimum strobe frequency in Hz (bottom of variable strobe range).
    pub(super) min_strobe_frequency: Option<f64>,

    /// First DMX value where variable strobe begins.
    pub(super) strobe_dmx_offset: Option<u8>,
}

/// A fixture definition.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Fixture {
    /// The name of the fixture.
    pub(super) name: String,

    /// The fixture type.
    pub(super) fixture_type: String,

    /// The universe.
    pub(super) universe: u16,

    /// The start channel.
    pub(super) start_channel: u16,

    /// Tags/roles/capabilities associated with this fixture.
    pub(super) tags: Vec<String>,

    /// Where the fixture hangs, in stage coordinates (see [`Vec3`]).
    pub(super) position: Option<Vec3>,

    /// How it is mounted: degrees about the stage X, Y and Z axes, applied
    /// in that order. Meaningless without a position.
    pub(super) rotation: Option<Vec3>,

    /// The GDTF mode this fixture is patched in (venue-exchange design §21):
    /// required of a GDTF type's fixture, an error on a native one. The
    /// parser takes the line as written; the loader enforces both. Serialized
    /// only when present, so a native-only venue reads exactly as it always
    /// did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) mode: Option<String>,

    /// The beam angle this unit actually has on stage, in degrees
    /// ([`beam_angle_valid`]): a diffuser or filter fitted to it makes it
    /// wider than its type says. A drawing hint for the web UI's 3D view;
    /// nothing in the engine reads it. `None` means the type's own angle.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) beam_angle: Option<f64>,
}

/// Where a venue was seeded from. Provenance, not a reference: the loader
/// never opens the MVR, and everything in the venue file is the user's to
/// edit. Re-import reads it to merge a revised MVR rather than replace.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VenueSource {
    /// The MVR archive, relative to the project directory.
    pub mvr: String,
    /// The MVR-space point (meters) that became the stage origin — the
    /// re-origin choice made at import, reapplied on re-import.
    pub origin: Vec3,
}

/// A venue definition.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Venue {
    /// The name of the venue.
    pub(super) name: String,

    /// The fixtures in the venue.
    pub(super) fixtures: HashMap<String, Fixture>,

    /// Named stage points shows can aim at, the positional analog of tags.
    pub(super) focus_points: BTreeMap<String, Vec3>,

    /// The MVR this venue was seeded from, if any.
    pub(super) source: Option<VenueSource>,
}

/// How a fixture's joints sit inside its mounting frame, read from its
/// GDTF geometry (design §18.6). Most fixtures are the identity: the
/// yoke turns about the mounting's Z and the head about the yoke's X,
/// with the beam down −Z at rest. Some are not — the Ayrton MagicDot SX
/// yaws its yoke geometry 90° — and for those the plain math would aim
/// the head into the wrong plane. With a calibration the beam is
///
/// ```text
/// direction = R · pre · Rz(pan + pan_offset) · Rx(tilt + tilt_offset) · (0, 0, −1)
/// ```
///
/// `pre` is the rotation the geometry puts before the pan joint, and the
/// offsets are yaws between the joints and the beam's rest angle in the
/// head. The three translations put the lens where the geometry has it,
/// so the beam is aimed from the lens, not the mounting point:
///
/// ```text
/// origin = mount_to_pan + pre · Rz(pan) · (pan_to_tilt + Rz(pan_offset) · Rx(tilt) · tilt_to_lens)
/// ```
///
/// Derived by `gdtf::aim_calibration`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AimCalibration {
    /// Rotation from the joint frame into the mounting frame (row-major).
    pub pre: [[f64; 3]; 3],
    /// Degrees added to the pan the geometry wants, to get the pan the
    /// fixture's own channel means.
    pub pan_offset: f64,
    /// Likewise for tilt.
    pub tilt_offset: f64,
    /// The pan joint's position in the mounting frame, meters.
    pub mount_to_pan: [f64; 3],
    /// The tilt joint's position in the pan joint's frame.
    pub pan_to_tilt: [f64; 3],
    /// The lens's position in the tilt joint's frame.
    pub tilt_to_lens: [f64; 3],
}
