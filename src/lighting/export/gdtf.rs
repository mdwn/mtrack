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

//! A GDTF for a native fixture type (design §16.4, §19): one mode carrying
//! the type's channels with the spec's own attribute definitions, its
//! cells as a template geometry with references, and no physical model —
//! mtrack knows a hand-written type's channels and nothing of its body,
//! and says so rather than invent one. Valid by the spec's rules (checked
//! by [`crate::lighting::gdtf::strict`]), so a console patches it and
//! groups its encoders sensibly; and mtrack's own distiller reads it back
//! as the same type.

use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::io::Write;

use super::escape;
use crate::lighting::types::{Cell, ChannelDef, FixtureType, PhysicalUnit};

/// The generated mode's name.
pub const MODE_NAME: &str = "mtrack";

/// An attribute definition, as Annex B of the spec gives it for the
/// attributes mtrack canonicalises on import; a custom channel gets a
/// plain one under `Control.Control`.
struct Attribute {
    name: String,
    pretty: String,
    feature: &'static str,
    activation: Option<&'static str>,
    unit: &'static str,
    color: Option<&'static str>,
    main: Option<&'static str>,
}

/// The Annex B definition of the attribute a canonical channel name maps
/// back to — the inverse of the distiller's table, so a generated GDTF
/// distills to the channels it came from.
fn canonical(channel: &str) -> Option<Attribute> {
    let color = |name: &str, pretty: &str, cie: &'static str| Attribute {
        name: name.to_string(),
        pretty: pretty.to_string(),
        feature: "Color.RGB",
        activation: Some("ColorRGB"),
        unit: "ColorComponent",
        color: Some(cie),
        main: None,
    };
    let plain = |name: &str, pretty: &str, feature: &'static str, unit: &'static str| Attribute {
        name: name.to_string(),
        pretty: pretty.to_string(),
        feature,
        activation: None,
        unit,
        color: None,
        main: None,
    };
    Some(match channel {
        "dimmer" => plain("Dimmer", "Dim", "Dimmer.Dimmer", "None"),
        "red" => color("ColorAdd_R", "R", "0.64,0.33,21.3"),
        "green" => color("ColorAdd_G", "G", "0.3,0.6,71.5"),
        "blue" => color("ColorAdd_B", "B", "0.15,0.06,7.2"),
        "white" => color("ColorAdd_W", "White", "0.313,0.329,100.0"),
        "warm_white" => color("ColorAdd_WW", "WW", "0.319,0.340,99.3"),
        "cool_white" => color("ColorAdd_CW", "CW", "0.306,0.329,97.9"),
        "uv" => color("ColorAdd_UV", "UV", "0.176,0.005,0.6"),
        "amber" => color("ColorAdd_RY", "Amber", "0.477,0.460,57.0"),
        "pan" => Attribute {
            activation: Some("PanTilt"),
            ..plain("Pan", "P", "Position.PanTilt", "Angle")
        },
        "tilt" => Attribute {
            activation: Some("PanTilt"),
            ..plain("Tilt", "T", "Position.PanTilt", "Angle")
        },
        "zoom" => plain("Zoom", "Zoom", "Focus.Focus", "Angle"),
        "focus" => plain("Focus1", "Focus1", "Focus.Focus", "None"),
        "gobo" => Attribute {
            activation: Some("Gobo1"),
            ..plain("Gobo1", "G1", "Gobo.Gobo", "None")
        },
        "ct" => plain("CTC", "CTC", "Color.Color", "Temperature"),
        "cto" => plain("CTO", "CTO", "Color.Color", "Temperature"),
        "ctb" => plain("CTB", "CTB", "Color.Color", "Temperature"),
        "prism" => Attribute {
            activation: Some("Prism"),
            ..plain("Prism1", "Prism1", "Beam.Beam", "None")
        },
        "frost" => plain("Frost1", "Frost1", "Beam.Beam", "None"),
        "iris" => plain("Iris", "Iris", "Beam.Beam", "None"),
        "effects" => plain("Effects1", "FX1", "Beam.Beam", "None"),
        "strobe" => plain("Shutter1", "Sh1", "Beam.Beam", "None"),
        _ => return None,
    })
}

