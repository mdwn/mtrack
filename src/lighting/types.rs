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

use std::collections::{BTreeMap, HashMap};
use std::fmt;

use serde::{Deserialize, Serialize};

/// The physical unit a channel value or range is expressed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PhysicalUnit {
    /// Degrees (pan/tilt angles).
    Degrees,
    /// Hertz (strobe frequencies).
    Hertz,
}

/// A physical value range (e.g. -270°..270°, 0.3 Hz..25 Hz).
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
}

/// A structured channel definition.
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

impl ChannelDef {
    /// Creates a plain channel definition with only an offset — the shape a
    /// v1 DSL channel map entry carries.
    pub fn at(offset: u16) -> ChannelDef {
        ChannelDef {
            offset,
            fine: None,
            range: None,
            functions: Vec::new(),
            mirrors: Vec::new(),
        }
    }

    /// Every byte offset this channel writes: its own, then its mirrors'.
    pub fn all_offsets(&self) -> impl Iterator<Item = u16> + '_ {
        std::iter::once(self.offset).chain(self.fine).chain(
            self.mirrors
                .iter()
                .flat_map(|(c, f)| std::iter::once(*c).chain(*f)),
        )
    }
}

/// A reference to the GDTF archive and mode a fixture type is distilled from.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct GdtfSource {
    /// Path to the GDTF archive, relative to the config directory.
    pub path: String,
    /// The DMX mode name within the archive.
    pub mode: String,
}

/// Movement limits — not part of GDTF; measured or configured per fixture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MovementLimits {
    /// Maximum pan speed in degrees per second.
    pub max_pan_speed: Option<f64>,
    /// Maximum tilt speed in degrees per second.
    pub max_tilt_speed: Option<f64>,
}

impl MovementLimits {
    /// Whether any limit is set.
    pub fn is_empty(&self) -> bool {
        self.max_pan_speed.is_none() && self.max_tilt_speed.is_none()
    }
}

/// The canonical channel name the strobe fields describe.
const STROBE_CHANNEL: &str = "strobe";

/// The function name used for the variable-strobe range.
const STROBE_FUNCTION: &str = "strobe";

/// A fixture type as the v1 DSL states it: a plain channel offset map plus
/// the three standalone strobe fields. This is a parse-time surface, not a
/// model — conversion into [`FixtureType`] is the single normalization
/// point, so there is no manual step to forget.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FixtureTypeV1 {
    /// The name of the fixture type.
    pub name: String,
    /// Channel name → 1-based offset.
    pub channels: HashMap<String, u16>,
    /// Maximum strobe frequency in Hz (if known).
    pub max_strobe_frequency: Option<f64>,
    /// Minimum strobe frequency in Hz (if known).
    pub min_strobe_frequency: Option<f64>,
    /// First DMX value where variable strobe begins (if known).
    pub strobe_dmx_offset: Option<u8>,
}

impl From<FixtureTypeV1> for FixtureType {
    fn from(v1: FixtureTypeV1) -> FixtureType {
        let channel_defs = v1
            .channels
            .into_iter()
            .map(|(name, offset)| (name, ChannelDef::at(offset)))
            .collect();
        FixtureType::from_parts(
            v1.name,
            channel_defs,
            v1.max_strobe_frequency,
            v1.min_strobe_frequency,
            v1.strobe_dmx_offset,
        )
    }
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
#[derive(Clone, Debug, Serialize)]
pub struct FixtureType {
    /// The name of the fixture type.
    name: String,

    /// Structured channel definitions, keyed by canonical channel name.
    #[serde(skip)]
    channel_defs: HashMap<String, ChannelDef>,

    /// Coarse channel offsets, derived from `channel_defs`. Kept as a
    /// separate map so `channels()` can hand out the v1 view by reference.
    channels: HashMap<String, u16>,

    /// The GDTF archive + mode this type is distilled from, if referential.
    #[serde(skip)]
    source: Option<GdtfSource>,

    /// Movement limits, if configured.
    #[serde(skip)]
    movement: MovementLimits,

    /// The rig model's path in the project's asset store
    /// (`lighting/.cache/assets/`), when the type has one — the 3D view's
    /// picture of it (design §16.2). Never serialized: it is a cache
    /// location, not part of the type.
    #[serde(skip)]
    rig: Option<String>,

