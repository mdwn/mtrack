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

//! What an MVR looks like from above, for the browser's import wizard: every
//! fixture's and focus point's position in the file's own millimeters, the
//! scenery objects, and — when the scenery carries a stage floor — the deck's
//! bounds. The wizard draws these on a plan so the user can click the point
//! that becomes the stage origin, and suggests the front edge of the deck.
//!
//! **Finding the deck.** MVR has no "this is the stage" marker, so a deck is
//! a plain scene object (never a truss, support, screen or projector) whose
//! name, or whose mesh's file name, says so: *stage*, *deck*, *floor*,
//! *podium*, *podest*, *riser*, *platform*, *bühne*. Its bounds come from the
//! glTF meshes it draws: each `.glb`'s `POSITION` accessor ranges, walked
//! through the glTF node transforms, turned from glTF's Y-up to the scene's
//! Z-up (the same turn the 3D view makes), then placed by the mesh's and the
//! object's transforms. Several matching objects (a deck built of sections)
//! are united. A file with no matching object, or whose matching meshes are
//! not readable `.glb`, has no deck: the answer is `None` and the wizard
//! falls back to the fixtures' footprint.

use std::collections::HashSet;
use std::error::Error;

use serde::Serialize;

use crate::lighting::mvr::{self, Matrix, MvrSceneObject};

/// Words that mark a scene object as the stage floor.
const DECK_WORDS: &[&str] = &[
    "stage", "deck", "floor", "podium", "podest", "riser", "platform", "bühne", "buhne",
];

/// Scenery kinds that are never a deck, whatever they are called.
const NOT_DECK_KINDS: &[&str] = &["Truss", "Support", "VideoScreen", "Projector"];

/// An axis-aligned box in scene millimeters.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct BoundsMm {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

impl BoundsMm {
    fn include(&mut self, point: [f64; 3]) {
        for (axis, value) in point.iter().enumerate() {
            self.min[axis] = self.min[axis].min(*value);
            self.max[axis] = self.max[axis].max(*value);
        }
    }

    fn union(&mut self, other: &BoundsMm) {
        self.include(other.min);
        self.include(other.max);
    }
}

/// A fixture as the file places it.
#[derive(Debug, Serialize)]
pub struct ViewFixture {
    pub name: String,
    pub layer: String,
    /// The fixture's position in the file's millimeters, when it has one.
    pub position_mm: Option<[f64; 3]>,
}

/// A focus point as the file places it.
#[derive(Debug, Serialize)]
pub struct ViewFocusPoint {
    pub name: String,
    pub position_mm: Option<[f64; 3]>,
}

/// A piece of scenery, with its bounds when they could be read.
#[derive(Debug, Serialize)]
pub struct ViewScenery {
    pub name: String,
    pub kind: String,
    /// Whether this object was recognised as (part of) the stage floor.
    pub deck: bool,
    /// Its extent in the file's millimeters, when a `.glb` mesh gave one.
    pub bounds_mm: Option<BoundsMm>,
}

/// The file seen from above.
#[derive(Debug, Serialize)]
pub struct SceneView {
    pub fixtures: Vec<ViewFixture>,
    pub focus_points: Vec<ViewFocusPoint>,
    pub scenery: Vec<ViewScenery>,
    /// The stage floor's bounds, when the scenery carries one.
    pub deck_mm: Option<BoundsMm>,
}

/// Reads an MVR's positions and scenery bounds. Writes nothing.
pub fn scene_view(bytes: &[u8]) -> Result<SceneView, Box<dyn Error>> {
    let scene = mvr::parse_archive(bytes)?;
    let entries: Vec<String> = mvr::list_entries(bytes)?;
    let mut deck: Option<BoundsMm> = None;
    let mut scenery = Vec::new();
    for object in &scene.objects {
        let is_deck = looks_like_deck(object);
        let bounds_mm = object_bounds(bytes, &entries, object);
        if is_deck {
            if let Some(bounds) = &bounds_mm {
                match &mut deck {
                    Some(total) => total.union(bounds),
                    None => deck = Some(*bounds),
                }
            }
        }
        scenery.push(ViewScenery {
            name: object.name.clone(),
            kind: object.kind.clone(),
            deck: is_deck,
            bounds_mm,
        });
    }
    Ok(SceneView {
        fixtures: scene
            .fixtures
            .iter()
            .map(|f| ViewFixture {
                name: f.name.clone(),
                layer: f.layer.clone(),
                position_mm: f.matrix.map(|m| m.o),
            })
            .collect(),
        focus_points: scene
            .focus_points
            .iter()
            .map(|f| ViewFocusPoint {
                name: f.name.clone(),
                position_mm: f.matrix.map(|m| m.o),
            })
            .collect(),
        scenery,
        deck_mm: deck,
    })
}

