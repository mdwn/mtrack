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

use crate::webui::wire::fixture_types::{
    BeamBrief, FixtureTypeDeleted, FixtureTypeEntry, FixtureTypeGdtf, FixtureTypeList, GdtfBeam,
    GdtfInspection, GdtfMode, GdtfSummary, LightingFileError, ModeUse, SceneFixture, StrobeRange,
    TypeVenue, TypeVenueFixture, VenueDetail, VenueList, VenueScene,
};
use crate::webui::wire::messages::FixturePlacement;
use crate::webui::wire::preview::{
    LightFileInfo, LightFiles, LightingGroup, LightingGroups, LightingValidation,
};
use axum::{
    body::Bytes,
    extract::{Path, Query, RawQuery, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use super::super::config_io;
use super::super::server::WebUiState;
use crate::lighting;

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
/// taken). A different archive of the same file name already in the library
/// is a 409; anything else refused is a 400.
pub(super) async fn import_gdtf(
    State(state): State<WebUiState>,
    Query(query): Query<GdtfImportQuery>,
    mut multipart: axum::extract::Multipart,
) -> impl IntoResponse {
    let (filename, bytes) = first_multipart_file(&mut multipart).await?;
    let project = project_root(&state.config_path)?;
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
                DEFAULT_FIXTURE_TYPES_DIR,
            )
            .map_err(|e| {
                let status = if e.downcast_ref::<lighting::import::LibraryClash>().is_some() {
                    StatusCode::CONFLICT
                } else {
                    StatusCode::BAD_REQUEST
                };
                (status, e.to_string())
            }),
        )
    })
    .await?
    .map_err(|(status, e)| (status, Json(json!({"error": e}))).into_response())?;
    Ok::<_, axum::response::Response>((StatusCode::OK, Json(report)).into_response())
}

/// GET /api/lighting — lists available .light files from the songs directory.
pub(super) async fn get_lighting_files(State(state): State<WebUiState>) -> impl IntoResponse {
    let songs_path = state.songs_path().clone();
    let mut light_files =
        match super::helpers::spawn_blocking_io("scan for lighting files", move || {
            let mut files = Vec::new();
            find_light_files(&songs_path, &songs_path, &mut files)?;
            Ok::<_, std::io::Error>(files)
        })
        .await
        {
            Ok(f) => f,
            Err(e) => return e,
        };
    light_files.sort_by(|a, b| a.path.cmp(&b.path));
    (StatusCode::OK, Json(LightFiles { files: light_files })).into_response()
}

/// The `.light` files under `dir`, as the player knows them
/// (`songs::LIGHTING_EXTENSIONS`, in any case), by the one walk of a
/// project's lighting files (`project_files::files_under`), each with its
/// path relative to `base`.
pub(crate) fn find_light_files(
    base: &std::path::Path,
    dir: &std::path::Path,
    results: &mut Vec<LightFileInfo>,
) -> Result<(), std::io::Error> {
    for path in lighting::project_files::files_under(dir, crate::songs::LIGHTING_EXTENSIONS)? {
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
    Ok(())
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
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Lighting file not found: {}", name)})),
            )
                .into_response();
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

/// Where the light show file `name` (a path below the songs root) is, or
/// would be written: held inside the root by the one containment rule
/// MCP's tools take (`SafePath::validate_relative`: no traversal, no linked
/// way out, a link to nothing refused), and a lighting file by its
/// extension (`FileKind`), as the delete takes it.
#[allow(clippy::result_large_err)]
fn light_show_path(
    state: &WebUiState,
    name: &str,
) -> Result<std::path::PathBuf, axum::response::Response> {
    use super::super::safe_path::{SafePath, VerifiedRoot};
    let root = VerifiedRoot::new(&state.songs_path()).map_err(|e| e.into_response())?;
    // A hidden file or directory is one the loader and listing pass over
    // (`songs::is_hidden`): a save there would vanish.
    if crate::songs::has_hidden_part(std::path::Path::new(name)) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!(
                "{name} is a hidden file name; the lighting system would not read it"
            )})),
        )
            .into_response());
    }
    let path = SafePath::validate_relative(name, &root).map_err(|e| e.into_response())?;
    if crate::songs::FileKind::of(&path) != crate::songs::FileKind::Lighting {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Can only write .light files"})),
        )
            .into_response());
    }
    Ok(path)
}

/// PUT /api/lighting/:name — validates and atomically writes a lighting DSL file.
pub(super) async fn put_lighting_file(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    body: String,
) -> impl IntoResponse {
    let verified_path = match light_show_path(&state, &name) {
        Ok(path) => path,
        Err(e) => return e,
    };

    // Held from reading the song's tempo: a song save changing it reloads
    // the songs under this lock.
    let _writing = super::songs_api::SONG_WRITES.lock().await;
    // Validate against the tempo the owning song loads this file with. A show
    // inheriting its song's tempo is valid content that a tempo-less parse
    // rejects, so writing it through the UI would fail for no real reason.
    let tempo = song_tempo_for_path(&state, &verified_path);
    if let Err(errors) = config_io::validate_light_show(
        &body,
        Some(&verified_path),
        Some(&state.songs_path()),
        tempo.as_ref(),
    ) {
        return (StatusCode::BAD_REQUEST, Json(json!({"errors": errors}))).into_response();
    }

    let vp = verified_path;
    let body_owned = body;
    match super::helpers::spawn_blocking_io("write lighting file", move || {
        config_io::staged_write(&vp, &body_owned)
    })
    .await
    {
        Ok(()) => (StatusCode::OK, Json(json!({"status": "saved"}))).into_response(),
        Err(e) => e,
    }
}

/// DELETE /api/lighting/:name — deletes a .light file from disk.
pub(super) async fn delete_lighting_file(
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
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Lighting file not found: {}", name)})),
            )
                .into_response();
        }
    };

    if crate::songs::FileKind::of(safe.as_path()) != crate::songs::FileKind::Lighting {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Can only delete .light files"})),
        )
            .into_response();
    }

    // codeql[rust/path-injection] safe is verified via SafePath::resolve against songs_path root.
    let path = safe.as_path().to_path_buf();
    // In turn with the file's writes: an MCP patch reading it before the
    // delete would write it back after.
    let _writing = super::songs_api::SONG_WRITES.lock().await;
    match super::helpers::spawn_blocking_io("delete lighting file", move || {
        std::fs::remove_file(&path)
    })
    .await
    {
        Ok(()) => (StatusCode::OK, Json(json!({"status": "deleted"}))).into_response(),
        Err(e) => e,
    }
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
        Some(file) => match light_show_path(&state, file) {
            Ok(path) => Some(path),
            Err(e) => return e,
        },
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

/// The project's default lighting directories, the engine's own
/// (`config::lighting`): a request's `?dir=` is the running profile's
/// configured directory, and without one the engine reads these.
pub(super) use crate::config::lighting::{DEFAULT_FIXTURE_TYPES_DIR, DEFAULT_VENUES_DIR};

/// Fixture type files: `.fixture` (rich channels, GDTF records) and the v1
/// `.light`, loaded as peers — the same pair the lighting system reads.
const FIXTURE_TYPE_EXTENSIONS: &[&str] = lighting::project_files::FIXTURE_TYPE_EXTENSIONS;

use lighting::project_files::{fixture_types_declared, venues_declared, Declares};

/// The fixture type names a file declares; none if it cannot be read or
/// does not parse.
fn declared_fixture_types(path: &std::path::Path) -> Vec<String> {
    fixture_types_declared(path).unwrap_or_default()
}

/// What a refused save or delete would have done to the file.
#[derive(Clone, Copy)]
enum Acting {
    Saving,
    Deleting,
}

/// The files a request may replace although they do not read: each
/// `replace_broken` query parameter (repeated, one per file), as written,
/// each a path in the lighting directory as a refusal named it.
fn replace_broken_files(raw_query: &Option<String>) -> Vec<String> {
    form_urlencoded::parse(raw_query.as_deref().unwrap_or("").as_bytes())
        .filter(|(key, _)| key == "replace_broken")
        .map(|(_, file)| file.into_owned())
        .collect()
}

/// The one rule for the venue or fixture type files a save or delete of
/// `name` would act on (`project_files::may_act_on`, each named by its path
/// in `dir`), as one 409: only files that declare `name`, or that do not
/// read and that the person chose to replace (`?replace_broken=`, naming
/// exactly each). A refusal of a file that reads names it alone, as nothing
/// replaces it; else every broken file is named, so one confirmation covers
/// them all. Nothing is written or removed when it refuses.
#[allow(clippy::result_large_err)]
fn require_declares(
    kind: &str,
    name: &str,
    dir: &std::path::Path,
    files: &[(&std::path::Path, &Declares)],
    replace_broken: &[String],
    acting: Acting,
) -> Result<(), axum::response::Response> {
    use crate::webui::wire::fixture_types::{BrokenLightingFile, LightingFileRefusal};
    let refused: Vec<lighting::project_files::Refusal> = files
        .iter()
        .filter_map(|(file, declares)| {
            let shown = lighting::project_files::display_in(dir, file);
            let keyword = if kind == "venue" {
                "venue"
            } else {
                "fixture_type"
            };
            lighting::project_files::may_act_on(name, keyword, &shown, declares, replace_broken)
                .err()
        })
        .collect();
    let Some(first) = refused.first() else {
        return Ok(());
    };
    let fate = match acting {
        Acting::Saving => "would be replaced",
        Acting::Deleting => "would be removed",
    };
    let foreign = refused.iter().find(|r| r.parse_error.is_none());
    let shown: Vec<&lighting::project_files::Refusal> = match foreign {
        Some(refusal) => vec![refusal],
        None => refused.iter().collect(),
    };
    let lead = foreign.unwrap_or(first);
    let reasons: Vec<String> = shown
        .iter()
        .map(|r| format!("{} does not define {kind} \"{name}\" ({})", r.file, r.why))
        .collect();
    Err((
        StatusCode::CONFLICT,
        Json(LightingFileRefusal {
            error: format!(
                "{}, and {fate}; fix or move the file by hand first",
                reasons.join("; ")
            ),
            file: lead.file.clone(),
            broken: foreign.is_none(),
            parse_error: lead.parse_error.clone(),
            broken_files: match foreign {
                Some(_) => Vec::new(),
                None => refused
                    .iter()
                    .map(|r| BrokenLightingFile {
                        file: r.file.clone(),
                        parse_error: r.parse_error.clone().unwrap_or_default(),
                        also_mentions: r.also_mentions.clone(),
                    })
                    .collect(),
            },
        }),
    )
        .into_response())
}

/// [`require_declares`] off the async runtime: the files are read and
/// parsed on a blocking thread.
async fn require_declares_blocking(
    kind: &'static str,
    name: &str,
    dir: &std::path::Path,
    files: Vec<std::path::PathBuf>,
    declared: fn(&std::path::Path) -> Declares,
    replace_broken: &[String],
    acting: Acting,
) -> Result<Vec<std::path::PathBuf>, axum::response::Response> {
    let read = files.clone();
    let declares = super::helpers::spawn_blocking_io("read a lighting file", move || {
        Ok::<_, String>(read.iter().map(|path| declared(path)).collect::<Vec<_>>())
    })
    .await?;
    let pairs: Vec<(&std::path::Path, &Declares)> = files
        .iter()
        .map(std::path::PathBuf::as_path)
        .zip(declares.iter())
        .collect();
    require_declares(kind, name, dir, &pairs, replace_broken, acting)?;
    Ok(broken_among(&pairs))
}

/// The files among `files` that are broken (`project_files::is_broken`):
/// once the one rule let a request act on them, the broken files a person
/// chose to replace or remove, which [`set_aside_broken`] keeps.
fn broken_among(files: &[(&std::path::Path, &Declares)]) -> Vec<std::path::PathBuf> {
    files
        .iter()
        .filter(|(_, declares)| lighting::project_files::is_broken(declares))
        .map(|(path, _)| path.to_path_buf())
        .collect()
}

use lighting::project_files::{put_back, set_aside_all};

/// The file a fixture type of this name lives in, or would be written to,
/// by the one locate rule the GDTF import takes too
/// (`project_files::locate_fixture_type`): the file at any depth that
/// declares it, as the lighting system loads it (its stem need not match the
/// name: an import may have written `astera_pixelbrick.fixture` for
/// "Astera-PixelBrick"); else the broken files that may be its (the one at
/// its stem, and any that may hold it, `project_files::may_hold`). One that
/// does not declare it is still the answer, so the caller can say what is
/// wrong with it, except a broken stem file that may hold neither the type
/// nor its archive when the project's GDTF library has a fixture of that
/// name (`root`): the library answers for it, and the broken file is left
/// alone.
pub(super) fn existing_fixture_type(
    root: &std::path::Path,
    dir: &std::path::Path,
    name: &str,
) -> Option<lighting::project_files::Located> {
    let found = lighting::project_files::locate_fixture_type(dir, name)
        .ok()
        .flatten()?;
    // Only a broken stem file that may hold neither the type nor its
    // archive is unrelated; in any doubt, it is the type's record, offered
    // for replacing, and the library does not answer.
    if lighting::project_files::is_broken(&found.declares) && found.other_broken.is_empty() {
        if let Some(library) = lighting::library::unrecorded_types(root, Some(dir)).get(name) {
            if !lighting::project_files::file_may_hold(
                &found.path,
                Some(("fixture_type", name)),
                Some(&library.archive),
            ) {
                return None;
            }
        }
    }
    Some(found)
}

/// The path of [`existing_fixture_type`].
pub(super) fn existing_fixture_type_file(
    root: &std::path::Path,
    dir: &std::path::Path,
    name: &str,
) -> Option<std::path::PathBuf> {
    existing_fixture_type(root, dir, name).map(|found| found.path)
}

/// A fixture type's file as a save or delete finds it
/// ([`locate_fixture_type_file`]).
pub(super) struct FoundTypeFile {
    pub path: std::path::PathBuf,
    /// The other fixture types the file declares.
    pub others: Vec<String>,
    /// When the file does not read, the other broken files that may be the
    /// type's, which the request acts on too.
    pub also_broken: Vec<std::path::PathBuf>,
}

/// [`existing_fixture_type`], off the async runtime, with the other fixture
/// types the file declares and the type's other broken candidates. Saving
/// or deleting one type rewrites or removes its whole file, so a caller
/// that cannot carry the others must refuse.
pub(super) async fn locate_fixture_type_file(
    root: &std::path::Path,
    dir: &std::path::Path,
    name: &str,
) -> Result<Option<FoundTypeFile>, axum::response::Response> {
    let (root, dir, name) = (root.to_path_buf(), dir.to_path_buf(), name.to_string());
    super::helpers::spawn_blocking_io("locate fixture type file", move || {
        Ok::<_, String>(existing_fixture_type(&root, &dir, &name).map(|found| {
            let mut others = found.declares.clone().unwrap_or_default();
            others.retain(|n| n != &name);
            others.sort();
            FoundTypeFile {
                path: found.path,
                others,
                also_broken: found.other_broken.into_iter().map(|(p, _)| p).collect(),
            }
        }))
    })
    .await
}

/// The 409 for a save or delete that would take a file's other fixture
/// types with it.
fn shared_fixture_type_file_response(
    name: &str,
    path: &std::path::Path,
    others: &[String],
    action: &str,
) -> axum::response::Response {
    shared_file_response("fixture type", name, path, others, action)
}