    /// Maximum strobe frequency in Hz (if supported). Derived from the
    /// strobe channel's function when one exists; private so a fixture type
    /// can only be built through the normalizing constructors.
    max_strobe_frequency: Option<f64>,

    /// Minimum strobe frequency in Hz (bottom of variable strobe range).
    min_strobe_frequency: Option<f64>,

    /// First DMX value where variable strobe begins.
    strobe_dmx_offset: Option<u8>,
}

impl FixtureType {
    /// Creates a new fixture type from plain channel offsets. For the full
    /// v1 surface (strobe fields included), convert a [`FixtureTypeV1`]
    /// with `.into()`.
    pub fn new(name: String, channels: HashMap<String, u16>) -> FixtureType {
        FixtureTypeV1 {
            name,
            channels,
            ..FixtureTypeV1::default()
        }
        .into()
    }

    /// Creates a new fixture type from structured channel definitions. The
    /// v1 strobe fields are derived from the strobe channel's variable
    /// strobe function so consumers of either view see the same values.
    pub fn from_channel_defs(
        name: String,
        channel_defs: HashMap<String, ChannelDef>,
    ) -> FixtureType {
        FixtureType::from_parts(name, channel_defs, None, None, None)
    }

    /// The single normalization point every constructor funnels through.
    ///
    /// Explicit v1 strobe fields win over values derived from a strobe
    /// function; when all three are present and the strobe channel carries
    /// no function yet, one is synthesized so the structured view and the
    /// v1 view always agree.
    pub(crate) fn from_parts(
        name: String,
        channel_defs: HashMap<String, ChannelDef>,
        max_strobe_frequency: Option<f64>,
        min_strobe_frequency: Option<f64>,
        strobe_dmx_offset: Option<u8>,
    ) -> FixtureType {
        let channels = channel_defs
            .iter()
            .map(|(name, def)| (name.clone(), def.offset))
            .collect();
        let mut fixture_type = FixtureType {
            name,
            channel_defs,
            channels,
            source: None,
            movement: MovementLimits::default(),
            rig: None,
            max_strobe_frequency: None,
            min_strobe_frequency: None,
            strobe_dmx_offset: None,
        };
        fixture_type.derive_strobe_fields();
        if max_strobe_frequency.is_some() {
            fixture_type.max_strobe_frequency = max_strobe_frequency;
        }
        if min_strobe_frequency.is_some() {
            fixture_type.min_strobe_frequency = min_strobe_frequency;
        }
        if strobe_dmx_offset.is_some() {
            fixture_type.strobe_dmx_offset = strobe_dmx_offset;
        }
        fixture_type.reconcile_strobe_function();
        fixture_type
    }

    /// Fills the v1 strobe fields from the strobe channel's variable strobe
    /// function, if one is declared with a frequency range.
    fn derive_strobe_fields(&mut self) {
        let Some(def) = self.channel_defs.get(STROBE_CHANNEL) else {
            return;
        };
        let Some(func) = def
            .functions
            .iter()
            .find(|f| f.name == STROBE_FUNCTION && f.physical.is_some())
        else {
            return;
        };
        let physical = func.physical.expect("filtered on is_some");
        if physical.unit != PhysicalUnit::Hertz {
            return;
        }
        self.strobe_dmx_offset = Some(func.dmx_from);
        self.min_strobe_frequency = Some(physical.from.min(physical.to));
        self.max_strobe_frequency = Some(physical.from.max(physical.to));
    }