fn looks_like_deck(object: &MvrSceneObject) -> bool {
    if NOT_DECK_KINDS.contains(&object.kind.as_str()) {
        return false;
    }
    let says_deck = |text: &str| {
        let lower = text.to_lowercase();
        DECK_WORDS.iter().any(|word| lower.contains(word))
    };
    says_deck(&object.name) || object.meshes.iter().any(|m| says_deck(&m.file))
}

/// The union of an object's `.glb` meshes' bounds, in scene millimeters.
fn object_bounds(bytes: &[u8], entries: &[String], object: &MvrSceneObject) -> Option<BoundsMm> {
    let object_matrix = object.matrix.unwrap_or(mvr::IDENTITY);
    let mut total: Option<BoundsMm> = None;
    for mesh in &object.meshes {
        if !mesh.file.to_ascii_lowercase().ends_with(".glb") {
            continue;
        }
        let Some(entry) = find_entry(entries, &mesh.file) else {
            continue;
        };
        let Ok(glb) = mvr::read_mesh_entry(bytes, &entry) else {
            continue;
        };
        let Some(corners) = glb_corners(&glb) else {
            continue;
        };
        let placed = object_matrix.compose(&mesh.matrix.unwrap_or(mvr::IDENTITY));
        for corner in corners {
            let point = apply_mm(&placed, corner);
            match &mut total {
                Some(bounds) => bounds.include(point),
                None => {
                    total = Some(BoundsMm {
                        min: point,
                        max: point,
                    })
                }
            }
        }
    }
    total.filter(|b| b.min.iter().chain(&b.max).all(|v| v.is_finite()))
}

/// A mesh point in the mesh's own meters, placed by a matrix whose rotation
/// and scale act on meters and whose translation is millimeters: the point
/// in scene millimeters.
fn apply_mm(matrix: &Matrix, p: [f64; 3]) -> [f64; 3] {
    let p = [p[0] * 1000.0, p[1] * 1000.0, p[2] * 1000.0];
    [
        matrix.u[0] * p[0] + matrix.v[0] * p[1] + matrix.w[0] * p[2] + matrix.o[0],
        matrix.u[1] * p[0] + matrix.v[1] * p[1] + matrix.w[1] * p[2] + matrix.o[1],
        matrix.u[2] * p[0] + matrix.v[2] * p[1] + matrix.w[2] * p[2] + matrix.o[2],
    ]
}

/// Entry names are matched as written, then case-folded.
fn find_entry(entries: &[String], file: &str) -> Option<String> {
    entries
        .iter()
        .find(|e| e.as_str() == file)
        .or_else(|| entries.iter().find(|e| e.eq_ignore_ascii_case(file)))
        .cloned()
}

// --- glTF -------------------------------------------------------------------

type Mat4 = [[f64; 4]; 4];

const IDENTITY4: Mat4 = [
    [1.0, 0.0, 0.0, 0.0],
    [0.0, 1.0, 0.0, 0.0],
    [0.0, 0.0, 1.0, 0.0],
    [0.0, 0.0, 0.0, 1.0],
];

fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
    let mut out = [[0.0; 4]; 4];
    for (r, row) in out.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = (0..4).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    out
}