/// The names a file a save retires (its twin in another form) declares and
/// the saved text does not, sorted: retiring it would lose them, so the save
/// is refused. One rule for fixture types and venues.
fn lost_by_retiring(declared: Vec<String>, saved: impl Fn(&str) -> bool) -> Vec<String> {
    let mut lost: Vec<String> = declared.into_iter().filter(|n| !saved(n)).collect();
    lost.sort();
    lost
}

/// The 409 for a save or delete that would take a file's other venues or
/// fixture types (`keyword`) with it.
fn shared_file_response(
    keyword: &str,
    name: &str,
    path: &std::path::Path,
    others: &[String],
    action: &str,
) -> axum::response::Response {
    (
        StatusCode::CONFLICT,
        Json(json!({"error": format!(
            "{} \"{}\" lives in {}, which also defines {}; {} would remove them — \
             edit the file as text instead",
            keyword,
            name,
            crate::util::filename_display(path),
            others
                .iter()
                .map(|n| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(", "),
            action
        )})),
    )
        .into_response()
}

/// Venue files: `.venue` (positions, focus points, MVR provenance) and the
/// v1 `.light`, loaded as peers.
pub(super) const VENUE_EXTENSIONS: &[&str] = lighting::project_files::VENUE_EXTENSIONS;

/// Where a save writes a venue or fixture type file: `file`, under `dir`,
/// held inside it by the one containment rule MCP's tools take
/// (`SafePath::validate_relative_entry`), and refused when it is a link to
/// nothing, as writing through it would create whatever it names.
#[allow(clippy::result_large_err)]
fn lighting_write_path(
    dir: &std::path::Path,
    file: &std::path::Path,
) -> Result<std::path::PathBuf, axum::response::Response> {
    use super::super::safe_path::{SafePath, VerifiedRoot};
    let refuse = |e: String| (StatusCode::BAD_REQUEST, Json(json!({"error": e}))).into_response();
    let relative = file
        .strip_prefix(dir)
        .map_err(|_| refuse(format!("{} is outside {}", file.display(), dir.display())))?
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/");
    // A file is named for what it holds; a name starting with a dot is a
    // hidden file the loaders pass over, so the save would vanish.
    if crate::songs::has_hidden_part(std::path::Path::new(&relative)) {
        return Err(refuse(format!(
            "{relative} is a hidden file name; the lighting system would not read it"
        )));
    }
    let root = VerifiedRoot::new(dir).map_err(|e| e.into_response())?;
    let entry =
        SafePath::validate_relative_entry(&relative, &root).map_err(|e| e.into_response())?;
    if entry.is_dangling_link() {
        return Err(refuse(format!(
            "{} is a link to a file that is not there",
            crate::util::filename_display(file)
        )));
    }
    Ok(entry.into_path())
}

/// Serialises every read-check-write of a venue file, so two saves cannot
/// interleave between the version check and the write; and every write of a
/// fixture type file, which a fixture's settings save rewrites with its
/// venues. MCP's venue and fixture type writes take it too, and its edits
/// hold it from their read to their write, or a save made between the two
/// would be lost (#578). The saves here run on a blocking thread, and take
/// it with `blocking_lock`.
pub(crate) static VENUE_WRITES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// The version of a venue file: a hash of its bytes. A client sends back the
/// one it read (`If-Match`); a save whose version is no longer the file's is
/// refused, the same optimistic concurrency the config store has for inline
/// profiles.
pub(super) fn content_version(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

/// A venue file found on disk, whatever it is called.
pub(super) struct LocatedVenue {
    pub path: std::path::PathBuf,
    pub bytes: Vec<u8>,
    /// Why the file cannot be patched, when it cannot (it does not parse, is
    /// not UTF-8, or does not define the venue after all).
    pub broken: Option<String>,
    /// When the file does not read, the other broken files that may be the
    /// venue's, which a save or delete acts on too.
    pub also_broken: Vec<std::path::PathBuf>,
}

impl LocatedVenue {
    pub fn version(&self) -> String {
        content_version(&self.bytes)
    }

    pub fn file(&self) -> String {
        crate::util::filename_display(&self.path).to_string()
    }

    /// The file's text, when it parses and defines the venue.
    pub fn text(&self) -> Option<&str> {
        if self.broken.is_some() {
            return None;
        }
        std::str::from_utf8(&self.bytes).ok()
    }
}

/// The venue file `name` lives in, or would be written to, by the one
/// locate rule the MVR import takes too (`project_files::locate_venue`): the
/// file at any depth that declares it; else the file at the name's stem;
/// else a broken file that opens a block for it. One that does not hold the
/// venue comes back with `broken` set, so the caller can refuse rather than
/// replace it. `None` means a genuinely new venue.
pub(super) fn locate_venue_file(
    dir: &std::path::Path,
    name: &str,
) -> Result<Option<LocatedVenue>, String> {
    if !dir.is_dir() {
        return Ok(None);
    }
    let Some(found) = lighting::project_files::locate_venue(dir, name)
        .map_err(|e| format!("cannot read {}: {e}", dir.display()))?
    else {
        return Ok(None);
    };
    let bytes = crate::util::read_file(&found.path)
        .map_err(|e| format!("cannot read {}: {e}", found.path.display()))?;
    let broken = match &found.declares {
        Ok(names) if names.iter().any(|n| n == name) => None,
        Ok(_) => Some(format!("the file does not define a venue named \"{name}\"")),
        Err(unread) => Some(unread.error.clone()),
    };
    Ok(Some(LocatedVenue {
        path: found.path,
        bytes,
        broken,
        also_broken: found.other_broken.into_iter().map(|(p, _)| p).collect(),
    }))
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
pub(super) fn stale_venue_response(
    name: &str,
    current: Option<String>,
) -> axum::response::Response {
    (
        StatusCode::CONFLICT,
        Json(json!({
            "error": format!(
                "venue \"{name}\" changed since you loaded it (another tab, page or an edit by \
                 hand); it has been reloaded, so reapply your change"
            ),
            "conflict": true,
            "version": current,
        })),
    )
        .into_response()
}

/// Whether a venue uses syntax only `.venue` files carry: provenance, focus
/// points, a fixture's position, rotation or beam angle, or its own GDTF mode
/// (§21).
fn needs_venue_extension(venue: &lighting::types::Venue) -> bool {
    venue.source().is_some()
        || !venue.focus_points().is_empty()
        || venue.fixtures().values().any(|f| {
            f.position().is_some()
                || f.rotation().is_some()
                || f.mode().is_some()
                || f.beam_angle().is_some()
        })
}

/// Resolves a lighting directory path relative to the project root.
/// Uses the provided override (from query param) or falls back to the default.
/// Returns an error response if the project root cannot be canonicalized or the
/// resolved path would escape it.
#[allow(clippy::result_large_err)]
pub(super) fn resolve_lighting_dir(
    config_path: &std::path::Path,
    override_dir: Option<&str>,
    default: &str,
) -> Result<std::path::PathBuf, axum::response::Response> {
    use super::super::safe_path::{SafePath, VerifiedRoot};

    let root = VerifiedRoot::new(&project_root(config_path)?).map_err(|e| e.into_response())?;

    let relative = crate::config::lighting::resolve_dir(override_dir, default);

    SafePath::validate_relative(relative, &root).map_err(|e| e.into_response())
}

/// Checks a referential type's archive the way the lighting system will when
/// it expands one: the path is project-relative, stays inside the project, and
/// is a file that is actually there. Nothing is distilled — that happens at
/// load, through the cache — but a `.fixture` whose archive is missing or
/// outside the project cannot load, and a save is the last moment where the
/// person still has the text in front of them to fix it.
#[allow(clippy::result_large_err)]
fn validate_referential_archives(
    root: &std::path::Path,
    types: &std::collections::HashMap<String, lighting::types::FixtureType>,
) -> Result<(), axum::response::Response> {
    let canonical_root = canonical_project_root(root)?;
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
#[allow(clippy::result_large_err)]
fn resolve_referential_archive(
    canonical_root: &std::path::Path,
    name: &str,
    source: &lighting::types::GdtfSource,
) -> Result<std::path::PathBuf, axum::response::Response> {
    let archive = canonical_root.join(&source.path);
    let canonical = archive.canonicalize().map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!(
                "fixture type \"{}\" references a GDTF archive that cannot be read: {}: {}",
                name, source.path, e
            )})),
        )
            .into_response()
    })?;
    if !canonical.starts_with(canonical_root) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!(
                "fixture type \"{}\" references a GDTF archive outside the project: {}",
                name, source.path
            )})),
        )
            .into_response());
    }
    if !canonical.is_file() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!(
                "fixture type \"{}\" references a GDTF archive that is not a file: {}",
                name, source.path
            )})),
        )
            .into_response());
    }
    Ok(canonical)
}

/// The name the URL asks for must be the name the DSL declares: the webui
/// resolves a type or venue by its file stem, so a body that renames the
/// declaration would write `oldname.fixture` holding `newname` — a file no
/// later GET, PUT or DELETE can reach.
#[allow(clippy::result_large_err)]
fn require_declared_name<T>(
    kind: &str,
    name: &str,
    parsed: &std::collections::HashMap<String, T>,
) -> Result<(), axum::response::Response> {
    if parsed.contains_key(name) {
        return Ok(());
    }
    let mut declared: Vec<&str> = parsed.keys().map(String::as_str).collect();
    declared.sort_unstable();
    let found = if declared.is_empty() {
        "it declares none".to_string()
    } else {
        format!("it declares \"{}\"", declared.join("\", \""))
    };
    Err((
        StatusCode::BAD_REQUEST,
        Json(json!({"error": format!(
            "the DSL declares no {} named \"{}\" ({}); rename the {} in the Name field, not in \
             the text",
            kind, name, found, kind
        )})),
    )
        .into_response())
}

/// Query parameters for lighting endpoints — allows overriding the directory.
#[derive(serde::Deserialize, Default)]
pub(super) struct LightingDirQuery {
    dir: Option<String>,
    /// The extension a *new* fixture type is written with (`light` or
    /// `fixture`). An existing file keeps its own, so this only decides
    /// which form a type is born in.
    ext: Option<String>,
    /// The venues directory, for the fixture-type endpoints that say which
    /// venue fixtures use a type (default `lighting/venues`).
    venues_dir: Option<String>,
    /// The fixture types directory, for the venue endpoints that need the
    /// types (default `lighting/fixture_types`).
    fixture_types_dir: Option<String>,
}

/// Validates that a fixture type or venue name is safe for use as a filename.
#[allow(clippy::result_large_err)]
pub(super) fn validate_lighting_name(name: &str) -> Result<(), axum::response::Response> {
    validate_lighting_name_to_write(name, true)
}

/// As [`validate_lighting_name`], for a fixture type or venue being written:
/// one not already on disk (`exists` false) is being created, and its name
/// must also pass the rules for new names.
#[allow(clippy::result_large_err)]
pub(super) fn validate_lighting_name_to_write(
    name: &str,
    exists: bool,
) -> Result<(), axum::response::Response> {
    use super::super::safe_path::SafePath;
    if SafePath::validate_name_to_write(name, exists).is_err() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Invalid name"})),
        )
            .into_response());
    }
    if RESERVED_NAMES.contains(&name) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!(
                "`{name}` is reserved: it is the name of an API route, so a file called that could not be addressed"
            )})),
        )
            .into_response());
    }
    Ok(())
}

/// Path segments the lighting API uses as routes beside `{name}`.
const RESERVED_NAMES: &[&str] = &["lock", "validate", "activate"];

/// The file types the asset store serves, by extension. Everything in the
/// store was written by mtrack itself from a GDTF archive; the allowlist
/// keeps the endpoint from ever becoming a general file server.
const ASSET_TYPES: &[(&str, &str)] = &[
    ("json", "application/json"),
    ("glb", "model/gltf-binary"),
    ("png", "image/png"),
    ("svg", "image/svg+xml"),
];

/// The largest asset served; the store's own caps are lower.
const MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024;

