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

use std::collections::HashMap;
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

impl PhysicalUnit {
    /// The DSL suffix for this unit.
    pub fn suffix(&self) -> &'static str {
        match self {
            PhysicalUnit::Degrees => "deg",
            PhysicalUnit::Hertz => "hz",
        }
    }
}

/// A physical value range (e.g. -270deg..270deg, 0.3hz..25.0hz).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct PhysicalRange {
    /// The value at the low end of the DMX range.
    pub from: f64,
    /// The value at the high end of the DMX range.
    pub to: f64,
    /// The unit both endpoints are expressed in.
    pub unit: PhysicalUnit,
}

impl fmt::Display for PhysicalRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = self.unit.suffix();
        write!(f, "{}{s}..{}{s}", self.from, self.to)
    }
}

/// A function of a channel: a named DMX sub-range, optionally mapped to a
/// physical value range (e.g. "strobe": 64..255 -> 0.3hz..25.0hz).
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
}

impl ChannelDef {
    /// Creates a plain channel definition with only an offset — the v1 shape.
    pub fn at(offset: u16) -> ChannelDef {
        ChannelDef {
            offset,
            fine: None,
            range: None,
            functions: Vec::new(),
        }
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

/// A fixture type definition.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FixtureType {
    /// The name of the fixture type.
    name: String,

    /// Structured channel definitions, keyed by canonical channel name.
    channel_defs: HashMap<String, ChannelDef>,

    /// Coarse channel offsets, derived from `channel_defs`. Kept as a
    /// separate map so `channels()` can hand out the v1 view by reference.
    channels: HashMap<String, u16>,

    /// The GDTF archive + mode this type is distilled from, if referential.
    source: Option<GdtfSource>,

    /// Movement limits, if configured.
    movement: MovementLimits,

    /// Maximum strobe frequency in Hz (if supported).
    pub max_strobe_frequency: Option<f64>,

    /// Minimum strobe frequency in Hz (bottom of variable strobe range).
    pub min_strobe_frequency: Option<f64>,

    /// First DMX value where variable strobe begins.
    pub strobe_dmx_offset: Option<u8>,
}

impl FixtureType {
    /// Creates a new fixture type from plain channel offsets — the v1 shape.
    pub fn new(name: String, channels: HashMap<String, u16>) -> FixtureType {
        let channel_defs = channels
            .iter()
            .map(|(name, offset)| (name.clone(), ChannelDef::at(*offset)))
            .collect();
        FixtureType {
            name,
            channel_defs,
            channels,
            source: None,
            movement: MovementLimits::default(),
            max_strobe_frequency: None,
            min_strobe_frequency: None,
            strobe_dmx_offset: None,
        }
    }

    /// Creates a new fixture type from structured channel definitions. The
    /// legacy strobe fields are derived from the strobe channel's variable
    /// strobe function so v1 consumers see the same values either way.
    pub fn from_channel_defs(
        name: String,
        channel_defs: HashMap<String, ChannelDef>,
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
            max_strobe_frequency: None,
            min_strobe_frequency: None,
            strobe_dmx_offset: None,
        };
        fixture_type.derive_strobe_fields();
        fixture_type
    }

    /// Fills the legacy strobe fields from the strobe channel's variable
    /// strobe function, if one is declared with a frequency range.
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

