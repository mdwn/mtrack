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

use crate::lighting::effects::LayerSet;
use crate::webui::wire::fixture_types::{
    BeamBrief, FixtureTypeDeleted, FixtureTypeEntry, FixtureTypeGdtf, FixtureTypeList, GdtfBeam,
    GdtfInspection, GdtfMode, GdtfSummary, LightingFileError, ModeUse, SceneFixture, StrobeRange,
    TypeVenue, TypeVenueFixture, VenueDetail, VenueList, VenueScene,
};
use crate::webui::wire::messages::FixturePlacement;
use crate::webui::wire::preview::{
    LightFileInfo, LightingGroup, LightingGroups, LightingValidation,
};
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use super::super::config_io;
use super::super::safe_path::{SafePathError, VerifiedRoot};
use super::super::server::WebUiState;
use super::lighting_writes;
use crate::lighting;
use crate::lighting::files::Kind;

/// The canonical project root the lighting endpoints write into.
#[allow(clippy::result_large_err)]
pub(super) fn project_root(
    config_path: &std::path::Path,
) -> Result<std::path::PathBuf, axum::response::Response> {
    let canonical = config_path.canonicalize().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Failed to resolve config path"})),
        )
            .into_response()
    })?;
    Ok(canonical
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf())
}

/// Pulls the first uploaded file out of a multipart body: (file name, bytes).
async fn first_multipart_file(
    multipart: &mut axum::extract::Multipart,
) -> Result<(String, axum::body::Bytes), axum::response::Response> {
    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Failed to read multipart field: {}", e)})),
        )
            .into_response()
    })? {
        let Some(filename) = field.file_name().map(|f| f.to_string()) else {
            continue;
        };
        // The name becomes a library path component; keep only the final
        // component of whatever the browser sent.
        let filename = std::path::Path::new(&filename)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        if filename.is_empty() {
            continue;
        }
        let bytes = field.bytes().await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Failed to read file data: {}", e)})),
            )
                .into_response()
        })?;
        return Ok((filename, bytes));
    }
    Err((
        StatusCode::BAD_REQUEST,
        Json(json!({"error": "No file uploaded"})),
    )
        .into_response())
}

// The archive's fixture name as a fixture type name: the importer's rule,
// so the name the picker suggests is the name an import writes.
use lighting::import::suggested_type_name;

/// A GDTF archive's modes, as its fixture page shows them: each mode's
/// footprint, what a show can do in it (capabilities, cells, strobe range),
/// its channel map and the distiller's warnings, or the reason it cannot be
/// imported.
fn inspect_description(description: &lighting::gdtf::Description) -> GdtfInspection {
    // Distilling every mode is a few milliseconds each (design 12.2), so the
    // picker can say what each one lets a show do before anything is written.
    let suggested = suggested_type_name(&description.name);
    let summaries = lighting::gdtf::mode_summaries(description);
    let modes = summaries
        .iter()
        .enumerate()
        .map(|(index, summary)| {
            let mut mode = GdtfMode {
                name: summary.name.clone(),
                footprint: summary.footprint,
                capabilities: None,
                wants: None,
                cells: None,
                strobe_range: None,
                channels: None,
                refused: None,
            };
            // The importer finds a mode by name and takes the first match, so
            // a repeat could only ever import its namesake.
            let repeats = summaries[..index].iter().any(|m| m.name == summary.name);
            let distilled = if repeats {
                Err("another mode has this name; an import would take the first".to_string())
            } else {
                lighting::gdtf::distill(description, &summary.name, &suggested)
                    .map_err(|e| e.to_string())
            };
            match distilled {
                Ok(distilled) => {
                    let fixture_type = &distilled.fixture_type;
                    let mut info = lighting::effects::FixtureInfo::new(
                        suggested.clone(),
                        1,
                        1,
                        suggested.clone(),
                        fixture_type.channels().clone(),
                        None,
                    );
                    info.cells = fixture_type.cells().to_vec();
                    let fit = lighting::fit::FitFixture::from_info(&info, &[]);
                    mode.capabilities = Some(fit.capability_names());
                    mode.wants = Some(fit.wants());
                    mode.cells = Some(fixture_type.cells().len());
                    mode.strobe_range =
                        fixture_type
                            .max_strobe_frequency()
                            .map(|max_hz| StrobeRange {
                                min_hz: fixture_type.min_strobe_frequency(),
                                max_hz,
                            });
                    let mut channels: Vec<(u16, String)> = fixture_type
                        .channels()
                        .iter()
                        .map(|(name, offset)| (*offset, name.clone()))
                        .collect();
                    channels.sort();
                    mode.channels = Some(channels);
                }
                Err(reason) => mode.refused = Some(reason),
            }
            mode
        })
        .collect();
    GdtfInspection { modes }
}

/// Query parameters for the GDTF import endpoint.
#[derive(serde::Deserialize)]
pub(super) struct GdtfImportQuery {
    /// A name for the fixture; given, a record pins it.
    name: Option<String>,
}

/// POST /api/lighting/gdtf/import[?name=...] — imports an uploaded GDTF
/// archive through the shared importer (the same one behind the CLI and
/// MCP): a copy into lighting/library/, where it is a fixture (design §22),
/// and a warmed rig for its page. One step: a venue fixture can use any of
/// its modes straight away. Answers the import report
/// (`already_imported` when the same archive was imported before and
/// nothing was written; `renamed_from` when the archive's fixture name was
/// taken). A different archive of the same file name already in the library,
/// or a record name or file that is taken, is a 409; anything else refused
/// is a 400.
pub(super) async fn import_gdtf(
    State(state): State<WebUiState>,
    Query(query): Query<GdtfImportQuery>,
    mut multipart: axum::extract::Multipart,
) -> impl IntoResponse {
    let (filename, bytes) = first_multipart_file(&mut multipart).await?;
    let project = project_root(&state.config_path)?;
    // Where the engine reads fixture types on this host, as the CLI and MCP
    // import into (#673): its name check and a pinned record go there.
    let types_dir = lighting_dir(&state.config_path, None, LightingDir::FixtureTypes)?;
    // The inner Result survives spawn_blocking_io so an import refusal (bad
    // archive, name collision, archive conflict) surfaces as the caller error
    // it is, not a 500.
    let report = super::helpers::spawn_blocking_io("import gdtf", move || {
        // A pinned name writes a fixture type record, in turn with the
        // other fixture type and venue saves (#578).
        let _guard = VENUE_WRITES.blocking_lock();
        Ok::<_, String>(
            lighting::import::import_gdtf_bytes(
                &bytes,
                &filename,
                query.name.as_deref(),
                &project,
                &types_dir,
            )
            .map_err(|e| lighting_writes::import_error(e.as_ref())),
        )
    })
    .await?
    .map_err(|(status, body)| (status, Json(body)).into_response())?;
    Ok::<_, axum::response::Response>((StatusCode::OK, Json(report)).into_response())
}

/// Recursively finds light show files (`.light`, in any case:
/// `songs::FileKind::of`) under a directory.
pub(crate) fn find_light_files(
    base: &std::path::Path,
    dir: &std::path::Path,
    results: &mut Vec<LightFileInfo>,
) -> Result<(), std::io::Error> {
    // codeql[rust/path-injection] dir is always state.songs_path(), set at startup.
    if !dir.is_dir() {
        return Ok(());
    }
    // codeql[rust/path-injection] dir is always state.songs_path(), set at startup.
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            find_light_files(base, &path, results)?;
        } else if crate::songs::FileKind::is_lighting(&path) {
            let relative = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .to_string();
            results.push(LightFileInfo {
                name,
                path: relative,
            });
        }
    }
    Ok(())
}

/// The 404 for a lighting file `name` that is not there.
fn lighting_file_not_found(name: &str) -> axum::response::Response {
    (
        StatusCode::NOT_FOUND,
        Json(json!({"error": format!("Lighting file not found: {name}")})),
    )
        .into_response()
}

/// GET /api/lighting/:name — returns the raw DSL content of a .light file.
///
/// The `name` parameter is the relative path within the songs directory (as returned by
/// the listing endpoint). Path traversal is guarded.
pub(super) async fn get_lighting_file(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    use super::super::safe_path::{SafePath, VerifiedRoot};

    let root = match VerifiedRoot::new(&state.songs_path()) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    let safe = match SafePath::resolve(&state.songs_path().join(&name), &root) {
        Ok(p) => p,
        Err(_) => {
            return lighting_file_not_found(&name);
        }
    };

    let path = safe.as_path().to_path_buf();
    match super::helpers::spawn_blocking_io("read lighting file", move || {
        crate::util::read_to_string(&path)
    })
    .await
    {
        Ok(content) => (
            StatusCode::OK,
            [("content-type", "text/plain; charset=utf-8")],
            content,
        )
            .into_response(),
        Err(e) => e,
    }
}

/// The one songs-root rule for a song's light show, written, deleted or
/// validated, by the web UI and MCP alike: the folder `path` is in is
/// inside the songs root `songs_root`, canonically, as
/// [`SafePath::resolve`](super::super::safe_path::SafePath::resolve) holds
/// every path the lighting endpoints take. A folder not there yet is held
/// to it by its nearest part that is there, canonicalized (a symlinked
/// `lighting/` pointing outside is outside), the rest of it plain names
/// (no `..`, and no link that leads nowhere, which a write would follow).
///
/// Answers the folder resolved, joined with `path`'s own name, which is
/// not resolved: a link there is the link, as the song loader names it,
/// so its `include`s resolve where the loader resolves them, and a delete
/// removes the link, never what it leads to. A write follows it, so
/// [`light_show_to_write`] holds what it leads to as well. 403 outside
/// the root.
#[allow(clippy::result_large_err)]
pub(crate) fn light_show_in_songs_root(
    root: &VerifiedRoot,
    path: &std::path::Path,
) -> Result<std::path::PathBuf, axum::response::Response> {
    use super::super::safe_path::{SafePath, SafePathError};

    let outside = || SafePathError::OutsideRoot.into_response();
    let (Some(folder), Some(file_name)) = (path.parent(), path.file_name()) else {
        return Err(outside());
    };
    let mut missing = Vec::new();
    let mut existing = folder;
    loop {
        match SafePath::resolve(existing, root) {
            Ok(safe) => {
                let mut resolved = safe.as_path().to_path_buf();
                resolved.extend(missing.iter().rev());
                resolved.push(file_name);
                return Ok(resolved);
            }
            Err(SafePathError::NotFound) => {
                // A link that leads nowhere: what a write made through it
                // would land on is not known to be inside.
                if std::fs::symlink_metadata(existing).is_ok() {
                    return Err(outside());
                }
                match (existing.parent(), existing.file_name()) {
                    (Some(parent), Some(name)) => {
                        missing.push(name.to_os_string());
                        existing = parent;
                    }
                    // A `..` past a part that is not there, or nothing
                    // there at all: nowhere that can be shown to be inside.
                    _ => return Err(outside()),
                }
            }
            Err(e) => return Err(e.into_response()),
        }
    }
}

/// Where a write of the light show `path` lands, held to the songs root:
/// [`light_show_in_songs_root`], and, the file being a link, which the
/// write follows, what it leads to inside the songs root too (403
/// otherwise). A link that leads nowhere is refused: the write would make
/// its target, wherever that is. Both the name written and the file a
/// link leads to must be `.light` files (400 otherwise): a show is never
/// written over a song's config or anything else.
#[allow(clippy::result_large_err)]
fn light_show_to_write(
    root: &VerifiedRoot,
    path: &std::path::Path,
) -> Result<std::path::PathBuf, axum::response::Response> {
    use super::super::safe_path::{SafePath, SafePathError};

    let not_a_show = || {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Can only write .light files"})),
        )
            .into_response()
    };
    let is_show = crate::songs::FileKind::is_lighting;
    let path = light_show_in_songs_root(root, path)?;
    if !is_show(&path) {
        return Err(not_a_show());
    }
    if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        let target = SafePath::resolve(&path, root).map_err(|e| match e {
            SafePathError::NotFound => SafePathError::OutsideRoot.into_response(),
            e => e.into_response(),
        })?;
        if !is_show(target.as_path()) {
            return Err(not_a_show());
        }
    }
    Ok(path)
}

/// Refuses a light show path, resolved as the handler resolves it, that is
/// inside the venues or fixture types directory the engine reads on this
/// host: those files are written by name, by the one rule for them
/// ([`lighting_writes`]), never as a song's light show. Compared by
/// identity ([`crate::util::is_within_dir`]), so no spelling (`..`, a
/// symlink, another case) gets around it.
async fn refuse_venue_or_type_file(
    config_path: &std::path::Path,
    path: &std::path::Path,
    name: &str,
) -> Result<(), lighting_writes::Answer> {
    let config_path = config_path.to_path_buf();
    let project = crate::util::project_dir_of(&config_path);
    let (path, name) = (path.to_path_buf(), name.to_string());
    let refused = tokio::task::spawn_blocking(move || {
        // A config that does not load is a 500, as at every endpoint that
        // reads these directories: the defaults may not be this host's.
        let dirs = configured_lighting_dirs(&config_path)?;
        Ok::<_, String>(
            [
                (Kind::Venue, dirs.venues),
                (Kind::FixtureType, dirs.fixture_types),
            ]
            .into_iter()
            .find(|(_, dir)| crate::util::is_within_dir(&path, &project.join(dir))),
        )
    })
    .await
    .unwrap_or(Ok(None))
    .map_err(|e| lighting_writes::Answer::new(StatusCode::INTERNAL_SERVER_ERROR, e))?;
    match refused {
        Some((kind, dir)) => Err(lighting_writes::Answer::bad(format!(
            "{name} is in the {noun}s directory ({dir}): a {noun} is saved by name, on the \
             Lighting page or with MCP's {noun} tools; this endpoint is for song light shows",
            noun = kind.noun()
        ))),
        None => Ok(()),
    }
}

/// PUT /api/lighting/:name — validates and atomically writes a lighting DSL
/// file: a song's light show ([`save_light_show`]).
pub(super) async fn put_lighting_file(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    body: String,
) -> impl IntoResponse {
    // A new file goes in a folder already there, as the editor saves it.
    let songs_root = state.songs_path();
    let root = match VerifiedRoot::new(&songs_root) {
        Ok(root) => root,
        Err(e) => return e.into_response(),
    };
    let verified_path = match light_show_in_songs_root(&root, &songs_root.join(&name)) {
        Ok(path) if path.parent().is_some_and(std::path::Path::is_dir) => path,
        Ok(_) => return lighting_file_not_found(&name),
        Err(e) => return e,
    };

    // Held from reading the song's tempo: a song save changing it reloads
    // the songs under this lock.
    let _writing = super::songs_api::SONG_WRITES.lock().await;
    let tempo = song_tempo_for_path(&state, &verified_path);
    match save_light_show(
        &state.config_path,
        &root,
        &verified_path,
        &name,
        body,
        tempo.as_ref(),
    )
    .await
    {
        Ok(()) => (StatusCode::OK, Json(json!({"status": "saved"}))).into_response(),
        Err(e) => e,
    }
}

/// Writes a song's light show `body` to `path` (shown as `name`), by the
/// one rule of the web UI's light show save and MCP's
/// `write_song_lighting` and `patch_song_lighting`: inside `songs_root`
/// ([`light_show_to_write`]); never a file in the venues or fixture
/// types directory ([`refuse_venue_or_type_file`]); parsed first (400 with
/// its `errors`, each `Invalid light show DSL: ...`, as the venue and
/// fixture type saves say it) against `tempo`, the tempo its
/// song loads it with (a show inheriting its song's tempo is valid content
/// that a tempo-less parse rejects), its `include`s resolved where it is
/// written and kept inside `root`; its directory made only then. The
/// caller holds [`super::songs_api::SONG_WRITES`].
pub(crate) async fn save_light_show(
    config_path: &std::path::Path,
    root: &VerifiedRoot,
    path: &std::path::Path,
    name: &str,
    body: String,
    tempo: Option<&crate::tempo::TempoMap>,
) -> Result<(), axum::response::Response> {
    // The venues and fixture types refusal first: it says what to do
    // instead, where the songs-root one, for those directories outside the
    // songs root, says only no.
    refuse_venue_or_type_file(config_path, path, name)
        .await
        .map_err(IntoResponse::into_response)?;
    let path = &light_show_to_write(root, path)?;
    if let Err(errors) =
        config_io::validate_light_show(&body, Some(path), Some(root.as_path()), tempo)
    {
        let errors: Vec<String> = errors
            .into_iter()
            .map(|e| format!("Invalid light show DSL: {e}"))
            .collect();
        return Err((StatusCode::BAD_REQUEST, Json(json!({"errors": errors}))).into_response());
    }

    let vp = path.to_path_buf();
    super::helpers::spawn_blocking_io("write lighting file", move || {
        if let Some(parent) = vp.parent() {
            crate::util::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        config_io::staged_write(&vp, &body)
    })
    .await
}

/// DELETE /api/lighting/:name — deletes a song's `.light` file from disk,
/// by the one rule of MCP's `delete_song_lighting` too
/// ([`check_light_show_delete`]): a link is removed, never what it leads
/// to.
pub(super) async fn delete_lighting_file(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let songs_root = state.songs_path();
    let root = match VerifiedRoot::new(&songs_root) {
        Ok(root) => root,
        Err(e) => return e.into_response(),
    };
    // In turn with the file's writes: an MCP patch reading it before the
    // delete would write it back after.
    let _writing = super::songs_api::SONG_WRITES.lock().await;
    let path =
        match check_light_show_delete(&state.config_path, &root, &songs_root.join(&name), &name)
            .await
        {
            Ok(path) => path,
            Err(refused) => return refused,
        };
    match remove_light_show(&songs_root, &path).await {
        Ok(()) => (StatusCode::OK, Json(json!({"status": "deleted"}))).into_response(),
        Err(e) => e,
    }
}

/// Whether the file at `path` (shown as `name`) may be deleted as a song's
/// light show, by the one rule of the web UI's delete and MCP's
/// `delete_song_lighting`: a `.light` file (400 otherwise), never one in
/// the venues or fixture types directory ([`refuse_venue_or_type_file`]),
/// in a folder inside `songs_root` ([`light_show_in_songs_root`]), and
/// there (404 otherwise). Answers the file to remove: a link itself, not
/// what it leads to, whose name is the one checked.
pub(crate) async fn check_light_show_delete(
    config_path: &std::path::Path,
    root: &VerifiedRoot,
    path: &std::path::Path,
    name: &str,
) -> Result<std::path::PathBuf, axum::response::Response> {
    // The name checked is the one removed: the resolved path keeps it.
    if !crate::songs::FileKind::is_lighting(path) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Can only delete .light files"})),
        )
            .into_response());
    }
    // As a save: the venues and fixture types refusal, which says what to
    // do instead, before the songs root's. Of where the file is, its
    // folder: a link there is removed, not what it leads to, so where it
    // leads does not matter.
    let folder = path.parent().unwrap_or(path);
    refuse_venue_or_type_file(config_path, folder, name)
        .await
        .map_err(IntoResponse::into_response)?;
    let path = light_show_in_songs_root(root, path)?;
    if std::fs::symlink_metadata(&path).is_err() {
        return Err(lighting_file_not_found(name));
    }
    Ok(path)
}

/// Removes a song's light show file, [`check_light_show_delete`] first,
/// off the async runtime: the file itself, a link and not what it leads
/// to, held to `songs_root` again ([`light_show_in_songs_root`]), as it
/// is checked some time before.
pub(crate) async fn remove_light_show(
    songs_root: &std::path::Path,
    path: &std::path::Path,
) -> Result<(), axum::response::Response> {
    let root = VerifiedRoot::new(songs_root).map_err(SafePathError::into_response)?;
    // codeql[rust/path-injection] the folder is verified via SafePath::resolve against the songs root.
    let path = light_show_in_songs_root(&root, path)?;
    super::helpers::spawn_blocking_io("delete lighting file", move || std::fs::remove_file(&path))
        .await
}

/// A `?song=` on an endpoint that otherwise works on bare DSL, naming the song
/// whose tempo map the source should be parsed against.
#[derive(serde::Deserialize)]
pub(super) struct ValidateQuery {
    song: Option<String>,
    /// The lighting file the source is a draft of, as `PUT /api/lighting/:name`
    /// names it, so its `include`s resolve where saving it would.
    file: Option<String>,
}

/// POST /api/lighting/validate — validates lighting DSL content without saving.
///
/// Pass `?song=` when the source belongs to one. Measure-based timing does not
/// parse at all without a tempo map, so validating a bar/beat show without it
/// reports a failure the save path does not agree with.
pub(super) async fn validate_lighting(
    State(state): State<WebUiState>,
    Query(query): Query<ValidateQuery>,
    body: String,
) -> impl IntoResponse {
    let song = query
        .song
        .as_deref()
        .and_then(|name| state.player.songs().get(name).ok());
    let tempo = song.as_ref().and_then(|song| song.lighting_tempo_map());
    let path = match query.file.as_deref() {
        Some(file) => {
            let songs_root = state.songs_path();
            let root = match VerifiedRoot::new(&songs_root) {
                Ok(root) => root,
                Err(e) => return e.into_response(),
            };
            match light_show_in_songs_root(&root, &songs_root.join(file)) {
                Ok(path) => Some(path),
                Err(e) => return e,
            }
        }
        None => song.as_ref().map(|song| song.lighting_draft_path()),
    };
    match config_io::validate_light_show(
        &body,
        path.as_deref(),
        Some(&state.songs_path()),
        tempo.as_ref(),
    ) {
        Ok(()) => (
            StatusCode::OK,
            Json(LightingValidation {
                valid: true,
                errors: None,
            }),
        )
            .into_response(),
        Err(errors) => (
            StatusCode::BAD_REQUEST,
            Json(LightingValidation {
                valid: false,
                errors: Some(errors),
            }),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Fixture Type & Venue CRUD endpoints
// ---------------------------------------------------------------------------

/// One of the project's lighting directories. A request names it (`?dir=`
/// and the like), or it is the one the engine reads on this host, by the
/// shared reader ([`crate::config::Player::lighting_dirs_at`]) the CLI and
/// MCP ask (#673): the defaults only where the config sets none.
#[derive(Clone, Copy, Debug)]
pub(super) enum LightingDir {
    FixtureTypes,
    Venues,
}

/// The lighting directories the engine reads on this host, by the shared
/// reader; a config that does not load is the error to answer (a 500), the
/// one rule of the web endpoints for it.
pub(super) fn configured_lighting_dirs(
    config_path: &std::path::Path,
) -> Result<crate::config::lighting::ProjectDirs, String> {
    // codeql[rust/path-injection] config_path is set at startup, not user input.
    crate::config::Player::lighting_dirs_at(config_path)
        .map_err(|e| format!("the config does not load: {e}"))
}

/// The project-relative directory `which` a request names, else the one
/// the engine reads on this host. A config that does not load is a 500,
/// as it is to MCP: the defaults might not be where this host's files are.
#[allow(clippy::result_large_err)]
pub(super) fn lighting_dir(
    config_path: &std::path::Path,
    requested: Option<&str>,
    which: LightingDir,
) -> Result<String, axum::response::Response> {
    if let Some(dir) = requested.filter(|d| !d.is_empty()) {
        return Ok(dir.to_string());
    }
    let dirs = configured_lighting_dirs(config_path).map_err(|e| {
        (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response()
    })?;
    Ok(match which {
        LightingDir::FixtureTypes => dirs.fixture_types,
        LightingDir::Venues => dirs.venues,
    })
}

/// The file a fixture type of this name is declared in, if one is, in the
/// directory or any subdirectory: the file the lighting system loads it
/// from, by the same reader ([`lighting::project_files::type_files`]). The
/// file's stem need not match the name: an import writes
/// `astera_pixelbrick.fixture` for "Astera-PixelBrick", and a file written
/// by hand is called whatever its author liked. A file that does not parse
/// is never searched for it.
pub(super) fn existing_fixture_type_file(
    dir: &std::path::Path,
    name: &str,
) -> Option<std::path::PathBuf> {
    lighting::project_files::type_files(dir)
        .get(name)
        .map(|declared| declared.file.clone())
}

/// [`existing_fixture_type_file`], off the async runtime.
pub(super) async fn locate_fixture_type_file(
    dir: &std::path::Path,
    name: &str,
) -> Result<Option<std::path::PathBuf>, axum::response::Response> {
    let (dir, name) = (dir.to_path_buf(), name.to_string());
    super::helpers::spawn_blocking_io("locate fixture type file", move || {
        Ok::<_, String>(existing_fixture_type_file(&dir, &name))
    })
    .await
}

/// Serialises every read-check-write of a venue file, so two saves cannot
/// interleave between the version check and the write; and every write of a
/// fixture type file, which a fixture's settings save rewrites with its
/// venues. MCP's venue and fixture type writes take it too, and its edits
/// hold it from their read to their write, or a save made between the two
/// would be lost (#578). The saves here run on a blocking thread, and take
/// it with `blocking_lock`.
pub(crate) static VENUE_WRITES: super::write_lock::WriteLock = super::write_lock::WriteLock::new();

/// The version of a venue file: a hash of its bytes. A client sends back the
/// one it read (`If-Match`); a save whose version is no longer the file's is
/// refused, the same optimistic concurrency the config store has for inline
/// profiles.
pub(super) fn content_version(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// The `If-Match` version a client sent, unquoted.
pub(super) fn if_match_version(headers: &axum::http::HeaderMap) -> Option<String> {
    headers
        .get("if-match")
        .and_then(|v| v.to_str().ok())
        .map(|v| {
            v.trim()
                .trim_start_matches("W/")
                .trim_matches('"')
                .to_string()
        })
        .filter(|v| !v.is_empty())
}

/// The 409 for a save made against a version that is no longer current.
pub(super) fn stale_venue_response(name: &str, current: Option<String>) -> lighting_writes::Answer {
    lighting_writes::Answer {
        status: StatusCode::CONFLICT,
        body: json!({
            "error": format!(
                "venue \"{name}\" changed since you loaded it (another tab, page or an edit by \
                 hand); it has been reloaded, so reapply your change"
            ),
            "conflict": true,
            "version": current,
        }),
    }
}

/// Resolves a lighting directory path relative to the project root: the
/// one the request names (from a query param), else this host's
/// ([`lighting_dir`]). Returns an error response if the project root cannot
/// be canonicalized or the resolved path would escape it.
#[allow(clippy::result_large_err)]
pub(super) fn resolve_lighting_dir(
    config_path: &std::path::Path,
    override_dir: Option<&str>,
    which: LightingDir,
) -> Result<std::path::PathBuf, axum::response::Response> {
    use super::super::safe_path::{SafePath, VerifiedRoot};

    let root = VerifiedRoot::new(&project_root(config_path)?).map_err(|e| e.into_response())?;

    let relative = lighting_dir(config_path, override_dir, which)?;

    SafePath::validate_relative(&relative, &root).map_err(|e| e.into_response())
}

/// Checks a referential type's archive the way the lighting system will when
/// it expands one: the path is project-relative, stays inside the project, and
/// is a file that is actually there. Nothing is distilled — that happens at
/// load, through the cache — but a `.fixture` whose archive is missing or
/// outside the project cannot load, and a save is the last moment where the
/// person still has the text in front of them to fix it.
pub(super) fn validate_referential_archives(
    root: &std::path::Path,
    types: &std::collections::HashMap<String, lighting::types::FixtureType>,
) -> Result<(), lighting_writes::Answer> {
    let canonical_root = root.canonicalize().map_err(|_| {
        lighting_writes::Answer::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Failed to resolve project root",
        )
    })?;
    for (name, fixture_type) in types {
        if let Some(source) = fixture_type.source() {
            resolve_referential_archive(&canonical_root, name, source)?;
        }
    }
    Ok(())
}

/// The project root, canonicalized, for [`resolve_referential_archive`].
#[allow(clippy::result_large_err)]
pub(super) fn canonical_project_root(
    root: &std::path::Path,
) -> Result<std::path::PathBuf, axum::response::Response> {
    root.canonicalize().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Failed to resolve project root"})),
        )
            .into_response()
    })
}

/// One referential type's archive, resolved as the lighting system resolves
/// it: joined to the project root, canonicalized, inside the project, and a
/// file. The 400 names the type and the path as the `.fixture` writes it.
fn resolve_referential_archive(
    canonical_root: &std::path::Path,
    name: &str,
    source: &lighting::types::GdtfSource,
) -> Result<std::path::PathBuf, lighting_writes::Answer> {
    let archive = canonical_root.join(&source.path);
    let canonical = archive.canonicalize().map_err(|e| {
        lighting_writes::Answer::bad(format!(
            "fixture type \"{}\" references a GDTF archive that cannot be read: {}: {}",
            name, source.path, e
        ))
    })?;
    if !canonical.starts_with(canonical_root) {
        return Err(lighting_writes::Answer::bad(format!(
            "fixture type \"{}\" references a GDTF archive outside the project: {}",
            name, source.path
        )));
    }
    if !canonical.is_file() {
        return Err(lighting_writes::Answer::bad(format!(
            "fixture type \"{}\" references a GDTF archive that is not a file: {}",
            name, source.path
        )));
    }
    Ok(canonical)
}

/// Query parameters for lighting endpoints — allows overriding the directory.
#[derive(serde::Deserialize, Default)]
pub(super) struct LightingDirQuery {
    dir: Option<String>,
    /// The form a fixture type is saved in: `fixture` moves a `.light` to
    /// a `.fixture`; `light` keeps a `.light` one. A new type is always a
    /// `.fixture`, and a `.fixture` never goes back to `.light`.
    ext: Option<String>,
    /// The venues directory, for the fixture-type endpoints that say which
    /// venue fixtures use a type (default `lighting/venues`).
    venues_dir: Option<String>,
    /// The fixture types directory, for the venue endpoints that need the
    /// types (default `lighting/fixture_types`).
    fixture_types_dir: Option<String>,
    /// A save's old name: the save renames that venue or fixture type.
    from: Option<String>,
}

/// Validates that a fixture type or venue name is safe for use as a filename.
pub(crate) fn validate_lighting_name(name: &str) -> Result<(), lighting_writes::Answer> {
    validate_lighting_name_to_write(name, true)
}

/// As [`validate_lighting_name`], for a fixture type or venue being written:
/// one not already on disk (`exists` false) is being created, and its name
/// must also pass the rules for new names.
pub(crate) fn validate_lighting_name_to_write(
    name: &str,
    exists: bool,
) -> Result<(), lighting_writes::Answer> {
    use super::super::safe_path::SafePath;
    if SafePath::validate_name_to_write(name, exists).is_err() {
        return Err(lighting_writes::Answer::bad("Invalid name"));
    }
    if RESERVED_NAMES.contains(&name) {
        return Err(lighting_writes::Answer::bad(format!(
            "`{name}` is reserved: it is the name of an API route, so a file called that could not be addressed"
        )));
    }
    Ok(())
}

/// Path segments the lighting API uses as routes beside `{name}`.
const RESERVED_NAMES: &[&str] = &["lock", "validate", "activate"];

/// GET /api/lighting/assets/{*path} — serves a file of the project's
/// asset store (`lighting/.cache/assets/`, design §16.2): rig models,
/// meshes and thumbnails for the 3D view. Paths are content-addressed, so
/// a hit is immutable and cached as such.
pub(super) async fn get_lighting_asset(
    State(state): State<WebUiState>,
    Path(path): Path<String>,
) -> impl IntoResponse {
    use lighting::asset_store::{read_asset, AssetError};

    let project = project_root(&state.config_path)?;
    let gone =
        |error: &str| (StatusCode::NOT_FOUND, Json(json!({ "error": error }))).into_response();
    let asset = tokio::task::spawn_blocking(move || read_asset(&project, &path))
        .await
        .map_err(|_| gone("Asset not found"))?
        .map_err(|e| match e {
            AssetError::NotAnAsset => gone("Not an asset type"),
            // No store yet (no referential fixture type has expanded) is a
            // plain 404, not a server error.
            AssetError::NoStore => gone("No asset store"),
            AssetError::Path(e) => e.into_response(),
            AssetError::Unreadable => gone("Asset not found"),
        })?;
    let (bytes, content_type) = (asset.bytes, asset.content_type);
    let len = bytes.len();

    // The bytes are a stranger's (copied out of a GDTF archive), so the
    // type is not to be sniffed, and an SVG — which may script — is served
    // as a picture only: no scripts, no fetches, sandboxed if navigated to.
    let mut response = axum::response::Response::builder()
        .status(StatusCode::OK)
        .header(axum::http::header::CONTENT_TYPE, content_type)
        .header(axum::http::header::CONTENT_LENGTH, len)
        .header(axum::http::header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(
            axum::http::header::CACHE_CONTROL,
            "public, max-age=31536000, immutable",
        );
    if content_type == "image/svg+xml" {
        response = response.header(
            axum::http::header::CONTENT_SECURITY_POLICY,
            "default-src 'none'; style-src 'unsafe-inline'; sandbox",
        );
    }
    Ok::<_, axum::response::Response>(
        response
            .body(axum::body::Body::from(bytes))
            .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response()),
    )
}

/// GET /api/lighting/fixture-types — lists all fixture types from the directory.
pub(super) async fn get_fixture_types(
    State(state): State<WebUiState>,
    Query(query): Query<LightingDirQuery>,
) -> impl IntoResponse {
    let dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        LightingDir::FixtureTypes,
    )
    .map_err(|e| e.into_response())?;
    let venues_dir = resolve_lighting_dir(
        &state.config_path,
        query.venues_dir.as_deref(),
        LightingDir::Venues,
    )?;
    let root = canonical_project_root(&project_root(&state.config_path)?)?;
    let all = super::helpers::spawn_blocking_io("load fixture types", move || {
        let mut all = std::collections::HashMap::new();
        let mut referential = Vec::new();
        // The project's one reader of fixture types (the lighting system
        // loads through it too): every type file in the directory and its
        // subdirectories, then every GDTF in the library no record points
        // at. A type's file travels with it: the form a type is in decides
        // how it may be edited.
        let project = lighting::project_files::fixture_types(&root, Some(&dir));
        let shown = |path: &std::path::Path| lighting::project_files::display_in(&dir, path);
        let mut errors: Vec<LightingFileError> = project
            .files
            .problems
            .iter()
            .map(|p| LightingFileError {
                file: shown(&p.file),
                error: p.error.clone(),
            })
            .collect();
        for declared in &project.files.items {
            let fixture_type = &declared.item;
            if let Some(source) = fixture_type.source() {
                referential.push((declared.name.clone(), source.clone()));
            }
            all.insert(
                declared.name.clone(),
                FixtureTypeEntry {
                    referential: fixture_type.source().is_some(),
                    // A native type's addresses, for the venue editor's
                    // patch; a GDTF fixture's are its mode's, so null here.
                    footprint: Some(
                        fixture_type
                            .source()
                            .is_none()
                            .then(|| fixture_type.footprint()),
                    ),
                    rich: fixture_type.uses_rich_channels(),
                    fixture_type: fixture_type.clone(),
                    file: Some(shown(&declared.file)),
                    extension: Some(
                        declared
                            .file
                            .extension()
                            .and_then(|e| e.to_str())
                            .unwrap_or_default()
                            .to_string(),
                    ),
                    owner: project.files.record.owner(&declared.name).map(|owner| {
                        lighting_writes::wire_owner(
                            lighting::files::Kind::FixtureType,
                            &dir,
                            &owner,
                        )
                    }),
                    form_editable: crate::util::read_to_string(&declared.file).is_ok_and(|text| {
                        lighting::fixture_patch::form_expresses(&text, &declared.name)
                    }),
                    gdtf: None,
                    dsl: None,
                },
            );
        }
        let library = project.library;
        // An archive that will not read is the listing's error; a name
        // collision is already resolved (the second fixture is renamed).
        for finding in &library.findings {
            if finding.kind == "gdtf-unreadable" {
                errors.push(LightingFileError {
                    file: finding.file.clone(),
                    error: finding.message.clone(),
                });
            }
        }
        for library_type in &library.types {
            if all.contains_key(&library_type.name) {
                continue;
            }
            let declared = lighting::library::declared(library_type);
            referential.push((
                library_type.name.clone(),
                declared
                    .source()
                    .cloned()
                    .expect("a library type is GDTF-sourced"),
            ));
            all.insert(
                library_type.name.clone(),
                FixtureTypeEntry {
                    referential: true,
                    footprint: Some(None),
                    rich: false,
                    fixture_type: declared,
                    file: None,
                    extension: None,
                    owner: None,
                    form_editable: false,
                    gdtf: None,
                    dsl: None,
                },
            );
        }
        // What each referential type's archive says, for its card. One bad
        // archive is that card's `gdtf: null`, never the listing's failure.
        let usage = fixtures_by_type(&venues_dir);
        let cache = lighting::distill::DistillCache::new(root.join("lighting").join(".cache"));
        let mut archives = ArchiveCache::default();
        for (name, source) in referential {
            let fixtures: Vec<Option<&str>> = usage
                .get(&name)
                .into_iter()
                .flatten()
                .flat_map(|(_, f)| f.iter().map(|(_, mode)| mode.as_deref()))
                .collect();
            let parsed = archives.get(&root, &name, &source);
            let summary = parsed.map(|(bytes, description)| {
                gdtf_list_summary(&cache, bytes, description, &name, &fixtures)
            });
            if let Some(entry) = all.get_mut(&name) {
                entry.gdtf = Some(summary);
            }
        }
        Ok::<_, String>((all, errors))
    })
    .await?;
    let (all, errors) = all;
    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(FixtureTypeList {
                fixture_types: all,
                errors,
            }),
        )
            .into_response(),
    )
}

