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

//! A GDTF fixture's own settings — its name and its movement limits — as the
//! fixture page's form edits them (lighting UI design §12.4, venue-exchange
//! design §22). They live in mtrack's record of the fixture, which exists
//! only once something is set: a GDTF in the library with no record is a
//! fixture named from its archive, and the first change writes the record
//! (the importer's writer), then patches it ([`lighting::fixture_patch`]).
//! A rename rewrites every venue line that names the type, planned in full
//! before anything is written.

use crate::webui::wire::fixture_types::{
    FixtureSettingsData, Rename, RenamedLines, SettingsRequest, SettingsSaved, StrobeTable,
};
use std::collections::BTreeMap;

/// The version a fixture with no record answers, and a save to it sends as
/// `If-Match`: "no record yet". A save against it refuses if a record has
/// appeared since.
pub(super) const UNRECORDED: &str = "unrecorded";
use std::path::{Path as FsPath, PathBuf};

use axum::http::StatusCode;
use serde_json::json;

use super::lighting_api::{
    canonical_project_root, content_version, locate_fixture_type_file, project_root,
    resolve_lighting_dir, validate_lighting_name, validate_lighting_name_to_write, LightingDir,
    VENUE_WRITES,
};
use super::lighting_writes::Answer;
use crate::lighting;
use crate::lighting::fixture_patch::{current_settings, patch_fixture_type, FixtureSettings};
use crate::lighting::types::{Fixture, MovementLimits, StrobeCurve, Venue};

/// A refusal; a 409 carries `conflict` so a client reloads and asks again,
/// as it does for a venue save.
fn error(status: StatusCode, message: String) -> Answer {
    let conflict = status == StatusCode::CONFLICT;
    Answer {
        status,
        body: json!({"error": message, "conflict": conflict}),
    }
}

/// A GDTF fixture's
/// settings, for the form: `{name, movement, strobe_curve, strobe, version}`. A fixture that is
/// its archive alone (no record) answers its derived name, no limits and the
/// version [`UNRECORDED`]. A native type
/// is a 404 here (it is edited as before).
pub(crate) async fn settings(
    config_path: &FsPath,
    name: &str,
    dir: Option<&str>,
) -> Result<FixtureSettingsData, Answer> {
    validate_lighting_name(name)?;
    let name = name.to_string();
    let dir = resolve_lighting_dir(config_path, dir, LightingDir::FixtureTypes)?;
    let root = canonical_project_root(&project_root(config_path)?)?;
    let Some(path) = locate_fixture_type_file(&dir, &name).await? else {
        let (dir, wanted, project) = (dir.clone(), name.clone(), root.clone());
        let archive = super::helpers::blocking_answer("read the GDTF library", move || {
            Ok::<_, String>(
                lighting::library::unrecorded_types(&project, Some(&dir))
                    .get(&wanted)
                    .map(|t| t.archive.clone()),
            )
        })
        .await?;
        let Some(archive) = archive else {
            return Err(error(
                StatusCode::NOT_FOUND,
                format!("Fixture type not found: {name}"),
            ));
        };
        let strobe = strobe_info(root, archive).await;
        return Ok(FixtureSettingsData {
            name,
            movement: MovementLimits::default(),
            strobe_curve: None,
            strobe,
            version: UNRECORDED.to_string(),
        });
    };
    let read = path.clone();
    let content = super::helpers::blocking_answer("read fixture type", move || {
        crate::util::read_to_string(&read)
    })
    .await?;
    let Some(settings) = current_settings(&content, &name) else {
        return Err(error(
            StatusCode::NOT_FOUND,
            format!("fixture type \"{name}\" is not a GDTF type"),
        ));
    };
    let archive = lighting::parser::parse_fixture_types(&content)
        .ok()
        .and_then(|types| types.get(&name)?.source().map(|s| s.path.clone()));
    let strobe = match archive {
        Some(archive) => strobe_info(root, archive).await,
        None => None,
    };
    Ok(FixtureSettingsData {
        name: settings.name,
        movement: settings.movement,
        strobe_curve: settings.strobe_curve,
        strobe,
        version: content_version(content.as_bytes()),
    })
}

