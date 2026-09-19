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

//! The rig model (venue-exchange design §16.2): what a fixture type looks
//! like and how it moves, distilled from the GDTF geometry tree into the
//! small, machine-only description a renderer needs.
//!
//! One node per geometry the mode drives, each with its parent, its
//! transform, and a shape — a mesh file the asset cache holds, or a GDTF
//! primitive with a size. The pan and tilt nodes are the axes the mode's
//! `Pan` and `Tilt` channels name; beams carry their photometrics; a
//! `GeometryReference` becomes a numbered cell carrying the referenced
//! geometry's beam and children. Conventions the viewer relies on, from the
//! GDTF spec: a beam emits along its node's local −Z, pan turns about the
//! pan node's local Z, tilt about the tilt node's local X, and the fixture
//! at rest hangs base-up with the head pointing −Z (straight down).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use super::description::{Description, GeometryKind, GeometryNode, Matrix4};
use super::GdtfError;

/// Bumped whenever the rig distilled from the same source can change; part
/// of the asset cache's file name, so an upgrade regenerates every rig.
pub const RIG_VERSION: u32 = 1;

/// The deepest chain of geometry references followed. Real fixtures nest
/// one level (cells in a head); a reference cycle is an attack, not a rig.
const MAX_REFERENCE_DEPTH: usize = 8;

/// A fixture type's rig, as the asset cache stores it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RigModel {
    /// The rig format version this was written by.
    pub version: u32,
    /// The GDTF fixture type's name.
    pub fixture_type: String,
    /// The mode the rig was distilled for.
    pub mode: String,
    /// The nodes, parents before children; the first is the root.
    pub nodes: Vec<RigNode>,
    /// The node that pans (about its local Z), if the mode has a pan.
    pub pan: Option<usize>,
    /// The node that tilts (about its local X), if the mode has a tilt.
    pub tilt: Option<usize>,
    /// The light sources.
    pub beams: Vec<RigBeam>,
    /// The thumbnail's path relative to the rig file, when the cache holds
    /// one.
    pub thumbnail: Option<String>,
    /// What was guessed or skipped, one line each — a rig is a picture,
    /// so it is drawn anyway, and the fill logs these.
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// One node of a rig.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RigNode {
    /// The geometry's name.
    pub name: String,
    /// The parent's index in [`RigModel::nodes`].
    pub parent: Option<usize>,
    /// What the node does.
    pub role: RigRole,
    /// The node's transform relative to its parent: row-major 4×4,
    /// translation in the fourth column, meters.
    pub transform: Matrix4,
    /// What the node looks like.
    pub shape: RigShape,
}

/// What a rig node does.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RigRole {
    /// A static part.
    Body,
    /// The part the pan channel turns.
    Pan,
    /// The part the tilt channel turns.
    Tilt,
    /// A light source.
    Beam,
    /// A pixel cell — the `index`-th geometry reference in the mode's tree,
    /// document order.
    Cell { index: usize },
}

/// What a rig node looks like.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum RigShape {
    /// A mesh in the asset cache, by path relative to the rig file.
    Model { file: String },
    /// A GDTF primitive (`Cube`, `Cylinder`, `Sphere`, `Base`, `Yoke`,
    /// `Head`, `Pigtail`, ...) of the given size in meters (x, y, z).
    Primitive { kind: String, size: [f64; 3] },
    /// Nothing to draw: a grouping node.
    Empty,
}

/// A light source.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RigBeam {
    /// The node the beam leaves, along its local −Z.
    pub node: usize,
    /// Beam angle in degrees.
    pub angle_deg: f64,
    /// Field angle in degrees, when the GDTF states one.
    pub field_deg: Option<f64>,
    /// `Wash`, `Spot`, `None`, ...
    pub kind: String,
    /// Luminous flux in lumens, when stated.
    pub flux_lm: Option<f64>,
    /// Color temperature in kelvin, when stated.
    pub cct_k: Option<f64>,
    /// Lens radius in meters, when stated.
    pub radius_m: Option<f64>,
}