    /// Makes the strobe channel's variable-strobe function agree with the
    /// v1 strobe fields, so the structured view and the field view carry
    /// the same information. Explicit fields have already won by the time
    /// this runs, so a disagreeing function is *rewritten*, not skipped —
    /// skipping would leave the two views the constructors promise agree
    /// silently diverged. An agreeing function is left untouched (its
    /// dmx_to and physical orientation may carry intent the fields can't
    /// express). Requires all three fields and a strobe channel; partial
    /// fields stay fields.
    fn reconcile_strobe_function(&mut self) {
        let (Some(offset), Some(min), Some(max)) = (
            self.strobe_dmx_offset,
            self.min_strobe_frequency,
            self.max_strobe_frequency,
        ) else {
            return;
        };
        let Some(def) = self.channel_defs.get_mut(STROBE_CHANNEL) else {
            return;
        };
        if let Some(func) = def.functions.iter_mut().find(|f| f.name == STROBE_FUNCTION) {
            let agrees = func.dmx_from == offset
                && func.physical.is_some_and(|p| {
                    p.unit == PhysicalUnit::Hertz
                        && p.from.min(p.to) == min
                        && p.from.max(p.to) == max
                });
            if !agrees {
                func.dmx_from = offset;
                // An override can push the start past the old end; the
                // fields carry no end of their own, so open the range up.
                if func.dmx_to < offset {
                    func.dmx_to = u8::MAX;
                }
                func.physical = Some(PhysicalRange {
                    from: min,
                    to: max,
                    unit: PhysicalUnit::Hertz,
                });
            }
            return;
        }
        def.functions.push(ChannelFunction {
            name: STROBE_FUNCTION.to_string(),
            dmx_from: offset,
            dmx_to: u8::MAX,
            physical: Some(PhysicalRange {
                from: min,
                to: max,
                unit: PhysicalUnit::Hertz,
            }),
        });
    }

    /// Gets the name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the coarse channel offsets — the v1 view.
    pub fn channels(&self) -> &HashMap<String, u16> {
        &self.channels
    }

    /// Gets the structured channel definitions.
    pub fn channel_defs(&self) -> &HashMap<String, ChannelDef> {
        &self.channel_defs
    }

    /// Gets the GDTF source, if this type is referential.
    pub fn source(&self) -> Option<&GdtfSource> {
        self.source.as_ref()
    }

    /// Sets the GDTF source, making this type referential.
    pub fn set_source(&mut self, source: GdtfSource) {
        self.source = Some(source);
    }

    /// Gets the movement limits.
    pub fn movement(&self) -> &MovementLimits {
        &self.movement
    }

    /// Sets the movement limits.
    pub fn set_movement(&mut self, movement: MovementLimits) {
        self.movement = movement;
    }

    /// The rig model's asset-store path, when the type has one.
    pub fn rig(&self) -> Option<&str> {
        self.rig.as_deref()
    }

    /// Sets (or clears) the rig model's asset-store path.
    pub fn set_rig(&mut self, rig: Option<String>) {
        self.rig = rig;
    }

    /// The DMX footprint: the highest byte offset any channel occupies.
    pub fn footprint(&self) -> u16 {
        self.channel_defs
            .values()
            .flat_map(|d| d.all_offsets())
            .max()
            .unwrap_or(0)
    }

    /// Gets the maximum strobe frequency.
    pub fn max_strobe_frequency(&self) -> Option<f64> {
        self.max_strobe_frequency
    }

    /// Gets the minimum strobe frequency.
    pub fn min_strobe_frequency(&self) -> Option<f64> {
        self.min_strobe_frequency
    }

