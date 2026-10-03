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

//! MVR import: a venue's patch, seeded into a `.venue` file you then own.
//!
//! An MVR is the venue's *rig facts* — what hangs where, patched how, which
//! GDTF and mode. It is not the whole venue: tags, focus names and the
//! stage origin are the band's, so the import seeds a file rather than
//! resolving one at runtime (venue-exchange design §4.2, §6). Every
//! embedded GDTF the patch references goes into the library on the way,
//! through the same placement `import-gdtf` uses — one fixture type per
//! archive, not per mode (design §21; an archive already there with the
//! same bytes is simply used) — and every seeded fixture line states its
//! own mode.
//!
//! Two entry points share one planner: [`inspect_mvr`] resolves everything
//! and writes nothing, so the CLI's bare form and the MCP tool can show
//! what an import *would* do; [`import_mvr`] plans and then writes. All
//! refusals happen in the plan — a refused import leaves the project
//! untouched.
//!
//! **Re-import.** A venue that records `imported from mvr(...)` merges a
//! revised MVR instead of being replaced: rig facts come from the new
//! file, tags and focus names stay yours, and the copy of the previous MVR
//! in `lighting/library/` says which of your fixtures the venue itself
//! removed (dropped, reported) versus which you added by hand (kept). A
//! patched fixture is never silently dropped: one whose GDTF is missing or
//! whose mode cannot be matched becomes a `TODO` comment in the venue
//! file, with everything the MVR knew about it.
//!
//! A merge is a **textual patch** of the venue file (see
//! [`crate::lighting::venue_patch`]), so its comments, `# TODO` lines and
//! header survive. Fields the venue's owner edited by hand — a moved
//! fixture, a corrected rotation, a re-patched address or mode, a nudged focus
//! point — are listed in the plan (`overwrites`) rather than replaced
//! silently, and [`MvrKeep`] names the ones to keep. A field counts as
//! hand-edited when the venue differs from the new MVR *and* from what the
//! previous MVR would have written; when there is no previous MVR to tell
//! by, or (for the fixture type) the type cannot be traced, any difference
//! counts.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::error::Error;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::{
    create_dir, fixture_filename_stem, place_in_library, write, write_library_entry, LibraryEntry,
};
use crate::lighting::gdtf;
use crate::lighting::mvr::{self, MvrFixture, Scene};
use crate::lighting::parser::parse_venues;
use crate::lighting::types::{fmt_vec3, Fixture, Vec3, Venue, VenueSource};
use crate::lighting::venue_patch::{patch_venue_with, PatchNotes};

/// The hand edits a merge should keep instead of overwriting with the MVR's
/// values. A field that is not a listed overwrite is ignored.
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

impl MvrKeep {
    fn keeps(&self, fixture: &str, field: &str) -> bool {
        self.fixtures
            .get(fixture)
            .is_some_and(|fields| fields.iter().any(|f| f == field))
    }
}

/// A field of the venue that the merge would replace with the MVR's value,
/// though it was edited by hand.
#[derive(Debug, Serialize)]
pub struct HandEdit {
    /// `position`, `rotation`, `patch`, `mode` or `type`.
    pub field: String,
    /// The venue file's value, as the file spells it.
    pub mine: String,
    /// The MVR's value.
    pub mvr: String,
}

/// The choices an MVR import takes from the user.
#[derive(Clone, Debug)]
pub struct MvrImportOptions {
    /// The venue's name; defaults to the archive's file stem.
    pub name: Option<String>,
    /// The MVR-space point, in millimeters, that becomes the stage origin
    /// (downstage-center on the deck). `None` keeps a merged venue's
    /// recorded origin, and is (0, 0, 0) for a fresh seed.
    pub origin_mm: Option<Vec3>,
    /// Fixture types directory, relative to the project.
    pub fixture_types_dir: String,
    /// Venues directory, relative to the project.
    pub venues_dir: String,
    /// Hand edits a merge keeps rather than overwrites. Empty means every
    /// field takes the MVR's value.
    pub keep: MvrKeep,
}

impl Default for MvrImportOptions {
    fn default() -> Self {
        MvrImportOptions {
            name: None,
            origin_mm: None,
            keep: MvrKeep::default(),
            fixture_types_dir: crate::config::lighting::DEFAULT_FIXTURE_TYPES_DIR.to_string(),
            venues_dir: crate::config::lighting::DEFAULT_VENUES_DIR.to_string(),
        }
    }
}

/// Everything an import would do, resolved but not written.
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
    gdtf: Option<TypeKey>,
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
    beam_angle: Option<f64>,
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
#[derive(Debug, Serialize)]
pub struct RemovedFixture {
    pub name: String,
    pub tags: Vec<String>,
}

/// A focus point as the venue file will state it.
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

/// A plan plus the bytes it needs to carry out.
struct Planned {
    plan: MvrPlan,
    /// Embedded GDTFs to place in the library (a new archive, a record
    /// pinning a name).
    gdtf_writes: Vec<GdtfWrite>,
    /// Distillation warnings per type, for the modes the venue uses.
    distillation_warnings: BTreeMap<String, Vec<String>>,
    /// Whether the MVR itself needs writing (new, or changed on re-import).
    write_archive: bool,
    /// The venue file's text.
    venue_text: String,
    venue_path: PathBuf,
}

struct GdtfWrite {
    entry: LibraryEntry,
    bytes: Vec<u8>,
}

/// Resolves an MVR against a project and reports what an import would do,
/// writing nothing.
pub fn inspect_mvr(
    mvr_path: &Path,
    options: &MvrImportOptions,
    project: &Path,
) -> Result<MvrPlan, Box<dyn Error>> {
    let (bytes, file_name) = read_archive(mvr_path)?;
    inspect_mvr_bytes(&bytes, &file_name, options, project)
}

/// [`inspect_mvr`] over in-memory bytes.
pub fn inspect_mvr_bytes(
    bytes: &[u8],
    archive_file_name: &str,
    options: &MvrImportOptions,
    project: &Path,
) -> Result<MvrPlan, Box<dyn Error>> {
    plan(bytes, archive_file_name, options, project).map(|planned| planned.plan)
}

/// Imports an MVR: the archive and its embedded GDTFs into
/// `lighting/library/`, a `.fixture` per referenced type, and a seeded (or
/// merged) `.venue`. All validation runs before anything is written.
pub fn import_mvr(
    mvr_path: &Path,
    options: &MvrImportOptions,
    project: &Path,
) -> Result<MvrImport, Box<dyn Error>> {
    let (bytes, file_name) = read_archive(mvr_path)?;
    import_mvr_bytes(&bytes, &file_name, options, project)
}

/// [`import_mvr`] over in-memory bytes — the shape uploads arrive in.
pub fn import_mvr_bytes(
    bytes: &[u8],
    archive_file_name: &str,
    options: &MvrImportOptions,
    project: &Path,
) -> Result<MvrImport, Box<dyn Error>> {
    let planned = plan(bytes, archive_file_name, options, project)?;
    let mut written = Vec::new();

    let library_dir = project.join("lighting/library");
    create_dir(&library_dir)?;
    if planned.write_archive {
        write(&library_dir.join(archive_file_name), bytes)?;
        written.push(planned.plan.archive.clone());
    }

    // Each embedded GDTF lands as `import-gdtf` would put it: the archive
    // in the library (the fixture), a record only to pin a name.
    for gdtf_write in &planned.gdtf_writes {
        let description = gdtf::parse_archive(&gdtf_write.bytes)?;
        write_library_entry(project, &gdtf_write.entry, &gdtf_write.bytes, &description)?;
        if !gdtf_write.entry.existing {
            written.push(gdtf_write.entry.archive.clone());
        }
        if let Some(record) = &gdtf_write.entry.record {
            written.push(record.clone());
        }
    }
    let distillation_warnings = planned.distillation_warnings;

    create_dir(planned.venue_path.parent().unwrap_or(project))?;
    write(&planned.venue_path, planned.venue_text.as_bytes())?;
    written.push(planned.plan.venue_file.clone());

    Ok(MvrImport {
        plan: planned.plan,
        written,
        distillation_warnings,
    })
}

fn read_archive(mvr_path: &Path) -> Result<(Vec<u8>, String), Box<dyn Error>> {
    let bytes = std::fs::read(mvr_path)
        .map_err(|e| format!("cannot read MVR archive {}: {e}", mvr_path.display()))?;
    let file_name = mvr_path
        .file_name()
        .ok_or("MVR path has no file name")?
        .to_string_lossy()
        .into_owned();
    Ok((bytes, file_name))
}

/// The key a fixture type is unique by: which embedded archive, which mode.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct TypeKey {
    entry: String,
    mode: String,
}

/// An embedded GDTF, parsed once however many fixtures reference it.
struct Embedded {
    file_name: String,
    bytes: Vec<u8>,
    description: Result<gdtf::Description, String>,
}