/// Where a rig's first beam points in the mounting frame at a pose, by
/// forward kinematics through the GDTF geometry: every node's own
/// transform, the pan node turned about its local Z by `pan`, the tilt
/// node about its local X by `tilt`, and the beam leaving its node along
/// −Z — the way Blender DMX renders a GDTF's physical values. `None` for
/// a rig with no beam. Shares nothing with the pointing math but the
/// spec's definitions, which is what makes it a cross-check of it.
pub fn beam_direction(rig: &RigModel, pan_deg: f64, tilt_deg: f64) -> Option<[f64; 3]> {
    let beam = rig.beams.first()?;
    // The chain from the root down to the beam's node.
    let mut chain = Vec::new();
    let mut at = Some(beam.node);
    while let Some(index) = at {
        chain.push(index);
        at = rig.nodes.get(index)?.parent;
    }
    chain.reverse();

    let mut r = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    for index in chain {
        let node = &rig.nodes[index];
        let t = node.transform;
        let own = [
            [t[0][0], t[0][1], t[0][2]],
            [t[1][0], t[1][1], t[1][2]],
            [t[2][0], t[2][1], t[2][2]],
        ];
        r = mat_mul(r, own);
        if rig.pan == Some(index) {
            let (c, s) = (pan_deg.to_radians().cos(), pan_deg.to_radians().sin());
            r = mat_mul(r, [[c, -s, 0.0], [s, c, 0.0], [0.0, 0.0, 1.0]]);
        }
        if rig.tilt == Some(index) {
            let (c, s) = (tilt_deg.to_radians().cos(), tilt_deg.to_radians().sin());
            r = mat_mul(r, [[1.0, 0.0, 0.0], [0.0, c, -s], [0.0, s, c]]);
        }
    }
    let d = [-r[0][2], -r[1][2], -r[2][2]];
    let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    (length > 1e-9).then(|| [d[0] / length, d[1] / length, d[2] / length])
}

fn mat_mul(a: [[f64; 3]; 3], b: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

/// The beam angle assumed when a GDTF's beam states none.
pub const DEFAULT_BEAM_ANGLE: f64 = 20.0;

/// Distills a mode's rig from a description. `model_files` names the mesh
/// stems the archive actually carries under `models/gltf/`; a model whose
/// file is missing falls back to its primitive.
pub fn distill_rig(
    description: &Description,
    mode_name: &str,
    model_files: &HashSet<String>,
) -> Result<RigModel, GdtfError> {
    let matched = super::match_mode(description, mode_name)?;
    let mode = description
        .modes
        .iter()
        .find(|m| m.name == matched.name)
        .expect("match_mode returns a mode of the description");

    // The axes are whatever geometry the mode's Pan and Tilt channels sit
    // on — the yoke and the head, on every mover in the corpus.
    let axis_for = |attribute: &str| -> Option<&str> {
        mode.channels
            .iter()
            .find(|c| c.logical_channels.iter().any(|l| l.attribute == attribute))
            .map(|c| c.geometry.as_str())
    };
    let pan_geometry = axis_for("Pan");
    let tilt_geometry = axis_for("Tilt");

    let mut rig = RigModel {
        version: RIG_VERSION,
        fixture_type: description.name.clone(),
        mode: matched.name.clone(),
        nodes: Vec::new(),
        pan: None,
        tilt: None,
        beams: Vec::new(),
        thumbnail: None,
        warnings: Vec::new(),
    };

    // The mode's root, or the first top-level geometry when the mode names
    // none the tree has (a rig is a picture; a wrong root is still one,
    // and it is said so).
    let root = description
        .geometries
        .iter()
        .position(|g| g.parent.is_none() && g.name == mode.geometry);
    let root = match root {
        Some(root) => root,
        None => {
            let Some(first) = description
                .geometries
                .iter()
                .position(|g| g.parent.is_none())
            else {
                rig.warnings
                    .push("the GDTF has no geometry tree; nothing to draw".to_string());
                return Ok(rig);
            };
            rig.warnings.push(format!(
                "mode \"{}\" names root geometry \"{}\", which the tree lacks; drawing \"{}\"",
                matched.name, mode.geometry, description.geometries[first].name
            ));
            first
        }
    };

    let mut walk = RigWalk {
        description,
        model_files,
        pan_geometry,
        tilt_geometry,
        rig: &mut rig,
        cells: 0,
        chain: Vec::new(),
    };
    walk.emit(root, None, None);
    Ok(rig)
}

struct RigWalk<'a> {
    description: &'a Description,
    model_files: &'a HashSet<String>,
    pan_geometry: Option<&'a str>,
    tilt_geometry: Option<&'a str>,
    rig: &'a mut RigModel,
    cells: usize,
    /// The reference chain being expanded, for cycle detection.
    chain: Vec<String>,
}

