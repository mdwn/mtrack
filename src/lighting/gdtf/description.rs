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
//! The subset (venue-exchange design §5, §16): fixture identity, DMX modes
//! with their channels, logical channels, and channel functions (offsets,
//! DMX starts, physical ranges), the geometry tree with its axes, beams and
//! references (the rig model distills from it), and the model table that
//! names the archive's meshes. Wheels, emitters, presets, protocols, and
//! revisions are passed over without being modeled.
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
    /// The model table: what each geometry node looks like.
    pub models: Vec<Model>,
    /// The geometry tree, flattened in document order; a node's parent
    /// precedes it. Top-level geometries have no parent.
    pub geometries: Vec<GeometryNode>,
    /// The thumbnail's file stem, when the fixture type names one (the
    /// archive then holds `<stem>.png` and/or `<stem>.svg`).
    pub thumbnail: Option<String>,
}

/// A 4×4 transform as this crate uses it: `M · v` takes a point from the
/// node's frame into its parent's, with the rotation's columns the node's
/// axes and the translation in the fourth column, meters. Not quite as
/// GDTF writes it: see [`parse_matrix`] for the reading of the file's
/// rows, which is Blender DMX's.
pub type Matrix4 = [[f64; 4]; 4];

/// The identity transform.
pub const IDENTITY: Matrix4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

/// An entry of the model table.
#[derive(Debug, Clone)]
pub struct Model {
    /// The model's name, what a geometry node's `Model` attribute names.
    pub name: String,
    /// The mesh file's stem (`models/gltf/<file>.glb` in the archive), when
    /// the model has one.
    pub file: Option<String>,
    /// The GDTF primitive standing in for a mesh (`Cube`, `Cylinder`,
    /// `Base`, `Yoke`, `Head`, `Pigtail`, ...); `Undefined` when a mesh is
    /// meant.
    pub primitive: String,
    /// Bounding size in meters: length (x), width (y), height (z).
    pub size: [f64; 3],
}

/// What a geometry node is.
#[derive(Debug, Clone, PartialEq)]
pub enum GeometryKind {
    /// A plain geometry.
    Geometry,
    /// A rotating part: pan (about its local Z) or tilt (about its local
    /// X), which the mode's `Pan`/`Tilt` channels name.
    Axis,
    /// A light source, emitting along its local −Z.
    Beam,
    /// An instance of another top-level geometry (a pixel cell, usually).
    Reference,
    /// Any other spec'd geometry type (filters, displays, structure, ...).
    Other(String),
}

/// Photometric data on a `Beam` node.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BeamData {
    /// Beam angle in degrees (the bright core).
    pub beam_angle: Option<f64>,
    /// Field angle in degrees (to 10% intensity).
    pub field_angle: Option<f64>,
    /// `Wash`, `Spot`, `None`, `Rectangle`, `PC`, `Fresnel`, `Glow`.
    pub beam_type: Option<String>,
    /// Luminous flux in lumens.
    pub luminous_flux: Option<f64>,
    /// Color temperature in kelvin.
    pub color_temperature: Option<f64>,
    /// Beam (lens) radius in meters.
    pub beam_radius: Option<f64>,
}