    /// Gets the strobe DMX offset.
    pub fn strobe_dmx_offset(&self) -> Option<u8> {
        self.strobe_dmx_offset
    }
}

impl FixtureType {
    /// Whether the channel definitions carry anything the v1 form cannot
    /// say: a fine byte, a physical range, or functions beyond the single
    /// strobe function the v1 strobe fields already describe. Such a type
    /// renders — and must live — in the rich form of a `.fixture` file.
    pub fn uses_rich_channels(&self) -> bool {
        self.channel_defs.values().any(|def| {
            def.fine.is_some()
                || def.range.is_some()
                || def.functions.len() > 1
                || def
                    .functions
                    .iter()
                    .any(|function| function.name != STROBE_FUNCTION)
        }) || self
            .channel_defs
            .iter()
            .any(|(name, def)| name != STROBE_CHANNEL && !def.functions.is_empty())
    }
}

/// A physical value as the DSL writes it: `270deg`, `0.5hz`. Six decimals
/// with trailing zeros trimmed — a strobe rate can be well under a
/// millihertz, which the coordinate formatter's three would round away.
fn fmt_physical(value: f64, unit: PhysicalUnit) -> String {
    let unit = match unit {
        PhysicalUnit::Degrees => "deg",
        PhysicalUnit::Hertz => "hz",
    };
    let mut text = format!("{value:.6}");
    if text.contains('.') {
        text = text.trim_end_matches('0').trim_end_matches('.').to_string();
    }
    if text == "-0" {
        text = "0".to_string();
    }
    format!("{text}{unit}")
}

impl fmt::Display for FixtureType {
    /// Renders the DSL form the type needs: the v1 `channel_map` form when
    /// the channels are plain offsets (plus the three strobe fields, which
    /// carry the same values as a synthesized strobe function), and the
    /// rich `channel` form when any channel has a fine byte, a range or
    /// functions — which only a `.fixture` file may hold.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.uses_rich_channels() {
            writeln!(f, "fixture_type \"{}\" {{", self.name)?;
            let mut defs: Vec<_> = self.channel_defs.iter().collect();
            defs.sort_by_key(|(name, def)| (def.offset, (*name).clone()));
            for (name, def) in defs {
                write!(f, "  channel \"{name}\" @ {}", def.offset)?;
                if let Some(fine) = def.fine {
                    write!(f, " fine {fine}")?;
                }
                if let Some(range) = def.range {
                    write!(
                        f,
                        " range {}..{}",
                        fmt_physical(range.from, range.unit),
                        fmt_physical(range.to, range.unit)
                    )?;
                }
                if def.functions.is_empty() {
                    writeln!(f)?;
                } else {
                    writeln!(f, " {{")?;
                    for function in &def.functions {
                        write!(
                            f,
                            "    function \"{}\" {}..{}",
                            function.name, function.dmx_from, function.dmx_to
                        )?;
                        if let Some(physical) = function.physical {
                            write!(
                                f,
                                " {}..{}",
                                fmt_physical(physical.from, physical.unit),
                                fmt_physical(physical.to, physical.unit)
                            )?;
                        }
                        writeln!(f)?;
                    }
                    writeln!(f, "  }}")?;
                }
            }
            if !self.movement.is_empty() {
                writeln!(f, "  movement {{")?;
                if let Some(speed) = self.movement.max_pan_speed {
                    writeln!(f, "    max_pan_speed: {}deg/s", fmt_coord(speed))?;
                }
                if let Some(speed) = self.movement.max_tilt_speed {
                    writeln!(f, "    max_tilt_speed: {}deg/s", fmt_coord(speed))?;
                }
                writeln!(f, "  }}")?;
            }
            return write!(f, "}}");
        }
        writeln!(f, "fixture_type \"{}\" {{", self.name)?;
        writeln!(f, "  channels: {}", self.channels.len())?;
        writeln!(f, "  channel_map: {{")?;
        let mut entries: Vec<_> = self.channels.iter().collect();
        entries.sort_by_key(|(_, v)| *v);
        for (i, (name, offset)) in entries.iter().enumerate() {
            let comma = if i + 1 < entries.len() { "," } else { "" };
            writeln!(f, "    \"{}\": {}{}", name, offset, comma)?;
        }
        writeln!(f, "  }}")?;
        if let Some(v) = self.max_strobe_frequency {
            writeln!(f, "  max_strobe_frequency: {v}")?;
        }
        if let Some(v) = self.min_strobe_frequency {
            writeln!(f, "  min_strobe_frequency: {v}")?;
        }
        if let Some(v) = self.strobe_dmx_offset {
            writeln!(f, "  strobe_dmx_offset: {v}")?;
        }
        write!(f, "}}")
    }
}

/// A fixture definition.
#[derive(Clone, Debug, Serialize)]
pub struct Fixture {
    /// The name of the fixture.
    name: String,

    /// The fixture type.
    fixture_type: String,

    /// The universe.
    universe: u16,

    /// The start channel.
    start_channel: u16,

    /// Tags/roles/capabilities associated with this fixture.
    tags: Vec<String>,

    /// Where the fixture hangs, in stage coordinates (see [`Vec3`]).
    position: Option<Vec3>,

    /// How it is mounted: degrees about the stage X, Y and Z axes, applied
    /// in that order. Meaningless without a position.
    rotation: Option<Vec3>,
}

/// A stage-space triple: meters, right-handed Z-up, origin downstage-center
/// on the deck, +x stage-left, +y upstage, +z up. (For a rotation, degrees.)
pub type Vec3 = [f64; 3];