/// The eight corners of every mesh's position range in a GLB, in the scene's
/// Z-up meters (node transforms applied, Y-up turned to Z-up).
fn glb_corners(glb: &[u8]) -> Option<Vec<[f64; 3]>> {
    if glb.len() < 20 || &glb[0..4] != b"glTF" {
        return None;
    }
    let json_len = u32::from_le_bytes(glb[12..16].try_into().ok()?) as usize;
    if &glb[16..20] != b"JSON" || glb.len() < 20 + json_len {
        return None;
    }
    let doc: serde_json::Value = serde_json::from_slice(&glb[20..20 + json_len]).ok()?;
    let nodes = doc.get("nodes")?.as_array()?;
    let scene_index = doc.get("scene").and_then(|s| s.as_u64()).unwrap_or(0) as usize;
    let roots: Vec<usize> = doc
        .get("scenes")
        .and_then(|s| s.get(scene_index))
        .and_then(|s| s.get("nodes"))
        .and_then(|n| n.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_u64())
                .map(|v| v as usize)
                .collect()
        })
        .unwrap_or_default();
    let mut corners = Vec::new();
    let mut visited = HashSet::new();
    // A visited set keeps a cyclic file from looping.
    let mut stack: Vec<(usize, Mat4)> = roots.into_iter().map(|r| (r, IDENTITY4)).collect();
    while let Some((index, parent)) = stack.pop() {
        if !visited.insert(index) {
            continue;
        }
        let node = nodes.get(index)?;
        let world = mul(&parent, &node_matrix(node));
        if let Some(mesh) = node.get("mesh").and_then(|m| m.as_u64()) {
            let mesh = doc.get("meshes")?.get(mesh as usize)?;
            for primitive in mesh.get("primitives")?.as_array()? {
                let accessor = primitive
                    .get("attributes")
                    .and_then(|a| a.get("POSITION"))
                    .and_then(|p| p.as_u64())
                    .and_then(|i| doc.get("accessors")?.get(i as usize));
                let Some((min, max)) =
                    accessor.and_then(|a| Some((triple(a.get("min")?)?, triple(a.get("max")?)?)))
                else {
                    continue;
                };
                for i in 0..8 {
                    let local = [
                        if i & 1 == 0 { min[0] } else { max[0] },
                        if i & 2 == 0 { min[1] } else { max[1] },
                        if i & 4 == 0 { min[2] } else { max[2] },
                    ];
                    let g: Vec<f64> = (0..3)
                        .map(|r| {
                            world[r][0] * local[0]
                                + world[r][1] * local[1]
                                + world[r][2] * local[2]
                                + world[r][3]
                        })
                        .collect();
                    // glTF is Y-up; the scene is Z-up (a quarter turn about X).
                    corners.push([g[0], -g[2], g[1]]);
                }
            }
        }
        if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
            for child in children.iter().filter_map(|c| c.as_u64()) {
                stack.push((child as usize, world));
            }
        }
    }
    (!corners.is_empty()).then_some(corners)
}

fn triple(value: &serde_json::Value) -> Option<[f64; 3]> {
    let a = value.as_array()?;
    Some([
        a.first()?.as_f64()?,
        a.get(1)?.as_f64()?,
        a.get(2)?.as_f64()?,
    ])
}

/// A glTF node's local transform, row-major.
fn node_matrix(node: &serde_json::Value) -> Mat4 {
    if let Some(m) = node.get("matrix").and_then(|m| m.as_array()) {
        if m.len() == 16 {
            let v: Vec<f64> = m.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect();
            // Column-major in the file.
            let mut out = [[0.0; 4]; 4];
            for (c, column) in v.chunks(4).enumerate() {
                for (r, value) in column.iter().enumerate() {
                    out[r][c] = *value;
                }
            }
            return out;
        }
    }
    let t = node.get("translation").and_then(triple).unwrap_or([0.0; 3]);
    let s = node.get("scale").and_then(triple).unwrap_or([1.0; 3]);
    let q = node
        .get("rotation")
        .and_then(|r| r.as_array())
        .filter(|r| r.len() == 4)
        .map(|r| {
            [
                r[0].as_f64().unwrap_or(0.0),
                r[1].as_f64().unwrap_or(0.0),
                r[2].as_f64().unwrap_or(0.0),
                r[3].as_f64().unwrap_or(1.0),
            ]
        })
        .unwrap_or([0.0, 0.0, 0.0, 1.0]);
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let r = [
        [
            1.0 - 2.0 * (y * y + z * z),
            2.0 * (x * y - z * w),
            2.0 * (x * z + y * w),
        ],
        [
            2.0 * (x * y + z * w),
            1.0 - 2.0 * (x * x + z * z),
            2.0 * (y * z - x * w),
        ],
        [
            2.0 * (x * z - y * w),
            2.0 * (y * z + x * w),
            1.0 - 2.0 * (x * x + y * y),
        ],
    ];
    let mut out = IDENTITY4;
    for row in 0..3 {
        for col in 0..3 {
            out[row][col] = r[row][col] * s[col];
        }
        out[row][3] = t[row];
    }
    out
}

