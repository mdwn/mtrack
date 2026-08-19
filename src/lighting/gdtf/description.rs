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

//! Streaming parser for the subset of `description.xml` mtrack consumes.
//!
//! The subset (venue-exchange design §5): fixture identity, DMX modes with
//! their channels, logical channels, and channel functions (offsets, DMX
//! starts, physical ranges), plus the names of geometry references so the
//! distiller can recognize multi-instance modes. Wheels, models, emitters,
//! presets, protocols, and revisions are passed over without being modeled.
//!
//! quick-xml performs no DTD processing or custom entity expansion, and the
//! walk enforces a nesting-depth cap — the input is a stranger's file.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::GdtfError;

/// The deepest element nesting accepted. Real descriptions sit around ten
/// levels; a thousand-deep document is an attack, not a fixture.
const MAX_DEPTH: usize = 64;

/// The parsed subset of a GDTF description.
#[derive(Debug)]
pub struct Description {
    /// The fixture type's name.
    pub name: String,
    /// The manufacturer.
    pub manufacturer: String,
    /// The DMX modes.
    pub modes: Vec<Mode>,
    /// Names of GeometryReference nodes — a mode whose channels sit on one
    /// is multi-instance (pixel bars and the like).
    pub geometry_reference_names: Vec<String>,
}

/// A DMX mode (personality).
#[derive(Debug)]
pub struct Mode {
    /// The mode's name — the string a referential fixture pins.
    pub name: String,
    /// The geometry the mode drives.
    pub geometry: String,
    /// The channels, in document order.
    pub channels: Vec<Channel>,
}

/// A DMX channel within a mode.
#[derive(Debug, Default)]
pub struct Channel {
    /// 1-based byte offsets; coarse first, fine after. Empty for a virtual
    /// channel (GDTF `Offset="None"`), which occupies no DMX footprint.
    pub offsets: Vec<u16>,
    /// The geometry this channel is attached to.
    pub geometry: String,
    /// The channel's logical channels, in document order.
    pub logical_channels: Vec<LogicalChannel>,
}

/// A logical channel: an attribute with functions over DMX sub-ranges.
#[derive(Debug, Default)]
pub struct LogicalChannel {
    /// The GDTF attribute (e.g. `ColorAdd_R`, `Pan`, `Shutter1`).
    pub attribute: String,
    /// The channel functions, in document order.
    pub functions: Vec<Function>,
}

/// A channel function: a named DMX sub-range, possibly with physical values.
#[derive(Debug, Default)]
pub struct Function {
    /// The function's name (e.g. "Variable Strobe").
    pub name: String,
    /// The function's attribute (e.g. `Shutter1Strobe`).
    pub attribute: String,
    /// The DMX value the function starts at.
    pub dmx_from: Option<DmxValue>,
    /// Physical value at the start of the range.
    pub physical_from: Option<f64>,
    /// Physical value at the end of the range.
    pub physical_to: Option<f64>,
}

/// A GDTF DMX value: `value/bytes`, e.g. `7/1` or `4294967295/4`.
#[derive(Clone, Copy, Debug)]
pub struct DmxValue {
    /// The raw value, in `bytes`-byte resolution.
    pub value: u64,
    /// The resolution the value is expressed in.
    pub bytes: u8,
}

impl DmxValue {
    /// The value's coarse (most significant) byte — what an 8-bit view of
    /// the channel sees.
    pub fn coarse(&self) -> u8 {
        (self.value >> (8 * (self.bytes.saturating_sub(1)))).min(255) as u8
    }
}

