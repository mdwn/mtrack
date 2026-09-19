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
//! addresses, transform — the focus points, and the scenery (scene
//! objects, trusses, supports, screens: a transform and the mesh files
//! they draw, through the symbol definitions they may reference). Group
//! nodes are passed over, though what is *inside* them is still
//! collected.
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
#[derive(Debug, Default)]
pub struct Scene {
    /// The patched fixtures, in document order.
    pub fixtures: Vec<MvrFixture>,
    /// The scene's focus points, in document order.
    pub focus_points: Vec<MvrFocusPoint>,
    /// The scenery — everything with a transform and meshes that is not a
    /// fixture — in document order.
    pub objects: Vec<MvrSceneObject>,
    /// Per-value parse degradations — what was dropped and why.
    pub warnings: Vec<String>,
}

/// A piece of scenery as MVR states it: a scene object, truss, support,
/// video screen or projector, with the meshes it draws.
#[derive(Debug, Default, Clone)]
pub struct MvrSceneObject {
    /// The object's name (often empty).
    pub name: String,
    /// The element kind: `SceneObject`, `Truss`, `Support`, ...
    pub kind: String,
    /// The layer it sits on, when one encloses it.
    pub layer: String,
    /// The object's transform, when present and parseable.
    pub matrix: Option<Matrix>,
    /// The meshes, own and through symbol definitions, each with its
    /// transform relative to the object.
    pub meshes: Vec<MvrMesh>,
}

/// A mesh file an object draws.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct MvrMesh {
    /// The archive entry name (`Geometry3D fileName`).
    pub file: String,
    /// The mesh's transform relative to its object, when it has one.
    pub matrix: Option<Matrix>,
}

/// A focus point as MVR states it: a named transform fixtures can aim at.
#[derive(Debug, Default)]
pub struct MvrFocusPoint {
    /// The focus point's name.
    pub name: String,
    /// Its transform, when present and parseable; the translation is the
    /// point.
    pub matrix: Option<Matrix>,
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

impl Matrix {
    /// The rotation the basis vectors encode, as degrees about the X, Y and
    /// Z axes applied in that order (the venue DSL's `rotation`), with
    /// whether the basis was orthonormal. A scaled basis (a mesh scale
    /// riding along) is normalized; a sheared one is reported so the
    /// import can say the rotation is approximate.
    pub fn rotation_degrees(&self) -> (Vec3, bool) {
        let mut u = self.u;
        let mut v = self.v;
        let mut w = self.w;
        let mut exact = true;
        for axis in [&mut u, &mut v, &mut w] {
            let length = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
            if length > 1e-9 {
                for c in axis.iter_mut() {
                    *c /= length;
                }
            } else {
                exact = false;
            }
        }
        let dot = |a: &Vec3, b: &Vec3| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
        if dot(&u, &v).abs() > 1e-3 || dot(&v, &w).abs() > 1e-3 || dot(&u, &w).abs() > 1e-3 {
            exact = false;
        }
        // R = Rz·Ry·Rx with columns u, v, w (the local axes in scene space).
        let r11 = u[0];
        let r21 = u[1];
        let r31 = u[2];
        let r32 = v[2];
        let r33 = w[2];
        let ry = (-r31).clamp(-1.0, 1.0).asin();
        let (rx, rz) = if ry.cos().abs() > 1e-6 {
            (r32.atan2(r33), r21.atan2(r11))
        } else {
            // Gimbal lock: fold the whole rotation into X.
            ((-v[0]).atan2(v[1]), 0.0)
        };
        ([rx.to_degrees(), ry.to_degrees(), rz.to_degrees()], exact)
    }