/// A GLB whose single mesh spans `min`..`max` (glTF Y-up meters), for tests.
#[cfg(test)]
pub(crate) fn test_glb(min: [f64; 3], max: [f64; 3]) -> Vec<u8> {
    let json = serde_json::json!({
        "asset": {"version": "2.0"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [{"mesh": 0}],
        "meshes": [{"primitives": [{"attributes": {"POSITION": 0}}]}],
        "accessors": [{"componentType": 5126, "count": 8, "type": "VEC3", "min": min, "max": max}],
    })
    .to_string();
    let mut json = json.into_bytes();
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let mut out = Vec::new();
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&((12 + 8 + json.len()) as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(b"JSON");
    out.extend_from_slice(&json);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::gdtf::build_zip;

    fn mvr_with(objects: &str, glbs: &[(&str, Vec<u8>)]) -> Vec<u8> {
        let scene = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<GeneralSceneDescription verMajor="1" verMinor="6"><Scene><Layers>
<Layer name="Stage"><ChildList>
<Fixture name="Brick 1"><Matrix>{{1,0,0}}{{0,1,0}}{{0,0,1}}{{-2000,3500,4200}}</Matrix></Fixture>
<FocusPoint name="Drummer"><Matrix>{{1,0,0}}{{0,1,0}}{{0,0,1}}{{0,2800,1400}}</Matrix></FocusPoint>
{objects}
</ChildList></Layer></Layers></Scene></GeneralSceneDescription>"#
        );
        let mut files: Vec<(&str, &[u8])> = vec![("GeneralSceneDescription.xml", scene.as_bytes())];
        for (name, bytes) in glbs {
            files.push((name, bytes.as_slice()));
        }
        build_zip(&files)
    }

    #[test]
    fn positions_come_out_in_the_files_millimeters() {
        let view = scene_view(&mvr_with("", &[])).unwrap();
        assert_eq!(view.fixtures.len(), 1);
        assert_eq!(
            view.fixtures[0].position_mm,
            Some([-2000.0, 3500.0, 4200.0])
        );
        assert_eq!(
            view.focus_points[0].position_mm,
            Some([0.0, 2800.0, 1400.0])
        );
        assert!(view.deck_mm.is_none(), "no scenery, no deck");
    }

    #[test]
    fn a_deck_named_object_gives_bounds_in_scene_millimeters() {
        // A 6 m x 4 m x 0.5 m slab (glTF Y-up: height is Y, depth is -Z),
        // centred on the object's origin, the object placed 1 m upstage.
        let glb = test_glb([-3.0, 0.0, -4.0], [3.0, 0.5, 0.0]);
        let view = scene_view(&mvr_with(
            r#"<SceneObject name="Main Stage"><Matrix>{1,0,0}{0,1,0}{0,0,1}{0,1000,0}</Matrix>
<Geometries><Geometry3D fileName="slab.glb"/></Geometries></SceneObject>
<SceneObject name="Lamp"><Geometries><Geometry3D fileName="slab.glb"/></Geometries></SceneObject>
<Truss name="Stage truss"><Geometries><Geometry3D fileName="slab.glb"/></Geometries></Truss>"#,
            &[("slab.glb", glb)],
        ))
        .unwrap();
        let deck = view.deck_mm.expect("a deck");
        assert_eq!(deck.min, [-3000.0, 1000.0, 0.0]);
        assert_eq!(deck.max, [3000.0, 5000.0, 500.0]);
        assert_eq!(view.scenery.iter().filter(|s| s.deck).count(), 1);
        assert!(
            view.scenery[1].bounds_mm.is_some(),
            "other bounds still read"
        );
    }

    #[test]
    fn a_deck_whose_mesh_is_not_a_readable_glb_has_no_bounds() {
        let view = scene_view(&mvr_with(
            r#"<SceneObject name="Deck"><Geometries><Geometry3D fileName="deck.3ds"/></Geometries></SceneObject>"#,
            &[("deck.3ds", b"3ds".to_vec())],
        ))
        .unwrap();
        assert!(view.scenery[0].deck);
        assert!(view.deck_mm.is_none());
        let view = scene_view(&mvr_with(
            r#"<SceneObject name="Deck"><Geometries><Geometry3D fileName="bad.glb"/></Geometries></SceneObject>"#,
            &[("bad.glb", b"not a glb at all, sorry".to_vec())],
        ))
        .unwrap();
        assert!(view.deck_mm.is_none());
    }
}