/// GET /api/lighting/fixture-types/:name — returns a single fixture type.
pub(super) async fn get_fixture_type(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        LightingDir::FixtureTypes,
    )?;
    let found = {
        let (dir, name) = (dir.clone(), name.clone());
        super::helpers::spawn_blocking_io("read fixture type", move || {
            let read = lighting::project_files::type_files(&dir);
            let (Some(declared), Some(owner)) = (read.get(&name), read.record.owner(&name)) else {
                return Ok(None);
            };
            let content = crate::util::read_to_string(&declared.file)?;
            Ok::<_, std::io::Error>(Some((
                declared.item.clone(),
                declared.file.clone(),
                lighting_writes::wire_owner(lighting::files::Kind::FixtureType, &dir, &owner),
                content,
            )))
        })
        .await?
    };
    let Some((ft, file_path, owner, content)) = found else {
        // A GDTF in the library with no record: a fixture with no file.
        let root = canonical_project_root(&project_root(&state.config_path)?)?;
        let (types_dir, wanted) = (dir.clone(), name.clone());
        let found = super::helpers::spawn_blocking_io("read the GDTF library", move || {
            Ok::<_, String>(
                lighting::library::unrecorded_types(&root, Some(&types_dir))
                    .get(&wanted)
                    .map(lighting::library::declared),
            )
        })
        .await?;
        return match found {
            Some(declared) => Ok((
                StatusCode::OK,
                Json(FixtureTypeEntry {
                    referential: true,
                    footprint: None,
                    rich: false,
                    fixture_type: declared,
                    file: None,
                    extension: None,
                    owner: None,
                    form_editable: false,
                    gdtf: None,
                    dsl: Some(None),
                }),
            )
                .into_response()),
            None => Err((
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Fixture type not found: {}", name)})),
            )
                .into_response()),
        };
    };
    Ok((
        StatusCode::OK,
        Json(FixtureTypeEntry {
            referential: ft.source().is_some(),
            footprint: None,
            rich: ft.uses_rich_channels(),
            fixture_type: ft,
            file: Some(crate::util::filename_display(&file_path).to_string()),
            extension: Some(
                file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or_default()
                    .to_string(),
            ),
            owner: Some(owner),
            form_editable: lighting::fixture_patch::form_expresses(&content, &name),
            gdtf: None,
            dsl: Some(Some(content)),
        }),
    )
        .into_response())
}

/// The GDTF source of a fixture type: its record's, or — for a GDTF in the
/// library with no record — its archive's, from the one function the engine
/// loads them with. `Ok(None)`: no such type; `Err`: a native type (the
/// 404 the caller answers).
async fn gdtf_source_of(
    root: &std::path::Path,
    dir: &std::path::Path,
    name: &str,
) -> Result<Option<lighting::types::GdtfSource>, axum::response::Response> {
    if let Some(file_path) = locate_fixture_type_file(dir, name).await? {
        let content = super::helpers::spawn_blocking_io("read fixture type", move || {
            crate::util::read_to_string(&file_path)
        })
        .await?;
        let types = lighting::parser::parse_fixture_types(&content).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to parse fixture type: {}", e)})),
            )
                .into_response()
        })?;
        let Some(fixture_type) = types.get(name) else {
            return Ok(None);
        };
        return match fixture_type.source() {
            Some(source) => Ok(Some(source.clone())),
            None => Err((
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("fixture type \"{}\" is not a GDTF type", name)})),
            )
                .into_response()),
        };
    }
    let (root, dir, name) = (root.to_path_buf(), dir.to_path_buf(), name.to_string());
    super::helpers::spawn_blocking_io("read the GDTF library", move || {
        Ok::<_, String>(
            lighting::library::unrecorded_types(&root, Some(&dir))
                .get(&name)
                .and_then(|t| lighting::library::declared(t).source().cloned()),
        )
    })
    .await
}

/// GET /api/lighting/fixture-types/:name/gdtf — what a GDTF fixture's
/// archive holds, for its page: every mode as the mode picker shows it,
/// the venue fixtures using it in their
/// own modes, and the rig model and thumbnail the 3D view draws (paths in
/// the asset store) — drawn in the first mode it can drive. The rig is
/// written through the expansion cache as the lighting system would write
/// it; a rig that cannot be made is not an error here — the modes are still
/// worth showing.
pub(super) async fn get_fixture_type_gdtf(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        LightingDir::FixtureTypes,
    )?;
    let root = canonical_project_root(&project_root(&state.config_path)?)?;
    let Some(source) = gdtf_source_of(&root, &dir, &name).await? else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Fixture type not found: {}", name)})),
        )
            .into_response());
    };
    let archive = resolve_referential_archive(&root, &name, &source)?;
    let venues_dir = resolve_lighting_dir(
        &state.config_path,
        query.venues_dir.as_deref(),
        LightingDir::Venues,
    )?;

    // Reading, parsing, distilling every mode and (cold) writing the rig are
    // all blocking work. The inner Result is the parse failure, a 400.
    let outcome = super::helpers::spawn_blocking_io("inspect GDTF", move || {
        let bytes = crate::util::read_file(&archive)?;
        let description = match lighting::gdtf::parse_archive(&bytes) {
            Ok(description) => description,
            Err(e) => return Ok::<_, std::io::Error>(Err(e.to_string())),
        };
        let cache = lighting::distill::DistillCache::new(root.join("lighting").join(".cache"));
        // A GDTF fixture has no mode of its own: it is drawn in the first
        // mode the archive offers that distils — the body is the same in
        // every mode.
        let rig_mode = drawn_mode(&description, &name);
        // Only the type's name goes in the log: everything else here is the
        // archive's own text.
        let rig = rig_mode.as_deref().and_then(|mode| {
            match cache.ensure_rig(&bytes, mode, || Ok(&description)) {
                Ok((rig, _warnings)) => Some(rig),
                Err(_) => {
                    tracing::warn!(fixture_type = %name, "no rig model for the fixture type's details view");
                    None
                }
            }
        });
        // The thumbnail sits beside the rig file, under the archive's hash.
        let thumbnail = rig.as_deref().and_then(|rel| {
            let model = cache.load_rig(rel).ok()?;
            let file = model.thumbnail?;
            let (dir, _) = rel.rsplit_once('/')?;
            Some(format!("{dir}/{file}"))
        });
        let beam = pinned_beam(&description, rig_mode.as_deref()).map(|b| GdtfBeam {
            beam_type: b.beam_type.clone(),
            beam_angle: b.beam_angle,
            field_angle: b.field_angle,
            luminous_flux: b.luminous_flux,
            color_temperature: b.color_temperature,
            power: b.power_consumption,
        });
        // Each fixture in its own mode, spelled as the archive spells it
        // (null when it names none, or none of the archive's).
        let venues = fixtures_by_type(&venues_dir)
            .remove(&name)
            .unwrap_or_default()
            .into_iter()
            .map(|(venue, fixtures)| TypeVenue {
                name: venue,
                fixtures: fixtures
                    .into_iter()
                    .map(|(fixture, mode)| TypeVenueFixture {
                        name: fixture,
                        mode: mode.as_deref().and_then(|m| resolve_mode(&description, m)),
                    })
                    .collect(),
            })
            .collect();
        Ok(Ok(FixtureTypeGdtf {
            archive: source.path,
            rig,
            thumbnail,
            beam,
            about: description.about.clone(),
            venues,
            inspection: inspect_description(&description),
        }))
    })
    .await?;
    match outcome {
        Ok(body) => Ok((StatusCode::OK, Json(body)).into_response()),
        Err(e) => Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Not a parseable GDTF: {}", e)})),
        )
            .into_response()),
    }
}

/// The beam a type's facts quote: the first `Beam` in the mode's own
/// geometry tree (through its geometry references), else the archive's
/// first. A fixture's beams are usually alike, and the mode's tree is the
/// one the fixture draws.
fn pinned_beam<'a>(
    description: &'a lighting::gdtf::Description,
    mode: Option<&str>,
) -> Option<&'a lighting::gdtf::BeamData> {
    mode.and_then(|m| description.modes.iter().find(|x| x.name == m))
        .and_then(|m| first_beam_under(description, &m.geometry, 0))
        .or_else(|| description.geometries.iter().find_map(|g| g.beam.as_ref()))
}

/// The first beam at or under a top-level geometry, in document order. A
/// reference is followed to the geometry it instances, a few levels deep at
/// most (GDTF does not nest them further, and a cycle must not spin).
fn first_beam_under<'a>(
    description: &'a lighting::gdtf::Description,
    top: &str,
    depth: usize,
) -> Option<&'a lighting::gdtf::BeamData> {
    if depth > 4 {
        return None;
    }
    let geometries = &description.geometries;
    let root = geometries
        .iter()
        .position(|g| g.parent.is_none() && g.name == top)?;
    // A parent precedes its children, so one pass in order finds the
    // subtree.
    let mut inside = vec![false; geometries.len()];
    for (index, node) in geometries.iter().enumerate().skip(root) {
        if index != root && !node.parent.is_some_and(|p| inside[p]) {
            continue;
        }
        inside[index] = true;
        if let Some(beam) = &node.beam {
            return Some(beam);
        }
        if let Some(beam) = node
            .reference
            .as_deref()
            .and_then(|r| first_beam_under(description, r, depth + 1))
        {
            return Some(beam);
        }
    }
    None
}

/// The fixtures of each type across a venues directory's files (both
/// extensions), by type, then venue name, then fixture in patch order. A
/// file that does not parse, or a missing directory, contributes nothing:
/// this answers "who uses it", not whether the venues are well.
fn fixtures_by_type(dir: &std::path::Path) -> FixturesByType {
    let venues: std::collections::BTreeMap<String, lighting::types::Venue> =
        lighting::project_files::venues(dir)
            .items
            .into_iter()
            .map(|d| (d.name, d.item))
            .collect();
    let mut by_type = FixturesByType::new();
    for (venue_name, venue) in venues {
        let mut of_type: std::collections::BTreeMap<&str, Vec<(String, Option<String>)>> =
            std::collections::BTreeMap::new();
        for fixture in venue.fixtures_by_patch() {
            of_type.entry(fixture.fixture_type()).or_default().push((
                fixture.name().to_string(),
                fixture.mode().map(str::to_string),
            ));
        }
        for (fixture_type, fixtures) in of_type {
            by_type
                .entry(fixture_type.to_string())
                .or_default()
                .push((venue_name.clone(), fixtures));
        }
    }
    by_type
}

/// Type name → (venue name → (fixture name, the mode its line names)).
type FixturesByType =
    std::collections::HashMap<String, Vec<(String, Vec<(String, Option<String>)>)>>;

/// A mode as the archive spells it, matched as the lighting system matches
/// one (exact, then normalized); `None` when it names no mode of the archive.
fn resolve_mode(description: &lighting::gdtf::Description, mode: &str) -> Option<String> {
    lighting::gdtf::match_mode(description, mode)
        .ok()
        .map(|m| m.name)
}

/// The mode a GDTF fixture is drawn in (its 3D view, thumbnail and quoted
/// beam): the first mode of the archive that distils — the importer warms
/// exactly that rig.
fn drawn_mode(description: &lighting::gdtf::Description, type_name: &str) -> Option<String> {
    lighting::import::first_drivable_mode(description, type_name)
}

/// GDTF archives read and parsed once per listing, by the path a `.fixture`
/// names; `None` for one that is missing, outside the project, or does not
/// parse.
#[derive(Default)]
struct ArchiveCache {
    parsed: std::collections::HashMap<String, Option<(Vec<u8>, lighting::gdtf::Description)>>,
}

impl ArchiveCache {
    fn get(
        &mut self,
        root: &std::path::Path,
        name: &str,
        source: &lighting::types::GdtfSource,
    ) -> Option<&(Vec<u8>, lighting::gdtf::Description)> {
        self.parsed
            .entry(source.path.clone())
            .or_insert_with(|| {
                let archive = resolve_referential_archive(root, name, source).ok()?;
                let bytes = crate::util::read_file(&archive).ok()?;
                let description = lighting::gdtf::parse_archive(&bytes).ok()?;
                Some((bytes, description))
            })
            .as_ref()
    }
}

/// A referential type's card in the listing: what the fixture is, its mode
/// count, the pinned mode and its beam, and how many venue fixtures use it.
/// The thumbnail is quoted only when the rig is already in the store — a
/// listing writes nothing.
fn gdtf_list_summary(
    cache: &lighting::distill::DistillCache,
    bytes: &[u8],
    description: &lighting::gdtf::Description,
    type_name: &str,
    fixtures: &[Option<&str>],
) -> GdtfSummary {
    // The fixture is drawn, and its beam quoted, in the first mode it can
    // drive: a GDTF fixture has no mode of its own.
    let drawn = drawn_mode(description, type_name);
    let beam = pinned_beam(description, drawn.as_deref()).map(|b| BeamBrief {
        beam_type: b.beam_type.clone(),
        angle: b.beam_angle,
    });
    // The thumbnail of the rig the details view draws, if it was drawn.
    let thumbnail = drawn.and_then(|mode| {
        let rel = lighting::distill::DistillCache::rig_path(bytes, &mode);
        let rig = cache.load_rig(&rel).ok()?;
        let file = rig.thumbnail?;
        let (dir, _) = rel.rsplit_once('/')?;
        Some(format!("{dir}/{file}"))
    });
    // How many venue fixtures use each mode, by the archive's spelling (a
    // mode that resolves to none is counted as written).
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for mode in fixtures.iter().flatten() {
        let mode = resolve_mode(description, mode).unwrap_or_else(|| mode.to_string());
        *counts.entry(mode).or_default() += 1;
    }
    let mut in_use: Vec<(String, usize)> = counts.into_iter().collect();
    in_use.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    GdtfSummary {
        fixture: description.name.clone(),
        manufacturer: description.manufacturer.clone(),
        modes: description.modes.len(),
        beam,
        thumbnail,
        used_by: fixtures.len(),
        in_use: in_use
            .into_iter()
            .map(|(mode, count)| ModeUse { mode, count })
            .collect(),
    }
}

/// PUT /api/lighting/fixture-types/:name — creates, updates or renames a
/// fixture type.
///
/// Accepts either JSON (the channel-map form) or plain text (the whole
/// file's DSL, which must declare this one type and nothing else). By the
/// one rule for type files ([`super::lighting_writes`]): a new name is
/// claimed at `<fixture types dir>/<stem>.fixture`; an existing one is
/// written only where its file is that type's alone, and refused (409, with
/// the file and why) otherwise. `?from=<old>` renames, as for venues, and,
/// as the fixture settings' rename does, retypes every venue line naming the
/// old name in `venues_dir` (refused, nothing written, when any venue file
/// cannot be; the answer's `rename` says what changed). A `.light` whose new
/// text needs a `.fixture` (rich channels, a GDTF reference, or
/// `?ext=fixture`) moves to `<stem>.fixture`.
pub(super) async fn put_fixture_type(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    if let Some(from) = &query.from {
        validate_lighting_name(from)?;
    }
    let dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        LightingDir::FixtureTypes,
    )?;

    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let from_form = content_type.contains("application/json");

    // Whatever form the body is in, an `ext` naming no known form is a
    // mistake worth reporting rather than quietly ignored.
    let ext = fixture_type_extension(query.ext.as_deref())?;

    let dsl = if from_form {
        // Parse JSON body and convert to DSL
        let json_body: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|e| lighting_writes::Answer::bad(format!("Invalid JSON: {}", e)))?;
        fixture_type_json_to_dsl(&name, &json_body).map_err(lighting_writes::Answer::bad)?
    } else {
        // Treat as raw DSL text
        String::from_utf8(body.to_vec())
            .map_err(|_| lighting_writes::Answer::bad("Invalid UTF-8"))?
    };

    let root = canonical_project_root(&project_root(&state.config_path)?)?;
    let venues_dir = resolve_lighting_dir(
        &state.config_path,
        query.venues_dir.as_deref(),
        LightingDir::Venues,
    )?;
    let save = lighting_writes::TypeSave {
        root,
        dir,
        venues_dir: venues_dir.clone(),
        name: name.clone(),
        from: query.from.clone(),
        ext,
        from_form,
        body: lighting_writes::Body::Text(dsl),
    };
    let rename = super::helpers::spawn_blocking_io("write fixture type", move || {
        Ok::<_, String>(lighting_writes::save_fixture_type(save))
    })
    .await??
    .rename;
    let venue_error = match &rename {
        Some(_) => {
            super::fixture_settings::reload_types_and_venues(
                &state.player,
                &state.config_path,
                &name,
                &venues_dir,
            )
            .await
        }
        None => None,
    };

    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(crate::webui::wire::fixture_types::FixtureTypeSaved {
                status: "saved".to_string(),
                name,
                rename,
                venue_error,
            }),
        )
            .into_response(),
    )
}

/// The form a fixture type save asks for (`?ext=`): `light` or `fixture`,
/// or none; anything else is a 400.
pub(crate) fn fixture_type_extension(
    ext: Option<&str>,
) -> Result<Option<&'static str>, lighting_writes::Answer> {
    match ext {
        None | Some("") => Ok(None),
        Some("light") => Ok(Some("light")),
        Some("fixture") => Ok(Some("fixture")),
        Some(other) => Err(lighting_writes::Answer::bad(format!(
            "Unknown fixture type extension: {}",
            other
        ))),
    }
}

/// DELETE /api/lighting/fixture-types/:name — deletes a fixture type file:
/// only one that is the type's alone (409 otherwise, with the file and
/// why). A GDTF in the library with no record is its archive, deleted.
#[allow(clippy::result_large_err)]
pub(super) async fn delete_fixture_type(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        LightingDir::FixtureTypes,
    )?;
    let root = canonical_project_root(&project_root(&state.config_path)?)?;
    let deleted = name.clone();
    let outcome = super::helpers::spawn_blocking_io("delete fixture type", move || {
        Ok::<_, String>(lighting_writes::delete_fixture_type(&root, &dir, &deleted))
    })
    .await??;
    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(FixtureTypeDeleted {
                status: "deleted".to_string(),
                name,
                archive_removed: outcome.archive_removed,
                archive_kept: outcome.archive_kept,
            }),
        )
            .into_response(),
    )
}

/// The tempo a `.light` file at this path will be loaded with, found by
/// matching the path against the loaded songs' directories.
fn song_tempo_for_path(
    state: &WebUiState,
    path: &std::path::Path,
) -> Option<crate::tempo::TempoMap> {
    // Both sides resolved: `path` is, and a song's folder need not be (one
    // built from a path through a linked songs folder). The plain
    // comparison first, so the folders are canonicalized only when no
    // song's matches as written.
    let songs = state.player.songs().list();
    let song = songs
        .iter()
        .find(|song| path.starts_with(song.base_path()))
        .or_else(|| {
            songs.iter().find(|song| {
                song.base_path()
                    .canonicalize()
                    .is_ok_and(|base| path.starts_with(base))
            })
        })?;
    song.lighting_tempo_map()
}

/// Returns the group names valid as cue targets, with the fixtures each
/// currently resolves to in the loaded venue.
///
/// These are the logical groups declared under `dmx.lighting.groups`. Venues no
/// longer define groups of their own — fixtures carry tags, and logical groups
/// select on them — so this is the only list a show can target.
pub(super) async fn get_lighting_groups(State(state): State<WebUiState>) -> impl IntoResponse {
    let Some(system) = state
        .player
        .dmx_engine()
        .and_then(|dmx| dmx.broadcast_handles().lighting_system)
    else {
        // No DMX device configured — authoring on a laptop, which is the normal
        // case for editing a show. The groups are declared in the player config,
        // not by the engine, so the names are still knowable; only the fixtures
        // each resolves to are not.
        return groups_from_config(&state.config_path).await;
    };

    // The lighting system's mutex is shared with the effects loop thread, and
    // resolution mutates its cache, so this belongs on the blocking pool.
    let groups = tokio::task::spawn_blocking(move || {
        let mut guard = system.lock();
        let names: Vec<String> = guard
            .logical_groups_iter()
            .map(|(n, _)| n.clone())
            .collect();
        let mut out: Vec<LightingGroup> = names
            .into_iter()
            .map(|name| LightingGroup {
                fixtures: guard.resolve_logical_group_graceful(&name),
                name,
            })
            .collect();
        out.sort_by(|a, b| a.name.cmp(&b.name));
        out
    })
    .await;

    match groups {
        Ok(groups) => Json(LightingGroups {
            groups,
            resolved: None,
        })
        .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to resolve groups: {e}")})),
        )
            .into_response(),
    }
}

/// GET /api/lighting/readiness — the facts behind the Lighting overview's five
/// checks: does the show reach the lights?
///
/// Facts, not verdicts. The UI turns them into ready / needs attention /
/// blocked / unknown, so the wording and the rules live in one place. Every
/// piece is something mtrack already knows: the loaded lighting system, the
/// songs' parsed shows, the same lint MCP's `validate_lighting` runs, and the
/// olad patch probe. All of it is blocking (the lighting-system mutex is shared
/// with the effects loop, the probe is HTTP), so it runs off the async worker.
pub(super) async fn get_readiness(State(state): State<WebUiState>) -> impl IntoResponse {
    let player = state.player.clone();
    match tokio::task::spawn_blocking(move || readiness_report(&player)).await {
        Ok(report) => Json(report).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to gather readiness: {e}")})),
        )
            .into_response(),
    }
}

/// GET /api/lighting/fit — the facts and suggestions behind the "Fit shows"
/// page: for each group the shows use, what it needs and what would satisfy it;
/// the focus points the shows aim at that the venue lacks; and the universes
/// with no output. The suggestion rules live in `lighting::fit`, shared with
/// MCP's `suggest_group_tags`. Blocking (lighting-system mutex, olad probe), so
/// off the async worker.
pub(super) async fn get_fit(State(state): State<WebUiState>) -> impl IntoResponse {
    let player = state.player.clone();
    match tokio::task::spawn_blocking(move || crate::lighting::fit::FitReport::gather(&player))
        .await
    {
        Ok(report) => Json(report).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to gather fit: {e}")})),
        )
            .into_response(),
    }
}

/// Body of `POST /api/lighting/evaluate`.
#[derive(serde::Deserialize)]
pub(super) struct EvaluateRequest {
    song: String,
    /// Song times, in seconds.
    times: Vec<f64>,
}

/// POST /api/lighting/evaluate — what a song's shows would be doing at each
/// requested time, offline: the Preview mode's data.
///
/// Each evaluation carries the live `state` message's `fixtures`, `poses` and
/// `cells` (built by the same helper, so the 3D scene is fed one shape from
/// both sources) plus `active_effects` with the groups each cue named. The
/// fixtures no cue targets are the same for every instant and come once,
/// beside the evaluations, as `untouched`. Runs the song's registered shows
/// through `evaluate_with_system` — the function MCP's `evaluate_show` uses —
/// and never touches the DMX engine's effect engine, so a playing song plays
/// on.
pub(super) async fn evaluate_lighting(
    State(state): State<WebUiState>,
    Json(request): Json<EvaluateRequest>,
) -> impl IntoResponse {
    let error = |status: StatusCode, message: String| {
        (status, Json(json!({"error": message}))).into_response()
    };
    if request.times.iter().any(|t| !t.is_finite() || *t < 0.0) {
        return error(
            StatusCode::BAD_REQUEST,
            "times must be non-negative seconds".to_string(),
        );
    }

    let songs = state.player.songs();
    let song = match songs.get(&request.song) {
        Ok(song) => song,
        // A song whose lighting fails to parse never loads: the registry
        // keeps its failure, and the message says why.
        Err(e) => return super::helpers::song_lookup_response(e),
    };
    let shows = crate::lighting::evaluate::registered_shows(&song);
    if shows.is_empty() {
        return error(
            StatusCode::BAD_REQUEST,
            format!("song `{}` has no lighting loaded", request.song),
        );
    }
    let tempo = song.lighting_tempo_map();
    // Duration::from_secs_f64 panics on what it cannot hold; the evaluator
    // bounds the count and the latest instant itself. Collected rather than
    // sized up front: a capacity taken from the request is what CodeQL flags.
    let times: Vec<std::time::Duration> = match request
        .times
        .iter()
        .map(|t| crate::util::duration_from_secs(*t))
        .collect::<Option<Vec<_>>>()
    {
        Some(times) => times,
        None => {
            return error(
                StatusCode::BAD_REQUEST,
                crate::lighting::evaluate::EvaluateError::TimeOutOfRange.to_string(),
            )
        }
    };
    let lighting_system = state
        .player
        .dmx_engine()
        .and_then(|dmx| dmx.broadcast_handles().lighting_system);

    // The lighting system is a `parking_lot` mutex shared with the effects
    // loop, and evaluation is pure CPU besides: off the async worker.
    let evaluated = tokio::task::spawn_blocking(move || {
        crate::lighting::evaluate::evaluate_with_system(
            shows,
            tempo.as_ref(),
            &times,
            lighting_system.as_deref(),
        )
    })
    .await;
    let evaluated = match evaluated {
        Ok(Ok(evaluated)) => evaluated,
        Ok(Err(e)) => return error(StatusCode::BAD_REQUEST, e.to_string()),
        Err(e) => {
            return error(
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to evaluate: {e}"),
            )
        }
    };

    use crate::webui::wire::preview::{Preview, PreviewEffect, PreviewEvaluation};
    let untouched = evaluated
        .venue_fixtures
        .iter()
        .filter(|name| !evaluated.targeted.contains(*name))
        .cloned()
        .collect();
    let evaluations = evaluated
        .evaluations
        .iter()
        .map(|evaluation| PreviewEvaluation {
            time: evaluation.time.as_secs_f64(),
            frame: super::super::state::stage_frame(&evaluation.fixtures, &evaluation.poses),
            active_effects: evaluation
                .active_effects
                .iter()
                .map(|effect| PreviewEffect {
                    id: effect.id.clone(),
                    groups: evaluated.effect_groups.get(&effect.id).cloned(),
                    kind: effect.effect_type.to_string(),
                    layer: LayerSet::standard().display_name(effect.layer),
                    elapsed: effect.elapsed.as_secs_f64(),
                    duration: effect.duration.as_secs_f64(),
                    fixtures: effect.fixtures.clone(),
                })
                .collect(),
        })
        .collect();
    Json(Preview {
        song: request.song,
        evaluations,
        untouched,
        venue_error: evaluated.venue_error,
    })
    .into_response()
}