    /// Synthesizes a variable-strobe function on the strobe channel from the
    /// legacy strobe fields. Used when normalizing a v1 definition so the
    /// structured view carries the same information as the fields.
    pub fn normalize_legacy_strobe(&mut self) {
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
        if def.functions.iter().any(|f| f.name == STROBE_FUNCTION) {
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

    /// The DMX footprint: the highest byte offset any channel occupies.
    pub fn footprint(&self) -> u16 {
        self.channel_defs
            .values()
            .map(|d| d.offset.max(d.fine.unwrap_or(0)))
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

/// Formats a float the way the DSL reads it back (no exponent notation for
/// the magnitudes fixtures use; integral values print without a trailing .0).
fn format_number(value: f64) -> String {
    format!("{value}")
}

impl fmt::Display for FixtureType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "fixture_type \"{}\"", self.name)?;
        if let Some(source) = &self.source {
            write!(
                f,
                "\n  from gdtf(\"{}\", mode \"{}\")\n{{",
                source.path, source.mode
            )?;
        } else {
            write!(f, " {{")?;
        }
        writeln!(f)?;
        writeln!(f, "  channels: {}", self.footprint())?;

        // Emit v2 channel lines, ordered by offset for stable output. A v1
        // definition that only set the legacy strobe fields is normalized
        // into a function first so nothing is lost in the rewrite.
        let mut this = self.clone();
        this.normalize_legacy_strobe();
        // Normalization needs all three fields and a strobe channel; a
        // partial set (e.g. only max_strobe_frequency) can't be expressed as
        // a function and must survive the rewrite as explicit fields.
        let strobe_in_functions = this
            .channel_defs
            .get(STROBE_CHANNEL)
            .is_some_and(|def| def.functions.iter().any(|f| f.name == STROBE_FUNCTION));
        let mut entries: Vec<_> = this.channel_defs.iter().collect();
        entries.sort_by_key(|(name, def)| (def.offset, name.as_str()));
        for (name, def) in entries {
            write!(f, "  channel \"{name}\" @ {}", def.offset)?;
            if let Some(fine) = def.fine {
                write!(f, " fine {fine}")?;
            }
            if let Some(range) = def.range {
                write!(f, " range: {range}")?;
            }
            if def.functions.is_empty() {
                writeln!(f)?;
            } else {
                writeln!(f, " {{")?;
                let functions: Vec<String> = def
                    .functions
                    .iter()
                    .map(|func| {
                        let mut s = format!(
                            "\"{}\": {}..{}",
                            func.name, func.dmx_from, func.dmx_to
                        );
                        if let Some(physical) = func.physical {
                            s.push_str(&format!(" -> {physical}"));
                        }
                        s
                    })
                    .collect();
                writeln!(f, "    functions: {{ {} }}", functions.join(", "))?;
                writeln!(f, "  }}")?;
            }
        }
        if !strobe_in_functions {
            if let Some(v) = self.max_strobe_frequency {
                writeln!(f, "  max_strobe_frequency: {}", format_number(v))?;
            }
            if let Some(v) = self.min_strobe_frequency {
                writeln!(f, "  min_strobe_frequency: {}", format_number(v))?;
            }
            if let Some(v) = self.strobe_dmx_offset {
                writeln!(f, "  strobe_dmx_offset: {v}")?;
            }
        }
        if !self.movement.is_empty() {
            write!(f, "  movement {{")?;
            if let Some(v) = self.movement.max_pan_speed {
                write!(f, " max_pan_speed: {}deg/s", format_number(v))?;
            }
            if let Some(v) = self.movement.max_tilt_speed {
                write!(f, " max_tilt_speed: {}deg/s", format_number(v))?;
            }
            writeln!(f, " }}")?;
        }
        write!(f, "}}")
    }
}

/// A point or rotation in stage coordinates: meters (or degrees for
/// rotations), right-handed Z-up, origin at downstage-center on the deck.
pub type Vec3 = [f64; 3];

fn format_vec3(v: &Vec3) -> String {
    format!(
        "({}, {}, {})",
        format_number(v[0]),
        format_number(v[1]),
        format_number(v[2])
    )
}

/// A fixture definition.
#[derive(Clone, Debug, Serialize, Deserialize)]
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

    /// Position in stage coordinates (meters), if known.
    position: Option<Vec3>,

    /// Mounting rotation in degrees, if known.
    rotation: Option<Vec3>,
}

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

    /// Gets the stage position, if known.
    pub fn position(&self) -> Option<Vec3> {
        self.position
    }

    /// Sets the stage position.
    pub fn set_position(&mut self, position: Option<Vec3>) {
        self.position = position;
    }

    /// Gets the mounting rotation in degrees, if known.
    pub fn rotation(&self) -> Option<Vec3> {
        self.rotation
    }

    /// Sets the mounting rotation.
    pub fn set_rotation(&mut self, rotation: Option<Vec3>) {
        self.rotation = rotation;
    }
}

