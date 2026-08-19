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
        fixture_type.synthesize_strobe_function();
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

    /// Synthesizes a variable-strobe function on the strobe channel from the
    /// v1 strobe fields, so a v1 definition's structured view carries the
    /// same information as its fields. Requires all three fields and a
    /// strobe channel; partial fields stay fields.
    fn synthesize_strobe_function(&mut self) {
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

impl fmt::Display for FixtureType {
    /// Renders the v1 DSL form. The structured channel data (fine bytes,
    /// ranges, functions) has no DSL rendering yet — that arrives with the
    /// grammar that can parse it back. Until then this stays exactly the
    /// output the current grammar round-trips; a synthesized strobe function
    /// carries the same values as the strobe field lines, so nothing is
    /// lost either way.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
#[derive(Clone, Serialize)]
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
}

/// A venue definition.
#[derive(Clone, Serialize)]
pub struct Venue {
    /// The name of the venue.
    name: String,

    /// The fixtures in the venue.
    fixtures: HashMap<String, Fixture>,
}

impl Venue {
    /// Creates a new venue.
    pub fn new(name: String, fixtures: HashMap<String, Fixture>) -> Venue {
        Venue { name, fixtures }
    }

    /// Gets the name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the fixtures.
    pub fn fixtures(&self) -> &HashMap<String, Fixture> {
        &self.fixtures
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
            writeln!(f)?;
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
            },
        );
        let ft = FixtureType::from_channel_defs("Mover".to_string(), defs);
        assert_eq!(ft.footprint(), 2);
        // The v1 view exposes only the coarse byte.
        assert_eq!(ft.channels().get("pan"), Some(&1));
    }

    #[test]
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