    /// `self` after `inner`: the transform of something placed by `inner`
    /// inside a frame placed by `self` (a symbol's mesh inside its object).
    pub fn compose(&self, inner: &Matrix) -> Matrix {
        let apply = |v: &Vec3| -> Vec3 {
            [
                self.u[0] * v[0] + self.v[0] * v[1] + self.w[0] * v[2],
                self.u[1] * v[0] + self.v[1] * v[1] + self.w[1] * v[2],
                self.u[2] * v[0] + self.v[2] * v[1] + self.w[2] * v[2],
            ]
        };
        let o = apply(&inner.o);
        Matrix {
            u: apply(&inner.u),
            v: apply(&inner.v),
            w: apply(&inner.w),
            o: [o[0] + self.o[0], o[1] + self.o[1], o[2] + self.o[2]],
        }
    }
}

/// The identity transform.
pub const IDENTITY: Matrix = Matrix {
    u: [1.0, 0.0, 0.0],
    v: [0.0, 1.0, 0.0],
    w: [0.0, 0.0, 1.0],
    o: [0.0, 0.0, 0.0],
};

/// A scene-space triple.
pub type Vec3 = [f64; 3];

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
    /// The console's fixture ID (`FixtureID`), the number an operator
    /// knows the fixture by; consoles name fixtures by type, so this is
    /// what tells them apart.
    pub fixture_id: Option<String>,
    /// The fixture's transform, when present and parseable.
    pub matrix: Option<Matrix>,
}

/// Which text-bearing leaf the walk is in.
#[derive(Clone, Copy, PartialEq)]
enum TextTarget {
    GdtfSpec,
    GdtfMode,
    Address,
    FixtureId,
    /// A Fixture's transform.
    Matrix,
    /// A FocusPoint's transform.
    FocusMatrix,
    /// A scenery object's transform.
    ObjectMatrix,
    /// A Geometry3D's own transform.
    MeshMatrix,
    /// A Symbol instance's transform.
    SymbolMatrix,
}

/// The element kinds that are scenery.
const SCENERY_ELEMENTS: &[&str] = &[
    "SceneObject",
    "Truss",
    "Support",
    "VideoScreen",
    "Projector",
];

/// The walk's mutable state.
#[derive(Default)]
struct Walk {
    scene: Scene,
    layer_stack: Vec<String>,
    current: Option<MvrFixture>,
    current_focus: Option<MvrFocusPoint>,
    /// Fixtures can in principle nest under group objects that are
    /// themselves inside a Fixture's subtree; count depth so only the
    /// outermost Fixture element opens/closes the current fixture.
    fixture_depth: usize,
    text_target: Option<TextTarget>,
    text_buffer: String,
    saw_root: bool,
    /// The scenery object being read, and how deep inside it the walk is.
    current_object: Option<MvrSceneObject>,
    object_depth: usize,
    /// A Geometry3D being read (its Matrix child may follow).
    current_mesh: Option<MvrMesh>,
    /// A Symbol instance being read: the symdef it names and its Matrix.
    current_symbol: Option<(String, Option<Matrix>)>,
    /// Symbol definitions by uuid: the meshes they hold.
    symdefs: std::collections::HashMap<String, Vec<MvrMesh>>,
    /// The symdef being read.
    current_symdef: Option<(String, Vec<MvrMesh>)>,
    /// Symbol references to resolve once the document is read:
    /// (object index, symdef uuid, symbol transform).
    symbol_refs: Vec<(usize, String, Option<Matrix>)>,
}

