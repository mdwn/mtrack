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
//! embedded GDTF the patch references is imported as a referential
//! `.fixture` on the way, through the same path `import-gdtf` takes.
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

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::path::{Path, PathBuf};

use serde::Serialize;

use super::{create_dir, fixture_filename_stem, gdtf_definition, import_gdtf_bytes, write};
use crate::lighting::gdtf;
use crate::lighting::mvr::{self, MvrFixture, Scene};
use crate::lighting::parser::{parse_fixture_types, parse_venues};
use crate::lighting::types::{fmt_vec3, Fixture, Vec3, Venue};

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
}

impl Default for MvrImportOptions {
    fn default() -> Self {
        MvrImportOptions {
            name: None,
            origin_mm: None,
            fixture_types_dir: "lighting/fixture_types".to_string(),
            venues_dir: "lighting/venues".to_string(),
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
    /// What was skipped, approximated or guessed.
    pub warnings: Vec<String>,
}

/// A fixture type the patch references.
#[derive(Debug, Serialize)]
pub struct PlannedFixtureType {
    /// The fixture type's name.
    pub name: String,
    /// The GDTF archive, project-relative.
    pub archive: String,
    /// The matched mode, as the GDTF spells it.
    pub mode: String,
    /// Whether a `.fixture` of this name already points at this archive
    /// and mode (so nothing is written for it).
    pub existing: bool,
    /// The `.fixture` file, project-relative.
    pub fixture_file: String,
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
    /// Universe and address; `None` when the MVR patched nothing.
    pub patch: Option<(u16, u16)>,
    /// Stage position, meters.
    pub position: Option<Vec3>,
    /// Mounting rotation, degrees.
    pub rotation: Option<Vec3>,
    /// Tags — empty on a fresh seed, the venue's own on a merge.
    pub tags: Vec<String>,
    /// Why this fixture could not be resolved, when it could not.
    pub todo: Option<String>,
    /// On a merge, what changed against the existing venue.
    pub change: Option<String>,
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
}

/// What an import did.
#[derive(Debug, Serialize)]
pub struct MvrImport {
    /// The plan that was carried out.
    #[serde(flatten)]
    pub plan: MvrPlan,
    /// Every file written, project-relative.
    pub written: Vec<String>,
    /// GDTF distillation warnings per newly imported fixture type.
    pub distillation_warnings: BTreeMap<String, Vec<String>>,
}

/// A plan plus the bytes it needs to carry out.
struct Planned {
    plan: MvrPlan,
    /// Embedded GDTFs to place in the library: (project-relative path,
    /// bare file name, bytes, fixture type name, mode).
    gdtf_writes: Vec<GdtfWrite>,
    /// Whether the MVR itself needs writing (new, or changed on re-import).
    write_archive: bool,
    /// The venue file's text.
    venue_text: String,
    venue_path: PathBuf,
}

struct GdtfWrite {
    file_name: String,
    bytes: Vec<u8>,
    type_name: String,
    mode: String,
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
    let mut distillation_warnings = BTreeMap::new();

    let library_dir = project.join("lighting/library");
    create_dir(&library_dir)?;
    if planned.write_archive {
        write(&library_dir.join(archive_file_name), bytes)?;
        written.push(planned.plan.archive.clone());
    }

    // Each new fixture type goes through the GDTF importer proper, so it
    // lands exactly as `import-gdtf` would have put it and is proven to
    // load through the player's own expansion path.
    for gdtf_write in &planned.gdtf_writes {
        let report = import_gdtf_bytes(
            &gdtf_write.bytes,
            &gdtf_write.file_name,
            &gdtf_write.mode,
            Some(&gdtf_write.type_name),
            project,
            &options.fixture_types_dir,
        )?;
        if !report.replaced_archive {
            written.push(report.archive.clone());
        }
        written.push(report.fixture_file.clone());
        distillation_warnings.insert(gdtf_write.type_name.clone(), report.warnings);
    }

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
        let Some(entry) = mvr::resolve_gdtf_entry(&entries, spec) else {
            resolved.push((
                index,
                None,
                Some(format!("GDTF \"{spec}\" is not embedded in the MVR")),
            ));
            continue;
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

    // --- Fixture type names: the GDTF's own, disambiguated only on collision.
    let mut keys: Vec<TypeKey> = resolved.iter().filter_map(|(_, k, _)| k.clone()).collect();
    keys.sort();
    keys.dedup();
    let type_names = name_fixture_types(&keys, &embedded, &mut warnings)?;

    // A mode that matched by name can still refuse to distill (pixel and
    // matrix modes). That is a TODO for its fixtures, found now rather than
    // by the GDTF importer halfway through the writes.
    for key in &keys {
        let Ok(description) = &embedded[&key.entry].description else {
            continue;
        };
        if let Err(e) = gdtf::distill(description, &key.mode, &type_names[key]) {
            let reason = format!("mode \"{}\" does not distill: {e}", key.mode);
            for (_, resolved_key, todo) in &mut resolved {
                if resolved_key.as_ref() == Some(key) {
                    *resolved_key = None;
                    *todo = Some(reason.clone());
                }
            }
        }
    }
    keys.retain(|key| resolved.iter().any(|(_, k, _)| k.as_ref() == Some(key)));

    let mut fixture_types = Vec::new();
    let mut gdtf_writes = Vec::new();
    for key in &keys {
        let type_name = &type_names[key];
        let item = &embedded[&key.entry];
        let archive = format!("lighting/library/{}", item.file_name);
        let fixture_file = format!(
            "{}/{}.fixture",
            options.fixture_types_dir.trim_end_matches('/'),
            fixture_filename_stem(type_name)
        );
        let existing =
            existing_fixture_type(project, &fixture_file, type_name, &archive, &key.mode)?;
        if !existing {
            // Same rule as import-gdtf: a same-named library archive with
            // different bytes would silently re-source other fixtures.
            let library = project.join(&archive);
            if library.exists() && std::fs::read(&library)? != item.bytes {
                return Err(format!(
                    "{archive} already exists with different content — other fixture types \
                     may reference it; remove it deliberately or rename the entry in the MVR"
                )
                .into());
            }
            // The definition the GDTF importer will write must parse back
            // and distill *now*, or the write loop could refuse halfway
            // through the batch — the one thing the plan exists to prevent.
            if let Ok(description) = &item.description {
                let definition =
                    gdtf_definition(type_name, &archive, &key.mode, &item.file_name, description);
                let parsed = parse_fixture_types(&definition).map_err(|e| {
                    format!("the .fixture for \"{type_name}\" would not parse back: {e}")
                })?;
                let pinned = parsed
                    .get(type_name)
                    .and_then(|t| t.source().map(|s| s.mode.clone()))
                    .ok_or_else(|| format!("the .fixture for \"{type_name}\" lost its type"))?;
                gdtf::distill(description, &pinned, type_name)
                    .map_err(|e| format!("the .fixture for \"{type_name}\" would not load: {e}"))?;
            }
            gdtf_writes.push(GdtfWrite {
                file_name: item.file_name.clone(),
                bytes: item.bytes.clone(),
                type_name: type_name.clone(),
                mode: key.mode.clone(),
            });
        }
        fixture_types.push(PlannedFixtureType {
            name: type_name.clone(),
            archive,
            mode: key.mode.clone(),
            existing,
            fixture_file,
        });
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
    // Consoles name fixtures by type ("Robe Spiider" forty times) and tell
    // them apart by fixture ID; a name that repeats takes its ID.
    let mut name_counts: HashMap<String, usize> = HashMap::new();
    for fixture in &scene.fixtures {
        *name_counts
            .entry(fixture.name.trim().to_string())
            .or_default() += 1;
    }
    let mut seen_names: HashMap<String, usize> = HashMap::new();
    let mut fixtures = Vec::new();
    let mut numbered = 0usize;
    let mut ordinal_named: Vec<String> = Vec::new();
    for (index, key, todo) in &resolved {
        let source = &scene.fixtures[*index];
        let base = source.name.trim();
        let repeated = name_counts.get(base).is_some_and(|c| *c > 1);
        let candidate = match (&source.fixture_id, repeated) {
            (Some(id), true) if !base.is_empty() => {
                numbered += 1;
                format!("{base} {}", id.trim())
            }
            _ => base.to_string(),
        };
        let before = ordinal_named.len();
        let name = unique_name(&candidate, *index, &mut seen_names, &mut ordinal_named);
        if ordinal_named.len() > before {
            // Collapsed below into one line rather than one per fixture.
        }
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
        fixtures.push(PlannedFixture {
            name,
            layer: source.layer.clone(),
            fixture_type: key.as_ref().map(|k| type_names[k].clone()),
            patch,
            position,
            rotation,
            tags: Vec::new(),
            todo,
            change: None,
        });
    }
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
            "{} fixtures shared a name and had no console fixture ID to tell them apart; \
             they took an ordinal: {}{}",
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
        for planned in &mut fixtures {
            match existing.fixtures().get(&planned.name) {
                Some(theirs) => {
                    planned.tags = theirs.tags().to_vec();
                    planned.change = Some(describe_change(theirs, planned));
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
                Some(point) => format!("moved from {}", fmt_vec3(point)),
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

    let plan = MvrPlan {
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
        warnings,
    };
    let venue_text = render_venue(&plan, archive_file_name, &kept, &kept_focus);
    // Prove the venue parses back, and to the same shape, before anything
    // is written: the loader's check, made while a refusal is still free.
    let parsed = parse_venues(&venue_text)
        .map_err(|e| format!("the seeded venue would not parse back: {e}"))?;
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

/// Whether a `.fixture` of this name already pins this archive and mode.
/// A file of the same name pointing elsewhere is a collision, refused.
fn existing_fixture_type(
    project: &Path,
    fixture_file: &str,
    type_name: &str,
    archive: &str,
    mode: &str,
) -> Result<bool, Box<dyn Error>> {
    let path = project.join(fixture_file);
    if !path.exists() {
        return Ok(false);
    }
    let content = std::fs::read_to_string(&path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let types = parse_fixture_types(&content)
        .map_err(|e| format!("{} does not parse: {e}", path.display()))?;
    let Some(existing) = types.get(type_name) else {
        return Err(format!(
            "{fixture_file} exists but does not define \"{type_name}\"; remove or rename it"
        )
        .into());
    };
    match existing.source() {
        Some(source) if source.path == archive && source.mode == mode => Ok(true),
        Some(source) => Err(format!(
            "{fixture_file} already defines \"{type_name}\" from {} mode \"{}\", but this MVR \
             patches it from {archive} mode \"{mode}\"; rename one of them",
            source.path, source.mode
        )
        .into()),
        None => Err(format!(
            "{fixture_file} already defines \"{type_name}\" natively; the MVR's GDTF-sourced \
             type of the same name would shadow it — rename one of them"
        )
        .into()),
    }
}

/// Names each (archive, mode) after its GDTF's fixture name, adding the
/// mode — and the archive stem, if that is not enough — only where two
/// keys would otherwise collide. Collision means the same name *or* the
/// same `.fixture` file stem: "Astera-PB15" and "Astera_PB15" are different
/// names on the same file, and the second write would have refused after
/// the first landed.
fn name_fixture_types(
    keys: &[TypeKey],
    embedded: &HashMap<String, Embedded>,
    warnings: &mut Vec<String>,
) -> Result<HashMap<TypeKey, String>, Box<dyn Error>> {
    let base = |key: &TypeKey, warnings: &mut Vec<String>| -> String {
        match &embedded[&key.entry].description {
            Ok(description) => dsl_safe(&description.name, warnings),
            Err(_) => dsl_safe(&key.entry, warnings),
        }
    };
    let mut names: HashMap<TypeKey, String> = HashMap::new();
    for key in keys {
        let name = base(key, warnings);
        names.insert(key.clone(), name);
    }
    for round in 0..2 {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for name in names.values() {
            *counts.entry(fixture_filename_stem(name)).or_default() += 1;
        }
        let mut colliding: Vec<TypeKey> = names
            .iter()
            .filter(|(_, name)| counts[&fixture_filename_stem(name)] > 1)
            .map(|(key, _)| key.clone())
            .collect();
        colliding.sort();
        if colliding.is_empty() {
            break;
        }
        for key in colliding {
            let stem = embedded[&key.entry]
                .file_name
                .trim_end_matches(".gdtf")
                .to_string();
            let mut scratch = Vec::new();
            let name = if round == 0 {
                format!("{} ({})", base(&key, &mut scratch), key.mode)
            } else {
                format!("{} ({}, {})", base(&key, &mut scratch), stem, key.mode)
            };
            names.insert(key, name);
        }
    }
    // Names that still stem alike after the archive stem is in ("Astera-PB15"
    // vs "Astera_PB15") get an ordinal, in key order, so both land.
    let mut by_stem: HashMap<String, Vec<TypeKey>> = HashMap::new();
    for key in keys {
        by_stem
            .entry(fixture_filename_stem(&names[key]))
            .or_default()
            .push(key.clone());
    }
    for (_, mut clashing) in by_stem.into_iter().filter(|(_, keys)| keys.len() > 1) {
        clashing.sort();
        for (ordinal, key) in clashing.into_iter().enumerate().skip(1) {
            let name = format!("{} #{}", names[&key], ordinal + 1);
            warnings.push(format!(
                "fixture type \"{}\" would share a file with another; written as \"{name}\"",
                names[&key]
            ));
            names.insert(key, name);
        }
    }
    let mut by_stem: HashMap<String, Vec<&String>> = HashMap::new();
    for name in names.values() {
        by_stem
            .entry(fixture_filename_stem(name))
            .or_default()
            .push(name);
    }
    if let Some((stem, clashing)) = by_stem.iter().find(|(_, names)| names.len() > 1) {
        return Err(format!(
            "fixture types {} would all be written to {stem}.fixture; rename the embedded \
             GDTF entries so their names differ by more than punctuation",
            clashing
                .iter()
                .map(|n| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(", ")
        )
        .into());
    }
    Ok(names)
}

/// A fixture name unique within the venue: the MVR's, made unique on
/// collision and invented when blank. A name that needed an ordinal is
/// recorded in `ordinal_named` for one collapsed warning.
fn unique_name(
    name: &str,
    index: usize,
    seen: &mut HashMap<String, usize>,
    ordinal_named: &mut Vec<String>,
) -> String {
    let base = if name.trim().is_empty() {
        format!("Fixture {}", index + 1)
    } else {
        let mut scratch = Vec::new();
        dsl_safe(name, &mut scratch)
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

fn describe_change(theirs: &Fixture, planned: &PlannedFixture) -> String {
    let mut changes = Vec::new();
    if let Some(fixture_type) = &planned.fixture_type {
        if fixture_type != theirs.fixture_type() {
            changes.push(format!("type {} → {fixture_type}", theirs.fixture_type()));
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
        let layer = if planned.layer.is_empty() {
            String::new()
        } else {
            let layer: String = planned
                .layer
                .chars()
                .map(|c| if c.is_control() { ' ' } else { c })
                .collect();
            format!("  # layer \"{layer}\"")
        };
        let line = match (&planned.fixture_type, planned.patch, &planned.todo) {
            (Some(fixture_type), Some((universe, address)), None) => {
                let fixture = Fixture::new(
                    planned.name.clone(),
                    fixture_type.clone(),
                    universe,
                    address,
                    planned.tags.clone(),
                )
                .with_position(planned.position)
                .with_rotation(planned.rotation);
                format!("  {fixture}{layer}\n")
            }
            _ => {
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
                format!(
                    "  # TODO fixture \"{}\"{patch}{position}: {reason}{}\n",
                    planned.name,
                    layer.trim_end()
                )
            }
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
                "lighting/fixture_types/synth_brick.fixture",
                "lighting/venues/kellys.venue",
            ]
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
                "  fixture \"Brick 1\" \"Synth Brick\" @ 1:1 position (-2, 7, 4.2) rotation (0, 0, 0)  # layer \"Front Truss\"\n"
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
            None,
            None,
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
            .fixture_types_iter()
            .any(|(name, _)| name == "Synth Brick"));
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
        assert_eq!(report.plan.fixture_types[0].mode, "8: RGBS ");
        let written =
            std::fs::read_to_string(dir.path().join(&report.plan.fixture_types[0].fixture_file))
                .unwrap();
        assert!(written.contains("mode \"8: RGBS \""), "{written}");
        let again = parse_fixture_types(&written).unwrap();
        assert_eq!(again["Synth Brick"].source().unwrap().mode, "8: RGBS ");
    }

    #[test]
    fn a_drifted_mode_name_matches_with_a_warning() {
        let dir = project();
        let fixtures = brick("B", "1.1", 0.0).replace("8: RGBS", "8 rgbs");
        let bytes = mvr_bytes(&scene_with(&fixtures, ""));
        let plan = inspect_mvr_bytes(&bytes, "k.mvr", &options(), dir.path()).unwrap();
        assert_eq!(plan.fixture_types[0].mode, "8: RGBS");
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
    fn a_native_fixture_type_of_the_same_name_is_a_collision() {
        let dir = project();
        let types = dir.path().join("lighting/fixture_types");
        std::fs::create_dir_all(&types).unwrap();
        std::fs::write(
            types.join("synth_brick.light"),
            "fixture_type \"Synth Brick\" {\n  channels: 3\n  channel_map: {\"red\": 1, \"green\": 2, \"blue\": 3}\n}\n",
        )
        .unwrap();
        // Same stem, but the importer checks the .fixture path; a .light
        // twin is invisible to it, so this one succeeds — the collision
        // the importer can see is a .fixture pointing elsewhere.
        std::fs::write(
            types.join("synth_brick.fixture"),
            "fixture_type \"Synth Brick\"\n  from gdtf(\"lighting/library/other.gdtf\", mode \"8: RGBS\")\n{\n}\n",
        )
        .unwrap();
        let bytes = mvr_bytes(&scene_with(&brick("B", "1.1", 0.0), ""));
        let err = import_mvr_bytes(&bytes, "k.mvr", &options(), dir.path())
            .unwrap_err()
            .to_string();
        assert!(err.contains("rename one of them"), "{err}");
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
            .map(|t| t.fixture_file.as_str())
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
