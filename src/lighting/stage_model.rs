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

//! What the 3D view draws: a fixture type's rig and a venue's scenery, as
//! the asset store writes them -- light, so the web UI's WebAssembly module
//! compiles them as they are.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A 4×4 transform as this crate uses it: `M · v` takes a point from the
/// node's frame into its parent's, with the rotation's columns the node's
/// axes and the translation in the fourth column, meters. Not quite as
/// GDTF writes it: the GDTF description's `parse_matrix` reads the file's
/// rows as Blender DMX does.
#[cfg_attr(feature = "tsify", tsify_next::declare)]
pub type Matrix4 = [[f64; 4]; 4];

/// A fixture type's rig, as the asset cache stores it.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
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
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
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
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
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
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "snake_case")]
pub enum RigShape {
    /// A mesh in the asset cache, by path relative to the rig file, with
    /// the Model's declared Length, Width, Height in meters (x, y, z). The
    /// spec has the mesh "explicitly scaled to this dimension", per axis,
    /// whatever size the file itself has; a size of 0 is the spec's
    /// default, "not stated", and the mesh is drawn as it is.
    Model { file: String, size: [f64; 3] },
    /// A GDTF primitive (`Cube`, `Cylinder`, `Sphere`, `Base`, `Yoke`,
    /// `Head`, `Pigtail`, ...) of the given size in meters (x, y, z).
    Primitive { kind: String, size: [f64; 3] },
    /// Nothing to draw: a grouping node.
    Empty,
}

/// A light source.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
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

/// A venue's scenery as the store writes it (design §16.3): every scene
/// object with its transform in stage space and the meshes it draws.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneryModel {
    pub version: u32,
    /// The objects, in document order.
    pub objects: Vec<SceneryObject>,
    /// Mesh files by extension: how much of the scenery each format holds,
    /// drawn or not.
    pub formats: BTreeMap<String, usize>,
    /// What was skipped and why.
    #[serde(default)]
    pub warnings: Vec<String>,
}

/// One piece of scenery.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneryObject {
    pub name: String,
    /// `SceneObject`, `Truss`, `Support`, ...
    pub kind: String,
    pub layer: String,
    /// Row-major 4×4 in stage space: the MVR basis (scale and all) with
    /// the translation in meters, re-origined like the fixtures.
    pub transform: Matrix4,
    /// Meshes the store holds, by path relative to the scenery file.
    pub meshes: Vec<SceneryMesh>,
    /// Mesh files the 3D view cannot draw (`.3ds` and the like), by name.
    pub skipped: Vec<String>,
}

/// A drawable mesh of an object.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SceneryMesh {
    pub file: String,
    /// Row-major 4×4 relative to the object, translation in meters.
    pub transform: Matrix4,
}