impl Fixture {
    /// Creates a new fixture.
    pub fn new(
        name: String,
        fixture_type: String,
        universe: u16,
        start_channel: u16,
        tags: Vec<String>,
    ) -> Fixture {
        Fixture {
            name,
            fixture_type,
            universe,
            start_channel,
            tags,
            position: None,
            rotation: None,
        }
    }

    /// Places the fixture.
    pub fn with_position(mut self, position: Option<Vec3>) -> Fixture {
        self.position = position;
        self
    }

    /// Orients the fixture.
    pub fn with_rotation(mut self, rotation: Option<Vec3>) -> Fixture {
        self.rotation = rotation;
        self
    }

    /// Gets the position, if the venue places this fixture.
    pub fn position(&self) -> Option<Vec3> {
        self.position
    }

    /// Gets the mounting rotation, if the venue states one.
    pub fn rotation(&self) -> Option<Vec3> {
        self.rotation
    }

    /// Gets the name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the fixture type.
    pub fn fixture_type(&self) -> &str {
        &self.fixture_type
    }

    /// Gets the universe.
    pub fn universe(&self) -> u16 {
        self.universe
    }

    /// Gets the start channel.
    pub fn start_channel(&self) -> u16 {
        self.start_channel
    }

    /// Gets the tags on this fixture.
    pub fn tags(&self) -> &[String] {
        &self.tags
    }
}

/// Where a venue was seeded from. Provenance, not a reference: the loader
/// never opens the MVR, and everything in the venue file is the user's to
/// edit. Re-import reads it to merge a revised MVR rather than replace.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct VenueSource {
    /// The MVR archive, relative to the project directory.
    pub mvr: String,
    /// The MVR-space point (meters) that became the stage origin — the
    /// re-origin choice made at import, reapplied on re-import.
    pub origin: Vec3,
}

/// A venue definition.
#[derive(Clone, Debug, Serialize)]
pub struct Venue {
    /// The name of the venue.
    name: String,

    /// The fixtures in the venue.
    fixtures: HashMap<String, Fixture>,

    /// Named stage points shows can aim at, the positional analog of tags.
    focus_points: BTreeMap<String, Vec3>,

    /// The MVR this venue was seeded from, if any.
    source: Option<VenueSource>,
}

impl Venue {
    /// Creates a new venue.
    pub fn new(name: String, fixtures: HashMap<String, Fixture>) -> Venue {
        Venue {
            name,
            fixtures,
            focus_points: BTreeMap::new(),
            source: None,
        }
    }

    /// Binds the venue's focus points.
    pub fn with_focus_points(mut self, focus_points: BTreeMap<String, Vec3>) -> Venue {
        self.focus_points = focus_points;
        self
    }

    /// Records where the venue was seeded from.
    pub fn with_source(mut self, source: Option<VenueSource>) -> Venue {
        self.source = source;
        self
    }

    /// Gets the name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the fixtures.
    pub fn fixtures(&self) -> &HashMap<String, Fixture> {
        &self.fixtures
    }

    /// Gets the focus points.
    pub fn focus_points(&self) -> &BTreeMap<String, Vec3> {
        &self.focus_points
    }

    /// Gets the import provenance, if the venue was seeded from an MVR.
    pub fn source(&self) -> Option<&VenueSource> {
        self.source.as_ref()
    }

    /// The fixtures sorted by patch, the order the DSL form lists them in.
    pub fn fixtures_by_patch(&self) -> Vec<&Fixture> {
        let mut fixtures: Vec<_> = self.fixtures.values().collect();
        fixtures.sort_by(|a, b| {
            (a.universe, a.start_channel, &a.name).cmp(&(b.universe, b.start_channel, &b.name))
        });
        fixtures
    }
}

/// A coordinate as the DSL writes it: millimeter precision, no exponent, no
/// negative zero — the grammar's `signed_number` reads it back exactly.
pub fn fmt_coord(value: f64) -> String {
    let rounded = (value * 1000.0).round() / 1000.0;
    if rounded == 0.0 || !rounded.is_finite() {
        "0".to_string()
    } else {
        format!("{rounded}")
    }
}