impl RigWalk<'_> {
    /// Emits `index` under `parent` (a rig node) and recurses into its
    /// children. `role_override` places a referenced geometry's root onto
    /// the reference's own node instead of a fresh one.
    fn emit(&mut self, index: usize, parent: Option<usize>, cell_node: Option<usize>) {
        let node = &self.description.geometries[index];
        let out = match cell_node {
            // A referenced root merges into the reference's node: its
            // shape when the reference has none, its beam always.
            Some(out) => {
                if self.rig.nodes[out].shape == RigShape::Empty {
                    self.rig.nodes[out].shape = self.shape_for(node);
                }
                self.push_beam(out, node);
                out
            }
            None => {
                let role = self.role_for(node);
                let shape = self.shape_for(node);
                self.rig.nodes.push(RigNode {
                    name: node.name.clone(),
                    parent,
                    role: role.clone(),
                    transform: node.position,
                    shape,
                });
                let out = self.rig.nodes.len() - 1;
                match role {
                    RigRole::Pan => self.rig.pan = Some(out),
                    RigRole::Tilt => self.rig.tilt = Some(out),
                    _ => {}
                }
                self.push_beam(out, node);
                out
            }
        };

        if node.kind == GeometryKind::Reference {
            self.expand_reference(node, out);
        }

        let children: Vec<usize> = self
            .description
            .geometries
            .iter()
            .enumerate()
            .filter(|(_, g)| g.parent == Some(index))
            .map(|(i, _)| i)
            .collect();
        for child in children {
            self.emit(child, Some(out), None);
        }
    }

    /// Draws the geometry a reference points at onto the reference's node.
    /// The spec has references name top-level geometries; one naming a
    /// nested geometry is honored anyway (preferring a top-level match),
    /// and one naming nothing is said so.
    fn expand_reference(&mut self, node: &GeometryNode, out: usize) {
        let Some(referenced) = node.reference.as_deref() else {
            self.rig.warnings.push(format!(
                "reference \"{}\" names no geometry; drawn empty",
                node.name
            ));
            return;
        };
        let geometries = &self.description.geometries;
        let target = geometries
            .iter()
            .position(|g| g.parent.is_none() && g.name == referenced)
            .or_else(|| geometries.iter().position(|g| g.name == referenced));
        let Some(target) = target else {
            self.rig.warnings.push(format!(
                "reference \"{}\" names geometry \"{referenced}\", which the tree lacks; drawn empty",
                node.name
            ));
            return;
        };
        if self.chain.iter().any(|c| c == referenced) {
            self.rig.warnings.push(format!(
                "reference \"{}\" to \"{referenced}\" is a cycle; stopped",
                node.name
            ));
            return;
        }
        if self.chain.len() >= MAX_REFERENCE_DEPTH {
            self.rig.warnings.push(format!(
                "reference \"{}\" nests deeper than {MAX_REFERENCE_DEPTH}; stopped",
                node.name
            ));
            return;
        }
        self.chain.push(referenced.to_string());
        self.emit(target, Some(out), Some(out));
        self.chain.pop();
    }

    fn role_for(&mut self, node: &GeometryNode) -> RigRole {
        if self.pan_geometry == Some(node.name.as_str()) {
            return RigRole::Pan;
        }
        if self.tilt_geometry == Some(node.name.as_str()) {
            return RigRole::Tilt;
        }
        match node.kind {
            GeometryKind::Beam => RigRole::Beam,
            GeometryKind::Reference => {
                let index = self.cells;
                self.cells += 1;
                RigRole::Cell { index }
            }
            _ => RigRole::Body,
        }
    }

    fn shape_for(&self, node: &GeometryNode) -> RigShape {
        let Some(model_name) = node.model.as_deref() else {
            return RigShape::Empty;
        };
        let Some(model) = self
            .description
            .models
            .iter()
            .find(|m| m.name == model_name)
        else {
            return RigShape::Empty;
        };
        if let Some(file) = model.file.as_deref() {
            if self.model_files.contains(&file.to_ascii_lowercase()) {
                return RigShape::Model {
                    file: format!("models/{}.glb", file.to_ascii_lowercase()),
                };
            }
        }
        let kind = if model.primitive.eq_ignore_ascii_case("undefined") {
            "Cube".to_string()
        } else {
            model.primitive.clone()
        };
        RigShape::Primitive {
            kind,
            size: model.size,
        }
    }

    fn push_beam(&mut self, out: usize, node: &GeometryNode) {
        let Some(beam) = node.beam.as_ref() else {
            return;
        };
        // A node draws one beam: a reference chain whose every level is a
        // beam keeps the outermost.
        if self.rig.beams.iter().any(|b| b.node == out) {
            return;
        }
        self.rig.beams.push(RigBeam {
            node: out,
            angle_deg: beam
                .beam_angle
                .filter(|a| *a > 0.0)
                .unwrap_or(DEFAULT_BEAM_ANGLE),
            field_deg: beam.field_angle.filter(|a| *a > 0.0),
            kind: beam.beam_type.clone().unwrap_or_else(|| "Wash".to_string()),
            flux_lm: beam.luminous_flux,
            cct_k: beam.color_temperature,
            radius_m: beam.beam_radius,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::gdtf::{parse_description, SYNTHETIC_DESCRIPTION};

    fn synthetic() -> Description {
        parse_description(SYNTHETIC_DESCRIPTION).unwrap()
    }

    fn names(rig: &RigModel) -> Vec<&str> {
        rig.nodes.iter().map(|n| n.name.as_str()).collect()
    }

    #[test]
    fn a_mover_mode_names_its_axes_and_beam() {
        let files: HashSet<String> = ["yoke".to_string()].into_iter().collect();
        let rig = distill_rig(&synthetic(), "Mover 16bit", &files).unwrap();
        assert_eq!(
            names(&rig),
            ["Base", "Yoke", "Head", "Lens", "Pixel 2", "Cell Lens"]
        );
        assert_eq!(rig.pan, Some(1));
        assert_eq!(rig.tilt, Some(2));
        assert_eq!(rig.nodes[1].role, RigRole::Pan);
        assert_eq!(rig.nodes[2].role, RigRole::Tilt);
        assert_eq!(rig.nodes[2].parent, Some(1));
        assert_eq!(rig.nodes[3].role, RigRole::Beam);
        assert_eq!(rig.nodes[3].parent, Some(2));
        assert_eq!(rig.nodes[1].transform[2][3], -0.1);

        // The yoke has a mesh in the archive; the base and head fall back
        // to their primitives.
        assert_eq!(
            rig.nodes[1].shape,
            RigShape::Model {
                file: "models/yoke.glb".to_string()
            }
        );
        assert_eq!(
            rig.nodes[0].shape,
            RigShape::Primitive {
                kind: "Base".to_string(),
                size: [0.30, 0.20, 0.10]
            }
        );

        // The head's lens, then the cell's.
        assert_eq!(rig.beams.len(), 2);
        assert_eq!(rig.beams[0].node, 3);
        assert_eq!(rig.beams[0].angle_deg, 12.0);
        assert_eq!(rig.beams[0].field_deg, Some(20.0));
        assert_eq!(rig.beams[0].kind, "Spot");
        assert_eq!(rig.beams[0].flux_lm, Some(5000.0));
        assert_eq!(rig.beams[0].cct_k, Some(6500.0));
        assert_eq!(rig.beams[0].radius_m, Some(0.05));
    }

    #[test]
    fn a_reference_becomes_a_cell_carrying_the_referenced_geometry() {
        let rig = distill_rig(&synthetic(), "8: RGBS", &HashSet::new()).unwrap();
        let pixel = rig.nodes.iter().position(|n| n.name == "Pixel 2").unwrap();
        assert_eq!(rig.nodes[pixel].role, RigRole::Cell { index: 0 });
        assert_eq!(rig.nodes[pixel].parent, Some(0));
        assert_eq!(rig.nodes[pixel].transform[0][3], 0.05);
        // The reference's own model wins; the referenced Cell's beam child
        // hangs under the cell node.
        assert_eq!(
            rig.nodes[pixel].shape,
            RigShape::Primitive {
                kind: "Cylinder".to_string(),
                size: [0.05, 0.05, 0.01]
            }
        );
        let cell_lens = rig
            .nodes
            .iter()
            .position(|n| n.name == "Cell Lens")
            .unwrap();
        assert_eq!(rig.nodes[cell_lens].parent, Some(pixel));
        let beam = rig.beams.iter().find(|b| b.node == cell_lens).unwrap();
        assert_eq!(beam.angle_deg, 30.0);
        assert_eq!(beam.kind, "Wash");
        assert_eq!(beam.field_deg, None);
        // No mover channels: no axes.
        assert_eq!(rig.pan, None);
        assert_eq!(rig.tilt, None);
        // A missing mesh (no "yoke" in the archive) is its primitive, and
        // an Undefined primitive is a cube.
        let yoke = rig.nodes.iter().position(|n| n.name == "Yoke").unwrap();
        assert_eq!(
            rig.nodes[yoke].shape,
            RigShape::Primitive {
                kind: "Cube".to_string(),
                size: [0.30, 0.10, 0.25]
            }
        );
    }

    #[test]
    fn a_reference_cycle_stops() {
        let xml = SYNTHETIC_DESCRIPTION.replace(
            "<Geometry Name=\"Cell\" Model=\"Cell\">",
            "<Geometry Name=\"Cell\" Model=\"Cell\"><GeometryReference Name=\"Loop\" Geometry=\"Cell\"/>",
        );
        let description = parse_description(&xml).unwrap();
        let rig = distill_rig(&description, "8: RGBS", &HashSet::new()).unwrap();
        // Pixel 2 → Cell (Loop → Cell refused as a cycle, but Loop itself
        // is a node).
        assert!(names(&rig).contains(&"Loop"));
        assert!(rig.nodes.len() < 12, "{:?}", names(&rig));
        assert!(
            rig.warnings.iter().any(|w| w.contains("cycle")),
            "{:?}",
            rig.warnings
        );
    }

    #[test]
    fn a_dangling_or_nested_reference_is_reported_or_honored() {
        // Nothing named "Ghost": an empty cell, and a warning naming it.
        let xml = SYNTHETIC_DESCRIPTION.replace(
            "Geometry=\"Cell\" Model=\"Cell\"",
            "Geometry=\"Ghost\" Model=\"Cell\"",
        );
        let rig = distill_rig(
            &parse_description(&xml).unwrap(),
            "8: RGBS",
            &HashSet::new(),
        )
        .unwrap();
        assert!(!names(&rig).contains(&"Cell Lens"));
        assert!(
            rig.warnings.iter().any(|w| w.contains("\"Ghost\"")),
            "{:?}",
            rig.warnings
        );
        // A reference to a nested geometry (the head's lens) draws it.
        let xml = SYNTHETIC_DESCRIPTION.replace(
            "Geometry=\"Cell\" Model=\"Cell\"",
            "Geometry=\"Lens\" Model=\"Cell\"",
        );
        let rig = distill_rig(
            &parse_description(&xml).unwrap(),
            "8: RGBS",
            &HashSet::new(),
        )
        .unwrap();
        let pixel = rig.nodes.iter().position(|n| n.name == "Pixel 2").unwrap();
        assert!(rig
            .beams
            .iter()
            .any(|b| b.node == pixel && b.angle_deg == 12.0));
        assert!(rig.warnings.is_empty(), "{:?}", rig.warnings);
    }

    #[test]
    fn a_description_without_geometry_is_an_empty_rig() {
        let xml = SYNTHETIC_DESCRIPTION.replace("Geometry=\"Base\"", "Geometry=\"Nowhere\"");
        let description = parse_description(&xml).unwrap();
        // The mode names a root the tree lacks: the first top-level one
        // stands in, and the rig says so.
        let rig = distill_rig(&description, "8: RGBS", &HashSet::new()).unwrap();
        assert_eq!(rig.nodes[0].name, "Base");
        assert!(
            rig.warnings
                .iter()
                .any(|w| w.contains("\"Nowhere\"") && w.contains("\"Base\"")),
            "{:?}",
            rig.warnings
        );

        let mut bare = parse_description(SYNTHETIC_DESCRIPTION).unwrap();
        bare.geometries.clear();
        let rig = distill_rig(&bare, "8: RGBS", &HashSet::new()).unwrap();
        assert!(rig.nodes.is_empty());
        assert!(rig.beams.is_empty());
        assert_eq!(rig.warnings.len(), 1, "{:?}", rig.warnings);
    }

    #[test]
    fn rigs_round_trip_through_json() {
        let rig = distill_rig(&synthetic(), "Mover 16bit", &HashSet::new()).unwrap();
        let json = serde_json::to_string(&rig).unwrap();
        assert!(json.contains("\"kind\":\"pan\""), "{json}");
        assert_eq!(serde_json::from_str::<RigModel>(&json).unwrap(), rig);
    }
}
