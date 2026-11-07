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

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use super::super::config_io;
use super::super::server::WebUiState;
use super::lighting_api::{
    canonical_project_root, content_version, if_match_version, locate_fixture_type_file,
    project_root, resolve_lighting_dir, validate_lighting_name, validate_lighting_name_to_write,
    venue_error_after_save, DEFAULT_FIXTURE_TYPES_DIR, DEFAULT_VENUES_DIR, VENUE_WRITES,
};
use crate::lighting;
use crate::lighting::fixture_patch::{current_settings, patch_fixture_type, FixtureSettings};
use crate::lighting::types::{Fixture, MovementLimits, StrobeCurve, Venue};

/// The directories a settings request works in.
#[derive(serde::Deserialize, Default)]
pub(super) struct SettingsQuery {
    dir: Option<String>,
    venues_dir: Option<String>,
}

/// A refusal; a 409 carries `conflict` so a client reloads and asks again,
/// as it does for a venue save.
fn error(status: StatusCode, message: String) -> Response {
    let conflict = status == StatusCode::CONFLICT;
    (
        status,
        Json(json!({"error": message, "conflict": conflict})),
    )
        .into_response()
}

/// GET /api/lighting/fixture-types/:name/settings — a GDTF fixture's
/// settings, for the form: `{name, movement, strobe_curve, strobe, version}`. A fixture that is
/// its archive alone (no record) answers its derived name, no limits and the
/// version [`UNRECORDED`]. A native type
/// is a 404 here (it is edited as before).
pub(super) async fn get_settings(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<SettingsQuery>,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        DEFAULT_FIXTURE_TYPES_DIR,
    )?;
    let root = canonical_project_root(&project_root(&state.config_path)?)?;
    let Some(path) = locate_fixture_type_file(&dir, &name).await? else {
        let (dir, wanted, project) = (dir.clone(), name.clone(), root.clone());
        let archive = super::helpers::spawn_blocking_io("read the GDTF library", move || {
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
        return Ok::<_, Response>(
            Json(FixtureSettingsData {
                name,
                movement: MovementLimits::default(),
                strobe_curve: None,
                strobe,
                version: UNRECORDED.to_string(),
            })
            .into_response(),
        );
    };
    let read = path.clone();
    let content = super::helpers::spawn_blocking_io("read fixture type", move || {
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
    Ok::<_, Response>(
        Json(FixtureSettingsData {
            name: settings.name,
            movement: settings.movement,
            strobe_curve: settings.strobe_curve,
            strobe,
            version: content_version(content.as_bytes()),
        })
        .into_response(),
    )
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
    /// Writes the rewrites in place. Each is a venue's own file, so none is
    /// moved or made.
    pub fn write(&self) -> Result<(), String> {
        for rewrite in &self.rewrites {
            config_io::staged_write(&rewrite.path, &rewrite.content)
                .map_err(|e| format!("writing {}: {e}", rewrite.file))?;
        }
        Ok(())
    }

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

/// A refusal of the one rule for type files, as a settings refusal.
fn refused(refused: lighting::writes::Refused) -> Refusal {
    use lighting::writes::Refused;
    let status = match &refused {
        Refused::NotFound { .. } => StatusCode::NOT_FOUND,
        Refused::Unreadable { .. }
        | Refused::Claim(lighting::files::Refusal::Unreadable { .. }) => {
            StatusCode::INTERNAL_SERVER_ERROR
        }
        _ => StatusCode::CONFLICT,
    };
    (status, refused.to_string())
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
) -> Result<Plan, Refusal> {
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
    let mut claimed = Vec::new();
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
        return Err((
            StatusCode::CONFLICT,
            format!(
                "fixture type \"{name}\" changed since you loaded it ({type_file}); it has been \
                 reloaded, so make your change again"
            ),
        ));
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
/// a GDTF's record ([`lighting::files::claim_record_for`]), so refused when
/// the name is declared or another archive's, or its `<stem>.fixture`, in
/// any case, is there or already `claimed` by this save.
fn new_record_for(
    root: &FsPath,
    types_dir: &FsPath,
    library_type: &lighting::library::LibraryType,
    named: &str,
    library: &[lighting::library::LibraryType],
    claimed: &mut Vec<PathBuf>,
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
    let claim = lighting::files::claim_record_for(types_dir, &library_type.archive, named, library)
        .map_err(|r| refused(lighting::writes::Refused::Claim(r)))?;
    let path = claim.path().to_path_buf();
    if claimed.contains(&path) {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "{} would hold two records; choose another name",
                lighting::project_files::display_in(types_dir, &path)
            ),
        ));
    }
    claimed.push(path.clone());
    let text = lighting::import::gdtf_definition(
        &library_type.name,
        &library_type.archive,
        &library_type.file_name,
        &description,
    );
    Ok((path, text))
}

/// The name of the venue the player plays against: the engine's, else the
/// config's.
fn current_venue_name(state: &WebUiState) -> Option<String> {
    if let Some(system) = state
        .player
        .broadcast_handles()
        .and_then(|h| h.lighting_system)
    {
        return system.lock().current_venue().map(str::to_string);
    }
    // codeql[rust/path-injection] config_path is set at startup, not user input.
    let config = crate::config::Player::deserialize(&state.config_path).ok()?;
    config
        .dmx()
        .and_then(|d| d.lighting())
        .and_then(|l| l.current_venue())
        .map(str::to_string)
}

/// After a fixture type's save that may have rewritten venues: one reload of
/// the running engine's types and venues together, so it never sees a
/// venue naming a type it has not read yet; then whether the current venue
/// still loads ([`venue_error_after_save`]).
pub(super) async fn reload_types_and_venues(
    state: &WebUiState,
    type_name: &str,
    venues_dir: &FsPath,
) -> Option<crate::lighting::findings::VenueError> {
    let player = state.player.clone();
    if let Ok(Err(e)) = tokio::task::spawn_blocking(move || player.reload_fixture_types()).await {
        tracing::warn!(fixture_type = %type_name, error = %e, "fixture type saved, but the running engine could not reload it");
    }
    let current = current_venue_name(state)?;
    venue_error_after_save(state, &current, venues_dir).await
}

/// POST /api/lighting/fixture-types/:name/settings — plans (`write: false`)
/// or saves (`write: true`) a GDTF fixture's settings: `{name, movement:
/// {max_pan_speed, max_tilt_speed}}`. The record is written by the one rule
/// for type files: patched where it lives when its file is its alone (409
/// otherwise), a rename moving a file at its old name's own place to the
/// new name's; for a fixture with none, the importer's record is claimed at
/// `<stem>.fixture` and made new. A rename rewrites every venue line that
/// names the type, in every venue file ([`plan_venue_retype`]), or — when
/// any file cannot be rewritten, or a venue using it is not its file's
/// alone — nothing at all. An
/// `If-Match` header carries the record's version ([`UNRECORDED`] for none)
/// and `venue_versions` the venue files' from the plan; either having
/// changed is a 409. A save holds the venue write lock, writes the venue
/// files then the record, and reloads the running engine's types and venues
/// once. The answer says what changed (or would): `{version, rename,
/// venue_versions, venue_error}`.
pub(super) async fn post_settings(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<SettingsQuery>,
    headers: axum::http::HeaderMap,
    Json(request): Json<SettingsRequest>,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let types_dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        DEFAULT_FIXTURE_TYPES_DIR,
    )?;
    let venues_dir = resolve_lighting_dir(
        &state.config_path,
        query.venues_dir.as_deref(),
        DEFAULT_VENUES_DIR,
    )?;
    let root = canonical_project_root(&project_root(&state.config_path)?)?;
    let expected = if_match_version(&headers);
    let write = request.write;
    let (vdir, current) = (venues_dir.clone(), name.clone());
    let outcome = super::helpers::spawn_blocking_io("save fixture settings", move || {
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
            // Each pin and a new record is made new, never over a file made
            // since it was claimed.
            let exists = |path: &FsPath| {
                Ok(Err((
                    StatusCode::CONFLICT,
                    format!(
                        "{} is there now; nothing more was written — make your change again",
                        lighting::project_files::display_in(&types_dir, path)
                    ),
                )))
            };
            for (path, text) in &plan.pins {
                match crate::util::write_new_file(path, text.as_bytes()) {
                    Ok(()) => {}
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return exists(path),
                    Err(e) => return Err(config_io::write_message(path, &e)),
                }
            }
            plan.venues.write()?;
            match lighting::writes::write(&plan.record, &plan.patched) {
                Ok(()) => {}
                Err(lighting::writes::WriteError::Exists(path)) => return exists(&path),
                Err(e) => return Err(e.to_string()),
            }
        }
        Ok(Ok((plan, request.name.trim().to_string())))
    })
    .await?;
    let (plan, new_name) = outcome.map_err(|(status, message)| error(status, message))?;

    let renamed = new_name != name;
    let venue_error = if write {
        reload_types_and_venues(&state, &new_name, &venues_dir).await
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
    Ok::<_, Response>(
        Json(SettingsSaved {
            version,
            rename,
            venue_versions,
            venue_error,
        })
        .into_response(),
    )
}

#[cfg(test)]
mod test {
    use super::super::router;
    use super::super::test_helpers::*;
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    const TYPE_FILE: &str = "# Imported from synth.gdtf.\n# Keep this.\nfixture_type \"Brick\"\n  \
                             from gdtf(\"library/synth.gdtf\")\n{\n}\n";

    /// A project: the synthetic archive, `Brick` in `types/brick.fixture`,
    /// a comment-heavy `.venue` and a `.light` venue using it, and a venue
    /// that does not.
    fn project(root: &FsPath) {
        let library = root.join("library");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(
            library.join("synth.gdtf"),
            crate::testutil::lighting::synthetic_gdtf(),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("types")).unwrap();
        std::fs::write(root.join("types/brick.fixture"), TYPE_FILE).unwrap();
        std::fs::create_dir_all(root.join("venues")).unwrap();
        std::fs::write(
            root.join("venues/house.venue"),
            "# The house rig.\n# Measured 2026-09.\nvenue \"house\" {\n  # front truss\n  \
             fixture \"B1\" Brick mode \"8: RGBS\" @ 1:1 tags [\"wash\"]  # left\n  \
             fixture \"B2\" Brick mode \"Mover 16bit\" @ 1:40\n  \
             fixture \"P\" Other @ 1:100\n  focus \"center\" (0, 1, 1)\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("venues/club.light"),
            "venue \"club\" {\n  fixture \"C1\" Brick mode \"8: RGBS\" @ 2:1\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("venues/other.light"),
            "venue \"other\" {\n  fixture \"X\" Other @ 1:1\n}\n",
        )
        .unwrap();
    }

    async fn post(
        state: WebUiState,
        name: &str,
        body: serde_json::Value,
        if_match: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut request = http::Request::builder()
            .method("POST")
            .uri(format!(
                "/lighting/fixture-types/{}/settings?dir=types&venues_dir=venues",
                encoded(name)
            ))
            .header("content-type", "application/json");
        if let Some(v) = if_match {
            request = request.header("if-match", v);
        }
        let response = router()
            .with_state(state)
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let text = response_body(response).await;
        (status, serde_json::from_str(&text).unwrap_or_default())
    }

    /// A fixture name as a path segment.
    fn encoded(name: &str) -> String {
        name.replace(' ', "%20")
            .replace('(', "%28")
            .replace(')', "%29")
    }

    fn read(root: &FsPath, rel: &str) -> String {
        std::fs::read_to_string(root.join(rel)).unwrap()
    }

    #[tokio::test]
    async fn the_form_reads_the_settings_and_a_native_type_has_none() {
        let (state, dir) = test_state();
        project(dir.path());
        std::fs::write(
            dir.path().join("types/par.light"),
            "fixture_type \"Par\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n",
        )
        .unwrap();
        let get = |name: &str| {
            router().with_state(state.clone()).oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/fixture-types/{name}/settings?dir=types"))
                    .body(Body::empty())
                    .unwrap(),
            )
        };
        let response = get("Brick").await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
        assert_eq!(body["name"], "Brick");
        assert!(body["movement"]["max_pan_speed"].is_null(), "{body}");
        assert_eq!(body["version"], content_version(TYPE_FILE.as_bytes()));
        assert_eq!(get("Par").await.unwrap().status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_rename_rewrites_every_venue_line_and_keeps_every_comment() {
        let (state, dir) = test_state();
        project(dir.path());
        let root = dir.path();
        let before_house = read(root, "venues/house.venue");
        let before_other = read(root, "venues/other.light");
        let body = json!({"name": "PixelBrick", "write": false});

        // The plan writes nothing and says what it would touch.
        let (status, plan) = post(state.clone(), "Brick", body.clone(), None).await;
        assert_eq!(status, StatusCode::OK, "{plan}");
        assert_eq!(plan["rename"]["lines"], 3, "{plan}");
        assert_eq!(
            plan["rename"]["venues"],
            json!([
                {"venue": "club", "file": "club.light", "lines": 1},
                {"venue": "house", "file": "house.venue", "lines": 2},
            ])
        );
        assert_eq!(read(root, "types/brick.fixture"), TYPE_FILE);

        let mut save = body.clone();
        save["write"] = json!(true);
        save["venue_versions"] = plan["venue_versions"].clone();
        let (status, saved) = post(
            state,
            "Brick",
            save,
            Some(&content_version(TYPE_FILE.as_bytes())),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert!(saved["venue_error"].is_null(), "{saved}");
        // The record was at its own name's place, so it moves to the new
        // name's, its comments kept.
        assert_eq!(
            read(root, "types/pixelbrick.fixture"),
            TYPE_FILE.replace("\"Brick\"", "\"PixelBrick\"")
        );
        assert!(!root.join("types/brick.fixture").exists());
        let house = read(root, "venues/house.venue");
        assert_eq!(
            house,
            before_house
                .replace("\"B1\" Brick mode", "\"B1\" PixelBrick mode")
                .replace("\"B2\" Brick mode", "\"B2\" PixelBrick mode"),
            "only the type changes, comments and layout stay"
        );
        assert!(read(root, "venues/club.light")
            .contains("fixture \"C1\" PixelBrick mode \"8: RGBS\" @ 2:1"));
        assert_eq!(read(root, "venues/other.light"), before_other);
    }

    #[tokio::test]
    async fn a_venue_file_that_does_not_parse_blocks_a_rename_and_nothing_is_written() {
        let (state, dir) = test_state();
        project(dir.path());
        let root = dir.path();
        std::fs::write(root.join("venues/broken.venue"), "venue \"b\" {{{").unwrap();
        let house = read(root, "venues/house.venue");
        let (status, body) = post(
            state,
            "Brick",
            json!({"name": "PixelBrick", "write": true}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("broken.venue"),
            "{body}"
        );
        assert_eq!(read(root, "types/brick.fixture"), TYPE_FILE);
        assert_eq!(read(root, "venues/house.venue"), house);
    }

    #[tokio::test]
    async fn stale_versions_are_refused_before_anything_is_written() {
        let (state, dir) = test_state();
        project(dir.path());
        let root = dir.path();
        let body = json!({"name": "PixelBrick", "write": true});
        let (status, _) = post(state.clone(), "Brick", body.clone(), Some("not-it")).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let mut stale = body.clone();
        stale["venue_versions"] = json!({"house.venue": "old"});
        let (status, refused) = post(state, "Brick", stale, None).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(refused["error"].as_str().unwrap().contains("house.venue"));
        assert_eq!(read(root, "types/brick.fixture"), TYPE_FILE);
    }

    #[tokio::test]
    async fn a_rename_onto_another_type_is_refused() {
        let (state, dir) = test_state();
        project(dir.path());
        std::fs::write(
            dir.path().join("types/other.light"),
            "fixture_type \"Other\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n",
        )
        .unwrap();
        let (status, body) = post(
            state,
            "Brick",
            json!({"name": "Other", "write": false}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(
            body["error"],
            "fixture type \"Other\" is already declared in other.light; choose another name"
        );
    }

    #[tokio::test]
    async fn movement_limits_are_patched_into_the_record() {
        let (state, dir) = test_state();
        project(dir.path());
        let (status, saved) = post(
            state,
            "Brick",
            json!({"name": "Brick", "write": true,
                   "movement": {"max_pan_speed": 240, "max_tilt_speed": null}}),
            Some(&content_version(TYPE_FILE.as_bytes())),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert!(saved["rename"].is_null(), "{saved}");
        assert_eq!(
            read(dir.path(), "types/brick.fixture"),
            "# Imported from synth.gdtf.\n# Keep this.\nfixture_type \"Brick\"\n  \
             from gdtf(\"library/synth.gdtf\")\n{\n  \
             movement { max_pan_speed: 240deg/s }\n}\n"
        );
    }

    /// A project whose fixture is its archive alone: `lighting/library/` in
    /// the config's directory, no record, and a venue using its derived
    /// name. The archive is copied twice under two file names so one name
    /// is shared — "Synth Brick" and "Synth Brick (twin)".
    fn library_project(root: &FsPath) {
        let library = root.join("lighting/library");
        std::fs::create_dir_all(&library).unwrap();
        let archive = crate::testutil::lighting::synthetic_gdtf();
        std::fs::write(library.join("synth.gdtf"), &archive).unwrap();
        std::fs::write(library.join("twin.gdtf"), &archive).unwrap();
        std::fs::create_dir_all(root.join("types")).unwrap();
        std::fs::create_dir_all(root.join("venues")).unwrap();
        std::fs::write(
            root.join("venues/house.venue"),
            "venue \"house\" {\n  fixture \"B1\" \"Synth Brick\" mode \"8: RGBS\" @ 1:1\n}\n",
        )
        .unwrap();
    }

    async fn get_settings(state: WebUiState, name: &str) -> (StatusCode, serde_json::Value) {
        let response = router()
            .with_state(state)
            .oneshot(
                http::Request::builder()
                    .uri(format!(
                        "/lighting/fixture-types/{}/settings?dir=types",
                        encoded(name)
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        (
            status,
            serde_json::from_str(&response_body(response).await).unwrap_or_default(),
        )
    }

    #[tokio::test]
    async fn an_unrecorded_fixture_answers_its_derived_name_and_a_save_writes_its_record() {
        let (state, dir) = test_state();
        let root = dir.path();
        library_project(root);
        let (status, body) = get_settings(state.clone(), "Synth Brick").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(
            body,
            json!({
                "name": "Synth Brick",
                "movement": {"max_pan_speed": null, "max_tilt_speed": null},
                "strobe_curve": null,
                "strobe": {"steps": 0, "automatic": "declared"},
                "version": "unrecorded",
            })
        );

        // A plan writes nothing, and nothing to change writes nothing.
        let (status, plan) = post(
            state.clone(),
            "Synth Brick",
            json!({"name": "Synth Brick", "write": true}),
            Some("unrecorded"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{plan}");
        assert_eq!(plan["version"], "unrecorded");
        assert_eq!(std::fs::read_dir(root.join("types")).unwrap().count(), 0);

        // A change writes the record the importer would have, then patches it.
        let (status, saved) = post(
            state.clone(),
            "Synth Brick",
            json!({"name": "Synth Brick", "write": true,
                   "movement": {"max_pan_speed": 120, "max_tilt_speed": null}}),
            Some("unrecorded"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        let record = read(root, "types/synth_brick.fixture");
        assert!(record.starts_with("# Imported from synth.gdtf"), "{record}");
        assert!(
            record.contains(
                "fixture_type \"Synth Brick\"\n  from gdtf(\"lighting/library/synth.gdtf\")"
            ),
            "{record}"
        );
        assert!(record.contains("max_pan_speed: 120deg/s"), "{record}");
        assert_eq!(saved["version"], content_version(record.as_bytes()));

        // "No record yet" is stale once there is one.
        let (status, _) = post(
            state.clone(),
            "Synth Brick",
            json!({"name": "Synth Brick", "write": true}),
            Some("unrecorded"),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        let (_, body) = get_settings(state, "Synth Brick").await;
        assert_ne!(body["version"], "unrecorded");
        assert_eq!(body["movement"]["max_pan_speed"], 120.0);
    }

    #[tokio::test]
    async fn the_strobe_curve_is_saved_in_the_record_and_read_back() {
        let (state, dir) = test_state();
        let root = dir.path();
        library_project(root);
        // Automatic is the default: saving it writes no record.
        let (status, _) = post(
            state.clone(),
            "Synth Brick",
            json!({"name": "Synth Brick", "write": true, "strobe_curve": null}),
            Some("unrecorded"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(std::fs::read_dir(root.join("types")).unwrap().count(), 0);

        let (status, saved) = post(
            state.clone(),
            "Synth Brick",
            json!({"name": "Synth Brick", "write": true, "strobe_curve": "linear"}),
            Some("unrecorded"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        let record = read(root, "types/synth_brick.fixture");
        assert!(record.contains("  strobe_curve: linear\n"), "{record}");
        let (_, body) = get_settings(state.clone(), "Synth Brick").await;
        assert_eq!(body["strobe_curve"], "linear");
        let version = body["version"].as_str().unwrap().to_string();

        // A stale version is refused and writes nothing.
        let (status, _) = post(
            state.clone(),
            "Synth Brick",
            json!({"name": "Synth Brick", "write": true, "strobe_curve": "period"}),
            Some("stale"),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(read(root, "types/synth_brick.fixture"), record);

        // Back to automatic: the statement goes; movement limits are kept
        // apart.
        let (status, _) = post(
            state.clone(),
            "Synth Brick",
            json!({"name": "Synth Brick", "write": true, "strobe_curve": null,
                   "movement": {"max_pan_speed": 90, "max_tilt_speed": null}}),
            Some(&version),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let record = read(root, "types/synth_brick.fixture");
        assert!(!record.contains("strobe_curve"), "{record}");
        assert!(record.contains("max_pan_speed: 90deg/s"), "{record}");
        let (_, body) = get_settings(state.clone(), "Synth Brick").await;
        assert_eq!(body["strobe_curve"], serde_json::Value::Null);
        assert_eq!(body["strobe"]["automatic"], "declared");
        // An unknown curve is refused.
        let (status, _) = post(
            state,
            "Synth Brick",
            json!({"name": "Synth Brick", "write": true, "strobe_curve": "log"}),
            None,
        )
        .await;
        assert!(status.is_client_error(), "{status}");
    }

    #[tokio::test]
    async fn renaming_an_unrecorded_fixture_pins_the_one_that_shares_its_name() {
        let (state, dir) = test_state();
        let root = dir.path();
        library_project(root);
        let (status, _) = get_settings(state.clone(), "Synth Brick (twin)").await;
        assert_eq!(status, StatusCode::OK);
        let (status, saved) = post(
            state,
            "Synth Brick",
            json!({"name": "House Brick", "write": true}),
            Some("unrecorded"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["rename"]["lines"], 1, "{saved}");
        assert!(read(root, "venues/house.venue")
            .contains("fixture \"B1\" \"House Brick\" mode \"8: RGBS\" @ 1:1"));
        let record = read(root, "types/house_brick.fixture");
        assert!(
            record.contains(
                "fixture_type \"House Brick\"\n  from gdtf(\"lighting/library/synth.gdtf\")"
            ),
            "{record}"
        );
        // Without its pin, the twin would take the freed name on the next load.
        let pin = read(root, "types/synth_brick_twin.fixture");
        assert!(
            pin.contains(
                "fixture_type \"Synth Brick (twin)\"\n  from gdtf(\"lighting/library/twin.gdtf\")"
            ),
            "{pin}"
        );
        let library = crate::lighting::library::unrecorded_types(root, Some(&root.join("types")));
        assert!(library.types.is_empty(), "{:?}", library.types);
    }

    #[tokio::test]
    async fn a_rename_onto_a_library_fixture_s_name_is_refused() {
        let (state, dir) = test_state();
        library_project(dir.path());
        let (status, body) = post(
            state,
            "Synth Brick",
            json!({"name": "Synth Brick (twin)", "write": false}),
            Some("unrecorded"),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
    }

    const OTHER_TYPE: &str =
        "fixture_type \"Old\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n";

    /// The files in `types`, by name.
    fn type_files(root: &FsPath) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(root.join("types"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[tokio::test]
    async fn a_pin_whose_file_is_taken_in_any_case_is_refused_not_suffixed() {
        for taken in ["synth_brick.fixture", "Synth_Brick.fixture"] {
            let (state, dir) = test_state();
            let root = dir.path();
            library_project(root);
            std::fs::write(root.join("types").join(taken), OTHER_TYPE).unwrap();

            let (status, body) = post(
                state,
                "Synth Brick",
                json!({"name": "Synth Brick", "write": true,
                       "movement": {"max_pan_speed": 120, "max_tilt_speed": null}}),
                Some("unrecorded"),
            )
            .await;

            assert_eq!(status, StatusCode::CONFLICT, "{body}");
            assert_eq!(
                body["error"],
                format!("{taken} is taken by \"Old\"; choose another name")
            );
            assert_eq!(type_files(root), [taken], "no synth_brick_2.fixture");
        }
    }

    #[tokio::test]
    async fn the_settings_of_a_type_whose_file_is_not_its_alone_are_read_only() {
        let (state, dir) = test_state();
        project(dir.path());
        let shared = format!("{TYPE_FILE}{OTHER_TYPE}");
        std::fs::write(dir.path().join("types/brick.fixture"), &shared).unwrap();

        let (status, body) = post(
            state,
            "Brick",
            json!({"name": "Brick", "write": true,
                   "movement": {"max_pan_speed": 240, "max_tilt_speed": null}}),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(
            body["error"],
            "Defined in brick.fixture alongside fixture type \"Old\"; edit that file by hand."
        );
        assert_eq!(read(dir.path(), "types/brick.fixture"), shared);
    }

    #[tokio::test]
    async fn a_rename_is_refused_when_a_venue_using_it_is_not_its_file_s_alone() {
        let (state, dir) = test_state();
        project(dir.path());
        let root = dir.path();
        let shared = "venue \"arena\" {\n  fixture \"A1\" Brick mode \"8: RGBS\" @ 3:1\n}\n\
                      venue \"annex\" {\n}\n";
        std::fs::write(root.join("venues/all.venue"), shared).unwrap();
        let house = read(root, "venues/house.venue");

        let (status, body) = post(
            state.clone(),
            "Brick",
            json!({"name": "PixelBrick", "write": true}),
            None,
        )
        .await;

        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert_eq!(
            body["error"],
            "fixture type \"Brick\" cannot be renamed, so nothing was written; venue \"arena\" \
             in all.venue uses it: Defined in all.venue alongside venue \"annex\"; edit that file \
             by hand."
        );
        assert_eq!(read(root, "venues/all.venue"), shared);
        assert_eq!(read(root, "venues/house.venue"), house);
        assert_eq!(read(root, "types/brick.fixture"), TYPE_FILE);
        assert_eq!(type_files(root), ["brick.fixture"]);

        // A settings change that renames nothing touches no venue, so the
        // shared file does not stand in its way.
        let (status, body) = post(
            state,
            "Brick",
            json!({"name": "Brick", "write": true,
                   "movement": {"max_pan_speed": 240, "max_tilt_speed": null}}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(read(root, "venues/all.venue"), shared);
    }

    /// The harness's record: "Astera-PixelBrick" in `astera_pixelbrick.fixture`,
    /// which is not its own name's file (`astera-pixelbrick.fixture`).
    const ASTERA: &str = "# Imported from synth.gdtf.\nfixture_type \"Astera-PixelBrick\"\n  \
                          from gdtf(\"library/synth.gdtf\")\n{\n}\n";

    #[tokio::test]
    async fn the_harness_record_is_edited_and_renamed_where_it_lives() {
        let (state, dir) = test_state();
        project(dir.path());
        let root = dir.path();
        std::fs::remove_file(root.join("types/brick.fixture")).unwrap();
        std::fs::write(root.join("types/astera_pixelbrick.fixture"), ASTERA).unwrap();
        std::fs::write(
            root.join("venues/house.venue"),
            "venue \"house\" {\n  fixture \"B1\" \"Astera-PixelBrick\" mode \"8: RGBS\" @ 1:1\n}\n",
        )
        .unwrap();

        let (status, saved) = post(
            state.clone(),
            "Astera-PixelBrick",
            json!({"name": "Astera-PixelBrick", "write": true,
                   "movement": {"max_pan_speed": 240, "max_tilt_speed": null}}),
            Some(&content_version(ASTERA.as_bytes())),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        let edited = read(root, "types/astera_pixelbrick.fixture");
        assert!(edited.contains("max_pan_speed: 240deg/s"), "{edited}");

        let (status, saved) = post(
            state,
            "Astera-PixelBrick",
            json!({"name": "PB15", "write": true,
                   "movement": {"max_pan_speed": 240, "max_tilt_speed": null}}),
            Some(&content_version(edited.as_bytes())),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(type_files(root), ["astera_pixelbrick.fixture"]);
        assert!(read(root, "types/astera_pixelbrick.fixture").contains("fixture_type \"PB15\""));
        assert!(read(root, "venues/house.venue").contains("fixture \"B1\" PB15 mode"));
    }
}
