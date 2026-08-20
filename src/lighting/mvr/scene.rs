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

//! Streaming parser for the subset of `GeneralSceneDescription.xml` mtrack
//! consumes: the patched fixtures — name, layer, GDTF reference and mode,
//! addresses, transform. Scene graph nodes that aren't fixtures (scenery,
//! trusses, groups) are passed over, though fixtures *inside* groups are
//! still collected.
//!
//! Unlike GDTF, MVR carries its per-fixture data as element text
//! (`<GDTFSpec>file</GDTFSpec>`), so the walk tracks which leaf it is
//! inside. Malformed per-fixture values (an unparseable matrix or address)
//! degrade to "absent" with a warning on the scene rather than failing the
//! whole file — one console's quirk must not hide the rest of the patch.

use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;

use super::MvrError;

/// The deepest element nesting accepted.
const MAX_DEPTH: usize = 64;

/// The parsed subset of an MVR scene.
#[derive(Debug)]
pub struct Scene {
    /// The patched fixtures, in document order.
    pub fixtures: Vec<MvrFixture>,
    /// Per-value parse degradations — what was dropped and why.
    pub warnings: Vec<String>,
}

/// A 4x3 MVR transform: three basis vectors and a translation, expressed in
/// the scene's coordinate system (right-handed Z-up, millimeters).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix {
    pub u: [f64; 3],
    pub v: [f64; 3],
    pub w: [f64; 3],
    /// The translation — the fixture's position in millimeters.
    pub o: [f64; 3],
}

/// A patched fixture as MVR states it.
#[derive(Debug, Default)]
pub struct MvrFixture {
    /// The fixture's name.
    pub name: String,
    /// The layer it sits on, when one encloses it.
    pub layer: String,
    /// The GDTF archive reference (`GDTFSpec`), as written — the embedded
    /// entry it names may or may not carry a `.gdtf` suffix.
    pub gdtf_spec: Option<String>,
    /// The DMX mode reference (`GDTFMode`).
    pub gdtf_mode: Option<String>,
    /// Patch addresses as (universe, address), in document order; the first
    /// is the fixture's patch, later ones are additional DMX breaks.
    pub addresses: Vec<(u16, u16)>,
    /// The fixture's transform, when present and parseable.
    pub matrix: Option<Matrix>,
}

/// Which text-bearing leaf inside a Fixture the walk is in.
#[derive(Clone, Copy, PartialEq)]
enum TextTarget {
    GdtfSpec,
    GdtfMode,
    Address,
    Matrix,
}

