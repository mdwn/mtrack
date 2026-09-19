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
    beam_ray(rig, pan_deg, tilt_deg).map(|(_, direction)| direction)
}

/// The first beam's ray at a pose in the mounting frame: where the lens
/// is (meters from the mounting point) and which way it points, by the
/// same forward kinematics as [`beam_direction`], translations included.
pub fn beam_ray(rig: &RigModel, pan_deg: f64, tilt_deg: f64) -> Option<([f64; 3], [f64; 3])> {
    let chain = beam_chain(rig)?;

    let mut r = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let mut origin = [0.0; 3];
    for index in chain {
        let node = &rig.nodes[index];
        let t = node.transform;
        let own = [
            [t[0][0], t[0][1], t[0][2]],
            [t[1][0], t[1][1], t[1][2]],
            [t[2][0], t[2][1], t[2][2]],
        ];
        let step = crate::lighting::effects::mat_vec(r, [t[0][3], t[1][3], t[2][3]]);
        origin = [
            origin[0] + step[0],
            origin[1] + step[1],
            origin[2] + step[2],
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
    (length > 1e-9).then(|| (origin, [d[0] / length, d[1] / length, d[2] / length]))
}

/// How this rig's joints sit in its mounting frame (design §18.6): the
/// rotation its geometry puts before the pan joint, any yaw between the
/// pan and tilt joints, and the angle the beam rests at in the head. A
/// rig whose geometry does not reduce to that — a tilt axis that is not
/// the yoke's X, a beam that is not in the head's Y–Z plane — cannot be
/// aimed by the closed form; the reason is returned so the fixture type
/// can say so and fall back to the plain math.
pub fn aim_calibration(rig: &RigModel) -> Result<crate::lighting::effects::AimCalibration, String> {
    use crate::lighting::effects::AimCalibration;
    let (Some(pan), Some(tilt)) = (rig.pan, rig.tilt) else {
        return Ok(AimCalibration::IDENTITY);
    };
    let Some(chain) = beam_chain(rig) else {
        return Ok(AimCalibration::IDENTITY);
    };
    let pan_at = chain
        .iter()
        .position(|&i| i == pan)
        .ok_or("the pan node is not above the beam")?;
    let tilt_at = chain
        .iter()
        .position(|&i| i == tilt)
        .ok_or("the tilt node is not above the beam")?;
    if tilt_at < pan_at {
        return Err("the tilt node is above the pan node".to_string());
    }
    let rotation_of = |index: usize| {
        let t = rig.nodes[index].transform;
        [
            [t[0][0], t[0][1], t[0][2]],
            [t[1][0], t[1][1], t[1][2]],
            [t[2][0], t[2][1], t[2][2]],
        ]
    };
    // The rotation a run of nodes composes, and where its last node's
    // origin ends up in the frame the run starts in.
    let walk = |nodes: &[usize]| {
        nodes.iter().fold(
            (
                [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
                [0.0; 3],
            ),
            |(r, origin), &i| {
                let t = rig.nodes[i].transform;
                let step = crate::lighting::effects::mat_vec(r, [t[0][3], t[1][3], t[2][3]]);
                (
                    mat_mul(r, rotation_of(i)),
                    [
                        origin[0] + step[0],
                        origin[1] + step[1],
                        origin[2] + step[2],
                    ],
                )
            },
        )
    };
    let (pre, mount_to_pan) = walk(&chain[..=pan_at]);
    let (between, pan_to_tilt) = walk(&chain[pan_at + 1..=tilt_at]);
    let (after, tilt_to_lens) = walk(&chain[tilt_at + 1..]);

    // Between the joints only a yaw is allowed: the tilt axis must still
    // be the yoke's X after the pan joint, i.e. `between` keeps Z. The
    // tolerance suits a file carrying six digits of a cosine.
    const ALIGNED: f64 = 1e-4;
    if (between[0][2].abs() + between[1][2].abs() + (between[2][2] - 1.0).abs()) > ALIGNED {
        return Err("the tilt axis is not perpendicular to the pan axis".to_string());
    }
    let pan_offset = between[1][0].atan2(between[0][0]).to_degrees();
    // After the tilt joint the beam must rest in the head's Y–Z plane, so
    // tilting sweeps it through straight down.
    let rest = [-after[0][2], -after[1][2], -after[2][2]];
    if rest[0].abs() > ALIGNED {
        return Err("the beam does not lie in the tilt plane".to_string());
    }
    let tilt_offset = rest[1].atan2(-rest[2]).to_degrees();
    Ok(AimCalibration {
        pre,
        pan_offset,
        tilt_offset,
        mount_to_pan,
        pan_to_tilt,
        tilt_to_lens,
    })
}

/// The nodes from the root down to the first beam's node, in that order;
/// `None` for a rig with no beam or a broken parent link.
fn beam_chain(rig: &RigModel) -> Option<Vec<usize>> {
    let beam = rig.beams.first()?;
    let mut chain = Vec::new();
    let mut at = Some(beam.node);
    while let Some(index) = at {
        chain.push(index);
        at = rig.nodes.get(index)?.parent;
    }
    chain.reverse();
    Some(chain)
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

    const YOKE_PLAIN: &str =
        r#"<Axis Name="Yoke" Model="Yoke" Position="{1,0,0,0}{0,1,0,0}{0,0,1,-0.1}{0,0,0,1}">"#;
    /// The MagicDot SX's yoke: yawed 90° about Z in the file.
    const YOKE_YAWED: &str =
        r#"<Axis Name="Yoke" Model="Yoke" Position="{0,1,0,0}{-1,0,0,0}{0,0,1,-0.1}{0,0,0,1}">"#;
    const LENS_PLAIN: &str = r#"Position="{1,0,0,0}{0,1,0,0}{0,0,1,-0.06}{0,0,0,1}"/>"#;
    /// A lens pitched 30° about X in the head: cos30 = 0.866, sin30 = 0.5.
    const LENS_PITCHED: &str =
        r#"Position="{1,0,0,0}{0,0.866025,-0.5,0}{0,0.5,0.866025,-0.06}{0,0,0,1}"/>"#;
    /// A lens yawed about Y: its beam leaves the tilt plane.
    const LENS_SKEWED: &str =
        r#"Position="{0.866025,0,0.5,0}{0,1,0,0}{-0.5,0,0.866025,-0.06}{0,0,0,1}"/>"#;

    fn mover_rig(yoke: &str, lens: &str) -> RigModel {
        let xml = SYNTHETIC_DESCRIPTION
            .replace(YOKE_PLAIN, yoke)
            .replace(LENS_PLAIN, lens);
        assert!(
            xml.contains(yoke) && xml.contains(lens),
            "fixture text changed"
        );
        let description = parse_description(&xml).unwrap();
        distill_rig(&description, "Mover 16bit", &HashSet::new()).unwrap()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-6
    }

    /// How far a ray from `from` along unit `dir` passes from `target`.
    fn ray_miss(from: [f64; 3], dir: [f64; 3], target: [f64; 3]) -> f64 {
        let d = [
            target[0] - from[0],
            target[1] - from[1],
            target[2] - from[2],
        ];
        let along = d[0] * dir[0] + d[1] * dir[1] + d[2] * dir[2];
        let off = [
            d[0] - along * dir[0],
            d[1] - along * dir[1],
            d[2] - along * dir[2],
        ];
        (off[0] * off[0] + off[1] * off[1] + off[2] * off[2]).sqrt()
    }

    /// A rig whose joints are the mounting axes calibrates to the
    /// identity; a yawed yoke turns up in `pre`; a pitched lens in the
    /// tilt offset; a lens that leaves the tilt plane is refused.
    #[test]
    fn calibration_reads_the_joints_from_the_geometry() {
        let plain = aim_calibration(&mover_rig(YOKE_PLAIN, LENS_PLAIN)).unwrap();
        assert!(plain.frame_is_identity(), "{plain:?}");
        // The lens sits 0.1 + 0.25 + 0.06 m below the mount, in three steps.
        assert_eq!(plain.mount_to_pan, [0.0, 0.0, -0.1]);
        assert_eq!(plain.pan_to_tilt, [0.0, 0.0, -0.25]);
        assert_eq!(plain.tilt_to_lens, [0.0, 0.0, -0.06]);

        let yawed = aim_calibration(&mover_rig(YOKE_YAWED, LENS_PLAIN)).unwrap();
        // Rz(−90°): local x is the parent's −y.
        assert!(
            close(yawed.pre[0][1], 1.0) && close(yawed.pre[1][0], -1.0),
            "{yawed:?}"
        );
        assert!(close(yawed.pan_offset, 0.0) && close(yawed.tilt_offset, 0.0));

        let pitched = aim_calibration(&mover_rig(YOKE_PLAIN, LENS_PITCHED)).unwrap();
        // The matrix above carries six digits of cos 30°.
        assert!((pitched.tilt_offset - 30.0).abs() < 1e-3, "{pitched:?}");
        assert!(pitched.pre == crate::lighting::effects::AimCalibration::IDENTITY.pre);

        let err = aim_calibration(&mover_rig(YOKE_PLAIN, LENS_SKEWED)).unwrap_err();
        assert!(err.contains("tilt plane"), "{err}");
    }

    /// Whatever the geometry, the calibrated direction is what the joints
    /// produce, and the calibrated aim sends the joints at the target.
    #[test]
    fn calibrated_aim_and_kinematics_agree_for_every_geometry() {
        use crate::lighting::effects::Pose;
        for (yoke, lens) in [
            (YOKE_PLAIN, LENS_PLAIN),
            (YOKE_YAWED, LENS_PLAIN),
            (YOKE_PLAIN, LENS_PITCHED),
            (YOKE_YAWED, LENS_PITCHED),
        ] {
            let rig = mover_rig(yoke, lens);
            let calibration = aim_calibration(&rig).unwrap();
            for pan in (-270..=270).step_by(30) {
                for tilt in (-135..=135).step_by(15) {
                    let pose = Pose {
                        pan: f64::from(pan),
                        tilt: f64::from(tilt),
                    };
                    let expected = calibration.direction([0.0; 3], pose);
                    let joints = beam_direction(&rig, pose.pan, pose.tilt).unwrap();
                    for i in 0..3 {
                        assert!(
                            (expected[i] - joints[i]).abs() < 1e-9,
                            "{yoke} {lens} {pose:?}: {expected:?} vs {joints:?}"
                        );
                    }
                }
            }
            // The lens is where the joints put it, and the beam from it
            // passes through the target.
            let rotation: [f64; 3] = [20.0, -10.0, 135.0];
            let targets: [[f64; 3]; 3] = [[1.0, 2.0, 0.0], [-3.0, 0.5, 1.0], [0.2, -1.0, 6.0]];
            for target in targets {
                let position: [f64; 3] = [0.0, 1.0, 4.0];
                for pose in calibration.aim_solutions(position, rotation, target) {
                    let (at, joints) = beam_ray(&rig, pose.pan, pose.tilt).unwrap();
                    let expected_at = calibration.origin(rotation, pose);
                    let at = crate::lighting::effects::out_of_frame(rotation, at);
                    for i in 0..3 {
                        assert!(
                            (at[i] - expected_at[i]).abs() < 1e-9,
                            "{yoke} {lens}: lens at {at:?}, calibration says {expected_at:?}"
                        );
                    }
                    let stage = crate::lighting::effects::out_of_frame(rotation, joints);
                    let miss = ray_miss(
                        [
                            position[0] + at[0],
                            position[1] + at[1],
                            position[2] + at[2],
                        ],
                        stage,
                        target,
                    );
                    assert!(
                        miss < 1e-6,
                        "{yoke} {lens} {target:?} {pose:?}: beam misses by {miss} m"
                    );
                }
            }
        }
    }
}