fn plan(
    bytes: &[u8],
    archive_file_name: &str,
    options: &MvrImportOptions,
    project: &Path,
) -> Result<Planned, Box<dyn Error>> {
    if Path::new(archive_file_name).file_name() != Some(std::ffi::OsStr::new(archive_file_name)) {
        return Err(
            format!("archive file name \"{archive_file_name}\" is not a bare file name").into(),
        );
    }
    // The name lands inside a quoted DSL string; the grammar has no escapes.
    if archive_file_name
        .chars()
        .any(|c| c == '"' || c.is_control())
    {
        return Err(format!(
            "archive file name {archive_file_name:?} contains a quote or control character; \
             rename the file"
        )
        .into());
    }
    let scene = mvr::parse_archive(bytes)?;
    let mut warnings = scene.warnings.clone();
    let scenery_objects = scene.objects.len();
    let mut undrawn: BTreeMap<String, usize> = BTreeMap::new();
    for mesh in scene.objects.iter().flat_map(|o| &o.meshes) {
        let ext = mesh
            .file
            .rsplit_once('.')
            .map(|(_, e)| e.to_ascii_lowercase())
            .unwrap_or_default();
        if ext != "glb" {
            *undrawn.entry(ext).or_default() += 1;
        }
    }
    let scenery_meshes_undrawn: usize = undrawn.values().sum();
    if scenery_meshes_undrawn > 0 {
        warnings.push(format!(
            "{scenery_meshes_undrawn} scenery mesh(es) are in formats the 3D view does not draw ({}); \
             glTF (.glb) scenery is drawn, and fixtures are drawn regardless",
            undrawn
                .iter()
                .map(|(ext, n)| format!("{n} .{ext}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }

    let venue_name = options.name.clone().unwrap_or_else(|| {
        Path::new(archive_file_name)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "venue".to_string())
    });
    if venue_name.trim().is_empty() {
        return Err("venue name is empty".into());
    }
    if venue_name.chars().any(|c| c == '"' || c.is_control()) {
        return Err(format!(
            "venue name {venue_name:?} contains a quote or control character; choose another"
        )
        .into());
    }
    let library_rel = format!("lighting/library/{archive_file_name}");
    let library_path = project.join(&library_rel);

    // --- The existing venue, if this is a re-import.
    let existing = find_existing_venue(project, &options.venues_dir, &venue_name)?;
    let (venue_path, venue_file) = match &existing {
        Some((path, _)) => (path.clone(), relative_display(project, path)),
        None => {
            let file = format!(
                "{}/{}.venue",
                options.venues_dir.trim_end_matches('/'),
                fixture_filename_stem(&venue_name)
            );
            (project.join(&file), file)
        }
    };
    let existing_venue = match &existing {
        Some((path, venue)) => {
            let Some(source) = venue.source() else {
                return Err(format!(
                    "{} holds a hand-written venue \"{venue_name}\" (no `imported from \
                     mvr(...)`); import under a different --name, or remove the file to \
                     seed it from the MVR",
                    path.display()
                )
                .into());
            };
            if source.mvr != library_rel {
                return Err(format!(
                    "{} was imported from {}, not {library_rel}; a venue merges only \
                     revisions of the MVR it came from — import under a different --name",
                    path.display(),
                    source.mvr
                )
                .into());
            }
            Some(venue)
        }
        None => None,
    };

    // --- The library copy of the MVR: new, unchanged, or a revision.
    let mut write_archive = true;
    let mut previous_scene: Option<Scene> = None;
    if library_path.exists() {
        let previous = std::fs::read(&library_path)
            .map_err(|e| format!("cannot read existing {}: {e}", library_path.display()))?;
        if previous == bytes {
            write_archive = false;
        } else if existing_venue.is_none() {
            return Err(format!(
                "{library_rel} already exists with different content and no venue \
                 \"{venue_name}\" was imported from it; rename the source file (the \
                 library filename follows it) or import under the venue's own name",
            )
            .into());
        }
        match mvr::parse_archive(&previous) {
            Ok(scene) => previous_scene = Some(scene),
            Err(e) => warnings.push(format!(
                "the previous {library_rel} does not parse ({e}); fixtures missing from \
                 the new MVR are kept rather than dropped"
            )),
        }
    }

    // --- Embedded GDTFs, parsed once each.
    let entries = mvr::list_gdtf_entries(bytes)?;
    let mut embedded: HashMap<String, Embedded> = HashMap::new();
    let mut resolved: Vec<(usize, Option<TypeKey>, Option<String>)> = Vec::new();
    for (index, fixture) in scene.fixtures.iter().enumerate() {
        let Some(spec) = fixture.gdtf_spec.as_deref() else {
            resolved.push((index, None, Some("no GDTF reference".to_string())));
            continue;
        };
        let entry = match mvr::resolve_gdtf_entry(&entries, spec) {
            Ok(Some(entry)) => entry,
            Ok(None) => {
                resolved.push((
                    index,
                    None,
                    Some(format!("GDTF \"{spec}\" is not embedded in the MVR")),
                ));
                continue;
            }
            Err(e) => {
                resolved.push((index, None, Some(e.to_string())));
                continue;
            }
        };
        let entry = entry.to_string();
        if !embedded.contains_key(&entry) {
            let gdtf_bytes = mvr::read_gdtf_entry(bytes, &entry)?;
            let description = gdtf::parse_archive(&gdtf_bytes).map_err(|e| e.to_string());
            let mut file_name = entry.rsplit('/').next().unwrap_or(&entry).to_string();
            if !file_name.to_ascii_lowercase().ends_with(".gdtf") {
                file_name.push_str(".gdtf");
            }
            embedded.insert(
                entry.clone(),
                Embedded {
                    file_name,
                    bytes: gdtf_bytes,
                    description,
                },
            );
        }
        let description = match &embedded[&entry].description {
            Ok(description) => description,
            Err(e) => {
                resolved.push((
                    index,
                    None,
                    Some(format!("embedded GDTF \"{entry}\" does not parse: {e}")),
                ));
                continue;
            }
        };
        let Some(requested) = fixture.gdtf_mode.as_deref() else {
            resolved.push((index, None, Some("no GDTF mode".to_string())));
            continue;
        };
        match gdtf::match_mode(description, requested) {
            // The mode lands in a quoted DSL string, which has no escapes.
            Ok(matched) if matched.name.chars().any(|c| c == '"' || c.is_control()) => resolved
                .push((
                    index,
                    None,
                    Some(format!(
                        "mode {:?} contains a quote or control character, which a venue file \
                         cannot write",
                        matched.name
                    )),
                )),
            Ok(matched) => {
                if matched.normalized {
                    let note = format!(
                        "mode \"{requested}\" of {entry} matched \"{}\" after normalization",
                        matched.name
                    );
                    if !warnings.contains(&note) {
                        warnings.push(note);
                    }
                }
                resolved.push((
                    index,
                    Some(TypeKey {
                        entry,
                        mode: matched.name,
                    }),
                    None,
                ));
            }
            Err(e) => resolved.push((index, None, Some(e.to_string()))),
        }
    }

    // --- Fixture type names: one type per embedded GDTF (the type is the
    // whole archive, design §21), named after the GDTF's own name and
    // disambiguated only on collision.
    let mut keys: Vec<TypeKey> = resolved.iter().filter_map(|(_, k, _)| k.clone()).collect();
    keys.sort();
    keys.dedup();
    let mut type_entries: Vec<String> = keys.iter().map(|k| k.entry.clone()).collect();
    type_entries.dedup();

    // A mode that matched by name can still refuse to distill (pixel and
    // matrix modes). That is a TODO for its fixtures, found now rather than
    // by the loader, for a mode a fixture line names.
    let mut entry_warnings: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for key in &keys {
        let Ok(description) = &embedded[&key.entry].description else {
            continue;
        };
        match gdtf::distill(description, &key.mode, &key.entry) {
            Ok(distilled) => {
                let notes = entry_warnings.entry(key.entry.clone()).or_default();
                for warning in distilled.warnings {
                    notes.push(format!("{}: {warning}", key.mode));
                }
            }
            Err(e) => {
                let reason = format!("mode \"{}\" does not distill: {e}", key.mode);
                for (_, resolved_key, todo) in &mut resolved {
                    if resolved_key.as_ref() == Some(key) {
                        *resolved_key = None;
                        *todo = Some(reason.clone());
                    }
                }
            }
        }
    }
    type_entries.retain(|entry| {
        resolved
            .iter()
            .any(|(_, k, _)| k.as_ref().is_some_and(|k| &k.entry == entry))
    });

    // --- One fixture type per embedded GDTF (design §22): the archive in the
    // library is the fixture. One already there (the same bytes) is the
    // fixture it already is; a new one is named from its GDTF, and a name
    // another fixture has is pinned with a record as `Name (stem)`.
    let fixture_dir = project.join(&options.fixture_types_dir);
    let mut claimed: HashSet<String> = HashSet::new();
    let mut type_names: HashMap<String, String> = HashMap::new();
    let mut fixture_types = Vec::new();
    let mut gdtf_writes = Vec::new();
    let mut distillation_warnings = BTreeMap::new();
    for entry in &type_entries {
        let item = &embedded[entry];
        let Ok(description) = &item.description else {
            continue;
        };
        let placed = place_in_library(
            project,
            &fixture_dir,
            &item.file_name,
            &item.bytes,
            description,
            None,
            &mut claimed,
        )?;
        let mut modes: Vec<String> = resolved
            .iter()
            .filter_map(|(_, k, _)| k.as_ref())
            .filter(|k| &k.entry == entry)
            .map(|k| k.mode.clone())
            .collect();
        modes.sort();
        modes.dedup();
        if let Some(from) = &placed.renamed_from {
            warnings.push(format!(
                "another fixture is already called \"{from}\"; {} is \"{}\"",
                item.file_name, placed.name
            ));
        }
        if let Some(notes) = entry_warnings.remove(entry) {
            if !notes.is_empty() {
                distillation_warnings.insert(placed.name.clone(), notes);
            }
        }
        type_names.insert(entry.clone(), placed.name.clone());
        fixture_types.push(PlannedFixtureType {
            name: placed.name.clone(),
            archive: placed.archive.clone(),
            modes,
            existing: placed.existing && placed.record.is_none(),
            fixture_file: placed.record.clone(),
        });
        if !placed.existing || placed.record.is_some() {
            gdtf_writes.push(GdtfWrite {
                entry: placed,
                bytes: item.bytes.clone(),
            });
        }
    }

    // --- Fixtures, in stage coordinates. A merge without an explicit origin
    // keeps the one the venue recorded, so re-importing an unmoved rig is
    // safe by default.
    let origin_mm = match options.origin_mm {
        Some(origin) => origin,
        None => existing_venue
            .and_then(|v| v.source())
            .map(|source| {
                [
                    source.origin[0] * 1000.0,
                    source.origin[1] * 1000.0,
                    source.origin[2] * 1000.0,
                ]
            })
            .unwrap_or([0.0; 3]),
    };
    let origin_m = [
        origin_mm[0] / 1000.0,
        origin_mm[1] / 1000.0,
        origin_mm[2] / 1000.0,
    ];
    let mut namer = Namer::new(&scene);
    let mut fixtures = Vec::new();
    for (index, key, todo) in &resolved {
        let source = &scene.fixtures[*index];
        let name = namer.name(source, *index, &mut warnings);
        let (position, rotation) = geometry(source, &origin_mm, &mut warnings);
        let patch = source.addresses.first().copied();
        if source.addresses.len() > 1 {
            warnings.push(format!(
                "fixture \"{name}\": {} DMX breaks; only the first ({}) is patched",
                source.addresses.len(),
                patch.map(|(u, a)| format!("{u}:{a}")).unwrap_or_default()
            ));
        }
        let mut todo = todo.clone();
        if todo.is_none() && patch.is_none() {
            todo = Some("no DMX address".to_string());
        }
        let fixture_type = key.as_ref().map(|k| type_names[&k.entry].clone());
        // Every line names its mode: a GDTF fixture has no default.
        let mode = key.as_ref().map(|key| key.mode.clone());
        fixtures.push(PlannedFixture {
            name,
            layer: source.layer.clone(),
            fixture_type,
            mode,
            gdtf: key.clone(),
            patch,
            position,
            rotation,
            beam_angle: None,
            tags: Vec::new(),
            todo,
            change: None,
            overwrites: Vec::new(),
            kept_edits: Vec::new(),
        });
    }
    let (numbered, ordinal_named) = (namer.numbered, namer.ordinal_named);
    if numbered > 0 {
        warnings.push(format!(
            "{numbered} fixtures shared a name with another; each took its console fixture \
             ID (\"Robe Spiider 12\") — rename freely, tags are what shows target"
        ));
    }
    if !ordinal_named.is_empty() {
        let shown: Vec<&str> = ordinal_named.iter().take(6).map(String::as_str).collect();
        let more = ordinal_named.len().saturating_sub(shown.len());
        warnings.push(format!(
            "{} fixtures shared a name (and a fixture ID, where the file had one) and took \
             an ordinal: {}{}",
            ordinal_named.len(),
            shown.join(", "),
            if more > 0 {
                format!(", and {more} more")
            } else {
                String::new()
            }
        ));
    }
    let mut focus_points: Vec<PlannedFocusPoint> = Vec::new();
    for focus in &scene.focus_points {
        let Some(matrix) = focus.matrix else {
            warnings.push(format!(
                "focus point \"{}\" has no position; skipped",
                focus.name
            ));
            continue;
        };
        let name = if focus.name.trim().is_empty() {
            format!("focus-{}", focus_points.len() + 1)
        } else {
            dsl_safe(&focus.name, &mut warnings)
        };
        if focus_points.iter().any(|f| f.name == name) {
            warnings.push(format!(
                "duplicate focus point \"{name}\"; later one skipped"
            ));
            continue;
        }
        focus_points.push(PlannedFocusPoint {
            name,
            point: to_stage(&matrix.o, &origin_mm),
            change: None,
            overwrites: Vec::new(),
            kept_edit: false,
        });
    }

    // --- Merge against the existing venue.
    let mut removed_fixtures = Vec::new();
    let mut kept_fixtures = Vec::new();
    let mut removed_focus_points = Vec::new();
    let mut kept_focus_points = Vec::new();
    let mut kept: Vec<Fixture> = Vec::new();
    let mut kept_focus: BTreeMap<String, Vec3> = BTreeMap::new();
    if let Some(existing) = existing_venue {
        if existing.source().map(|s| s.origin) != Some(origin_m) {
            warnings.push(format!(
                "origin changed from {} to {}; every position moves with it",
                fmt_vec3(&existing.source().map(|s| s.origin).unwrap_or_default()),
                fmt_vec3(&origin_m)
            ));
        }
        // What the previous import would have written, to tell a hand edit
        // from a change the MVR itself made.
        let previous_origin_mm = existing
            .source()
            .map(|s| {
                [
                    s.origin[0] * 1000.0,
                    s.origin[1] * 1000.0,
                    s.origin[2] * 1000.0,
                ]
            })
            .unwrap_or([0.0; 3]);
        let prior: Option<HashMap<String, Prior>> = previous_scene.as_ref().map(|prev| {
            let mut namer = Namer::new(prev);
            let mut scratch = Vec::new();
            prev.fixtures
                .iter()
                .enumerate()
                .map(|(index, f)| {
                    let name = namer.name(f, index, &mut scratch);
                    let (position, rotation) = geometry(f, &previous_origin_mm, &mut scratch);
                    let patch = f.addresses.first().copied();
                    (
                        name,
                        Prior {
                            position,
                            rotation,
                            patch,
                            mode: f.gdtf_mode.clone(),
                        },
                    )
                })
                .collect()
        });
        let prior_focus: Option<HashMap<String, Vec3>> = previous_scene.as_ref().map(|prev| {
            let mut scratch = Vec::new();
            prev.focus_points
                .iter()
                .filter_map(|f| {
                    let matrix = f.matrix?;
                    (!f.name.trim().is_empty()).then(|| {
                        (
                            dsl_safe(&f.name, &mut scratch),
                            to_stage(&matrix.o, &previous_origin_mm),
                        )
                    })
                })
                .collect()
        });
        for planned in &mut fixtures {
            match existing.fixtures().get(&planned.name) {
                Some(theirs) => {
                    let prior = prior.as_ref().and_then(|p| p.get(&planned.name));
                    let modes = compare_modes(theirs, planned, prior, &embedded);
                    planned.tags = theirs.tags().to_vec();
                    planned.beam_angle = theirs.beam_angle();
                    planned.change = Some(describe_change(theirs, planned, modes.as_ref()));
                    let edits = hand_edits(theirs, planned, prior, modes.as_ref());
                    for edit in edits {
                        if options.keep.keeps(&planned.name, &edit.field) {
                            match edit.field.as_str() {
                                "position" => planned.position = theirs.position(),
                                "rotation" => planned.rotation = theirs.rotation(),
                                "patch" => {
                                    planned.patch =
                                        Some((theirs.universe(), theirs.start_channel()))
                                }
                                "mode" => planned.mode = theirs.mode().map(str::to_string),
                                // The type and its mode go together: a mode
                                // is only a mode of its own type.
                                _ => {
                                    planned.fixture_type = Some(theirs.fixture_type().to_string());
                                    planned.mode = theirs.mode().map(str::to_string);
                                }
                            }
                            planned.kept_edits.push(edit.field);
                        } else {
                            planned.overwrites.push(edit);
                        }
                    }
                }
                None => planned.change = Some("added".to_string()),
            }
        }
        let previous_names: Option<Vec<&str>> = previous_scene
            .as_ref()
            .map(|s| s.fixtures.iter().map(|f| f.name.as_str()).collect());
        for theirs in existing.fixtures_by_patch() {
            if fixtures.iter().any(|f| f.name == theirs.name()) {
                continue;
            }
            let venue_removed_it = previous_names
                .as_ref()
                .is_some_and(|names| names.contains(&theirs.name()));
            if venue_removed_it {
                removed_fixtures.push(RemovedFixture {
                    name: theirs.name().to_string(),
                    tags: theirs.tags().to_vec(),
                });
            } else {
                kept_fixtures.push(theirs.name().to_string());
                kept.push(theirs.clone());
            }
        }
        let previous_focus: Vec<&str> = previous_scene
            .as_ref()
            .map(|s| s.focus_points.iter().map(|f| f.name.as_str()).collect())
            .unwrap_or_default();
        for planned in &mut focus_points {
            planned.change = Some(match existing.focus_points().get(&planned.name) {
                Some(point) if close(point, &planned.point) => "unchanged".to_string(),
                Some(point) => {
                    let hand_moved = prior_focus
                        .as_ref()
                        .and_then(|p| p.get(&planned.name))
                        .is_none_or(|previous| !close(point, previous));
                    let change = format!("moved from {}", fmt_vec3(point));
                    if hand_moved {
                        if options.keep.focus_points.contains(&planned.name) {
                            planned.kept_edit = true;
                            planned.point = *point;
                        } else {
                            planned.overwrites.push(HandEdit {
                                field: "position".to_string(),
                                mine: fmt_vec3(point),
                                mvr: fmt_vec3(&planned.point),
                            });
                        }
                    }
                    change
                }
                // A focus point the band renamed still sits where the
                // console put it; don't bring the console's name back.
                None if existing
                    .focus_points()
                    .values()
                    .any(|point| close(point, &planned.point)) =>
                {
                    "unchanged (renamed in the venue)".to_string()
                }
                None => "added".to_string(),
            });
        }
        focus_points.retain(|f| f.change.as_deref() != Some("unchanged (renamed in the venue)"));
        for (name, point) in existing.focus_points() {
            if focus_points.iter().any(|f| &f.name == name) {
                continue;
            }
            if previous_focus.contains(&name.as_str()) {
                removed_focus_points.push(name.clone());
            } else {
                kept_focus_points.push(name.clone());
                kept_focus.insert(name.clone(), *point);
            }
        }
    }

    let mut plan = MvrPlan {
        venue_name: venue_name.clone(),
        venue_file,
        archive: library_rel.clone(),
        merge: existing_venue.is_some(),
        origin: origin_m,
        fixture_types,
        fixtures,
        removed_fixtures,
        kept_fixtures,
        focus_points,
        removed_focus_points,
        kept_focus_points,
        scenery_objects,
        scenery_meshes_undrawn,
        warnings,
    };
    let mut venue_text = render_venue(&plan, archive_file_name, &kept, &kept_focus);
    if let Some((path, _)) = &existing {
        // A merge patches the file in place, so comments, TODO lines and
        // the header survive. A file the patcher cannot handle safely is
        // regenerated, and the plan says so.
        match patch_existing(path, &plan, &kept, &kept_focus) {
            Ok(text) => venue_text = text,
            Err(reason) => plan.warnings.push(format!(
                "could not patch the venue file in place ({reason}); it was regenerated, so \
                 its comments are not kept"
            )),
        }
    }
    // Prove the venue parses back, and to the same shape, before anything
    // is written: the loader's check, made while a refusal is still free.
    let parsed = parse_venues(&venue_text).map_err(|e| {
        format!("the seeded venue would not parse back: {e}\n--- venue text ---\n{venue_text}")
    })?;
    let Some(parsed) = parsed.get(&plan.venue_name) else {
        return Err("the seeded venue would not parse back under its own name".into());
    };
    let expected = plan.fixtures.iter().filter(|f| f.todo.is_none()).count() + kept.len();
    if parsed.fixtures().len() != expected {
        return Err(format!(
            "the seeded venue would parse back with {} fixtures instead of {expected}",
            parsed.fixtures().len()
        )
        .into());
    }
    Ok(Planned {
        plan,
        gdtf_writes,
        distillation_warnings,
        write_archive,
        venue_text,
        venue_path,
    })
}

/// Finds a venue file of this name in the venues directory: `.venue` first,
/// then the v1 `.light`. The file must hold the named venue.
fn find_existing_venue(
    project: &Path,
    venues_dir: &str,
    venue_name: &str,
) -> Result<Option<(PathBuf, Venue)>, Box<dyn Error>> {
    // Wherever the venue lives — any file, any depth — as the lighting
    // system finds it; a merge must patch that file, not seed a twin.
    if let Some(declared) =
        crate::lighting::project_files::venues(&project.join(venues_dir)).get(venue_name)
    {
        return Ok(Some((declared.file.clone(), declared.item.clone())));
    }
    let stem = fixture_filename_stem(venue_name);
    for extension in ["venue", "light"] {
        let path = project.join(venues_dir).join(format!("{stem}.{extension}"));
        if !path.exists() {
            continue;
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        let mut venues = parse_venues(&content)
            .map_err(|e| format!("{} does not parse: {e}", path.display()))?;
        return match venues.remove(venue_name) {
            Some(venue) => Ok(Some((path, venue))),
            None => Err(format!(
                "{} exists but does not define a venue named \"{venue_name}\" (it holds: {}); \
                 import under a different --name",
                path.display(),
                venues
                    .keys()
                    .map(|k| format!("\"{k}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
            .into()),
        };
    }
    Ok(None)
}

/// Names a scene's fixtures the way the venue file will state them. Consoles
/// name fixtures by type ("Robe Spiider" forty times) and tell them apart by
/// fixture ID; a name that repeats takes its ID.
struct Namer {
    counts: HashMap<String, usize>,
    seen: HashMap<String, usize>,
    /// Fixtures that took their console fixture ID.
    numbered: usize,
    /// Names that needed an ordinal.
    ordinal_named: Vec<String>,
}

impl Namer {
    fn new(scene: &Scene) -> Namer {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for fixture in &scene.fixtures {
            *counts.entry(fixture.name.trim().to_string()).or_default() += 1;
        }
        Namer {
            counts,
            seen: HashMap::new(),
            numbered: 0,
            ordinal_named: Vec::new(),
        }
    }

    /// The name of `fixture`, the `index`th of its scene; call in order.
    fn name(&mut self, fixture: &MvrFixture, index: usize, warnings: &mut Vec<String>) -> String {
        let base = fixture.name.trim();
        let repeated = self.counts.get(base).is_some_and(|c| *c > 1);
        let candidate = match (&fixture.fixture_id, repeated) {
            (Some(id), true) if !base.is_empty() => {
                self.numbered += 1;
                format!("{base} {}", id.trim())
            }
            _ => base.to_string(),
        };
        unique_name(
            &candidate,
            index,
            &mut self.seen,
            &mut self.ordinal_named,
            warnings,
        )
    }
}

/// A fixture name unique within the venue: the MVR's, made unique on
/// collision and invented when blank. A name that needed an ordinal is
/// recorded in `ordinal_named` for one collapsed warning.
fn unique_name(
    name: &str,
    index: usize,
    seen: &mut HashMap<String, usize>,
    ordinal_named: &mut Vec<String>,
    warnings: &mut Vec<String>,
) -> String {
    let base = if name.trim().is_empty() {
        format!("Fixture {}", index + 1)
    } else {
        dsl_safe(name, warnings)
    };
    let count = seen.entry(base.clone()).or_default();
    *count += 1;
    if *count == 1 {
        return base;
    }
    let unique = format!("{base} ({count})");
    ordinal_named.push(unique.clone());
    unique
}

/// A name as the DSL can quote it: the grammar's strings have no escapes,
/// so a quote or control character in an MVR-supplied name is replaced
/// (reported) rather than written into a file that would not parse back.
fn dsl_safe(name: &str, warnings: &mut Vec<String>) -> String {
    let trimmed = name.trim();
    if !trimmed.chars().any(|c| c == '"' || c.is_control()) {
        return trimmed.to_string();
    }
    let safe: String = trimmed
        .chars()
        .map(|c| match c {
            '"' => '\'',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    warnings.push(format!(
        "name {trimmed:?} contains a quote or control character; written as \"{safe}\""
    ));
    safe
}

fn geometry(
    fixture: &MvrFixture,
    origin_mm: &Vec3,
    warnings: &mut Vec<String>,
) -> (Option<Vec3>, Option<Vec3>) {
    let Some(matrix) = fixture.matrix else {
        return (None, None);
    };
    let (rotation, exact) = matrix.rotation_degrees();
    if !exact {
        warnings.push(format!(
            "fixture \"{}\": transform is sheared or degenerate; rotation is approximate",
            fixture.name
        ));
    }
    (Some(to_stage(&matrix.o, origin_mm)), Some(rotation))
}

/// MVR millimeters, re-origined, to stage meters.
fn to_stage(mm: &Vec3, origin_mm: &Vec3) -> Vec3 {
    [
        (mm[0] - origin_mm[0]) / 1000.0,
        (mm[1] - origin_mm[1]) / 1000.0,
        (mm[2] - origin_mm[2]) / 1000.0,
    ]
}

/// Within a centimeter: the tolerance for "the same point".
fn close(a: &Vec3, b: &Vec3) -> bool {
    (0..3).all(|i| (a[i] - b[i]).abs() < 0.01)
}

/// A merged fixture's mode in the venue and in the MVR, as the GDTF spells
/// them, for a fixture whose type the MVR does not change.
struct ModeFacts {
    /// The venue's: the line's mode. `None` when the line names none (a
    /// load error the merge corrects).
    mine: Option<String>,
    /// The new MVR's.
    mvr: String,
    /// The previous MVR's, when there is a previous MVR that patched it.
    prior: Option<String>,
}

/// The mode facts for a fixture the venue and the MVR both have, when they
/// agree on its type (a type change is its own edit, and a mode is only a
/// mode of its type). Every spelling is matched against the GDTF, so a
/// trimmed or re-cased name is the same mode.
fn compare_modes(
    theirs: &Fixture,
    planned: &PlannedFixture,
    prior: Option<&Prior>,
    embedded: &HashMap<String, Embedded>,
) -> Option<ModeFacts> {
    let key = planned.gdtf.as_ref()?;
    let type_name = planned.fixture_type.as_ref()?;
    if type_name != theirs.fixture_type() {
        return None;
    }
    let canonical = |written: &str| match &embedded[&key.entry].description {
        Ok(description) => gdtf::match_mode(description, written)
            .map(|m| m.name)
            .unwrap_or_else(|_| written.to_string()),
        Err(_) => written.to_string(),
    };
    let mine = theirs.mode().map(canonical);
    Some(ModeFacts {
        mine,
        mvr: key.mode.clone(),
        prior: prior.and_then(|p| p.mode.as_deref()).map(canonical),
    })
}

/// A mode for a message: the name, or that there is none.
fn mode_text(mode: Option<&str>) -> String {
    match mode {
        Some(mode) => format!("\"{mode}\""),
        None => "no mode".to_string(),
    }
}

fn describe_change(
    theirs: &Fixture,
    planned: &PlannedFixture,
    modes: Option<&ModeFacts>,
) -> String {
    let mut changes = Vec::new();
    if let Some(fixture_type) = &planned.fixture_type {
        if fixture_type != theirs.fixture_type() {
            changes.push(format!("type {} → {fixture_type}", theirs.fixture_type()));
        }
    }
    if let Some(modes) = modes {
        if modes.mine.as_deref() != Some(modes.mvr.as_str()) {
            changes.push(format!(
                "mode {} → \"{}\"",
                mode_text(modes.mine.as_deref()),
                modes.mvr
            ));
        }
    }
    if let Some((universe, address)) = planned.patch {
        if (universe, address) != (theirs.universe(), theirs.start_channel()) {
            changes.push(format!(
                "patch {}:{} → {universe}:{address}",
                theirs.universe(),
                theirs.start_channel()
            ));
        }
    }
    match (theirs.position(), planned.position) {
        (Some(a), Some(b)) if !close(&a, &b) => {
            changes.push(format!("moved from {}", fmt_vec3(&a)));
        }
        (None, Some(_)) => changes.push("now positioned".to_string()),
        (Some(_), None) => changes.push("position lost (MVR has none)".to_string()),
        _ => {}
    }
    if planned.todo.is_some() {
        changes.push("now a TODO".to_string());
    }
    if changes.is_empty() {
        "unchanged".to_string()
    } else {
        changes.join("; ")
    }
}

fn relative_display(project: &Path, path: &Path) -> String {
    path.strip_prefix(project)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

/// What the previous MVR put on a fixture, as the previous import stated it.
struct Prior {
    position: Option<Vec3>,
    rotation: Option<Vec3>,
    patch: Option<(u16, u16)>,
    /// The `GDTFMode` as the previous MVR wrote it (unmatched).
    mode: Option<String>,
}

/// The fields of `theirs` (the venue's fixture) that a merge would replace
/// with the MVR's value although the venue's owner edited them: the venue
/// differs from the new MVR and from what the previous MVR said. Without a
/// previous value to compare (`prior` is `None`) any difference is a hand
/// edit; so is any type difference, since the previous type name is not
/// recoverable from the previous MVR without re-resolving its GDTFs.
fn hand_edits(
    theirs: &Fixture,
    planned: &PlannedFixture,
    prior: Option<&Prior>,
    modes: Option<&ModeFacts>,
) -> Vec<HandEdit> {
    let mut edits = Vec::new();
    // Treated the way the patch is: a mode the venue's owner changed is
    // listed, and kept only when asked.
    if let Some(modes) = modes {
        if modes.mine.as_deref() != Some(modes.mvr.as_str())
            && (prior.is_none() || modes.prior != modes.mine)
        {
            edits.push(HandEdit {
                field: "mode".to_string(),
                mine: mode_text(modes.mine.as_deref()),
                mvr: mode_text(Some(&modes.mvr)),
            });
        }
    }
    if let (Some(mine), Some(mvr)) = (theirs.position(), planned.position) {
        if !close(&mine, &mvr)
            && prior
                .and_then(|p| p.position)
                .is_none_or(|previous| !close(&mine, &previous))
        {
            edits.push(HandEdit {
                field: "position".to_string(),
                mine: fmt_vec3(&mine),
                mvr: fmt_vec3(&mvr),
            });
        }
    }
    if let (Some(mine), Some(mvr)) = (theirs.rotation(), planned.rotation) {
        if !close(&mine, &mvr)
            && prior
                .and_then(|p| p.rotation)
                .is_none_or(|previous| !close(&mine, &previous))
        {
            edits.push(HandEdit {
                field: "rotation".to_string(),
                mine: fmt_vec3(&mine),
                mvr: fmt_vec3(&mvr),
            });
        }
    }
    if let Some(mvr) = planned.patch {
        let mine = (theirs.universe(), theirs.start_channel());
        if mine != mvr
            && prior
                .and_then(|p| p.patch)
                .is_none_or(|previous| previous != mine)
        {
            edits.push(HandEdit {
                field: "patch".to_string(),
                mine: format!("{}:{}", mine.0, mine.1),
                mvr: format!("{}:{}", mvr.0, mvr.1),
            });
        }
    }
    if let Some(mvr) = &planned.fixture_type {
        if mvr != theirs.fixture_type() {
            edits.push(HandEdit {
                field: "type".to_string(),
                mine: theirs.fixture_type().to_string(),
                mvr: mvr.clone(),
            });
        }
    }
    edits
}

/// `# layer "..."` for a fixture on an MVR layer.
fn layer_comment(planned: &PlannedFixture) -> Option<String> {
    if planned.layer.is_empty() {
        return None;
    }
    let layer: String = planned
        .layer
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    Some(format!("# layer \"{layer}\""))
}

/// The `# TODO fixture ...` line for a fixture the import could not resolve.
fn todo_line(planned: &PlannedFixture) -> String {
    let reason = planned
        .todo
        .clone()
        .unwrap_or_else(|| "unresolved".to_string());
    let patch = planned
        .patch
        .map(|(u, a)| format!(" @ {u}:{a}"))
        .unwrap_or_default();
    let position = planned
        .position
        .map(|p| format!(" position {}", fmt_vec3(&p)))
        .unwrap_or_default();
    let layer = layer_comment(planned)
        .map(|c| format!("  {c}"))
        .unwrap_or_default();
    format!(
        "  # TODO fixture \"{}\"{patch}{position}: {reason}{layer}",
        planned.name
    )
}

/// The venue the plan describes, as the parser would hold it.
fn desired_venue(plan: &MvrPlan, kept: &[Fixture], kept_focus: &BTreeMap<String, Vec3>) -> Venue {
    let mut fixtures = HashMap::new();
    for planned in &plan.fixtures {
        if let (Some(fixture_type), Some((universe, address)), None) =
            (&planned.fixture_type, planned.patch, &planned.todo)
        {
            fixtures.insert(
                planned.name.clone(),
                Fixture::new(
                    planned.name.clone(),
                    fixture_type.clone(),
                    universe,
                    address,
                    planned.tags.clone(),
                )
                .with_mode(planned.mode.clone())
                .with_position(planned.position)
                .with_rotation(planned.rotation)
                .with_beam_angle(planned.beam_angle),
            );
        }
    }
    for fixture in kept {
        fixtures.insert(fixture.name().to_string(), fixture.clone());
    }
    let mut focus = kept_focus.clone();
    for planned in &plan.focus_points {
        focus.insert(planned.name.clone(), planned.point);
    }
    Venue::new(plan.venue_name.clone(), fixtures)
        .with_focus_points(focus)
        .with_source(Some(VenueSource {
            mvr: plan.archive.clone(),
            origin: plan.origin,
        }))
}

/// A merge as a textual patch of the existing venue file: comments, the
/// header and `# TODO` lines the import did not write survive, an unchanged
/// TODO line stays where it is, and one whose fixture resolved (or changed)
/// is replaced.
fn patch_existing(
    path: &Path,
    plan: &MvrPlan,
    kept: &[Fixture],
    kept_focus: &BTreeMap<String, Vec3>,
) -> Result<String, String> {
    let content = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let desired = desired_venue(plan, kept, kept_focus);
    let by_name: HashMap<&str, &PlannedFixture> =
        plan.fixtures.iter().map(|f| (f.name.as_str(), f)).collect();
    let mut notes = PatchNotes::default();
    let mut present: std::collections::HashSet<&str> = std::collections::HashSet::new();
    let mut stripped = String::with_capacity(content.len());
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim();
        let stale = trimmed
            .strip_prefix("# TODO fixture \"")
            .and_then(|rest| rest.split('"').next())
            .and_then(|name| by_name.get_key_value(name))
            .map(|(name, planned)| {
                let current = planned.todo.is_some() && todo_line(planned).trim() == trimmed;
                if current {
                    present.insert(name);
                }
                !current
            })
            .unwrap_or(false);
        if !stale {
            stripped.push_str(line);
        }
    }
    for planned in &plan.fixtures {
        if planned.todo.is_some() {
            if !present.contains(planned.name.as_str()) {
                notes.trailing_lines.push(todo_line(planned));
            }
        } else if let Some(comment) = layer_comment(planned) {
            notes.fixture_comments.insert(planned.name.clone(), comment);
        }
    }
    patch_venue_with(&stripped, &plan.venue_name, &desired, &notes)
}

/// The venue file's text: a header for the reader, the provenance line,
/// one fixture per line with its MVR layer as a trailing comment, TODOs
/// for what could not be resolved, the focus points.
fn render_venue(
    plan: &MvrPlan,
    archive_file_name: &str,
    kept: &[Fixture],
    kept_focus: &BTreeMap<String, Vec3>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# Seeded from {archive_file_name}. This file is yours: add tags (shows target tags,\n\
         # not fixture names), name the focus points, correct positions. Re-running the\n\
         # import merges a revised MVR into it — rig facts from the MVR, tags from here.\n\
         # Coordinates: meters, right-handed Z-up, origin downstage-center on the deck,\n\
         # +x stage-left, +y upstage. Rotation: degrees about X, Y, Z in that order.\n"
    ));
    out.push_str(&format!("venue \"{}\" {{\n", plan.venue_name));
    out.push_str(&format!(
        "  imported from mvr(\"{}\") origin {}\n",
        plan.archive,
        fmt_vec3(&plan.origin)
    ));

    let mut lines: Vec<((u16, u16, String), String)> = Vec::new();
    for planned in &plan.fixtures {
        let key = (
            planned.patch.map(|p| p.0).unwrap_or(u16::MAX),
            planned.patch.map(|p| p.1).unwrap_or(u16::MAX),
            planned.name.clone(),
        );
        let layer = layer_comment(planned)
            .map(|c| format!("  {c}"))
            .unwrap_or_default();
        let line = match (&planned.fixture_type, planned.patch, &planned.todo) {
            (Some(fixture_type), Some((universe, address)), None) => {
                let fixture = Fixture::new(
                    planned.name.clone(),
                    fixture_type.clone(),
                    universe,
                    address,
                    planned.tags.clone(),
                )
                .with_mode(planned.mode.clone())
                .with_position(planned.position)
                .with_rotation(planned.rotation)
                .with_beam_angle(planned.beam_angle);
                format!("  {fixture}{layer}\n")
            }
            _ => format!("{}\n", todo_line(planned)),
        };
        lines.push((key, line));
    }
    for fixture in kept {
        lines.push((
            (
                fixture.universe(),
                fixture.start_channel(),
                fixture.name().to_string(),
            ),
            format!("  {fixture}\n"),
        ));
    }
    lines.sort();
    for (_, line) in lines {
        out.push_str(&line);
    }

    let mut focus: BTreeMap<&str, Vec3> = kept_focus
        .iter()
        .map(|(name, point)| (name.as_str(), *point))
        .collect();
    for planned in &plan.focus_points {
        focus.insert(&planned.name, planned.point);
    }
    for (name, point) in focus {
        out.push_str(&format!("  focus \"{name}\" {}\n", fmt_vec3(&point)));
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::gdtf::{build_zip, SYNTHETIC_DESCRIPTION};
    use crate::lighting::mvr::SYNTHETIC_SCENE;

    fn gdtf_bytes() -> Vec<u8> {
        build_zip(&[("description.xml", SYNTHETIC_DESCRIPTION.as_bytes())])
    }

    /// An MVR embedding the synthetic PB15 and patching a few of them.
    fn mvr_bytes(scene: &str) -> Vec<u8> {
        let gdtf = gdtf_bytes();
        build_zip(&[
            ("GeneralSceneDescription.xml", scene.as_bytes()),
            ("Astera_PB15.gdtf", gdtf.as_slice()),
        ])
    }

    fn scene_with(fixtures: &str, focus: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<GeneralSceneDescription verMajor="1" verMinor="6"><Scene><Layers>
<Layer name="Front Truss"><ChildList>
{fixtures}
{focus}
</ChildList></Layer>
</Layers></Scene></GeneralSceneDescription>"#
        )
    }

    fn brick(name: &str, address: &str, x_mm: f64) -> String {
        format!(
            r#"<Fixture name="{name}"><Matrix>{{1,0,0}}{{0,1,0}}{{0,0,1}}{{{x_mm},3500,4200}}</Matrix>
<GDTFSpec>Astera_PB15.gdtf</GDTFSpec><GDTFMode>8: RGBS</GDTFMode>
<Addresses><Address break="0">{address}</Address></Addresses></Fixture>"#
        )
    }

    fn project() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("lighting")).unwrap();
        dir
    }

    fn options() -> MvrImportOptions {
        MvrImportOptions {
            name: Some("kellys".to_string()),
            origin_mm: Some([0.0, -3500.0, 0.0]),
            ..MvrImportOptions::default()
        }
    }

    #[test]
    fn a_fresh_import_seeds_a_venue_and_its_fixture_types() {
        let dir = project();
        let bytes = mvr_bytes(&scene_with(
            &format!(
                "{}\n{}",
                brick("Brick 1", "1.1", -2000.0),
                brick("Brick 2", "1.5", 2000.0)
            ),
            r#"<FocusPoint name="Drummer"><Matrix>{1,0,0}{0,1,0}{0,0,1}{0,6300,1400}</Matrix></FocusPoint>"#,
        ));

        let plan = inspect_mvr_bytes(&bytes, "kellys.mvr", &options(), dir.path()).unwrap();
        assert!(!plan.merge);
        assert_eq!(plan.venue_file, "lighting/venues/kellys.venue");
        assert_eq!(plan.fixture_types.len(), 1);
        assert_eq!(plan.fixture_types[0].name, "Synth Brick");
        assert!(!plan.fixture_types[0].existing);
        assert_eq!(plan.fixtures.len(), 2);
        assert_eq!(plan.fixtures[0].position, Some([-2.0, 7.0, 4.2]));
        assert_eq!(plan.fixtures[0].rotation, Some([0.0, 0.0, 0.0]));
        assert_eq!(plan.focus_points[0].point, [0.0, 9.8, 1.4]);
        assert!(
            std::fs::read_dir(dir.path().join("lighting"))
                .unwrap()
                .count()
                == 0,
            "inspecting writes nothing"
        );

        let report = import_mvr_bytes(&bytes, "kellys.mvr", &options(), dir.path()).unwrap();
        assert_eq!(
            report.written,
            vec![
                "lighting/library/kellys.mvr",
                "lighting/library/Astera_PB15.gdtf",
                "lighting/venues/kellys.venue",
            ]
        );
        assert!(
            !dir.path().join("lighting/fixture_types").exists(),
            "the archive is the fixture type; nothing is pinned"
        );
        let venue_text =
            std::fs::read_to_string(dir.path().join("lighting/venues/kellys.venue")).unwrap();
        assert!(
            venue_text.contains(
                "  imported from mvr(\"lighting/library/kellys.mvr\") origin (0, -3.5, 0)\n"
            ),
            "{venue_text}"
        );
        assert!(
            venue_text.contains(
                "  fixture \"Brick 1\" \"Synth Brick\" mode \"8: RGBS\" @ 1:1 position (-2, 7, 4.2) rotation (0, 0, 0)  # layer \"Front Truss\"\n"
            ),
            "{venue_text}"
        );
        assert!(
            venue_text.contains("  focus \"Drummer\" (0, 9.8, 1.4)\n"),
            "{venue_text}"
        );

        // It loads through the real loader, fixture types expanding via the cache.
        let mut system = crate::lighting::system::LightingSystem::new();
        let config = crate::config::lighting::Lighting::new(
            Some("kellys".to_string()),
            None,
            Some(crate::config::lighting::Directories::new(
                Some("lighting/fixture_types".to_string()),
                Some("lighting/venues".to_string()),
            )),
        );
        system.load(&config, dir.path()).unwrap();
        let (_, venue) = system
            .venues_iter()
            .find(|(name, _)| name.as_str() == "kellys")
            .expect("venue loaded");
        assert_eq!(venue.fixtures().len(), 2);
        assert_eq!(venue.focus_points().len(), 1);
        assert!(system
            .gdtf_types_iter()
            .any(|(name, _)| name == "Synth Brick"));
        system
            .get_current_venue_fixtures()
            .expect("every fixture resolves in its own mode");
    }

    #[test]
    fn unresolvable_fixtures_become_todos_not_silence() {
        let dir = project();
        let fixtures = format!(
            "{}\n{}\n{}",
            brick("Good", "1.1", 0.0),
            r#"<Fixture name="Lost"><GDTFSpec>Nowhere.gdtf</GDTFSpec><GDTFMode>X</GDTFMode>
<Addresses><Address break="0">1.20</Address></Addresses></Fixture>"#,
            r#"<Fixture name="Wrong Mode"><GDTFSpec>Astera_PB15.gdtf</GDTFSpec><GDTFMode>99: Nope</GDTFMode>
<Addresses><Address break="0">1.30</Address></Addresses></Fixture>"#,
        );
        let bytes = mvr_bytes(&scene_with(&fixtures, ""));
        let report = import_mvr_bytes(&bytes, "kellys.mvr", &options(), dir.path()).unwrap();
        let todos: Vec<&PlannedFixture> = report
            .plan
            .fixtures
            .iter()
            .filter(|f| f.todo.is_some())
            .collect();
        assert_eq!(todos.len(), 2);
        let text =
            std::fs::read_to_string(dir.path().join("lighting/venues/kellys.venue")).unwrap();
        assert!(
            text.contains("# TODO fixture \"Lost\" @ 1:20: GDTF \"Nowhere.gdtf\" is not embedded"),
            "{text}"
        );
        assert!(text.contains("# TODO fixture \"Wrong Mode\" @ 1:30: GDTF \"Synth Brick\" has no mode matching \"99: Nope\""), "{text}");
        // The good one is a real fixture and the file parses.
        let venues = parse_venues(&text).unwrap();
        assert_eq!(venues["kellys"].fixtures().len(), 1);
    }

    #[test]
    fn a_mode_named_with_a_trailing_space_imports_and_loads() {
        // Real GDTFs do this ("Mode 8 - Pixel RGBW "). The importer matches
        // the console's trimmed reference, writes the exact name, and the
        // written definition must load — proven in the plan, before any
        // write, and then for real.
        let dir = project();
        let description = SYNTHETIC_DESCRIPTION.replace("Name=\"8: RGBS\"", "Name=\"8: RGBS \"");
        let gdtf = build_zip(&[("description.xml", description.as_bytes())]);
        let scene = scene_with(&brick("B", "1.1", 0.0), "");
        let bytes = build_zip(&[
            ("GeneralSceneDescription.xml", scene.as_bytes()),
            ("Astera_PB15.gdtf", gdtf.as_slice()),
        ]);
        let report = import_mvr_bytes(&bytes, "kellys.mvr", &options(), dir.path()).unwrap();
        assert_eq!(report.plan.fixtures[0].mode.as_deref(), Some("8: RGBS "));
        let written = venue_text(dir.path());
        assert!(written.contains("mode \"8: RGBS \""), "{written}");
        let again = parse_venues(&written).unwrap();
        assert_eq!(again["kellys"].fixtures()["B"].mode(), Some("8: RGBS "));
    }

    #[test]
    fn a_drifted_mode_name_matches_with_a_warning() {
        let dir = project();
        let fixtures = brick("B", "1.1", 0.0).replace("8: RGBS", "8 rgbs");
        let bytes = mvr_bytes(&scene_with(&fixtures, ""));
        let plan = inspect_mvr_bytes(&bytes, "k.mvr", &options(), dir.path()).unwrap();
        assert_eq!(plan.fixtures[0].mode.as_deref(), Some("8: RGBS"));
        assert!(
            plan.warnings
                .iter()
                .any(|w| w.contains("after normalization")),
            "{:?}",
            plan.warnings
        );
    }

    #[test]
    fn re_import_merges_rig_facts_and_keeps_tags() {
        let dir = project();
        let first = mvr_bytes(&scene_with(
            &format!(
                "{}\n{}",
                brick("Brick 1", "1.1", -2000.0),
                brick("Brick 2", "1.5", 2000.0)
            ),
            r#"<FocusPoint name="FocusPoint 1"><Matrix>{1,0,0}{0,1,0}{0,0,1}{0,6300,1400}</Matrix></FocusPoint>"#,
        ));
        import_mvr_bytes(&first, "kellys.mvr", &options(), dir.path()).unwrap();

        // The band tags the fixtures, renames the focus point and hangs one
        // of their own.
        let venue_path = dir.path().join("lighting/venues/kellys.venue");
        let edited = std::fs::read_to_string(&venue_path)
            .unwrap()
            .replace("@ 1:1 position", "@ 1:1 tags [\"wash\", \"left\"] position")
            .replace("focus \"FocusPoint 1\"", "focus \"drummer\"")
            .replace(
                "}\n",
                "  fixture \"Own Strobe\" \"Synth Brick\" @ 1:100 tags [\"strobe\"]\n}\n",
            );
        std::fs::write(&venue_path, edited).unwrap();

        // The venue moves Brick 1, removes Brick 2, adds Brick 3, and the
        // focus point is where it was.
        let second = mvr_bytes(&scene_with(
            &format!(
                "{}\n{}",
                brick("Brick 1", "1.9", -1000.0),
                brick("Brick 3", "1.13", 0.0)
            ),
            r#"<FocusPoint name="FocusPoint 1"><Matrix>{1,0,0}{0,1,0}{0,0,1}{0,6300,1400}</Matrix></FocusPoint>"#,
        ));
        let report = import_mvr_bytes(&second, "kellys.mvr", &options(), dir.path()).unwrap();
        let plan = &report.plan;
        assert!(plan.merge);
        assert!(plan.fixture_types[0].existing, "the .fixture is reused");
        assert!(report
            .written
            .contains(&"lighting/library/kellys.mvr".to_string()));
        assert!(!report.written.iter().any(|w| w.ends_with(".fixture")));

        let brick1 = plan.fixtures.iter().find(|f| f.name == "Brick 1").unwrap();
        assert_eq!(brick1.tags, ["wash", "left"], "tags survive");
        assert_eq!(
            brick1.change.as_deref(),
            Some("patch 1:1 → 1:9; moved from (-2, 7, 4.2)")
        );
        let brick3 = plan.fixtures.iter().find(|f| f.name == "Brick 3").unwrap();
        assert_eq!(brick3.change.as_deref(), Some("added"));
        assert_eq!(plan.removed_fixtures.len(), 1);
        assert_eq!(plan.removed_fixtures[0].name, "Brick 2");
        assert_eq!(plan.kept_fixtures, ["Own Strobe"]);
        assert!(
            plan.focus_points.is_empty(),
            "the renamed focus point is not re-added"
        );
        assert_eq!(plan.kept_focus_points, ["drummer"]);

        let text = std::fs::read_to_string(&venue_path).unwrap();
        let venue = &parse_venues(&text).unwrap()["kellys"];
        assert_eq!(venue.fixtures().len(), 3);
        assert_eq!(venue.fixtures()["Brick 1"].start_channel(), 9);
        assert_eq!(venue.fixtures()["Brick 1"].tags(), ["wash", "left"]);
        assert_eq!(venue.fixtures()["Own Strobe"].tags(), ["strobe"]);
        assert!(!venue.fixtures().contains_key("Brick 2"));
        assert_eq!(venue.focus_points().len(), 1);
        assert!(venue.focus_points().contains_key("drummer"));
    }

    #[test]
    fn a_hand_written_venue_is_never_overwritten() {
        let dir = project();
        let venues = dir.path().join("lighting/venues");
        std::fs::create_dir_all(&venues).unwrap();
        std::fs::write(
            venues.join("kellys.light"),
            "venue \"kellys\" {\n  fixture \"A\" T @ 1:1\n}\n",
        )
        .unwrap();
        let bytes = mvr_bytes(&scene_with(&brick("B", "1.1", 0.0), ""));
        let err = import_mvr_bytes(&bytes, "kellys.mvr", &options(), dir.path())
            .unwrap_err()
            .to_string();
        assert!(err.contains("hand-written venue"), "{err}");
        assert!(
            !dir.path().join("lighting/library").exists(),
            "nothing written"
        );
    }

    #[test]
    fn a_library_mvr_of_another_venue_is_not_replaced() {
        let dir = project();
        let bytes = mvr_bytes(&scene_with(&brick("B", "1.1", 0.0), ""));
        import_mvr_bytes(&bytes, "house.mvr", &options(), dir.path()).unwrap();

        let revised = mvr_bytes(&scene_with(&brick("B", "1.2", 0.0), ""));
        let other = MvrImportOptions {
            name: Some("other".to_string()),
            ..options()
        };
        let err = import_mvr_bytes(&revised, "house.mvr", &other, dir.path())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("already exists with different content"),
            "{err}"
        );
    }

    #[test]
    fn a_name_a_hand_written_type_has_pins_the_newcomer_with_its_stem() {
        let dir = project();
        let types = dir.path().join("lighting/fixture_types");
        std::fs::create_dir_all(&types).unwrap();
        std::fs::write(
            types.join("synth_brick.light"),
            "fixture_type \"Synth Brick\" {\n  channels: 3\n  channel_map: {\"red\": 1, \"green\": 2, \"blue\": 3}\n}\n",
        )
        .unwrap();
        let bytes = mvr_bytes(&scene_with(&brick("B", "1.1", 0.0), ""));
        let report = import_mvr_bytes(&bytes, "k.mvr", &options(), dir.path()).unwrap();
        let planned = &report.plan.fixture_types[0];
        assert_eq!(planned.name, "Synth Brick (Astera_PB15)");
        let record = planned.fixture_file.as_deref().expect("the name is pinned");
        let text = std::fs::read_to_string(dir.path().join(record)).unwrap();
        assert!(
            text.contains("fixture_type \"Synth Brick (Astera_PB15)\""),
            "{text}"
        );
        assert!(
            venue_text(dir.path())
                .contains("fixture \"B\" \"Synth Brick (Astera_PB15)\" mode \"8: RGBS\" @ 1:1"),
            "{}",
            venue_text(dir.path())
        );
    }

    #[test]
    fn an_archive_already_in_the_library_is_used_not_copied() {
        let dir = project();
        let library = dir.path().join("lighting/library");
        std::fs::create_dir_all(&library).unwrap();
        // Imported on its own earlier, under the name its maker gave it.
        std::fs::write(library.join("pb15.gdtf"), gdtf_bytes()).unwrap();
        let bytes = mvr_bytes(&scene_with(&brick("B", "1.1", 0.0), ""));
        let report = import_mvr_bytes(&bytes, "k.mvr", &options(), dir.path()).unwrap();
        let planned = &report.plan.fixture_types[0];
        assert!(planned.existing);
        assert_eq!(planned.name, "Synth Brick");
        assert_eq!(planned.archive, "lighting/library/pb15.gdtf");
        assert_eq!(planned.fixture_file, None);
        assert!(!library.join("Astera_PB15.gdtf").exists(), "no second copy");
        assert_eq!(
            report.written,
            ["lighting/library/k.mvr", "lighting/venues/kellys.venue"]
        );
    }

    #[test]
    fn names_with_quotes_are_written_safely_and_nothing_is_half_written() {
        let dir = project();
        let fixtures = brick("Foo&quot;Bar", "1.1", 0.0);
        let focus = r#"<FocusPoint name="Drum &quot;kit&quot;"><Matrix>{1,0,0}{0,1,0}{0,0,1}{0,0,0}</Matrix></FocusPoint>"#;
        let bytes = mvr_bytes(&scene_with(&fixtures, focus));
        let report = import_mvr_bytes(&bytes, "kellys.mvr", &options(), dir.path()).unwrap();
        assert_eq!(report.plan.fixtures[0].name, "Foo'Bar");
        assert_eq!(report.plan.focus_points[0].name, "Drum 'kit'");
        assert!(
            report
                .plan
                .warnings
                .iter()
                .any(|w| w.contains("contains a quote")),
            "{:?}",
            report.plan.warnings
        );
        let text =
            std::fs::read_to_string(dir.path().join("lighting/venues/kellys.venue")).unwrap();
        let venue = &parse_venues(&text).unwrap()["kellys"];
        assert!(venue.fixtures().contains_key("Foo'Bar"));

        // A quote in the archive name cannot be written at all — refused
        // before any write.
        let other = project();
        let err = import_mvr_bytes(&bytes, "ke\"llys.mvr", &options(), other.path())
            .unwrap_err()
            .to_string();
        assert!(err.contains("quote"), "{err}");
        assert!(!other.path().join("lighting/library").exists());
    }

    #[test]
    fn a_quoted_fixture_name_is_reported_without_a_focus_point_to_help() {
        let dir = project();
        let bytes = mvr_bytes(&scene_with(&brick("Foo&quot;Bar", "1.1", 0.0), ""));
        let plan = inspect_mvr_bytes(&bytes, "kellys.mvr", &options(), dir.path()).unwrap();
        assert_eq!(plan.fixtures[0].name, "Foo'Bar");
        assert!(
            plan.warnings.iter().any(|w| w.contains("contains a quote")),
            "{:?}",
            plan.warnings
        );
    }

    #[test]
    fn a_reused_fixture_type_still_checks_the_archive_bytes() {
        let dir = project();
        let bytes = mvr_bytes(&scene_with(&brick("B", "1.1", 0.0), ""));
        import_mvr_bytes(&bytes, "kellys.mvr", &options(), dir.path()).unwrap();
        // The library archive changes underneath the venue that uses it.
        std::fs::write(
            dir.path().join("lighting/library/Astera_PB15.gdtf"),
            b"not the same bytes",
        )
        .unwrap();
        let err = inspect_mvr_bytes(&bytes, "kellys.mvr", &options(), dir.path())
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("a different GDTF called \"Astera_PB15.gdtf\""),
            "{err}"
        );
    }

    #[test]
    fn two_entries_that_stem_alike_are_disambiguated_in_the_plan() {
        let dir = project();
        let gdtf = gdtf_bytes();
        let scene = scene_with(
            &format!(
                "{}\n{}",
                brick("A", "1.1", 0.0).replace("Astera_PB15.gdtf", "Astera-PB15.gdtf"),
                brick("B", "1.5", 0.0)
            ),
            "",
        );
        let bytes = build_zip(&[
            ("GeneralSceneDescription.xml", scene.as_bytes()),
            ("Astera-PB15.gdtf", gdtf.as_slice()),
            ("Astera_PB15.gdtf", gdtf.as_slice()),
        ]);
        let report = import_mvr_bytes(&bytes, "kellys.mvr", &options(), dir.path()).unwrap();
        let files: Vec<&str> = report
            .plan
            .fixture_types
            .iter()
            .map(|t| t.archive.as_str())
            .collect();
        assert_eq!(files.len(), 2);
        assert_ne!(files[0], files[1], "{files:?}");
        for file in files {
            assert!(dir.path().join(file).exists(), "{file}");
        }
    }

    #[test]
    fn a_pixel_mode_distills_ganged_and_says_so() {
        let dir = project();
        // A mode whose channels sit on pixel instances distills with the
        // pixels ganged to one color, and the import report carries the
        // distillation warning that says so.
        let pixel_bar = build_zip(&[(
            "description.xml",
            br#"<GDTF><FixtureType Name="Bar" Manufacturer="m">
  <Geometries>
    <Geometry Name="Base"><GeometryReference Name="Pixel 1" Geometry="Cell"/><GeometryReference Name="Pixel 2" Geometry="Cell"/></Geometry>
  </Geometries>
  <DMXModes>
    <DMXMode Name="Pixel Mode" Geometry="Base">
      <DMXChannels>
        <DMXChannel Offset="1" Geometry="Pixel 1">
          <LogicalChannel Attribute="ColorAdd_R">
            <ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/>
          </LogicalChannel>
        </DMXChannel>
        <DMXChannel Offset="2" Geometry="Pixel 2">
          <LogicalChannel Attribute="ColorAdd_R">
            <ChannelFunction Name="R" Attribute="ColorAdd_R" DMXFrom="0/1"/>
          </LogicalChannel>
        </DMXChannel>
      </DMXChannels>
    </DMXMode>
  </DMXModes>
</FixtureType></GDTF>"#,
        )]);
        let scene = scene_with(
            &format!(
                "{}\n{}",
                brick("Good", "1.1", 0.0),
                r#"<Fixture name="Pixels"><GDTFSpec>Bar.gdtf</GDTFSpec><GDTFMode>Pixel Mode</GDTFMode>
<Addresses><Address break="0">1.10</Address></Addresses></Fixture>"#
            ),
            "",
        );
        let gdtf = gdtf_bytes();
        let bytes = build_zip(&[
            ("GeneralSceneDescription.xml", scene.as_bytes()),
            ("Astera_PB15.gdtf", gdtf.as_slice()),
            ("Bar.gdtf", pixel_bar.as_slice()),
        ]);
        let report = import_mvr_bytes(&bytes, "kellys.mvr", &options(), dir.path()).unwrap();
        let pixels = report
            .plan
            .fixtures
            .iter()
            .find(|f| f.name == "Pixels")
            .unwrap();
        assert!(pixels.todo.is_none(), "{:?}", pixels.todo);
        assert_eq!(report.plan.fixture_types.len(), 2);
        let bar_warnings = &report.distillation_warnings["Bar"];
        assert!(
            bar_warnings.iter().any(|w| w.contains("ganged")),
            "{bar_warnings:?}"
        );
    }

    #[test]
    fn a_merge_without_an_origin_keeps_the_recorded_one() {
        let dir = project();
        let bytes = mvr_bytes(&scene_with(&brick("B", "1.1", 0.0), ""));
        import_mvr_bytes(&bytes, "kellys.mvr", &options(), dir.path()).unwrap();
        let again = MvrImportOptions {
            origin_mm: None,
            ..options()
        };
        let plan = inspect_mvr_bytes(&bytes, "kellys.mvr", &again, dir.path()).unwrap();
        assert_eq!(plan.origin, [0.0, -3.5, 0.0]);
        assert!(
            !plan.warnings.iter().any(|w| w.contains("origin changed")),
            "{:?}",
            plan.warnings
        );
        assert_eq!(plan.fixtures[0].change.as_deref(), Some("unchanged"));
    }

    /// Seeds "kellys" from two bricks (and a fixture with no such mode) and
    /// hand-edits the venue file: a header, comments, a moved Brick 1, a
    /// re-patched and re-typed Brick 2 and a nudged focus point.
    fn seeded_and_hand_edited(dir: &Path) -> PathBuf {
        let lost = r#"<Fixture name="Lost"><GDTFSpec>Astera_PB15.gdtf</GDTFSpec><GDTFMode>99: Nope</GDTFMode>
<Addresses><Address break="0">1.30</Address></Addresses></Fixture>"#;
        let first = mvr_bytes(&scene_with(
            &format!(
                "{}\n{}\n{lost}",
                brick("Brick 1", "1.1", -2000.0),
                brick("Brick 2", "1.5", 2000.0)
            ),
            r#"<FocusPoint name="Drummer"><Matrix>{1,0,0}{0,1,0}{0,0,1}{0,6300,1400}</Matrix></FocusPoint>"#,
        ));
        import_mvr_bytes(&first, "kellys.mvr", &options(), dir).unwrap();
        let path = dir.join("lighting/venues/kellys.venue");
        let text = std::fs::read_to_string(&path).unwrap();
        let venue = &parse_venues(&text).unwrap()["kellys"];
        let old_position = fmt_vec3(&venue.fixtures()["Brick 1"].position().unwrap());
        let edited = format!("# HOUSE NOTE: hung by Sam.\n{text}")
            .replace(
                &format!("position {old_position}"),
                "position (-1.5, 7, 4.2) # moved after the rig check",
            )
            .replace("@ 1:5", "@ 1:105")
            .replace(
                "focus \"Drummer\" (0, 9.8, 1.4)",
                "focus \"Drummer\" (0.4, 9.8, 1.4)",
            )
            .replace(
                "venue \"kellys\" {\n",
                "venue \"kellys\" {\n  # keep this comment\n",
            );
        assert_ne!(edited, text);
        std::fs::write(&path, edited).unwrap();
        path
    }

    /// The same rig again — a re-export that changed nothing the venue's
    /// owner did not also change.
    fn unchanged_mvr() -> Vec<u8> {
        let lost = r#"<Fixture name="Lost"><GDTFSpec>Astera_PB15.gdtf</GDTFSpec><GDTFMode>99: Nope</GDTFMode>
<Addresses><Address break="0">1.30</Address></Addresses></Fixture>"#;
        // A different file (a note in the scene), so the library copy is
        // treated as a revision of the same rig.
        mvr_bytes(&scene_with(
            &format!(
                "{}\n{}\n{lost}\n<!-- revised -->",
                brick("Brick 1", "1.1", -2000.0),
                brick("Brick 2", "1.5", 2000.0)
            ),
            r#"<FocusPoint name="Drummer"><Matrix>{1,0,0}{0,1,0}{0,0,1}{0,6300,1400}</Matrix></FocusPoint>"#,
        ))
    }

    #[test]
    fn a_merge_keeps_the_files_comments_todos_and_header() {
        let dir = project();
        let path = seeded_and_hand_edited(dir.path());
        import_mvr_bytes(&unchanged_mvr(), "kellys.mvr", &options(), dir.path()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# HOUSE NOTE: hung by Sam.\n"), "{text}");
        assert!(text.contains("  # keep this comment\n"), "{text}");
        assert!(text.contains("# moved after the rig check"), "{text}");
        assert!(text.contains("# layer \"Front Truss\""), "{text}");
        assert!(
            text.contains("# TODO fixture \"Lost\" @ 1:30"),
            "the TODO for a fixture with no mode stays: {text}"
        );
        assert_eq!(text.matches("# TODO fixture \"Lost\"").count(), 1, "{text}");
        assert!(parse_venues(&text).is_ok());
    }

    #[test]
    fn a_merge_lists_hand_edits_and_overwrites_them_without_keep() {
        let dir = project();
        let path = seeded_and_hand_edited(dir.path());
        let plan =
            inspect_mvr_bytes(&unchanged_mvr(), "kellys.mvr", &options(), dir.path()).unwrap();
        let brick1 = plan.fixtures.iter().find(|f| f.name == "Brick 1").unwrap();
        assert_eq!(
            brick1
                .overwrites
                .iter()
                .map(|e| e.field.as_str())
                .collect::<Vec<_>>(),
            ["position"],
            "{brick1:?}"
        );
        assert_eq!(brick1.overwrites[0].mine, "(-1.5, 7, 4.2)");
        let brick2 = plan.fixtures.iter().find(|f| f.name == "Brick 2").unwrap();
        assert_eq!(
            brick2
                .overwrites
                .iter()
                .map(|e| (e.field.as_str(), e.mine.as_str(), e.mvr.as_str()))
                .collect::<Vec<_>>(),
            [("patch", "1:105", "1:5")]
        );
        let focus = plan
            .focus_points
            .iter()
            .find(|f| f.name == "Drummer")
            .unwrap();
        assert_eq!(focus.overwrites.len(), 1, "{focus:?}");
        assert_eq!(focus.overwrites[0].mine, "(0.4, 9.8, 1.4)");
        // Planning wrote nothing.
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("position (-1.5, 7, 4.2)"));

        // Without a keep list the MVR's values win.
        import_mvr_bytes(&unchanged_mvr(), "kellys.mvr", &options(), dir.path()).unwrap();
        let venue = &parse_venues(&std::fs::read_to_string(&path).unwrap()).unwrap()["kellys"];
        assert_eq!(
            venue.fixtures()["Brick 1"].position(),
            Some([-2.0, 7.0, 4.2])
        );
        assert_eq!(venue.fixtures()["Brick 2"].start_channel(), 5);
        assert_eq!(venue.focus_points()["Drummer"], [0.0, 9.8, 1.4]);
    }

    #[test]
    fn a_reimport_keeps_a_fixtures_beam_angle_without_being_asked() {
        let dir = project();
        let path = seeded_and_hand_edited(dir.path());
        // The MVR has no beam angle, so a first import left none.
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("beam_angle"), "{text}");
        std::fs::write(&path, text.replace("@ 1:105", "@ 1:105 beam_angle 60")).unwrap();

        let plan =
            inspect_mvr_bytes(&unchanged_mvr(), "kellys.mvr", &options(), dir.path()).unwrap();
        for f in &plan.fixtures {
            assert!(
                f.overwrites.iter().all(|e| e.field != "beam_angle"),
                "beam_angle is never a hand edit to choose: {f:?}"
            );
        }
        // Not kept by name, and Brick 2's patch is overwritten: the angle stays.
        import_mvr_bytes(&unchanged_mvr(), "kellys.mvr", &options(), dir.path()).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        let venue = &parse_venues(&text).unwrap()["kellys"];
        assert_eq!(venue.fixtures()["Brick 2"].start_channel(), 5);
        assert_eq!(venue.fixtures()["Brick 2"].beam_angle(), Some(60.0));
        assert_eq!(venue.fixtures()["Brick 1"].beam_angle(), None);
    }

    #[test]
    fn a_merge_keeps_the_hand_edits_it_is_asked_to() {
        let dir = project();
        let path = seeded_and_hand_edited(dir.path());
        let keep = MvrKeep {
            fixtures: BTreeMap::from([("Brick 1".to_string(), vec!["position".to_string()])]),
            focus_points: vec!["Drummer".to_string()],
        };
        let options = MvrImportOptions { keep, ..options() };
        let report =
            import_mvr_bytes(&unchanged_mvr(), "kellys.mvr", &options, dir.path()).unwrap();
        let brick1 = report
            .plan
            .fixtures
            .iter()
            .find(|f| f.name == "Brick 1")
            .unwrap();
        assert!(brick1.overwrites.is_empty());
        assert_eq!(brick1.kept_edits, ["position"]);

        let text = std::fs::read_to_string(&path).unwrap();
        let venue = &parse_venues(&text).unwrap()["kellys"];
        assert_eq!(
            venue.fixtures()["Brick 1"].position(),
            Some([-1.5, 7.0, 4.2])
        );
        assert_eq!(venue.focus_points()["Drummer"], [0.4, 9.8, 1.4]);
        // Not on the keep list: Brick 2's patch goes back to the MVR's.
        assert_eq!(venue.fixtures()["Brick 2"].start_channel(), 5);
        assert!(text.contains("# moved after the rig check"), "{text}");
    }

    #[test]
    fn a_change_the_mvr_made_is_not_a_hand_edit() {
        let dir = project();
        let first = mvr_bytes(&scene_with(&brick("Brick 1", "1.1", -2000.0), ""));
        import_mvr_bytes(&first, "kellys.mvr", &options(), dir.path()).unwrap();
        // The rig moves and re-patches Brick 1 itself; the venue is untouched.
        let second = mvr_bytes(&scene_with(&brick("Brick 1", "1.9", -1000.0), ""));
        let plan = inspect_mvr_bytes(&second, "kellys.mvr", &options(), dir.path()).unwrap();
        assert!(
            plan.fixtures[0].overwrites.is_empty(),
            "{:?}",
            plan.fixtures[0]
        );
        assert!(plan.fixtures[0]
            .change
            .as_deref()
            .unwrap()
            .contains("patch"));
    }

    /// A brick patched in `mode` rather than "8: RGBS".
    fn brick_in(name: &str, address: &str, x_mm: f64, mode: &str) -> String {
        brick(name, address, x_mm).replace("8: RGBS", mode)
    }

    fn venue_text(dir: &Path) -> String {
        std::fs::read_to_string(dir.join("lighting/venues/kellys.venue")).unwrap()
    }

    #[test]
    fn two_modes_of_one_archive_are_one_type_and_every_line_states_its_mode() {
        let dir = project();
        let bytes = mvr_bytes(&scene_with(
            &[
                brick_in("Spot", "1.1", 0.0, "Mover 16bit"),
                brick("Brick 1", "1.10", -2000.0),
                brick("Brick 2", "1.20", 2000.0),
            ]
            .join("\n"),
            "",
        ));
        let report = import_mvr_bytes(&bytes, "kellys.mvr", &options(), dir.path()).unwrap();
        let types = &report.plan.fixture_types;
        assert_eq!(types.len(), 1, "{types:?}");
        assert_eq!(types[0].name, "Synth Brick", "no mode in the name");
        assert_eq!(types[0].modes, ["8: RGBS", "Mover 16bit"]);
        assert_eq!(types[0].fixture_file, None, "no record to write");
        let mode = |name: &str| {
            report
                .plan
                .fixtures
                .iter()
                .find(|f| f.name == name)
                .unwrap()
                .mode
                .clone()
        };
        assert_eq!(mode("Spot").as_deref(), Some("Mover 16bit"));
        assert_eq!(mode("Brick 1").as_deref(), Some("8: RGBS"));

        let text = venue_text(dir.path());
        assert!(
            text.contains("  fixture \"Spot\" \"Synth Brick\" mode \"Mover 16bit\" @ 1:1 "),
            "{text}"
        );
        assert!(
            text.contains("  fixture \"Brick 1\" \"Synth Brick\" mode \"8: RGBS\" @ 1:10 "),
            "{text}"
        );

        // Through the real loader, each fixture gets its own mode's channels.
        let config = crate::config::lighting::Lighting::new(
            Some("kellys".to_string()),
            None,
            Some(crate::config::lighting::Directories::new(
                Some("lighting/fixture_types".to_string()),
                Some("lighting/venues".to_string()),
            )),
        );
        let mut system = crate::lighting::system::LightingSystem::new();
        system.load(&config, dir.path()).unwrap();
        let infos = system.get_current_venue_fixtures().unwrap();
        let channels = |name: &str| &infos.iter().find(|f| f.name == name).unwrap().channels;
        assert_eq!(channels("Spot").get("pan"), Some(&1));
        assert_eq!(channels("Brick 1").get("red"), Some(&1));
    }

    #[test]
    fn a_merge_moves_a_line_s_mode_the_way_it_moves_an_address() {
        let dir = project();
        let first = mvr_bytes(&scene_with(
            &[
                brick("Brick 1", "1.1", -2000.0),
                brick("Brick 2", "1.10", 2000.0),
                brick("Brick 3", "1.20", 0.0),
            ]
            .join("\n"),
            "",
        ));
        import_mvr_bytes(&first, "kellys.mvr", &options(), dir.path()).unwrap();
        // A hand-added fixture in a mode of its own.
        let path = dir.path().join("lighting/venues/kellys.venue");
        std::fs::write(
            &path,
            venue_text(dir.path()).replace(
                "}\n",
                "  fixture \"Own Spot\" \"Synth Brick\" mode \"Mover 16bit\" @ 2:1\n}\n",
            ),
        )
        .unwrap();

        // The console re-modes Brick 2.
        let second = mvr_bytes(&scene_with(
            &[
                brick("Brick 1", "1.1", -2000.0),
                brick_in("Brick 2", "1.10", 2000.0, "Mover 16bit"),
                brick("Brick 3", "1.20", 0.0),
            ]
            .join("\n"),
            "",
        ));
        let report = import_mvr_bytes(&second, "kellys.mvr", &options(), dir.path()).unwrap();
        let brick2 = report
            .plan
            .fixtures
            .iter()
            .find(|f| f.name == "Brick 2")
            .unwrap();
        assert_eq!(
            brick2.change.as_deref(),
            Some("mode \"8: RGBS\" → \"Mover 16bit\"")
        );
        assert!(brick2.overwrites.is_empty(), "the MVR changed it, not you");
        assert_eq!(report.plan.kept_fixtures, ["Own Spot"]);
        let venue = &parse_venues(&venue_text(dir.path())).unwrap()["kellys"];
        assert_eq!(venue.fixtures()["Brick 2"].mode(), Some("Mover 16bit"));
        assert_eq!(venue.fixtures()["Brick 1"].mode(), Some("8: RGBS"));
        assert_eq!(venue.fixtures()["Own Spot"].mode(), Some("Mover 16bit"));
    }

    #[test]
    fn a_hand_changed_mode_is_listed_and_kept_when_asked() {
        let dir = project();
        let bytes = mvr_bytes(&scene_with(&brick("Brick 1", "1.1", 0.0), ""));
        import_mvr_bytes(&bytes, "kellys.mvr", &options(), dir.path()).unwrap();
        let path = dir.path().join("lighting/venues/kellys.venue");
        std::fs::write(
            &path,
            venue_text(dir.path()).replace(
                "\"Synth Brick\" mode \"8: RGBS\" @ 1:1",
                "\"Synth Brick\" mode \"Mover 16bit\" @ 1:1",
            ),
        )
        .unwrap();
        let revised = mvr_bytes(&scene_with(
            &format!("{}\n<!-- revised -->", brick("Brick 1", "1.1", 0.0)),
            "",
        ));
        let plan = inspect_mvr_bytes(&revised, "kellys.mvr", &options(), dir.path()).unwrap();
        let edits: Vec<(&str, &str, &str)> = plan.fixtures[0]
            .overwrites
            .iter()
            .map(|e| (e.field.as_str(), e.mine.as_str(), e.mvr.as_str()))
            .collect();
        assert_eq!(edits, [("mode", "\"Mover 16bit\"", "\"8: RGBS\"")]);

        let keep = MvrKeep {
            fixtures: BTreeMap::from([("Brick 1".to_string(), vec!["mode".to_string()])]),
            focus_points: Vec::new(),
        };
        import_mvr_bytes(
            &revised,
            "kellys.mvr",
            &MvrImportOptions { keep, ..options() },
            dir.path(),
        )
        .unwrap();
        let venue = &parse_venues(&venue_text(dir.path())).unwrap()["kellys"];
        assert_eq!(venue.fixtures()["Brick 1"].mode(), Some("Mover 16bit"));

        // Without the keep, the MVR's mode wins.
        import_mvr_bytes(&revised, "kellys.mvr", &options(), dir.path()).unwrap();
        let venue = &parse_venues(&venue_text(dir.path())).unwrap()["kellys"];
        assert_eq!(venue.fixtures()["Brick 1"].mode(), Some("8: RGBS"));
    }

    #[test]
    fn the_synthetic_scene_walks_end_to_end() {
        // The mvr module's own synthetic scene: one resolvable brick, one
        // mover whose GDTF is not embedded, a focus point.
        let dir = project();
        let bytes = mvr_bytes(SYNTHETIC_SCENE);
        let plan = inspect_mvr_bytes(
            &bytes,
            "synthetic.mvr",
            &MvrImportOptions::default(),
            dir.path(),
        )
        .unwrap();
        assert_eq!(plan.venue_name, "synthetic");
        assert_eq!(plan.fixtures.len(), 2);
        assert!(plan.fixtures[0].todo.is_none());
        assert!(plan.fixtures[1]
            .todo
            .as_deref()
            .unwrap()
            .contains("not embedded"));
        assert!(
            plan.warnings.iter().any(|w| w.contains("2 DMX breaks")),
            "{:?}",
            plan.warnings
        );
        assert_eq!(plan.focus_points[0].name, "Drummer");
    }
}