/// A stage triple in DSL form.
pub fn fmt_vec3(v: &Vec3) -> String {
    format!(
        "({}, {}, {})",
        fmt_coord(v[0]),
        fmt_coord(v[1]),
        fmt_coord(v[2])
    )
}

/// A fixture-type reference as the DSL accepts it: bare when it is an
/// identifier, quoted otherwise ("Astera PixelBrick").
fn fmt_type_reference(name: &str) -> String {
    let mut chars = name.chars();
    let bare = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    if bare {
        name.to_string()
    } else {
        format!("\"{name}\"")
    }
}

impl fmt::Display for Fixture {
    /// One venue line, without the leading indent or trailing newline.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "fixture \"{}\" {} @ {}:{}",
            self.name,
            fmt_type_reference(&self.fixture_type),
            self.universe,
            self.start_channel
        )?;
        if !self.tags.is_empty() {
            let tags: Vec<String> = self.tags.iter().map(|t| format!("\"{t}\"")).collect();
            write!(f, " tags [{}]", tags.join(", "))?;
        }
        if let Some(position) = &self.position {
            write!(f, " position {}", fmt_vec3(position))?;
        }
        if let Some(rotation) = &self.rotation {
            write!(f, " rotation {}", fmt_vec3(rotation))?;
        }
        Ok(())
    }
}