/// GET /api/lighting/assets/{*path} — serves a file of the project's
/// asset store (`lighting/.cache/assets/`, design §16.2): rig models,
/// meshes and thumbnails for the 3D view. Paths are content-addressed, so
/// a hit is immutable and cached as such.
pub(super) async fn get_lighting_asset(
    State(state): State<WebUiState>,
    Path(path): Path<String>,
) -> impl IntoResponse {
    use super::super::safe_path::{SafePath, VerifiedRoot};

    let content_type = crate::songs::extension_of(std::path::Path::new(&path))
        .and_then(|ext| ASSET_TYPES.iter().find(|(known, _)| *known == ext))
        .map(|(_, mime)| *mime);
    let Some(content_type) = content_type else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Not an asset type"})),
        )
            .into_response());
    };

    let store = project_root(&state.config_path)?
        .join("lighting")
        .join(".cache")
        .join(lighting::distill::ASSETS_DIR);
    // No store yet (no referential fixture type has expanded) is a plain
    // 404, not a server error.
    let root = VerifiedRoot::new(&store).map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "No asset store"})),
        )
            .into_response()
    })?;
    let file = SafePath::validate_relative(&path, &root).map_err(|e| e.into_response())?;

    let (bytes, len) = super::helpers::spawn_blocking_io("read asset", move || {
        let meta = std::fs::metadata(&file)?;
        if !meta.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "not a file",
            ));
        }
        if meta.len() > MAX_ASSET_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "asset over the size cap",
            ));
        }
        Ok((std::fs::read(&file)?, meta.len()))
    })
    .await
    .map_err(|_| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "Asset not found"})),
        )
            .into_response()
    })?;

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
        DEFAULT_FIXTURE_TYPES_DIR,
    )
    .map_err(|e| e.into_response())?;
    let venues_dir = resolve_lighting_dir(
        &state.config_path,
        query.venues_dir.as_deref(),
        DEFAULT_VENUES_DIR,
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
                    strobe_curve: fixture_type.strobe_curve(),
                    fixture_type: fixture_type.clone(),
                    file: Some(shown(&declared.file)),
                    // The file's kind, as the editor and the loader take it
                    // (`Par.FIXTURE` is a `fixture`).
                    extension: Some(crate::songs::extension_of(&declared.file).unwrap_or_default()),
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
                    strobe_curve: declared.strobe_curve(),
                    fixture_type: declared,
                    file: None,
                    extension: None,
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
        DEFAULT_FIXTURE_TYPES_DIR,
    )?;
    let root = canonical_project_root(&project_root(&state.config_path)?)?;
    let Some(FoundTypeFile {
        path: file_path, ..
    }) = locate_fixture_type_file(&root, &dir, &name).await?
    else {
        // A GDTF in the library with no record: a fixture with no file.
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
                    strobe_curve: declared.strobe_curve(),
                    fixture_type: declared,
                    file: None,
                    extension: None,
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
    let fp = file_path.clone();
    let content = super::helpers::spawn_blocking_io("read fixture type", move || {
        crate::util::read_to_string(&fp)
    })
    .await?;
    let types = lighting::parser::parse_fixture_types(&content).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to parse fixture type: {}", e)})),
        )
            .into_response()
    })?;
    match types.get(&name) {
        Some(ft) => Ok((
            StatusCode::OK,
            Json(FixtureTypeEntry {
                referential: ft.source().is_some(),
                footprint: None,
                rich: ft.uses_rich_channels(),
                strobe_curve: ft.strobe_curve(),
                fixture_type: ft.clone(),
                file: Some(crate::util::filename_display(&file_path).to_string()),
                extension: Some(crate::songs::extension_of(&file_path).unwrap_or_default()),
                gdtf: None,
                dsl: Some(Some(content)),
            }),
        )
            .into_response()),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Fixture type '{}' not found in file", name)})),
        )
            .into_response()),
    }
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
    if let Some(FoundTypeFile {
        path: file_path, ..
    }) = locate_fixture_type_file(root, dir, name).await?
    {
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
        DEFAULT_FIXTURE_TYPES_DIR,
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
        DEFAULT_VENUES_DIR,
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

/// PUT /api/lighting/fixture-types/:name — creates or updates a fixture type.
///
/// Accepts either JSON (structured) or plain text (raw DSL).
pub(super) async fn put_fixture_type(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
    RawQuery(raw_query): RawQuery,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        DEFAULT_FIXTURE_TYPES_DIR,
    )?;

    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let from_form = content_type.contains("application/json");

    // Whatever form the body is in, an `ext` naming no known form is a
    // mistake worth reporting rather than quietly writing a `.light`.
    let requested_extension = match query.ext.as_deref() {
        None | Some("") => None,
        Some(known @ ("light" | "fixture")) => Some(known),
        Some(other) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Unknown fixture type extension: {}", other)})),
            )
                .into_response())
        }
    };

    let dsl = if from_form {
        // Parse JSON body and convert to DSL
        let json_body: serde_json::Value = serde_json::from_slice(&body).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid JSON: {}", e)})),
            )
                .into_response()
        })?;
        fixture_type_json_to_dsl(&name, &json_body)
            .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e}))).into_response())?
    } else {
        // Treat as raw DSL text
        String::from_utf8(body.to_vec()).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Invalid UTF-8"})),
            )
                .into_response()
        })?
    };

    // Validate the DSL parses correctly
    let types = lighting::parser::parse_fixture_types(&dsl).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Invalid fixture type DSL: {}", e)})),
        )
            .into_response()
    })?;
    require_declared_name("fixture type", &name, &types)?;
    validate_referential_archives(&project_root(&state.config_path)?, &types)?;

    // Held from finding the file to writing it, as MCP's patch holds it: a
    // type added to a shared file between the two would pass the check on
    // the file's other types below, and be lost to the form's save.
    let _writing = VENUE_WRITES.lock().await;
    // Read against the resolved directory rather than the created one: every
    // refusal below should land before anything is made on disk.
    let types_root = canonical_project_root(&project_root(&state.config_path)?)?;
    let found = locate_fixture_type_file(&types_root, &dir, &name).await?;
    // The type's other broken candidates, which the save acts on too.
    let also_broken = found
        .as_ref()
        .map(|f| f.also_broken.clone())
        .unwrap_or_default();
    let located = found.map(|f| (f.path, f.others));
    validate_lighting_name_to_write(&name, located.is_some())?;
    let replace_broken = replace_broken_files(&raw_query);
    // An existing type is saved back to the file it lives in, whatever that
    // file is called; only a new type is named after itself, past a file at
    // its stem that holds other types (`project_files::new_file_stem`).
    let stem = match located
        .as_ref()
        .and_then(|(path, _)| path.file_stem().and_then(|s| s.to_str()))
    {
        Some(stem) => stem.to_string(),
        None => {
            lighting::project_files::new_file_stem(&dir, &name, FIXTURE_TYPE_EXTENSIONS, &|_| false)
        }
    };
    let existing_extension = located
        .as_ref()
        .and_then(|(path, _)| crate::songs::extension_of(path));

    // The channel-map form writes one type, so it cannot carry the others a
    // shared file declares. Raw text is the whole file and says what it means.
    if from_form {
        if let Some((path, others)) = located.as_ref().filter(|(_, others)| !others.is_empty()) {
            return Err(shared_fixture_type_file_response(
                &name,
                path,
                others,
                "saving this one from the channel-map editor",
            ));
        }
    }

    // The channel-map form cannot say what a `.fixture` file holds, so
    // saving one through it would silently drop the type's rich channels or
    // its GDTF reference.
    if from_form && existing_extension.as_deref() == Some("fixture") {
        return Err((
            StatusCode::CONFLICT,
            Json(json!({"error": format!(
                "fixture type \"{}\" lives in {}.fixture, whose form the channel-map editor \
                 cannot express — edit this type as text",
                name, stem
            )})),
        )
            .into_response());
    }

    // An explicit `ext` wins — that is how a `.light` type converts to a
    // `.fixture`. Without one, an existing file keeps its own extension and
    // a new type is born a `.light`.
    let extension = match (requested_extension, existing_extension.as_deref()) {
        (Some(requested), _) => requested,
        (None, Some(existing)) => match existing {
            "fixture" => "fixture",
            _ => "light",
        },
        (None, None) => "light",
    };

    // The extension is the version marker: rich channel syntax written into
    // a `.light` file would be saved, then skipped by the loader, and one
    // missing type fails the whole venue's registration.
    if extension == "light" {
        if let Some(rich) = types.values().find(|t| t.uses_rich_channels()) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!(
                    "fixture type \"{}\" uses rich channel syntax (fine, range, functions), which \
                     belongs in a .fixture file; this editor writes .light files — save it as \
                     {}.fixture in the fixture types directory by hand, or through \
                     `mtrack import-gdtf`",
                    rich.name(),
                    stem
                )})),
            )
                .into_response());
        }
    }

    // Everything that can refuse the save has: the same helper the playlists
    // and profiles writes use, so a refusal is reported as the configuration
    // problem it is rather than a server fault. The file goes under the
    // directory that was made, not the spelling.
    let resolved_dir = dir.clone();
    let dir = super::helpers::ensure_configured_dir(&dir, &state).await?;

    // An existing type stays in its own folder, at any depth; a new one is
    // born at the top. A type is one file: whichever form it is saved in, the
    // other one is retired, or the loader would register the name twice.
    let folder = located
        .as_ref()
        .and_then(|(path, _)| path.parent())
        .map_or_else(|| dir.clone(), std::path::Path::to_path_buf);
    // The files at the stem, by their kind (`songs::extension_of`), never by
    // their spelling: on a filesystem that ignores case, `Par.fixture` and
    // `Par.FIXTURE` are one file.
    let at_stem = crate::util::files_at_stem(&folder, &stem, FIXTURE_TYPE_EXTENSIONS);
    let is_form =
        |path: &std::path::Path| crate::songs::extension_of(path).as_deref() == Some(extension);
    // The file of this form already at the stem, if any, is written in
    // place, in its own spelling (the type's own file first); a new form is
    // spelled lowercase. One holding other types is not this type's to
    // overwrite.
    let in_form = located
        .as_ref()
        .map(|(path, _)| path.clone())
        .filter(|path| is_form(path))
        .or_else(|| at_stem.iter().find(|path| is_form(path)).cloned());
    // Only the type's own file is its to rewrite whole; a file at its stem
    // that does not declare it (`existing_fixture_type_file`'s fallback)
    // holds someone else's types.
    let own_file = located
        .as_ref()
        .map(|(path, _)| path)
        .filter(|path| declared_fixture_types(path).contains(&name));
    if let Some(path) = &in_form {
        let mut others = declared_fixture_types(path);
        others.retain(|n| n != &name);
        if !others.is_empty() && own_file != Some(path) {
            others.sort();
            return Err(shared_fixture_type_file_response(
                &name,
                path,
                &others,
                "saving this type in that form",
            ));
        }
    }
    let file_path = in_form.unwrap_or_else(|| folder.join(format!("{stem}.{extension}")));
    // The file found or made under the directory as it was read, or as it
    // was made: held inside it either way.
    let file_path = match file_path.strip_prefix(&resolved_dir) {
        Ok(relative) => lighting_write_path(&dir, &dir.join(relative))?,
        Err(_) => lighting_write_path(&dir, &file_path)?,
    };
    // The files the save acts on must hold the type, whatever form it is
    // saved in, as a venue's must: the file found for the name, and the one
    // written when it is another. One that does not parse, or holds only
    // other types, is not replaced, unless the person chose to replace it;
    // every broken one is named at once, so one confirmation covers them.
    // Each is named by its path in the directory as it was read.
    let located_path = located.as_ref().map(|(path, _)| path.clone());
    let target_as_read = file_path
        .strip_prefix(&dir)
        .map(|relative| resolved_dir.join(relative))
        .unwrap_or_else(|_| file_path.clone());
    let mut acted_on: Vec<std::path::PathBuf> = located_path.iter().cloned().collect();
    acted_on.extend(also_broken);
    if file_path.exists() && !acted_on.contains(&target_as_read) {
        acted_on.push(target_as_read);
    }
    let broken = require_declares_blocking(
        "fixture type",
        &name,
        &resolved_dir,
        acted_on,
        fixture_types_declared,
        &replace_broken,
        Acting::Saving,
    )
    .await?;
    // The other forms of this type are retired, or the loader would register
    // the name twice: each file at the stem in another form that declares
    // this type. One that also declares a type the saved text does not is
    // not retired, which would lose that type: the save is refused.
    let mut stale_twins = Vec::new();
    for path in at_stem.into_iter().filter(|path| !is_form(path)) {
        let declared = declared_fixture_types(&path);
        if !declared.contains(&name) {
            continue;
        }
        let lost = lost_by_retiring(declared, |n| types.contains_key(n));
        if !lost.is_empty() {
            return Err(shared_fixture_type_file_response(
                &name,
                &path,
                &lost,
                "saving this type in another form",
            ));
        }
        stale_twins.push(path);
    }
    // A broken file the person chose to replace is set aside, kept, before
    // the save writes (`set_aside_broken`), whether the save writes in its
    // place or, in another form, beside it; so is each twin the save
    // retires. (A twin declares the type, so reads, and is never broken.)
    // A failed write puts them all back.
    let fp = file_path;
    let dsl_owned = dsl;
    let kept_root = resolved_dir.clone();
    let mut retired = broken;
    retired.extend(stale_twins);
    let kept = super::helpers::spawn_blocking_io("write fixture type", move || {
        let set_aside = set_aside_all(&retired)?;
        if let Err(e) = config_io::staged_write(&fp, &dsl_owned) {
            return Err(format!("{e}{}", put_back(&set_aside)));
        }
        Ok::<_, String>(
            set_aside
                .iter()
                .map(|(_, backup)| lighting::project_files::display_in(&kept_root, backup))
                .collect::<Vec<String>>(),
        )
    })
    .await?;

    // The reply names any broken file kept aside, for the editor to say so.
    let mut reply = json!({"status": "saved", "name": name});
    if !kept.is_empty() {
        reply["kept"] = json!(kept);
    }
    Ok::<_, axum::response::Response>((StatusCode::OK, Json(reply)).into_response())
}

/// DELETE /api/lighting/fixture-types/:name — deletes a fixture type file.
pub(super) async fn delete_fixture_type(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
    RawQuery(raw_query): RawQuery,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        DEFAULT_FIXTURE_TYPES_DIR,
    )?;
    let root = canonical_project_root(&project_root(&state.config_path)?)?;
    // Held from finding the record to deleting it, as MCP's delete does: a
    // save between the two would be deleted unseen (#578).
    let _writing = VENUE_WRITES.lock().await;
    let Some(FoundTypeFile {
        path: file_path,
        others,
        also_broken,
    }) = locate_fixture_type_file(&root, &dir, &name).await?
    else {
        // A GDTF in the library with no record is a fixture of its own;
        // deleting it is deleting the archive. A file that does not read and
        // may be its record (`project_files::may_hold`) goes first, set aside
        // as any broken file is, once the person chose to remove it: left,
        // it would name an archive no longer there.
        let (types_dir, wanted, project) = (dir.clone(), name.clone(), root.clone());
        let found = super::helpers::spawn_blocking_io("read the GDTF library", move || {
            let library = lighting::library::unrecorded_types(&project, Some(&types_dir));
            Ok::<_, String>(library.get(&wanted).map(|found| {
                let holding: Vec<(std::path::PathBuf, Declares)> =
                    lighting::project_files::broken_type_files_holding(
                        &types_dir,
                        Some(&wanted),
                        Some(&found.archive),
                    )
                    .into_iter()
                    .map(|path| {
                        let declares = fixture_types_declared(&path);
                        (path, declares)
                    })
                    .collect();
                (found.archive.clone(), holding)
            }))
        })
        .await?;
        let Some((archive, holding)) = found else {
            return Err((
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Fixture type not found: {}", name)})),
            )
                .into_response());
        };
        let pairs: Vec<(&std::path::Path, &Declares)> =
            holding.iter().map(|(p, d)| (p.as_path(), d)).collect();
        require_declares(
            "fixture type",
            &name,
            &dir,
            &pairs,
            &replace_broken_files(&raw_query),
            Acting::Deleting,
        )?;
        let records: Vec<std::path::PathBuf> = holding.into_iter().map(|(p, _)| p).collect();
        let kept_root = dir.clone();
        let archive_path = root.join(&archive);
        let kept = super::helpers::spawn_blocking_io("delete a GDTF", move || {
            let set_aside = set_aside_all(&records)?;
            if let Err(e) = std::fs::remove_file(&archive_path) {
                return Err(format!("{e}{}", put_back(&set_aside)));
            }
            Ok::<_, String>(
                set_aside
                    .iter()
                    .map(|(_, backup)| lighting::project_files::display_in(&kept_root, backup))
                    .collect::<Vec<_>>(),
            )
        })
        .await?;
        return Ok((
            StatusCode::OK,
            Json(FixtureTypeDeleted {
                status: "deleted".to_string(),
                name,
                archive_removed: Some(archive),
                archive_kept: false,
                archive_kept_because: None,
                kept,
            }),
        )
            .into_response());
    };
    let broken = require_declares_blocking(
        "fixture type",
        &name,
        &dir,
        std::iter::once(file_path.clone())
            .chain(also_broken)
            .collect(),
        fixture_types_declared,
        &replace_broken_files(&raw_query),
        Acting::Deleting,
    )
    .await?;
    if !others.is_empty() {
        return Err(shared_fixture_type_file_response(
            &name,
            &file_path,
            &others,
            "deleting this one",
        ));
    }
    let fp = file_path;
    let deleted = name.clone();
    let kept_root = dir.clone();
    let (archive_removed, archive_kept, archive_kept_because, kept) =
        super::helpers::spawn_blocking_io("delete fixture type", move || {
            // The broken files the person chose to remove are set aside,
            // kept (`set_aside_broken`); they name no archive to free.
            if !broken.is_empty() {
                let kept = set_aside_all(&broken)
                    .map_err(std::io::Error::other)?
                    .iter()
                    .map(|(_, backup)| lighting::project_files::display_in(&kept_root, backup))
                    .collect();
                return Ok::<_, std::io::Error>((None, false, None, kept));
            }
            // A GDTF fixture's archive is mtrack's to keep, not the user's:
            // read which one this type uses before its record goes.
            let archive = crate::util::read_to_string(&fp)
                .ok()
                .and_then(|text| lighting::parser::parse_fixture_types(&text).ok())
                .and_then(|types| types.get(&deleted).and_then(|t| t.source().cloned()));
            std::fs::remove_file(&fp)?;
            let (removed, archive_kept, because) = match archive {
                Some(source) => remove_unused_archive(&root, &dir, &source.path),
                None => (None, false, None),
            };
            Ok((removed, archive_kept, because, Vec::new()))
        })
        .await?;
    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(FixtureTypeDeleted {
                status: "deleted".to_string(),
                name,
                archive_removed,
                archive_kept,
                archive_kept_because,
                kept,
            }),
        )
            .into_response(),
    )
}