/// What the strobe-curve control needs from a fixture's GDTF: `null` when
/// no mode has a strobe rate in hertz, else `{steps, automatic}` — the
/// size of the strobe function's own table (0: endpoints only, so the
/// declared curve is linear in hertz) and the curve mtrack uses when the
/// record states none (always `declared` for a GDTF, the spec's rule). An
/// archive that cannot be read is `null` too.
async fn strobe_info(root: PathBuf, archive: String) -> Option<StrobeTable> {
    tokio::task::spawn_blocking(move || {
        let bytes = crate::util::read_file(&root.join(&archive)).ok()?;
        let description = lighting::gdtf::parse_archive(&bytes).ok()?;
        let steps = lighting::gdtf::strobe_table_steps(&description)?;
        let automatic = StrobeCurve::automatic(true);
        Some(StrobeTable { steps, automatic })
    })
    .await
    .ok()
    .flatten()
}

/// One venue file's planned rewrite for a rename.
pub(super) struct VenueRewrite {
    path: PathBuf,
    file: String,
    content: String,
    /// The file as it was read, which a failed save puts back.
    original: String,
    /// (venue, lines changed).
    venues: Vec<(String, usize)>,
}

/// What a fixture type's rename does to the venues, worked out from the
/// files: the one plan the fixture settings save and the type editor's
/// rename both write.
pub(super) struct VenueRetype {
    pub rewrites: Vec<VenueRewrite>,
    /// Every venue file read, by file name, with its version.
    pub versions: BTreeMap<String, String>,
}

impl VenueRetype {
    /// The rename as the answer says it.
    pub fn summary(&self, from: &str, to: &str) -> Rename {
        let venues = self
            .rewrites
            .iter()
            .flat_map(|r| {
                r.venues.iter().map(move |(venue, lines)| RenamedLines {
                    venue: venue.clone(),
                    file: r.file.clone(),
                    lines: *lines,
                })
            })
            .collect();
        let lines: usize = self
            .rewrites
            .iter()
            .flat_map(|r| &r.venues)
            .map(|(_, n)| n)
            .sum();
        Rename {
            from: from.to_string(),
            to: to.to_string(),
            venues,
            lines,
        }
    }
}

/// Writes a fixture type's save with what goes with it: the `pins` (made
/// new), the type's file by its `record` plan (made or rewritten), the
/// `venues` rewrites, and last the old type file of a move removed. All or
/// nothing, by the one rule for writes ([`lighting::writes::Changes`]): a
/// failure at any step, the last included, puts back everything written
/// before it and what the failing step changed, and the answer says so,
/// naming any file that could not be put back.
pub(super) fn write_type_save(
    types_dir: &FsPath,
    pins: &[(PathBuf, String)],
    record: &lighting::writes::Plan,
    text: &str,
    venues: &VenueRetype,
) -> Result<(), Refusal> {
    lighting::writes::Changes::all_or_nothing(|changes| {
        for (path, pin) in pins {
            let shown = lighting::project_files::display_in(types_dir, path);
            changes.create(path, pin.as_bytes(), shown)?;
        }
        changes.plan(record, text)?;
        for rewrite in &venues.rewrites {
            changes.rewrite(
                &rewrite.path,
                rewrite.content.as_bytes(),
                rewrite.original.as_bytes(),
                format!("venue file {}", rewrite.file),
            )?;
        }
        changes.retire(record)
    })
    .map_err(|e| (super::lighting_writes::write_status(&e), e.to_string()))
}

/// What a settings save would do, worked out from the files.
struct Plan {
    /// The record's write: a new record claimed for a library fixture with
    /// none, or its file's by the one rule for type files.
    record: lighting::writes::Plan,
    type_version: String,
    patched: String,
    venues: VenueRetype,
    /// The record is new: written for the first time by this save.
    new_record: bool,
    /// Records pinning the current names of the library fixtures that share
    /// this one's name (a rename would otherwise move theirs): (path, text),
    /// each claimed new.
    pins: Vec<(PathBuf, String)>,
    /// Nothing differs from what is there: a save writes nothing.
    unchanged: bool,
}