impl fmt::Display for Venue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "venue \"{}\" {{", self.name)?;
        if let Some(source) = &self.source {
            writeln!(
                f,
                "  imported from mvr(\"{}\") origin {}",
                source.mvr,
                fmt_vec3(&source.origin)
            )?;
        }
        for fix in self.fixtures_by_patch() {
            writeln!(f, "  {fix}")?;
        }
        for (name, point) in &self.focus_points {
            writeln!(f, "  focus \"{name}\" {}", fmt_vec3(point))?;
        }
        write!(f, "}}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── FixtureType ────────────────────────────────────────────────

    #[test]
    fn fixture_type_new() {
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);
        let ft = FixtureType::new("RGB Par".to_string(), channels);
        assert_eq!(ft.name(), "RGB Par");
        assert_eq!(ft.channels().len(), 3);
        assert_eq!(ft.max_strobe_frequency(), None);
        assert_eq!(ft.min_strobe_frequency(), None);
        assert_eq!(ft.strobe_dmx_offset(), None);
        // The structured view mirrors the plain offsets.
        assert_eq!(ft.channel_defs().len(), 3);
        assert_eq!(ft.channel_defs().get("red").unwrap().offset, 1);
        assert_eq!(ft.footprint(), 3);
    }

    #[test]
    fn fixture_type_strobe_fields() {
        let ft: FixtureType = FixtureTypeV1 {
            name: "Strobe".to_string(),
            max_strobe_frequency: Some(25.0),
            min_strobe_frequency: Some(1.0),
            strobe_dmx_offset: Some(128),
            ..FixtureTypeV1::default()
        }
        .into();
        assert_eq!(ft.max_strobe_frequency(), Some(25.0));
        assert_eq!(ft.min_strobe_frequency(), Some(1.0));
        assert_eq!(ft.strobe_dmx_offset(), Some(128));
    }

    #[test]
    fn fixture_type_v1_conversion_synthesizes_strobe_function() {
        // From<FixtureTypeV1> is the single normalization point — no manual
        // step: the strobe function appears as part of the conversion.
        let mut channels = HashMap::new();
        channels.insert("strobe".to_string(), 4);
        let ft: FixtureType = FixtureTypeV1 {
            name: "Brick".to_string(),
            channels,
            max_strobe_frequency: Some(25.0),
            min_strobe_frequency: Some(0.4),
            strobe_dmx_offset: Some(7),
        }
        .into();

        let def = ft.channel_defs().get("strobe").unwrap();
        assert_eq!(def.functions.len(), 1);
        let func = &def.functions[0];
        assert_eq!(func.name, "strobe");
        assert_eq!(func.dmx_from, 7);
        assert_eq!(func.dmx_to, 255);
        let physical = func.physical.unwrap();
        assert_eq!(physical.from, 0.4);
        assert_eq!(physical.to, 25.0);
        assert_eq!(physical.unit, PhysicalUnit::Hertz);
    }

    #[test]
    fn fixture_type_partial_strobe_fields_stay_fields() {
        // Only max is known — nothing to synthesize, and the field must
        // survive as-is for the strobe strategies that read it.
        let mut channels = HashMap::new();
        channels.insert("strobe".to_string(), 2);
        let ft: FixtureType = FixtureTypeV1 {
            name: "Strobe".to_string(),
            channels,
            max_strobe_frequency: Some(20.0),
            ..FixtureTypeV1::default()
        }
        .into();
        assert_eq!(ft.max_strobe_frequency(), Some(20.0));
        assert!(ft
            .channel_defs()
            .get("strobe")
            .unwrap()
            .functions
            .is_empty());
    }

    #[test]
    fn fixture_type_from_channel_defs_derives_strobe_fields() {
        let mut defs = HashMap::new();
        defs.insert("red".to_string(), ChannelDef::at(1));
        defs.insert(
            "strobe".to_string(),
            ChannelDef {
                offset: 4,
                fine: None,
                range: None,
                functions: vec![
                    ChannelFunction {
                        name: "off".to_string(),
                        dmx_from: 0,
                        dmx_to: 6,
                        physical: None,
                    },
                    ChannelFunction {
                        name: "strobe".to_string(),
                        dmx_from: 7,
                        dmx_to: 255,
                        physical: Some(PhysicalRange {
                            from: 0.4,
                            to: 25.0,
                            unit: PhysicalUnit::Hertz,
                        }),
                    },
                ],
                mirrors: Vec::new(),
            },
        );
        let ft = FixtureType::from_channel_defs("PixelBrick".to_string(), defs);
        // The v1 view is derived, so consumers of either view agree.
        assert_eq!(ft.strobe_dmx_offset(), Some(7));
        assert_eq!(ft.min_strobe_frequency(), Some(0.4));
        assert_eq!(ft.max_strobe_frequency(), Some(25.0));
        assert_eq!(ft.channels().get("strobe"), Some(&4));
    }

    #[test]
    fn fixture_type_footprint_includes_fine_bytes() {
        let mut defs = HashMap::new();
        defs.insert(
            "pan".to_string(),
            ChannelDef {
                offset: 1,
                fine: Some(2),
                range: Some(PhysicalRange {
                    from: -270.0,
                    to: 270.0,
                    unit: PhysicalUnit::Degrees,
                }),
                functions: Vec::new(),
                mirrors: Vec::new(),
            },
        );
        let ft = FixtureType::from_channel_defs("Mover".to_string(), defs);
        assert_eq!(ft.footprint(), 2);
        // The v1 view exposes only the coarse byte.
        assert_eq!(ft.channels().get("pan"), Some(&1));
    }

    #[test]
    // The v1 form is what a plain type renders as, byte for byte: the
    // rich form (fine, range, functions) exists since P1c-3 but only a
    // type that needs it uses it.
    fn fixture_type_display_is_unchanged_v1_form() {
        // The Display output must stay exactly what today's grammar
        // round-trips — the rich model has no DSL rendering yet.
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("strobe".to_string(), 4);
        let ft: FixtureType = FixtureTypeV1 {
            name: "Brick".to_string(),
            channels,
            max_strobe_frequency: Some(25.0),
            min_strobe_frequency: Some(0.4),
            strobe_dmx_offset: Some(7),
        }
        .into();
        let output = ft.to_string();
        assert!(output.contains("channel_map: {"), "{output}");
        assert!(output.contains("\"red\": 1,"), "{output}");
        assert!(output.contains("max_strobe_frequency: 25"), "{output}");
        assert!(output.contains("strobe_dmx_offset: 7"), "{output}");
        assert!(!output.contains("functions"), "{output}");
    }

    #[test]
    fn fixture_type_serialization_surface_is_unchanged() {
        // FixtureType serializes into webui/MCP responses. The rich model
        // is internal-only for now, so the JSON must not grow fields until
        // the features consuming it ship.
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        let mut ft = FixtureType::new("Par".to_string(), channels);
        ft.set_source(GdtfSource {
            path: "library/par.gdtf".to_string(),
            mode: "Mode 1".to_string(),
        });
        let json = serde_json::to_value(&ft).unwrap();
        let keys: Vec<&str> = json
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        assert_eq!(
            keys,
            vec![
                "channels",
                "max_strobe_frequency",
                "min_strobe_frequency",
                "name",
                "strobe_dmx_offset"
            ],
            "serialized surface changed: {json}"
        );
    }

    #[test]
    fn from_parts_rewrites_a_disagreeing_strobe_function() {
        // Explicit fields win — and the function must follow, or the two
        // views the constructors promise agree would silently diverge.
        // This is the P1a override path: a .fixture override's strobe
        // fields on top of a GDTF-distilled function.
        let mut defs = HashMap::new();
        defs.insert(
            "strobe".to_string(),
            ChannelDef {
                offset: 4,
                fine: None,
                range: None,
                functions: vec![ChannelFunction {
                    name: "strobe".to_string(),
                    dmx_from: 7,
                    dmx_to: 200,
                    physical: Some(PhysicalRange {
                        from: 0.4,
                        to: 25.0,
                        unit: PhysicalUnit::Hertz,
                    }),
                }],
                mirrors: Vec::new(),
            },
        );
        let ft =
            FixtureType::from_parts("Brick".to_string(), defs, Some(99.0), Some(50.0), Some(210));

        // The getters carry the explicit values...
        assert_eq!(ft.max_strobe_frequency(), Some(99.0));
        assert_eq!(ft.min_strobe_frequency(), Some(50.0));
        assert_eq!(ft.strobe_dmx_offset(), Some(210));
        // ...and the function was rewritten to agree.
        let func = &ft.channel_defs().get("strobe").unwrap().functions[0];
        assert_eq!(func.dmx_from, 210);
        // The override pushed the start past the old end (200), so the
        // range opened up rather than inverting.
        assert_eq!(func.dmx_to, u8::MAX);
        let physical = func.physical.unwrap();
        assert_eq!((physical.from, physical.to), (50.0, 99.0));
    }

    #[test]
    fn from_parts_leaves_an_agreeing_strobe_function_untouched() {
        // Agreement is checked orientation-insensitively: a descending
        // physical range and a custom dmx_to carry intent the fields can't
        // express, and must survive.
        let mut defs = HashMap::new();
        defs.insert(
            "strobe".to_string(),
            ChannelDef {
                offset: 4,
                fine: None,
                range: None,
                functions: vec![ChannelFunction {
                    name: "strobe".to_string(),
                    dmx_from: 7,
                    dmx_to: 200,
                    physical: Some(PhysicalRange {
                        from: 25.0,
                        to: 0.4,
                        unit: PhysicalUnit::Hertz,
                    }),
                }],
                mirrors: Vec::new(),
            },
        );
        let ft = FixtureType::from_parts("Brick".to_string(), defs, Some(25.0), Some(0.4), Some(7));
        let func = &ft.channel_defs().get("strobe").unwrap().functions[0];
        assert_eq!(func.dmx_to, 200);
        let physical = func.physical.unwrap();
        assert_eq!((physical.from, physical.to), (25.0, 0.4));
    }

    // ── Fixture ────────────────────────────────────────────────────

    #[test]
    fn fixture_new() {
        let f = Fixture::new(
            "par1".to_string(),
            "RGB Par".to_string(),
            1,
            10,
            vec!["front".to_string(), "wash".to_string()],
        );
        assert_eq!(f.name(), "par1");
        assert_eq!(f.fixture_type(), "RGB Par");
        assert_eq!(f.universe(), 1);
        assert_eq!(f.start_channel(), 10);
        assert_eq!(f.tags(), &["front", "wash"]);
    }

    #[test]
    fn fixture_no_tags() {
        let f = Fixture::new("spot1".to_string(), "Spot".to_string(), 2, 1, vec![]);
        assert!(f.tags().is_empty());
    }

    // ── Venue ──────────────────────────────────────────────────────

    #[test]
    fn venue_new() {
        let mut fixtures = HashMap::new();
        fixtures.insert(
            "par1".to_string(),
            Fixture::new("par1".to_string(), "RGB".to_string(), 1, 1, vec![]),
        );

        let v = Venue::new("Club".to_string(), fixtures);
        assert_eq!(v.name(), "Club");
        assert_eq!(v.fixtures().len(), 1);
        assert!(v.fixtures().contains_key("par1"));
    }
}