/// The two strobe functions Annex B hangs off `Shutter1`, declared beside
/// it whenever a strobe channel's functions use them.
fn shutter_function_attribute(function: &str) -> Option<Attribute> {
    let strobe = |name: &str, pretty: &str| Attribute {
        name: name.to_string(),
        pretty: pretty.to_string(),
        feature: "Beam.Beam",
        activation: None,
        unit: "Frequency",
        color: None,
        main: Some("Shutter1"),
    };
    match function {
        "strobe" => Some(strobe("Shutter1Strobe", "Strobe1")),
        "strobe_random" => Some(strobe("Shutter1StrobeRandom", "Random1")),
        _ => None,
    }
}

/// The attribute a channel is exported under.
fn attribute_for(channel: &str, def: &ChannelDef) -> Attribute {
    canonical(channel).unwrap_or_else(|| Attribute {
        name: pascal(channel),
        pretty: channel.to_string(),
        feature: "Control.Control",
        activation: None,
        unit: match def.range.map(|r| r.unit) {
            Some(PhysicalUnit::Degrees) => "Angle",
            Some(PhysicalUnit::Hertz) => "Frequency",
            None => "None",
        },
        color: None,
        main: None,
    })
}

/// `some_name:section` → `SomeNameSection`, a legal attribute name.
fn pascal(name: &str) -> String {
    let mut out = String::new();
    let mut upper = true;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(if upper { c.to_ascii_uppercase() } else { c });
            upper = false;
        } else {
            upper = true;
        }
    }
    if out.is_empty() {
        "Custom".to_string()
    } else {
        out
    }
}

/// A value held to the spec's `Name` charset (Annex C): in the first 128
/// code points only letters, digits and a short list of punctuation —
/// notably not `.` or `,`, which are the node-path and matrix separators.
/// Everything above 127 is allowed as is. Offenders become `_`.
pub fn gdtf_name(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if !c.is_ascii() || c.is_ascii_alphanumeric() || " \"#%'()*+-/:;<=>@_`".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Generates the `.gdtf` archive bytes for a fixture type.
pub fn generate(fixture_type: &FixtureType) -> Result<Vec<u8>, Box<dyn Error>> {
    let xml = description(fixture_type)?;
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        writer.start_file("description.xml", options)?;
        writer.write_all(xml.as_bytes())?;
        writer.finish()?;
    }
    Ok(cursor.into_inner())
}

/// The archive's name by the spec's convention, `<Manufacturer>@<Name>`,
/// held to what every filesystem a console runs on accepts: letters,
/// digits, space, `-`, `_`, `@`, `.`.
pub fn archive_name(fixture_type: &FixtureType) -> String {
    let stem: String = fixture_type
        .name()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("mtrack@{}.gdtf", stem.trim())
}

/// Everything the channels of one geometry contribute: their attribute
/// definitions and their `DMXChannel` elements.
#[derive(Default)]
struct Emitted {
    attributes: Vec<Attribute>,
    channels: String,
}

impl Emitted {
    /// Declares an attribute, returning the name it went in under. The
    /// same definition again is the same attribute; a different one with
    /// the same name (say `gobo-wheel` and `gobo_wheel`, both
    /// `GoboWheel`) is suffixed, or the distiller would read the second
    /// channel back as a mirror of the first. Every attribute goes
    /// through here, the shutter's function attributes included.
    fn declare(&mut self, mut attribute: Attribute) -> String {
        let base = attribute.name.clone();
        let mut n = 1;
        loop {
            match self.attributes.iter().find(|a| a.name == attribute.name) {
                Some(existing) if existing.pretty == attribute.pretty => {
                    return existing.name.clone();
                }
                Some(_) => {
                    n += 1;
                    attribute.name = format!("{base}{n}");
                }
                None => {
                    let name = attribute.name.clone();
                    self.attributes.push(attribute);
                    return name;
                }
            }
        }
    }
}

/// The DMX byte a physical value of zero lands on within a channel's
/// range — the centre of a symmetric pan or tilt travel.
fn byte_at_physical_zero(def: &ChannelDef) -> u8 {
    match def.range {
        Some(r) if (r.to - r.from).abs() > f64::EPSILON => {
            let fraction = ((0.0 - r.from) / (r.to - r.from)).clamp(0.0, 1.0);
            (fraction * 255.0).round() as u8
        }
        _ => 128,
    }
}