/// A planning refusal: the status and the message.
pub(super) type Refusal = (StatusCode, String);

/// Why a settings plan was not made.
enum PlanError {
    /// A refusal.
    Refused(Refusal),
    /// The record is not at the version the caller sent: the message, and
    /// the version it is at.
    Stale { message: String, version: String },
}

impl From<Refusal> for PlanError {
    fn from(refusal: Refusal) -> Self {
        PlanError::Refused(refusal)
    }
}

/// A refusal of the one rule for type files, as a settings refusal.
fn refused(refused: lighting::writes::Refused) -> Refusal {
    (
        super::lighting_writes::refused_status(&refused),
        refused.to_string(),
    )
}

/// A venue with every fixture of type `from` re-typed `to`, and how many.
fn retyped(venue: &Venue, from: &str, to: &str) -> (Venue, usize) {
    let mut count = 0;
    let fixtures = venue
        .fixtures()
        .iter()
        .map(|(name, f)| {
            let f = if f.fixture_type() == from {
                count += 1;
                Fixture::new(
                    f.name().to_string(),
                    to.to_string(),
                    f.universe(),
                    f.start_channel(),
                    f.tags().to_vec(),
                )
                .with_mode(f.mode().map(str::to_string))
                .with_position(f.position())
                .with_rotation(f.rotation())
                .with_beam_angle(f.beam_angle())
            } else {
                f.clone()
            };
            (name.clone(), f)
        })
        .collect();
    let renamed = Venue::new(venue.name().to_string(), fixtures)
        .with_focus_points(venue.focus_points().clone())
        .with_source(venue.source().cloned());
    (renamed, count)
}

/// Every venue file under `venues_dir` (the files the lighting system
/// loads), read and versioned, each refused when its version is no longer
/// the one in `expected`; and, for a rename of type `from` to `to`, every
/// venue line naming `from` retyped. Nothing is written here.
///
/// A rename is refused, so nothing is written, when any file cannot be
/// read or does not parse (it cannot be checked for the type), or when a
/// venue using the type is not its file's alone, by the one rule for venue
/// files: each such venue is named with its file and why.
pub(super) fn plan_venue_retype(
    venues_dir: &FsPath,
    from: &str,
    to: Option<&str>,
    expected: &std::collections::HashMap<String, String>,
) -> Result<VenueRetype, Refusal> {
    let read = if venues_dir.is_dir() {
        lighting::project_files::venues(venues_dir)
    } else {
        Default::default()
    };
    let mut versions = BTreeMap::new();
    let mut rewrites = Vec::new();
    let mut read_only: Vec<String> = Vec::new();
    for file_read in read.record.files() {
        let path = &file_read.file;
        // Relative to the venues directory: two subdirectories may each
        // hold a `club.light`, and each needs its own version.
        let file = lighting::project_files::display_in(venues_dir, path);
        let text = crate::util::read_to_string(path).map_err(|e| {
            (
                StatusCode::CONFLICT,
                format!("venue file {file} cannot be read ({e}); nothing was written"),
            )
        })?;
        let version = content_version(text.as_bytes());
        if expected.get(&file).is_some_and(|v| *v != version) {
            return Err((
                StatusCode::CONFLICT,
                format!(
                    "venue file {file} changed since the plan was made; nothing was written — \
                     review the change again"
                ),
            ));
        }
        versions.insert(file.clone(), version);
        let Some(to) = to else {
            continue;
        };
        let venues = lighting::parser::parse_venues(&text).map_err(|e| {
            // A file that does not parse cannot be checked for the name, so
            // a rename cannot promise to have found every line.
            (
                StatusCode::CONFLICT,
                format!(
                    "venue file {file} does not parse ({e}), so the fixtures in it cannot be \
                     renamed; nothing was written — fix it by hand first"
                ),
            )
        })?;
        let mut names: Vec<&String> = venues.keys().collect();
        names.sort();
        let mut text_now = text.clone();
        let mut touched = Vec::new();
        for venue_name in names {
            let (desired, count) = retyped(&venues[venue_name], from, to);
            if count == 0 {
                continue;
            }
            match read.record.owner(venue_name) {
                Some(owner)
                    if owner.editable == lighting::files::Editable::Editable
                        && owner.path == *path => {}
                owner => {
                    let why = owner
                        .and_then(|o| o.reason(lighting::files::Kind::Venue, venues_dir))
                        .unwrap_or_default();
                    let said = format!("venue \"{venue_name}\" in {file} uses it: {why}");
                    if !read_only.contains(&said) {
                        read_only.push(said);
                    }
                    continue;
                }
            }
            text_now = lighting::venue_patch::patch_venue(&text_now, venue_name, &desired)
                .map_err(|e| {
                    (
                        StatusCode::CONFLICT,
                        format!(
                            "venue \"{venue_name}\" in {file} cannot be patched ({e}); nothing \
                             was written"
                        ),
                    )
                })?;
            touched.push((venue_name.clone(), count));
        }
        if !touched.is_empty() {
            rewrites.push(VenueRewrite {
                path: path.clone(),
                file,
                content: text_now,
                original: text,
                venues: touched,
            });
        }
    }
    if !read_only.is_empty() {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "fixture type \"{from}\" cannot be renamed, so nothing was written; {}",
                read_only.join(" ")
            ),
        ));
    }
    Ok(VenueRetype { rewrites, versions })
}