/// Parses `GeneralSceneDescription.xml` content into the consumed subset.
pub fn parse_scene(xml: &str) -> Result<Scene, MvrError> {
    let mut reader = Reader::from_str(xml);

    let mut scene = Scene {
        fixtures: Vec::new(),
        warnings: Vec::new(),
    };

    let mut stack: Vec<String> = Vec::new();
    let mut layer_stack: Vec<String> = Vec::new();
    let mut current: Option<MvrFixture> = None;
    // Fixtures can in principle nest under group objects that are themselves
    // inside a Fixture's subtree; count depth so only the outermost Fixture
    // element opens/closes the current fixture.
    let mut fixture_depth = 0usize;
    let mut text_target: Option<TextTarget> = None;
    let mut text_buffer = String::new();
    let mut saw_root = false;

    loop {
        let event = reader
            .read_event()
            .map_err(|e| MvrError::new(format!("XML error in scene description: {e}")))?;
        match event {
            Event::Start(ref element) => {
                let name = element_name(element)?;
                open_element(
                    element,
                    &name,
                    &mut scene,
                    &mut layer_stack,
                    &mut current,
                    &mut fixture_depth,
                    &mut text_target,
                    &mut text_buffer,
                    &mut saw_root,
                )?;
                stack.push(name);
                if stack.len() > MAX_DEPTH {
                    return Err(MvrError::new(format!(
                        "scene description nests deeper than {MAX_DEPTH} elements"
                    )));
                }
            }
            Event::Empty(ref element) => {
                let name = element_name(element)?;
                open_element(
                    element,
                    &name,
                    &mut scene,
                    &mut layer_stack,
                    &mut current,
                    &mut fixture_depth,
                    &mut text_target,
                    &mut text_buffer,
                    &mut saw_root,
                )?;
                close_element(
                    &name,
                    &mut scene,
                    &mut layer_stack,
                    &mut current,
                    &mut fixture_depth,
                    &mut text_target,
                    &text_buffer,
                );
            }
            Event::Text(ref text) => {
                if text_target.is_some() {
                    let value = text
                        .xml_content(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|e| MvrError::new(format!("malformed XML text: {e}")))?;
                    text_buffer.push_str(&value);
                }
            }
            Event::End(_) => {
                if let Some(name) = stack.pop() {
                    close_element(
                        &name,
                        &mut scene,
                        &mut layer_stack,
                        &mut current,
                        &mut fixture_depth,
                        &mut text_target,
                        &text_buffer,
                    );
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if !saw_root {
        return Err(MvrError::new(
            "scene description has no GeneralSceneDescription element",
        ));
    }
    Ok(scene)
}

fn element_name(element: &BytesStart<'_>) -> Result<String, MvrError> {
    std::str::from_utf8(element.name().as_ref())
        .map(|s| s.to_string())
        .map_err(|_| MvrError::new("non-UTF-8 element name in scene description"))
}

fn attr(element: &BytesStart<'_>, name: &str) -> Result<Option<String>, MvrError> {
    for attribute in element.attributes() {
        let attribute =
            attribute.map_err(|e| MvrError::new(format!("malformed XML attribute: {e}")))?;
        if attribute.key.as_ref() == name.as_bytes() {
            let value = attribute
                .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .map_err(|e| MvrError::new(format!("malformed XML attribute value: {e}")))?;
            return Ok(Some(value.into_owned()));
        }
    }
    Ok(None)
}

// ptr_arg: the buffer is cleared here (a String operation), not just read.
#[allow(clippy::too_many_arguments, clippy::ptr_arg)]
fn open_element(
    element: &BytesStart<'_>,
    name: &str,
    scene: &mut Scene,
    layer_stack: &mut Vec<String>,
    current: &mut Option<MvrFixture>,
    fixture_depth: &mut usize,
    text_target: &mut Option<TextTarget>,
    text_buffer: &mut String,
    saw_root: &mut bool,
) -> Result<(), MvrError> {
    match name {
        "GeneralSceneDescription" => *saw_root = true,
        "Layer" => {
            layer_stack.push(attr(element, "name")?.unwrap_or_default());
        }
        "Fixture" => {
            if current.is_some() {
                *fixture_depth += 1;
                scene
                    .warnings
                    .push("a Fixture nested inside another Fixture was ignored".to_string());
            } else {
                *current = Some(MvrFixture {
                    name: attr(element, "name")?.unwrap_or_default(),
                    layer: layer_stack.last().cloned().unwrap_or_default(),
                    ..MvrFixture::default()
                });
                *fixture_depth = 0;
            }
        }
        "GDTFSpec" | "GDTFMode" | "Address" | "Matrix" if current.is_some() => {
            *text_target = Some(match name {
                "GDTFSpec" => TextTarget::GdtfSpec,
                "GDTFMode" => TextTarget::GdtfMode,
                "Address" => TextTarget::Address,
                _ => TextTarget::Matrix,
            });
            text_buffer.clear();
        }
        _ => {}
    }
    Ok(())
}

fn close_element(
    name: &str,
    scene: &mut Scene,
    layer_stack: &mut Vec<String>,
    current: &mut Option<MvrFixture>,
    fixture_depth: &mut usize,
    text_target: &mut Option<TextTarget>,
    text_buffer: &str,
) {
    match name {
        "Layer" => {
            layer_stack.pop();
        }
        "Fixture" => {
            if *fixture_depth > 0 {
                *fixture_depth -= 1;
            } else if let Some(fixture) = current.take() {
                scene.fixtures.push(fixture);
            }
        }
        "GDTFSpec" | "GDTFMode" | "Address" | "Matrix" => {
            let Some(target) = text_target.take() else {
                return;
            };
            let Some(fixture) = current.as_mut() else {
                return;
            };
            let text = text_buffer.trim().to_string();
            match target {
                TextTarget::GdtfSpec if !text.is_empty() => fixture.gdtf_spec = Some(text),
                TextTarget::GdtfMode if !text.is_empty() => fixture.gdtf_mode = Some(text),
                TextTarget::Address => match parse_address(&text) {
                    Some(address) => fixture.addresses.push(address),
                    None => scene.warnings.push(format!(
                        "fixture \"{}\": unparseable address \"{text}\"; dropped",
                        fixture.name
                    )),
                },
                TextTarget::Matrix => match parse_matrix(&text) {
                    Some(matrix) => fixture.matrix = Some(matrix),
                    None => scene.warnings.push(format!(
                        "fixture \"{}\": unparseable matrix \"{text}\"; position dropped",
                        fixture.name
                    )),
                },
                _ => {}
            }
        }
        _ => {}
    }
}

/// Parses an MVR address: either dotted `universe.address` or an absolute
/// value where `absolute = (universe - 1) * 512 + address`.
fn parse_address(text: &str) -> Option<(u16, u16)> {
    let text = text.trim();
    if let Some((universe, address)) = text.split_once('.') {
        let universe: u16 = universe.trim().parse().ok()?;
        let address: u16 = address.trim().parse().ok()?;
        if universe == 0 || !(1..=512).contains(&address) {
            return None;
        }
        return Some((universe, address));
    }
    let absolute: u32 = text.parse().ok()?;
    if absolute == 0 {
        return None;
    }
    let universe = ((absolute - 1) / 512 + 1).try_into().ok()?;
    let address = ((absolute - 1) % 512 + 1) as u16;
    Some((universe, address))
}

/// Parses an MVR 4x3 matrix: `{u}{v}{w}{o}`, each group three finite,
/// comma-separated numbers.
fn parse_matrix(text: &str) -> Option<Matrix> {
    let mut groups: Vec<[f64; 3]> = Vec::with_capacity(4);
    let mut rest = text.trim();
    while let Some(open) = rest.find('{') {
        let close = rest[open..].find('}')? + open;
        let mut values = rest[open + 1..close]
            .split(',')
            .map(|v| v.trim().parse::<f64>());
        let mut group = [0.0f64; 3];
        for slot in &mut group {
            let value = values.next()?.ok()?;
            if !value.is_finite() {
                return None;
            }
            *slot = value;
        }
        if values.next().is_some() {
            return None;
        }
        groups.push(group);
        rest = &rest[close + 1..];
    }
    if groups.len() != 4 || !rest.trim().is_empty() {
        return None;
    }
    Some(Matrix {
        u: groups[0],
        v: groups[1],
        w: groups[2],
        o: groups[3],
    })
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// A small venue: two patched fixtures on different universes (one
    /// dotted, one absolute address), one with a transform, plus scenery
    /// and a truss the parser must pass over.
    pub(crate) const SYNTHETIC_SCENE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<GeneralSceneDescription verMajor="1" verMinor="6">
  <UserData/>
  <Scene>
    <Layers>
      <Layer name="Front Truss">
        <ChildList>
          <Fixture name="Brick 1" uuid="aaaa">
            <Matrix>{1,0,0}{0,1,0}{0,0,1}{-2000,3500,4200}</Matrix>
            <GDTFSpec>Astera_PB15.gdtf</GDTFSpec>
            <GDTFMode>8: RGBS</GDTFMode>
            <Addresses>
              <Address break="0">1.1</Address>
            </Addresses>
          </Fixture>
          <SceneObject name="Truss A">
            <Matrix>{1,0,0}{0,1,0}{0,0,1}{0,0,6000}</Matrix>
          </SceneObject>
        </ChildList>
      </Layer>
      <Layer name="Back Wall">
        <ChildList>
          <GroupObject name="Backline">
            <ChildList>
              <Fixture name="Mover 1" uuid="bbbb">
                <GDTFSpec>Mover</GDTFSpec>
                <GDTFMode>Standard</GDTFMode>
                <Addresses>
                  <Address break="0">513</Address>
                  <Address break="1">2.100</Address>
                </Addresses>
              </Fixture>
            </ChildList>
          </GroupObject>
        </ChildList>
      </Layer>
    </Layers>
  </Scene>
</GeneralSceneDescription>
"#;

    #[test]
    fn parses_fixtures_layers_addresses_and_transforms() {
        let scene = parse_scene(SYNTHETIC_SCENE).unwrap();
        assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);
        assert_eq!(
            scene.fixtures.len(),
            2,
            "scenery and groups are not fixtures"
        );

        let brick = &scene.fixtures[0];
        assert_eq!(brick.name, "Brick 1");
        assert_eq!(brick.layer, "Front Truss");
        assert_eq!(brick.gdtf_spec.as_deref(), Some("Astera_PB15.gdtf"));
        assert_eq!(brick.gdtf_mode.as_deref(), Some("8: RGBS"));
        assert_eq!(brick.addresses, vec![(1, 1)]);
        let matrix = brick.matrix.unwrap();
        assert_eq!(matrix.o, [-2000.0, 3500.0, 4200.0]);

        let mover = &scene.fixtures[1];
        assert_eq!(mover.layer, "Back Wall", "layer tracks through groups");
        // Absolute 513 is universe 2, address 1.
        assert_eq!(mover.addresses, vec![(2, 1), (2, 100)]);
        assert!(mover.matrix.is_none());
    }

    #[test]
    fn address_forms() {
        assert_eq!(parse_address("1.1"), Some((1, 1)));
        assert_eq!(parse_address("2.512"), Some((2, 512)));
        assert_eq!(parse_address("1"), Some((1, 1)));
        assert_eq!(parse_address("512"), Some((1, 512)));
        assert_eq!(parse_address("513"), Some((2, 1)));
        assert_eq!(parse_address("0"), None);
        assert_eq!(parse_address("1.0"), None);
        assert_eq!(parse_address("1.513"), None);
        assert_eq!(parse_address("junk"), None);
    }

    #[test]
    fn malformed_values_degrade_with_warnings() {
        let xml = r#"<GeneralSceneDescription><Scene><Layers><Layer name="L"><ChildList>
<Fixture name="F">
  <Matrix>{1,0,0}{0,1,0}</Matrix>
  <Addresses><Address break="0">not-an-address</Address></Addresses>
</Fixture>
</ChildList></Layer></Layers></Scene></GeneralSceneDescription>"#;
        let scene = parse_scene(xml).unwrap();
        assert_eq!(scene.fixtures.len(), 1);
        assert!(scene.fixtures[0].matrix.is_none());
        assert!(scene.fixtures[0].addresses.is_empty());
        assert_eq!(scene.warnings.len(), 2, "{:?}", scene.warnings);
        assert!(scene.warnings[0].contains("unparseable matrix"));
        assert!(scene.warnings[1].contains("unparseable address"));
    }

    #[test]
    fn matrix_forms() {
        assert!(parse_matrix("{1,0,0}{0,1,0}{0,0,1}{0,0,0}").is_some());
        assert!(parse_matrix(" {1,0,0} {0,1,0} {0,0,1} {5.5,-2,0} ").is_some());
        assert!(parse_matrix("{1,0,0}{0,1,0}{0,0,1}").is_none());
        assert!(parse_matrix("{1,0}{0,1,0}{0,0,1}{0,0,0}").is_none());
        assert!(parse_matrix("{1,0,0,0}{0,1,0}{0,0,1}{0,0,0}").is_none());
        assert!(parse_matrix("{1,0,0}{0,1,0}{0,0,1}{0,0,inf}").is_none());
        assert!(parse_matrix("").is_none());
    }

    #[test]
    fn a_scene_less_document_is_an_error() {
        let err = parse_scene("<NotMvr/>").unwrap_err().to_string();
        assert!(err.contains("no GeneralSceneDescription"), "{err}");
    }

    #[test]
    fn depth_bomb_is_rejected() {
        let mut xml = String::from("<GeneralSceneDescription>");
        for _ in 0..100 {
            xml.push_str("<a>");
        }
        for _ in 0..100 {
            xml.push_str("</a>");
        }
        xml.push_str("</GeneralSceneDescription>");
        let err = parse_scene(&xml).unwrap_err().to_string();
        assert!(err.contains("nests deeper"), "{err}");
    }
}