/// After a GDTF fixture's record is deleted: its archive in
/// `lighting/library/`, removed when no other fixture type in `types_dir`
/// uses it — otherwise archives the user cannot see pile up — nor any file
/// that does not read may hold it. Returns the removed archive
/// (project-relative), whether it was kept, and why when that is a file
/// that does not read. An archive outside the library is never
/// touched, and neither is the expansion cache (it is keyed by content and
/// rebuilt on demand).
fn remove_unused_archive(
    root: &std::path::Path,
    types_dir: &std::path::Path,
    archive_rel: &str,
) -> (Option<String>, bool, Option<String>) {
    let library = root.join("lighting").join("library");
    let Ok(target) = root.join(archive_rel).canonicalize() else {
        return (None, false, None);
    };
    let in_library = library
        .canonicalize()
        .is_ok_and(|library| target.starts_with(library));
    if !in_library {
        return (None, false, None);
    }
    let still_used = lighting::import::existing_types(types_dir)
        .values()
        .flatten()
        .any(|other| {
            root.join(other)
                .canonicalize()
                .is_ok_and(|other| other == target)
        });
    if still_used {
        return (None, true, None);
    }
    // Nor while a file that does not read may be a record of it
    // (`project_files::may_hold`): it would name an archive no longer there.
    if let Some(record) =
        lighting::project_files::broken_type_files_holding(types_dir, None, Some(archive_rel))
            .first()
    {
        let record = lighting::project_files::display_in(types_dir, record);
        return (
            None,
            true,
            Some(format!(
                "{record} does not read and may be a record of {archive_rel}; the archive is kept"
            )),
        );
    }
    match std::fs::remove_file(&target) {
        Ok(()) => (Some(archive_rel.to_string()), false, None),
        Err(e) => {
            tracing::warn!(error = %e, "could not remove a deleted fixture's GDTF archive");
            (None, false, None)
        }
    }
}

/// The tempo a `.light` file at this path will be loaded with, found by
/// matching the path against the loaded songs' directories.
fn song_tempo_for_path(
    state: &WebUiState,
    path: &std::path::Path,
) -> Option<crate::tempo::TempoMap> {
    let song = state
        .player
        .songs()
        .list()
        .into_iter()
        .find(|song| path.starts_with(song.base_path()))?;
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
                    layer: effect.layer.keyword().to_string(),
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
            let report = crate::dmx::patch_check::probe_universes(dmx.ola_http_port(), &to_probe);
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
                    .dmx()
                    .and_then(|dmx| dmx.lighting())
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
    let dir = resolve_lighting_dir(&state.config_path, query.dir.as_deref(), DEFAULT_VENUES_DIR)
        .map_err(|e| e.into_response())?;
    if !dir.is_dir() {
        return Ok::<_, axum::response::Response>(
            (
                StatusCode::OK,
                Json(VenueList {
                    venues: Default::default(),
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
        for declared in read.items {
            if let Ok(bytes) = crate::util::read_file(&declared.file) {
                versions.insert(declared.name.clone(), content_version(&bytes));
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
        Ok::<_, String>((all, errors, versions))
    })
    .await?;
    let (all, errors, versions) = all;
    Ok((
        StatusCode::OK,
        Json(VenueList {
            venues: all,
            errors: Some(errors),
            versions: Some(versions),
        }),
    )
        .into_response())
}

/// GET /api/lighting/venues/:name — returns a single venue.
pub(super) async fn get_venue(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let dir = resolve_lighting_dir(&state.config_path, query.dir.as_deref(), DEFAULT_VENUES_DIR)?;
    let located = {
        let (dir, name) = (dir.clone(), name.clone());
        super::helpers::spawn_blocking_io("locate venue file", move || {
            locate_venue_file(&dir, &name)
        })
        .await?
    };
    let Some(located) = located else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Venue not found: {}", name)})),
        )
            .into_response());
    };
    if let Some(why) = &located.broken {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to parse venue: {} ({})", why, located.file())})),
        )
            .into_response());
    }
    let version = located.version();
    let content = String::from_utf8_lossy(&located.bytes).into_owned();
    let venues = lighting::parser::parse_venues(&content).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to parse venue: {}", e)})),
        )
            .into_response()
    })?;
    match venues.get(&name) {
        Some(v) => Ok((
            StatusCode::OK,
            Json(VenueDetail {
                venue: v.clone(),
                dsl: content,
                version,
            }),
        )
            .into_response()),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Venue '{}' not found in file", name)})),
        )
            .into_response()),
    }
}