/// Works out the whole save from the files. Nothing is written here.
#[allow(clippy::too_many_arguments)]
fn plan(
    root: &FsPath,
    types_dir: &FsPath,
    venues_dir: &FsPath,
    name: &str,
    request: &SettingsRequest,
    expected_type_version: Option<&str>,
) -> Result<Plan, PlanError> {
    let bad = |m: String| (StatusCode::BAD_REQUEST, m);
    let types = lighting::project_files::fixture_types(root, Some(types_dir));
    let library = &types.library;
    let new_name = request.name.trim().to_string();
    let renaming = new_name != name;
    if renaming {
        validate_lighting_name_to_write(&new_name, false).map_err(|_| {
            bad(format!(
                "\"{new_name}\" cannot be a fixture type name (letters, digits, spaces, \
                 hyphens and underscores; not a route name)"
            ))
        })?;
    }
    let mut claimed = lighting::files::Claimed::default();
    // The record, written by the one rule for type files: where its file is
    // its alone (a rename moving one at its own place), or — for a GDTF in
    // the library with none yet — a new one claimed under the name it will
    // hold, created by this save.
    let (record, content, type_version, new_record) = match types.files.record.owner(name) {
        Some(_) => {
            let save = lighting::writes::Save {
                kind: lighting::files::Kind::FixtureType,
                dir: types_dir,
                record: &types.files.record,
                library: &library.types,
                name: &new_name,
                from: Some(name),
                // A GDTF record states its archive, which only a `.fixture`
                // holds.
                needs_new_format: true,
            };
            let record = save.plan().map_err(refused)?;
            let content = record
                .current
                .as_deref()
                .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
                .unwrap_or_default();
            let version = content_version(content.as_bytes());
            (record, content, version, false)
        }
        None => {
            let unrecorded = library.get(name).ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    format!("Fixture type not found: {name}"),
                )
            })?;
            let (path, content) = new_record_for(
                root,
                types_dir,
                unrecorded,
                &new_name,
                &library.types,
                &mut claimed,
            )?;
            let record = lighting::writes::Plan {
                dir: types_dir.to_path_buf(),
                write: lighting::writes::Write::Create(path),
                retire: None,
                current: None,
            };
            (record, content, UNRECORDED.to_string(), true)
        }
    };
    // The file the record is in now: the one a move retires, else the one
    // written.
    let type_file = lighting::project_files::display_in(
        types_dir,
        record.retire.as_deref().unwrap_or(record.write.path()),
    );
    if expected_type_version.is_some_and(|v| v != type_version) {
        return Err(PlanError::Stale {
            message: format!(
                "fixture type \"{name}\" changed since you loaded it ({type_file}); it has been \
                 reloaded, so make your change again"
            ),
            version: type_version,
        });
    }
    let current = current_settings(&content, name)
        .ok_or_else(|| bad(format!("fixture type \"{name}\" is not a GDTF type")))?;
    let settings = FixtureSettings {
        name: new_name.clone(),
        movement: request.movement,
        strobe_curve: request.strobe_curve,
    };
    let unchanged = settings == current;
    let patched = patch_fixture_type(&content, name, &settings).map_err(bad)?;
    // A library fixture renamed away from a name it shares with another
    // (`Name` and `Name (stem)`) would move the other's: pin theirs too.
    let mut pins = Vec::new();
    if renaming && new_record {
        let base = library
            .get(name)
            .map(|t| t.renamed_from.clone().unwrap_or_else(|| t.name.clone()));
        for sibling in library.types.iter().filter(|t| {
            t.name != name && Some(t.renamed_from.clone().unwrap_or_else(|| t.name.clone())) == base
        }) {
            pins.push(new_record_for(
                root,
                types_dir,
                sibling,
                &sibling.name,
                &library.types,
                &mut claimed,
            )?);
        }
    }

    let venues = plan_venue_retype(
        venues_dir,
        name,
        renaming.then_some(new_name.as_str()),
        &request.venue_versions,
    )?;

    Ok(Plan {
        record,
        type_version,
        patched,
        venues,
        new_record,
        pins,
        unchanged,
    })
}

