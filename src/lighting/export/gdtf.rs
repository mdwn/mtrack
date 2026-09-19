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

//! A minimal GDTF for a native fixture type (design §16.4): one mode, the
//! channel definitions, a box body, no wheels or models — enough for a
//! console to patch the fixture and for mtrack's own distiller to read it
//! back as the same channels. The seed of GDTF export proper.

use std::error::Error;
use std::io::Write;

use super::escape;
use crate::lighting::types::{ChannelDef, FixtureType, PhysicalUnit};

/// The generated mode's name.
pub const MODE_NAME: &str = "mtrack";

/// The GDTF attribute mtrack's canonical channel names map back to — the
/// inverse of the distiller's table, so a generated GDTF distills to the
/// channels it came from.
fn attribute_for(channel: &str) -> String {
    match channel {
        "dimmer" => "Dimmer",
        "red" => "ColorAdd_R",
        "green" => "ColorAdd_G",
        "blue" => "ColorAdd_B",
        "white" => "ColorAdd_W",
        "warm_white" => "ColorAdd_WW",
        "cool_white" => "ColorAdd_CW",
        "uv" => "ColorAdd_UV",
        "amber" => "ColorAdd_RY",
        "pan" => "Pan",
        "tilt" => "Tilt",
        "zoom" => "Zoom",
        "focus" => "Focus1",
        "gobo" => "Gobo1",
        "ct" => "CTC",
        "cto" => "CTO",
        "ctb" => "CTB",
        "prism" => "Prism1",
        "frost" => "Frost1",
        "iris" => "Iris",
        "effects" => "Effects1",
        "strobe" => "Shutter1",
        other => return pascal(other),
    }
    .to_string()
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

/// Generates the `.gdtf` archive bytes for a fixture type.
pub fn generate(fixture_type: &FixtureType) -> Result<Vec<u8>, Box<dyn Error>> {
    let xml = description(fixture_type);
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

/// The `description.xml` for a fixture type.
pub fn description(fixture_type: &FixtureType) -> String {
    let mut channels: Vec<(&String, &ChannelDef)> = fixture_type.channel_defs().iter().collect();
    channels.sort_by_key(|(name, def)| (def.offset, (*name).clone()));

    let mut attributes: Vec<String> = Vec::new();
    let mut attribute_xml = String::new();
    let mut channel_xml = String::new();
    for (name, def) in &channels {
        // Two channels folding to one attribute name (say `gobo-wheel` and
        // `gobo_wheel`) must stay two attributes, or the distiller reads
        // the second back as a mirror of the first.
        let mut attribute = attribute_for(name);
        let base = attribute.clone();
        let mut n = 1;
        while attributes.contains(&attribute) {
            n += 1;
            attribute = format!("{base}{n}");
        }
        {
            attributes.push(attribute.clone());
            let unit = match def.range.map(|r| r.unit) {
                Some(PhysicalUnit::Degrees) => "Angle",
                Some(PhysicalUnit::Hertz) => "Frequency",
                None => "None",
            };
            attribute_xml.push_str(&format!(
                "        <Attribute Name=\"{}\" Pretty=\"{}\" Feature=\"Control.Control\" PhysicalUnit=\"{unit}\"/>\n",
                escape(&attribute),
                escape(name)
            ));
        }
        let offset = match def.fine {
            Some(fine) => format!("{},{fine}", def.offset),
            None => def.offset.to_string(),
        };
        let bytes = if def.fine.is_some() { 2 } else { 1 };
        channel_xml.push_str(&format!(
            "          <DMXChannel DMXBreak=\"1\" Offset=\"{offset}\" Geometry=\"Body\" Highlight=\"None\" InitialFunction=\"Body_{a}.{a}.{n}\">\n            <LogicalChannel Attribute=\"{a}\" Snap=\"No\" Master=\"None\" DMXChangeTimeLimit=\"0\">\n",
            a = escape(&attribute),
            n = escape(name)
        ));
        let physical = |from: f64, to: f64| {
            format!(" PhysicalFrom=\"{}\" PhysicalTo=\"{}\"", num(from), num(to))
        };
        if def.functions.is_empty() {
            let range = def
                .range
                .map(|r| physical(r.from, r.to))
                .unwrap_or_else(|| physical(0.0, 1.0));
            channel_xml.push_str(&format!(
                "              <ChannelFunction Name=\"{}\" Attribute=\"{}\" DMXFrom=\"0/{bytes}\" Default=\"0/{bytes}\"{range}/>\n",
                escape(name),
                escape(&attribute)
            ));
        } else {
            for function in &def.functions {
                // The variable-strobe function is what the distiller keys
                // on by attribute; everything else keeps the channel's.
                let function_attribute = match (attribute.as_str(), function.name.as_str()) {
                    ("Shutter1", "strobe") => "Shutter1Strobe".to_string(),
                    ("Shutter1", "strobe_random") => "Shutter1StrobeRandom".to_string(),
                    _ => attribute.clone(),
                };
                let range = function
                    .physical
                    .map(|r| physical(r.from, r.to))
                    .unwrap_or_default();
                channel_xml.push_str(&format!(
                    "              <ChannelFunction Name=\"{}\" Attribute=\"{}\" DMXFrom=\"{from}/{bytes}\" Default=\"{from}/{bytes}\"{range}/>\n",
                    escape(&function.name),
                    escape(&function_attribute),
                    from = u64::from(function.dmx_from) << (8 * (bytes - 1)),
                ));
            }
        }
        channel_xml.push_str("            </LogicalChannel>\n          </DMXChannel>\n");
    }

    let name = fixture_type.name();
    let short: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <GDTF DataVersion=\"1.2\">\n\
         \x20 <FixtureType Name=\"{name}\" ShortName=\"{short}\" LongName=\"{name}\" Manufacturer=\"mtrack\" \
         Description=\"Generated by mtrack from the native fixture type &quot;{name}&quot;: its channels, no models.\" \
         FixtureTypeID=\"{id}\" Thumbnail=\"\" RefFT=\"\" CanHaveChildren=\"No\">\n\
         \x20   <AttributeDefinitions>\n\
         \x20     <ActivationGroups/>\n\
         \x20     <FeatureGroups>\n\
         \x20       <FeatureGroup Name=\"Control\" Pretty=\"Control\">\n\
         \x20         <Feature Name=\"Control\"/>\n\
         \x20       </FeatureGroup>\n\
         \x20     </FeatureGroups>\n\
         \x20     <Attributes>\n{attribute_xml}\
         \x20     </Attributes>\n\
         \x20   </AttributeDefinitions>\n\
         \x20   <Wheels/>\n\
         \x20   <PhysicalDescriptions/>\n\
         \x20   <Models>\n\
         \x20     <Model Name=\"Body\" Length=\"0.300000\" Width=\"0.300000\" Height=\"0.300000\" PrimitiveType=\"Cube\" File=\"\"/>\n\
         \x20   </Models>\n\
         \x20   <Geometries>\n\
         \x20     <Geometry Name=\"Body\" Model=\"Body\" Position=\"{{1,0,0,0}}{{0,1,0,0}}{{0,0,1,0}}{{0,0,0,1}}\"/>\n\
         \x20   </Geometries>\n\
         \x20   <DMXModes>\n\
         \x20     <DMXMode Name=\"{mode}\" Geometry=\"Body\">\n\
         \x20       <DMXChannels>\n{channel_xml}\
         \x20       </DMXChannels>\n\
         \x20       <Relations/>\n\
         \x20       <FTMacros/>\n\
         \x20     </DMXMode>\n\
         \x20   </DMXModes>\n\
         \x20   <Revisions/>\n\
         \x20   <FTPresets/>\n\
         \x20   <Protocols/>\n\
         \x20 </FixtureType>\n\
         </GDTF>\n",
        name = escape(name),
        short = escape(&short),
        id = super::uuid("mtrack-gdtf", "type", name),
        mode = MODE_NAME,
    )
}

fn num(value: f64) -> String {
    super::num(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::gdtf;
    use crate::lighting::parser::parse_fixture_types;

    #[test]
    fn a_rich_native_type_distills_back_to_itself() {
        let dsl = "fixture_type \"Mover\" {\n  channel \"pan\" @ 1 fine 2 range -270deg..270deg\n  channel \"tilt\" @ 3 fine 4 range -135deg..135deg\n  channel \"dimmer\" @ 5\n  channel \"strobe\" @ 6 {\n    function \"open\" 0..15\n    function \"strobe\" 16..255 1hz..25hz\n  }\n  channel \"red\" @ 7\n  channel \"gobo:wheel\" @ 8\n}\n";
        let types = parse_fixture_types(dsl).unwrap();
        let mover = &types["Mover"];
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
        assert_eq!(back.channels()["red"], 7);
        assert!(
            back.channels().contains_key("gobowheel") || back.channels().contains_key("gobo_wheel"),
            "{:?}",
            back.channels()
        );
        // The rig distills too (a box body, no beam): the 3D view has
        // something to draw.
        let rig = gdtf::distill_rig(&description, MODE_NAME, &Default::default()).unwrap();
        assert_eq!(rig.nodes.len(), 1);
        assert_eq!(attribute_for("amber"), "ColorAdd_RY");
        assert_eq!(pascal("red:flower"), "RedFlower");
    }

    #[test]
    fn colliding_custom_names_stay_distinct_channels() {
        let dsl = "fixture_type \"Odd\" {\n  channel \"gobo-wheel\" @ 1\n  channel \"gobo_wheel\" @ 2\n  channel \"dimmer\" @ 3 fine 4 {\n    function \"off\" 0..9\n    function \"on\" 10..255\n  }\n}\n";
        let types = parse_fixture_types(dsl).unwrap();
        let xml = description(&types["Odd"]);
        assert!(xml.contains("Attribute=\"GoboWheel\""), "{xml}");
        assert!(xml.contains("Attribute=\"GoboWheel2\""), "{xml}");
        // A 16-bit channel's functions start in 16-bit resolution.
        assert!(xml.contains("DMXFrom=\"2560/2\""), "{xml}");
        let description = gdtf::parse_archive(&generate(&types["Odd"]).unwrap()).unwrap();
        let back = gdtf::distill(&description, MODE_NAME, "Odd")
            .unwrap()
            .fixture_type;
        assert_eq!(back.channels().len(), 3, "{:?}", back.channels());
        assert_eq!(back.channel_defs()["dimmer"].fine, Some(4));
    }
}