/// Parses `GeneralSceneDescription.xml` content into the consumed subset.
pub fn parse_scene(xml: &str) -> Result<Scene, MvrError> {
    let mut reader = Reader::from_str(xml);
    let mut walk = Walk::default();
    let mut stack: Vec<String> = Vec::new();

    loop {
        let event = reader
            .read_event()
            .map_err(|e| MvrError::new(format!("XML error in scene description: {e}")))?;
        match event {
            Event::Start(ref element) => {
                let name = element_name(element)?;
                walk.open(element, &name)?;
                stack.push(name);
                if stack.len() > MAX_DEPTH {
                    return Err(MvrError::new(format!(
                        "scene description nests deeper than {MAX_DEPTH} elements"
                    )));
                }
            }
            Event::Empty(ref element) => {
                let name = element_name(element)?;
                walk.open(element, &name)?;
                walk.close(&name);
            }
            Event::Text(ref text) => {
                if walk.text_target.is_some() {
                    let value = text
                        .xml_content(quick_xml::XmlVersion::Implicit1_0)
                        .map_err(|e| MvrError::new(format!("malformed XML text: {e}")))?;
                    walk.text_buffer.push_str(&value);
                }
            }
            Event::End(_) => {
                if let Some(name) = stack.pop() {
                    walk.close(&name);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }

    if !walk.saw_root {
        return Err(MvrError::new(
            "scene description has no GeneralSceneDescription element",
        ));
    }
    // Symbols resolve last: a symdef may be defined anywhere in the file.
    let mut unresolved = std::collections::BTreeSet::new();
    for (index, symdef, matrix) in walk.symbol_refs {
        match walk.symdefs.get(&symdef) {
            Some(meshes) => {
                for mesh in meshes {
                    let matrix = match (&matrix, &mesh.matrix) {
                        (Some(symbol), Some(own)) => Some(symbol.compose(own)),
                        (Some(symbol), None) => Some(*symbol),
                        (None, own) => *own,
                    };
                    walk.scene.objects[index].meshes.push(MvrMesh {
                        file: mesh.file.clone(),
                        matrix,
                    });
                }
            }
            None => {
                unresolved.insert(symdef);
            }
        }
    }
    for symdef in unresolved {
        walk.scene.warnings.push(format!(
            "symbol definition {symdef} is referenced but never defined"
        ));
    }
    Ok(walk.scene)
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

impl Walk {
    fn open(&mut self, element: &BytesStart<'_>, name: &str) -> Result<(), MvrError> {
        match name {
            "GeneralSceneDescription" => self.saw_root = true,
            "Layer" => {
                self.layer_stack
                    .push(attr(element, "name")?.unwrap_or_default());
            }
            "Fixture" => {
                if self.current.is_some() {
                    self.fixture_depth += 1;
                    self.scene
                        .warnings
                        .push("a Fixture nested inside another Fixture was ignored".to_string());
                } else {
                    self.current = Some(MvrFixture {
                        name: attr(element, "name")?.unwrap_or_default(),
                        layer: self.layer_stack.last().cloned().unwrap_or_default(),
                        ..MvrFixture::default()
                    });
                    self.fixture_depth = 0;
                }
            }
            "FocusPoint" if self.current.is_none() && self.current_focus.is_none() => {
                self.current_focus = Some(MvrFocusPoint {
                    name: attr(element, "name")?.unwrap_or_default(),
                    matrix: None,
                });
            }
            "Symdef" if self.current_symdef.is_none() => {
                self.current_symdef =
                    Some((attr(element, "uuid")?.unwrap_or_default(), Vec::new()));
            }
            kind if SCENERY_ELEMENTS.contains(&kind) && self.current.is_none() => {
                if self.current_object.is_some() {
                    self.object_depth += 1;
                } else {
                    self.current_object = Some(MvrSceneObject {
                        name: attr(element, "name")?.unwrap_or_default(),
                        kind: kind.to_string(),
                        layer: self.layer_stack.last().cloned().unwrap_or_default(),
                        matrix: None,
                        meshes: Vec::new(),
                    });
                    self.object_depth = 0;
                }
            }
            "Geometry3D" if self.current_object.is_some() || self.current_symdef.is_some() => {
                if let Some(file) = attr(element, "fileName")?.filter(|f| !f.trim().is_empty()) {
                    self.current_mesh = Some(MvrMesh {
                        file: file.trim().to_string(),
                        matrix: None,
                    });
                }
            }
            "Symbol" if self.current_object.is_some() => {
                if let Some(symdef) = attr(element, "symdef")?.filter(|s| !s.trim().is_empty()) {
                    self.current_symbol = Some((symdef.trim().to_string(), None));
                }
            }
            "Matrix" if self.current_mesh.is_some() => {
                self.text_target = Some(TextTarget::MeshMatrix);
                self.text_buffer.clear();
            }
            "Matrix" if self.current_symbol.is_some() => {
                self.text_target = Some(TextTarget::SymbolMatrix);
                self.text_buffer.clear();
            }
            "GDTFSpec" | "GDTFMode" | "Address" | "FixtureID" | "Matrix"
                if self.current.is_some() =>
            {
                self.text_target = Some(match name {
                    "GDTFSpec" => TextTarget::GdtfSpec,
                    "GDTFMode" => TextTarget::GdtfMode,
                    "Address" => TextTarget::Address,
                    "FixtureID" => TextTarget::FixtureId,
                    _ => TextTarget::Matrix,
                });
                self.text_buffer.clear();
            }
            "Matrix" if self.current_focus.is_some() => {
                self.text_target = Some(TextTarget::FocusMatrix);
                self.text_buffer.clear();
            }
            "Matrix" if self.current_object.is_some() && self.object_depth == 0 => {
                self.text_target = Some(TextTarget::ObjectMatrix);
                self.text_buffer.clear();
            }
            _ => {}
        }
        Ok(())
    }

    fn close(&mut self, name: &str) {
        match name {
            "Layer" => {
                self.layer_stack.pop();
            }
            "Fixture" => {
                if self.fixture_depth > 0 {
                    self.fixture_depth -= 1;
                } else if let Some(fixture) = self.current.take() {
                    self.scene.fixtures.push(fixture);
                }
            }
            "FocusPoint" => {
                if let Some(focus) = self.current_focus.take() {
                    self.scene.focus_points.push(focus);
                }
            }
            "Symdef" => {
                if let Some((uuid, meshes)) = self.current_symdef.take() {
                    self.symdefs.insert(uuid, meshes);
                }
            }
            kind if SCENERY_ELEMENTS.contains(&kind) => {
                if self.object_depth > 0 {
                    self.object_depth -= 1;
                } else if let Some(object) = self.current_object.take() {
                    self.scene.objects.push(object);
                }
            }
            "Geometry3D" => {
                if let Some(mesh) = self.current_mesh.take() {
                    if let Some((_, meshes)) = self.current_symdef.as_mut() {
                        meshes.push(mesh);
                    } else if let Some(object) = self.current_object.as_mut() {
                        object.meshes.push(mesh);
                    }
                }
            }
            "Symbol" => {
                if let Some((symdef, matrix)) = self.current_symbol.take() {
                    if self.current_object.is_some() {
                        // Resolved after the walk; the object's index is
                        // what it will have once pushed.
                        self.symbol_refs
                            .push((self.scene.objects.len(), symdef, matrix));
                    }
                }
            }
            "GDTFSpec" | "GDTFMode" | "Address" | "FixtureID" | "Matrix" => {
                let Some(target) = self.text_target.take() else {
                    return;
                };
                let text = self.text_buffer.trim().to_string();
                if matches!(
                    target,
                    TextTarget::ObjectMatrix | TextTarget::MeshMatrix | TextTarget::SymbolMatrix
                ) {
                    let parsed = parse_matrix(&text);
                    if parsed.is_none() {
                        let name = self
                            .current_object
                            .as_ref()
                            .map(|o| o.name.clone())
                            .unwrap_or_default();
                        self.scene.warnings.push(format!(
                            "scenery \"{name}\": unparseable matrix \"{text}\"; dropped"
                        ));
                    }
                    match target {
                        TextTarget::MeshMatrix => {
                            if let Some(mesh) = self.current_mesh.as_mut() {
                                mesh.matrix = parsed;
                            }
                        }
                        TextTarget::SymbolMatrix => {
                            if let Some(symbol) = self.current_symbol.as_mut() {
                                symbol.1 = parsed;
                            }
                        }
                        _ => {
                            if let Some(object) = self.current_object.as_mut() {
                                object.matrix = parsed;
                            }
                        }
                    }
                    return;
                }
                if target == TextTarget::FocusMatrix {
                    let Some(focus) = self.current_focus.as_mut() else {
                        return;
                    };
                    match parse_matrix(&text) {
                        Some(matrix) => focus.matrix = Some(matrix),
                        None => self.scene.warnings.push(format!(
                            "focus point \"{}\": unparseable matrix \"{text}\"; dropped",
                            focus.name
                        )),
                    }
                    return;
                }
                let Some(fixture) = self.current.as_mut() else {
                    return;
                };
                match target {
                    TextTarget::GdtfSpec if !text.is_empty() => fixture.gdtf_spec = Some(text),
                    TextTarget::GdtfMode if !text.is_empty() => fixture.gdtf_mode = Some(text),
                    TextTarget::FixtureId if !text.is_empty() => fixture.fixture_id = Some(text),
                    TextTarget::Address => match parse_address(&text) {
                        Some(address) => fixture.addresses.push(address),
                        None => self.scene.warnings.push(format!(
                            "fixture \"{}\": unparseable address \"{text}\"; dropped",
                            fixture.name
                        )),
                    },
                    TextTarget::Matrix => match parse_matrix(&text) {
                        Some(matrix) => fixture.matrix = Some(matrix),
                        None => self.scene.warnings.push(format!(
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
    <AUXData>
      <Symdef uuid="sym-box" name="Box">
        <ChildList>
          <Geometry3D fileName="Box.3ds"/>
          <Geometry3D fileName="Lid.glb">
            <Matrix>{1,0,0}{0,1,0}{0,0,1}{0,0,500}</Matrix>
          </Geometry3D>
        </ChildList>
      </Symdef>
    </AUXData>
    <Layers>
      <Layer name="Front Truss">
        <ChildList>
          <Truss name="Truss A" uuid="tttt">
            <Matrix>{0,1,0}{-1,0,0}{0,0,1}{0,0,6000}</Matrix>
            <Geometries>
              <Symbol uuid="s1" symdef="sym-box">
                <Matrix>{2,0,0}{0,2,0}{0,0,2}{100,0,0}</Matrix>
              </Symbol>
            </Geometries>
          </Truss>
          <Fixture name="Brick 1" uuid="aaaa">
            <FixtureID>101</FixtureID>
            <Matrix>{1,0,0}{0,1,0}{0,0,1}{-2000,3500,4200}</Matrix>
            <GDTFSpec>Astera_PB15.gdtf</GDTFSpec>
            <GDTFMode>8: RGBS</GDTFMode>
            <Addresses>
              <Address break="0">1.1</Address>
            </Addresses>
          </Fixture>
          <SceneObject name="Deck" uuid="dddd">
            <Matrix>{7.5,0,0}{0,4,0}{0,0,0.8}{0,0,400}</Matrix>
            <Geometries>
              <Geometry3D fileName="deck.glb"/>
            </Geometries>
          </SceneObject>
          <FocusPoint name="Drummer" uuid="cccc">
            <Matrix>{1,0,0}{0,1,0}{0,0,1}{0,2800,1400}</Matrix>
          </FocusPoint>
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
        assert_eq!(brick.fixture_id.as_deref(), Some("101"));
        let matrix = brick.matrix.unwrap();
        assert_eq!(matrix.o, [-2000.0, 3500.0, 4200.0]);

        let mover = &scene.fixtures[1];
        assert_eq!(mover.layer, "Back Wall", "layer tracks through groups");
        // Absolute 513 is universe 2, address 1.
        assert_eq!(mover.addresses, vec![(2, 1), (2, 100)]);
        assert!(mover.matrix.is_none());

        assert_eq!(scene.focus_points.len(), 1);
        let drummer = &scene.focus_points[0];
        assert_eq!(drummer.name, "Drummer");
        assert_eq!(drummer.matrix.unwrap().o, [0.0, 2800.0, 1400.0]);
    }

    #[test]
    fn a_focus_point_with_a_bad_matrix_degrades_with_a_warning() {
        let xml = r#"<GeneralSceneDescription><Scene><Layers><Layer name="L"><ChildList>
<FocusPoint name="F"><Matrix>nope</Matrix></FocusPoint>
</ChildList></Layer></Layers></Scene></GeneralSceneDescription>"#;
        let scene = parse_scene(xml).unwrap();
        assert_eq!(scene.focus_points.len(), 1);
        assert!(scene.focus_points[0].matrix.is_none());
        assert_eq!(scene.warnings.len(), 1, "{:?}", scene.warnings);
        assert!(scene.warnings[0].contains("focus point \"F\""));
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
    fn rotations_come_out_in_degrees() {
        let identity = Matrix {
            u: [1.0, 0.0, 0.0],
            v: [0.0, 1.0, 0.0],
            w: [0.0, 0.0, 1.0],
            o: [0.0; 3],
        };
        let (angles, exact) = identity.rotation_degrees();
        assert!(exact);
        assert!(angles.iter().all(|a| a.abs() < 1e-9), "{angles:?}");

        // Yawed 180° about Z: local x points at -x, local y at -y.
        let about_face = Matrix {
            u: [-1.0, 0.0, 0.0],
            v: [0.0, -1.0, 0.0],
            w: [0.0, 0.0, 1.0],
            o: [0.0; 3],
        };
        let (angles, exact) = about_face.rotation_degrees();
        assert!(exact);
        assert!((angles[2].abs() - 180.0).abs() < 1e-9, "{angles:?}");
        assert!(
            angles[0].abs() < 1e-9 && angles[1].abs() < 1e-9,
            "{angles:?}"
        );

        // Tilted 90° about X (a downward-hung fixture), scaled by 2: the
        // scale is normalized away, the tilt survives.
        let hung = Matrix {
            u: [2.0, 0.0, 0.0],
            v: [0.0, 0.0, 2.0],
            w: [0.0, -2.0, 0.0],
            o: [0.0; 3],
        };
        let (angles, exact) = hung.rotation_degrees();
        assert!(exact, "uniform scale is not shear");
        assert!((angles[0] - 90.0).abs() < 1e-9, "{angles:?}");

        // Sheared: reported as approximate rather than trusted.
        let sheared = Matrix {
            u: [1.0, 0.5, 0.0],
            v: [0.0, 1.0, 0.0],
            w: [0.0, 0.0, 1.0],
            o: [0.0; 3],
        };
        assert!(!sheared.rotation_degrees().1);
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

    #[test]
    fn scenery_is_collected_with_symbols_resolved() {
        let scene = parse_scene(SYNTHETIC_SCENE).unwrap();
        assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);
        assert_eq!(scene.fixtures.len(), 2, "scenery does not eat fixtures");
        assert_eq!(scene.objects.len(), 2);

        let truss = &scene.objects[0];
        assert_eq!(truss.kind, "Truss");
        assert_eq!(truss.name, "Truss A");
        assert_eq!(truss.layer, "Front Truss");
        assert_eq!(truss.matrix.unwrap().o, [0.0, 0.0, 6000.0]);
        // The symbol's two meshes, the symbol's transform composed with
        // each mesh's own.
        assert_eq!(truss.meshes.len(), 2);
        assert_eq!(truss.meshes[0].file, "Box.3ds");
        let box_matrix = truss.meshes[0].matrix.unwrap();
        assert_eq!(box_matrix.u, [2.0, 0.0, 0.0]);
        assert_eq!(box_matrix.o, [100.0, 0.0, 0.0]);
        assert_eq!(truss.meshes[1].file, "Lid.glb");
        let lid = truss.meshes[1].matrix.unwrap();
        assert_eq!(
            lid.o,
            [100.0, 0.0, 1000.0],
            "scaled by the symbol, then offset"
        );

        let deck = &scene.objects[1];
        assert_eq!(deck.kind, "SceneObject");
        assert_eq!(deck.matrix.unwrap().u, [7.5, 0.0, 0.0], "scale is kept");
        assert_eq!(
            deck.meshes,
            vec![MvrMesh {
                file: "deck.glb".to_string(),
                matrix: None
            }]
        );
    }

    #[test]
    fn a_symbol_to_nothing_is_reported() {
        let xml = SYNTHETIC_SCENE.replace("symdef=\"sym-box\"", "symdef=\"nope\"");
        let scene = parse_scene(&xml).unwrap();
        assert!(scene.objects[0].meshes.is_empty());
        assert!(
            scene.warnings.iter().any(|w| w.contains("nope")),
            "{:?}",
            scene.warnings
        );
    }

    #[test]
    fn matrices_compose_outer_then_inner() {
        let outer = Matrix {
            u: [0.0, 1.0, 0.0],
            v: [-1.0, 0.0, 0.0],
            w: [0.0, 0.0, 1.0],
            o: [10.0, 0.0, 0.0],
        };
        let inner = Matrix {
            u: [2.0, 0.0, 0.0],
            v: [0.0, 2.0, 0.0],
            w: [0.0, 0.0, 2.0],
            o: [1.0, 0.0, 0.0],
        };
        let m = outer.compose(&inner);
        // Inner's +x (scaled 2) turned by outer onto +y; the inner offset
        // turned and then shifted.
        assert_eq!(m.u, [0.0, 2.0, 0.0]);
        assert_eq!(m.o, [10.0, 1.0, 0.0]);
        assert_eq!(IDENTITY.compose(&inner), inner);
    }
}