/// The readiness facts for the player's running profile.
fn readiness_report(player: &crate::player::Player) -> crate::webui::wire::readiness::Readiness {
    use crate::lighting::readiness::{group_names, VenueFacts};
    use crate::webui::wire::readiness::*;
    use std::collections::BTreeSet;

    // Shows are reported with or without DMX: a laptop with no DMX output is
    // where shows are written, and a show that does not load is broken there
    // too. Everything that needs a venue or an output is empty without one.
    let dmx = player.dmx_engine();
    let configured = dmx
        .as_ref()
        .map(|d| d.configured_universes())
        .unwrap_or_default();

    // Every song with lighting, and the shows its files hold.
    let songs = player.songs();
    let with_lighting = crate::lighting::readiness::songs_with_lighting(&songs);
    let all_shows: Vec<lighting::parser::LightShow> = with_lighting
        .iter()
        .flat_map(|(_, shows)| shows.iter().cloned())
        .collect();
    let names = group_names(&all_shows);

    // The lighting system, under its lock for as short a time as it takes to
    // read what the sections below need.
    let mut facts = VenueFacts::default();
    let mut venue = None;
    let mut in_use: BTreeSet<String> = BTreeSet::new();
    let mut unresolved = Vec::new();
    let mut library = Vec::new();
    let mut used_universes: BTreeSet<u16> = BTreeSet::new();
    if let Some(system) = dmx
        .as_ref()
        .and_then(|d| d.broadcast_handles().lighting_system)
    {
        let mut guard = system.lock();
        facts = VenueFacts::collect(&mut guard, &names, Some(&configured));
        // The library's own findings: two archives that would share a name,
        // an archive that cannot be read. Neither stops a load.
        library = guard.library_findings().to_vec();
        if let Some(current) = guard.get_current_venue() {
            let mut fixtures: Vec<&lighting::types::Fixture> =
                current.fixtures().values().collect();
            fixtures.sort_by(|a, b| a.name().cmp(b.name()));
            for fixture in &fixtures {
                used_universes.insert(fixture.universe());
                match guard.fixture_problem(current.name(), fixture) {
                    None => {
                        in_use.insert(fixture.fixture_type().to_string());
                    }
                    Some(reason) => unresolved.push(UnresolvedFixture {
                        fixture: fixture.name().to_string(),
                        fixture_type: fixture.fixture_type().to_string(),
                        reason,
                    }),
                }
            }
            venue = Some(ReadinessVenue {
                name: current.name().to_string(),
                fixtures: fixtures.len(),
                placed: fixtures.iter().filter(|f| f.position().is_some()).count(),
                focus_points: current.focus_points().keys().cloned().collect(),
            });
        }
    }

    let groups: Vec<ReadinessGroup> = names
        .iter()
        .filter(|_| dmx.is_some())
        .map(|name| ReadinessGroup {
            name: name.clone(),
            fixtures: facts.group_fixture_counts.get(name).copied().unwrap_or(0),
            songs: with_lighting
                .iter()
                .filter(|(_, shows)| group_names(shows).contains(name))
                .map(|(song, _)| song.name().to_string())
                .collect(),
        })
        .collect();

    let mut shows: Vec<ReadinessShow> = with_lighting
        .iter()
        .map(|(song, shows)| {
            let warnings: Vec<_> = facts
                .lint(shows, Some(song))
                .into_iter()
                // Venue-level: the Output and Venue sections already carry
                // them, and repeating them under every song would say each
                // once per song.
                .filter(|w| {
                    !matches!(
                        w.kind,
                        "unconfigured-universe" | "patch-overlap" | "patch-overrun"
                    )
                })
                .collect();
            let files: Vec<String> = song
                .dsl_lighting_shows()
                .iter()
                .map(|dsl| {
                    let path = dsl.file_path();
                    path.strip_prefix(song.base_path())
                        .unwrap_or(path)
                        .display()
                        .to_string()
                })
                .collect();
            ReadinessShow {
                song: song.name().to_string(),
                files,
                error: None,
                warnings,
            }
        })
        .collect();
    // A song whose show does not parse never loads (`Song::new` fails as a
    // whole), so it is not in the list above: the player recorded the failure,
    // with the parser's own message, and that is the error.
    for failure in songs.failures() {
        if failure.concerns_lighting() {
            shows.push(ReadinessShow {
                song: failure.name().to_string(),
                files: Vec::new(),
                error: Some(failure.error().to_string()),
                warnings: Vec::new(),
            });
        }
    }
    shows.sort_by(|a, b| a.song.cmp(&b.song));

    let unconfigured: Vec<u16> = used_universes
        .iter()
        .copied()
        .filter(|u| !configured.contains(u))
        .collect();

    // Ask olad about the universes the venue streams to, when there are any,
    // else about everything configured. A universe with no output configured is
    // never streamed to, so asking olad about it would only repeat the
    // `unconfigured` finding. Nothing to ask means no answer, not a reachable
    // olad.
    let to_probe: Vec<u16> = if used_universes.is_empty() {
        configured.clone()
    } else {
        used_universes
            .iter()
            .copied()
            .filter(|u| configured.contains(u))
            .collect()
    };
    let olad = match &dmx {
        Some(dmx) if !to_probe.is_empty() => {
            let report = dmx.olad().probe(&to_probe);
            Some(OladPatch {
                reachable: report.reachable,
                unpatched: report.unpatched,
            })
        }
        _ => None,
    };

    // The venue's own findings: whether it loads at all (one fixture that
    // cannot be driven fails the whole venue, and nothing lights), and
    // fixtures patched over each other or past the universe's end.
    let mut report = Readiness {
        dmx: dmx.is_some(),
        venue,
        venue_error: facts.venue_error,
        patch_warnings: facts.patch_warnings,
        fixture_types: ReadinessTypes {
            in_use: in_use.into_iter().collect(),
            unresolved,
            library,
        },
        groups,
        shows,
        output: ReadinessOutput {
            universes: used_universes.into_iter().collect(),
            unconfigured,
            olad,
        },
        checks: Vec::new(),
    };
    report.checks = report.verdicts();
    report
}

/// The logical group names declared under `dmx.lighting.groups`, read from the
/// player config when no engine is running to resolve them against.
async fn groups_from_config(config_path: &std::path::Path) -> axum::response::Response {
    // codeql[rust/path-injection] config_path is set at startup, not user input.
    let path = config_path.to_path_buf();
    // A parse failure is reported, not swallowed. `.ok()` here answered
    // "no groups" for a config that does not load at all — the same silent
    // empty list this fallback exists to remove, one layer down, and the reason
    // a missing `dmx.universes` looked like a rig with nothing declared.
    let loaded = tokio::task::spawn_blocking(move || {
        crate::config::Player::deserialize(&path)
            .map(|player| {
                player
                    .host_lighting()
                    .map(|lighting| {
                        let mut names: Vec<String> = lighting.groups().keys().cloned().collect();
                        names.sort();
                        names
                    })
                    .unwrap_or_default()
            })
            .map_err(|e| e.to_string())
    })
    .await;

    let names = match loaded {
        Ok(Ok(names)) => names,
        Ok(Err(e)) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": format!(
                        "no DMX engine is running, and the player config could not be read \
                         to list the declared groups: {e}"
                    )
                })),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("reading the player config failed: {e}")})),
            )
                .into_response();
        }
    };

    let groups = names
        .into_iter()
        .map(|name| LightingGroup {
            name,
            fixtures: vec![],
        })
        .collect();
    Json(LightingGroups {
        groups,
        resolved: Some(false),
    })
    .into_response()
}

/// GET /api/lighting/venues — lists all venues from the directory.
pub(super) async fn get_venues(
    State(state): State<WebUiState>,
    Query(query): Query<LightingDirQuery>,
) -> impl IntoResponse {
    let dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        LightingDir::Venues,
    )
    .map_err(|e| e.into_response())?;
    if !dir.is_dir() {
        return Ok::<_, axum::response::Response>(
            (
                StatusCode::OK,
                Json(VenueList {
                    venues: Default::default(),
                    owners: Default::default(),
                    errors: None,
                    versions: None,
                }),
            )
                .into_response(),
        );
    }
    let all = super::helpers::spawn_blocking_io("load venues", move || {
        // The lighting system's own reader: both extensions, the directory
        // and its subdirectories, the first file to claim a name keeps it.
        let read = lighting::project_files::venues(&dir);
        let mut all = std::collections::HashMap::new();
        let mut versions = std::collections::HashMap::new();
        let mut owners = std::collections::HashMap::new();
        for declared in read.items {
            if let Ok(bytes) = crate::util::read_file(&declared.file) {
                versions.insert(declared.name.clone(), content_version(&bytes));
            }
            if let Some(owner) = read.record.owner(&declared.name) {
                owners.insert(
                    declared.name.clone(),
                    lighting_writes::wire_owner(Kind::Venue, &dir, &owner),
                );
            }
            all.insert(declared.name, declared.item);
        }
        let errors: Vec<LightingFileError> = read
            .problems
            .iter()
            .map(|p| LightingFileError {
                file: lighting::project_files::display_in(&dir, &p.file),
                error: p.error.clone(),
            })
            .collect();
        Ok::<_, String>((all, owners, errors, versions))
    })
    .await?;
    let (all, owners, errors, versions) = all;
    Ok((
        StatusCode::OK,
        Json(VenueList {
            venues: all,
            owners,
            errors: Some(errors),
            versions: Some(versions),
        }),
    )
        .into_response())
}

/// GET /api/lighting/venues/:name — returns a single venue, from the file
/// the loader read it from. A file that does not parse is never searched
/// for it.
pub(super) async fn get_venue(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        LightingDir::Venues,
    )?;
    let found = {
        let (dir, name) = (dir.clone(), name.clone());
        super::helpers::spawn_blocking_io("read venue file", move || {
            let read = lighting::project_files::venues(&dir);
            let Some(declared) = read.get(&name) else {
                return Ok(None);
            };
            crate::util::read_file(&declared.file)
                .map(|bytes| Some((declared.item.clone(), bytes)))
                .map_err(|e| format!("cannot read {}: {e}", declared.file.display()))
        })
        .await?
    };
    let Some((venue, bytes)) = found else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Venue not found: {}", name)})),
        )
            .into_response());
    };
    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(VenueDetail {
                venue,
                version: content_version(&bytes),
                dsl: String::from_utf8_lossy(&bytes).into_owned(),
            }),
        )
            .into_response(),
    )
}

/// PUT /api/lighting/venues/:name — creates, updates or renames a venue.
///
/// By the one rule for venue files ([`super::lighting_writes`]): a new name
/// is claimed at `<venues dir>/<stem>.venue`; an existing one is written
/// only where its file is that venue's alone, and refused (409, with the
/// file and why) otherwise. `?from=<old>` renames: a file at the old name's
/// canonical place moves to the new name's, any other is renamed in place.
/// A JSON body patches the file (never regenerates an existing one); a DSL
/// body is the whole file, and must declare this one venue and nothing
/// else. An `If-Match` version, when sent, must be the file's current one
/// (409 otherwise); the response carries the new `version`.
pub(super) async fn put_venue(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    if let Some(from) = &query.from {
        validate_lighting_name(from)?;
    }
    let dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        LightingDir::Venues,
    )?;

    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let expected = if_match_version(&headers);

    let (body, needs_new_format) = if content_type.contains("application/json") {
        let json_body: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|e| lighting_writes::Answer::bad(format!("Invalid JSON: {}", e)))?;
        let venue = venue_from_json(&name, &json_body).map_err(lighting_writes::Answer::bad)?;
        let needs = lighting_writes::needs_venue_extension(&venue);
        let renamed_to = name.clone();
        let subject = query.from.clone().unwrap_or_else(|| name.clone());
        let renaming = !lighting::files::same_name(&subject, &name);
        // An existing file is patched, not regenerated: comments, TODO
        // lines and the file's own layout survive a save from the UI.
        let edit: lighting_writes::Edit = Box::new(move |current| {
            let patched = match current {
                // Patched under the name it has, then renamed.
                Some(text) => venue_from_json(&subject, &json_body)
                    .and_then(|as_is| lighting::venue_patch::patch_venue(text, &subject, &as_is))
                    .and_then(|patched| {
                        if renaming {
                            lighting::venue_patch::rename_venue(&patched, &subject, &renamed_to)
                        } else {
                            Ok(patched)
                        }
                    }),
                None => venue_json_to_dsl(&renamed_to, &json_body),
            };
            patched.map_err(lighting_writes::Answer::bad)
        });
        (lighting_writes::Body::Edit(edit), Some(needs))
    } else {
        let text = String::from_utf8(body.to_vec())
            .map_err(|_| lighting_writes::Answer::bad("Invalid UTF-8"))?;
        (lighting_writes::Body::Text(text), None)
    };

    let save = lighting_writes::VenueSave {
        project: super::helpers::project_dir(&state),
        dir: dir.clone(),
        name: name.clone(),
        from: query.from.clone(),
        expected,
        needs_new_format,
        body,
    };
    let saved = super::helpers::spawn_blocking_io("write venue", move || {
        Ok::<_, String>(lighting_writes::save_venue(save))
    })
    .await??;
    let version = content_version(saved.text.as_bytes());

    let reloaded = reload_if_current_venue(&state, &name).await;
    let venue_error = venue_error_after_save(&state, &name, &dir).await;

    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(crate::webui::wire::venue::VenueSaved {
                status: "saved".to_string(),
                name,
                reloaded,
                version,
                venue_error,
            }),
        )
            .into_response(),
    )
}

/// After a venue save: when the saved venue is the current one and no
/// longer loads — a fixture whose type or mode cannot be driven fails the
/// whole venue, and then nothing lights — the fixture and why, as
/// `{venue, fixture, reason}`; `null` otherwise. The save stands either
/// way; this is what the editor shows beside it. Read from the running
/// engine after its reload, or, with no engine (a laptop), from the
/// project's files for the venue the config names as current.
pub(super) async fn venue_error_after_save(
    state: &WebUiState,
    name: &str,
    venues_dir: &std::path::Path,
) -> Option<crate::lighting::findings::VenueError> {
    venue_error_of(&state.player, &state.config_path, name, venues_dir).await
}

/// [`venue_error_after_save`] for `player`, whose config is at
/// `config_path`: what MCP's writes answer too.
pub(crate) async fn venue_error_of(
    player: &std::sync::Arc<crate::player::Player>,
    config_path: &std::path::Path,
    name: &str,
    venues_dir: &std::path::Path,
) -> Option<crate::lighting::findings::VenueError> {
    let player = player.clone();
    let config_path = config_path.to_path_buf();
    let venue = name.to_string();
    let venues_dir = venues_dir.to_path_buf();
    let found = tokio::task::spawn_blocking(move || {
        if let Some(system) = player.broadcast_handles().and_then(|h| h.lighting_system) {
            let guard = system.lock();
            if guard.current_venue() != Some(venue.as_str()) {
                return None;
            }
            return guard.venue_problem(&venue);
        }
        // codeql[rust/path-injection] config_path is set at startup, not user input.
        let config = crate::config::Player::deserialize(&config_path).ok()?;
        let lighting = config.host_lighting()?;
        if lighting.current_venue() != Some(venue.as_str()) {
            return None;
        }
        let root = canonical_project_root(&project_root(&config_path).ok()?).ok()?;
        let types = lighting.fixture_types_dir();
        let system = system_from_files(&root, &root.join(types), &venues_dir).ok()?;
        system.venue_problem(&venue)
    })
    .await
    .ok()
    .flatten();
    found.map(|(fixture, reason)| crate::lighting::findings::VenueError {
        venue: name.to_string(),
        fixture,
        reason,
    })
}

/// After a venue file changed on disk: if it is the venue the running engine
/// is playing against, re-read it and tell every stage watcher. Returns whether that happened. A reload failure is logged,
/// not returned — the save itself is durable and the loader will say the
/// same thing at next startup.
pub(super) async fn reload_if_current_venue(state: &WebUiState, name: &str) -> bool {
    // The lighting system's lock is a blocking mutex and the reload reads
    // and parses files, so both stay off the async workers.
    let player = state.player.clone();
    let venue = name.to_string();
    let outcome = tokio::task::spawn_blocking(move || {
        let is_current = player
            .broadcast_handles()
            .and_then(|h| h.lighting_system)
            .is_some_and(|system| system.lock().current_venue() == Some(venue.as_str()));
        if !is_current {
            return Ok(false);
        }
        player
            .reload_current_venue()
            .map(|()| true)
            .map_err(|e| e.to_string())
    })
    .await;
    match outcome {
        Ok(Ok(reloaded)) => reloaded,
        Ok(Err(e)) => {
            tracing::warn!(venue = name, error = %e, "venue saved, but the running engine could not reload it");
            false
        }
        Err(e) => {
            tracing::warn!(venue = name, error = %e, "venue reload task failed");
            false
        }
    }
}

/// DELETE /api/lighting/venues/:name — deletes a venue file: only one that
/// is the venue's alone (409 otherwise, with the file and why).
pub(super) async fn delete_venue(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        LightingDir::Venues,
    )?;
    let wanted = name.clone();
    super::helpers::spawn_blocking_io("delete venue", move || {
        Ok::<_, String>(lighting_writes::delete_venue(&dir, &wanted))
    })
    .await??;
    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(json!({"status": "deleted", "name": name})),
        )
            .into_response(),
    )
}

// ---------------------------------------------------------------------------
// Lighting helpers
// ---------------------------------------------------------------------------

/// Converts a JSON fixture type definition to DSL format.
fn fixture_type_json_to_dsl(name: &str, json: &serde_json::Value) -> Result<String, String> {
    let channels = json
        .get("channels")
        .and_then(|v| v.as_object())
        .ok_or("Missing 'channels' object")?;

    let mut dsl = format!("fixture_type \"{name}\" {{\n");
    dsl.push_str(&format!("  channels: {}\n", channels.len()));
    dsl.push_str("  channel_map: {\n");

    let mut entries: Vec<(&String, &serde_json::Value)> = channels.iter().collect();
    entries.sort_by_key(|(_, v)| v.as_u64().unwrap_or(0));
    // The editor's own check of its rows (`venue_rows`), its first finding
    // the refusal.
    let mut rows = Vec::new();
    for (ch_name, ch_offset) in &entries {
        let offset = ch_offset
            .as_u64()
            .ok_or(format!("Channel '{}' offset must be a number", ch_name))?;
        rows.push(lighting::venue_rows::ChannelRow {
            name: ch_name.to_string(),
            offset: Some(offset as f64),
        });
    }
    if let Some(found) = lighting::venue_rows::channel_row_problems(&rows).first() {
        use lighting::venue_rows::ChannelProblem;
        let name = &rows[found.row as usize].name;
        return Err(match found.problems[0] {
            ChannelProblem::NoName => "A channel has no name".to_string(),
            ChannelProblem::DuplicateName => format!("Channel '{name}' listed more than once"),
            ChannelProblem::BadOffset => {
                format!("Channel '{name}' offset must be from 1 to 65535")
            }
        });
    }
    for (i, (ch_name, ch_offset)) in entries.iter().enumerate() {
        let offset = ch_offset.as_u64().unwrap_or_default();
        let comma = if i + 1 < entries.len() { "," } else { "" };
        dsl.push_str(&format!("    \"{ch_name}\": {offset}{comma}\n"));
    }
    dsl.push_str("  }\n");

    if let Some(v) = json.get("max_strobe_frequency").and_then(|v| v.as_f64()) {
        dsl.push_str(&format!("  max_strobe_frequency: {v}\n"));
    }
    if let Some(v) = json.get("min_strobe_frequency").and_then(|v| v.as_f64()) {
        dsl.push_str(&format!("  min_strobe_frequency: {v}\n"));
    }
    if let Some(v) = json.get("strobe_dmx_offset").and_then(|v| v.as_u64()) {
        dsl.push_str(&format!("  strobe_dmx_offset: {v}\n"));
    }

    dsl.push_str("}\n");
    Ok(dsl)
}

/// GET /api/lighting/venues/:name/patch — the addresses each fixture of a
/// venue occupies, from the files, with no DMX engine needed (venues are
/// authored on laptops): each fixture's span from its own mode's expansion
/// (a native type's channels, a GDTF type's mode distilled through the
/// cache), and the venue's overlaps and overruns as `lighting::patch`
/// finds them. A fixture whose type or mode does not load has a `null`
/// footprint and is left out of the checks — never guessed at. The venue
/// inspector checks a candidate address or mode against these spans before
/// it saves.
pub(super) async fn get_venue_patch(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let venues_dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        LightingDir::Venues,
    )?;
    let types_dir = resolve_lighting_dir(
        &state.config_path,
        query.fixture_types_dir.as_deref(),
        LightingDir::FixtureTypes,
    )?;
    let root = canonical_project_root(&project_root(&state.config_path)?)?;
    let outcome = super::helpers::spawn_blocking_io("read venue patch", move || {
        let system = system_from_files(&root, &types_dir, &venues_dir)?;
        let Some(venue) = system
            .venues_iter()
            .find(|(n, _)| **n == name)
            .map(|(_, v)| v)
        else {
            return Ok::<_, String>(None);
        };
        Ok(Some(venue_patch(&system, venue)))
    })
    .await?;
    match outcome {
        Some(body) => Ok((StatusCode::OK, Json(body)).into_response()),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Venue not found"})),
        )
            .into_response()),
    }
}

/// GET /api/lighting/venues/:name/scene — what the 3D view draws for a
/// venue, read from the files with no engine needed: the shape of the
/// stage's metadata for the current venue (each fixture's type, mode,
/// position, rotation and rig; the venue's focus points and scenery), so a
/// venue that is not the current one can be looked at in 3D. Loading the
/// system distils each (archive, mode) rig into the store, as the engine's
/// load does; a fixture whose type or mode does not load has no rig and is
/// drawn generically.
pub(super) async fn get_venue_scene(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let venues_dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        LightingDir::Venues,
    )?;
    let types_dir = resolve_lighting_dir(
        &state.config_path,
        query.fixture_types_dir.as_deref(),
        LightingDir::FixtureTypes,
    )?;
    let root = canonical_project_root(&project_root(&state.config_path)?)?;
    let outcome = super::helpers::spawn_blocking_io("read venue scene", move || {
        let system = system_from_files(&root, &types_dir, &venues_dir)?;
        let Some(venue) = system
            .venues_iter()
            .find(|(n, _)| **n == name)
            .map(|(_, v)| v)
        else {
            return Ok::<_, String>(None);
        };
        Ok(Some(venue_scene(&system, venue)))
    })
    .await?;
    match outcome {
        Some(body) => Ok((StatusCode::OK, Json(body)).into_response()),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Venue not found"})),
        )
            .into_response()),
    }
}

/// The scene answer for one venue of a loaded system.
fn venue_scene(
    system: &lighting::system::LightingSystem,
    venue: &lighting::types::Venue,
) -> VenueScene {
    let fixtures = venue
        .fixtures()
        .iter()
        .map(|(name, fixture)| {
            let rig = system
                .resolve_fixture_type(fixture)
                .ok()
                .and_then(|t| t.rig().map(str::to_string));
            let scene = SceneFixture {
                placement: FixturePlacement {
                    tags: fixture.tags().to_vec(),
                    fixture_type: fixture.fixture_type().to_string(),
                    position: fixture.position(),
                    rotation: fixture.rotation(),
                    beam_angle: fixture.beam_angle(),
                    rig,
                },
                mode: fixture.mode().map(String::from),
            };
            (name.clone(), scene)
        })
        .collect();
    VenueScene {
        fixtures,
        venue: super::super::state::stage_venue(system, venue),
    }
}

/// A lighting system loaded from a project's type and venue files alone, as
/// the engine would load them, with no venue made current (so the loader
/// reports nothing on its behalf).
pub(super) fn system_from_files(
    root: &std::path::Path,
    types_dir: &std::path::Path,
    venues_dir: &std::path::Path,
) -> Result<lighting::system::LightingSystem, String> {
    let display = |p: &std::path::Path| p.to_string_lossy().into_owned();
    let config = crate::config::Lighting::new(
        None,
        None,
        Some(crate::config::lighting::Directories::new(
            types_dir.is_dir().then(|| display(types_dir)),
            venues_dir.is_dir().then(|| display(venues_dir)),
        )),
    );
    let mut system = lighting::system::LightingSystem::new();
    system.load(&config, root).map_err(|e| e.to_string())?;
    Ok(system)
}

/// The patch answer for one venue of a loaded system.
fn venue_patch(
    system: &lighting::system::LightingSystem,
    venue: &lighting::types::Venue,
) -> crate::webui::wire::venue::VenuePatch {
    use crate::webui::wire::venue::{PatchedFixture, VenuePatch};
    let spans = venue
        .fixtures_by_patch()
        .into_iter()
        .map(|fixture| PatchedFixture {
            fixture: fixture.name().to_string(),
            universe: fixture.universe(),
            address: fixture.start_channel(),
            footprint: system
                .resolve_fixture_type(fixture)
                .ok()
                .map(|t| t.footprint()),
            fixture_type: fixture.fixture_type().to_string(),
            mode: fixture.mode().map(String::from),
        })
        .collect();
    let known = system.patch_spans(venue);
    VenuePatch {
        spans,
        overlaps: lighting::patch::venue_overlaps(&known),
        overruns: lighting::patch::venue_overruns(&known),
    }
}

/// Converts a JSON venue definition to DSL format.
fn venue_json_to_dsl(name: &str, json: &serde_json::Value) -> Result<String, String> {
    venue_from_json(name, json).map(|venue| format!("{venue}\n"))
}