/// Emits one channel on `geometry`.
fn emit_channel(out: &mut Emitted, geometry: &str, name: &str, def: &ChannelDef) {
    let attribute = attribute_for(name, def);
    let is_dimmer = attribute.name == "Dimmer";
    let is_color = attribute.feature == "Color.RGB";
    let is_pose = attribute.name == "Pan" || attribute.name == "Tilt";
    let is_shutter = attribute.name == "Shutter1";
    let attribute_name = out.declare(attribute);

    let offset = match def.fine {
        Some(fine) => format!("{},{fine}", def.offset),
        None => def.offset.to_string(),
    };
    let physical = |from: f64, to: f64| {
        format!(
            " PhysicalFrom=\"{}\" PhysicalTo=\"{}\"",
            super::num(from),
            super::num(to)
        )
    };

    // The functions, in DMX order, with the spec's rule that one ends
    // where the next starts: a gap between two authored ranges is filled
    // with a `NoFeature` function so the ranges survive the round trip.
    struct Fn_ {
        name: String,
        attribute: String,
        from: u8,
        physical: String,
    }
    let mut functions: Vec<Fn_> = Vec::new();
    if def.functions.is_empty() {
        let range = def
            .range
            .map(|r| physical(r.from, r.to))
            .unwrap_or_else(|| physical(0.0, 1.0));
        functions.push(Fn_ {
            name: gdtf_name(name),
            attribute: attribute_name.clone(),
            from: 0,
            physical: range,
        });
    } else {
        let mut sorted: Vec<&crate::lighting::types::ChannelFunction> =
            def.functions.iter().collect();
        sorted.sort_by_key(|f| f.dmx_from);
        let mut gaps = 0;
        for (i, function) in sorted.iter().enumerate() {
            let function_attribute = if is_shutter {
                shutter_function_attribute(&function.name)
                    .map(|a| out.declare(a))
                    .unwrap_or_else(|| attribute_name.clone())
            } else {
                attribute_name.clone()
            };
            functions.push(Fn_ {
                name: gdtf_name(&function.name),
                attribute: function_attribute,
                from: function.dmx_from,
                physical: function
                    .physical
                    .map(|r| physical(r.from, r.to))
                    .unwrap_or_default(),
            });
            let next_from = sorted.get(i + 1).map(|f| f.dmx_from);
            if let Some(next_from) = next_from {
                if function.dmx_to < u8::MAX && function.dmx_to + 1 < next_from {
                    gaps += 1;
                    functions.push(Fn_ {
                        name: format!("unused {gaps}"),
                        attribute: "NoFeature".to_string(),
                        from: function.dmx_to + 1,
                        physical: String::new(),
                    });
                }
            }
        }
    }

    // Where a console's "home" and "highlight" put the channel: a dimmer
    // and a colour dark at home and full when highlighted, a mover at the
    // centre of its travel, a shutter on its `open` function.
    let default = if is_pose {
        byte_at_physical_zero(def)
    } else if is_shutter {
        functions
            .iter()
            .find(|f| f.name == "open")
            .map(|f| f.from)
            .unwrap_or(0)
    } else {
        0
    };
    let highlight = if is_dimmer || is_color {
        "255/1".to_string()
    } else {
        "None".to_string()
    };
    // The initial function is the one that holds the default byte.
    let initial = functions
        .iter()
        .rfind(|f| f.from <= default)
        .or(functions.first())
        .map(|f| f.name.clone())
        .unwrap_or_default();

    out.channels.push_str(&format!(
        "          <DMXChannel DMXBreak=\"1\" Offset=\"{offset}\" Geometry=\"{g}\" Highlight=\"{highlight}\" InitialFunction=\"{g}_{a}.{a}.{initial}\">\n            <LogicalChannel Attribute=\"{a}\" Snap=\"No\" Master=\"None\" DMXChangeTimeLimit=\"0\">\n",
        g = escape(geometry),
        a = escape(&attribute_name),
        initial = escape(&initial),
    ));
    for function in &functions {
        // Byte-mirroring notation on the coarse byte: `v/1` means the
        // same coarse value on every wider channel, which is exactly the
        // 8-bit sub-range the type authored. A function's default is what
        // it goes to when a console picks it, so it lies in its own range:
        // the channel's home byte for a channel that is one function, else
        // the function's start.
        let own_default = if functions.len() == 1 {
            default
        } else {
            function.from
        };
        out.channels.push_str(&format!(
            "              <ChannelFunction Name=\"{}\" Attribute=\"{}\" DMXFrom=\"{from}/1\" Default=\"{own_default}/1\"{physical}/>\n",
            escape(&function.name),
            escape(&function.attribute),
            from = function.from,
            physical = function.physical,
        ));
    }
    out.channels
        .push_str("            </LogicalChannel>\n          </DMXChannel>\n");
}