/// PUT /api/lighting/venues/:name — creates or updates a venue.
///
/// The file is found by scanning every venue file for the block, so a venue
/// defined in a file whose stem differs from its name is patched in place. A
/// JSON body patches the file (never regenerates an existing one); a file
/// that does not parse is refused (409) rather than overwritten. An
/// `If-Match` version, when sent, must be the file's current one (409
/// otherwise); the response carries the new `version`.
pub(super) async fn put_venue(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
    RawQuery(raw_query): RawQuery,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let dir = resolve_lighting_dir(&state.config_path, query.dir.as_deref(), DEFAULT_VENUES_DIR)?;

    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let expected = if_match_version(&headers);

    enum Incoming {
        Json(serde_json::Value, lighting::types::Venue),
        Dsl(String),
    }
    let incoming = if content_type.contains("application/json") {
        let json_body: serde_json::Value = serde_json::from_slice(&body).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid JSON: {}", e)})),
            )
                .into_response()
        })?;
        let bad = |e: String| (StatusCode::BAD_REQUEST, Json(json!({"error": e}))).into_response();
        let venue = venue_from_json(&name, &json_body).map_err(bad)?;
        Incoming::Json(json_body, venue)
    } else {
        Incoming::Dsl(String::from_utf8(body.to_vec()).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Invalid UTF-8"})),
            )
                .into_response()
        })?)
    };

    let dir = super::helpers::ensure_configured_dir(&dir, &state).await?;
    let saved_in = dir.clone();
    let venue_name = name.clone();
    let replace_broken = replace_broken_files(&raw_query);
    let outcome = super::helpers::spawn_blocking_io("write venue", move || {
        let name = venue_name;
        let _guard = VENUE_WRITES.blocking_lock();
        let reply = |r: axum::response::Response| Ok::<_, String>(Err(r));
        let bad = |e: String| (StatusCode::BAD_REQUEST, Json(json!({"error": e}))).into_response();
        let located = locate_venue_file(&dir, &name)?;
        if let Err(r) = validate_lighting_name_to_write(&name, located.is_some()) {
            return reply(r);
        }
        if let Some(expected) = &expected {
            let current = located.as_ref().map(LocatedVenue::version);
            if current.as_deref() != Some(expected.as_str()) {
                return reply(stale_venue_response(&name, current));
            }
        }
        let dsl = match incoming {
            Incoming::Dsl(text) => text,
            // An existing file is patched, not regenerated: comments, TODO
            // lines and the file's own layout survive a save from the UI.
            Incoming::Json(json_body, venue) => match &located {
                Some(l) => match l.text() {
                    Some(text) => match lighting::venue_patch::patch_venue(text, &name, &venue) {
                        Ok(patched) => patched,
                        Err(e) => return reply(bad(e)),
                    },
                    // A file that does not hold the venue is not patched: the
                    // venue is written whole, and the rule below decides
                    // whether that file may be replaced (a broken one the
                    // person chose to replace) or not.
                    None => match venue_json_to_dsl(&name, &json_body) {
                        Ok(dsl) => dsl,
                        Err(e) => return reply(bad(e)),
                    },
                },
                None => match venue_json_to_dsl(&name, &json_body) {
                    Ok(dsl) => dsl,
                    Err(e) => return reply(bad(e)),
                },
            },
        };

        // Validate the DSL parses correctly
        let venues = match lighting::parser::parse_venues(&dsl) {
            Ok(v) => v,
            Err(e) => return reply(bad(format!("Invalid venue DSL: {e}"))),
        };
        if let Err(r) = require_declared_name("venue", &name, &venues) {
            return reply(r);
        }
        let typed = venues.values().any(needs_venue_extension);

        // The extension is the version marker: a venue using positions,
        // focus points or MVR provenance is a `.venue`; anything else stays
        // `.light`. An existing file keeps its name and extension unless the
        // content outgrows `.light`, in which case it moves to `.venue` and
        // the `.light` original is retired after the durable write.
        let file_path = match &located {
            Some(l)
                if !typed || crate::songs::extension_of(&l.path).as_deref() == Some("venue") =>
            {
                l.path.clone()
            }
            Some(l) => l.path.with_extension("venue"),
            // A new venue, named past a file at its stem that holds other
            // venues (`project_files::new_file_stem`).
            None => dir.join(format!(
                "{}.{}",
                lighting::project_files::new_file_stem(&dir, &name, VENUE_EXTENSIONS, &|_| false),
                if typed { "venue" } else { "light" }
            )),
        };
        // The files the save acts on must hold the venue, whichever way it
        // comes: the file found for the name, and the one written when it is
        // another. One that does not parse, or holds another venue, is not
        // replaced, unless the person chose to replace it; every broken one
        // is named at once.
        let mut acted_on: Vec<(std::path::PathBuf, Declares)> = Vec::new();
        if let Some(l) = &located {
            acted_on.push((l.path.clone(), venues_declared(&l.path)));
            for path in &l.also_broken {
                acted_on.push((path.clone(), venues_declared(path)));
            }
        }
        if file_path.exists() && !acted_on.iter().any(|(p, _)| *p == file_path) {
            acted_on.push((file_path.clone(), venues_declared(&file_path)));
        }
        let pairs: Vec<(&std::path::Path, &Declares)> =
            acted_on.iter().map(|(p, d)| (p.as_path(), d)).collect();
        if let Err(r) = require_declares(
            "venue",
            &name,
            &dir,
            &pairs,
            &replace_broken,
            Acting::Saving,
        ) {
            return reply(r);
        }
        let broken = broken_among(&pairs);
        let stale_twin = located
            .as_ref()
            .map(|l| l.path.clone())
            .filter(|path| path != &file_path && !broken.contains(path));
        // A twin that also declares a venue the saved text does not is not
        // retired, which would lose that venue: the save is refused, by the
        // rule fixture types take (`lost_by_retiring`).
        if let Some(twin) = &stale_twin {
            if let Ok(declared) = venues_declared(twin) {
                let lost = lost_by_retiring(declared, |n| venues.contains_key(n));
                if !lost.is_empty() {
                    return reply(shared_file_response(
                        "venue",
                        &name,
                        twin,
                        &lost,
                        "saving this venue in another form",
                    ));
                }
            }
        }
        // The file the venue moves to is taken unless it is a broken one the
        // person chose to replace, set aside below.
        if stale_twin.is_some() && file_path.exists() && !broken.contains(&file_path) {
            return reply(
                (
                    StatusCode::CONFLICT,
                    Json(json!({"error": format!(
                        "{} already exists; it would have to be replaced to keep this venue's \
                         new fields",
                        crate::util::filename_display(&file_path)
                    )})),
                )
                    .into_response(),
            );
        }
        let file_path = match lighting_write_path(&dir, &file_path) {
            Ok(path) => path,
            Err(r) => return reply(r),
        };
        // A broken file the person chose to replace is set aside, kept
        // (`set_aside_broken`), before the save writes; so is the `.light`
        // twin the save retires. A failed write puts them all back.
        let mut retired = broken;
        retired.extend(stale_twin);
        let set_aside = set_aside_all(&retired)?;
        if let Err(e) = config_io::staged_write(&file_path, &dsl) {
            return Err(format!("{e}{}", put_back(&set_aside)));
        }
        let kept: Vec<String> = set_aside
            .iter()
            .map(|(_, backup)| lighting::project_files::display_in(&dir, backup))
            .collect();
        Ok::<_, String>(Ok((content_version(dsl.as_bytes()), kept)))
    })
    .await?;
    let (version, kept) = outcome?;

    let reloaded = reload_if_current_venue(&state, &name).await;
    let venue_error = venue_error_after_save(&state, &name, &saved_in).await;

    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(crate::webui::wire::venue::VenueSaved {
                status: "saved".to_string(),
                name,
                reloaded,
                version,
                venue_error,
                kept,
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
    let player = state.player.clone();
    let config_path = state.config_path.clone();
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
        let lighting = config.dmx().and_then(|d| d.lighting())?;
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
/// is playing against, re-read it and push fresh stage metadata to every
/// web client. Returns whether that happened. A reload failure is logged,
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

/// DELETE /api/lighting/venues/:name — deletes a venue file.
pub(super) async fn delete_venue(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
    RawQuery(raw_query): RawQuery,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let dir = resolve_lighting_dir(&state.config_path, query.dir.as_deref(), DEFAULT_VENUES_DIR)?;
    // Held from reading the file to deleting it, as MCP's delete does: a
    // save adding another venue to it between the two would be lost (#578).
    let _writing = VENUE_WRITES.lock().await;
    // By declaration, as GET and PUT find it: an MVR import names the file by
    // its own rule, so the stem need not be this API's spelling of the name.
    let located = {
        let (dir, name) = (dir.clone(), name.clone());
        super::helpers::spawn_blocking_io("locate venue file", move || {
            locate_venue_file(&dir, &name)
        })
        .await?
    };
    let Some(located) = located else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Venue not found: {}", name)})),
        )
            .into_response());
    };
    // What the file declares, read once off the async runtime: the one rule
    // for the file a delete acts on, then the others it would take with it.
    // The venue's other broken candidates go with it, as they would come
    // back to declare it once fixed.
    let files: Vec<std::path::PathBuf> = std::iter::once(located.path.clone())
        .chain(located.also_broken.iter().cloned())
        .collect();
    let all_declares = {
        let files = files.clone();
        super::helpers::spawn_blocking_io("read a venue file", move || {
            Ok::<_, String>(
                files
                    .iter()
                    .map(|path| venues_declared(path))
                    .collect::<Vec<_>>(),
            )
        })
        .await?
    };
    let pairs: Vec<(&std::path::Path, &Declares)> = files
        .iter()
        .map(std::path::PathBuf::as_path)
        .zip(all_declares.iter())
        .collect();
    require_declares(
        "venue",
        &name,
        &dir,
        &pairs,
        &replace_broken_files(&raw_query),
        Acting::Deleting,
    )?;
    let broken = broken_among(&pairs);
    let declares = all_declares[0].clone();
    // Deleting the file deletes everything in it.
    let mut others: Vec<String> = declares
        .map(|venues| venues.into_iter().filter(|n| n != &name).collect())
        .unwrap_or_default();
    if !others.is_empty() {
        others.sort();
        return Err((
            StatusCode::CONFLICT,
            Json(json!({"error": format!(
                "venue \"{}\" lives in {}, which also defines {}; deleting this one would remove \
                 them — edit the file by hand instead",
                name,
                located.file(),
                others
                    .iter()
                    .map(|n| format!("\"{n}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            )})),
        )
            .into_response());
    }
    let fp = located.path;
    let kept_root = dir.clone();
    // The broken files the person chose to remove are set aside, kept
    // (`set_aside_broken`); one that reads is removed.
    let kept = super::helpers::spawn_blocking_io("delete venue", move || {
        let set_aside = set_aside_all(&broken)?;
        if !broken.contains(&fp) {
            if let Err(e) = std::fs::remove_file(&fp) {
                return Err(format!("{e}{}", put_back(&set_aside)));
            }
        }
        Ok::<_, String>(
            set_aside
                .iter()
                .map(|(_, backup)| lighting::project_files::display_in(&kept_root, backup))
                .collect::<Vec<_>>(),
        )
    })
    .await?;
    let mut reply = json!({"status": "deleted", "name": name});
    if !kept.is_empty() {
        reply["kept"] = json!(kept);
    }
    Ok::<_, axum::response::Response>((StatusCode::OK, Json(reply)).into_response())
}

// ---------------------------------------------------------------------------
// Lighting helpers
// ---------------------------------------------------------------------------

/// Converts a JSON fixture type definition to DSL format.
fn fixture_type_json_to_dsl(name: &str, json: &serde_json::Value) -> Result<String, String> {
    fixture_type_from_json(name, json).map(|fixture_type| format!("{fixture_type}\n"))
}

/// Builds the v1 fixture type a JSON save describes: its channel map, the
/// three strobe fields and its `strobe_curve`.
fn fixture_type_from_json(
    name: &str,
    json: &serde_json::Value,
) -> Result<lighting::types::FixtureType, String> {
    let channels = json
        .get("channels")
        .and_then(|v| v.as_object())
        .ok_or("Missing 'channels' object")?;

    let mut entries: Vec<(&String, &serde_json::Value)> = channels.iter().collect();
    entries.sort_by_key(|(_, v)| v.as_u64().unwrap_or(0));
    // The editor's own check of its rows (`venue_rows`), its first finding
    // the refusal.
    let mut rows = Vec::new();
    for (ch_name, ch_offset) in entries {
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

    let strobe_dmx_offset = match json.get("strobe_dmx_offset").and_then(|v| v.as_u64()) {
        None => None,
        Some(v) => Some(
            u8::try_from(v).map_err(|_| "strobe_dmx_offset must be from 0 to 255".to_string())?,
        ),
    };
    let strobe_curve = match json.get("strobe_curve") {
        None | Some(serde_json::Value::Null) => None,
        Some(value) => Some(
            serde_json::from_value::<lighting::types::StrobeCurve>(value.clone())
                .map_err(|_| format!("Unknown strobe_curve: {value}"))?,
        ),
    };
    let mut fixture_type: lighting::types::FixtureType = lighting::types::FixtureTypeV1 {
        name: name.to_string(),
        // Each offset in 1..=65535, by the check above.
        channels: rows
            .into_iter()
            .map(|row| (row.name, row.offset.unwrap_or_default() as u16))
            .collect(),
        max_strobe_frequency: json.get("max_strobe_frequency").and_then(|v| v.as_f64()),
        min_strobe_frequency: json.get("min_strobe_frequency").and_then(|v| v.as_f64()),
        strobe_dmx_offset,
    }
    .into();
    fixture_type.set_strobe_curve(strobe_curve);
    Ok(fixture_type)
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
    let venues_dir =
        resolve_lighting_dir(&state.config_path, query.dir.as_deref(), DEFAULT_VENUES_DIR)?;
    let types_dir = resolve_lighting_dir(
        &state.config_path,
        query.fixture_types_dir.as_deref(),
        DEFAULT_FIXTURE_TYPES_DIR,
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
/// websocket's metadata for the current venue (each fixture's type, mode,
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
    let venues_dir =
        resolve_lighting_dir(&state.config_path, query.dir.as_deref(), DEFAULT_VENUES_DIR)?;
    let types_dir = resolve_lighting_dir(
        &state.config_path,
        query.fixture_types_dir.as_deref(),
        DEFAULT_FIXTURE_TYPES_DIR,
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
    async fn get_lighting_files_empty() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["files"].is_array());
    }

    #[tokio::test]
    async fn get_lighting_files_with_files() {
        let (state, _dir) = test_state();
        std::fs::write(state.songs_path().join("show.light"), "content").unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["files"].as_array().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn get_lighting_files_sorted() {
        let (state, _dir) = test_state();
        std::fs::write(state.songs_path().join("z_show.light"), "content").unwrap();
        std::fs::write(state.songs_path().join("a_show.light"), "content").unwrap();
        std::fs::write(state.songs_path().join("m_show.light"), "content").unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let files = parsed["files"].as_array().unwrap();
        assert_eq!(files.len(), 3);
        let paths: Vec<&str> = files.iter().map(|f| f["path"].as_str().unwrap()).collect();
        assert_eq!(paths, vec!["a_show.light", "m_show.light", "z_show.light"]);
    }

    /// A `.LIGHT` file, listed and loaded as a light show, can be deleted.
    #[tokio::test]
    async fn delete_lighting_file_in_any_case() {
        let (state, _dir) = test_state();
        let file = state.songs_path().join("Show.LIGHT");
        std::fs::write(&file, "light content").unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri("/lighting/Show.LIGHT")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(!file.exists());
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

    /// A light show save writes only a lighting file (`FileKind`, in any
    /// case), as the delete deletes only one, and not through a link to
    /// nothing.
    #[tokio::test]
    async fn put_lighting_file_writes_only_a_lighting_file() {
        let (state, _dir) = test_state();
        let content =
            "show \"test\" {\n    @00:00.000\n    lights: static color: \"red\", duration: 5s\n}\n";
        let outside = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(
            outside.path().join("x.light"),
            state.songs_path().join("dead.light"),
        )
        .unwrap();
        let put = |uri: &'static str| {
            router().with_state(state.clone()).oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(uri)
                    .body(Body::from(content))
                    .unwrap(),
            )
        };
        for uri in [
            "/lighting/notes.txt",
            "/lighting/song.yaml",
            "/lighting/Foo%2F.x.light",
            "/lighting/.x.light",
        ] {
            assert_eq!(
                put(uri).await.unwrap().status(),
                StatusCode::BAD_REQUEST,
                "{uri}"
            );
        }
        assert!(!state.songs_path().join("notes.txt").exists());
        assert!(!state.songs_path().join(".x.light").exists());
        assert_eq!(
            put("/lighting/Show.LIGHT").await.unwrap().status(),
            StatusCode::OK
        );
        #[cfg(unix)]
        {
            assert_ne!(
                put("/lighting/dead.light").await.unwrap().status(),
                StatusCode::OK
            );
            assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
        }
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
    async fn get_lighting_files_missing_dir() {
        let (state, _dir) = test_state();
        {
            let mut sources = state.player.song_sources().unwrap_or_default();
            sources.songs = std::path::PathBuf::from("/nonexistent/songs");
            state.player.set_song_sources(sources);
        }
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
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

    /// `Show.LIGHT` is a light show to the listing as it is to `songs
    /// --init` (`FileKind`), and a hidden directory or file is passed over
    /// as the song loader passes it over.
    #[test]
    fn find_light_files_reads_the_extension_in_any_case_and_skips_hidden() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        std::fs::create_dir_all(base.join("song/.git")).unwrap();
        std::fs::write(base.join("song/Show.LIGHT"), "content").unwrap();
        std::fs::write(base.join("song/.git/old.light"), "content").unwrap();
        std::fs::write(base.join("song/.draft.light"), "content").unwrap();

        let mut results = Vec::new();
        find_light_files(base, base, &mut results).unwrap();

        let paths: Vec<&str> = results.iter().map(|r| r.path.as_str()).collect();
        assert_eq!(paths, ["song/Show.LIGHT"]);
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

    #[tokio::test]
    async fn get_lighting_files_scan_error_returns_500() {
        use std::os::unix::fs::PermissionsExt;

        // An unreadable songs directory is an error, not an empty list.
        let (state, _dir) = test_state();
        let songs = state.songs_path();
        std::fs::set_permissions(&songs, std::fs::Permissions::from_mode(0o000)).unwrap();

        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        std::fs::set_permissions(&songs, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["error"]
            .as_str()
            .unwrap()
            .contains("Failed to scan for lighting files"));
    }

    #[tokio::test]
    async fn get_lighting_files_passes_over_an_unreadable_directory() {
        use std::os::unix::fs::PermissionsExt;

        // The one walk of lighting files passes over a subdirectory it
        // cannot read, with a warning, and lists the rest.
        let (state, _dir) = test_state();
        std::fs::write(state.songs_path().join("show.light"), "content").unwrap();
        let sub = state.songs_path().join("unreadable_dir");
        std::fs::create_dir(&sub).unwrap();
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();

        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/lighting")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let paths: Vec<&str> = parsed["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        assert_eq!(paths, ["show.light"]);
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
    // Unit tests: sanitize_filename
    // -----------------------------------------------------------------------

    #[test]
    fn sanitize_filename_removes_special_chars() {
        use lighting::project_files::name_stem;
        assert_eq!(name_stem("hello world"), "hello_world");
        assert_eq!(name_stem("My-Fixture_01"), "my-fixture_01");
        assert_eq!(name_stem("a/b\\c.d!e"), "a_b_c_d_e");
        assert_eq!(name_stem("UPPER"), "upper");
        assert_eq!(name_stem(""), "untitled");
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

    /// A type's `strobe_curve` survives a form save: it is written, and
    /// reads back as it was sent.
    #[test]
    fn fixture_type_json_to_dsl_keeps_strobe_curve() {
        let json = serde_json::json!({
            "channels": {"dimmer": 1, "strobe": 2},
            "strobe_curve": "linear"
        });
        let dsl = fixture_type_json_to_dsl("Flash", &json).unwrap();
        assert!(dsl.contains("strobe_curve: linear"), "{dsl}");
        let types = lighting::parser::parse_fixture_types(&dsl).unwrap();
        assert_eq!(
            types["Flash"].strobe_curve(),
            Some(lighting::types::StrobeCurve::Linear)
        );

        let json = serde_json::json!({"channels": {"dimmer": 1}, "strobe_curve": null});
        let dsl = fixture_type_json_to_dsl("Plain", &json).unwrap();
        assert!(!dsl.contains("strobe_curve"), "{dsl}");

        let json = serde_json::json!({"channels": {"dimmer": 1}, "strobe_curve": "wobbly"});
        assert!(fixture_type_json_to_dsl("Bad", &json).is_err());
    }

    /// The editor's row check refuses what it did before, with its words.
    #[test]
    fn fixture_type_json_to_dsl_refuses_bad_rows() {
        let json = serde_json::json!({"channels": {"red": 0}});
        assert_eq!(
            fixture_type_json_to_dsl("Bad", &json).unwrap_err(),
            "Channel 'red' offset must be from 1 to 65535"
        );
        let json = serde_json::json!({"channels": {"": 1}});
        assert_eq!(
            fixture_type_json_to_dsl("Bad", &json).unwrap_err(),
            "A channel has no name"
        );
        let json = serde_json::json!({"channels": {"red": "one"}});
        assert_eq!(
            fixture_type_json_to_dsl("Bad", &json).unwrap_err(),
            "Channel 'red' offset must be a number"
        );
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
        assert!(needs_venue_extension(&venue));
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
    /// names it — which is not how this API spells the same name.
    fn imported_fixture_type_dir(
        project: &std::path::Path,
        rel: &str,
        name: &str,
        dsl: String,
    ) -> std::path::PathBuf {
        let stem = lighting::import::fixture_filename_stem(name);
        assert_ne!(
            stem,
            lighting::project_files::name_stem(name),
            "the stems must differ"
        );
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

    /// `Par.FIXTURE` is a `.fixture` to the form-save guard, as it is to the
    /// loader: the channel-map form is refused, and no `par.light` is made
    /// beside it to be shadowed on reload.
    #[tokio::test]
    async fn a_fixture_file_in_capitals_is_not_form_saved() {
        let (state, _dir) = test_state();
        let rel = "ft_upper";
        let dir = _dir.path().join(rel);
        std::fs::create_dir(&dir).unwrap();
        let file = dir.join("Par.FIXTURE");
        std::fs::write(&file, sample_rich_fixture_type_dsl("Par")).unwrap();

        let (status, parsed) = fixture_type_request(
            state.clone(),
            "GET",
            format!("/lighting/fixture-types/Par?dir={rel}"),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["extension"], "fixture");

        let form = serde_json::json!({"channels": {"red": 1}});
        let (status, parsed) = fixture_type_request(
            state,
            "PUT",
            format!("/lighting/fixture-types/Par?dir={rel}"),
            "application/json",
            Body::from(serde_json::to_vec(&form).unwrap()),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert_eq!(
            std::fs::read_to_string(&file).unwrap(),
            sample_rich_fixture_type_dsl("Par")
        );
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }

    /// A file at a name's stem is found whatever its extension's case, for
    /// a fixture type that it does not declare and for a venue.
    #[test]
    fn existing_files_at_the_stem_are_found_in_any_case() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("par.FIXTURE"), "not a type").unwrap();
        std::fs::write(dir.path().join("house.VENUE"), "{{{").unwrap();
        assert_eq!(
            existing_fixture_type_file(dir.path(), dir.path(), "par"),
            Some(dir.path().join("par.FIXTURE"))
        );
        assert_eq!(
            locate_venue_file(dir.path(), "house")
                .unwrap()
                .map(|l| l.path),
            Some(dir.path().join("house.VENUE"))
        );
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
        let file = dir.join(format!(
            "{}.light",
            lighting::import::fixture_filename_stem("House-Rig")
        ));
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
        let file_path = _dir.path().join(rel).join("myfixture.light");
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

        // A name another fixture has is the caller's error, not a server
        // fault.
        let other = crate::lighting::gdtf::build_zip(&[(
            "description.xml",
            crate::lighting::gdtf::SYNTHETIC_DESCRIPTION
                .replace("Synth Brick", "Other Brick")
                .as_bytes(),
        )]);
        let (status, parsed) = import_upload(&app, "other.gdtf", &other, "?name=Brick").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{parsed}");
        assert!(
            parsed["error"].as_str().unwrap().contains("already exists"),
            "{parsed}"
        );
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

    /// A GDTF in the library is its fixture even behind a broken file at its
    /// name's stem that does not seem to open its block: GET, settings and
    /// DELETE act on the library's, and the broken file is left alone.
    #[tokio::test]
    async fn the_library_answers_behind_an_unrelated_broken_stem_file() {
        let (state, dir) = test_state();
        let app = router().with_state(state.clone());
        let (status, _) = import_upload(&app, "synth.gdtf", &synthetic_gdtf_bytes(), "").await;
        assert_eq!(status, StatusCode::OK);
        let types = dir.path().join("lighting/fixture_types");
        std::fs::create_dir_all(&types).unwrap();
        let broken = "fixture_type \"Spot\" {\n";
        std::fs::write(types.join("synth_brick.light"), broken).unwrap();
        let uri = "/lighting/fixture-types/Synth%20Brick";
        let request = |method: &'static str, uri: String| {
            fixture_type_request(state.clone(), method, uri, "text/plain", Body::empty())
        };

        let (status, parsed) = request("GET", uri.to_string()).await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["referential"], true, "{parsed}");
        let (status, parsed) = request("GET", format!("{uri}/settings")).await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        let (status, parsed) = request("DELETE", uri.to_string()).await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["archive_removed"], "lighting/library/synth.gdtf");
        assert!(!dir.path().join("lighting/library/synth.gdtf").exists());
        assert_eq!(
            std::fs::read_to_string(types.join("synth_brick.light")).unwrap(),
            broken
        );
    }

    /// A broken record mtrack's GDTF import wrote is the type's, never
    /// "unrelated": its DELETE offers to remove it (409) and leaves the
    /// library archive alone; and a broken file anywhere that names the
    /// archive keeps the archive from being deleted (409).
    #[tokio::test]
    async fn a_broken_import_record_keeps_its_archive() {
        let (state, dir) = test_state();
        let app = router().with_state(state.clone());
        let (status, _) = import_upload(&app, "synth.gdtf", &synthetic_gdtf_bytes(), "").await;
        assert_eq!(status, StatusCode::OK);
        let archive = dir.path().join("lighting/library/synth.gdtf");
        let types = dir.path().join("lighting/fixture_types");
        std::fs::create_dir_all(&types).unwrap();
        let delete = || {
            fixture_type_request(
                state.clone(),
                "DELETE",
                "/lighting/fixture-types/Synth%20Brick".to_string(),
                "text/plain",
                Body::empty(),
            )
        };

        let record =
            "fixture_type \"Synth Brick\"\n  from gdtf(\"lighting/library/synth.gdtf\")\n{\n";
        std::fs::write(types.join("synth_brick.fixture"), record).unwrap();
        let (status, parsed) = delete().await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert_eq!(
            parsed["broken_files"][0]["file"], "synth_brick.fixture",
            "{parsed}"
        );
        assert!(archive.exists());
        std::fs::remove_file(types.join("synth_brick.fixture")).unwrap();

        std::fs::write(
            types.join("other.fixture"),
            "{{{ lighting/library/synth.gdtf\n",
        )
        .unwrap();
        let (status, parsed) = delete().await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert_eq!(
            parsed["broken_files"][0]["file"], "other.fixture",
            "{parsed}"
        );
        assert!(archive.exists());
        // Offered for removing as any broken file is: the retry sets it
        // aside, kept, and deletes the library's fixture.
        let (status, parsed) = fixture_type_request(
            state.clone(),
            "DELETE",
            "/lighting/fixture-types/Synth%20Brick?replace_broken=other.fixture".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert!(!archive.exists());
        let kept = parsed["kept"][0].as_str().unwrap();
        assert!(types.join(kept).is_file());
    }

    /// A recorded GDTF fixture's archive is not removed with its record
    /// while a file that does not read may be another record of it; the
    /// reply says why it stays.
    #[tokio::test]
    async fn an_archive_a_broken_file_names_is_kept() {
        let (state, dir) = test_state();
        let app = router().with_state(state.clone());
        let (status, _) = import_upload(&app, "synth.gdtf", &synthetic_gdtf_bytes(), "").await;
        assert_eq!(status, StatusCode::OK);
        let types = dir.path().join("lighting/fixture_types");
        std::fs::create_dir_all(&types).unwrap();
        std::fs::write(
            types.join("par.fixture"),
            "fixture_type \"Par\"\n  from gdtf(\"lighting/library/synth.gdtf\")\n{\n}\n",
        )
        .unwrap();
        std::fs::write(
            types.join("stage_par.fixture"),
            "fixture_type \"Stage Par\"\n  from gdtf(\"lighting/library/synth.gdtf\")\n{\n",
        )
        .unwrap();
        let (status, parsed) = fixture_type_request(
            state,
            "DELETE",
            "/lighting/fixture-types/Par".to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["archive_kept"], true, "{parsed}");
        assert!(
            parsed["archive_kept_because"]
                .as_str()
                .unwrap()
                .contains("stage_par.fixture"),
            "{parsed}"
        );
        assert!(dir.path().join("lighting/library/synth.gdtf").exists());
    }

    /// A venue moving from `.light` to `.venue` onto a broken `club.venue`
    /// the person chose to replace saves: the broken file and the retired
    /// twin are both set aside, kept.
    #[tokio::test]
    async fn a_typed_venue_save_replaces_a_broken_venue_file() {
        let (state, _dir) = test_state();
        let dir = _dir.path().join("v_move");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("club.light"), sample_venue_dsl("Club")).unwrap();
        std::fs::write(dir.join("club.venue"), "venue \"Club\" {\n").unwrap();
        let typed = "venue \"Club\" {\n  focus \"DS\" (0, 0, 0)\n}\n";
        let put = |query: &'static str| {
            fixture_type_request(
                state.clone(),
                "PUT",
                format!("/lighting/venues/Club?dir=v_move{query}"),
                "text/plain",
                Body::from(typed),
            )
        };

        let (status, parsed) = put("").await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert_eq!(parsed["broken_files"][0]["file"], "club.venue", "{parsed}");

        let (status, parsed) = put("&replace_broken=club.venue").await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(
            std::fs::read_to_string(dir.join("club.venue")).unwrap(),
            typed
        );
        assert!(!dir.join("club.light").exists());
        let mut kept: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".replaced-"))
            .collect();
        kept.sort();
        assert_eq!(kept.len(), 2, "{kept:?}");
        assert!(kept[0].starts_with("club.light.replaced-"), "{kept:?}");
        assert!(kept[1].starts_with("club.venue.replaced-"), "{kept:?}");
    }

    /// A file at the target that reads but declares nothing (empty, or
    /// comments only) is offered for replacing as a broken one is, and set
    /// aside when replaced.
    #[tokio::test]
    async fn an_empty_target_file_is_offered_for_replacing() {
        let (state, _dir) = test_state();
        for (n, text) in ["", "// to do\n"].iter().enumerate() {
            let rel = format!("v_empty_{n}");
            let dir = _dir.path().join(&rel);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("club.light"), sample_venue_dsl("Club")).unwrap();
            std::fs::write(dir.join("club.venue"), text).unwrap();
            let typed = "venue \"Club\" {\n  focus \"DS\" (0, 0, 0)\n}\n";
            let put = |query: &'static str| {
                fixture_type_request(
                    state.clone(),
                    "PUT",
                    format!("/lighting/venues/Club?dir={rel}{query}"),
                    "text/plain",
                    Body::from(typed),
                )
            };
            let (status, parsed) = put("").await;
            assert_eq!(status, StatusCode::CONFLICT, "{text:?}: {parsed}");
            assert_eq!(parsed["broken"], true, "{text:?}: {parsed}");
            assert_eq!(parsed["broken_files"][0]["file"], "club.venue", "{parsed}");
            let (status, parsed) = put("&replace_broken=club.venue").await;
            assert_eq!(status, StatusCode::OK, "{text:?}: {parsed}");
            assert_eq!(
                std::fs::read_to_string(dir.join("club.venue")).unwrap(),
                typed
            );
            let kept = parsed["kept"]
                .as_array()
                .unwrap()
                .iter()
                .map(|k| k.as_str().unwrap())
                .find(|k| k.starts_with("club.venue."))
                .unwrap()
                .to_string();
            assert_eq!(std::fs::read_to_string(dir.join(kept)).unwrap(), *text);
        }
    }

    /// A file holding only other blocks (a sequence library in a shared
    /// directory) reads: it is not offered for replacing, only refused; an
    /// empty one at the name's stem is the name's, offered for replacing on
    /// a save and for removing on a delete (not passed over for
    /// `club_2.light`, nor a 404).
    #[tokio::test]
    async fn a_file_of_other_blocks_reads_and_an_empty_stem_file_is_offered() {
        let (state, _dir) = test_state();
        let dir = _dir.path().join("v_seq");
        std::fs::create_dir_all(&dir).unwrap();
        let library =
            "sequence \"q\" {\n    @0.000\n    spot: static color: \"blue\", duration: 1s\n}\n";
        std::fs::write(dir.join("club.light"), sample_venue_dsl("Club")).unwrap();
        std::fs::write(dir.join("club.venue"), library).unwrap();
        let typed = "venue \"Club\" {\n  focus \"DS\" (0, 0, 0)\n}\n";
        let (status, parsed) = fixture_type_request(
            state.clone(),
            "PUT",
            "/lighting/venues/Club?dir=v_seq&replace_broken=club.venue".to_string(),
            "text/plain",
            Body::from(typed),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert_eq!(parsed["broken"], false, "{parsed}");
        assert_eq!(
            std::fs::read_to_string(dir.join("club.venue")).unwrap(),
            library
        );

        let dir = _dir.path().join("v_empty_stem");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("club.light"), "// later\n").unwrap();
        let send = |method: &'static str, query: &'static str| {
            fixture_type_request(
                state.clone(),
                method,
                format!("/lighting/venues/Club?dir=v_empty_stem{query}"),
                "text/plain",
                Body::from(sample_venue_dsl("Club")),
            )
        };
        let (status, parsed) = send("DELETE", "").await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert_eq!(parsed["broken_files"][0]["file"], "club.light", "{parsed}");
        let (status, parsed) = send("PUT", "").await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert_eq!(parsed["broken_files"][0]["file"], "club.light", "{parsed}");
        let (status, parsed) = send("PUT", "&replace_broken=club.light").await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(
            std::fs::read_to_string(dir.join("club.light")).unwrap(),
            sample_venue_dsl("Club")
        );
        assert!(!dir.join("club_2.light").exists());
    }

    /// A raw-text save that would retire a `.light` twin holding another
    /// venue too is refused, as a fixture type's is: Arena is not lost.
    #[tokio::test]
    async fn a_venue_save_does_not_retire_a_twin_holding_another_venue() {
        let (state, _dir) = test_state();
        let dir = _dir.path().join("v_lost");
        std::fs::create_dir_all(&dir).unwrap();
        let both = format!(
            "{}\n{}\n",
            sample_venue_dsl("Club"),
            sample_venue_dsl("Arena")
        );
        std::fs::write(dir.join("club.light"), &both).unwrap();
        let (status, parsed) = fixture_type_request(
            state,
            "PUT",
            "/lighting/venues/Club?dir=v_lost".to_string(),
            "text/plain",
            Body::from("venue \"Club\" {\n  focus \"DS\" (0, 0, 0)\n}\n"),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert!(
            parsed["error"].as_str().unwrap().contains("\"Arena\""),
            "{parsed}"
        );
        assert_eq!(
            std::fs::read_to_string(dir.join("club.light")).unwrap(),
            both
        );
        assert!(!dir.join("club.venue").exists());
    }

    /// A delete that cannot set every broken file aside puts back those it
    /// did, and removes nothing.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_delete_that_cannot_set_every_file_aside_moves_none() {
        use std::os::unix::fs::PermissionsExt;
        let (state, _dir) = test_state();
        let dir = _dir.path().join("v_put_back");
        for sub in ["a", "b"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
            std::fs::write(dir.join(sub).join("club.light"), "venue \"Club\" {\n").unwrap();
        }
        std::fs::set_permissions(dir.join("b"), std::fs::Permissions::from_mode(0o555)).unwrap();
        let (status, parsed) = fixture_type_request(
            state,
            "DELETE",
            "/lighting/venues/Club?dir=v_put_back&replace_broken=a/club.light&replace_broken=b/club.light"
                .to_string(),
            "text/plain",
            Body::empty(),
        )
        .await;
        std::fs::set_permissions(dir.join("b"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR, "{parsed}");
        for sub in ["a", "b"] {
            let names: Vec<String> = std::fs::read_dir(dir.join(sub))
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            assert_eq!(names, ["club.light"], "{sub}");
        }
    }

    /// Every broken file that may be a name's is acted on together: a broken
    /// file at the stem and one elsewhere opening the name's block, or two
    /// such files with none at the stem, are named in one 409 and set aside
    /// together, so none comes back to declare the name once fixed.
    #[tokio::test]
    async fn every_broken_candidate_is_offered_and_kept() {
        let (state, _dir) = test_state();
        for (rel, files) in [
            (
                "v_cands_stem",
                vec![
                    ("club.light", "{{{\n"),
                    ("tour/old.light", "venue \"Club\" {\n"),
                ],
            ),
            (
                "v_cands_two",
                vec![
                    ("a/x.light", "venue \"Club\" {\n"),
                    ("b/y.light", "venue \"Club\" {\n  ?\n"),
                ],
            ),
        ] {
            let dir = _dir.path().join(rel);
            for (file, text) in &files {
                std::fs::create_dir_all(dir.join(file).parent().unwrap()).unwrap();
                std::fs::write(dir.join(file), text).unwrap();
            }
            let send = |query: String| {
                fixture_type_request(
                    state.clone(),
                    "PUT",
                    format!("/lighting/venues/Club?dir={rel}{query}"),
                    "text/plain",
                    Body::from(sample_venue_dsl("Club")),
                )
            };
            let (status, parsed) = send(String::new()).await;
            assert_eq!(status, StatusCode::CONFLICT, "{rel}: {parsed}");
            let mut named: Vec<String> = parsed["broken_files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|f| f["file"].as_str().unwrap().to_string())
                .collect();
            named.sort();
            let mut expected: Vec<String> = files.iter().map(|(f, _)| f.to_string()).collect();
            expected.sort();
            assert_eq!(named, expected, "{rel}");
            let flags: String = expected
                .iter()
                .map(|f| format!("&replace_broken={f}"))
                .collect();
            let (status, parsed) = send(flags).await;
            assert_eq!(status, StatusCode::OK, "{rel}: {parsed}");
            assert_eq!(parsed["kept"].as_array().map(Vec::len), Some(2), "{parsed}");
            let loaded = lighting::project_files::venues(&dir)
                .items
                .iter()
                .filter(|d| d.name == "Club")
                .count();
            assert_eq!(loaded, 1, "{rel}");
        }
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
        let file_path = _dir.path().join(rel).join("jsonfixture.light");
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
        let file_path = _dir.path().join(rel).join("myvenue.light");
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
        let file_path = _dir.path().join(rel).join("jsonvenue.light");
        assert!(file_path.exists());
        let content = std::fs::read_to_string(&file_path).unwrap();
        let venues = lighting::parser::parse_venues(&content).unwrap();
        assert!(venues.contains_key("JSONVenue"));
    }

    #[tokio::test]
    async fn put_fixture_type_refuses_rich_channel_syntax() {
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
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        assert!(body.contains(".fixture"), "{body}");
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
    async fn put_fixture_type_json_refuses_a_fixture_file() {
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

        assert_eq!(response.status(), StatusCode::CONFLICT);
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
    async fn put_fixture_type_retires_the_stale_twin() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_twin");
        std::fs::create_dir(&ft_dir).unwrap();
        // A GDTF import can land a `.fixture` beside a hand-written `.light`
        // of the same name; the next save resolves the pair to one file.
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

        assert_eq!(response.status(), StatusCode::OK);
        assert!(ft_dir.join("mover.fixture").exists());
        assert!(!ft_dir.join("mover.light").exists());
    }

    /// A retired twin is set aside, kept, as any file a save replaces is,
    /// and named in the reply: `Par.light` beside a saved `par.fixture`.
    #[tokio::test]
    async fn put_fixture_type_sets_the_retired_twin_aside() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_aside");
        std::fs::create_dir(&ft_dir).unwrap();
        std::fs::write(ft_dir.join("Par.light"), sample_fixture_type_dsl("Par")).unwrap();
        std::fs::write(
            ft_dir.join("par.fixture"),
            sample_rich_fixture_type_dsl("Par"),
        )
        .unwrap();

        let (status, parsed) = fixture_type_request(
            state,
            "PUT",
            "/lighting/fixture-types/Par?dir=ft_aside&ext=fixture".to_string(),
            "text/plain",
            Body::from(sample_rich_fixture_type_dsl("Par")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert!(!ft_dir.join("Par.light").exists());
        let kept: Vec<String> = std::fs::read_dir(&ft_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.starts_with("Par.light.replaced-"))
            .collect();
        assert_eq!(kept.len(), 1, "{kept:?}");
        assert_eq!(
            std::fs::read_to_string(ft_dir.join(&kept[0])).unwrap(),
            sample_fixture_type_dsl("Par")
        );
        assert_eq!(parsed["kept"], json!([kept[0].clone()]), "{parsed}");
    }

    /// A file at the stem in the same form is not a twin, whatever its
    /// case, and a twin declaring another type is not retired: saving Par
    /// beside a `Par.LIGHT` holding Spot keeps Spot.
    #[tokio::test]
    async fn put_fixture_type_keeps_a_same_stem_file_of_another_type() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_case");
        std::fs::create_dir(&ft_dir).unwrap();
        std::fs::write(ft_dir.join("Par.light"), sample_fixture_type_dsl("Par")).unwrap();
        std::fs::write(ft_dir.join("Par.LIGHT"), sample_fixture_type_dsl("Spot")).unwrap();

        for ext in ["light", "fixture"] {
            let (status, parsed) = fixture_type_request(
                state.clone(),
                "PUT",
                format!("/lighting/fixture-types/Par?dir=ft_case&ext={ext}"),
                "text/plain",
                Body::from(sample_fixture_type_dsl("Par")),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{ext}: {parsed}");
            assert_eq!(
                std::fs::read_to_string(ft_dir.join("Par.LIGHT")).unwrap(),
                sample_fixture_type_dsl("Spot"),
                "{ext}"
            );
        }
        // Saved as a `.fixture`, the type's `.light` form is retired.
        assert!(ft_dir.join("Par.fixture").exists());
        assert!(!ft_dir.join("Par.light").exists());
    }

    /// A twin holding another type besides this one is not retired by a
    /// save in the other form, which would lose that type: the save is
    /// refused, and both files are left as they were.
    #[tokio::test]
    async fn put_fixture_type_keeps_a_twin_holding_another_type() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_shared_twin");
        std::fs::create_dir(&ft_dir).unwrap();
        let both = format!(
            "{}\n{}\n",
            sample_fixture_type_dsl("Par"),
            sample_fixture_type_dsl("Spot")
        );
        std::fs::write(ft_dir.join("Par.light"), &both).unwrap();
        std::fs::write(
            ft_dir.join("Par.fixture"),
            sample_rich_fixture_type_dsl("Par"),
        )
        .unwrap();

        let (status, parsed) = fixture_type_request(
            state,
            "PUT",
            "/lighting/fixture-types/Par?dir=ft_shared_twin&ext=fixture".to_string(),
            "text/plain",
            Body::from(sample_rich_fixture_type_dsl("Par")),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert!(
            parsed["error"].as_str().unwrap().contains("\"Spot\""),
            "{parsed}"
        );
        assert_eq!(
            std::fs::read_to_string(ft_dir.join("Par.light")).unwrap(),
            both
        );
    }

    /// A new type whose stem a file holding another type already has gets a
    /// file of its own (`project_files::new_file_stem`), in any case, as raw
    /// text or from the form; that file is left as it is.
    #[tokio::test]
    async fn a_new_type_is_named_past_a_stem_file_holding_another_type() {
        let (state, _dir) = test_state();
        for (rel, stray, name) in [
            ("ft_stray", "par.light", "par"),
            ("ft_stem_case", "Par.light", "Par"),
        ] {
            let ft_dir = _dir.path().join(rel);
            std::fs::create_dir(&ft_dir).unwrap();
            std::fs::write(ft_dir.join(stray), sample_fixture_type_dsl("Spot")).unwrap();
            let (status, parsed) = fixture_type_request(
                state.clone(),
                "PUT",
                format!("/lighting/fixture-types/{name}?dir={rel}"),
                "text/plain",
                Body::from(sample_fixture_type_dsl(name)),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{rel}: {parsed}");
            assert_eq!(
                std::fs::read_to_string(ft_dir.join(stray)).unwrap(),
                sample_fixture_type_dsl("Spot")
            );
            assert!(
                std::fs::read_to_string(ft_dir.join("par_2.light"))
                    .unwrap()
                    .contains(&format!("\"{name}\"")),
                "{rel}"
            );
        }
        // From the form too.
        let ft_dir = _dir.path().join("ft_stray_form");
        std::fs::create_dir(&ft_dir).unwrap();
        std::fs::write(ft_dir.join("par.light"), sample_fixture_type_dsl("Spot")).unwrap();
        let form = serde_json::json!({"channels": {"red": 1}});
        let (status, parsed) = fixture_type_request(
            state,
            "PUT",
            "/lighting/fixture-types/par?dir=ft_stray_form".to_string(),
            "application/json",
            Body::from(serde_json::to_vec(&form).unwrap()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert!(ft_dir.join("par_2.light").is_file());
    }

    /// Venues whose names share a stem get files of their own: "Club
    /// (Main)" beside "Club Main".
    #[tokio::test]
    async fn venues_sharing_a_stem_get_files_of_their_own() {
        let (state, _dir) = test_state();
        std::fs::create_dir(_dir.path().join("v_stems")).unwrap();
        for name in ["Club Main", "Club (Main)"] {
            let (status, parsed) = fixture_type_request(
                state.clone(),
                "PUT",
                format!(
                    "/lighting/venues/{}?dir=v_stems",
                    name.replace(' ', "%20")
                        .replace('(', "%28")
                        .replace(')', "%29")
                ),
                "text/plain",
                Body::from(sample_venue_dsl(name)),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{name}: {parsed}");
        }
        let dir = _dir.path().join("v_stems");
        assert!(std::fs::read_to_string(dir.join("club_main.light"))
            .unwrap()
            .contains("\"Club Main\""));
        assert!(std::fs::read_to_string(dir.join("club_main_2.light"))
            .unwrap()
            .contains("\"Club (Main)\""));
    }

    /// A broken file at a name's stem is the name's to replace, whatever it
    /// seems to hold: "Club Main" beside a broken `club_main.light` that
    /// seems to hold only "Club (Main)" is a 409 offering to replace it,
    /// listing what it may hold, and a confirmed save keeps it aside.
    #[tokio::test]
    async fn a_broken_stem_file_is_offered_for_replacing_whatever_it_seems_to_hold() {
        let (state, _dir) = test_state();
        let dir = _dir.path().join("v_broken_stem");
        std::fs::create_dir(&dir).unwrap();
        let broken = "venue \"Club (Main)\" {\n";
        std::fs::write(dir.join("club_main.light"), broken).unwrap();
        let uri = "/lighting/venues/Club%20Main?dir=v_broken_stem";
        let put = |query: &'static str| {
            fixture_type_request(
                state.clone(),
                "PUT",
                format!("{uri}{query}"),
                "text/plain",
                Body::from(sample_venue_dsl("Club Main")),
            )
        };
        let (status, parsed) = put("").await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert_eq!(parsed["broken_files"][0]["file"], "club_main.light");
        assert_eq!(
            parsed["broken_files"][0]["also_mentions"],
            json!(["venue Club (Main)"])
        );
        let (status, parsed) = put("&replace_broken=club_main.light").await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        let kept = parsed["kept"][0].as_str().unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(kept)).unwrap(), broken);
        assert!(std::fs::read_to_string(dir.join("club_main.light"))
            .unwrap()
            .contains("\"Club Main\""));
    }

    /// However a broken file confuses the block scan (an unclosed quote, a
    /// `\"` the grammar does not have, a name that is not quoted, a TODO
    /// naming a block keyword, a word like `show-par`), it is offered for
    /// replacing, and kept aside when replaced.
    #[tokio::test]
    async fn any_broken_file_is_offered_and_kept() {
        let (state, _dir) = test_state();
        for (n, text) in [
            "venue \"Club\" {\n  label: \"oops\n}\nvenue \"Arena\" {\n",
            "venue \"Club\" {\n  label: \"a \\\" b\"\n}\nvenue \"Arena\" {\n",
            "venue Club {\n",
            "venue \"Club\" {\n  // TODO: move this to the show\n",
            "venue \"Club\" {\n  fixture \"P\" show-par @ 1:1\n",
        ]
        .iter()
        .enumerate()
        {
            let rel = format!("v_scan_{n}");
            let dir = _dir.path().join(&rel);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("club.light"), text).unwrap();
            let send = |query: String| {
                fixture_type_request(
                    state.clone(),
                    "PUT",
                    format!("/lighting/venues/Club?dir={rel}{query}"),
                    "text/plain",
                    Body::from(sample_venue_dsl("Club")),
                )
            };
            let (status, parsed) = send(String::new()).await;
            assert_eq!(status, StatusCode::CONFLICT, "{text:?}: {parsed}");
            assert_eq!(parsed["broken"], true, "{text:?}: {parsed}");
            assert_eq!(parsed["broken_files"][0]["file"], "club.light", "{text:?}");
            let (status, parsed) = send("&replace_broken=club.light".to_string()).await;
            assert_eq!(status, StatusCode::OK, "{text:?}: {parsed}");
            let kept = parsed["kept"][0].as_str().unwrap();
            assert_eq!(std::fs::read_to_string(dir.join(kept)).unwrap(), *text);
        }
    }

    /// No file is written under a hidden name, which the loaders pass over
    /// (`project_files::files_under`): `venues/.club.light` is neither
    /// loaded nor written.
    #[test]
    fn a_hidden_file_name_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        assert!(lighting_write_path(dir.path(), &dir.path().join(".light")).is_err());
        assert!(lighting_write_path(dir.path(), &dir.path().join(".club.light")).is_err());
        assert!(lighting_write_path(dir.path(), &dir.path().join("a.light")).is_ok());
        std::fs::write(dir.path().join(".club.light"), sample_venue_dsl("Club")).unwrap();
        std::fs::write(dir.path().join("a.light"), sample_venue_dsl("A")).unwrap();
        assert_eq!(
            lighting::project_files::files_under(dir.path(), VENUE_EXTENSIONS).unwrap(),
            [dir.path().join("a.light")]
        );
    }

    /// A form already at the stem in another spelling is written in place,
    /// not beside it and then retired: on a filesystem that ignores case
    /// `Par.fixture` would be that very file, and the save would be lost.
    #[tokio::test]
    async fn put_fixture_type_writes_a_form_in_its_own_spelling() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_spelling");
        std::fs::create_dir(&ft_dir).unwrap();
        std::fs::write(ft_dir.join("Par.light"), sample_fixture_type_dsl("Par")).unwrap();
        std::fs::write(
            ft_dir.join("Par.FIXTURE"),
            sample_rich_fixture_type_dsl("Par"),
        )
        .unwrap();

        let edited = sample_rich_fixture_type_dsl("Par").replace("270", "180");
        let (status, parsed) = fixture_type_request(
            state,
            "PUT",
            "/lighting/fixture-types/Par?dir=ft_spelling&ext=fixture".to_string(),
            "text/plain",
            Body::from(edited.clone()),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        let names: Vec<String> = std::fs::read_dir(&ft_dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| !n.starts_with("Par.light.replaced-"))
            .collect();
        // The retired `.light` twin is set aside, kept.
        assert_eq!(names, ["Par.FIXTURE"]);
        assert!(!ft_dir.join("Par.light").exists());
        assert_eq!(
            std::fs::read_to_string(ft_dir.join("Par.FIXTURE")).unwrap(),
            edited
        );
    }

    /// The one rule for a file a save or delete acts on: only one that
    /// declares the name. A venue file at the stem (any case) that reads and
    /// holds another venue is not the venue's: a save writes a file of its
    /// own past it, a delete acts on that one, and the file at the stem is
    /// left as it is. A broken fixture type file at the stem is the type's,
    /// whatever it seems to hold: refused with 409 by the raw save, the form
    /// save and the delete (each offering to replace it), and left as it is.
    #[tokio::test]
    async fn saves_and_deletes_refuse_a_file_not_holding_the_name() {
        let (state, _dir) = test_state();
        let other = sample_venue_dsl("Other");
        let broken = "fixture_type \"Spot\" {\n  channels: 4\n";
        for (rel, file) in [
            ("v_other_upper", "Club.venue"),
            ("v_other_lower", "club.venue"),
        ] {
            std::fs::create_dir(_dir.path().join(rel)).unwrap();
            std::fs::write(_dir.path().join(rel).join(file), &other).unwrap();
            let form = serde_json::json!({
                "fixtures": [{"name": "A", "fixture_type": "Par", "universe": 1, "start_channel": 1}]
            });
            for (method, content_type, body) in [
                ("PUT", "text/plain", Body::from(sample_venue_dsl("Club"))),
                (
                    "PUT",
                    "application/json",
                    Body::from(serde_json::to_vec(&form).unwrap()),
                ),
                ("DELETE", "text/plain", Body::empty()),
            ] {
                let (status, parsed) = fixture_type_request(
                    state.clone(),
                    method,
                    format!("/lighting/venues/Club?dir={rel}"),
                    content_type,
                    body,
                )
                .await;
                assert_eq!(
                    status,
                    StatusCode::OK,
                    "{file} {method} {content_type}: {parsed}"
                );
            }
            assert_eq!(
                std::fs::read_to_string(_dir.path().join(rel).join(file)).unwrap(),
                other
            );
        }
        for (rel, file) in [
            ("ft_broken_upper", "Par.light"),
            ("ft_broken_lower", "par.light"),
        ] {
            std::fs::create_dir(_dir.path().join(rel)).unwrap();
            std::fs::write(_dir.path().join(rel).join(file), broken).unwrap();
            let form = serde_json::json!({"channels": {"red": 1}});
            for (method, content_type, body) in [
                (
                    "PUT",
                    "text/plain",
                    Body::from(sample_fixture_type_dsl("Par")),
                ),
                (
                    "PUT",
                    "application/json",
                    Body::from(serde_json::to_vec(&form).unwrap()),
                ),
                ("DELETE", "text/plain", Body::empty()),
            ] {
                let (status, parsed) = fixture_type_request(
                    state.clone(),
                    method,
                    format!("/lighting/fixture-types/Par?dir={rel}"),
                    content_type,
                    body,
                )
                .await;
                assert_eq!(
                    status,
                    StatusCode::CONFLICT,
                    "{file} {method} {content_type}: {parsed}"
                );
            }
            assert_eq!(
                std::fs::read_to_string(_dir.path().join(rel).join(file)).unwrap(),
                broken
            );
        }
    }

    /// A broken file is replaced (or removed) only when the request names
    /// exactly it (`replace_broken`), as the web UI does once the person
    /// confirms; never a file that reads and holds other names, nor when the
    /// flag names another file. The 409 carries the file and its parse error.
    #[tokio::test]
    async fn replace_broken_acts_on_exactly_the_named_broken_file() {
        let (state, _dir) = test_state();
        let broken = "fixture_type \"Par\" {\n  channels: 4\n";
        let request = |method: &'static str, uri: String, body: String| {
            let state = state.clone();
            async move { fixture_type_request(state, method, uri, "text/plain", Body::from(body)).await }
        };

        // Fixture types: a raw save over a broken file, then in another
        // form, then a delete.
        for (rel, ext) in [("ft_rb_light", "light"), ("ft_rb_fixture", "fixture")] {
            let dir = _dir.path().join(rel);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("Par.light"), broken).unwrap();
            let body = if ext == "light" {
                sample_fixture_type_dsl("Par")
            } else {
                sample_rich_fixture_type_dsl("Par")
            };
            let base = format!("/lighting/fixture-types/Par?dir={rel}&ext={ext}");

            let (status, parsed) = request("PUT", base.clone(), body.clone()).await;
            assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
            assert_eq!(parsed["file"], "Par.light");
            assert_eq!(parsed["broken"], true);
            assert!(parsed["parse_error"].is_string(), "{parsed}");
            let (status, _) = request(
                "PUT",
                format!("{base}&replace_broken=Other.light"),
                body.clone(),
            )
            .await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(
                std::fs::read_to_string(dir.join("Par.light")).unwrap(),
                broken
            );

            let (status, parsed) = request(
                "PUT",
                format!("{base}&replace_broken=Par.light"),
                body.clone(),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{ext}: {parsed}");
            // The broken file is kept aside, under a name no loader reads,
            // with its own bytes, and the reply names it.
            let mut names: Vec<String> = std::fs::read_dir(&dir)
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            assert_eq!(names.len(), 2, "{names:?}");
            assert_eq!(names[0], format!("Par.{ext}"), "{ext}");
            let backup = &names[1];
            assert!(backup.starts_with("Par.light.replaced-"), "{backup}");
            assert_eq!(std::fs::read_to_string(dir.join(backup)).unwrap(), broken);
            assert_eq!(parsed["kept"][0], *backup, "{parsed}");
            let loaded =
                lighting::project_files::files_under(&dir, FIXTURE_TYPE_EXTENSIONS).unwrap();
            assert_eq!(loaded, [dir.join(format!("Par.{ext}"))]);
        }
        let dir = _dir.path().join("ft_rb_delete");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("par.light"), broken.replace("\"Par\"", "\"par\"")).unwrap();
        let (status, _) = request(
            "DELETE",
            "/lighting/fixture-types/par?dir=ft_rb_delete".into(),
            String::new(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        let (status, parsed) = request(
            "DELETE",
            "/lighting/fixture-types/par?dir=ft_rb_delete&replace_broken=par.light".into(),
            String::new(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert!(!dir.join("par.light").exists());
        let kept = parsed["kept"][0].as_str().unwrap();
        assert!(kept.starts_with("par.light.replaced-"), "{parsed}");
        assert!(dir.join(kept).is_file());

        // A file that reads and holds another type is never replaced, flag
        // or no flag: the new type gets a file of its own past it.
        let dir = _dir.path().join("ft_rb_foreign");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("par.light"), sample_fixture_type_dsl("Spot")).unwrap();
        let (status, parsed) = request(
            "PUT",
            "/lighting/fixture-types/par?dir=ft_rb_foreign&replace_broken=par.light".into(),
            sample_fixture_type_dsl("par"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(
            std::fs::read_to_string(dir.join("par.light")).unwrap(),
            sample_fixture_type_dsl("Spot")
        );
        assert!(dir.join("par_2.light").is_file());

        // Venues: a save and a delete over a broken file; a foreign one.
        let dir = _dir.path().join("v_rb");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("club.light"), "venue \"Club\" {\n").unwrap();
        let (status, _) = request(
            "PUT",
            "/lighting/venues/Club?dir=v_rb".into(),
            sample_venue_dsl("Club"),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        let (status, parsed) = request(
            "PUT",
            "/lighting/venues/Club?dir=v_rb&replace_broken=club.light".into(),
            sample_venue_dsl("Club"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        std::fs::write(dir.join("club.light"), "venue \"Club\" {\n").unwrap();
        let (status, parsed) = request(
            "DELETE",
            "/lighting/venues/Club?dir=v_rb&replace_broken=club.light".into(),
            String::new(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert!(!dir.join("club.light").exists());
        let kept = parsed["kept"][0].as_str().unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join(kept)).unwrap(),
            "venue \"Club\" {\n"
        );
        // A broken file that seems to open another venue's block too is
        // offered for removing, listing what it may hold, and kept aside
        // with both blocks when removed.
        let both = "venue \"Club\" {\n}\nvenue \"Arena\" {\n";
        std::fs::write(dir.join("club.light"), both).unwrap();
        let (status, parsed) = request(
            "DELETE",
            "/lighting/venues/Club?dir=v_rb".into(),
            String::new(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert_eq!(parsed["broken"], true, "{parsed}");
        assert_eq!(
            parsed["broken_files"][0]["also_mentions"],
            json!(["venue Arena"])
        );
        assert!(
            parsed["error"]
                .as_str()
                .unwrap()
                .contains("would be removed"),
            "{parsed}"
        );
        let (status, parsed) = request(
            "DELETE",
            "/lighting/venues/Club?dir=v_rb&replace_broken=club.light".into(),
            String::new(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        let kept = parsed["kept"][0].as_str().unwrap();
        assert_eq!(std::fs::read_to_string(dir.join(kept)).unwrap(), both);
    }

    /// "Replace it" works for each shape a save comes in: a venue saved as
    /// JSON (as the web UI saves one) over a broken file writes the venue
    /// whole; two broken files a fixture type save acts on are named in one
    /// 409 and replaced together; and a broken file is named by its path in
    /// the directory, so its namesake in another subdirectory is not it.
    #[tokio::test]
    async fn replace_broken_covers_json_twins_and_subdirectories() {
        let (state, _dir) = test_state();
        let send = |method: &'static str, uri: String, content_type: &'static str, body: String| {
            let state = state.clone();
            async move { fixture_type_request(state, method, uri, content_type, Body::from(body)).await }
        };

        // A venue saved as JSON over a broken file.
        let dir = _dir.path().join("v_json");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("club.light"), "venue \"Club\" {\n").unwrap();
        let form = serde_json::json!({
            "fixtures": [{"name": "A", "fixture_type": "Par", "universe": 1, "start_channel": 1}]
        })
        .to_string();
        let (status, parsed) = send(
            "PUT",
            "/lighting/venues/Club?dir=v_json".into(),
            "application/json",
            form.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert_eq!(parsed["broken_files"][0]["file"], "club.light");
        let (status, parsed) = send(
            "PUT",
            "/lighting/venues/Club?dir=v_json&replace_broken=club.light".into(),
            "application/json",
            form,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        let text = std::fs::read_to_string(dir.join("club.light")).unwrap();
        assert!(text.contains("venue \"Club\""), "{text}");
        assert!(lighting::parser::parse_venues(&text).is_ok(), "{text}");

        // Two broken forms of one fixture type: both named, both replaced.
        let dir = _dir.path().join("ft_two_broken");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("Par.fixture"), "fixture_type \"Par\" {\n").unwrap();
        std::fs::write(
            dir.join("Par.light"),
            "fixture_type \"Par\" {\n  channels: 1\n",
        )
        .unwrap();
        let uri = "/lighting/fixture-types/Par?dir=ft_two_broken&ext=light";
        let (status, parsed) = send(
            "PUT",
            uri.into(),
            "text/plain",
            sample_fixture_type_dsl("Par"),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        let mut named: Vec<String> = parsed["broken_files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["file"].as_str().unwrap().to_string())
            .collect();
        named.sort();
        assert_eq!(named, ["Par.fixture", "Par.light"]);
        let (status, parsed) = send(
            "PUT",
            format!("{uri}&replace_broken=Par.fixture&replace_broken=Par.light"),
            "text/plain",
            sample_fixture_type_dsl("Par"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["kept"].as_array().map(Vec::len), Some(2), "{parsed}");
        assert_eq!(
            lighting::project_files::files_under(&dir, FIXTURE_TYPE_EXTENSIONS).unwrap(),
            [dir.join("Par.light")]
        );

        // Two broken namesakes in two subdirectories: both are the name's
        // candidates, named by their paths; a flag naming one, or a bare
        // basename, does not reach the other, and naming both replaces both.
        let dir = _dir.path().join("v_subdirs");
        for sub in ["a", "b"] {
            std::fs::create_dir_all(dir.join(sub)).unwrap();
            std::fs::write(dir.join(sub).join("club.light"), "venue \"Club\" {\n").unwrap();
        }
        let uri = "/lighting/venues/Club?dir=v_subdirs";
        let (status, parsed) =
            send("PUT", uri.into(), "text/plain", sample_venue_dsl("Club")).await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert_eq!(parsed["file"], "a/club.light");
        assert_eq!(parsed["broken_files"][1]["file"], "b/club.light");
        for (flag, refused) in [
            ("club.light", "a/club.light"),
            ("a/club.light", "b/club.light"),
            ("b/club.light", "a/club.light"),
        ] {
            let (status, parsed) = send(
                "PUT",
                format!("{uri}&replace_broken={flag}"),
                "text/plain",
                sample_venue_dsl("Club"),
            )
            .await;
            assert_eq!(status, StatusCode::CONFLICT, "{flag}");
            assert_eq!(parsed["file"], refused, "{flag}");
        }
        let (status, parsed) = send(
            "PUT",
            format!("{uri}&replace_broken=a/club.light&replace_broken=b/club.light"),
            "text/plain",
            sample_venue_dsl("Club"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(
            std::fs::read_to_string(dir.join("a/club.light")).unwrap(),
            sample_venue_dsl("Club")
        );
        assert!(!dir.join("b/club.light").exists());
        assert_eq!(parsed["kept"].as_array().map(Vec::len), Some(2), "{parsed}");

        // A broken record an older stem named is found by the name it opens.
        let dir = _dir.path().join("ft_old_stem");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(
            dir.join("robe_esprite.fixture"),
            "fixture_type \"Robe-Esprite\" {\n",
        )
        .unwrap();
        let (status, parsed) = send(
            "PUT",
            "/lighting/fixture-types/Robe-Esprite?dir=ft_old_stem&ext=fixture".into(),
            "text/plain",
            sample_rich_fixture_type_dsl("Robe-Esprite"),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
        assert_eq!(parsed["file"], "robe_esprite.fixture");
        assert!(!dir.join("robe-esprite.fixture").exists());

        // A broken file that seems to open blocks for other names too is
        // offered for replacing all the same, listing them, and kept aside
        // with every block when replaced: a venue's (`tour/all.light`: Club
        // and Arena), and a fixture type's (`show.light`: fixture type Par
        // and venue Club).
        for (rel, file, uri, body, also, text) in [
            (
                "v_multi",
                "tour/all.light",
                "/lighting/venues/Club",
                sample_venue_dsl("Club"),
                "venue Arena",
                "venue \"Club\" {\n}\nvenue \"Arena\" {\n",
            ),
            (
                "ft_multi",
                "show.light",
                "/lighting/fixture-types/Par",
                sample_fixture_type_dsl("Par"),
                "venue Club",
                "fixture_type \"Par\" {\n}\nvenue \"Club\" {\n",
            ),
        ] {
            let dir = _dir.path().join(rel);
            std::fs::create_dir_all(dir.join(file).parent().unwrap()).unwrap();
            std::fs::write(dir.join(file), text).unwrap();
            let (status, parsed) = send(
                "PUT",
                format!("{uri}?dir={rel}"),
                "text/plain",
                body.clone(),
            )
            .await;
            assert_eq!(status, StatusCode::CONFLICT, "{parsed}");
            assert_eq!(parsed["broken_files"][0]["file"], file, "{parsed}");
            assert_eq!(parsed["broken_files"][0]["also_mentions"], json!([also]));
            let (status, parsed) = send(
                "PUT",
                format!("{uri}?dir={rel}&replace_broken={file}"),
                "text/plain",
                body,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{parsed}");
            let kept = parsed["kept"][0].as_str().unwrap();
            assert_eq!(std::fs::read_to_string(dir.join(kept)).unwrap(), text);
        }

        // A file named with a comma is named back as written.
        let dir = _dir.path().join("v_comma");
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("a,b.light"), "venue \"A,B\" {\n").unwrap();
        let (status, parsed) = send(
            "PUT",
            "/lighting/venues/A,B?dir=v_comma&replace_broken=a%2Cb.light".into(),
            "text/plain",
            sample_venue_dsl("A,B"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
    }

    /// A save does not write through a link to nothing, which would create
    /// the file it names, outside the project or not: a new file is named
    /// past it (`project_files::new_file_stem` takes any entry as taken),
    /// and a write to one is refused by the one containment rule MCP's
    /// tools take.
    #[cfg(unix)]
    #[tokio::test]
    async fn saves_do_not_write_through_a_link_to_nothing() {
        let (state, _dir) = test_state();
        let outside = tempfile::tempdir().unwrap();
        for (rel, file) in [("ft_dead", "par.light"), ("v_dead", "club.light")] {
            let dir = _dir.path().join(rel);
            std::fs::create_dir(&dir).unwrap();
            std::os::unix::fs::symlink(outside.path().join(file), dir.join(file)).unwrap();
        }

        let (status, parsed) = fixture_type_request(
            state.clone(),
            "PUT",
            "/lighting/fixture-types/par?dir=ft_dead".to_string(),
            "text/plain",
            Body::from(sample_fixture_type_dsl("par")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert!(_dir.path().join("ft_dead/par_2.light").is_file());

        let (status, parsed) = fixture_type_request(
            state,
            "PUT",
            "/lighting/venues/club?dir=v_dead".to_string(),
            "text/plain",
            Body::from(sample_venue_dsl("club")),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert!(_dir.path().join("v_dead/club_2.light").is_file());
        assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);

        let dir = _dir.path().join("v_dead");
        assert!(lighting_write_path(&dir, &dir.join("club.light")).is_err());
    }

    #[tokio::test]
    async fn put_venue_json_with_geometry_lands_in_a_venue_file() {
        let (state, _dir) = test_state();
        let rel = "v_put_geometry";
        let app = router().with_state(state);

        // First a plain venue: it is a .light.
        let plain = serde_json::json!({
            "fixtures": [{"name": "Spot1", "fixture_type": "GenericPar", "universe": 1, "start_channel": 1}]
        });
        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(format!("/lighting/venues/Geo?dir={}", rel))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&plain).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(_dir.path().join(rel).join("geo.light").exists());

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
        let plain = serde_json::json!({
            "fixtures": [{"name": "B1", "fixture_type": "Brick", "universe": 1, "start_channel": 1}]
        });
        let uri = format!("/lighting/venues/Rig?dir={rel}");
        let status = put_venue_body(
            state.clone(),
            uri.clone(),
            "application/json",
            serde_json::to_vec(&plain).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(_dir.path().join(rel).join("rig.light").exists());

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

        let dead_port = crate::testutil::net::free_tcp_port();
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
    async fn a_broken_file_is_refused_and_left_byte_identical() {
        let (state, dir) = test_state();
        let d = venues_dir(&dir);
        let broken = "// mine\nvenue \"House\" {\n  fixture \"A\" brick @ !!!\n}\n";
        std::fs::write(d.join("rig.venue"), broken).unwrap();
        let app = router().with_state(state);

        let (status, body) = put(&app, None, body_for(1.0)).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let msg = body["error"].as_str().unwrap();
        assert!(
            msg.contains("rig.venue") && msg.contains("by hand"),
            "{msg}"
        );
        assert_eq!(
            std::fs::read_to_string(d.join("rig.venue")).unwrap(),
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