/// The record the importer would write for a library fixture with none,
/// under the name `named` (a rename's new one): claimed by the one rule for
/// a GDTF's record ([`lighting::files::Claimed::record_for`]), so refused
/// when the name is declared or another archive's, or its `<stem>.fixture`,
/// in any case, is there or already `claimed` by this save.
fn new_record_for(
    root: &FsPath,
    types_dir: &FsPath,
    library_type: &lighting::library::LibraryType,
    named: &str,
    library: &[lighting::library::LibraryType],
    claimed: &mut lighting::files::Claimed,
) -> Result<(PathBuf, String), Refusal> {
    let bytes = crate::util::read_file(&root.join(&library_type.archive)).map_err(|e| {
        (
            StatusCode::CONFLICT,
            format!("{} cannot be read: {e}", library_type.file_name),
        )
    })?;
    let description = lighting::gdtf::parse_archive(&bytes).map_err(|e| {
        (
            StatusCode::CONFLICT,
            format!("{} is not a readable GDTF: {e}", library_type.file_name),
        )
    })?;
    let claim = claimed
        .record_for(types_dir, &library_type.archive, named, library)
        .map_err(|r| refused(lighting::writes::Refused::Claim(r)))?;
    let path = claim.path().to_path_buf();
    let text = lighting::import::gdtf_definition(
        &library_type.name,
        &library_type.archive,
        &library_type.file_name,
        &description,
    );
    Ok((path, text))
}

/// The name of the venue the player plays against: the engine's, else the
/// one this host's profile in the config at `config_path` names.
fn current_venue_name(player: &crate::player::Player, config_path: &FsPath) -> Option<String> {
    if let Some(system) = player.broadcast_handles().and_then(|h| h.lighting_system) {
        return system.lock().current_venue().map(str::to_string);
    }
    // codeql[rust/path-injection] config_path is set at startup, not user input.
    let config = crate::config::Player::deserialize(config_path).ok()?;
    config
        .host_lighting()
        .and_then(|l| l.current_venue())
        .map(str::to_string)
}

/// After a fixture type's save that may have rewritten venues: one reload of
/// the running engine's types and venues together, so it never sees a
/// venue naming a type it has not read yet; then whether the current venue
/// still loads ([`super::lighting_api::venue_error_after_save`]). The web UI's and MCP's.
pub(crate) async fn reload_types_and_venues(
    player: &std::sync::Arc<crate::player::Player>,
    config_path: &FsPath,
    type_name: &str,
    venues_dir: &FsPath,
) -> Option<crate::lighting::findings::VenueError> {
    let reloading = player.clone();
    if let Ok(Err(e)) = tokio::task::spawn_blocking(move || reloading.reload_fixture_types()).await
    {
        tracing::warn!(fixture_type = %type_name, error = %e, "fixture type saved, but the running engine could not reload it");
    }
    let current = current_venue_name(player, config_path)?;
    super::lighting_api::venue_error_of(player, config_path, &current, venues_dir).await
}