/// Sorted by offset, then name, so the file reads in DMX order.
fn in_dmx_order(defs: &HashMap<String, ChannelDef>) -> Vec<(&String, &ChannelDef)> {
    let mut channels: Vec<(&String, &ChannelDef)> = defs.iter().collect();
    channels.sort_by_key(|(name, def)| (def.offset, (*name).clone()));
    channels
}

/// The lowest byte a cell's channels occupy.
fn first_byte(cell: &Cell) -> u16 {
    cell.channels.values().map(|d| d.offset).min().unwrap_or(1)
}

/// Every channel's byte relative to the cell's first, in name order: two
/// cells with the same layout have the same shape.
fn cell_shape(cell: &Cell) -> Vec<(String, u16, Option<u16>)> {
    let base = first_byte(cell);
    let mut shape: Vec<(String, u16, Option<u16>)> = cell
        .channels
        .iter()
        .map(|(name, def)| (name.clone(), def.offset - base, def.fine.map(|f| f - base)))
        .collect();
    shape.sort();
    shape
}

/// The `description.xml` for a fixture type. Fails only for a pixel
/// fixture whose cells are not laid out alike: a GDTF reference shifts a
/// whole cell by one offset, so a cell whose bytes sit differently from
/// the first's has no honest place in the file.
pub fn description(fixture_type: &FixtureType) -> Result<String, String> {
    let cells = fixture_type.cells();
    if let Some(first) = cells.first() {
        let shape = cell_shape(first);
        for cell in &cells[1..] {
            if cell_shape(cell) != shape {
                return Err(format!(
                    "fixture type \"{}\": cell \"{}\" lays its channels out differently from \
                     cell \"{}\"; a GDTF shifts a whole cell by one offset, so every cell must \
                     have the same layout to be exported",
                    fixture_type.name(),
                    cell.name,
                    first.name
                ));
            }
        }
    }
    let cell_owned: BTreeSet<&String> = cells
        .first()
        .map(|c| c.channels.keys().collect())
        .unwrap_or_default();

    let mut emitted = Emitted::default();
    for (name, def) in in_dmx_order(fixture_type.channel_defs()) {
        if cell_owned.contains(name) {
            continue;
        }
        emit_channel(&mut emitted, "Body", name, def);
    }
    // A pixel fixture's cells: one template geometry carrying the cell
    // channels at the first cell's bytes, and one reference per cell
    // whose Break shifts them — the structure the distiller reads back
    // into the same cells (design §17.2).
    let mut geometry_xml = String::new();
    if let Some(first) = cells.first() {
        for (name, def) in in_dmx_order(&first.channels) {
            emit_channel(&mut emitted, "Cell", name, def);
        }
        let base = first_byte(first);
        let mut references = String::new();
        for cell in cells {
            let offset = first_byte(cell).saturating_sub(base) + 1;
            references.push_str(&format!(
                "        <GeometryReference Name=\"{}\" Geometry=\"Cell\" Position=\"{{1,0,0,{x}}}{{0,1,0,{y}}}{{0,0,1,{z}}}{{0,0,0,1}}\">\n          <Break DMXBreak=\"1\" DMXOffset=\"{offset}\"/>\n        </GeometryReference>\n",
                escape(&gdtf_name(&cell.name)),
                x = super::num(cell.offset[0]),
                y = super::num(cell.offset[1]),
                z = super::num(cell.offset[2]),
            ));
        }
        geometry_xml.push_str(&format!(
            "      <Geometry Name=\"Body\">\n{references}      </Geometry>\n      <Geometry Name=\"Cell\"/>\n"
        ));
    } else {
        geometry_xml.push_str("      <Geometry Name=\"Body\"/>\n");
    }

    // The feature groups and activation groups the attributes use, and
    // nothing more.
    let mut features: Vec<(&str, &str)> = emitted
        .attributes
        .iter()
        .filter_map(|a| a.feature.split_once('.'))
        .collect();
    features.sort();
    features.dedup();
    let mut feature_xml = String::new();
    let mut group: Option<&str> = None;
    for (g, f) in &features {
        if group != Some(g) {
            if group.is_some() {
                feature_xml.push_str("        </FeatureGroup>\n");
            }
            feature_xml.push_str(&format!(
                "        <FeatureGroup Name=\"{g}\" Pretty=\"{g}\">\n"
            ));
            group = Some(g);
        }
        feature_xml.push_str(&format!("          <Feature Name=\"{f}\"/>\n"));
    }
    if group.is_some() {
        feature_xml.push_str("        </FeatureGroup>\n");
    }
    let mut activations: Vec<&str> = emitted
        .attributes
        .iter()
        .filter_map(|a| a.activation)
        .collect();
    activations.sort();
    activations.dedup();
    let activation_xml: String = activations
        .iter()
        .map(|a| format!("        <ActivationGroup Name=\"{a}\"/>\n"))
        .collect();
    let mut attribute_xml = String::new();
    for a in &emitted.attributes {
        attribute_xml.push_str(&format!(
            "        <Attribute Name=\"{}\" Pretty=\"{}\"{}{} Feature=\"{}\" PhysicalUnit=\"{}\"{}/>\n",
            escape(&a.name),
            escape(&a.pretty),
            a.activation
                .map(|g| format!(" ActivationGroup=\"{g}\""))
                .unwrap_or_default(),
            a.main
                .map(|m| format!(" MainAttribute=\"{m}\""))
                .unwrap_or_default(),
            a.feature,
            a.unit,
            a.color.map(|c| format!(" Color=\"{c}\"")).unwrap_or_default(),
        ));
    }

    let name = gdtf_name(fixture_type.name());
    let mut short: String = name
        .split(|c: char| !c.is_alphanumeric())
        .filter_map(|word| word.chars().next())
        .take(8)
        .collect();
    if short.is_empty() {
        short = "FT".to_string();
    }
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <GDTF DataVersion=\"1.2\">\n\
         \x20 <FixtureType Name=\"{name}\" ShortName=\"{short}\" LongName=\"{name}\" Manufacturer=\"mtrack\" \
         Description=\"Generated by mtrack from the fixture type &quot;{name}&quot;: its channels and cells. No physical model: for one, import the manufacturer's GDTF.\" \
         FixtureTypeID=\"{id}\" Thumbnail=\"\" RefFT=\"\" CanHaveChildren=\"No\">\n\
         \x20   <AttributeDefinitions>\n\
         \x20     <ActivationGroups>\n{activation_xml}\
         \x20     </ActivationGroups>\n\
         \x20     <FeatureGroups>\n{feature_xml}\
         \x20     </FeatureGroups>\n\
         \x20     <Attributes>\n{attribute_xml}\
         \x20     </Attributes>\n\
         \x20   </AttributeDefinitions>\n\
         \x20   <Wheels/>\n\
         \x20   <PhysicalDescriptions/>\n\
         \x20   <Models/>\n\
         \x20   <Geometries>\n{geometry_xml}\
         \x20   </Geometries>\n\
         \x20   <DMXModes>\n\
         \x20     <DMXMode Name=\"{mode}\" Geometry=\"Body\">\n\
         \x20       <DMXChannels>\n{channel_xml}\
         \x20       </DMXChannels>\n\
         \x20       <Relations/>\n\
         \x20       <FTMacros/>\n\
         \x20     </DMXMode>\n\
         \x20   </DMXModes>\n\
         \x20   <Revisions>\n\
         \x20     <Revision Text=\"Generated by mtrack {version} from the fixture type &quot;{name}&quot;. Channels and cells only; no physical model.\" Date=\"{date}\" UserID=\"0\" ModifiedBy=\"mtrack {version}\"/>\n\
         \x20   </Revisions>\n\
         \x20   <FTPresets/>\n\
         \x20   <Protocols/>\n\
         \x20 </FixtureType>\n\
         </GDTF>\n",
        name = escape(&name),
        short = escape(&short),
        id = super::uuid("mtrack-gdtf", "type", fixture_type.name()),
        mode = MODE_NAME,
        channel_xml = emitted.channels,
        version = env!("CARGO_PKG_VERSION"),
        date = revision_date(),
    ))
}