/// Parses `description.xml` content into the consumed subset.
pub fn parse_description(xml: &str) -> Result<Description, GdtfError> {
    let mut reader = Reader::from_str(xml);

    let mut description = Description {
        name: String::new(),
        manufacturer: String::new(),
        modes: Vec::new(),
        geometry_reference_names: Vec::new(),
    };

    // The element stack provides context: tags like DMXMode only mean
    // something in the right subtree, and unrelated subtrees (Wheels,
    // Models, ...) fall through every match arm untouched.
    let mut stack: Vec<String> = Vec::new();
    let mut current_mode: Option<Mode> = None;
    let mut current_channel: Option<Channel> = None;
    let mut current_logical: Option<LogicalChannel> = None;

    loop {
        let event = reader
            .read_event()
            .map_err(|e| GdtfError::new(format!("XML error in description.xml: {e}")))?;
        match event {
            Event::Start(ref element) => {
                handle_element(
                    element,
                    &stack,
                    &mut description,
                    &mut current_mode,
                    &mut current_channel,
                    &mut current_logical,
                )?;
                stack.push(element_name(element)?);
                if stack.len() > MAX_DEPTH {
                    return Err(GdtfError::new(format!(
                        "description.xml nests deeper than {MAX_DEPTH} elements"
                    )));
                }
            }
            Event::Empty(ref element) => {
                // Self-closing: open and close in one step.
                handle_element(
                    element,
                    &stack,
                    &mut description,
                    &mut current_mode,
                    &mut current_channel,
                    &mut current_logical,
                )?;
                close_element(
                    &element_name(element)?,
                    &mut description,
                    &mut current_mode,
                    &mut current_channel,
                    &mut current_logical,
                );
            }
            Event::End(_) => {
                if let Some(name) = stack.pop() {
                    close_element(
                        &name,
                        &mut description,
                        &mut current_mode,
                        &mut current_channel,
                        &mut current_logical,
                    );
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if description.name.is_empty() {
        return Err(GdtfError::new("description.xml has no FixtureType element"));
    }
    Ok(description)
}

fn element_name(element: &BytesStart<'_>) -> Result<String, GdtfError> {
    std::str::from_utf8(element.name().as_ref())
        .map(|s| s.to_string())
        .map_err(|_| GdtfError::new("non-UTF-8 element name in description.xml"))
}

/// Reads an attribute's unescaped value, if present.
fn attr(element: &BytesStart<'_>, name: &str) -> Result<Option<String>, GdtfError> {
    for attribute in element.attributes() {
        let attribute =
            attribute.map_err(|e| GdtfError::new(format!("malformed XML attribute: {e}")))?;
        if attribute.key.as_ref() == name.as_bytes() {
            let value = attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|e| GdtfError::new(format!("malformed XML attribute value: {e}")))?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

fn handle_element(
    element: &BytesStart<'_>,
    stack: &[String],
    description: &mut Description,
    current_mode: &mut Option<Mode>,
    current_channel: &mut Option<Channel>,
    current_logical: &mut Option<LogicalChannel>,
) -> Result<(), GdtfError> {
    let in_subtree = |name: &str| stack.iter().any(|s| s == name);
    match element_name(element)?.as_str() {
        "FixtureType" => {
            description.name = attr(element, "Name")?.unwrap_or_default();
            description.manufacturer = attr(element, "Manufacturer")?.unwrap_or_default();
        }
        "DMXMode" if in_subtree("DMXModes") => {
            *current_mode = Some(Mode {
                name: attr(element, "Name")?.unwrap_or_default(),
                geometry: attr(element, "Geometry")?.unwrap_or_default(),
                channels: Vec::new(),
            });
        }
        "DMXChannel" if current_mode.is_some() => {
            *current_channel = Some(Channel {
                offsets: parse_offsets(attr(element, "Offset")?.as_deref())?,
                geometry: attr(element, "Geometry")?.unwrap_or_default(),
                logical_channels: Vec::new(),
            });
        }
        "LogicalChannel" if current_channel.is_some() => {
            *current_logical = Some(LogicalChannel {
                attribute: attr(element, "Attribute")?.unwrap_or_default(),
                functions: Vec::new(),
            });
        }
        "ChannelFunction" => {
            if let Some(logical) = current_logical.as_mut() {
                logical.functions.push(Function {
                    name: attr(element, "Name")?.unwrap_or_default(),
                    attribute: attr(element, "Attribute")?.unwrap_or_default(),
                    dmx_from: attr(element, "DMXFrom")?
                        .as_deref()
                        .and_then(parse_dmx_value),
                    physical_from: parse_finite(attr(element, "PhysicalFrom")?.as_deref()),
                    physical_to: parse_finite(attr(element, "PhysicalTo")?.as_deref()),
                });
            }
        }
        "GeometryReference" if in_subtree("Geometries") => {
            if let Some(name) = attr(element, "Name")? {
                description.geometry_reference_names.push(name);
            }
        }
        _ => {}
    }
    Ok(())
}

fn close_element(
    name: &str,
    description: &mut Description,
    current_mode: &mut Option<Mode>,
    current_channel: &mut Option<Channel>,
    current_logical: &mut Option<LogicalChannel>,
) {
    match name {
        "LogicalChannel" => {
            if let (Some(channel), Some(logical)) =
                (current_channel.as_mut(), current_logical.take())
            {
                channel.logical_channels.push(logical);
            }
        }
        "DMXChannel" => {
            if let (Some(mode), Some(channel)) = (current_mode.as_mut(), current_channel.take()) {
                mode.channels.push(channel);
            }
        }
        "DMXMode" => {
            if let Some(mode) = current_mode.take() {
                description.modes.push(mode);
            }
        }
        _ => {}
    }
}

/// Parses a GDTF `Offset` attribute: comma-separated 1-based byte offsets,
/// or "None"/empty for a virtual channel.
fn parse_offsets(offset: Option<&str>) -> Result<Vec<u16>, GdtfError> {
    let Some(offset) = offset else {
        return Ok(Vec::new());
    };
    let offset = offset.trim();
    if offset.is_empty() || offset.eq_ignore_ascii_case("none") {
        return Ok(Vec::new());
    }
    offset
        .split(',')
        .map(|part| {
            let value: u16 = part
                .trim()
                .parse()
                .map_err(|_| GdtfError::new(format!("unparseable channel offset \"{offset}\"")))?;
            if !(1..=512).contains(&value) {
                return Err(GdtfError::new(format!(
                    "channel offset {value} outside the DMX universe (1..=512)"
                )));
            }
            Ok(value)
        })
        .collect()
}

/// Parses a GDTF DMX value: `value/bytes` with an optional byte-mirroring
/// `s` suffix. Only the coarse byte is consumed downstream, which mirroring
/// and shifting agree on.
fn parse_dmx_value(text: &str) -> Option<DmxValue> {
    let text = text.trim().trim_end_matches(['s', 'S']);
    let (value, bytes) = match text.split_once('/') {
        Some((value, bytes)) => (value, bytes),
        None => (text, "1"),
    };
    let value: u64 = value.trim().parse().ok()?;
    let bytes: u8 = bytes.trim().parse().ok()?;
    if bytes == 0 || bytes > 8 {
        return None;
    }
    Some(DmxValue { value, bytes })
}

/// Parses a physical value, dropping non-finite garbage.
fn parse_finite(text: Option<&str>) -> Option<f64> {
    let value: f64 = text?.trim().parse().ok()?;
    value.is_finite().then_some(value)
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A PixelBrick-shaped synthetic description: two modes, a virtual
    /// dimmer, a strobe channel with function ranges, and a 16-bit mover
    /// mode. Structure mirrors the real Astera PB15 file.
    pub(in super::super) const SYNTHETIC_DESCRIPTION: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<GDTF DataVersion="1.2">
  <FixtureType Name="Synth Brick" ShortName="Brick" Manufacturer="mtrack synthetic">
    <AttributeDefinitions/>
    <Wheels>
      <Wheel Name="IgnoredWheel"><Slot Name="Open"/></Wheel>
    </Wheels>
    <Geometries>
      <Geometry Name="Base">
        <Geometry Name="Head"/>
        <GeometryReference Name="Pixel 2" Geometry="Head"/>
      </Geometry>
    </Geometries>
    <DMXModes>
      <DMXMode Name="8: RGBS" Geometry="Base">
        <DMXChannels>
          <DMXChannel Offset="None" Geometry="Base">
            <LogicalChannel Attribute="Dimmer">
              <ChannelFunction Name="Dimmer 1" Attribute="Dimmer" DMXFrom="0/4" PhysicalFrom="0" PhysicalTo="1"/>
            </LogicalChannel>
          </DMXChannel>
          <DMXChannel Offset="1" Geometry="Base">
            <LogicalChannel Attribute="ColorAdd_R">
              <ChannelFunction Name="ColorAdd_R 1" Attribute="ColorAdd_R" DMXFrom="0/1" PhysicalFrom="0" PhysicalTo="1"/>
            </LogicalChannel>
          </DMXChannel>
          <DMXChannel Offset="2" Geometry="Base">
            <LogicalChannel Attribute="ColorAdd_G">
              <ChannelFunction Name="ColorAdd_G 1" Attribute="ColorAdd_G" DMXFrom="0/1"/>
            </LogicalChannel>
          </DMXChannel>
          <DMXChannel Offset="3" Geometry="Base">
            <LogicalChannel Attribute="ColorAdd_B">
              <ChannelFunction Name="ColorAdd_B 1" Attribute="ColorAdd_B" DMXFrom="0/1"/>
            </LogicalChannel>
          </DMXChannel>
          <DMXChannel Offset="4" Geometry="Base">
            <LogicalChannel Attribute="Shutter1">
              <ChannelFunction Name="Strobe Off" Attribute="Shutter1" DMXFrom="0/1" PhysicalFrom="1" PhysicalTo="1"/>
              <ChannelFunction Name="Random Strobe" Attribute="Shutter1StrobeRandom" DMXFrom="4/1" PhysicalFrom="25" PhysicalTo="0.4"/>
              <ChannelFunction Name="Variable Strobe" Attribute="Shutter1Strobe" DMXFrom="7/1" PhysicalFrom="0.4" PhysicalTo="25"/>
            </LogicalChannel>
          </DMXChannel>
        </DMXChannels>
      </DMXMode>
      <DMXMode Name="Mover 16bit" Geometry="Base">
        <DMXChannels>
          <DMXChannel Offset="1,2" Geometry="Head">
            <LogicalChannel Attribute="Pan">
              <ChannelFunction Name="Pan 1" Attribute="Pan" DMXFrom="0/2" PhysicalFrom="-270" PhysicalTo="270"/>
            </LogicalChannel>
          </DMXChannel>
          <DMXChannel Offset="3,4" Geometry="Head">
            <LogicalChannel Attribute="Tilt">
              <ChannelFunction Name="Tilt 1" Attribute="Tilt" DMXFrom="0/2" PhysicalFrom="-135" PhysicalTo="135"/>
            </LogicalChannel>
          </DMXChannel>
          <DMXChannel Offset="5" Geometry="Head">
            <LogicalChannel Attribute="Frobnicator">
              <ChannelFunction Name="Frob 1" Attribute="Frobnicator" DMXFrom="0/1"/>
            </LogicalChannel>
          </DMXChannel>
        </DMXChannels>
      </DMXMode>
    </DMXModes>
  </FixtureType>
</GDTF>
"#;

    #[test]
    fn parses_the_consumed_subset() {
        let description = parse_description(SYNTHETIC_DESCRIPTION).unwrap();
        assert_eq!(description.name, "Synth Brick");
        assert_eq!(description.manufacturer, "mtrack synthetic");
        assert_eq!(description.modes.len(), 2);
        assert_eq!(description.geometry_reference_names, vec!["Pixel 2"]);

        let rgbs = &description.modes[0];
        assert_eq!(rgbs.name, "8: RGBS");
        assert_eq!(rgbs.channels.len(), 5);
        assert!(rgbs.channels[0].offsets.is_empty(), "virtual dimmer");
        assert_eq!(rgbs.channels[1].offsets, vec![1]);
        let strobe = &rgbs.channels[4];
        assert_eq!(strobe.logical_channels[0].attribute, "Shutter1");
        let functions = &strobe.logical_channels[0].functions;
        assert_eq!(functions.len(), 3);
        assert_eq!(functions[2].name, "Variable Strobe");
        assert_eq!(functions[2].dmx_from.unwrap().coarse(), 7);
        assert_eq!(functions[2].physical_from, Some(0.4));
        assert_eq!(functions[2].physical_to, Some(25.0));

        let mover = &description.modes[1];
        assert_eq!(mover.channels[0].offsets, vec![1, 2]);
        assert_eq!(mover.channels[0].logical_channels[0].attribute, "Pan");
    }

    #[test]
    fn coarse_byte_of_multibyte_values() {
        assert_eq!(parse_dmx_value("7/1").unwrap().coarse(), 7);
        assert_eq!(parse_dmx_value("4294967295/4").unwrap().coarse(), 255);
        assert_eq!(parse_dmx_value("32768/2").unwrap().coarse(), 128);
        assert_eq!(parse_dmx_value("7/2s").unwrap().coarse(), 0);
        assert_eq!(parse_dmx_value("7").unwrap().coarse(), 7);
        assert!(parse_dmx_value("7/0").is_none());
        assert!(parse_dmx_value("junk").is_none());
    }

    #[test]
    fn offsets_outside_the_universe_are_rejected() {
        let err = parse_offsets(Some("513")).unwrap_err().to_string();
        assert!(err.contains("outside the DMX universe"), "{err}");
        assert!(parse_offsets(Some("None")).unwrap().is_empty());
        assert!(parse_offsets(None).unwrap().is_empty());
        assert_eq!(parse_offsets(Some("1, 2")).unwrap(), vec![1, 2]);
    }

    #[test]
    fn depth_bomb_is_rejected() {
        let mut xml = String::from("<GDTF>");
        for _ in 0..100 {
            xml.push_str("<a>");
        }
        for _ in 0..100 {
            xml.push_str("</a>");
        }
        xml.push_str("</GDTF>");
        let err = parse_description(&xml).unwrap_err().to_string();
        assert!(err.contains("nests deeper"), "{err}");
    }

    #[test]
    fn a_fixture_less_document_is_an_error() {
        let err = parse_description("<NotGdtf/>").unwrap_err().to_string();
        assert!(err.contains("no FixtureType"), "{err}");
    }
}
