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

//! What an MVR import plans and reports, how the file looks from above, and
//! what an export holds: light, so the web UI's WebAssembly module compiles
//! it as it is. The import and export are `import`'s and `export`'s.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::geometry::Vec3;

/// Where exports land, relative to the project.
pub const EXPORT_DIR: &str = "lighting/export";

/// A fixture-type (or venue) name reduced to a safe filename stem.
pub fn fixture_filename_stem(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    let out = out.trim_end_matches('_').to_string();
    if out.is_empty() {
        "fixture".to_string()
    } else {
        out
    }
}

/// An export's file name: `output` as given (trimmed), or the venue's stem
/// with `.mvr`; an error unless it is a bare `.mvr` file name, with no
/// directories and nothing hidden.
pub fn export_file_name(output: Option<&str>, venue: &str) -> Result<String, String> {
    let output = match output {
        Some(output) => output.trim().to_string(),
        None => format!("{}.mvr", fixture_filename_stem(venue)),
    };
    let bare = std::path::Path::new(&output).file_name() == Some(std::ffi::OsStr::new(&output));
    if !bare || output.starts_with('.') || output.contains('\\') || output.contains('\0') {
        return Err(format!(
            "output \"{output}\" must be a bare file name; exports land in {EXPORT_DIR}/"
        ));
    }
    if !crate::songs::has_extension(std::path::Path::new(&output), &["mvr"]) || output.len() <= 4 {
        return Err(format!("output \"{output}\" must end in .mvr"));
    }
    Ok(output)
}

/// The hand edits a merge should keep instead of overwriting with the MVR's
/// values. A field that is not a listed overwrite is ignored.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Clone, Debug, Default, Deserialize)]
pub struct MvrKeep {
    /// Fixture name → the fields to keep: `position`, `rotation`, `patch`,
    /// `mode`, `type`.
    #[serde(default)]
    pub fixtures: BTreeMap<String, Vec<String>>,
    /// Focus points whose position to keep.
    #[serde(default)]
    pub focus_points: Vec<String>,
}

/// A field of the venue that the merge would replace with the MVR's value,
/// though it was edited by hand.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Debug, Serialize)]
pub struct HandEdit {
    /// `position`, `rotation`, `patch`, `mode` or `type`.
    pub field: String,
    /// The venue file's value, as the file spells it.
    pub mine: String,
    /// The MVR's value.
    pub mvr: String,
}

/// Everything an import would do, resolved but not written.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Debug, Serialize)]
pub struct MvrPlan {
    /// The venue's name.
    pub venue_name: String,
    /// The venue file, project-relative.
    pub venue_file: String,
    /// Where the MVR lands, project-relative.
    pub archive: String,
    /// Whether an MVR-seeded venue of this name already exists and is being
    /// merged rather than created.
    pub merge: bool,
    /// The stage origin in MVR space, meters.
    pub origin: Vec3,
    /// The fixture types the patch references.
    pub fixture_types: Vec<PlannedFixtureType>,
    /// The patched fixtures, in the order the venue file will list them.
    pub fixtures: Vec<PlannedFixture>,
    /// Fixtures the venue itself removed since the last import (present in
    /// the previous MVR, absent from this one). Dropped, with their tags.
    pub removed_fixtures: Vec<RemovedFixture>,
    /// Fixtures in the existing venue the MVR never knew about — hand
    /// additions, kept as they are.
    pub kept_fixtures: Vec<String>,
    /// The scene's focus points.
    pub focus_points: Vec<PlannedFocusPoint>,
    /// Focus points the previous MVR had that this one does not. Dropped.
    pub removed_focus_points: Vec<String>,
    /// Focus points of the existing venue the MVR never knew about. Kept.
    pub kept_focus_points: Vec<String>,
    /// Scenery objects in the scene (trusses, decks, screens), which the
    /// 3D view draws from the MVR's meshes.
    pub scenery_objects: usize,
    /// Scenery meshes in formats the 3D view does not draw (`.3ds`).
    pub scenery_meshes_undrawn: usize,
    /// What was skipped, approximated or guessed.
    pub warnings: Vec<String>,
}

/// A fixture type the patch references: an embedded GDTF, which the
/// library makes a fixture (design §22).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Debug, Serialize)]
pub struct PlannedFixtureType {
    /// The fixture type's name.
    pub name: String,
    /// The GDTF archive, project-relative.
    pub archive: String,
    /// Every mode this file patches the type's fixtures in, sorted; each
    /// fixture's line names its own.
    pub modes: Vec<String>,
    /// Whether the archive is already in the library (so nothing is written
    /// for it).
    pub existing: bool,
    /// mtrack's record of the type, project-relative, when one is written:
    /// only to pin a name another fixture already had.
    pub fixture_file: Option<String>,
}