/// Now, as the spec's `Date` (`yyyy-mm-ddThh:mm:ss`, UTC). Civil date
/// from days since the epoch, so no calendar dependency is needed.
fn revision_date() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let (h, m, s) = ((secs % 86_400) / 3600, (secs % 3600) / 60, secs % 60);
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if mo <= 2 { y + 1 } else { y };
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{m:02}:{s:02}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::gdtf;
    use crate::lighting::parser::parse_fixture_types;

    fn strict(xml: &str) {
        let problems = gdtf::strict::check(xml);
        assert!(problems.is_empty(), "not valid GDTF: {problems:#?}");
    }

    #[test]
    fn a_rich_native_type_distills_back_to_itself() {
        let dsl = "fixture_type \"Mover\" {\n  channel \"pan\" @ 1 fine 2 range -270deg..270deg\n  channel \"tilt\" @ 3 fine 4 range -135deg..135deg\n  channel \"dimmer\" @ 5\n  channel \"strobe\" @ 6 {\n    function \"open\" 0..15\n    function \"strobe\" 32..255 1hz..25hz\n  }\n  channel \"red\" @ 7\n  channel \"gobo:wheel\" @ 8\n}\n";
        let types = parse_fixture_types(dsl).unwrap();
        let mover = &types["Mover"];
        let xml = description(mover).unwrap();
        strict(&xml);
        let bytes = generate(mover).unwrap();
        let description = gdtf::parse_archive(&bytes).unwrap();
        assert_eq!(description.name, "Mover");
        assert_eq!(description.modes.len(), 1);
        assert_eq!(description.modes[0].name, MODE_NAME);
        let distilled = gdtf::distill(&description, MODE_NAME, "Mover").unwrap();
        let back = distilled.fixture_type;
        assert_eq!(back.channels()["pan"], 1);
        assert_eq!(back.channel_defs()["pan"].fine, Some(2));
        let range = back.channel_defs()["pan"].range.unwrap();
        assert_eq!((range.from, range.to), (-270.0, 270.0));
        assert_eq!(back.channels()["tilt"], 3);
        assert_eq!(back.channels()["dimmer"], 5);
        assert_eq!(back.channels()["strobe"], 6);
        assert_eq!(back.max_strobe_frequency(), Some(25.0));
        // The gap between `open` and `strobe` is a NoFeature function in
        // the file and no function at all once read back.
        let strobe = &back.channel_defs()["strobe"];
        let names: Vec<&str> = strobe.functions.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["open", "strobe"], "{:?}", strobe.functions);
        assert_eq!(
            (strobe.functions[0].dmx_from, strobe.functions[0].dmx_to),
            (0, 15)
        );
        assert_eq!(
            (strobe.functions[1].dmx_from, strobe.functions[1].dmx_to),
            (32, 255)
        );
        assert_eq!(back.channels()["red"], 7);
        assert!(
            back.channels().contains_key("gobowheel") || back.channels().contains_key("gobo_wheel"),
            "{:?}",
            back.channels()
        );
        // Annex B, not one bank of "Control": pan and tilt share their
        // activation group, the strobe functions hang off Shutter1, and
        // the mover's home is the centre of its travel.
        assert!(xml.contains("Name=\"Pan\" Pretty=\"P\" ActivationGroup=\"PanTilt\" Feature=\"Position.PanTilt\" PhysicalUnit=\"Angle\""));
        assert!(xml.contains("Name=\"Shutter1Strobe\" Pretty=\"Strobe1\" MainAttribute=\"Shutter1\" Feature=\"Beam.Beam\" PhysicalUnit=\"Frequency\""));
        assert!(xml.contains("Name=\"ColorAdd_R\" Pretty=\"R\" ActivationGroup=\"ColorRGB\" Feature=\"Color.RGB\" PhysicalUnit=\"ColorComponent\" Color=\"0.64,0.33,21.3\""));
        assert!(xml.contains("Offset=\"1,2\" Geometry=\"Body\" Highlight=\"None\" InitialFunction=\"Body_Pan.Pan.pan\""));
        assert!(xml.contains("Name=\"pan\" Attribute=\"Pan\" DMXFrom=\"0/1\" Default=\"128/1\""));
        assert!(xml.contains("InitialFunction=\"Body_Shutter1.Shutter1.open\""));
        assert!(xml.contains("Name=\"unused 1\" Attribute=\"NoFeature\" DMXFrom=\"16/1\""));
        // A function's default is in its own range: picking "strobe" on a
        // console must not reselect "open".
        assert!(xml.contains(
            "Name=\"strobe\" Attribute=\"Shutter1Strobe\" DMXFrom=\"32/1\" Default=\"32/1\""
        ));
        assert!(xml.contains("<Revision Text=\"Generated by mtrack "));
        // Only the custom channel lands in the generic control group.
        assert_eq!(xml.matches("Feature=\"Control.Control\"").count(), 1);
        assert!(
            xml.contains("Name=\"GoboWheel\" Pretty=\"gobo:wheel\" Feature=\"Control.Control\"")
        );
    }

    #[test]
    fn colliding_custom_names_stay_distinct_channels() {
        let dsl = "fixture_type \"Odd\" {\n  channel \"gobo-wheel\" @ 1\n  channel \"gobo_wheel\" @ 2\n  channel \"dimmer\" @ 3 fine 4 {\n    function \"off\" 0..9\n    function \"on\" 10..255\n  }\n}\n";
        let types = parse_fixture_types(dsl).unwrap();
        let xml = description(&types["Odd"]).unwrap();
        strict(&xml);
        assert!(xml.contains("Attribute=\"GoboWheel\""), "first attribute");
        assert!(
            xml.contains("Attribute=\"GoboWheel2\""),
            "second attribute suffixed"
        );
        assert!(
            xml.contains("Feature=\"Control.Control\""),
            "a custom channel"
        );
        // A 16-bit channel's functions are written on the coarse byte in
        // byte-mirroring notation, as the spec reads `v/1`.
        assert!(xml.contains("DMXFrom=\"10/1\""), "coarse-byte DMXFrom");
        let description = gdtf::parse_archive(&generate(&types["Odd"]).unwrap()).unwrap();
        let back = gdtf::distill(&description, MODE_NAME, "Odd")
            .unwrap()
            .fixture_type;
        assert_eq!(back.channels().len(), 3, "{:?}", back.channels());
        assert_eq!(back.channel_defs()["dimmer"].fine, Some(4));
        assert_eq!(back.channel_defs()["dimmer"].functions[1].dmx_from, 10);
    }

    /// A hand-written pixel bar exports its cells as a template geometry
    /// with one reference per cell, and comes back as the same cells at
    /// the same bytes with the same offsets — and the footprint a console
    /// patches is the whole bar, not its first cell.
    #[test]
    fn a_cell_bar_round_trips_through_its_geometry_references() {
        let dsl = "fixture_type \"Bar\" {\n  channel \"dimmer\" @ 1\n  \
                   cell \"1\" at (-0.3, 0, 0) { channel \"red\" @ 2  channel \"green\" @ 3  channel \"blue\" @ 4 }\n  \
                   cell \"2\" at (0, 0, 0) { channel \"red\" @ 5  channel \"green\" @ 6  channel \"blue\" @ 7 }\n  \
                   cell \"3\" at (0.3, 0, 0) { channel \"red\" @ 8  channel \"green\" @ 9  channel \"blue\" @ 10 }\n}\n";
        let types = parse_fixture_types(dsl).unwrap();
        let bar = &types["Bar"];
        assert_eq!(bar.footprint(), 10);
        let xml = description(bar).unwrap();
        strict(&xml);
        assert!(xml.contains("<GeometryReference Name=\"2\" Geometry=\"Cell\" Position=\"{1,0,0,0}{0,1,0,0}{0,0,1,0}{0,0,0,1}\">\n          <Break DMXBreak=\"1\" DMXOffset=\"4\"/>"), "expected text missing from the description");
        let description = gdtf::parse_archive(&generate(bar).unwrap()).unwrap();
        let back = gdtf::distill(&description, MODE_NAME, "Bar")
            .unwrap()
            .fixture_type;
        assert_eq!(back.footprint(), 10, "{:?}", back.channel_defs());
        let cells = back.cells();
        assert_eq!(
            cells.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
            ["1", "2", "3"]
        );
        assert_eq!(cells[2].channels["blue"].offset, 10);
        assert!(
            (cells[0].offset[0] + 0.3).abs() < 1e-9,
            "{:?}",
            cells[0].offset
        );
        assert_eq!(back.channels()["dimmer"], 1);
        assert_eq!(
            back.channel_defs()["red"].mirrors,
            vec![(5, None), (8, None)]
        );
    }

    #[test]
    fn names_are_held_to_the_spec_charset() {
        assert_eq!(gdtf_name("Wash, Front 1.2"), "Wash_ Front 1_2");
        assert_eq!(gdtf_name("移动灯 (A)"), "移动灯 (A)");
        let types = parse_fixture_types(
            "fixture_type \"Wash, Front 1.2\" {\n  channel \"dimmer\" @ 1\n}\n",
        )
        .unwrap();
        let xml = description(&types["Wash, Front 1.2"]).unwrap();
        strict(&xml);
        assert!(
            xml.contains("<FixtureType Name=\"Wash_ Front 1_2\" ShortName=\"WF12\""),
            "expected text missing from the description"
        );
        // The archive's name is held to what a console's filesystem takes,
        // which is stricter than the spec's Name charset.
        assert_eq!(
            archive_name(&types["Wash, Front 1.2"]),
            "mtrack@Wash_ Front 1_2.gdtf"
        );
        let odd = parse_fixture_types(
            "fixture_type \"Chauvet: Slim<PAR>/1\" {\n  channel \"dimmer\" @ 1\n}\n",
        )
        .unwrap();
        assert_eq!(
            archive_name(&odd["Chauvet: Slim<PAR>/1"]),
            "mtrack@Chauvet_ Slim_PAR__1.gdtf"
        );
    }

    /// A GDTF reference shifts a whole cell by one offset, so a cell laid
    /// out unlike the first cannot be written honestly: the export refuses
    /// rather than address the wrong byte.
    #[test]
    fn cells_laid_out_differently_are_refused() {
        let dsl = "fixture_type \"Bar\" {\n  \
                   cell \"1\" at (-0.3, 0, 0) { channel \"red\" @ 2  channel \"green\" @ 3  channel \"blue\" @ 4 }\n  \
                   cell \"2\" at (0.3, 0, 0) { channel \"red\" @ 10  channel \"green\" @ 11  channel \"blue\" @ 13 }\n}\n";
        let types = parse_fixture_types(dsl).unwrap();
        let err = description(&types["Bar"]).unwrap_err();
        assert!(
            err.contains("cell \"2\" lays its channels out differently"),
            "{err}"
        );
        assert!(generate(&types["Bar"]).is_err());
    }

    /// A custom channel that happens to spell a shutter function's
    /// attribute does not capture it: the real one is declared apart.
    #[test]
    fn a_custom_channel_cannot_capture_a_shutter_function_attribute() {
        let dsl = "fixture_type \"Odd\" {\n  channel \"shutter1 strobe\" @ 1\n  channel \"strobe\" @ 2 {\n    function \"open\" 0..15\n    function \"strobe\" 16..255 1hz..25hz\n  }\n}\n";
        let types = parse_fixture_types(dsl).unwrap();
        let xml = description(&types["Odd"]).unwrap();
        assert!(
            xml.contains(
                "Name=\"Shutter1Strobe\" Pretty=\"shutter1 strobe\" Feature=\"Control.Control\""
            ),
            "expected text missing from the description"
        );
        assert!(
            xml.contains("Name=\"Shutter1Strobe2\" Pretty=\"Strobe1\" MainAttribute=\"Shutter1\""),
            "expected text missing from the description"
        );
        assert!(
            xml.contains("Attribute=\"Shutter1Strobe2\" DMXFrom=\"16/1\""),
            "expected text missing from the description"
        );
        assert!(gdtf::strict::check(&xml).is_empty());
    }
}