/// A venue definition.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Venue {
    /// The name of the venue.
    name: String,

    /// The fixtures in the venue.
    fixtures: HashMap<String, Fixture>,

    /// Named focus points in stage coordinates, bound per-venue.
    focus_points: HashMap<String, Vec3>,
}

impl Venue {
    /// Creates a new venue.
    pub fn new(name: String, fixtures: HashMap<String, Fixture>) -> Venue {
        Venue {
            name,
            fixtures,
            focus_points: HashMap::new(),
        }
    }

    /// Gets the name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the fixtures.
    pub fn fixtures(&self) -> &HashMap<String, Fixture> {
        &self.fixtures
    }

    /// Gets the named focus points.
    pub fn focus_points(&self) -> &HashMap<String, Vec3> {
        &self.focus_points
    }

    /// Sets the named focus points.
    pub fn set_focus_points(&mut self, focus_points: HashMap<String, Vec3>) {
        self.focus_points = focus_points;
    }
}

impl fmt::Display for Venue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "venue \"{}\" {{", self.name)?;
        let mut fixtures: Vec<_> = self.fixtures.values().collect();
        fixtures.sort_by_key(|fix| (fix.universe, fix.start_channel));
        for fix in &fixtures {
            write!(
                f,
                "  fixture \"{}\" {} @ {}:{}",
                fix.name, fix.fixture_type, fix.universe, fix.start_channel
            )?;
            if !fix.tags.is_empty() {
                let tags: Vec<String> = fix.tags.iter().map(|t| format!("\"{t}\"")).collect();
                write!(f, " tags [{}]", tags.join(", "))?;
            }
            if let Some(position) = &fix.position {
                write!(f, " position {}", format_vec3(position))?;
            }
            if let Some(rotation) = &fix.rotation {
                write!(f, " rotation {}", format_vec3(rotation))?;
            }
            writeln!(f)?;
        }
        let mut focus_points: Vec<_> = self.focus_points.iter().collect();
        focus_points.sort_by_key(|(name, _)| name.as_str());
        for (name, point) in focus_points {
            writeln!(f, "  focus \"{name}\" {}", format_vec3(point))?;
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
        let mut ft = FixtureType::new("Strobe".to_string(), HashMap::new());
        ft.max_strobe_frequency = Some(25.0);
        ft.min_strobe_frequency = Some(1.0);
        ft.strobe_dmx_offset = Some(128);
        assert_eq!(ft.max_strobe_frequency(), Some(25.0));
        assert_eq!(ft.min_strobe_frequency(), Some(1.0));
        assert_eq!(ft.strobe_dmx_offset(), Some(128));
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
            },
        );
        let ft = FixtureType::from_channel_defs("PixelBrick".to_string(), defs);
        // The legacy view is derived, so v1 consumers see the same values.
        assert_eq!(ft.strobe_dmx_offset(), Some(7));
        assert_eq!(ft.min_strobe_frequency(), Some(0.4));
        assert_eq!(ft.max_strobe_frequency(), Some(25.0));
        assert_eq!(ft.channels().get("strobe"), Some(&4));
    }

    #[test]
    fn fixture_type_normalize_legacy_strobe() {
        let mut channels = HashMap::new();
        channels.insert("strobe".to_string(), 4);
        let mut ft = FixtureType::new("Brick".to_string(), channels);
        ft.max_strobe_frequency = Some(25.0);
        ft.min_strobe_frequency = Some(0.4);
        ft.strobe_dmx_offset = Some(7);
        ft.normalize_legacy_strobe();

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
            },
        );
        let ft = FixtureType::from_channel_defs("Mover".to_string(), defs);
        assert_eq!(ft.footprint(), 2);
        // The v1 view exposes only the coarse byte.
        assert_eq!(ft.channels().get("pan"), Some(&1));
    }

    #[test]
    fn fixture_type_display_v2() {
        let mut defs = HashMap::new();
        defs.insert("red".to_string(), ChannelDef::at(1));
        defs.insert(
            "pan".to_string(),
            ChannelDef {
                offset: 2,
                fine: Some(3),
                range: Some(PhysicalRange {
                    from: -270.0,
                    to: 270.0,
                    unit: PhysicalUnit::Degrees,
                }),
                functions: Vec::new(),
            },
        );
        let ft = FixtureType::from_channel_defs("Mover".to_string(), defs);
        let output = ft.to_string();
        assert!(output.contains("channel \"red\" @ 1"), "{output}");
        assert!(
            output.contains("channel \"pan\" @ 2 fine 3 range: -270deg..270deg"),
            "{output}"
        );
        assert!(output.contains("channels: 3"), "{output}");
    }

    #[test]
    fn fixture_type_display_referential() {
        let mut ft = FixtureType::new("Esprite".to_string(), HashMap::new());
        ft.set_source(GdtfSource {
            path: "library/Robe@Esprite@V1.1.gdtf".to_string(),
            mode: "Mode 1".to_string(),
        });
        ft.set_movement(MovementLimits {
            max_pan_speed: Some(240.0),
            max_tilt_speed: None,
        });
        let output = ft.to_string();
        assert!(
            output.contains("from gdtf(\"library/Robe@Esprite@V1.1.gdtf\", mode \"Mode 1\")"),
            "{output}"
        );
        assert!(output.contains("max_pan_speed: 240deg/s"), "{output}");
    }

    #[test]
    fn fixture_type_display_legacy_strobe_becomes_function() {
        // A v1 definition rewrites to the v2 function form on Display.
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("strobe".to_string(), 4);
        let mut ft = FixtureType::new("Brick".to_string(), channels);
        ft.max_strobe_frequency = Some(25.0);
        ft.min_strobe_frequency = Some(0.4);
        ft.strobe_dmx_offset = Some(7);
        let output = ft.to_string();
        assert!(
            output.contains("functions: { \"strobe\": 7..255 -> 0.4hz..25hz }"),
            "{output}"
        );
        assert!(!output.contains("max_strobe_frequency"), "{output}");
    }

    #[test]
    fn fixture_type_display_partial_strobe_fields_survive() {
        // Only max_strobe_frequency is known — it can't become a function
        // (no offset, no min), so the rewrite must keep the explicit field.
        let mut channels = HashMap::new();
        channels.insert("dimmer".to_string(), 1);
        channels.insert("strobe".to_string(), 2);
        let mut ft = FixtureType::new("Strobe".to_string(), channels);
        ft.max_strobe_frequency = Some(20.0);
        let output = ft.to_string();
        assert!(output.contains("max_strobe_frequency: 20"), "{output}");
        assert!(!output.contains("functions"), "{output}");
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
        assert_eq!(f.position(), None);
        assert_eq!(f.rotation(), None);
    }

    #[test]
    fn fixture_no_tags() {
        let f = Fixture::new("spot1".to_string(), "Spot".to_string(), 2, 1, vec![]);
        assert!(f.tags().is_empty());
    }

    #[test]
    fn fixture_position_rotation() {
        let mut f = Fixture::new("spot1".to_string(), "Spot".to_string(), 1, 1, vec![]);
        f.set_position(Some([-2.0, 3.5, 4.2]));
        f.set_rotation(Some([0.0, 0.0, 180.0]));
        assert_eq!(f.position(), Some([-2.0, 3.5, 4.2]));
        assert_eq!(f.rotation(), Some([0.0, 0.0, 180.0]));
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
        assert!(v.focus_points().is_empty());
    }

    #[test]
    fn venue_focus_points_display() {
        let mut fixtures = HashMap::new();
        let mut fixture = Fixture::new("spot1".to_string(), "Spot".to_string(), 1, 1, vec![]);
        fixture.set_position(Some([-2.0, 3.5, 4.2]));
        fixtures.insert("spot1".to_string(), fixture);

        let mut v = Venue::new("Club".to_string(), fixtures);
        let mut focus = HashMap::new();
        focus.insert("drummer".to_string(), [0.0, 2.8, 1.4]);
        v.set_focus_points(focus);

        let output = v.to_string();
        assert!(output.contains("position (-2, 3.5, 4.2)"), "{output}");
        assert!(output.contains("focus \"drummer\" (0, 2.8, 1.4)"), "{output}");
    }
}