/// One node of the geometry tree.
#[derive(Debug, Clone)]
pub struct GeometryNode {
    /// The node's name — what channels and references point at.
    pub name: String,
    /// What the node is.
    pub kind: GeometryKind,
    /// The parent's index in [`Description::geometries`].
    pub parent: Option<usize>,
    /// The model the node is drawn with.
    pub model: Option<String>,
    /// The node's transform relative to its parent.
    pub position: Matrix4,
    /// Beam data, on a `Beam` node.
    pub beam: Option<BeamData>,
    /// The referenced top-level geometry's name, on a `GeometryReference`.
    pub reference: Option<String>,
    /// A reference's `Break` children as `(DMXBreak, DMXOffset)`: where
    /// the referenced geometry's template channels land for this
    /// instance (1-based offset added, minus one, to the template's).
    pub breaks: Vec<(String, u16)>,
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
    /// The DMX break the channel lives on: a number (`"1"` for a single-
    /// break fixture) or `"Overwrite"`, which a template channel uses to
    /// take its offset from the last `Break` of each reference.
    pub dmx_break: String,
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

/// The largest description accepted here. The archive layer enforces its
/// own cap; this one exists so a future direct caller (pasted XML, say)
/// can't bypass it.
const MAX_XML_BYTES: usize = 64 * 1024 * 1024;

/// Parses `description.xml` content into the consumed subset.
pub fn parse_description(xml: &str) -> Result<Description, GdtfError> {
    if xml.len() > MAX_XML_BYTES {
        return Err(GdtfError::new(format!(
            "description.xml is {} bytes; refusing more than {MAX_XML_BYTES}",
            xml.len()
        )));
    }
    let mut reader = Reader::from_str(xml);

    let mut walk = Walk {
        description: Description {
            name: String::new(),
            manufacturer: String::new(),
            modes: Vec::new(),
            geometry_reference_names: Vec::new(),
            models: Vec::new(),
            geometries: Vec::new(),
            thumbnail: None,
        },
        current_mode: None,
        current_channel: None,
        current_logical: None,
        geometry_stack: Vec::new(),
    };

    // The element stack provides context: tags like DMXMode only mean
    // something in the right subtree, and unrelated subtrees (Wheels,
    // Protocols, ...) fall through every match arm untouched. The geometry
    // stack runs parallel to it, holding the geometry node each element
    // opened (if any), so a node finds its parent.
    let mut stack: Vec<String> = Vec::new();

    loop {
        let event = reader
            .read_event()
            .map_err(|e| GdtfError::new(format!("XML error in description.xml: {e}")))?;
        match event {
            Event::Start(ref element) => {
                let node = walk.handle_element(element, &stack)?;
                stack.push(element_name(element)?);
                walk.geometry_stack.push(node);
                if stack.len() > MAX_DEPTH {
                    return Err(GdtfError::new(format!(
                        "description.xml nests deeper than {MAX_DEPTH} elements"
                    )));
                }
            }
            Event::Empty(ref element) => {
                // Self-closing: open and close in one step.
                walk.handle_element(element, &stack)?;
                walk.close_element(&element_name(element)?);
            }
            Event::End(_) => {
                walk.geometry_stack.pop();
                if let Some(name) = stack.pop() {
                    walk.close_element(&name);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    let description = walk.description;
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

/// The walk's state: what is open at each level of the document.
struct Walk {
    description: Description,
    current_mode: Option<Mode>,
    current_channel: Option<Channel>,
    current_logical: Option<LogicalChannel>,
    /// Parallel to the element stack: the geometry node each open element
    /// is, if it is one.
    geometry_stack: Vec<Option<usize>>,
}

/// The geometry element types the spec defines, all of which sit in the
/// tree and carry a `Position`. `Break` (under a reference) and the
/// laser/structure sub-elements are not nodes.
const GEOMETRY_ELEMENTS: &[&str] = &[
    "Geometry",
    "Axis",
    "Beam",
    "GeometryReference",
    "FilterBeam",
    "FilterColor",
    "FilterGobo",
    "FilterShaper",
    "MediaServerLayer",
    "MediaServerCamera",
    "MediaServerMaster",
    "Display",
    "Laser",
    "WiringObject",
    "Inventory",
    "Structure",
    "Support",
    "Magnet",
];

impl Walk {
    /// Handles an opening (or self-closing) element; returns the index of
    /// the geometry node it opened, if it is one.
    fn handle_element(
        &mut self,
        element: &BytesStart<'_>,
        stack: &[String],
    ) -> Result<Option<usize>, GdtfError> {
        let in_subtree = |name: &str| stack.iter().any(|s| s == name);
        let name = element_name(element)?;
        match name.as_str() {
            "FixtureType" => {
                self.description.name = attr(element, "Name")?.unwrap_or_default();
                self.description.manufacturer = attr(element, "Manufacturer")?.unwrap_or_default();
                self.description.thumbnail = attr(element, "Thumbnail")?
                    .map(|t| t.trim().to_string())
                    .filter(|t| !t.is_empty());
            }
            "Model" if in_subtree("Models") => {
                self.description.models.push(Model {
                    name: attr(element, "Name")?.unwrap_or_default(),
                    file: attr(element, "File")?
                        .map(|f| f.trim().to_string())
                        .filter(|f| !f.is_empty()),
                    primitive: attr(element, "PrimitiveType")?
                        .unwrap_or_else(|| "Undefined".to_string()),
                    // A size is a length; a negative one is garbage, not
                    // a mirror.
                    size: [
                        parse_positive(attr(element, "Length")?.as_deref()),
                        parse_positive(attr(element, "Width")?.as_deref()),
                        parse_positive(attr(element, "Height")?.as_deref()),
                    ],
                });
            }
            "DMXMode" if in_subtree("DMXModes") => {
                self.current_mode = Some(Mode {
                    name: attr(element, "Name")?.unwrap_or_default(),
                    geometry: attr(element, "Geometry")?.unwrap_or_default(),
                    channels: Vec::new(),
                });
            }
            "DMXChannel" if self.current_mode.is_some() => {
                self.current_channel = Some(Channel {
                    offsets: parse_offsets(attr(element, "Offset")?.as_deref())?,
                    geometry: attr(element, "Geometry")?.unwrap_or_default(),
                    dmx_break: attr(element, "DMXBreak")?
                        .map(|b| b.trim().to_string())
                        .filter(|b| !b.is_empty())
                        .unwrap_or_else(|| "1".to_string()),
                    logical_channels: Vec::new(),
                });
            }
            "LogicalChannel" if self.current_channel.is_some() => {
                self.current_logical = Some(LogicalChannel {
                    attribute: attr(element, "Attribute")?.unwrap_or_default(),
                    functions: Vec::new(),
                });
            }
            "ChannelFunction" => {
                if let Some(logical) = self.current_logical.as_mut() {
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
            kind if in_subtree("Geometries") && GEOMETRY_ELEMENTS.contains(&kind) => {
                return self.geometry_node(kind, element).map(Some);
            }
            "Break" if in_subtree("Geometries") => {
                // A reference's DMX break: the offset its template
                // channels take. The nearest open node is the reference.
                let parent = self.geometry_stack.iter().rev().find_map(|n| *n);
                if let Some(index) = parent {
                    let node = &mut self.description.geometries[index];
                    if node.kind == GeometryKind::Reference {
                        let dmx_break = attr(element, "DMXBreak")?
                            .map(|b| b.trim().to_string())
                            .unwrap_or_else(|| "1".to_string());
                        if let Some(offset) = attr(element, "DMXOffset")?
                            .and_then(|o| o.trim().parse::<u16>().ok())
                            .filter(|o| *o >= 1)
                        {
                            node.breaks.push((dmx_break, offset));
                        }
                    }
                }
            }
            _ => {}
        }
        Ok(None)
    }

    /// Records a geometry node under the nearest open node.
    fn geometry_node(&mut self, kind: &str, element: &BytesStart<'_>) -> Result<usize, GdtfError> {
        let name = attr(element, "Name")?.unwrap_or_default();
        let kind = match kind {
            "Geometry" => GeometryKind::Geometry,
            "Axis" => GeometryKind::Axis,
            "Beam" => GeometryKind::Beam,
            "GeometryReference" => GeometryKind::Reference,
            other => GeometryKind::Other(other.to_string()),
        };
        let beam = (kind == GeometryKind::Beam).then(|| BeamData {
            beam_angle: parse_finite(attr(element, "BeamAngle").ok().flatten().as_deref()),
            field_angle: parse_finite(attr(element, "FieldAngle").ok().flatten().as_deref()),
            beam_type: attr(element, "BeamType").ok().flatten(),
            luminous_flux: parse_finite(attr(element, "LuminousFlux").ok().flatten().as_deref()),
            color_temperature: parse_finite(
                attr(element, "ColorTemperature").ok().flatten().as_deref(),
            ),
            beam_radius: parse_finite(attr(element, "BeamRadius").ok().flatten().as_deref()),
        });
        let reference = if kind == GeometryKind::Reference {
            let referenced = attr(element, "Geometry")?;
            self.description.geometry_reference_names.push(name.clone());
            referenced
        } else {
            None
        };
        let parent = self.geometry_stack.iter().rev().find_map(|n| *n);
        let position = match attr(element, "Position")? {
            Some(text) => parse_matrix(&text).ok_or_else(|| {
                GdtfError::new(format!("geometry \"{name}\" has an unparseable Position"))
            })?,
            None => IDENTITY,
        };
        self.description.geometries.push(GeometryNode {
            name,
            kind,
            parent,
            model: attr(element, "Model")?.filter(|m| !m.is_empty()),
            position,
            beam,
            reference,
            breaks: Vec::new(),
        });
        Ok(self.description.geometries.len() - 1)
    }

    fn close_element(&mut self, name: &str) {
        match name {
            "LogicalChannel" => {
                if let (Some(channel), Some(logical)) =
                    (self.current_channel.as_mut(), self.current_logical.take())
                {
                    channel.logical_channels.push(logical);
                }
            }
            "DMXChannel" => {
                if let (Some(mode), Some(channel)) =
                    (self.current_mode.as_mut(), self.current_channel.take())
                {
                    mode.channels.push(channel);
                }
            }
            "DMXMode" => {
                if let Some(mode) = self.current_mode.take() {
                    self.description.modes.push(mode);
                }
            }
            _ => {}
        }
    }
}

/// Parses a GDTF matrix: four `{a,b,c,d}` rows with the translation in
/// the fourth column — and the rotation read as Blender DMX reads it,
/// which is the reading manufacturers' files have been drawn with for
/// years: the first three values of each row are a node axis, so the
/// stored 3×3 is the transpose of the rotation that takes the node's
/// frame into its parent's (design §18.6). A yoke geometry stored as
/// `{0,1,0,…}{-1,0,0,…}` is yawed +90°, not −90°; the Ayrton MagicDot SX
/// aimed in Blender DMX with mtrack's bytes is what settled it. Three-
/// value rows (the MVR spelling, basis vectors then the translation) are
/// accepted too, since exporters mix them up; they read the same way.
fn parse_matrix(text: &str) -> Option<Matrix4> {
    let mut rows: Vec<Vec<f64>> = Vec::with_capacity(4);
    let mut rest = text.trim();
    while let Some(open) = rest.find('{') {
        let close = rest[open..].find('}')? + open;
        let values = rest[open + 1..close]
            .split(',')
            .map(|v| v.trim().parse::<f64>().ok().filter(|v| v.is_finite()))
            .collect::<Option<Vec<f64>>>()?;
        rows.push(values);
        rest = &rest[close + 1..];
    }
    if rows.len() != 4 {
        return None;
    }
    if rows.iter().all(|r| r.len() == 4) {
        let mut m = IDENTITY;
        for (i, row) in rows.iter().enumerate() {
            m[i].copy_from_slice(row);
        }
        // The stored rows are the node's axes: the rotation is their
        // transpose. The translation stays in the fourth column.
        for (r, c) in [(0, 1), (0, 2), (1, 2)] {
            let (a, b) = (m[r][c], m[c][r]);
            m[r][c] = b;
            m[c][r] = a;
        }
        return Some(m);
    }
    if rows.iter().all(|r| r.len() == 3) {
        // Columns u, v, w, o: basis vectors then the translation.
        let mut m = IDENTITY;
        for (col, values) in rows.iter().enumerate() {
            for (r, v) in values.iter().enumerate() {
                m[r][col] = *v;
            }
        }
        return Some(m);
    }
    None
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

/// Parses a size: finite and positive, else zero.
fn parse_positive(text: Option<&str>) -> f64 {
    parse_finite(text).filter(|v| *v > 0.0).unwrap_or(0.0)
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
    pub(crate) const SYNTHETIC_DESCRIPTION: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<GDTF DataVersion="1.2">
  <FixtureType Name="Synth Brick" ShortName="Brick" Manufacturer="mtrack synthetic">
    <AttributeDefinitions/>
    <Wheels>
      <Wheel Name="IgnoredWheel"><Slot Name="Open"/></Wheel>
    </Wheels>
    <Models>
      <Model Name="Base" File="" PrimitiveType="Base" Length="0.30" Width="0.20" Height="0.10"/>
      <Model Name="Yoke" File="yoke" PrimitiveType="Undefined" Length="0.30" Width="0.10" Height="0.25"/>
      <Model Name="Head" File="" PrimitiveType="Cylinder" Length="0.20" Width="0.20" Height="0.15"/>
      <Model Name="Cell" File="" PrimitiveType="Cylinder" Length="0.05" Width="0.05" Height="0.01"/>
    </Models>
    <Geometries>
      <Geometry Name="Base" Model="Base" Position="{1,0,0,0}{0,1,0,0}{0,0,1,0}{0,0,0,1}">
        <Axis Name="Yoke" Model="Yoke" Position="{1,0,0,0}{0,1,0,0}{0,0,1,-0.1}{0,0,0,1}">
          <Axis Name="Head" Model="Head" Position="{1,0,0,0}{0,1,0,0}{0,0,1,-0.25}{0,0,0,1}">
            <Beam Name="Lens" Model="Head" BeamAngle="12" FieldAngle="20" BeamType="Spot" LuminousFlux="5000" ColorTemperature="6500" BeamRadius="0.05" Position="{1,0,0,0}{0,1,0,0}{0,0,1,-0.06}{0,0,0,1}"/>
          </Axis>
        </Axis>
        <GeometryReference Name="Pixel 2" Geometry="Cell" Model="Cell" Position="{1,0,0,0.05}{0,1,0,0}{0,0,1,0}{0,0,0,1}">
          <Break DMXBreak="1" DMXOffset="1"/>
        </GeometryReference>
      </Geometry>
      <Geometry Name="Cell" Model="Cell">
        <Beam Name="Cell Lens" Model="Cell" BeamAngle="30" BeamType="Wash"/>
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
          <DMXChannel Offset="1,2" Geometry="Yoke">
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
    fn custom_entities_are_never_expanded() {
        // The module doc claims no DTD/entity expansion; back it up. A
        // custom entity in an attribute must surface as a parse error, not
        // an expansion.
        let xml = r#"<?xml version="1.0"?>
<!DOCTYPE GDTF [<!ENTITY boom "expanded">]>
<GDTF><FixtureType Name="&boom;" Manufacturer="m"/></GDTF>"#;
        match parse_description(xml) {
            Err(_) => {}
            Ok(description) => {
                assert_ne!(description.name, "expanded", "entity was expanded");
            }
        }
    }

    #[test]
    fn a_fixture_less_document_is_an_error() {
        let err = parse_description("<NotGdtf/>").unwrap_err().to_string();
        assert!(err.contains("no FixtureType"), "{err}");
    }

    #[test]
    fn the_geometry_tree_models_and_beams_are_parsed() {
        let description = parse_description(SYNTHETIC_DESCRIPTION).unwrap();
        assert!(description.thumbnail.is_none());
        let names: Vec<&str> = description.models.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(names, ["Base", "Yoke", "Head", "Cell"]);
        assert_eq!(description.models[1].file.as_deref(), Some("yoke"));
        assert_eq!(description.models[0].file, None, "an empty File is no file");
        assert_eq!(description.models[0].primitive, "Base");
        assert_eq!(description.models[2].size, [0.20, 0.20, 0.15]);
        let negative = parse_description(&SYNTHETIC_DESCRIPTION.replace(
            "Length=\"0.20\" Width=\"0.20\"",
            "Length=\"-0.20\" Width=\"nan\"",
        ))
        .unwrap();
        assert_eq!(negative.models[2].size, [0.0, 0.0, 0.15]);

        let by_name = |name: &str| {
            description
                .geometries
                .iter()
                .position(|g| g.name == name)
                .unwrap_or_else(|| panic!("no geometry {name}"))
        };
        let (base, yoke, head, lens, pixel, cell, cell_lens) = (
            by_name("Base"),
            by_name("Yoke"),
            by_name("Head"),
            by_name("Lens"),
            by_name("Pixel 2"),
            by_name("Cell"),
            by_name("Cell Lens"),
        );
        let g = &description.geometries;
        assert_eq!(g[base].parent, None);
        assert_eq!(g[yoke].parent, Some(base));
        assert_eq!(g[head].parent, Some(yoke));
        assert_eq!(g[lens].parent, Some(head));
        assert_eq!(g[pixel].parent, Some(base), "Break is not a node");
        assert_eq!(g[cell].parent, None);
        assert_eq!(g[cell_lens].parent, Some(cell));
        assert_eq!(g[yoke].kind, GeometryKind::Axis);
        assert_eq!(g[lens].kind, GeometryKind::Beam);
        assert_eq!(g[pixel].kind, GeometryKind::Reference);
        assert_eq!(g[pixel].reference.as_deref(), Some("Cell"));
        assert_eq!(g[pixel].breaks, vec![("1".to_string(), 1)]);
        assert_eq!(description.modes[0].channels[1].dmx_break, "1");
        assert_eq!(g[yoke].model.as_deref(), Some("Yoke"));
        assert_eq!(
            g[yoke].position[2][3], -0.1,
            "translation sits in the fourth column"
        );
        assert_eq!(g[pixel].position[0][3], 0.05);
        assert_eq!(g[cell].position, IDENTITY, "no Position is the identity");
        let beam = g[lens].beam.as_ref().unwrap();
        assert_eq!(beam.beam_angle, Some(12.0));
        assert_eq!(beam.field_angle, Some(20.0));
        assert_eq!(beam.beam_type.as_deref(), Some("Spot"));
        assert_eq!(beam.luminous_flux, Some(5000.0));
        assert_eq!(beam.color_temperature, Some(6500.0));
        assert_eq!(beam.beam_radius, Some(0.05));
        assert!(g[head].beam.is_none());
        assert_eq!(description.geometry_reference_names, vec!["Pixel 2"]);
    }

    #[test]
    fn matrices_parse_in_both_spellings() {
        let gdtf = parse_matrix("{0.5,0.866,0,-0.047}{-0.866,0.5,0,0.027}{0,0,1,-0.001}{0,0,0,1}")
            .unwrap();
        // The stored rows are the node's axes, so the rotation is their
        // transpose; the translation is the fourth column as written.
        assert_eq!(gdtf[0][1], -0.866);
        assert_eq!(gdtf[1][0], 0.866);
        assert_eq!(gdtf[0][3], -0.047);
        assert_eq!(gdtf[1][3], 0.027);
        // The MVR column spelling lands the same numbers in the same cells.
        // MVR's spelling writes the same axes as rows with the translation
        // last: the same numbers read the same way in both.
        let mvr = parse_matrix("{0.5,0.866,0}{-0.866,0.5,0}{0,0,1}{-0.047,0.027,-0.001}").unwrap();
        assert_eq!(mvr, gdtf);
        assert!(parse_matrix("{1,0,0}{0,1,0}").is_none());
        assert!(parse_matrix("{1,0,0,x}{0,1,0,0}{0,0,1,0}{0,0,0,1}").is_none());
        assert!(parse_matrix("{inf,0,0,0}{0,1,0,0}{0,0,1,0}{0,0,0,1}").is_none());
    }

    #[test]
    fn a_bad_position_is_an_error_naming_the_geometry() {
        let xml = SYNTHETIC_DESCRIPTION.replace(
            "Name=\"Yoke\" Model=\"Yoke\" Position=\"{1,0,0,0}{0,1,0,0}{0,0,1,-0.1}{0,0,0,1}\"",
            "Name=\"Yoke\" Model=\"Yoke\" Position=\"{1,0,0}\"",
        );
        let err = parse_description(&xml).unwrap_err().to_string();
        assert!(err.contains("\"Yoke\""), "{err}");
    }
}