/// A patched fixture as the venue file will state it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Debug, Serialize)]
pub struct PlannedFixture {
    /// The fixture's name.
    pub name: String,
    /// The MVR layer it sits on.
    pub layer: String,
    /// The resolved fixture type; `None` when the fixture is a TODO.
    pub fixture_type: Option<String>,
    /// The mode its venue line names, as the GDTF spells it; `None` only
    /// for a TODO (or a kept hand edit that has none).
    pub mode: Option<String>,
    /// The embedded GDTF and the mode the MVR patches it in, as the GDTF
    /// spells it — what a merge compares the venue's fixture against.
    #[serde(skip)]
    pub(crate) gdtf: Option<TypeKey>,
    /// Universe and address; `None` when the MVR patched nothing.
    pub patch: Option<(u16, u16)>,
    /// Stage position, meters.
    pub position: Option<Vec3>,
    /// Mounting rotation, degrees.
    pub rotation: Option<Vec3>,
    /// The beam angle the venue states, in degrees. The MVR has no place for
    /// it, so it is never the import's to set: a first import leaves it
    /// `None` and a merge carries the venue's own value over, always.
    #[serde(skip)]
    pub(crate) beam_angle: Option<f64>,
    /// Tags — empty on a fresh seed, the venue's own on a merge.
    pub tags: Vec<String>,
    /// Why this fixture could not be resolved, when it could not.
    pub todo: Option<String>,
    /// On a merge, what changed against the existing venue.
    pub change: Option<String>,
    /// On a merge, hand-edited fields this import overwrites with the MVR's
    /// values (empty when none, or when the request keeps them).
    pub overwrites: Vec<HandEdit>,
    /// Hand-edited fields this import keeps, as the request asked.
    pub kept_edits: Vec<String>,
}

/// A fixture the venue removed, with the tags that go with it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Debug, Serialize)]
pub struct RemovedFixture {
    pub name: String,
    pub tags: Vec<String>,
}

/// A focus point as the venue file will state it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Debug, Serialize)]
pub struct PlannedFocusPoint {
    pub name: String,
    /// Stage position, meters.
    pub point: Vec3,
    /// On a merge, what changed against the existing venue.
    pub change: Option<String>,
    /// On a merge, the venue's hand-moved position this import overwrites
    /// (empty when none, or when the request keeps it).
    pub overwrites: Vec<HandEdit>,
    /// Whether the request keeps a hand-moved position.
    pub kept_edit: bool,
}

/// What an import did.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Debug, Serialize)]
pub struct MvrImport {
    /// The plan that was carried out.
    #[serde(flatten)]
    pub plan: MvrPlan,
    /// Every file written, project-relative.
    pub written: Vec<String>,
    /// GDTF distillation warnings per fixture type, for the modes the venue
    /// uses ("mode: warning").
    pub distillation_warnings: BTreeMap<String, Vec<String>>,
}

/// An axis-aligned box in scene millimeters.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct BoundsMm {
    pub min: [f64; 3],
    pub max: [f64; 3],
}

/// A fixture as the file places it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Debug, Serialize)]
pub struct ViewFixture {
    pub name: String,
    pub layer: String,
    /// The fixture's position in the file's millimeters, when it has one.
    pub position_mm: Option<[f64; 3]>,
}

/// A focus point as the file places it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Debug, Serialize)]
pub struct ViewFocusPoint {
    pub name: String,
    pub position_mm: Option<[f64; 3]>,
}

/// A piece of scenery, with its bounds when they could be read.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
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
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Debug, Serialize)]
pub struct SceneView {
    pub fixtures: Vec<ViewFixture>,
    pub focus_points: Vec<ViewFocusPoint>,
    pub scenery: Vec<ViewScenery>,
    /// The stage floor's bounds, when the scenery carries one.
    pub deck_mm: Option<BoundsMm>,
}

/// What an export produced, for reporting.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Debug, Serialize)]
pub struct MvrExport {
    /// The venue's name.
    pub venue: String,
    /// The `.mvr` written, project-relative.
    pub output: String,
    /// Fixtures in the scene.
    pub fixtures: usize,
    /// Focus points in the scene.
    pub focus_points: usize,
    /// GDTF archives embedded from the library, by entry name.
    pub embedded_gdtfs: Vec<String>,
    /// GDTF archives generated for native fixture types: entry name →
    /// fixture type.
    pub generated_gdtfs: BTreeMap<String, String>,
    /// Fixtures with a position (so with a transform in the scene).
    pub positioned_fixtures: usize,
    /// The fixed fixtures (types with no pan or tilt) that have a position,
    /// each with the focus point it is aimed at, if any. A pre-viz tool aims
    /// the unlinked ones at the origin.
    pub fixed_fixtures: Vec<FixedFixture>,
    /// What was skipped or approximated.
    pub warnings: Vec<String>,
}

/// A fixed fixture and the focus point the export links it to.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Clone, Debug, Serialize)]
pub struct FixedFixture {
    pub name: String,
    /// The focus point its rest beam passes within a degree of, if any.
    pub focus: Option<String>,
}

/// The key a fixture type is unique by: which embedded archive, which mode.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct TypeKey {
    pub(crate) entry: String,
    pub(crate) mode: String,
}