/// Plans (`write: false`) or saves (`write: true`) a GDTF fixture's
/// settings: `{name, movement: {max_pan_speed, max_tilt_speed}}`. The record
/// is written by the one rule for type files: patched where it lives when
/// its file is its alone (409 otherwise), a rename moving a file at its old
/// name's own place to the new name's; for a fixture with none, the
/// importer's record is claimed at `<stem>.fixture` and made new. A rename
/// rewrites every venue line that names the type, in every venue file
/// ([`plan_venue_retype`]), or — when any file cannot be rewritten, or a
/// venue using it is not its file's alone — nothing at all. The record's
/// version (`expected_version`; [`UNRECORDED`] for none) and `venue_versions`
/// the venue files' from the plan; either having changed is a 409 (the
/// record's carries the version it is at, as `version`). A save holds the
/// venue write lock, writes the venue files then the record, and reloads the
/// running engine's types and venues once. The answer says what changed (or
/// would): `{version, rename, venue_versions, venue_error}`.
pub(crate) async fn save_settings(
    player: &std::sync::Arc<crate::player::Player>,
    config_path: &FsPath,
    name: &str,
    dirs: (Option<&str>, Option<&str>),
    expected_version: Option<String>,
    request: SettingsRequest,
) -> Result<SettingsSaved, Answer> {
    validate_lighting_name(name)?;
    let name = name.to_string();
    let types_dir = resolve_lighting_dir(config_path, dirs.0, LightingDir::FixtureTypes)?;
    let venues_dir = resolve_lighting_dir(config_path, dirs.1, LightingDir::Venues)?;
    let root = canonical_project_root(&project_root(config_path)?)?;
    let expected = expected_version;
    let write = request.write;
    let (vdir, current) = (venues_dir.clone(), name.clone());
    let outcome = super::helpers::blocking_answer("save fixture settings", move || {
        // The same lock as a venue save: a rename's venue writes and a drag
        // on the stage plot never interleave.
        let _guard = write.then(|| VENUE_WRITES.blocking_lock());
        let plan = match plan(
            &root,
            &types_dir,
            &vdir,
            &current,
            &request,
            expected.as_deref(),
        ) {
            Ok(plan) => plan,
            Err(refusal) => return Ok::<_, String>(Err(refusal)),
        };
        if write && !plan.unchanged {
            if plan.new_record || !plan.pins.is_empty() {
                std::fs::create_dir_all(&types_dir)
                    .map_err(|e| format!("creating {}: {e}", types_dir.display()))?;
            }
            if let Err(refusal) = write_type_save(
                &types_dir,
                &plan.pins,
                &plan.record,
                &plan.patched,
                &plan.venues,
            ) {
                return Ok(Err(refusal.into()));
            }
        }
        Ok(Ok((plan, request.name.trim().to_string())))
    })
    .await?;
    let (plan, new_name) = outcome.map_err(|e| match e {
        PlanError::Refused((status, message)) => error(status, message),
        PlanError::Stale { message, version } => {
            let mut answer = error(StatusCode::CONFLICT, message);
            answer.body["version"] = json!(version);
            answer
        }
    })?;

    let renamed = new_name != name;
    let venue_error = if write {
        reload_types_and_venues(player, config_path, &new_name, &venues_dir).await
    } else {
        None
    };
    let version = if write && !plan.unchanged {
        content_version(plan.patched.as_bytes())
    } else if plan.new_record {
        UNRECORDED.to_string()
    } else {
        plan.type_version.clone()
    };
    let rename = renamed.then(|| plan.venues.summary(&name, &new_name));
    let mut venue_versions = plan.venues.versions.clone();
    if write {
        for r in &plan.venues.rewrites {
            venue_versions.insert(r.file.clone(), content_version(r.content.as_bytes()));
        }
    }
    Ok(SettingsSaved {
        version,
        rename,
        venue_versions,
        venue_error,
    })
}