/// Builds the venue a JSON save describes.
fn venue_from_json(name: &str, json: &serde_json::Value) -> Result<lighting::types::Venue, String> {
    use lighting::types::{Fixture, Vec3, Venue, VenueSource};

    fn vec3(value: &serde_json::Value, what: &str) -> Result<Vec3, String> {
        let items = value
            .as_array()
            .filter(|a| a.len() == 3)
            .ok_or_else(|| format!("{what} must be [x, y, z]"))?;
        let mut out = [0.0; 3];
        for (slot, item) in out.iter_mut().zip(items) {
            *slot = item
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| format!("{what} must be [x, y, z] numbers"))?;
        }
        Ok(out)
    }
    fn optional_vec3(fix: &serde_json::Value, key: &str) -> Result<Option<Vec3>, String> {
        match fix.get(key) {
            None | Some(serde_json::Value::Null) => Ok(None),
            Some(value) => vec3(value, key).map(Some),
        }
    }

    let fixtures = json
        .get("fixtures")
        .and_then(|v| v.as_array())
        .ok_or("Missing 'fixtures' array")?;

    let mut rows = Vec::new();
    for fix in fixtures {
        let field = |key: &str| fix.get(key).ok_or(format!("Fixture missing '{key}'"));
        let text = |key| {
            field(key)?
                .as_str()
                .ok_or(format!("Fixture missing '{key}'"))
        };
        let number = |key| {
            field(key)?
                .as_u64()
                .ok_or(format!("Fixture missing '{key}'"))
        };
        rows.push(lighting::venue_rows::VenueRow {
            name: text("name")?.to_string(),
            fixture_type: text("fixture_type")?.to_string(),
            universe: Some(number("universe")? as f64),
            start_channel: Some(number("start_channel")? as f64),
            mode: None,
        });
    }
    // A row the editor could not name or address is refused, not saved as
    // a fixture nobody can address (or dropped without a word), by the
    // editor's own check of its rows (`venue_rows`).
    if let Some(found) = lighting::venue_rows::venue_row_problems(&rows, |_| false).first() {
        use lighting::venue_rows::RowProblem;
        let row = &rows[found.row as usize];
        let name = &row.name;
        return Err(match found.problems[0] {
            RowProblem::NoName => "A fixture has no name".to_string(),
            RowProblem::DuplicateName => format!("Fixture '{name}' listed more than once"),
            RowProblem::NoType | RowProblem::NoMode => {
                format!("Fixture '{name}' has no fixture type")
            }
            RowProblem::BadUniverse if row.universe == Some(0.0) => {
                format!("Fixture '{name}': universe must be 1 or more")
            }
            RowProblem::BadUniverse => "Fixture 'universe' out of range".to_string(),
            RowProblem::BadAddress if row.start_channel == Some(0.0) => {
                format!("Fixture '{name}': start channel must be 1 or more")
            }
            RowProblem::BadAddress => "Fixture 'start_channel' out of range".to_string(),
        });
    }

    let mut by_name = std::collections::HashMap::new();
    for (fix, row) in fixtures.iter().zip(&rows) {
        let (fix_name, fix_type) = (row.name.as_str(), row.fixture_type.as_str());
        let tags: Vec<String> = fix
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|tags| {
                tags.iter()
                    .filter_map(|t| t.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let fixture = Fixture::new(
            fix_name.to_string(),
            fix_type.to_string(),
            // Both in 1..=65535, by the check above.
            row.universe.unwrap_or_default() as u16,
            row.start_channel.unwrap_or_default() as u16,
            tags,
        )
        .with_position(optional_vec3(fix, "position")?)
        .with_rotation(optional_vec3(fix, "rotation")?)
        // What the unit really throws, in degrees (a drawing hint); absent or
        // null is the type's own. Dropping it here would silently lose it.
        .with_beam_angle(match fix.get("beam_angle") {
            None | Some(serde_json::Value::Null) => None,
            Some(value) => Some(
                value
                    .as_f64()
                    .filter(|v| lighting::fixture_data::beam_angle_valid(*v))
                    .ok_or_else(|| {
                        format!(
                            "Fixture '{fix_name}': 'beam_angle' must be a number of degrees, \
                             greater than 0 and at most {}, or null",
                            lighting::fixture_data::MAX_BEAM_ANGLE
                        )
                    })?,
            ),
        })
        // The fixture's own GDTF mode (§21); absent or null is the type's
        // default. Dropping it here would silently re-mode the fixture.
        .with_mode(match fix.get("mode") {
            None | Some(serde_json::Value::Null) => None,
            Some(mode) => Some(
                mode.as_str()
                    .filter(|m| !m.trim().is_empty())
                    .ok_or("Fixture 'mode' must be a non-empty string or null")?
                    .to_string(),
            ),
        });
        by_name.insert(fix_name.to_string(), fixture);
    }

    let mut focus_points = std::collections::BTreeMap::new();
    if let Some(points) = json.get("focus_points").and_then(|v| v.as_object()) {
        for (focus_name, point) in points {
            focus_points.insert(focus_name.clone(), vec3(point, "focus point")?);
        }
    }
    let source = match json.get("source") {
        None | Some(serde_json::Value::Null) => None,
        Some(source) => Some(VenueSource {
            mvr: source
                .get("mvr")
                .and_then(|v| v.as_str())
                .ok_or("source missing 'mvr'")?
                .to_string(),
            origin: match source.get("origin") {
                None | Some(serde_json::Value::Null) => [0.0; 3],
                Some(origin) => vec3(origin, "source origin")?,
            },
        }),
    };

    Ok(Venue::new(name.to_string(), by_name)
        .with_focus_points(focus_points)
        .with_source(source))
}

#[cfg(test)]
mod test {
    use super::super::router;
    use super::super::test_helpers::*;
    use super::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    /// With no DMX device, the group names still come back — from the config.
    ///
    /// Authoring on a laptop is the normal case for editing a show, and the
    /// groups are declared under `dmx.lighting.groups` in the player config, not
    /// by the engine. Answering `{"groups": []}` there emptied the cue-target
    /// autocomplete with no error to explain it, because the endpoint only read
    /// them off a live engine.
    #[tokio::test]
    async fn lighting_groups_come_from_this_hosts_profile() {
        let (state, dir) = test_state();
        std::fs::write(
            &state.config_path,
            r#"songs: songs
profiles:
  - hostname: not-this-host-x9
    dmx:
      universes: []
      lighting:
        groups:
          elsewhere:
            name: elsewhere
            constraints:
              - AllOf: ["x"]
  - dmx:
      universes: []
      lighting:
        groups:
          here:
            name: here
            constraints:
              - AllOf: ["y"]
"#,
        )
        .unwrap();
        let _keep = dir;
        let response = router()
            .with_state(state)
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/groups")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&response_body(response).await).unwrap();
        let names: Vec<&str> = parsed["groups"]
            .as_array()
            .expect("groups")
            .iter()
            .filter_map(|g| g["name"].as_str())
            .collect();
        assert_eq!(names, vec!["here"], "{parsed}");
    }

    #[tokio::test]
    async fn lighting_groups_come_from_the_config_without_a_dmx_engine() {
        let (state, dir) = test_state();
        std::fs::write(
            &state.config_path,
            "songs: songs\ndmx:\n  universes:\n    - universe: 1\n      name: main\n  \
             lighting:\n    groups:\n      front_wash:\n        \
             name: front_wash\n        constraints:\n          - AllOf: [\"wash\"]\n      \
             back_wash:\n        name: back_wash\n        constraints:\n          \
             - AllOf: [\"back\"]\n",
        )
        .unwrap();
        let _keep = dir;

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/groups")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let parsed: serde_json::Value =
            serde_json::from_str(&response_body(response).await).unwrap();
        let names: Vec<&str> = parsed["groups"]
            .as_array()
            .expect("groups")
            .iter()
            .filter_map(|g| g["name"].as_str())
            .collect();
        assert_eq!(
            names,
            vec!["back_wash", "front_wash"],
            "the declared groups must be listed even with no engine to resolve them"
        );
        // Flagged as unresolved: the names are knowable without an engine, the
        // fixtures each resolves to are not, and a caller has to be able to tell
        // "no fixtures" from "not asked yet".
        assert_eq!(parsed["resolved"], serde_json::json!(false));
        for group in parsed["groups"].as_array().expect("groups") {
            assert!(group["fixtures"].as_array().expect("fixtures").is_empty());
        }
    }

    /// A config that will not load is reported, not answered as "no groups".
    ///
    /// The fallback exists because an empty list was indistinguishable from a
    /// rig with nothing declared. Swallowing a parse error with `.ok()` put that
    /// exact ambiguity back one layer down — a missing `dmx.universes` read as
    /// an empty autocomplete with no explanation.
    #[tokio::test]
    async fn a_config_that_does_not_load_is_reported_not_answered_empty() {
        let (state, dir) = test_state();
        // `dmx.universes` is required, so this parses as YAML and fails to load
        // as config — the shape most likely to be hit by hand-editing.
        std::fs::write(
            &state.config_path,
            "songs: songs\ndmx:\n  lighting:\n    groups:\n      wash:\n        name: wash\n",
        )
        .unwrap();
        let _keep = dir;

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/groups")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let parsed: serde_json::Value =
            serde_json::from_str(&response_body(response).await).unwrap();
        assert!(
            parsed["error"]
                .as_str()
                .is_some_and(|e| e.contains("could not be read")),
            "the reason must reach the caller: {parsed}"
        );
    }

    #[tokio::test]
    async fn get_lighting_file_success() {
        let (state, _dir) = test_state();
        std::fs::write(state.songs_path().join("show.light"), "light content").unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/show.light")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        assert_eq!(body, "light content");
    }

    #[tokio::test]
    async fn get_lighting_file_path_traversal() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/..%2F..%2Fetc%2Fpasswd")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_ne!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_lighting_file_symlink_escape() {
        let (state, _dir) = test_state();
        let outside_dir = tempfile::tempdir().unwrap();
        let secret_file = outside_dir.path().join("secret.light");
        std::fs::write(&secret_file, "secret content").unwrap();
        std::os::unix::fs::symlink(&secret_file, state.songs_path().join("evil.light")).unwrap();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/evil.light")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = response.status();
        assert!(
            status == StatusCode::FORBIDDEN
                || status == StatusCode::BAD_REQUEST
                || status == StatusCode::NOT_FOUND,
            "expected rejection for symlink escape, got {status}"
        );
    }

    #[tokio::test]
    async fn get_lighting_file_not_found() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/nonexistent.light")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_lighting_file_unreadable() {
        use std::os::unix::fs::PermissionsExt;

        let (state, _dir) = test_state();
        let file = state.songs_path().join("unreadable.light");
        std::fs::write(&file, "content").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/unreadable.light")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn validate_lighting_valid() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let body = r#"
show "test" {
    @00:00.000
    lights: static color: "red", duration: 5s
}
"#;
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/lighting/validate")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["valid"], true);
    }

    #[tokio::test]
    async fn validate_lighting_invalid() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/lighting/validate")
                    .body(Body::from("invalid {{{ content"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn validate_lighting_resolves_includes_from_the_named_file() {
        let (state, _dir) = test_state();
        let songs = state.songs_path().clone();
        std::fs::create_dir_all(songs.join("song/lighting")).unwrap();
        std::fs::write(
            songs.join("common.light"),
            "sequence \"wash\" {\n    @00:00.000\n    lights: static color: \"red\", duration: 5s\n}\n",
        )
        .unwrap();
        std::fs::write(songs.parent().unwrap().join("outside.light"), "").unwrap();
        let validate = |file: &str, body: &'static str| {
            let app = router().with_state(state.clone());
            let uri = format!("/lighting/validate?file={file}");
            async move {
                let response = app
                    .oneshot(
                        http::Request::builder()
                            .method("POST")
                            .uri(uri)
                            .body(Body::from(body))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                (response.status(), response_body(response).await)
            }
        };
        let show = "include \"../../common.light\"\nshow \"test\" {\n    @00:00.000\n    sequence \"wash\"\n}\n";

        let (status, body) = validate("song/lighting/show.light", show).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        // From another folder the same include misses.
        let (status, _) = validate("song/show.light", show).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        // And it may not leave the songs folder.
        let (status, body) = validate("song/show.light", "include \"../../outside.light\"\n").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body.contains("outside the songs folder"), "{body}");
    }

    #[tokio::test]
    async fn put_lighting_file_valid() {
        let (state, _dir) = test_state();
        let file_path = state.songs_path().join("new.light");
        let content =
            "show \"test\" {\n    @00:00.000\n    lights: static color: \"red\", duration: 5s\n}\n";
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/lighting/new.light")
                    .body(Body::from(content))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(file_path.exists());
    }

    #[tokio::test]
    async fn put_lighting_file_path_traversal() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/lighting/..%2F..%2Fevil.light")
                    .body(Body::from("content"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_ne!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn put_lighting_file_outside_base() {
        let (state, _dir) = test_state();
        let outside_dir = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(outside_dir.path(), state.songs_path().join("escape")).unwrap();

        let content =
            "show \"test\" {\n    @00:00.000\n    lights: static color: \"red\", duration: 5s\n}\n";
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/lighting/escape%2Fevil.light")
                    .body(Body::from(content))
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = response.status();
        assert!(
            status == StatusCode::FORBIDDEN
                || status == StatusCode::BAD_REQUEST
                || status == StatusCode::NOT_FOUND,
            "expected rejection for symlink escape, got {status}"
        );
    }

    /// A project whose songs root is the project itself (`songs: "."`, as
    /// on the harness), so the light-show endpoint reaches the lighting
    /// directories; `config` is its mtrack.yaml.
    fn light_shows_at_the_root(config: &str) -> (axum::Router, tempfile::TempDir) {
        let (state, dir) = test_state();
        std::fs::write(dir.path().join("mtrack.yaml"), config).unwrap();
        let mut sources = state.player.song_sources().unwrap_or_default();
        sources.songs = dir.path().to_path_buf();
        state.player.set_song_sources(sources);
        for sub in [
            "lighting/venues",
            "lighting/fixture_types",
            "lighting/x",
            "shows",
        ] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        std::fs::write(
            dir.path().join("lighting/venues/club.light"),
            "venue \"club\" {\n  fixture \"A\" Par @ 1:1\n}\n",
        )
        .unwrap();
        (router().with_state(state), dir)
    }

    async fn light_show_call(app: &axum::Router, method: &str, path: &str) -> (StatusCode, String) {
        let show =
            "show \"test\" {\n    @00:00.000\n    lights: static color: \"red\", duration: 5s\n}\n";
        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(format!("/lighting/{path}"))
                    .body(Body::from(show))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        (status, response_body(response).await)
    }

    #[tokio::test]
    async fn the_light_show_endpoint_writes_no_venue_or_type_file() {
        let (app, dir) = light_shows_at_the_root("songs: \".\"\n");
        let club = dir.path().join("lighting/venues/club.light");
        let before = std::fs::read_to_string(&club).unwrap();
        std::os::unix::fs::symlink(dir.path().join("lighting/venues"), dir.path().join("alias"))
            .unwrap();
        for path in [
            "lighting%2Fvenues%2Fclub.light",
            "lighting%2Fvenues%2Fnew.light",
            "lighting%2Ffixture_types%2Fnew.light",
            "lighting%2Fx%2F..%2Fvenues%2Fnew.light",
            "shows%2F..%2Flighting%2Fvenues%2Fclub.light",
            "alias%2Fclub.light",
            "alias%2Fnew.light",
        ] {
            let (status, body) = light_show_call(&app, "PUT", path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "PUT {path}: {body}");
            assert!(body.contains("song light shows"), "PUT {path}: {body}");
        }
        for path in [
            "lighting%2Fvenues%2Fclub.light",
            "lighting%2Fx%2F..%2Fvenues%2Fclub.light",
            "alias%2Fclub.light",
        ] {
            let (status, body) = light_show_call(&app, "DELETE", path).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "DELETE {path}: {body}");
        }
        // Another case is another directory here (a case-sensitive file
        // system), so there is nothing to write into; on one that folds
        // case it is the venues directory, which identity finds.
        for path in [
            "Lighting%2FVenues%2Fnew.light",
            "lighting%2FVENUES%2Fclub.light",
        ] {
            let (status, body) = light_show_call(&app, "PUT", path).await;
            assert_ne!(status, StatusCode::OK, "PUT {path}: {body}");
        }
        assert_eq!(std::fs::read_to_string(&club).unwrap(), before);
        let venues: Vec<_> = std::fs::read_dir(dir.path().join("lighting/venues"))
            .unwrap()
            .collect();
        assert_eq!(venues.len(), 1, "nothing new in the venues directory");
        assert!(!dir.path().join("lighting/fixture_types/new.light").exists());

        // A song's light show elsewhere is still written and deleted.
        let (status, body) = light_show_call(&app, "PUT", "shows%2Fa.light").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, body) = light_show_call(&app, "DELETE", "shows%2Fa.light").await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    #[test]
    fn a_path_is_within_a_dir_by_identity() {
        let dir = tempfile::tempdir().unwrap();
        let venues = dir.path().join("lighting/venues");
        std::fs::create_dir_all(&venues).unwrap();
        std::fs::create_dir_all(dir.path().join("other")).unwrap();
        std::os::unix::fs::symlink(&venues, dir.path().join("alias")).unwrap();
        let within = |p: &str| crate::util::is_within_dir(&dir.path().join(p), &venues);
        assert!(within("lighting/venues/new.light"));
        assert!(within("lighting/venues/deep/er/new.light"));
        assert!(within("other/../lighting/venues/new.light"));
        assert!(within("alias/new.light"));
        assert!(!within("other/new.light"));
        assert!(!within("lighting/new.light"));
        assert!(!crate::util::is_within_dir(
            &dir.path().join("x"),
            &dir.path().join("missing")
        ));
    }

    #[tokio::test]
    async fn the_light_show_endpoint_refuses_the_hosts_configured_dirs() {
        let config = "songs: \".\"\nprofiles:\n  - hostname: not-this-host-x9\n    dmx:\n      universes: []\n      lighting:\n        directories:\n          venues: shows\n  - dmx:\n      universes: []\n      lighting:\n        directories:\n          venues: rig/venues\n";
        let (app, dir) = light_shows_at_the_root(config);
        std::fs::create_dir_all(dir.path().join("rig/venues")).unwrap();
        let (status, body) = light_show_call(&app, "PUT", "rig%2Fvenues%2Fnew.light").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body.contains("venues directory (rig/venues)"), "{body}");
        // The other host's directory, and the default one this host does
        // not read, are not this host's venues.
        let (status, body) = light_show_call(&app, "PUT", "shows%2Fa.light").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, body) = light_show_call(&app, "PUT", "lighting%2Fvenues%2Fnew.light").await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    /// A config that does not load is a 500 here as at every endpoint that
    /// reads the lighting directories, not the defaults, which may not be
    /// this host's.
    #[tokio::test]
    async fn the_light_show_endpoint_answers_a_broken_config_with_a_500() {
        let (app, _dir) = light_shows_at_the_root("profiles: [\n");
        let (status, body) = light_show_call(&app, "PUT", "shows%2Fa.light").await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{body}");
        assert!(body.contains("the config does not load"), "{body}");
    }

    #[tokio::test]
    async fn put_lighting_file_invalid_dsl() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/lighting/test.light")
                    .body(Body::from("invalid {{{ content"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[test]
    fn find_light_files_discovers_files() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        std::fs::create_dir(base.join("song1")).unwrap();
        std::fs::write(base.join("song1/show.light"), "content").unwrap();
        std::fs::write(base.join("top.light"), "content").unwrap();
        std::fs::write(base.join("not_a_light.txt"), "content").unwrap();

        let mut results = Vec::new();
        find_light_files(base, base, &mut results).unwrap();

        assert_eq!(results.len(), 2);
        let paths: Vec<&str> = results.iter().map(|r| r.path.as_str()).collect();
        assert!(paths.contains(&"song1/show.light"));
        assert!(paths.contains(&"top.light"));
    }

    /// A light show's extension is read in any case: `Show.LIGHT` is listed,
    /// as `songs --init` registers it.
    #[test]
    fn find_light_files_reads_extensions_in_any_case() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("Show.LIGHT"), "content").unwrap();
        std::fs::write(dir.path().join("show.lights"), "content").unwrap();

        let mut results = Vec::new();
        find_light_files(dir.path(), dir.path(), &mut results).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "Show.LIGHT");
        assert_eq!(results[0].name, "Show");
    }

    /// A request to the lighting endpoints, its status and body.
    async fn lighting_request(
        state: &WebUiState,
        method: &str,
        uri: &str,
        body: &str,
    ) -> (StatusCode, String) {
        let response = router()
            .with_state(state.clone())
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        (response.status(), response_body(response).await)
    }

    /// A light show that is a link is deleted as the link: what it leads
    /// to, another song's config or show, is left.
    #[tokio::test]
    async fn delete_lighting_file_removes_a_link_not_its_target() {
        let (state, _dir) = test_state();
        let songs = state.songs_path();
        std::fs::create_dir_all(songs.join("a/lighting")).unwrap();
        std::fs::create_dir_all(songs.join("b/lighting")).unwrap();
        std::fs::write(songs.join("a/lighting/show.light"), "a's show").unwrap();
        std::fs::write(songs.join("b/song.yaml"), "name: B\n").unwrap();
        std::os::unix::fs::symlink("../song.yaml", songs.join("b/lighting/show.light")).unwrap();
        std::os::unix::fs::symlink(
            "../../a/lighting/show.light",
            songs.join("b/lighting/a.light"),
        )
        .unwrap();

        for file in ["show.light", "a.light"] {
            let uri = format!("/lighting/b%2Flighting%2F{file}");
            let (status, body) = lighting_request(&state, "DELETE", &uri, "").await;
            assert_eq!(status, StatusCode::OK, "{file}: {body}");
            assert!(
                std::fs::symlink_metadata(songs.join("b/lighting").join(file)).is_err(),
                "{file}: the link is gone"
            );
        }
        assert_eq!(
            std::fs::read_to_string(songs.join("b/song.yaml")).unwrap(),
            "name: B\n"
        );
        assert_eq!(
            std::fs::read_to_string(songs.join("a/lighting/show.light")).unwrap(),
            "a's show"
        );
    }

    /// Only `.light` files are written as light shows: not a song's config
    /// by name.
    #[tokio::test]
    async fn put_lighting_file_refuses_a_file_that_is_no_show() {
        let (state, _dir) = test_state();
        let songs = state.songs_path();
        std::fs::create_dir_all(songs.join("b")).unwrap();
        std::fs::write(songs.join("b/song.yaml"), "name: B\n").unwrap();
        let show =
            "show \"test\" {\n    @00:00.000\n    lights: static color: \"red\", duration: 5s\n}\n";

        let (status, body) = lighting_request(&state, "PUT", "/lighting/b%2Fsong.yaml", show).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            std::fs::read_to_string(songs.join("b/song.yaml")).unwrap(),
            "name: B\n"
        );
    }

    /// A bar/beat show is checked against its song's tempo when the song's
    /// folder is spelled through a link (the songs folder reached through
    /// one): the show's path, resolved, and the song's folder are compared
    /// resolved. The loader resolves the folders it loads, so this is a
    /// song built from an unresolved path.
    #[tokio::test]
    async fn put_lighting_file_finds_the_songs_tempo_through_a_linked_folder() {
        let outer = tempfile::tempdir().unwrap();
        let real = outer.path().join("real");
        std::fs::create_dir_all(real.join("t/lighting")).unwrap();
        std::fs::write(
            real.join("t/song.yaml"),
            "name: T\ntracks: []\ntempo:\n  bpm: 120\n  time_signature: 4/4\n",
        )
        .unwrap();
        let link = outer.path().join("songs-link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let config = crate::config::Song::deserialize(&link.join("t/song.yaml")).unwrap();
        let song = crate::songs::Song::new(&link.join("t"), &config).unwrap();
        assert!(song.lighting_tempo_map().is_some());
        let songs = crate::songs::Songs::new(std::collections::HashMap::from([(
            "T".to_string(),
            std::sync::Arc::new(song),
        )]));
        let (state, _dir) = test_state_with_registry(std::sync::Arc::new(songs));
        {
            let mut sources = state.player.song_sources().unwrap_or_default();
            sources.songs = link;
            state.player.set_song_sources(sources);
        }
        let show =
            "show \"bars\" {\n    @1/1\n    lights: static color: \"red\", duration: 1beat\n}\n";

        let (status, body) =
            lighting_request(&state, "PUT", "/lighting/t%2Flighting%2Fshow.light", show).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    /// A light show that is a link leading nowhere is not written: the
    /// write would make its target, outside the songs folder.
    #[tokio::test]
    async fn put_lighting_file_refuses_a_link_leading_nowhere() {
        let (state, _dir) = test_state();
        let songs = state.songs_path();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("x.light");
        std::fs::create_dir_all(songs.join("foo/lighting")).unwrap();
        std::os::unix::fs::symlink(&target, songs.join("foo/lighting/show.light")).unwrap();
        let show =
            "show \"test\" {\n    @00:00.000\n    lights: static color: \"red\", duration: 5s\n}\n";

        let (status, body) =
            lighting_request(&state, "PUT", "/lighting/foo%2Flighting%2Fshow.light", show).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert!(!target.exists(), "written outside");
    }

    /// A light show saved through a link has its `include`s resolved
    /// where the song loader resolves them: beside the link, not beside
    /// what it leads to.
    #[tokio::test]
    async fn put_lighting_file_resolves_includes_beside_a_link() {
        let (state, _dir) = test_state();
        let songs = state.songs_path();
        std::fs::create_dir_all(songs.join("a/lighting")).unwrap();
        std::fs::create_dir_all(songs.join("b/lighting")).unwrap();
        let shared = "sequence \"wash\" {\n    @00:00.000\n    lights: static color: \"red\", duration: 5s\n}\n";
        std::fs::write(songs.join("a/lighting/common.light"), shared).unwrap();
        std::fs::write(songs.join("a/lighting/show.light"), "").unwrap();
        std::os::unix::fs::symlink(
            "../../a/lighting/show.light",
            songs.join("b/lighting/show.light"),
        )
        .unwrap();
        let show =
            "include \"common.light\"\nshow \"test\" {\n    @00:00.000\n    sequence \"wash\"\n}\n";

        // Beside B's link there is no common.light: the loader would fail.
        let (status, body) =
            lighting_request(&state, "PUT", "/lighting/b%2Flighting%2Fshow.light", show).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(
            std::fs::read_to_string(songs.join("a/lighting/show.light")).unwrap(),
            ""
        );
        // Saved as A's, it is found.
        let (status, body) =
            lighting_request(&state, "PUT", "/lighting/a%2Flighting%2Fshow.light", show).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    /// The editor deletes a `Show.LIGHT` as it does a `show.light`; a file
    /// that is no light show is still refused.
    #[tokio::test]
    async fn delete_lighting_file_reads_extensions_in_any_case() {
        let (state, _dir) = test_state();
        let show = state.songs_path().join("Show.LIGHT");
        std::fs::write(&show, "content").unwrap();
        let notes = state.songs_path().join("notes.txt");
        std::fs::write(&notes, "content").unwrap();

        for (uri, expected) in [
            ("/lighting/Show.LIGHT", StatusCode::OK),
            ("/lighting/notes.txt", StatusCode::BAD_REQUEST),
        ] {
            let response = router()
                .with_state(state.clone())
                .oneshot(
                    http::Request::builder()
                        .method("DELETE")
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{uri}");
        }
        assert!(!show.exists());
        assert!(notes.exists());
    }

    #[test]
    fn find_light_files_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let mut results = Vec::new();
        find_light_files(dir.path(), dir.path(), &mut results).unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn find_light_files_extracts_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("my_show.light"), "content").unwrap();

        let mut results = Vec::new();
        find_light_files(dir.path(), dir.path(), &mut results).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "my_show");
    }

    #[test]
    fn find_light_files_deeply_nested() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        let deep_dir = base.join("a").join("b").join("c");
        std::fs::create_dir_all(&deep_dir).unwrap();
        std::fs::write(deep_dir.join("deep.light"), "content").unwrap();

        let mut results = Vec::new();
        find_light_files(base, base, &mut results).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].path, "a/b/c/deep.light");
    }

    #[test]
    fn find_light_files_nonexistent_dir() {
        let results_vec = &mut Vec::new();
        let result = find_light_files(
            std::path::Path::new("/nonexistent"),
            std::path::Path::new("/nonexistent"),
            results_vec,
        );
        assert!(result.is_ok());
        assert!(results_vec.is_empty());
    }

    #[test]
    fn find_light_files_multiple_extensions_only_light() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        std::fs::write(base.join("show.light"), "content").unwrap();
        std::fs::write(base.join("show.yaml"), "content").unwrap();
        std::fs::write(base.join("show.txt"), "content").unwrap();
        std::fs::write(base.join("show.mid"), "content").unwrap();

        let mut results = Vec::new();
        find_light_files(base, base, &mut results).unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "show");
    }

    #[tokio::test]
    async fn get_lighting_file_path_traversal_via_dotdot() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/..%2Fpasswd")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // Path traversal: expect rejection (NOT_FOUND, FORBIDDEN, or BAD_REQUEST).
        let status = response.status();
        assert!(
            status == StatusCode::NOT_FOUND
                || status == StatusCode::FORBIDDEN
                || status == StatusCode::BAD_REQUEST,
            "expected rejection, got {status}"
        );
    }

    #[tokio::test]
    async fn get_lighting_file_not_found_body_contains_name() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/missing.light")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["error"]
            .as_str()
            .unwrap()
            .contains("Lighting file not found"));
    }

    #[tokio::test]
    async fn get_lighting_file_unreadable_body_contains_message() {
        use std::os::unix::fs::PermissionsExt;

        let (state, _dir) = test_state();
        let file = state.songs_path().join("broken.light");
        std::fs::write(&file, "content").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/broken.light")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["error"]
            .as_str()
            .unwrap()
            .contains("Failed to read lighting file"));
    }

    #[tokio::test]
    async fn put_lighting_file_path_traversal_returns_invalid_path() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/lighting/..%2Fevil.light")
                    .body(Body::from(
                        "show \"test\" {\n    @00:00.000\n    lights: static color: \"red\", duration: 5s\n}\n",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = response.status();
        assert!(
            status == StatusCode::NOT_FOUND
                || status == StatusCode::FORBIDDEN
                || status == StatusCode::BAD_REQUEST,
            "expected rejection for path traversal, got {status}"
        );
    }

    #[tokio::test]
    async fn put_lighting_file_write_failure_returns_500() {
        use std::os::unix::fs::PermissionsExt;

        let (state, _dir) = test_state();
        let sub = state.songs_path().join("readonly");
        std::fs::create_dir(&sub).unwrap();
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o555)).unwrap();

        let content =
            "show \"test\" {\n    @00:00.000\n    lights: static color: \"red\", duration: 5s\n}\n";
        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/lighting/readonly%2Ftest.light")
                    .body(Body::from(content))
                    .unwrap(),
            )
            .await
            .unwrap();

        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["error"].is_string());
    }

    // -----------------------------------------------------------------------
    // Helper function to create a fixture type DSL string for tests.
    // -----------------------------------------------------------------------
    fn sample_fixture_type_dsl(name: &str) -> String {
        format!(
            r#"fixture_type "{name}" {{
  channels: 3
  channel_map: {{
    "red": 1,
    "green": 2,
    "blue": 3
  }}
}}"#
        )
    }

    /// The rich channel form, which only a `.fixture` file may hold.
    fn sample_rich_fixture_type_dsl(name: &str) -> String {
        format!(
            "fixture_type \"{name}\" {{\n  channel \"pan\" @ 1 fine 2 range -270deg..270deg\n}}\n"
        )
    }

    /// A GDTF-referential type. It parses without the archive — expansion
    /// happens in the lighting system, so its `channels` map is empty here.
    fn sample_referential_fixture_type_dsl(name: &str) -> String {
        format!("fixture_type \"{name}\" from gdtf(\"library/synth.gdtf\") {{\n}}\n")
    }

    // Helper function to create a venue DSL string for tests.
    fn sample_venue_dsl(name: &str) -> String {
        format!(
            r#"venue "{name}" {{
  fixture "Spot1" GenericPar @ 1:1
  fixture "Spot2" GenericPar @ 1:5
}}"#
        )
    }

    // -----------------------------------------------------------------------
    // Unit tests: validate_lighting_name
    // -----------------------------------------------------------------------

    #[test]
    fn validate_lighting_name_valid() {
        assert!(validate_lighting_name("my-fixture").is_ok());
        assert!(validate_lighting_name("Venue_01").is_ok());
        assert!(validate_lighting_name("simple").is_ok());
    }

    #[test]
    fn validate_lighting_name_rejects_route_segments() {
        for name in ["lock", "validate", "activate"] {
            assert!(validate_lighting_name(name).is_err(), "{name}");
        }
        assert!(validate_lighting_name("locker").is_ok());
    }

    #[test]
    fn validate_lighting_name_invalid_empty() {
        assert!(validate_lighting_name("").is_err());
    }

    #[test]
    fn validate_lighting_name_invalid_dots() {
        assert!(validate_lighting_name("..").is_err());
        assert!(validate_lighting_name("a/../b").is_err());
    }

    #[test]
    fn validate_lighting_name_invalid_slashes() {
        assert!(validate_lighting_name("a/b").is_err());
        assert!(validate_lighting_name("a\\b").is_err());
    }

    #[test]
    fn validate_lighting_name_invalid_null() {
        assert!(validate_lighting_name("a\0b").is_err());
    }

    // -----------------------------------------------------------------------
    // Unit tests: fixture_type_json_to_dsl
    // -----------------------------------------------------------------------

    #[test]
    fn fixture_type_json_to_dsl_basic() {
        let json = serde_json::json!({
            "channels": {
                "red": 1,
                "green": 2,
                "blue": 3
            }
        });
        let dsl = fixture_type_json_to_dsl("TestFixture", &json).unwrap();
        assert!(dsl.contains("fixture_type \"TestFixture\""));
        assert!(dsl.contains("channels: 3"));
        assert!(dsl.contains("\"red\": 1"));
        assert!(dsl.contains("\"green\": 2"));
        assert!(dsl.contains("\"blue\": 3"));
        // Verify the DSL actually parses.
        let types = lighting::parser::parse_fixture_types(&dsl).unwrap();
        assert!(types.contains_key("TestFixture"));
    }

    #[test]
    fn fixture_type_json_to_dsl_with_strobe() {
        let json = serde_json::json!({
            "channels": {
                "dimmer": 1,
                "strobe": 2
            },
            "max_strobe_frequency": 25.0,
            "min_strobe_frequency": 0.5,
            "strobe_dmx_offset": 10
        });
        let dsl = fixture_type_json_to_dsl("StrobeLight", &json).unwrap();
        assert!(dsl.contains("max_strobe_frequency: 25"));
        assert!(dsl.contains("min_strobe_frequency: 0.5"));
        assert!(dsl.contains("strobe_dmx_offset: 10"));
        // Verify the DSL actually parses.
        let types = lighting::parser::parse_fixture_types(&dsl).unwrap();
        let ft = types.get("StrobeLight").unwrap();
        assert_eq!(ft.max_strobe_frequency(), Some(25.0));
    }

    #[test]
    fn fixture_type_json_to_dsl_missing_channels() {
        let json = serde_json::json!({"foo": "bar"});
        assert!(fixture_type_json_to_dsl("Bad", &json).is_err());
    }

    // -----------------------------------------------------------------------
    // Unit tests: venue_json_to_dsl
    // -----------------------------------------------------------------------

    #[test]
    fn venue_json_to_dsl_basic() {
        let json = serde_json::json!({
            "fixtures": [
                {
                    "name": "Spot1",
                    "fixture_type": "GenericPar",
                    "universe": 1,
                    "start_channel": 1
                }
            ]
        });
        let dsl = venue_json_to_dsl("TestVenue", &json).unwrap();
        assert!(dsl.contains("venue \"TestVenue\""));
        assert!(dsl.contains("fixture \"Spot1\" GenericPar @ 1:1"));
        // Verify the DSL actually parses.
        let venues = lighting::parser::parse_venues(&dsl).unwrap();
        assert!(venues.contains_key("TestVenue"));
    }

    #[test]
    fn venue_json_to_dsl_carries_beam_angle() {
        let venue = |beam: serde_json::Value| {
            let mut fixture = serde_json::json!({
                "name": "Brick1", "fixture_type": "Brick", "universe": 1, "start_channel": 1,
                "rotation": [60, 0, 0],
            });
            if !beam.is_null() {
                fixture["beam_angle"] = beam;
            }
            serde_json::json!({"fixtures": [fixture]})
        };
        let dsl = venue_json_to_dsl("V", &venue(serde_json::json!(60))).unwrap();
        assert!(
            dsl.contains("rotation (60, 0, 0) beam_angle 60\n"),
            "after rotation, no trailing .0: {dsl}"
        );
        let dsl = venue_json_to_dsl("V", &venue(serde_json::json!(22.5))).unwrap();
        assert!(dsl.contains("beam_angle 22.5"), "{dsl}");

        for absent in [serde_json::Value::Null, serde_json::json!(null)] {
            let dsl = venue_json_to_dsl("V", &venue(absent)).unwrap();
            assert!(!dsl.contains("beam_angle"), "{dsl}");
        }
        let mut explicit_null = venue(serde_json::json!(60));
        explicit_null["fixtures"][0]["beam_angle"] = serde_json::Value::Null;
        assert!(!venue_json_to_dsl("V", &explicit_null)
            .unwrap()
            .contains("beam_angle"));

        for bad in [
            serde_json::json!("wide"),
            serde_json::json!(0),
            serde_json::json!(-3),
            serde_json::json!(181),
            serde_json::json!(true),
        ] {
            let err = venue_json_to_dsl("V", &venue(bad.clone())).unwrap_err();
            assert!(
                err.contains("'beam_angle'") && err.contains("Brick1"),
                "{bad}: {err}"
            );
        }
    }

    #[test]
    fn a_venue_with_a_beam_angle_needs_the_venue_extension() {
        let venue = lighting::parser::parse_venues(
            "venue \"v\" {\n  fixture \"A\" T @ 1:1 beam_angle 60\n}\n",
        )
        .unwrap()
        .remove("v")
        .unwrap();
        assert!(lighting_writes::needs_venue_extension(&venue));
    }

    #[test]
    fn venue_json_to_dsl_with_tags() {
        let json = serde_json::json!({
            "fixtures": [
                {
                    "name": "Wash1",
                    "fixture_type": "Par",
                    "universe": 1,
                    "start_channel": 1,
                    "tags": ["front", "wash"]
                }
            ]
        });
        let dsl = venue_json_to_dsl("Tagged", &json).unwrap();
        assert!(dsl.contains("tags [\"front\", \"wash\"]"));
        // Verify the DSL actually parses.
        let venues = lighting::parser::parse_venues(&dsl).unwrap();
        let v = venues.get("Tagged").unwrap();
        let w1 = v.fixtures().get("Wash1").unwrap();
        assert_eq!(w1.tags(), &["front", "wash"]);
    }

    #[test]
    fn venue_json_to_dsl_ignores_groups() {
        // Venue groups are gone. A stale client still sending them must not
        // produce a file the parser then rejects.
        let json = serde_json::json!({
            "fixtures": [
                {
                    "name": "L1",
                    "fixture_type": "Par",
                    "universe": 1,
                    "start_channel": 1
                }
            ],
            "groups": {
                "front": ["L1"]
            }
        });
        let dsl = venue_json_to_dsl("Grouped", &json).unwrap();
        assert!(
            !dsl.contains("group"),
            "groups should not be emitted: {dsl}"
        );
        lighting::parser::parse_venues(&dsl).expect("emitted DSL must parse");
    }

    #[test]
    fn venue_json_to_dsl_missing_fixtures() {
        let json = serde_json::json!({"foo": "bar"});
        assert!(venue_json_to_dsl("Bad", &json).is_err());
    }

    #[test]
    fn venue_json_to_dsl_fixture_missing_name() {
        let json = serde_json::json!({
            "fixtures": [
                {
                    "fixture_type": "Par",
                    "universe": 1,
                    "start_channel": 1
                }
            ]
        });
        assert!(venue_json_to_dsl("Bad", &json).is_err());
    }

    // -----------------------------------------------------------------------
    // Fixture Types endpoint tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_fixture_types_empty() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_empty");
        std::fs::create_dir(&ft_dir).unwrap();
        let rel = ft_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/fixture-types?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["fixture_types"].is_object());
        assert_eq!(parsed["fixture_types"].as_object().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn get_fixture_types_nonexistent_dir() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/fixture-types?dir=nonexistent_dir")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["fixture_types"].as_object().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn get_fixture_types_with_files() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_test");
        std::fs::create_dir(&ft_dir).unwrap();
        std::fs::write(
            ft_dir.join("led_par.light"),
            sample_fixture_type_dsl("LED_Par"),
        )
        .unwrap();
        std::fs::write(
            ft_dir.join("mover.fixture"),
            sample_rich_fixture_type_dsl("Mover"),
        )
        .unwrap();
        std::fs::write(
            ft_dir.join("brick.fixture"),
            sample_referential_fixture_type_dsl("Brick"),
        )
        .unwrap();
        let rel = ft_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/fixture-types?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let par = &parsed["fixture_types"]["LED_Par"];
        assert!(par["fixture_type"].is_object(), "{parsed}");
        assert_eq!(par["file"], "led_par.light");
        assert_eq!(par["extension"], "light");
        assert_eq!(par["referential"], false);
        assert_eq!(par["rich"], false);

        let mover = &parsed["fixture_types"]["Mover"];
        assert_eq!(mover["file"], "mover.fixture");
        assert_eq!(mover["extension"], "fixture");
        assert_eq!(mover["rich"], true);
        assert_eq!(mover["referential"], false);

        // A referential type has no channels of its own here; only the
        // lighting system's expansion resolves them from the archive.
        let brick = &parsed["fixture_types"]["Brick"];
        assert_eq!(brick["referential"], true);
        assert_eq!(brick["extension"], "fixture");
        assert_eq!(
            brick["fixture_type"]["channels"].as_object().unwrap().len(),
            0
        );
    }

    #[tokio::test]
    async fn get_fixture_type_success() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_get");
        std::fs::create_dir(&ft_dir).unwrap();
        std::fs::write(
            ft_dir.join("led_par.light"),
            sample_fixture_type_dsl("LED_Par"),
        )
        .unwrap();
        let rel = ft_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/fixture-types/LED_Par?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["fixture_type"].is_object());
        assert!(parsed["dsl"].is_string());
        assert_eq!(parsed["file"], "led_par.light");
        assert_eq!(parsed["extension"], "light");
        assert_eq!(parsed["referential"], false);
        assert_eq!(parsed["rich"], false);
    }

    #[tokio::test]
    async fn get_fixture_type_reads_a_fixture_file() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_get_fixture");
        std::fs::create_dir(&ft_dir).unwrap();
        std::fs::write(
            ft_dir.join("brick.fixture"),
            sample_referential_fixture_type_dsl("Brick"),
        )
        .unwrap();
        let rel = ft_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/fixture-types/Brick?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["file"], "brick.fixture");
        assert_eq!(parsed["extension"], "fixture");
        assert_eq!(parsed["referential"], true);
        assert!(
            parsed["dsl"].as_str().unwrap().contains("from gdtf("),
            "{parsed}"
        );
    }

    /// A fixture types directory holding one file, named as `import-gdtf`
    /// used to name it (the harness's `astera_pixelbrick.fixture`), which is
    /// not the name's own stem.
    fn imported_fixture_type_dir(
        project: &std::path::Path,
        rel: &str,
        name: &str,
        dsl: String,
    ) -> std::path::PathBuf {
        let stem = lighting::files::stem(name).replace('-', "_");
        assert_ne!(stem, lighting::files::stem(name), "the stems must differ");
        let dir = project.join(rel);
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join(format!("{stem}.fixture"));
        std::fs::write(&file, dsl).unwrap();
        file
    }

    async fn fixture_type_request(
        state: WebUiState,
        method: &str,
        uri: String,
        content_type: &str,
        body: Body,
    ) -> (StatusCode, serde_json::Value) {
        let response = router()
            .with_state(state)
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", content_type)
                    .body(body)
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = response_body(response).await;
        (status, serde_json::from_str(&body).unwrap_or_default())
    }

    #[tokio::test]
    async fn get_fixture_type_finds_an_imported_file_by_its_declaration() {
        let (state, _dir) = test_state();
        let rel = "ft_get_imported";
        imported_fixture_type_dir(
            _dir.path(),
            rel,
            "Astera-PixelBrick",
            sample_referential_fixture_type_dsl("Astera-PixelBrick"),
        );

        let (status, parsed) = fixture_type_request(
            state,
            "GET",
            format!("/lighting/fixture-types/Astera-PixelBrick?dir={rel}"),
            "text/plain",
            Body::empty(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["file"], "astera_pixelbrick.fixture");
        assert_eq!(parsed["referential"], true);
    }

    #[tokio::test]
    async fn put_fixture_type_saves_back_to_the_file_the_type_lives_in() {
        let (state, _dir) = test_state();
        let rel = "ft_put_imported";
        let file = imported_fixture_type_dir(
            _dir.path(),
            rel,
            "Astera-PixelBrick",
            sample_rich_fixture_type_dsl("Astera-PixelBrick"),
        );
        let edited = sample_rich_fixture_type_dsl("Astera-PixelBrick").replace("270", "180");

        let (status, parsed) = fixture_type_request(
            state,
            "PUT",
            format!("/lighting/fixture-types/Astera-PixelBrick?dir={rel}&ext=fixture"),
            "text/plain",
            Body::from(edited.clone()),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), edited);
        // One type, one file: no second file at this API's own stem.
        assert_eq!(std::fs::read_dir(_dir.path().join(rel)).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn delete_fixture_type_removes_an_imported_file() {
        let (state, _dir) = test_state();
        let rel = "ft_del_imported";
        let file = imported_fixture_type_dir(
            _dir.path(),
            rel,
            "Astera-PixelBrick",
            sample_referential_fixture_type_dsl("Astera-PixelBrick"),
        );

        let (status, parsed) = fixture_type_request(
            state,
            "DELETE",
            format!("/lighting/fixture-types/Astera-PixelBrick?dir={rel}"),
            "text/plain",
            Body::empty(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert!(!file.exists());
    }

    #[tokio::test]
    async fn fixture_type_in_a_shared_file_is_read_but_not_deleted_or_form_saved() {
        let (state, _dir) = test_state();
        let rel = "ft_shared";
        let dir = _dir.path().join(rel);
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("house.light");
        let both = format!(
            "{}\n{}\n",
            sample_fixture_type_dsl("ParA"),
            sample_fixture_type_dsl("ParB")
        );
        std::fs::write(&file, &both).unwrap();

        let (status, parsed) = fixture_type_request(
            state.clone(),
            "GET",
            format!("/lighting/fixture-types/ParB?dir={rel}"),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["file"], "house.light");

        let (status, parsed) = fixture_type_request(
            state.clone(),
            "DELETE",
            format!("/lighting/fixture-types/ParB?dir={rel}"),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert!(parsed["error"].as_str().unwrap().contains("\"ParA\""));

        let form = serde_json::json!({"channels": {"red": 1}});
        let (status, parsed) = fixture_type_request(
            state,
            "PUT",
            format!("/lighting/fixture-types/ParB?dir={rel}"),
            "application/json",
            Body::from(serde_json::to_vec(&form).unwrap()),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");

        assert_eq!(std::fs::read_to_string(&file).unwrap(), both);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn delete_venue_finds_an_imported_file_by_its_declaration() {
        let (state, _dir) = test_state();
        let rel = "v_del_imported";
        let dir = _dir.path().join(rel);
        std::fs::create_dir(&dir).unwrap();
        // Not the name's stem (`house-rig`).
        let file = dir.join("house_rig.light");
        std::fs::write(&file, sample_venue_dsl("House-Rig")).unwrap();

        let (status, parsed) = fixture_type_request(
            state,
            "DELETE",
            format!("/lighting/venues/House-Rig?dir={rel}"),
            "text/plain",
            Body::empty(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert!(!file.exists());
    }

    #[tokio::test]
    async fn get_fixture_type_not_found() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_notfound");
        std::fs::create_dir(&ft_dir).unwrap();
        let rel = ft_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/fixture-types/nonexistent?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn put_fixture_type_raw_dsl() {
        let (state, _dir) = test_state();
        let rel = "ft_put_raw";
        let app = router().with_state(state);

        let dsl = sample_fixture_type_dsl("MyFixture");
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/fixture-types/MyFixture?dir={}", rel))
                    .header("content-type", "text/plain")
                    .body(Body::from(dsl))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["status"], "saved");
        assert_eq!(parsed["name"], "MyFixture");

        // Verify file was created.
        let file_path = _dir.path().join(rel).join("myfixture.fixture");
        assert!(file_path.exists());
    }

    fn multipart_body(filename: &str, bytes: &[u8]) -> (String, Vec<u8>) {
        let boundary = "mtrack-test-boundary";
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        (format!("multipart/form-data; boundary={boundary}"), body)
    }

    fn synthetic_gdtf_bytes() -> Vec<u8> {
        crate::lighting::gdtf::build_zip(&[(
            "description.xml",
            crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.as_bytes(),
        )])
    }

    #[tokio::test]
    async fn import_gdtf_writes_and_reports_through_the_shared_importer() {
        let (state, dir) = test_state();
        let app = router().with_state(state);
        // A traversal-shaped filename must land as its final component only.
        let (content_type, body) = multipart_body("../synth.gdtf", &synthetic_gdtf_bytes());

        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/lighting/gdtf/import?name=Brick")
                    .header("content-type", content_type)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let parsed: serde_json::Value =
            serde_json::from_str(&response_body(response).await).unwrap();
        assert_eq!(parsed["type_name"], "Brick");
        assert!(dir.path().join("lighting/library/synth.gdtf").exists());
        assert!(
            !dir.path().parent().unwrap().join("synth.gdtf").exists(),
            "traversal filename must not escape the library"
        );
        assert!(dir
            .path()
            .join("lighting/fixture_types/brick.fixture")
            .exists());
        assert!(dir.path().join("lighting/.cache").is_dir());

        // A name another fixture has is a conflict naming its file, not a
        // server fault; so is a name whose file another type has.
        let other = crate::lighting::gdtf::build_zip(&[(
            "description.xml",
            crate::lighting::gdtf::SYNTHETIC_DESCRIPTION
                .replace("Synth Brick", "Other Brick")
                .as_bytes(),
        )]);
        let (status, parsed) = import_upload(&app, "other.gdtf", &other, "?name=Brick").await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert_eq!(
            parsed["error"],
            "fixture type \"Brick\" is already declared in brick.fixture; choose another name"
        );
        let (status, parsed) = import_upload(&app, "other.gdtf", &other, "?name=BRICK%201").await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        let (status, parsed) = import_upload(&app, "other.gdtf", &other, "?name=brick_1").await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert_eq!(
            parsed["error"],
            "brick_1.fixture is taken by \"BRICK 1\"; choose another name"
        );
    }

    /// #673: the import's name check and its pinned record go to the
    /// fixture types directory this host's profile sets, as the CLI and MCP
    /// put them, not the default one the player does not read; and the
    /// other endpoints that default a directory read it there too.
    #[tokio::test]
    async fn an_import_goes_to_the_hosts_configured_fixture_types_dir() {
        let config = "songs: \".\"\nprofiles:\n  - hostname: not-this-host-x9\n    dmx:\n      \
                      universes: []\n      lighting:\n        directories:\n          \
                      fixture_types: other/types\n  - dmx:\n      universes: []\n      \
                      lighting:\n        directories:\n          fixture_types: rig/types\n";
        let (app, dir) = light_shows_at_the_root(config);
        std::fs::create_dir_all(dir.path().join("rig/types")).unwrap();
        let (status, parsed) =
            import_upload(&app, "synth.gdtf", &synthetic_gdtf_bytes(), "?name=Brick").await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert!(dir.path().join("rig/types/brick.fixture").exists());
        assert!(!dir
            .path()
            .join("lighting/fixture_types/brick.fixture")
            .exists());
        assert!(!dir.path().join("other/types/brick.fixture").exists());
        // The list with no `?dir=` is this host's directory's.
        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/fixture-types")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        assert!(body.contains("\"Brick\""), "{body}");
        // Taken there: another archive under the name is a conflict.
        let other = crate::lighting::gdtf::build_zip(&[(
            "description.xml",
            crate::lighting::gdtf::SYNTHETIC_DESCRIPTION
                .replace("Synth Brick", "Other Brick")
                .as_bytes(),
        )]);
        let (status, parsed) = import_upload(&app, "other.gdtf", &other, "?name=Brick").await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
    }

    /// Uploads an archive to the import endpoint; the status and the answer.
    async fn import_upload(
        app: &axum::Router,
        file: &str,
        bytes: &[u8],
        query: &str,
    ) -> (StatusCode, serde_json::Value) {
        let (content_type, body) = multipart_body(file, bytes);
        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri(format!("/lighting/gdtf/import{query}"))
                    .header("content-type", content_type)
                    .body(Body::from(body))
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
    async fn an_import_with_no_mode_is_one_step_and_says_what_the_user_has() {
        let (state, dir) = test_state();
        let app = router().with_state(state.clone());

        let (status, parsed) = import_upload(&app, "synth.gdtf", &synthetic_gdtf_bytes(), "").await;

        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["type_name"], "Synth Brick");
        assert_eq!(parsed["fixture"], "Synth Brick");
        assert_eq!(parsed["manufacturer"], "mtrack synthetic");
        assert_eq!(parsed["modes"], 2);
        assert_eq!(parsed["archive"], "lighting/library/synth.gdtf");
        assert_eq!(parsed["already_imported"], false);
        assert!(parsed["renamed_from"].is_null());
        assert_eq!(parsed["refused_modes"], serde_json::json!([]));
        assert!(parsed.get("warnings").is_none(), "{parsed}");
        assert!(parsed.get("replaced_archive").is_none(), "{parsed}");
        assert!(
            !dir.path().join("lighting/fixture_types").exists(),
            "an import is a copy; no record"
        );

        // The same archive again: nothing written, and it says so.
        let (status, parsed) = import_upload(&app, "copy.gdtf", &synthetic_gdtf_bytes(), "").await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["already_imported"], true);
        assert_eq!(parsed["type_name"], "Synth Brick");
        assert!(!dir.path().join("lighting/library/copy.gdtf").exists());

        // Another archive of the same fixture name gets its file stem.
        let other = crate::lighting::gdtf::build_zip(&[(
            "description.xml",
            crate::lighting::gdtf::SYNTHETIC_DESCRIPTION
                .replace("mtrack synthetic", "someone else")
                .as_bytes(),
        )]);
        let (status, parsed) = import_upload(&app, "brick-v2.gdtf", &other, "").await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["type_name"], "Synth Brick (brick-v2)");
        assert_eq!(parsed["renamed_from"], "Synth Brick");
        let pin = std::fs::read_to_string(
            dir.path()
                .join("lighting/fixture_types/synth_brick_brick-v2.fixture"),
        )
        .unwrap_or_else(|e| panic!("the newcomer's name is pinned: {e}"));
        assert!(
            pin.contains("from gdtf(\"lighting/library/brick-v2.gdtf\")"),
            "{pin}"
        );

        // A different archive of a name the library already has: 409, in
        // the user's terms, and nothing written.
        let third = crate::lighting::gdtf::build_zip(&[(
            "description.xml",
            crate::lighting::gdtf::SYNTHETIC_DESCRIPTION
                .replace("Synth Brick", "Third Brick")
                .as_bytes(),
        )]);
        let (status, parsed) = import_upload(&app, "synth.gdtf", &third, "").await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        let error = parsed["error"].as_str().unwrap();
        assert!(error.contains("Rename your file"), "{error}");
        assert!(!error.contains(".fixture"), "{error}");

        // The fixture page takes it with no record.
        let (status, parsed) = fixture_type_request(
            state,
            "GET",
            "/lighting/fixture-types/Synth%20Brick/gdtf".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert!(parsed.get("mode").is_none(), "{parsed}");
        assert!(parsed["rig"].is_string(), "{parsed}");
    }

    #[tokio::test]
    async fn deleting_a_gdtf_fixture_removes_its_archive_unless_another_uses_it() {
        let (state, dir) = test_state();
        let app = router().with_state(state.clone());
        let delete = |name: &str| {
            fixture_type_request(
                state.clone(),
                "DELETE",
                format!("/lighting/fixture-types/{}", name.replace(' ', "%20")),
                "text/plain",
                Body::empty(),
            )
        };

        // A fixture that is its archive alone: deleting it removes the archive.
        let (status, _) = import_upload(&app, "synth.gdtf", &synthetic_gdtf_bytes(), "").await;
        assert_eq!(status, StatusCode::OK);
        let archive = dir.path().join("lighting/library/synth.gdtf");
        let (status, parsed) = delete("Synth Brick").await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(
            parsed,
            serde_json::json!({
                "status": "deleted",
                "name": "Synth Brick",
                "archive_removed": "lighting/library/synth.gdtf",
                "archive_kept": false,
            })
        );
        assert!(!archive.exists());

        // Two records of one archive: the first delete keeps it.
        let (status, _) = import_upload(&app, "synth.gdtf", &synthetic_gdtf_bytes(), "").await;
        assert_eq!(status, StatusCode::OK);
        let types = dir.path().join("lighting/fixture_types");
        std::fs::create_dir_all(&types).unwrap();
        for (file, name) in [("a.fixture", "Brick A"), ("b.fixture", "Brick B")] {
            std::fs::write(
                types.join(file),
                format!("fixture_type \"{name}\"\n  from gdtf(\"lighting/library/synth.gdtf\")\n{{\n}}\n"),
            )
            .unwrap();
        }
        let (status, parsed) = delete("Brick A").await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert!(parsed["archive_removed"].is_null());
        assert_eq!(parsed["archive_kept"], true);
        assert!(archive.exists(), "Brick B still uses it");
        assert!(!types.join("a.fixture").exists());

        let (status, parsed) = delete("Brick B").await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["archive_removed"], "lighting/library/synth.gdtf");
        assert_eq!(parsed["archive_kept"], false);
        assert!(!archive.exists());
        assert!(
            dir.path().join("lighting/.cache").is_dir(),
            "the cache is left alone"
        );
    }

    #[tokio::test]
    async fn deleting_a_hand_written_type_touches_no_archive() {
        let (state, dir) = test_state();
        let types = dir.path().join("lighting/fixture_types");
        std::fs::create_dir_all(&types).unwrap();
        std::fs::write(types.join("par.light"), sample_fixture_type_dsl("Par")).unwrap();
        let (status, parsed) = fixture_type_request(
            state,
            "DELETE",
            "/lighting/fixture-types/Par".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert!(parsed["archive_removed"].is_null());
        assert_eq!(parsed["archive_kept"], false);
    }

    #[tokio::test]
    async fn put_fixture_type_json() {
        let (state, _dir) = test_state();
        let rel = "ft_put_json";
        let app = router().with_state(state);

        let json_body = serde_json::json!({
            "channels": {
                "red": 1,
                "green": 2,
                "blue": 3
            }
        });
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/fixture-types/JSONFixture?dir={}", rel))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&json_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["status"], "saved");

        // Verify the file was created and contains valid DSL.
        let file_path = _dir.path().join(rel).join("jsonfixture.fixture");
        assert!(file_path.exists());
        let content = std::fs::read_to_string(&file_path).unwrap();
        let types = lighting::parser::parse_fixture_types(&content).unwrap();
        assert!(types.contains_key("JSONFixture"));
    }

    #[tokio::test]
    async fn put_fixture_type_invalid_name() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        // Empty name won't match the route, so test path traversal.
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/lighting/fixture-types/..%2Fevil?dir=ft_test")
                    .header("content-type", "text/plain")
                    .body(Body::from("content"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn put_fixture_type_invalid_dsl() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/lighting/fixture-types/BadDSL?dir=ft_bad")
                    .header("content-type", "text/plain")
                    .body(Body::from("invalid {{{ content"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn put_fixture_type_invalid_json() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/lighting/fixture-types/BadJSON?dir=ft_badjson")
                    .header("content-type", "application/json")
                    .body(Body::from("not valid json"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn delete_fixture_type_success() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_del");
        std::fs::create_dir(&ft_dir).unwrap();
        let file_path = ft_dir.join("todelete.light");
        std::fs::write(&file_path, sample_fixture_type_dsl("ToDelete")).unwrap();
        let rel = ft_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri(format!("/lighting/fixture-types/ToDelete?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["status"], "deleted");
        assert!(!file_path.exists());
    }

    #[tokio::test]
    async fn delete_fixture_type_removes_a_fixture_file() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_del_fixture");
        std::fs::create_dir(&ft_dir).unwrap();
        let file_path = ft_dir.join("brick.fixture");
        std::fs::write(&file_path, sample_referential_fixture_type_dsl("Brick")).unwrap();
        let rel = ft_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri(format!("/lighting/fixture-types/Brick?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(!file_path.exists());
    }

    #[tokio::test]
    async fn delete_fixture_type_not_found() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_del_nf");
        std::fs::create_dir(&ft_dir).unwrap();
        let rel = ft_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri(format!("/lighting/fixture-types/nonexistent?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // Venues endpoint tests
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_venues_empty() {
        let (state, _dir) = test_state();
        let v_dir = _dir.path().join("v_empty");
        std::fs::create_dir(&v_dir).unwrap();
        let rel = v_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/venues?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["venues"].is_object());
        assert_eq!(parsed["venues"].as_object().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn get_venues_nonexistent_dir() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/venues?dir=nonexistent_venues")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["venues"].as_object().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn get_venues_with_files() {
        let (state, _dir) = test_state();
        let v_dir = _dir.path().join("v_test");
        std::fs::create_dir(&v_dir).unwrap();
        std::fs::write(v_dir.join("club.light"), sample_venue_dsl("Club")).unwrap();
        let rel = v_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/venues?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["venues"]["Club"].is_object());
    }

    #[tokio::test]
    async fn get_venue_success() {
        let (state, _dir) = test_state();
        let v_dir = _dir.path().join("v_get");
        std::fs::create_dir(&v_dir).unwrap();
        std::fs::write(v_dir.join("club.light"), sample_venue_dsl("Club")).unwrap();
        let rel = v_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/venues/Club?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["venue"].is_object());
        assert!(parsed["dsl"].is_string());
    }

    #[tokio::test]
    async fn get_venue_not_found() {
        let (state, _dir) = test_state();
        let v_dir = _dir.path().join("v_notfound");
        std::fs::create_dir(&v_dir).unwrap();
        let rel = v_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/venues/nonexistent?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn put_venue_raw_dsl() {
        let (state, _dir) = test_state();
        let rel = "v_put_raw";
        let app = router().with_state(state);

        let dsl = sample_venue_dsl("MyVenue");
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/venues/MyVenue?dir={}", rel))
                    .header("content-type", "text/plain")
                    .body(Body::from(dsl))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["status"], "saved");
        assert_eq!(parsed["name"], "MyVenue");

        // Verify file was created.
        let file_path = _dir.path().join(rel).join("myvenue.venue");
        assert!(file_path.exists());
    }

    #[tokio::test]
    async fn put_venue_json() {
        let (state, _dir) = test_state();
        let rel = "v_put_json";
        let app = router().with_state(state);

        let json_body = serde_json::json!({
            "fixtures": [
                {
                    "name": "Spot1",
                    "fixture_type": "GenericPar",
                    "universe": 1,
                    "start_channel": 1
                },
                {
                    "name": "Spot2",
                    "fixture_type": "GenericPar",
                    "universe": 1,
                    "start_channel": 5
                }
            ]
        });
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/venues/JSONVenue?dir={}", rel))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&json_body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["status"], "saved");

        // Verify the file was created and contains valid DSL.
        let file_path = _dir.path().join(rel).join("jsonvenue.venue");
        assert!(file_path.exists());
        let content = std::fs::read_to_string(&file_path).unwrap();
        let venues = lighting::parser::parse_venues(&content).unwrap();
        assert!(venues.contains_key("JSONVenue"));
    }

    #[tokio::test]
    async fn a_new_rich_fixture_type_is_a_fixture_file() {
        let (state, _dir) = test_state();
        let rel = "ft_rich";
        let app = router().with_state(state);
        let dsl =
            "fixture_type \"Mover\" {\n  channel \"pan\" @ 1 fine 2 range -270deg..270deg\n}\n";
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/fixture-types/Mover?dir={}", rel))
                    .header("content-type", "text/plain")
                    .body(Body::from(dsl))
                    .unwrap(),
            )
            .await
            .unwrap();
        // A new type is always born a `.fixture`, rich or not.
        assert_eq!(response.status(), StatusCode::OK);
        assert!(_dir.path().join(rel).join("mover.fixture").exists());
        assert!(!_dir.path().join(rel).join("mover.light").exists());
    }

    #[tokio::test]
    async fn put_fixture_type_accepts_rich_channel_syntax_as_fixture() {
        let (state, _dir) = test_state();
        let rel = "ft_rich_ok";
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/lighting/fixture-types/Mover?dir={}&ext=fixture",
                        rel
                    ))
                    .header("content-type", "text/plain")
                    .body(Body::from(sample_rich_fixture_type_dsl("Mover")))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(_dir.path().join(rel).join("mover.fixture").exists());
        assert!(!_dir.path().join(rel).join("mover.light").exists());
    }

    #[tokio::test]
    async fn put_fixture_type_json_refuses_a_rich_type() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_form_refusal");
        std::fs::create_dir(&ft_dir).unwrap();
        let file_path = ft_dir.join("mover.fixture");
        std::fs::write(&file_path, sample_rich_fixture_type_dsl("Mover")).unwrap();
        let rel = ft_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/fixture-types/Mover?dir={}", rel))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({"channels": {"dimmer": 1}}))
                            .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        assert!(body.contains("as text"), "{body}");
        // The rich definition is untouched.
        assert!(std::fs::read_to_string(&file_path)
            .unwrap()
            .contains("fine 2"));
    }

    #[tokio::test]
    async fn put_fixture_type_refuses_a_renamed_declaration() {
        // The file is keyed on the URL name, so a body that renames the
        // declaration would write `mover.light` holding "Rover" — a file no
        // later GET, PUT or DELETE could reach.
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_rename");
        std::fs::create_dir(&ft_dir).unwrap();
        let file_path = ft_dir.join("mover.light");
        std::fs::write(&file_path, sample_fixture_type_dsl("Mover")).unwrap();
        let rel = ft_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/fixture-types/Mover?dir={}", rel))
                    .header("content-type", "text/plain")
                    .body(Body::from(sample_fixture_type_dsl("Rover")))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        assert!(body.contains("declares no fixture type"), "{body}");
        assert!(body.contains("Rover"), "{body}");
        // The original is untouched, and no orphan was written.
        assert!(std::fs::read_to_string(&file_path)
            .unwrap()
            .contains("Mover"));
        assert!(!ft_dir.join("rover.light").exists());
    }

    #[tokio::test]
    async fn put_venue_refuses_a_renamed_declaration() {
        let (state, _dir) = test_state();
        let rel = "venue_rename";
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/venues/MyVenue?dir={}", rel))
                    .header("content-type", "text/plain")
                    .body(Body::from(sample_venue_dsl("OtherVenue")))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        assert!(body.contains("declares no venue"), "{body}");
        assert!(!_dir.path().join(rel).join("myvenue.light").exists());
    }

    #[tokio::test]
    async fn put_fixture_type_rejects_an_unknown_extension() {
        let (state, _dir) = test_state();
        let rel = "ft_bad_ext";
        let app = router().with_state(state.clone());

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/lighting/fixture-types/Mover?dir={}&ext=bogus",
                        rel
                    ))
                    .header("content-type", "text/plain")
                    .body(Body::from(sample_fixture_type_dsl("Mover")))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        assert!(body.contains("Unknown fixture type extension"), "{body}");

        // The JSON body is held to the same query, so a typo is caught in
        // the form editor too rather than silently writing a `.light`.
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/lighting/fixture-types/Mover?dir={}&ext=bogus",
                        rel
                    ))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({"channels": {"dimmer": 1}}))
                            .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        // Nothing was created — not even the directory.
        assert!(!_dir.path().join(rel).exists());
    }

    #[tokio::test]
    async fn put_fixture_type_converts_a_light_to_a_fixture() {
        // The only way out of v1: an explicit `ext` wins over the existing
        // file's extension, and the `.light` is retired behind it.
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_convert");
        std::fs::create_dir(&ft_dir).unwrap();
        std::fs::write(ft_dir.join("mover.light"), sample_fixture_type_dsl("Mover")).unwrap();
        let rel = ft_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/lighting/fixture-types/Mover?dir={}&ext=fixture",
                        rel
                    ))
                    .header("content-type", "text/plain")
                    .body(Body::from(sample_rich_fixture_type_dsl("Mover")))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(ft_dir.join("mover.fixture").exists());
        assert!(!ft_dir.join("mover.light").exists());
    }

    #[tokio::test]
    async fn put_fixture_type_keeps_the_existing_extension_without_ext() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_keep_ext");
        std::fs::create_dir(&ft_dir).unwrap();
        std::fs::write(
            ft_dir.join("mover.fixture"),
            sample_rich_fixture_type_dsl("Mover"),
        )
        .unwrap();
        let rel = ft_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/fixture-types/Mover?dir={}", rel))
                    .header("content-type", "text/plain")
                    .body(Body::from(sample_rich_fixture_type_dsl("Mover")))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(ft_dir.join("mover.fixture").exists());
        assert!(!ft_dir.join("mover.light").exists());
    }

    #[tokio::test]
    async fn put_fixture_type_refuses_a_missing_gdtf_archive() {
        // The lighting system resolves the archive against the project at
        // load; a save is the last moment the text is still in front of the
        // person who can fix the path.
        let (state, _dir) = test_state();
        let rel = "ft_missing_archive";
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/lighting/fixture-types/Brick?dir={}&ext=fixture",
                        rel
                    ))
                    .header("content-type", "text/plain")
                    .body(Body::from(sample_referential_fixture_type_dsl("Brick")))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        assert!(body.contains("GDTF archive"), "{body}");
        assert!(body.contains("library/synth.gdtf"), "{body}");
        assert!(!_dir.path().join(rel).exists());
    }

    #[tokio::test]
    async fn put_fixture_type_accepts_a_referential_type_whose_archive_is_there() {
        let (state, _dir) = test_state();
        let library = _dir.path().join("library");
        std::fs::create_dir(&library).unwrap();
        std::fs::write(library.join("synth.gdtf"), synthetic_gdtf_bytes()).unwrap();
        let rel = "ft_present_archive";
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/lighting/fixture-types/Brick?dir={}&ext=fixture",
                        rel
                    ))
                    .header("content-type", "text/plain")
                    .body(Body::from(sample_referential_fixture_type_dsl("Brick")))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(_dir.path().join(rel).join("brick.fixture").exists());
    }

    #[tokio::test]
    async fn get_fixture_type_gdtf_answers_modes_and_a_rig_for_an_imported_file() {
        let (state, _dir) = test_state();
        let library = _dir.path().join("library");
        std::fs::create_dir(&library).unwrap();
        std::fs::write(library.join("synth.gdtf"), synthetic_gdtf_bytes()).unwrap();
        let rel = "ft_gdtf_details";
        // Named as `import-gdtf` names it, not by this API's own stem.
        imported_fixture_type_dir(
            _dir.path(),
            rel,
            "Astera-PixelBrick",
            sample_referential_fixture_type_dsl("Astera-PixelBrick"),
        );

        let (status, parsed) = fixture_type_request(
            state,
            "GET",
            format!("/lighting/fixture-types/Astera-PixelBrick/gdtf?dir={rel}"),
            "text/plain",
            Body::empty(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["archive"], "library/synth.gdtf");
        assert!(
            parsed.get("mode").is_none(),
            "a type names no mode: {parsed}"
        );
        let modes = parsed["inspection"]["modes"].as_array().expect("modes");
        assert!(!modes.is_empty(), "{parsed}");
        assert!(modes.iter().any(|m| m["name"] == "8: RGBS"), "{parsed}");
        let rig = parsed["rig"].as_str().expect("a rig path");
        assert!(
            _dir.path()
                .join("lighting/.cache")
                .join(lighting::distill::ASSETS_DIR)
                .join(rig)
                .is_file(),
            "the rig must be in the asset store: {rig}"
        );
    }

    /// A project with the synthetic archive in `library/`, a `Brick` type
    /// referring to it in `ft_rel`, and a venues directory `v_rel` patching
    /// three Bricks (one in a second venue) beside a file that will not parse.
    fn project_with_bricks(project: &std::path::Path, ft_rel: &str, v_rel: &str) {
        let library = project.join("library");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(library.join("synth.gdtf"), synthetic_gdtf_bytes()).unwrap();
        let ft = project.join(ft_rel);
        std::fs::create_dir_all(&ft).unwrap();
        std::fs::write(
            ft.join("brick.fixture"),
            sample_referential_fixture_type_dsl("Brick"),
        )
        .unwrap();
        let venues = project.join(v_rel);
        std::fs::create_dir_all(&venues).unwrap();
        std::fs::write(
            venues.join("stage.light"),
            "venue \"stage\" {\n  fixture \"Brick2\" Brick mode \"8: RGBS\" @ 1:5\n  \
             fixture \"Brick1\" Brick mode \"8: RGBS\" @ 1:1\n  fixture \"Par\" GenericPar @ 1:20\n}\n",
        )
        .unwrap();
        std::fs::write(
            venues.join("club.venue"),
            // One fixture in another mode, spelled loosely (the archive's
            // own spelling comes back), and one in a mode it does not have.
            "venue \"club\" {\n  fixture \"Solo\" Brick mode \"mover 16BIT\" @ 2:1\n  \
             fixture \"Odd\" Brick mode \"Nope\" @ 2:40\n}\n",
        )
        .unwrap();
        std::fs::write(venues.join("broken.light"), "venue {{{").unwrap();
    }

    /// A fixture types directory with types at the top and in
    /// subdirectories: a native type in each, a GDTF record in `rig/`, a
    /// second declaration of `Top` further down, and an unrecorded archive
    /// in the library.
    fn project_with_nested_types(project: &std::path::Path, ft_rel: &str) {
        let library = project.join("library");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(library.join("synth.gdtf"), synthetic_gdtf_bytes()).unwrap();
        let shelf = project.join("lighting/library");
        std::fs::create_dir_all(&shelf).unwrap();
        std::fs::write(shelf.join("loose.gdtf"), synthetic_gdtf_bytes()).unwrap();
        let ft = project.join(ft_rel);
        std::fs::create_dir_all(ft.join("rig/old")).unwrap();
        std::fs::write(ft.join("top.light"), sample_fixture_type_dsl("Top")).unwrap();
        std::fs::write(ft.join("rig/wash.light"), sample_fixture_type_dsl("Wash")).unwrap();
        std::fs::write(
            ft.join("rig/brick.fixture"),
            sample_referential_fixture_type_dsl("Brick"),
        )
        .unwrap();
        std::fs::write(ft.join("rig/old/top.light"), sample_fixture_type_dsl("Top")).unwrap();
    }

    #[tokio::test]
    async fn a_fixture_type_in_a_subdirectory_lists_opens_saves_in_place_and_deletes() {
        let (state, dir) = test_state();
        let ft = dir.path().join("ft_nested");
        project_with_nested_types(dir.path(), "ft_nested");

        let (status, list) = fixture_type_request(
            state.clone(),
            "GET",
            "/lighting/fixture-types?dir=ft_nested".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{list}");
        let types = &list["fixture_types"];
        assert_eq!(types["Wash"]["file"], "rig/wash.light", "{list}");
        assert_eq!(types["Brick"]["file"], "rig/brick.fixture", "{list}");
        // The second `Top` is reported the way any duplicate is, against
        // both files; the first in path order keeps the name.
        assert_eq!(types["Top"]["file"], "rig/old/top.light", "{list}");
        let errors = list["errors"].as_array().unwrap();
        assert!(
            errors.iter().any(|e| e["file"] == "top.light"
                && e["error"]
                    .as_str()
                    .unwrap()
                    .contains("\"Top\" is defined in both rig/old/top.light and top.light")),
            "{list}"
        );

        for uri in [
            "/lighting/fixture-types/Wash?dir=ft_nested",
            "/lighting/fixture-types/Brick?dir=ft_nested",
            "/lighting/fixture-types/Brick/gdtf?dir=ft_nested",
        ] {
            let (status, parsed) = fixture_type_request(
                state.clone(),
                "GET",
                uri.to_string(),
                "text/plain",
                Body::empty(),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{uri}: {parsed}");
        }

        let form = serde_json::json!({"channels": {"dimmer": 1}});
        let (status, parsed) = fixture_type_request(
            state.clone(),
            "PUT",
            "/lighting/fixture-types/Wash?dir=ft_nested".to_string(),
            "application/json",
            Body::from(serde_json::to_vec(&form).unwrap()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        let saved = std::fs::read_to_string(ft.join("rig/wash.light")).unwrap();
        assert!(saved.contains("dimmer"), "{saved}");
        assert!(
            !ft.join("wash.light").exists(),
            "saved beside, not in place"
        );

        let (status, parsed) = fixture_type_request(
            state.clone(),
            "DELETE",
            "/lighting/fixture-types/Wash?dir=ft_nested".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert!(!ft.join("rig/wash.light").exists());
        assert!(ft.join("rig/brick.fixture").exists());
    }

    #[tokio::test]
    async fn the_engine_and_the_fixture_types_list_read_the_same_tree() {
        let (state, dir) = test_state();
        project_with_nested_types(dir.path(), "ft_agree");
        let venues = dir.path().join("v_agree");
        std::fs::create_dir_all(venues.join("tour/old")).unwrap();
        std::fs::write(venues.join("house.light"), sample_venue_dsl("house")).unwrap();
        std::fs::write(venues.join("tour/club.light"), sample_venue_dsl("club")).unwrap();
        std::fs::write(
            venues.join("tour/old/house.light"),
            sample_venue_dsl("house"),
        )
        .unwrap();

        let mut system = lighting::system::LightingSystem::new();
        system
            .load(
                &crate::config::Lighting::new(
                    None,
                    None,
                    Some(crate::config::lighting::Directories::new(
                        Some("ft_agree".into()),
                        Some("v_agree".into()),
                    )),
                ),
                dir.path(),
            )
            .unwrap();
        let mut engine: Vec<String> = system
            .fixture_types_iter()
            .chain(system.gdtf_types_iter())
            .map(|(name, _)| name.clone())
            .collect();
        engine.sort();
        let mut engine_venues: Vec<String> =
            system.venues_iter().map(|(name, _)| name.clone()).collect();
        engine_venues.sort();

        let (status, list) = fixture_type_request(
            state.clone(),
            "GET",
            "/lighting/fixture-types?dir=ft_agree".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{list}");
        let mut web: Vec<String> = list["fixture_types"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        web.sort();
        assert_eq!(engine, web);
        assert_eq!(web, ["Brick", "Synth Brick", "Top", "Wash"]);

        let (status, list) = fixture_type_request(
            state,
            "GET",
            "/lighting/venues?dir=v_agree".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{list}");
        let mut web_venues: Vec<String> = list["venues"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        web_venues.sort();
        assert_eq!(engine_venues, web_venues);
        assert_eq!(web_venues, ["club", "house"]);
    }

    #[tokio::test]
    async fn a_venue_in_a_subdirectory_lists_opens_saves_in_place_and_deletes() {
        let (state, dir) = test_state();
        let venues = dir.path().join("v_nested");
        std::fs::create_dir_all(venues.join("tour/old")).unwrap();
        std::fs::write(venues.join("house.light"), sample_venue_dsl("house")).unwrap();
        std::fs::write(venues.join("tour/club.light"), sample_venue_dsl("club")).unwrap();
        std::fs::write(
            venues.join("tour/old/house.light"),
            sample_venue_dsl("house"),
        )
        .unwrap();

        let (status, list) = fixture_type_request(
            state.clone(),
            "GET",
            "/lighting/venues?dir=v_nested".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{list}");
        assert!(list["venues"]["club"].is_object(), "{list}");
        let errors = list["errors"].as_array().unwrap();
        assert!(
            errors.iter().any(|e| e["file"] == "tour/old/house.light"
                && e["error"]
                    .as_str()
                    .unwrap()
                    .contains("\"house\" is defined in both house.light and tour/old/house.light")),
            "{list}"
        );

        let (status, parsed) = fixture_type_request(
            state.clone(),
            "GET",
            "/lighting/venues/club?dir=v_nested".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");

        let edited = sample_venue_dsl("club").replace("@ 1:5", "@ 1:9");
        let (status, parsed) = fixture_type_request(
            state.clone(),
            "PUT",
            "/lighting/venues/club?dir=v_nested".to_string(),
            "text/plain",
            Body::from(edited.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(
            std::fs::read_to_string(venues.join("tour/club.light")).unwrap(),
            edited
        );
        assert!(
            !venues.join("club.light").exists(),
            "saved beside, not in place"
        );

        let (status, parsed) = fixture_type_request(
            state,
            "DELETE",
            "/lighting/venues/club?dir=v_nested".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert!(!venues.join("tour/club.light").exists());
    }

    #[tokio::test]
    async fn get_fixture_type_gdtf_names_the_venue_fixtures_and_the_facts() {
        let (state, _dir) = test_state();
        project_with_bricks(_dir.path(), "ft_gdtf_used", "v_gdtf_used");

        let (status, parsed) = fixture_type_request(
            state,
            "GET",
            "/lighting/fixture-types/Brick/gdtf?dir=ft_gdtf_used&venues_dir=v_gdtf_used"
                .to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(
            parsed["venues"],
            serde_json::json!([
                {"name": "club", "fixtures": [
                    {"name": "Solo", "mode": "Mover 16bit"},
                    {"name": "Odd", "mode": null},
                ]},
                {"name": "stage", "fixtures": [
                    {"name": "Brick1", "mode": "8: RGBS"},
                    {"name": "Brick2", "mode": "8: RGBS"},
                ]},
            ]),
            "by venue name, fixtures in patch order with the mode each is driven in, \
             other types left out"
        );
        assert_eq!(parsed["about"], "A brick that is not real.");
        // "8: RGBS" drives the Base tree, whose first beam is the head's lens.
        assert_eq!(parsed["beam"]["type"], "Spot");
        assert_eq!(parsed["beam"]["beam_angle"], 12.0);
        assert_eq!(parsed["beam"]["field_angle"], 20.0);
        assert_eq!(parsed["beam"]["luminous_flux"], 5000.0);
        assert_eq!(parsed["beam"]["color_temperature"], 6500.0);
        assert_eq!(parsed["beam"]["power"], 120.0);
    }

    #[tokio::test]
    async fn get_fixture_type_gdtf_with_no_venues_lists_none() {
        let (state, _dir) = test_state();
        project_with_bricks(_dir.path(), "ft_gdtf_nov", "v_gdtf_nov");

        let (status, parsed) = fixture_type_request(
            state,
            "GET",
            "/lighting/fixture-types/Brick/gdtf?dir=ft_gdtf_nov&venues_dir=no_such_dir".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["venues"], serde_json::json!([]));
    }

    #[tokio::test]
    async fn get_fixture_types_summarises_each_archive_and_survives_a_missing_one() {
        let (state, _dir) = test_state();
        project_with_bricks(_dir.path(), "ft_list_gdtf", "v_list_gdtf");
        std::fs::write(
            _dir.path().join("ft_list_gdtf").join("lost.fixture"),
            "fixture_type \"Lost\" from gdtf(\"library/gone.gdtf\") {\n}\n",
        )
        .unwrap();
        std::fs::write(
            _dir.path().join("ft_list_gdtf").join("par.light"),
            sample_fixture_type_dsl("Par"),
        )
        .unwrap();

        let (status, parsed) = fixture_type_request(
            state,
            "GET",
            "/lighting/fixture-types?dir=ft_list_gdtf&venues_dir=v_list_gdtf".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{parsed}");
        let types = &parsed["fixture_types"];
        let brick = &types["Brick"]["gdtf"];
        assert_eq!(brick["fixture"], "Synth Brick");
        assert_eq!(brick["manufacturer"], "mtrack synthetic");
        assert!(brick["modes"].as_u64().unwrap() > 0, "{brick}");
        assert!(brick.get("mode").is_none(), "{brick}");
        assert_eq!(
            brick["beam"],
            serde_json::json!({"type": "Spot", "angle": 12.0})
        );
        assert_eq!(brick["used_by"], 4);
        assert!(
            types["Brick"]["footprint"].is_null(),
            "a GDTF type's footprint is each fixture's mode's: {parsed}"
        );
        assert!(types["Lost"]["footprint"].is_null(), "unknown is null");
        assert!(
            types["Par"]["footprint"].as_u64().unwrap() > 0,
            "a native type's own footprint: {parsed}"
        );
        assert_eq!(
            brick["in_use"],
            serde_json::json!([
                {"mode": "8: RGBS", "count": 2},
                {"mode": "Mover 16bit", "count": 1},
                {"mode": "Nope", "count": 1},
            ]),
            "most-used first, the archive's spelling, an unknown mode as written"
        );
        assert!(brick["thumbnail"].is_null(), "no rig in the store yet");
        assert!(
            types["Lost"]["gdtf"].is_null(),
            "a missing archive is that card's null: {parsed}"
        );
        assert!(
            types["Par"].get("gdtf").is_none(),
            "a native type carries no summary"
        );
        assert!(
            !_dir.path().join("lighting/.cache").exists(),
            "a listing writes nothing"
        );
    }

    #[tokio::test]
    async fn an_unused_gdtf_type_is_listed_and_drawn_in_its_first_mode() {
        let (state, _dir) = test_state();
        project_with_bricks(_dir.path(), "ft_unused", "v_unused");
        std::fs::write(
            _dir.path().join("ft_unused").join("bare.fixture"),
            "fixture_type \"Bare\" from gdtf(\"library/synth.gdtf\") {\n}\n",
        )
        .unwrap();

        let (status, parsed) = fixture_type_request(
            state.clone(),
            "GET",
            "/lighting/fixture-types?dir=ft_unused&venues_dir=v_unused".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        let bare = &parsed["fixture_types"]["Bare"];
        assert_eq!(bare["referential"], true, "{parsed}");
        assert!(bare["footprint"].is_null(), "{bare}");
        assert!(bare.get("default_mode").is_none(), "{bare}");
        assert_eq!(bare["gdtf"]["in_use"], serde_json::json!([]));

        let (status, parsed) = fixture_type_request(
            state,
            "GET",
            "/lighting/fixture-types/Bare/gdtf?dir=ft_unused&venues_dir=v_unused".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["venues"], serde_json::json!([]));
        assert!(
            parsed["rig"].is_string(),
            "drawn in the first mode: {parsed}"
        );
    }

    /// A project whose config names `rig` as the current venue, with the
    /// synthetic archive, a `Brick` type and a venues directory.
    fn project_with_current_venue(project: &std::path::Path, config: &std::path::Path) {
        project_with_bricks(project, "lighting/fixture_types", "lighting/venues");
        std::fs::write(
            config,
            "songs: songs\ndmx:\n  universes:\n    - universe: 1\n      name: main\n  \
             lighting:\n    current_venue: rig\n    directories:\n      \
             fixture_types: lighting/fixture_types\n      venues: lighting/venues\n",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn a_save_that_leaves_the_current_venue_failing_says_so_and_still_saves() {
        let (state, _dir) = test_state();
        project_with_current_venue(_dir.path(), &state.config_path);
        let body = serde_json::json!({
            "fixtures": [
                {"name": "B1", "fixture_type": "Brick", "universe": 1, "start_channel": 1,
                 "mode": "8: RGBS"},
                {"name": "B2", "fixture_type": "Brick", "universe": 1, "start_channel": 20,
                 "mode": "Nope"},
            ]
        });
        let response = router()
            .with_state(state.clone())
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/lighting/venues/rig")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let parsed: serde_json::Value =
            serde_json::from_str(&response_body(response).await).unwrap();
        assert_eq!(parsed["venue_error"]["venue"], "rig", "{parsed}");
        assert_eq!(parsed["venue_error"]["fixture"], "B2", "{parsed}");
        assert!(
            parsed["venue_error"]["reason"]
                .as_str()
                .unwrap()
                .contains("Nope"),
            "{parsed}"
        );
        assert!(_dir.path().join("lighting/venues/rig.venue").exists());

        // Fixed: the error goes.
        let body = serde_json::json!({
            "fixtures": [
                {"name": "B1", "fixture_type": "Brick", "universe": 1, "start_channel": 1,
                 "mode": "8: RGBS"},
                {"name": "B2", "fixture_type": "Brick", "universe": 1, "start_channel": 20,
                 "mode": "Mover 16bit"},
            ]
        });
        let response = router()
            .with_state(state)
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/lighting/venues/rig")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&response_body(response).await).unwrap();
        assert!(parsed["venue_error"].is_null(), "{parsed}");
    }

    #[tokio::test]
    async fn a_save_of_a_venue_that_is_not_current_reports_nothing() {
        let (state, _dir) = test_state();
        project_with_current_venue(_dir.path(), &state.config_path);
        let body = serde_json::json!({
            "fixtures": [{"name": "B1", "fixture_type": "Brick", "universe": 1,
                          "start_channel": 1, "mode": "Nope"}]
        });
        let response = router()
            .with_state(state)
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/lighting/venues/other")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let parsed: serde_json::Value =
            serde_json::from_str(&response_body(response).await).unwrap();
        assert!(parsed["venue_error"].is_null(), "{parsed}");
    }

    #[tokio::test]
    async fn the_patch_of_a_venue_comes_from_its_files_with_each_fixture_s_mode() {
        let (state, _dir) = test_state();
        project_with_bricks(_dir.path(), "ft_patch", "v_patch");
        // A gang at 1:1, a partial overlap at 1:3, a fixture in a wider
        // mode, one whose mode does not load, and one past the universe end.
        std::fs::write(
            _dir.path().join("v_patch").join("p.venue"),
            "venue \"p\" {\n  fixture \"G1\" Brick mode \"8: RGBS\" @ 1:1\n  \
             fixture \"G2\" Brick mode \"8: RGBS\" @ 1:1\n  \
             fixture \"Part\" Brick mode \"8: RGBS\" @ 1:3\n  \
             fixture \"Wide\" Brick mode \"Mover 16bit\" @ 1:100\n  \
             fixture \"Odd\" Brick mode \"Nope\" @ 1:200\n  fixture \"Bare\" Brick @ 1:300\n  \
             fixture \"End\" Brick mode \"8: RGBS\" @ 1:511\n}\n",
        )
        .unwrap();

        let (status, parsed) = fixture_type_request(
            state.clone(),
            "GET",
            "/lighting/venues/p/patch?dir=v_patch&fixture_types_dir=ft_patch".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        let spans = parsed["spans"].as_array().unwrap();
        let span = |name: &str| spans.iter().find(|s| s["fixture"] == name).unwrap();
        let rgbs = span("G1")["footprint"].as_u64().unwrap();
        assert!(rgbs > 0, "{parsed}");
        assert_eq!(span("G1")["type"], "Brick");
        assert_eq!(span("G1")["mode"], "8: RGBS");
        assert_eq!(span("Wide")["mode"], "Mover 16bit");
        assert_ne!(span("Wide")["footprint"], span("G1")["footprint"]);
        assert!(
            span("Odd")["footprint"].is_null(),
            "never guessed: {parsed}"
        );
        assert!(
            span("Bare")["footprint"].is_null() && span("Bare")["mode"].is_null(),
            "a GDTF fixture with no mode has no footprint: {parsed}"
        );

        // The gang is quiet; the partial overlap names the gang once.
        let overlaps = parsed["overlaps"].as_array().unwrap();
        assert_eq!(overlaps.len(), 1, "{parsed}");
        assert_eq!(overlaps[0]["first_gang"], serde_json::json!(["G1", "G2"]));
        assert_eq!(overlaps[0]["second"], "Part");
        let overruns = parsed["overruns"].as_array().unwrap();
        assert_eq!(overruns.len(), 1, "{parsed}");
        assert_eq!(overruns[0]["fixture"], "End");

        let (status, _) = fixture_type_request(
            state,
            "GET",
            "/lighting/venues/nowhere/patch?dir=v_patch&fixture_types_dir=ft_patch".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// The engine and the web layer find a project's lighting files in the
    /// same directories for every shape of config. The web UI sends a
    /// profile's configured directory as `?dir=` and nothing otherwise; the
    /// engine resolves through `config::lighting`. A first-run project (no
    /// `directories`) used to have the UI save where the engine never read.
    #[tokio::test]
    async fn the_engine_and_the_web_layer_resolve_the_same_directories() {
        use crate::config::lighting::{Directories, Lighting};
        /// `None`: no lighting section; `Some(None)`: no `directories`;
        /// else (fixture types, venues) as configured.
        type Section = Option<Option<(Option<&'static str>, Option<&'static str>)>>;
        let cases: [(&str, Section); 5] = [
            ("no lighting section", None),
            ("no directories", Some(None)),
            ("types set", Some(Some((Some("my/types"), None)))),
            ("venues set", Some(Some((None, Some("my/venues"))))),
            ("both set", Some(Some((Some("t2"), Some("v2"))))),
        ];
        for (case, lighting) in cases {
            let (state, dir) = test_state();
            let root = dir.path();
            let section = lighting.map(|dirs| {
                Lighting::new(
                    Some("v".to_string()),
                    None,
                    dirs.map(|(t, v)| {
                        Directories::new(t.map(str::to_string), v.map(str::to_string))
                    }),
                )
            });
            // Where the shared resolution says the files are.
            let types = crate::config::lighting::fixture_types_dir(section.as_ref());
            let venues = crate::config::lighting::venues_dir(section.as_ref());
            std::fs::create_dir_all(root.join(types)).unwrap();
            std::fs::create_dir_all(root.join(venues)).unwrap();
            std::fs::write(
                root.join(types).join("par.light"),
                sample_fixture_type_dsl("Par"),
            )
            .unwrap();
            std::fs::write(
                root.join(venues).join("v.light"),
                "venue \"v\" {\n  fixture \"A\" Par @ 1:1\n}\n",
            )
            .unwrap();

            // The engine (a profile with a lighting section) loads them.
            if let Some(config) = &section {
                let mut system = lighting::system::LightingSystem::new();
                system.load(config, root).unwrap();
                assert_eq!(
                    system.get_current_venue_fixtures().map(|f| f.len()).ok(),
                    Some(1),
                    "{case}: the engine finds the venue and its type"
                );
            }

            // The web layer, given what the UI sends for this profile.
            let configured = section.as_ref().and_then(|l| l.directories());
            let mut query = Vec::new();
            if let Some(v) = configured.and_then(|d| d.venues()) {
                query.push(format!("dir={v}"));
            }
            if let Some(t) = configured.and_then(|d| d.fixture_types()) {
                query.push(format!("fixture_types_dir={t}"));
            }
            let (status, parsed) = fixture_type_request(
                state.clone(),
                "GET",
                format!("/lighting/venues/v/patch?{}", query.join("&")),
                "text/plain",
                Body::empty(),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{case}: {parsed}");
            assert!(
                parsed["spans"][0]["footprint"].as_u64().is_some(),
                "{case}: the web layer finds the venue and its type: {parsed}"
            );
            let types_query = configured
                .and_then(|d| d.fixture_types())
                .map(|t| format!("?dir={t}"))
                .unwrap_or_default();
            let (status, parsed) = fixture_type_request(
                state,
                "GET",
                format!("/lighting/fixture-types{types_query}"),
                "text/plain",
                Body::empty(),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{case}: {parsed}");
            assert!(
                parsed.to_string().contains("\"Par\""),
                "{case}: the web layer lists the type: {parsed}"
            );
        }
    }

    #[tokio::test]
    async fn the_scene_of_a_venue_comes_from_its_files_with_each_fixture_s_rig() {
        let (state, _dir) = test_state();
        project_with_bricks(_dir.path(), "ft_scene", "v_scene");
        std::fs::write(
            _dir.path().join("v_scene").join("s.venue"),
            "venue \"s\" {\n  fixture \"A\" Brick mode \"8: RGBS\" @ 1:1 position (-2, 2, 3) beam_angle 45\n  \
             fixture \"B\" Brick mode \"Mover 16bit\" @ 1:20\n  \
             fixture \"Odd\" Brick mode \"Nope\" @ 1:200\n}\n",
        )
        .unwrap();

        let (status, parsed) = fixture_type_request(
            state.clone(),
            "GET",
            "/lighting/venues/s/scene?dir=v_scene&fixture_types_dir=ft_scene".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["venue"]["name"], "s");
        assert!(parsed["venue"]["scenery"].is_null(), "{parsed}");
        let fixtures = &parsed["fixtures"];
        assert_eq!(fixtures.as_object().unwrap().len(), 3, "{parsed}");
        assert_eq!(fixtures["A"]["type"], "Brick");
        assert_eq!(fixtures["A"]["mode"], "8: RGBS");
        assert_eq!(
            fixtures["A"]["position"],
            serde_json::json!([-2.0, 2.0, 3.0])
        );
        assert!(fixtures["B"]["position"].is_null(), "{parsed}");
        assert_eq!(fixtures["A"]["beam_angle"], 45.0);
        assert!(fixtures["B"]["beam_angle"].is_null(), "{parsed}");
        // Each (archive, mode) has its own rig, written to the store.
        let rig_a = fixtures["A"]["rig"].as_str().expect("a rig for A");
        let rig_b = fixtures["B"]["rig"].as_str().expect("a rig for B");
        assert_ne!(rig_a, rig_b);
        assert!(_dir
            .path()
            .join("lighting/.cache/assets")
            .join(rig_a)
            .is_file());
        // A mode that does not load is drawn generically, not dropped.
        assert!(fixtures["Odd"]["rig"].is_null(), "{parsed}");

        let (status, _) = fixture_type_request(
            state,
            "GET",
            "/lighting/venues/nowhere/scene?dir=v_scene&fixture_types_dir=ft_scene".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_fixture_type_gdtf_refuses_a_native_type() {
        let (state, _dir) = test_state();
        let rel = "ft_gdtf_native";
        let dir = _dir.path().join(rel);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("par.light"), sample_fixture_type_dsl("Par")).unwrap();

        let (status, parsed) = fixture_type_request(
            state,
            "GET",
            format!("/lighting/fixture-types/Par/gdtf?dir={rel}"),
            "text/plain",
            Body::empty(),
        )
        .await;

        assert_eq!(status, StatusCode::NOT_FOUND, "{parsed}");
        assert_eq!(parsed["error"], "fixture type \"Par\" is not a GDTF type");
    }

    #[tokio::test]
    async fn get_fixture_type_gdtf_unknown_type_is_not_found() {
        let (state, _dir) = test_state();
        let rel = "ft_gdtf_unknown";
        std::fs::create_dir(_dir.path().join(rel)).unwrap();

        let (status, parsed) = fixture_type_request(
            state,
            "GET",
            format!("/lighting/fixture-types/Nothing/gdtf?dir={rel}"),
            "text/plain",
            Body::empty(),
        )
        .await;

        assert_eq!(status, StatusCode::NOT_FOUND, "{parsed}");
        assert_eq!(parsed["error"], "Fixture type not found: Nothing");
    }

    #[tokio::test]
    async fn get_fixture_type_gdtf_missing_archive_is_a_bad_request() {
        let (state, _dir) = test_state();
        let rel = "ft_gdtf_missing";
        let dir = _dir.path().join(rel);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(
            dir.join("brick.fixture"),
            sample_referential_fixture_type_dsl("Brick"),
        )
        .unwrap();

        let (status, parsed) = fixture_type_request(
            state,
            "GET",
            format!("/lighting/fixture-types/Brick/gdtf?dir={rel}"),
            "text/plain",
            Body::empty(),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{parsed}");
        assert!(
            parsed["error"]
                .as_str()
                .unwrap()
                .contains("library/synth.gdtf"),
            "{parsed}"
        );
    }

    #[tokio::test]
    async fn put_fixture_type_refuses_a_gdtf_archive_outside_the_project() {
        let (state, _dir) = test_state();
        let rel = "ft_escaping_archive";
        let app = router().with_state(state);
        let dsl = "fixture_type \"Brick\" from gdtf(\"../../etc/passwd\") {\n}\n";

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/lighting/fixture-types/Brick?dir={}&ext=fixture",
                        rel
                    ))
                    .header("content-type", "text/plain")
                    .body(Body::from(dsl))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(!_dir.path().join(rel).exists());
    }

    #[tokio::test]
    async fn get_fixture_types_reports_a_name_defined_twice() {
        // Last-wins would show one file and hide the other, while the
        // lighting system registers the name twice.
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_dupes");
        std::fs::create_dir(&ft_dir).unwrap();
        std::fs::write(ft_dir.join("a.light"), sample_fixture_type_dsl("Mover")).unwrap();
        std::fs::write(
            ft_dir.join("b.fixture"),
            sample_rich_fixture_type_dsl("Mover"),
        )
        .unwrap();
        std::fs::write(ft_dir.join("c.light"), sample_fixture_type_dsl("Par")).unwrap();
        let rel = ft_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/fixture-types?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let errors = parsed["errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1, "{parsed}");
        let message = errors[0]["error"].as_str().unwrap();
        assert!(message.contains("defined in both"), "{message}");
        assert!(message.contains("a.light"), "{message}");
        assert!(message.contains("b.fixture"), "{message}");
        // The rest of the directory still lists, including the duplicate
        // under one of its two files.
        assert!(parsed["fixture_types"]["Par"].is_object(), "{parsed}");
        assert!(parsed["fixture_types"]["Mover"].is_object(), "{parsed}");
    }

    #[tokio::test]
    async fn get_venues_reports_a_name_defined_twice() {
        // The same shape as the fixture types: a venue directory holds both
        // extensions, so two files can claim one name.
        let (state, _dir) = test_state();
        let venue_dir = _dir.path().join("venue_dupes");
        std::fs::create_dir(&venue_dir).unwrap();
        std::fs::write(venue_dir.join("a.light"), sample_venue_dsl("MainHall")).unwrap();
        std::fs::write(venue_dir.join("b.venue"), sample_venue_dsl("MainHall")).unwrap();
        std::fs::write(venue_dir.join("c.light"), sample_venue_dsl("Club")).unwrap();
        let rel = venue_dir
            .strip_prefix(_dir.path())
            .unwrap()
            .to_str()
            .unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/venues?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let errors = parsed["errors"].as_array().unwrap();
        assert_eq!(errors.len(), 1, "{parsed}");
        let message = errors[0]["error"].as_str().unwrap();
        assert!(message.contains("venue \"MainHall\""), "{message}");
        assert!(message.contains("defined in both"), "{message}");
        assert!(message.contains("a.light"), "{message}");
        assert!(message.contains("b.venue"), "{message}");
        // One bad pair does not empty the list.
        assert!(parsed["venues"]["Club"].is_object(), "{parsed}");
        assert!(parsed["venues"]["MainHall"].is_object(), "{parsed}");
    }

    #[tokio::test]
    async fn a_type_in_two_files_is_read_only() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_twin");
        std::fs::create_dir(&ft_dir).unwrap();
        // A GDTF import can land a `.fixture` beside a hand-written `.light`
        // of the same name: the name is declared twice, so neither file is
        // its alone, and the save is refused rather than guessing which.
        std::fs::write(ft_dir.join("mover.light"), sample_fixture_type_dsl("Mover")).unwrap();
        std::fs::write(
            ft_dir.join("mover.fixture"),
            sample_rich_fixture_type_dsl("Mover"),
        )
        .unwrap();
        let rel = ft_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/fixture-types/Mover?dir={}", rel))
                    .header("content-type", "text/plain")
                    .body(Body::from(sample_rich_fixture_type_dsl("Mover")))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
        assert_eq!(body["read_only"], true);
        assert_eq!(
            body["files"],
            serde_json::json!(["mover.fixture", "mover.light"])
        );
        assert!(ft_dir.join("mover.fixture").exists());
        assert!(ft_dir.join("mover.light").exists());
    }

    #[tokio::test]
    async fn put_venue_json_with_geometry_lands_in_a_venue_file() {
        let (state, _dir) = test_state();
        let rel = "v_put_geometry";
        let app = router().with_state(state);

        // First a plain venue, written by hand as a .light.
        std::fs::create_dir_all(_dir.path().join(rel)).unwrap();
        std::fs::write(
            _dir.path().join(rel).join("geo.light"),
            "venue \"Geo\" {\n  fixture \"Spot1\" GenericPar @ 1:1\n}\n",
        )
        .unwrap();

        // Then the same venue gains a position and a focus point: it moves
        // to .venue and the .light twin is retired.
        let geometry = serde_json::json!({
            "fixtures": [{
                "name": "Spot1", "fixture_type": "GenericPar", "universe": 1, "start_channel": 1,
                "tags": ["spot"], "position": [-2.0, 3.5, 4.2], "rotation": [0, 0, 180]
            }],
            "focus_points": {"drummer": [0.0, 2.8, 1.4]},
            "source": {"mvr": "lighting/library/geo.mvr", "origin": [0, -3.5, 0]}
        });
        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/venues/Geo?dir={}", rel))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&geometry).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let venue_path = _dir.path().join(rel).join("geo.venue");
        assert!(venue_path.exists());
        assert!(!_dir.path().join(rel).join("geo.light").exists());
        let content = std::fs::read_to_string(&venue_path).unwrap();
        let venue = &lighting::parser::parse_venues(&content).unwrap()["Geo"];
        assert_eq!(venue.fixtures()["Spot1"].position(), Some([-2.0, 3.5, 4.2]));
        assert_eq!(venue.focus_points()["drummer"], [0.0, 2.8, 1.4]);
        assert_eq!(venue.source().unwrap().mvr, "lighting/library/geo.mvr");

        // GET resolves the .venue and its JSON carries the geometry back.
        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/venues/Geo?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
        assert_eq!(
            body["venue"]["fixtures"]["Spot1"]["position"],
            serde_json::json!([-2.0, 3.5, 4.2])
        );
        assert_eq!(
            body["venue"]["focus_points"]["drummer"],
            serde_json::json!([0.0, 2.8, 1.4])
        );

        // The listing sees it, and DELETE finds it.
        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/venues?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
        assert!(body["venues"]["Geo"].is_object(), "{body}");
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri(format!("/lighting/venues/Geo?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(!venue_path.exists());
    }

    /// Sends a venue PUT and reads the file it wrote back.
    async fn put_venue_body(
        state: WebUiState,
        uri: String,
        content_type: &str,
        body: Vec<u8>,
    ) -> StatusCode {
        router()
            .with_state(state)
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(uri)
                    .header("content-type", content_type)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn a_json_save_of_a_new_venue_writes_each_fixture_s_mode() {
        let (state, _dir) = test_state();
        let rel = "v_put_mode_new";
        let body = serde_json::json!({
            "fixtures": [
                {"name": "B1", "fixture_type": "Brick", "universe": 1, "start_channel": 1,
                 "mode": "Mover 16bit"},
                {"name": "B2", "fixture_type": "Brick", "universe": 1, "start_channel": 20,
                 "mode": null},
            ]
        });
        let status = put_venue_body(
            state,
            format!("/lighting/venues/Rig?dir={rel}"),
            "application/json",
            serde_json::to_vec(&body).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // A mode is `.venue` syntax: the file is born a .venue.
        let path = _dir.path().join(rel).join("rig.venue");
        assert!(path.exists(), "a moded venue is a .venue");
        assert!(!_dir.path().join(rel).join("rig.light").exists());
        let venue = &lighting::parser::parse_venues(&std::fs::read_to_string(&path).unwrap())
            .unwrap()["Rig"];
        assert_eq!(venue.fixtures()["B1"].mode(), Some("Mover 16bit"));
        assert_eq!(venue.fixtures()["B2"].mode(), None);
    }

    #[tokio::test]
    async fn a_light_venue_that_gains_a_mode_moves_to_venue() {
        let (state, _dir) = test_state();
        let rel = "v_put_mode_ext";
        // A `.light` venue written by hand (a new one is a `.venue`).
        std::fs::create_dir_all(_dir.path().join(rel)).unwrap();
        std::fs::write(
            _dir.path().join(rel).join("rig.light"),
            "venue \"Rig\" {\n  fixture \"B1\" Brick @ 1:1\n}\n",
        )
        .unwrap();
        let uri = format!("/lighting/venues/Rig?dir={rel}");

        let moded = serde_json::json!({
            "fixtures": [{"name": "B1", "fixture_type": "Brick", "universe": 1,
                          "start_channel": 1, "mode": "8: RGBS"}]
        });
        let status = put_venue_body(
            state,
            uri,
            "application/json",
            serde_json::to_vec(&moded).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let path = _dir.path().join(rel).join("rig.venue");
        assert!(path.exists());
        assert!(!_dir.path().join(rel).join("rig.light").exists());
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("fixture \"B1\" Brick mode \"8: RGBS\" @ 1:1"));
    }

    #[tokio::test]
    async fn a_json_save_of_an_existing_venue_changes_or_clears_only_the_mode_it_is_given() {
        let (state, _dir) = test_state();
        let rel = "v_put_mode_patch";
        let dir = _dir.path().join(rel);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rig.venue");
        std::fs::write(
            &path,
            "# mine\nvenue \"Rig\" {\n  fixture \"B1\" Brick mode \"Mover 16bit\" @ 1:1  # keep\n  \
             fixture \"B2\" Brick @ 1:20\n}\n",
        )
        .unwrap();
        // The inspector's save: B2 gains a mode, B1 keeps its own.
        let body = serde_json::json!({
            "fixtures": [
                {"name": "B1", "fixture_type": "Brick", "universe": 1, "start_channel": 1,
                 "mode": "Mover 16bit"},
                {"name": "B2", "fixture_type": "Brick", "universe": 1, "start_channel": 20,
                 "mode": "8: RGBS"},
            ]
        });
        let status = put_venue_body(
            state.clone(),
            format!("/lighting/venues/Rig?dir={rel}"),
            "application/json",
            serde_json::to_vec(&body).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(
            after.contains("  fixture \"B1\" Brick mode \"Mover 16bit\" @ 1:1  # keep\n"),
            "{after}"
        );
        assert!(
            after.contains("fixture \"B2\" Brick mode \"8: RGBS\" @ 1:20"),
            "{after}"
        );

        // Back to the type default: the line loses its mode.
        let body = serde_json::json!({
            "fixtures": [
                {"name": "B1", "fixture_type": "Brick", "universe": 1, "start_channel": 1,
                 "mode": "Mover 16bit"},
                {"name": "B2", "fixture_type": "Brick", "universe": 1, "start_channel": 20},
            ]
        });
        let status = put_venue_body(
            state,
            format!("/lighting/venues/Rig?dir={rel}"),
            "application/json",
            serde_json::to_vec(&body).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.contains("fixture \"B2\" Brick @ 1:20"), "{after}");
        assert!(after.contains("mode \"Mover 16bit\""), "{after}");
    }

    #[tokio::test]
    async fn a_dsl_venue_save_keeps_its_modes() {
        let (state, _dir) = test_state();
        let rel = "v_put_mode_dsl";
        let dsl = "venue \"Rig\" {\n  fixture \"B1\" Brick mode \"Mover 16bit\" @ 1:1\n}\n";
        let status = put_venue_body(
            state,
            format!("/lighting/venues/Rig?dir={rel}"),
            "text/plain",
            dsl.as_bytes().to_vec(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let path = _dir.path().join(rel).join("rig.venue");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), dsl);
    }

    #[tokio::test]
    async fn a_json_save_refuses_a_mode_that_is_not_a_string() {
        let (state, _dir) = test_state();
        let body = serde_json::json!({
            "fixtures": [{"name": "B1", "fixture_type": "Brick", "universe": 1,
                          "start_channel": 1, "mode": 8}]
        });
        let status = put_venue_body(
            state,
            "/lighting/venues/Rig?dir=v_put_mode_bad".to_string(),
            "application/json",
            serde_json::to_vec(&body).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn put_venue_invalid_name() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/lighting/venues/..%2Fevil?dir=v_test")
                    .header("content-type", "text/plain")
                    .body(Body::from("content"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn put_venue_invalid_dsl() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/lighting/venues/BadDSL?dir=v_bad")
                    .header("content-type", "text/plain")
                    .body(Body::from("invalid {{{ content"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn put_venue_invalid_json() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/lighting/venues/BadJSON?dir=v_badjson")
                    .header("content-type", "application/json")
                    .body(Body::from("not valid json"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    /// A JSON venue save that `venue_from_json` must refuse, and the error.
    async fn refused_venue_save(fixtures: serde_json::Value, rel: &str) -> String {
        let (state, _dir) = test_state();
        let response = router()
            .with_state(state)
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/venues/Rig?dir={rel}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_vec(&serde_json::json!({ "fixtures": fixtures })).unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert!(!_dir.path().join(rel).exists(), "nothing is written");
        let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
        body["error"].as_str().unwrap().to_string()
    }

    #[tokio::test]
    async fn a_venue_save_with_two_fixtures_of_one_name_is_refused_by_name() {
        let error = refused_venue_save(
            serde_json::json!([
                {"name": "Spot", "fixture_type": "Par", "universe": 1, "start_channel": 1},
                {"name": "Spot", "fixture_type": "Par", "universe": 1, "start_channel": 5},
            ]),
            "v_dup_names",
        )
        .await;
        assert!(error.contains("'Spot'"), "{error}");
        assert!(error.contains("more than once"), "{error}");
    }

    #[tokio::test]
    async fn a_venue_save_with_a_nameless_or_typeless_fixture_is_refused() {
        let error = refused_venue_save(
            serde_json::json!([
                {"name": "  ", "fixture_type": "Par", "universe": 1, "start_channel": 1},
            ]),
            "v_no_name",
        )
        .await;
        assert!(error.contains("no name"), "{error}");
        let error = refused_venue_save(
            serde_json::json!([
                {"name": "Spot", "fixture_type": "", "universe": 1, "start_channel": 1},
            ]),
            "v_no_type",
        )
        .await;
        assert!(error.contains("no fixture type"), "{error}");
        let error = refused_venue_save(
            serde_json::json!([
                {"name": "Spot", "fixture_type": "Par", "universe": 0, "start_channel": 1},
            ]),
            "v_zero_universe",
        )
        .await;
        assert!(error.contains("universe"), "{error}");
        let error = refused_venue_save(
            serde_json::json!([
                {"name": "Spot", "fixture_type": "Par", "universe": 1, "start_channel": 0},
            ]),
            "v_zero_channel",
        )
        .await;
        assert!(error.contains("start channel"), "{error}");
    }

    #[tokio::test]
    async fn delete_venue_success() {
        let (state, _dir) = test_state();
        let v_dir = _dir.path().join("v_del");
        std::fs::create_dir(&v_dir).unwrap();
        let file_path = v_dir.join("todelete.light");
        std::fs::write(&file_path, sample_venue_dsl("ToDelete")).unwrap();
        let rel = v_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri(format!("/lighting/venues/ToDelete?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["status"], "deleted");
        assert!(!file_path.exists());
    }

    #[tokio::test]
    async fn delete_venue_not_found() {
        let (state, _dir) = test_state();
        let v_dir = _dir.path().join("v_del_nf");
        std::fs::create_dir(&v_dir).unwrap();
        let rel = v_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri(format!("/lighting/venues/nonexistent?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // Fixture types / venues: round-trip tests (PUT then GET)
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn put_then_get_fixture_type() {
        let (state, _dir) = test_state();
        let rel = "ft_roundtrip";
        let dsl = sample_fixture_type_dsl("RoundTrip");

        // PUT
        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/fixture-types/RoundTrip?dir={}", rel))
                    .header("content-type", "text/plain")
                    .body(Body::from(dsl))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // GET
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/fixture-types/RoundTrip?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["fixture_type"].is_object());
    }

    #[tokio::test]
    async fn put_then_get_venue() {
        let (state, _dir) = test_state();
        let rel = "v_roundtrip";
        let dsl = sample_venue_dsl("RoundTrip");

        // PUT
        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/venues/RoundTrip?dir={}", rel))
                    .header("content-type", "text/plain")
                    .body(Body::from(dsl))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // GET
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/venues/RoundTrip?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["venue"].is_object());
    }

    #[tokio::test]
    async fn put_then_delete_fixture_type() {
        let (state, _dir) = test_state();
        let rel = "ft_put_del";
        let dsl = sample_fixture_type_dsl("Deletable");

        // PUT
        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/fixture-types/Deletable?dir={}", rel))
                    .header("content-type", "text/plain")
                    .body(Body::from(dsl))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // DELETE
        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri(format!("/lighting/fixture-types/Deletable?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // GET should now 404
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/fixture-types/Deletable?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn put_then_delete_venue() {
        let (state, _dir) = test_state();
        let rel = "v_put_del";
        let dsl = sample_venue_dsl("Deletable");

        // PUT
        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/venues/Deletable?dir={}", rel))
                    .header("content-type", "text/plain")
                    .body(Body::from(dsl))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // DELETE
        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri(format!("/lighting/venues/Deletable?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // GET should now 404
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/venues/Deletable?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // resolve_lighting_dir: absolute path rejected
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_fixture_types_absolute_dir_rejected() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/fixture-types?dir=%2Ftmp%2Fevil")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn get_venues_absolute_dir_rejected() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/venues?dir=%2Ftmp%2Fevil")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    // -----------------------------------------------------------------------
    // Multiple fixture types in listing
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_fixture_types_multiple_files() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_multi");
        std::fs::create_dir(&ft_dir).unwrap();
        std::fs::write(ft_dir.join("a.light"), sample_fixture_type_dsl("TypeA")).unwrap();
        std::fs::write(ft_dir.join("b.light"), sample_fixture_type_dsl("TypeB")).unwrap();
        let rel = ft_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/fixture-types?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let types = parsed["fixture_types"].as_object().unwrap();
        assert_eq!(types.len(), 2);
        assert!(types.contains_key("TypeA"));
        assert!(types.contains_key("TypeB"));
    }

    #[tokio::test]
    async fn get_venues_multiple_files() {
        let (state, _dir) = test_state();
        let v_dir = _dir.path().join("v_multi");
        std::fs::create_dir(&v_dir).unwrap();
        std::fs::write(v_dir.join("a.light"), sample_venue_dsl("VenueA")).unwrap();
        std::fs::write(v_dir.join("b.light"), sample_venue_dsl("VenueB")).unwrap();
        let rel = v_dir.strip_prefix(_dir.path()).unwrap().to_str().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/venues?dir={}", rel))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let venues = parsed["venues"].as_object().unwrap();
        assert_eq!(venues.len(), 2);
        assert!(venues.contains_key("VenueA"));
        assert!(venues.contains_key("VenueB"));
    }

    #[tokio::test]
    async fn assets_are_served_from_the_store_with_containment() {
        let (state, dir) = test_state();
        let store = dir.path().join("lighting/.cache/assets/abc123");
        std::fs::create_dir_all(store.join("models")).unwrap();
        std::fs::write(store.join("rig-1-v1.json"), "{\"version\":1}").unwrap();
        std::fs::write(store.join("models/yoke.glb"), b"glTF").unwrap();
        std::fs::write(store.join("thumbnail.svg"), "<svg><script/></svg>").unwrap();
        std::fs::write(store.join("notes.txt"), "no").unwrap();
        // Something outside the store that a traversal would reach.
        std::fs::write(dir.path().join("lighting/secret.json"), "{}").unwrap();

        let get = |uri: String| {
            let app = router().with_state(state.clone());
            async move {
                app.oneshot(
                    http::Request::builder()
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap()
            }
        };

        let response = get("/lighting/assets/abc123/rig-1-v1.json".to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["content-type"],
            "application/json",
            "json is served as json"
        );
        assert!(response.headers()["cache-control"]
            .to_str()
            .unwrap()
            .contains("immutable"));
        assert_eq!(response_body(response).await, "{\"version\":1}");

        let response = get("/lighting/assets/abc123/models/yoke.glb".to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "model/gltf-binary");
        assert_eq!(response.headers()["x-content-type-options"], "nosniff");
        assert!(response.headers().get("content-security-policy").is_none());

        let response = get("/lighting/assets/abc123/thumbnail.svg".to_string()).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["content-type"], "image/svg+xml");
        let csp = response.headers()["content-security-policy"]
            .to_str()
            .unwrap();
        assert!(
            csp.contains("default-src 'none'") && csp.contains("sandbox"),
            "{csp}"
        );

        // Not an asset type, missing, a directory, and a traversal.
        for (uri, why) in [
            (
                "/lighting/assets/abc123/notes.txt",
                "text is not an asset type",
            ),
            ("/lighting/assets/abc123/models/head.glb", "missing"),
            ("/lighting/assets/abc123/models", "a directory"),
            ("/lighting/assets/../secret.json", "traversal"),
            ("/lighting/assets/abc123/../../secret.json", "traversal"),
        ] {
            let response = get(uri.to_string()).await;
            assert!(
                response.status() == StatusCode::NOT_FOUND
                    || response.status() == StatusCode::BAD_REQUEST
                    || response.status() == StatusCode::FORBIDDEN,
                "{uri} ({why}): {}",
                response.status()
            );
        }
    }

    #[tokio::test]
    async fn a_project_without_a_store_answers_not_found() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/assets/abc/rig.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // GET /lighting/readiness
    // -----------------------------------------------------------------------

    const TYPE_PAR: &str =
        "fixture_type \"Par\" {\n  channels: 2\n  channel_map: { \"dimmer\": 1, \"red\": 2 }\n}\n";

    /// A project on disk plus a player running a DMX engine over it.
    struct Rig {
        state: WebUiState,
        _state_dir: tempfile::TempDir,
        _project: tempfile::TempDir,
        /// The port olad's web server is looked for on, where nothing listens.
        _olad: crate::testutil::net::ClosedPort,
    }

    /// Writes one song directory per (name, show file body) and loads them.
    fn song_registry(
        root: &std::path::Path,
        songs: &[(&str, &str)],
    ) -> std::sync::Arc<crate::songs::Songs> {
        let songs_dir = root.join("songs");
        std::fs::create_dir_all(&songs_dir).unwrap();
        for (name, show) in songs {
            let dir = songs_dir.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            crate::testutil::write_wav(dir.join("kick.wav"), vec![vec![0i32; 4410]], 44100)
                .unwrap();
            std::fs::write(dir.join("show.light"), show).unwrap();
            std::fs::write(
                dir.join("song.yaml"),
                format!(
                    "kind: song\nname: {name}\ntracks:\n  - name: kick\n    file: kick.wav\n\
                     lighting:\n  - file: show.light\n"
                ),
            )
            .unwrap();
        }
        crate::songs::get_all_songs(&songs_dir).unwrap()
    }

    /// `venue` is the venue file's body (`None` for no current venue), `songs`
    /// are (directory, show file body) pairs, `universes` the configured
    /// outputs. olad's web server is pointed at a port nothing listens on so
    /// the probe never meets a real daemon.
    fn rig(
        types: &[(&str, &str)],
        venue: Option<&str>,
        songs: &[(&str, &str)],
        universes: &[u16],
    ) -> Rig {
        rig_with_library(types, venue, songs, universes, &[])
    }

    /// [`rig`], with these archives in `lighting/library/` before it loads.
    fn rig_with_library(
        types: &[(&str, &str)],
        venue: Option<&str>,
        songs: &[(&str, &str)],
        universes: &[u16],
        library: &[(&str, &[u8])],
    ) -> Rig {
        let project = tempfile::tempdir().unwrap();
        let root = project.path();
        std::fs::create_dir_all(root.join("lighting/library")).unwrap();
        for (file, bytes) in library {
            std::fs::write(root.join("lighting/library").join(file), bytes).unwrap();
        }
        std::fs::create_dir_all(root.join("types")).unwrap();
        std::fs::create_dir_all(root.join("venues")).unwrap();
        for (file, body) in types {
            std::fs::write(root.join("types").join(file), body).unwrap();
        }
        if let Some(venue) = venue {
            std::fs::write(root.join("venues/v.light"), venue).unwrap();
        }
        let songs = song_registry(root, songs);

        let olad = crate::testutil::net::ClosedPort::bind();
        let dead_port = olad.port();
        let universe_yaml: String = universes
            .iter()
            .map(|u| format!("  - universe: {u}\n    name: u{u}\n"))
            .collect();
        let current_venue = if venue.is_some() {
            "  current_venue: v\n"
        } else {
            ""
        };
        let dmx_yaml = format!(
            "ola_http_port: {dead_port}\nuniverses:\n{universe_yaml}lighting:\n  directories:\n    \
             fixture_types: types\n    venues: venues\n{current_venue}  groups:\n    washes:\n      \
             name: washes\n      constraints:\n        - AllOf: [\"wash\"]\n"
        );
        let dmx: crate::config::Dmx = config::Config::builder()
            .add_source(config::File::from_str(&dmx_yaml, config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        let engine = crate::dmx::engine::Engine::new(
            &dmx,
            dmx.lighting(),
            Some(root),
            crate::dmx::ola_client::OlaClientFactory::create_mock_client(),
        )
        .unwrap();
        let (state, state_dir) = test_state_with_dmx(songs, std::sync::Arc::new(engine));
        Rig {
            state,
            _state_dir: state_dir,
            _project: project,
            _olad: olad,
        }
    }

    async fn readiness(state: WebUiState) -> serde_json::Value {
        let response = router()
            .with_state(state)
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/readiness")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_str(&response_body(response).await).unwrap()
    }

    #[tokio::test]
    async fn readiness_without_dmx_has_no_venue_but_still_reports_shows() {
        let project = tempfile::tempdir().unwrap();
        let songs = song_registry(
            project.path(),
            &[
                (
                    "Good",
                    "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n}\n",
                ),
                // No `duration`: the song does not load, DMX or not.
                (
                    "Bad",
                    "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\"\n}\n",
                ),
            ],
        );
        let (state, _dir) = test_state_with_registry(songs);
        let report = readiness(state).await;
        assert_eq!(report["dmx"], json!(false));
        assert_eq!(report["venue"], json!(null));
        assert_eq!(report["groups"], json!([]));
        assert_eq!(report["output"]["olad"], json!(null));
        assert_eq!(report["fixture_types"]["unresolved"], json!([]));

        let shows = report["shows"].as_array().unwrap();
        let bad = shows.iter().find(|s| s["song"] == json!("Bad")).unwrap();
        assert!(bad["error"].as_str().unwrap().contains("duration"));
        let good = shows.iter().find(|s| s["song"] == json!("Good")).unwrap();
        assert!(good.get("error").is_none());
        // Nothing that needs a venue is guessed at without one.
        for w in good["warnings"].as_array().unwrap() {
            assert!(
                !["empty-group", "unconfigured-universe", "capability-gap"]
                    .contains(&w["kind"].as_str().unwrap()),
                "{w}"
            );
        }
    }

    #[tokio::test]
    async fn readiness_puts_patch_findings_on_the_venue_not_under_every_song() {
        let rig = rig(
            &[("par.light", TYPE_PAR)],
            // Partly over each other: an overlap. And a gang at 1:10.
            Some(
                "venue \"v\" {\n  fixture \"A\" Par @ 1:1 tags [\"wash\"]\n  \
                 fixture \"B\" Par @ 1:2\n  fixture \"G1\" Par @ 1:10\n  \
                 fixture \"G2\" Par @ 1:10\n}\n",
            ),
            &[(
                "Esaweg",
                "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n}\n",
            )],
            &[1],
        );
        let report = readiness(rig.state.clone()).await;
        assert!(report["venue_error"].is_null(), "{report}");
        let patch = report["patch_warnings"].as_array().unwrap();
        assert_eq!(patch.len(), 1, "the gang is quiet: {report}");
        assert_eq!(patch[0]["kind"], "patch-overlap");
        assert!(patch[0]["message"].as_str().unwrap().contains("\"A\""));
        for show in report["shows"].as_array().unwrap() {
            assert!(
                show["warnings"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|w| w["kind"] != "patch-overlap"),
                "{report}"
            );
        }
    }

    #[tokio::test]
    async fn readiness_says_when_the_venue_will_not_load_and_which_fixture() {
        let rig = rig(
            &[("par.light", TYPE_PAR)],
            // A native type has no modes: this fails the whole venue.
            Some("venue \"v\" {\n  fixture \"A\" Par @ 1:1\n  fixture \"B\" Par mode \"x\" @ 1:5\n}\n"),
            &[],
            &[1],
        );
        let report = readiness(rig.state.clone()).await;
        assert_eq!(report["venue_error"]["venue"], "v", "{report}");
        assert_eq!(report["venue_error"]["fixture"], "B", "{report}");
        assert!(
            report["venue_error"]["reason"]
                .as_str()
                .unwrap()
                .contains("not GDTF-sourced"),
            "{report}"
        );
        // And the fixture is listed as unresolved, with the same reason.
        assert_eq!(report["fixture_types"]["unresolved"][0]["fixture"], "B");
    }

    #[tokio::test]
    async fn readiness_reports_an_untagged_venue_as_groups_with_no_fixtures() {
        let rig = rig(
            &[("par.light", TYPE_PAR)],
            // A is tagged, B is not; a show that targets `washes` finds A and a
            // show that targets `movers` (declared nowhere) finds nothing.
            Some("venue \"v\" {\n  fixture \"A\" Par @ 1:1 tags [\"wash\"]\n  fixture \"B\" Par @ 1:3\n}\n"),
            &[(
                "Esaweg",
                "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n    \
                 movers: static color: \"blue\", duration: 2s\n}\n",
            )],
            &[1],
        );
        let report = readiness(rig.state.clone()).await;

        assert_eq!(report["dmx"], json!(true));
        assert_eq!(report["venue"]["name"], json!("v"));
        assert_eq!(report["venue"]["fixtures"], json!(2));
        assert_eq!(report["venue"]["placed"], json!(0));
        assert_eq!(report["venue"]["focus_points"], json!([]));
        assert_eq!(report["fixture_types"]["in_use"], json!(["Par"]));
        assert_eq!(report["fixture_types"]["unresolved"], json!([]));

        let groups = report["groups"].as_array().unwrap();
        let by_name = |name: &str| groups.iter().find(|g| g["name"] == json!(name)).unwrap();
        assert_eq!(by_name("washes")["fixtures"], json!(1));
        assert_eq!(by_name("movers")["fixtures"], json!(0));
        assert_eq!(by_name("movers")["songs"], json!(["Esaweg"]));

        let shows = report["shows"].as_array().unwrap();
        assert_eq!(shows.len(), 1);
        assert_eq!(shows[0]["song"], json!("Esaweg"));
        assert_eq!(shows[0]["files"], json!(["show.light"]));
        assert!(shows[0].get("error").is_none(), "{report}");
        let kinds: Vec<&str> = shows[0]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|w| w["kind"].as_str())
            .collect();
        assert!(kinds.contains(&"empty-group"), "{kinds:?}");
        // The Groups check carries the empty group, not the song's.
        let checks = report["checks"].as_array().unwrap();
        let groups_check = checks.iter().find(|c| c["key"] == json!("groups")).unwrap();
        assert_eq!(groups_check["state"], json!("attention"), "{report}");

        // Nothing listens where olad's web server should be.
        assert_eq!(report["output"]["universes"], json!([1]));
        assert_eq!(report["output"]["unconfigured"], json!([]));
        assert_eq!(report["output"]["olad"]["reachable"], json!(false));
    }

    #[tokio::test]
    async fn readiness_reports_a_show_that_does_not_parse() {
        let rig = rig(
            &[("par.light", TYPE_PAR)],
            Some("venue \"v\" {\n  fixture \"A\" Par @ 1:1 tags [\"wash\"]\n}\n"),
            &[
                (
                    "Good",
                    "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n}\n",
                ),
                // No `duration`: the parser refuses it, and the song with it.
                (
                    "Bad",
                    "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\"\n}\n",
                ),
            ],
            &[1],
        );
        let report = readiness(rig.state.clone()).await;
        let shows = report["shows"].as_array().unwrap();
        let bad = shows.iter().find(|s| s["song"] == json!("Bad")).unwrap();
        let error = bad["error"].as_str().expect("error present");
        assert!(error.contains("duration"), "{error}");
        let good = shows.iter().find(|s| s["song"] == json!("Good")).unwrap();
        assert!(good.get("error").is_none());
    }

    #[tokio::test]
    async fn readiness_lists_a_universe_with_no_output() {
        let rig = rig(
            &[("par.light", TYPE_PAR)],
            Some("venue \"v\" {\n  fixture \"A\" Par @ 1:1\n  fixture \"B\" Par @ 4:1\n}\n"),
            &[(
                "Song",
                "show \"S\" {\n    @00:00.000\n    all: static color: \"red\", duration: 2s\n}\n",
            )],
            &[1],
        );
        let report = readiness(rig.state.clone()).await;
        assert_eq!(report["output"]["universes"], json!([1, 4]));
        assert_eq!(report["output"]["unconfigured"], json!([4]));
        // Output carries it; no song repeats it.
        assert_eq!(report["shows"].as_array().unwrap().len(), 1);
        for show in report["shows"].as_array().unwrap() {
            for w in show["warnings"].as_array().unwrap() {
                assert_ne!(w["kind"], json!("unconfigured-universe"), "{report}");
            }
        }
    }

    #[tokio::test]
    async fn readiness_says_why_a_fixture_type_did_not_load() {
        let rig = rig(
            &[
                ("par.light", TYPE_PAR),
                (
                    "ghost.fixture",
                    "fixture_type \"Ghost\"\n  from gdtf(\"lighting/library/missing.gdtf\")\n{ }\n",
                ),
            ],
            Some(
                "venue \"v\" {\n  fixture \"A\" Par @ 1:1\n  fixture \"G\" Ghost mode \"M\" @ 1:5\n  \
                 fixture \"N\" Nope @ 1:9\n}\n",
            ),
            &[],
            &[1],
        );
        let report = readiness(rig.state.clone()).await;
        assert_eq!(report["fixture_types"]["in_use"], json!(["Par"]));
        let unresolved = report["fixture_types"]["unresolved"].as_array().unwrap();
        assert_eq!(unresolved.len(), 2);
        assert_eq!(unresolved[0]["fixture"], json!("G"));
        assert_eq!(unresolved[0]["type"], json!("Ghost"));
        assert!(
            unresolved[0]["reason"]
                .as_str()
                .unwrap()
                .contains("missing.gdtf"),
            "the loader's own reason: {report}"
        );
        assert_eq!(unresolved[1]["fixture"], json!("N"));
        assert!(unresolved[1]["reason"]
            .as_str()
            .unwrap()
            .contains("no fixture type named 'Nope'"));
    }

    #[tokio::test]
    async fn readiness_lists_the_library_s_name_collisions_and_unreadable_archives() {
        let archive = synthetic_gdtf_bytes();
        let rig = rig_with_library(
            &[("par.light", TYPE_PAR)],
            Some(
                "venue \"v\" {\n  fixture \"A\" Par @ 1:1\n  \
                 fixture \"B\" \"Synth Brick (b)\" mode \"8: RGBS\" @ 1:5\n}\n",
            ),
            &[],
            &[1],
            &[
                ("a.gdtf", archive.as_slice()),
                ("b.gdtf", archive.as_slice()),
                ("junk.gdtf", b"not a zip".as_slice()),
            ],
        );
        let report = readiness(rig.state.clone()).await;
        assert!(report["venue_error"].is_null(), "{report}");
        let library = report["fixture_types"]["library"].as_array().unwrap();
        let kinds: Vec<(&str, &str)> = library
            .iter()
            .map(|f| (f["kind"].as_str().unwrap(), f["file"].as_str().unwrap()))
            .collect();
        assert_eq!(
            kinds,
            [
                ("gdtf-name-collision", "b.gdtf"),
                ("gdtf-unreadable", "junk.gdtf")
            ],
            "{report}"
        );
        // One finding per renamed archive, naming both.
        let collision = library[0]["message"].as_str().unwrap();
        assert!(
            collision.contains("a.gdtf") && collision.contains("\"Synth Brick (b)\""),
            "{collision}"
        );
    }

    #[tokio::test]
    async fn readiness_without_a_current_venue_has_none() {
        let rig = rig(&[("par.light", TYPE_PAR)], None, &[], &[1]);
        let report = readiness(rig.state.clone()).await;
        assert_eq!(report["dmx"], json!(true));
        assert_eq!(report["venue"], json!(null));
        assert_eq!(report["output"]["universes"], json!([]));
    }

    // -----------------------------------------------------------------------
    // GET /lighting/fit
    // -----------------------------------------------------------------------

    const TYPE_MOVER: &str = "fixture_type \"Mover\" {\n  channels: 6\n  channel_map: { \"pan\": 1, \"tilt\": 2, \"red\": 3, \"green\": 4, \"blue\": 5, \"dimmer\": 6 }\n}\n";

    const TYPE_WASH: &str = "fixture_type \"Wash\" {\n  channels: 4\n  channel_map: { \"red\": 1, \"green\": 2, \"blue\": 3, \"dimmer\": 4 }\n}\n";

    async fn fit(state: WebUiState) -> serde_json::Value {
        let response = router()
            .with_state(state)
            .oneshot(
                http::Request::builder()
                    .uri("/lighting/fit")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        serde_json::from_str(&response_body(response).await).unwrap()
    }

    #[tokio::test]
    async fn fit_without_dmx_has_no_venue_and_no_groups() {
        let project = tempfile::tempdir().unwrap();
        let songs = song_registry(
            project.path(),
            &[(
                "Song",
                "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n}\n",
            )],
        );
        let (state, _dir) = test_state_with_registry(songs);
        let report = fit(state).await;
        assert_eq!(report["venue"], json!(null));
        assert_eq!(report["groups"], json!([]));
        assert_eq!(report["focus_points_wanted"], json!([]));
    }

    #[tokio::test]
    async fn fit_suggests_tags_for_an_untagged_group_and_lists_wanted_focus_points() {
        // `washes` needs the `wash` tag and is empty. The shows ask it to move
        // and to colour, which only the Mover does.
        let rig = rig(
            &[("par.light", TYPE_PAR), ("mover.light", TYPE_MOVER)],
            Some(
                "venue \"v\" {\n  fixture \"A\" Par @ 1:1\n  fixture \"B\" Par @ 1:3\n  \
                 fixture \"M\" Mover @ 1:10 tags [\"other\"]\n  focus \"center\" (0, 3, 0)\n}\n",
            ),
            &[(
                "Esaweg",
                "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n    \
                 washes: move focus: \"drummer\", duration: 1s\n    \
                 washes: move focus: \"center\", duration: 1s\n}\n",
            )],
            &[1],
        );
        let report = fit(rig.state.clone()).await;

        assert_eq!(report["venue"]["name"], json!("v"));
        assert_eq!(report["venue"]["fixtures"].as_array().unwrap().len(), 3);
        assert_eq!(report["venue"]["focus_points"], json!(["center"]));
        let m = report["venue"]["fixtures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == json!("M"))
            .unwrap();
        assert_eq!(m["tags"], json!(["other"]));
        assert_eq!(
            m["capabilities"],
            json!(["color", "pan_tilt", "strobe", "dimmer"])
        );

        let groups = report["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        let washes = &groups[0];
        assert_eq!(washes["name"], json!("washes"));
        assert_eq!(washes["defined"], json!(true));
        assert_eq!(washes["needs"]["all_of"], json!(["wash"]));
        assert_eq!(washes["fixtures"], json!([]));
        assert_eq!(washes["songs"], json!(["Esaweg"]));
        assert_eq!(washes["wants"], json!(["move", "color"]));
        assert_eq!(washes["suggestion"]["fixtures"], json!(["M"]));
        assert_eq!(washes["suggestion"]["tags"], json!(["wash"]));
        assert_eq!(washes["suggestion"]["reason"]["count"], json!(1));
        assert_eq!(washes["suggestion"]["reason"]["type"], json!("Mover"));
        assert_eq!(
            washes["suggestion"]["reason"]["can"],
            json!(["move", "color"])
        );
        assert_eq!(washes["others"], json!([]));

        assert_eq!(
            report["focus_points_wanted"],
            json!([{"name": "drummer", "songs": ["Esaweg"]}])
        );
    }

    #[tokio::test]
    async fn fit_offers_other_clusters_and_skips_groups_that_need_no_suggestion() {
        let rig = rig(
            &[("par.light", TYPE_PAR), ("mover.light", TYPE_MOVER)],
            Some(
                "venue \"v\" {\n  fixture \"A\" Par @ 1:1\n  fixture \"B\" Par @ 1:3\n  \
                 fixture \"M\" Mover @ 1:10\n  fixture \"W\" Par @ 1:20 tags [\"wash\"]\n}\n",
            ),
            &[
                (
                    "Untagged",
                    "show \"S\" {\n    @00:00.000\n    movers: static color: \"red\", duration: 2s\n}\n",
                ),
                (
                    "Tagged",
                    "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n}\n",
                ),
            ],
            &[1],
        );
        let report = fit(rig.state.clone()).await;
        let groups = report["groups"].as_array().unwrap();
        let by_name = |n: &str| groups.iter().find(|g| g["name"] == json!(n)).unwrap();
        // `movers` is declared nowhere in this rig: nothing to tag for.
        assert_eq!(by_name("movers")["defined"], json!(false));
        assert_eq!(by_name("movers")["suggestion"], json!(null));
        // `washes` already finds W.
        assert_eq!(by_name("washes")["fixtures"], json!(["W"]));
        assert_eq!(by_name("washes")["suggestion"], json!(null));
    }

    #[tokio::test]
    async fn fit_lists_other_clusters_under_others() {
        // Three colour-capable candidates in two types: Pars win (two of them),
        // the Mover is the alternative.
        let rig = rig(
            &[("wash.light", TYPE_WASH), ("mover.light", TYPE_MOVER)],
            Some(
                "venue \"v\" {\n  fixture \"A\" Wash @ 1:1\n  fixture \"B\" Wash @ 1:7\n  \
                 fixture \"M\" Mover @ 1:10\n}\n",
            ),
            &[(
                "Song",
                "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n}\n",
            )],
            &[1],
        );
        let report = fit(rig.state.clone()).await;
        let washes = &report["groups"][0];
        assert_eq!(washes["suggestion"]["fixtures"], json!(["A", "B"]));
        assert_eq!(washes["others"][0]["type"], json!("Mover"));
        assert_eq!(washes["others"][0]["fixtures"], json!(["M"]));
    }

    #[tokio::test]
    async fn fit_says_which_want_nothing_meets() {
        let rig = rig(
            &[("wash.light", TYPE_WASH)],
            Some("venue \"v\" {\n  fixture \"A\" Wash @ 1:1\n}\n"),
            &[(
                "Song",
                "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n    \
                 washes: move pan: 45deg, tilt: 0deg, duration: 1s\n}\n",
            )],
            &[1],
        );
        let report = fit(rig.state.clone()).await;
        let washes = &report["groups"][0];
        assert_eq!(washes["suggestion"], json!(null));
        assert_eq!(washes["unmet"], json!(["move"]));
        assert_eq!(washes["unmet_together"], json!(false));
    }

    #[tokio::test]
    async fn fit_lists_unconfigured_universes_and_the_olad_port() {
        let rig = rig(
            &[("par.light", TYPE_PAR)],
            Some("venue \"v\" {\n  fixture \"A\" Par @ 1:1\n  fixture \"B\" Par @ 4:1\n}\n"),
            &[(
                "Song",
                "show \"S\" {\n    @00:00.000\n    all: static color: \"red\", duration: 2s\n}\n",
            )],
            &[1],
        );
        let report = fit(rig.state.clone()).await;
        assert_eq!(report["output"]["unconfigured"], json!([4]));
        // Nothing listens where olad's web server should be.
        assert_eq!(report["output"]["reachable"], json!(false));
        assert_eq!(report["output"]["unpatched"], json!([]));
        assert!(report["output"]["ola_http_port"].as_u64().unwrap() > 0);
    }

    // -----------------------------------------------------------------------
    // POST /lighting/evaluate (the 3D preview)
    // -----------------------------------------------------------------------

    const PREVIEW_VENUE: &str = "venue \"v\" {\n  \
        fixture \"M\" Mover @ 1:1 tags [\"wash\"] position (0, 3.5, 4) rotation (0, 0, 180)\n  \
        fixture \"P\" Par @ 1:20\n  \
        focus \"center\" (0, 0, 0)\n}\n";

    const PREVIEW_SHOW: &str = "show \"S\" {\n    @00:00.000\n    \
        washes: static color: \"red\", duration: 10s\n    \
        @00:01.000\n    washes: move focus: \"center\", duration: 4s\n}\n";

    async fn evaluate(
        state: WebUiState,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let response = router()
            .with_state(state)
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/lighting/evaluate")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        (
            status,
            serde_json::from_str(&response_body(response).await).unwrap(),
        )
    }

    #[tokio::test]
    async fn evaluate_matches_evaluate_show_and_lists_untouched_fixtures() {
        let rig = rig(
            &[("mover.light", TYPE_MOVER), ("par.light", TYPE_PAR)],
            Some(PREVIEW_VENUE),
            &[("Song", PREVIEW_SHOW)],
            &[1],
        );
        let (status, body) = evaluate(
            rig.state.clone(),
            json!({"song": "Song", "times": [0.5, 3.0]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let evaluations = body["evaluations"].as_array().unwrap();
        assert_eq!(evaluations.len(), 2);

        // The same shows evaluated directly, with the group resolved by hand.
        let song = rig.state.player.songs().get("Song").unwrap();
        let shows = crate::lighting::evaluate::registered_shows(&song);
        let system = rig
            .state
            .player
            .dmx_engine()
            .and_then(|dmx| dmx.broadcast_handles().lighting_system)
            .unwrap();
        let (fixtures, focus) = {
            let guard = system.lock();
            (
                guard.get_current_venue_fixtures().unwrap(),
                guard
                    .get_current_venue()
                    .unwrap()
                    .focus_points()
                    .iter()
                    .map(|(n, p)| (n.clone(), *p))
                    .collect::<std::collections::HashMap<_, _>>(),
            )
        };
        let direct = crate::lighting::evaluate::evaluate_show(
            shows,
            &fixtures,
            &focus,
            None,
            &[
                std::time::Duration::from_secs_f64(0.5),
                std::time::Duration::from_secs_f64(3.0),
            ],
            |mut effect| {
                effect.target_fixtures = vec!["M".to_string()];
                effect
            },
        );
        for (got, want) in evaluations.iter().zip(&direct) {
            let maps = crate::webui::state::stage_frame(&want.fixtures, &want.poses);
            let (poses, cells) = (json!(maps.poses), json!(maps.cells));
            assert_eq!(got["fixtures"], json!(maps.fixtures));
            assert_eq!(got["strobe_rates"], json!(maps.strobe_rates));
            // Pan and tilt are exact; the beam's direction and footprint are
            // trigonometry, which two evaluations agree on to the last bit or
            // two, not always the last.
            for (name, want_pose) in poses.as_object().unwrap() {
                let got_pose = &got["poses"][name];
                assert_eq!(got_pose["pan"], want_pose["pan"]);
                assert_eq!(got_pose["tilt"], want_pose["tilt"]);
                for key in ["aim", "floor"] {
                    let (g, w) = (got_pose[key].as_array(), want_pose[key].as_array());
                    assert_eq!(g.map(|a| a.len()), w.map(|a| a.len()), "{name} {key}");
                    for (g, w) in g.into_iter().flatten().zip(w.into_iter().flatten()) {
                        assert!((g.as_f64().unwrap() - w.as_f64().unwrap()).abs() < 1e-9);
                    }
                }
            }
            assert_eq!(
                got["poses"].as_object().unwrap().len(),
                poses.as_object().unwrap().len()
            );
            assert_eq!(got["cells"], json!(cells));
            assert_eq!(got["time"], json!(want.time.as_secs_f64()));
        }
        // The move is under way at 3 s: the mover has left rest.
        assert_ne!(
            evaluations[0]["poses"]["M"], evaluations[1]["poses"]["M"],
            "{body}"
        );
        assert_eq!(evaluations[1]["fixtures"]["M"]["red"], json!(255));

        let active = evaluations[1]["active_effects"].as_array().unwrap();
        let kinds: Vec<&str> = active.iter().filter_map(|e| e["kind"].as_str()).collect();
        assert!(
            kinds.contains(&"Static") && kinds.contains(&"Move"),
            "{kinds:?}"
        );
        assert!(active.iter().all(|e| e["groups"] == json!(["washes"])));
        let mv = active.iter().find(|e| e["kind"] == json!("Move")).unwrap();
        assert_eq!(mv["elapsed"], json!(2.0));
        assert_eq!(mv["duration"], json!(4.0));

        // P is in no group the show targets.
        assert_eq!(body["untouched"], json!(["P"]));
    }

    #[tokio::test]
    async fn evaluate_answers_unknown_and_unloadable_songs() {
        let rig = rig(
            &[("mover.light", TYPE_MOVER), ("par.light", TYPE_PAR)],
            Some(PREVIEW_VENUE),
            &[
                ("Song", PREVIEW_SHOW),
                // No `duration`: the show does not parse, so the song does not load.
                (
                    "Bad",
                    "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\"\n}\n",
                ),
            ],
            &[1],
        );
        let (status, body) =
            evaluate(rig.state.clone(), json!({"song": "Nope", "times": [0]})).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

        let (status, body) =
            evaluate(rig.state.clone(), json!({"song": "Bad", "times": [0]})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("duration"),
            "{body}"
        );

        let (status, _) =
            evaluate(rig.state.clone(), json!({"song": "Song", "times": [-1.0]})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn evaluate_is_a_read_and_works_while_locked() {
        let rig = rig(
            &[("mover.light", TYPE_MOVER), ("par.light", TYPE_PAR)],
            Some(PREVIEW_VENUE),
            &[("Song", PREVIEW_SHOW)],
            &[1],
        );
        rig.state.player.set_locked(true);
        let response = super::super::guarded_router(&rig.state)
            .with_state(rig.state.clone())
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/lighting/evaluate")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"song": "Song", "times": [1.0]}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn evaluate_refuses_times_it_cannot_hold_instead_of_panicking() {
        let rig = rig(
            &[("mover.light", TYPE_MOVER), ("par.light", TYPE_PAR)],
            Some(PREVIEW_VENUE),
            &[("Song", PREVIEW_SHOW)],
            &[1],
        );
        // Past what a Duration holds: used to panic the handler.
        let (status, body) =
            evaluate(rig.state.clone(), json!({"song": "Song", "times": [1e300]})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], "time out of range");
        // Representable but longer than any song.
        let (status, _) =
            evaluate(rig.state.clone(), json!({"song": "Song", "times": [1e6]})).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, body) =
            evaluate(rig.state.clone(), json!({"song": "Song", "times": [2.5]})).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    // -----------------------------------------------------------------------
    // A GDTF's modes, as the fixture page shows them
    // -----------------------------------------------------------------------

    async fn inspect(bytes: &[u8]) -> serde_json::Value {
        json!(inspect_description(
            &lighting::gdtf::parse_archive(bytes).unwrap()
        ))
    }

    #[tokio::test]
    async fn inspect_gdtf_says_what_each_mode_can_do() {
        let parsed = inspect(&synthetic_gdtf_bytes()).await;
        let modes = parsed["modes"].as_array().unwrap();
        let rgbs = modes.iter().find(|m| m["name"] == "8: RGBS").unwrap();
        // RGB and a strobe, and a dimmer only as a virtual channel.
        assert_eq!(rgbs["capabilities"], json!(["color", "strobe"]), "{rgbs}");
        assert_eq!(rgbs["strobe_range"]["min_hz"], json!(0.4), "{rgbs}");
        assert_eq!(rgbs["strobe_range"]["max_hz"], json!(25.0), "{rgbs}");
        assert_eq!(rgbs["cells"], json!(0));
        assert!(rgbs.get("refused").is_none());
        assert!(rgbs["channels"]
            .as_array()
            .unwrap()
            .contains(&json!([1, "red"])));
        assert!(rgbs.get("warnings").is_none(), "nothing reads them");

        let mover = modes.iter().find(|m| m["name"] == "Mover 16bit").unwrap();
        assert_eq!(mover["capabilities"], json!(["pan_tilt"]), "{mover}");
        assert!(mover.get("strobe_range").is_none());
        assert_eq!(mover["footprint"], json!(5));
    }

    #[tokio::test]
    async fn inspect_gdtf_answers_every_mode_of_a_many_mode_archive() {
        // A big real GDTF has dozens of modes; all of them are distilled (off
        // the async worker) and reported.
        let xml = crate::lighting::gdtf::SYNTHETIC_DESCRIPTION;
        let start = xml.find("<DMXMode Name=\"8: RGBS\"").unwrap();
        let end = start + xml[start..].find("</DMXMode>").unwrap() + "</DMXMode>".len();
        let template = &xml[start..end];
        let many: String = (1..=30)
            .map(|n| template.replace("8: RGBS", &format!("Mode {n}")))
            .collect::<Vec<_>>()
            .join("\n");
        let xml = format!("{}{many}{}", &xml[..start], &xml[end..]);
        let bytes = crate::lighting::gdtf::build_zip(&[("description.xml", xml.as_bytes())]);

        let parsed = inspect(&bytes).await;
        let modes = parsed["modes"].as_array().unwrap();
        for n in 1..=30 {
            let mode = modes
                .iter()
                .find(|m| m["name"] == format!("Mode {n}").as_str())
                .unwrap_or_else(|| panic!("Mode {n} missing from {modes:?}"));
            assert_eq!(mode["capabilities"], json!(["color", "strobe"]), "{mode}");
            assert!(mode.get("refused").is_none(), "{mode}");
        }
        assert!(modes.iter().any(|m| m["name"] == "Mover 16bit"));
        assert_eq!(modes.len(), 31);
    }

    #[tokio::test]
    async fn inspect_gdtf_refuses_a_mode_it_cannot_tell_from_another() {
        // Two modes with one name: an import takes the first, so the second
        // is listed with its reason, not offered.
        let xml = crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.replace("Mover 16bit", "8: RGBS");
        let bytes = crate::lighting::gdtf::build_zip(&[("description.xml", xml.as_bytes())]);
        let parsed = inspect(&bytes).await;
        let modes = parsed["modes"].as_array().unwrap();
        assert_eq!(modes.len(), 2);
        assert!(modes[0].get("refused").is_none());
        assert!(
            modes[1]["refused"]
                .as_str()
                .unwrap()
                .contains("another mode"),
            "{modes:?}"
        );
        assert!(modes[1].get("capabilities").is_none());
    }

    #[test]
    fn a_suggested_type_name_is_safe_in_a_fixture_file() {
        assert_eq!(
            suggested_type_name("Spiider \"Pro\" {v2}"),
            "Spiider Pro v2"
        );
        assert_eq!(suggested_type_name("  A   B "), "A B");
        assert_eq!(suggested_type_name("\"\"{}"), "fixture");
    }
}

#[cfg(test)]
mod venue_file_tests {
    use super::super::router;
    use super::super::test_helpers::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    const REL: &str = "v_files";

    const FILE: &str = "// House rig, do not lose me.\nvenue \"House\" {\n  fixture \"A\" brick @ 1:1 position (1, 2, 3)  // keep me\n  // spare\n  fixture \"B\" brick @ 1:5\n}\n";

    fn body_for(x: f64) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({
            "fixtures": [
                {"name": "A", "fixture_type": "brick", "universe": 1, "start_channel": 1, "position": [x, 2.0, 3.0]},
                {"name": "B", "fixture_type": "brick", "universe": 1, "start_channel": 5},
            ]
        }))
        .unwrap()
    }

    async fn put(
        app: &axum::Router,
        version: Option<&str>,
        body: Vec<u8>,
    ) -> (StatusCode, serde_json::Value) {
        let mut req = http::Request::builder()
            .method("PUT")
            .uri(format!("/lighting/venues/House?dir={REL}"))
            .header("content-type", "application/json");
        if let Some(v) = version {
            req = req.header("if-match", v);
        }
        let response = app
            .clone()
            .oneshot(req.body(Body::from(body)).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let text = response_body(response).await;
        (status, serde_json::from_str(&text).unwrap_or_default())
    }

    fn venues_dir(dir: &tempfile::TempDir) -> std::path::PathBuf {
        let d = dir.path().join(REL);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[tokio::test]
    async fn a_venue_in_a_file_with_another_stem_is_patched_there() {
        let (state, dir) = test_state();
        let d = venues_dir(&dir);
        std::fs::write(d.join("rig_2024.venue"), FILE).unwrap();
        let app = router().with_state(state);

        let (status, _) = put(&app, None, body_for(9.0)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            std::fs::read_dir(&d).unwrap().count(),
            1,
            "no second file may be created"
        );
        let text = std::fs::read_to_string(d.join("rig_2024.venue")).unwrap();
        assert!(
            text.starts_with("// House rig, do not lose me.\n"),
            "{text}"
        );
        assert!(text.contains("// keep me"), "{text}");
        assert!(text.contains("\n  // spare\n"), "{text}");
        assert!(text.contains("position (9, 2, 3)"), "{text}");
    }

    #[tokio::test]
    async fn a_broken_file_is_never_searched_and_left_byte_identical() {
        let (state, dir) = test_state();
        let d = venues_dir(&dir);
        let broken = "// mine\nvenue \"House\" {\n  fixture \"A\" brick @ !!!\n}\n";
        std::fs::write(d.join("rig.venue"), broken).unwrap();
        let app = router().with_state(state);

        // Nothing is guessed from a file that does not parse: "House" is
        // declared nowhere, so it is new, at its own canonical file.
        let (status, _) = put(&app, None, body_for(1.0)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            std::fs::read_to_string(d.join("rig.venue")).unwrap(),
            broken
        );
        assert!(d.join("house.venue").exists());
    }

    #[tokio::test]
    async fn a_broken_file_at_the_canonical_path_is_refused_and_left_byte_identical() {
        let (state, dir) = test_state();
        let d = venues_dir(&dir);
        let broken = "// mine\nvenue \"House\" {\n  fixture \"A\" brick @ !!!\n}\n";
        std::fs::write(d.join("house.venue"), broken).unwrap();
        let app = router().with_state(state);

        let (status, body) = put(&app, None, body_for(1.0)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let msg = body["error"].as_str().unwrap();
        assert!(msg.contains("house.venue"), "{msg}");
        assert_eq!(
            std::fs::read_to_string(d.join("house.venue")).unwrap(),
            broken
        );
        assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
    }

    #[tokio::test]
    async fn a_non_utf8_file_at_the_stem_is_refused() {
        let (state, dir) = test_state();
        let d = venues_dir(&dir);
        let bytes: Vec<u8> = vec![0xff, 0xfe, b'v', b'e'];
        std::fs::write(d.join("house.venue"), &bytes).unwrap();
        let app = router().with_state(state);

        let (status, _) = put(&app, None, body_for(1.0)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(std::fs::read(d.join("house.venue")).unwrap(), bytes);
    }

    #[tokio::test]
    async fn a_new_venue_is_generated() {
        let (state, dir) = test_state();
        venues_dir(&dir);
        let app = router().with_state(state);
        let (status, body) = put(&app, None, body_for(1.0)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body["version"].as_str().is_some_and(|v| v.len() == 64));
        assert!(dir.path().join(REL).join("house.venue").exists());
    }

    #[tokio::test]
    async fn a_stale_version_is_refused_and_the_current_one_saves() {
        let (state, dir) = test_state();
        let d = venues_dir(&dir);
        std::fs::write(d.join("house.venue"), FILE).unwrap();
        let app = router().with_state(state);

        // GET hands out the version.
        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/venues/House?dir={REL}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let got: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
        let v1 = got["version"].as_str().unwrap().to_string();

        // Another writer changes the file.
        let (status, saved) = put(&app, Some(&v1), body_for(4.0)).await;
        assert_eq!(status, StatusCode::OK);
        let v2 = saved["version"].as_str().unwrap().to_string();
        assert_ne!(v1, v2);
        let after_first = std::fs::read_to_string(d.join("house.venue")).unwrap();

        // The first reader saves against the version it read: refused,
        // file untouched, current version reported.
        let (status, body) = put(&app, Some(&v1), body_for(7.0)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["version"].as_str().unwrap(), v2);
        assert_eq!(
            std::fs::read_to_string(d.join("house.venue")).unwrap(),
            after_first
        );

        // The current version is accepted.
        let (status, _) = put(&app, Some(&v2), body_for(7.0)).await;
        assert_eq!(status, StatusCode::OK);
        assert!(std::fs::read_to_string(d.join("house.venue"))
            .unwrap()
            .contains("position (7, 2, 3)"));
    }

    #[tokio::test]
    async fn the_list_carries_each_venues_version() {
        let (state, dir) = test_state();
        let d = venues_dir(&dir);
        std::fs::write(d.join("house.venue"), FILE).unwrap();
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/venues?dir={REL}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let got: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
        assert_eq!(
            got["versions"]["House"].as_str().unwrap(),
            super::content_version(FILE.as_bytes())
        );
    }
}
