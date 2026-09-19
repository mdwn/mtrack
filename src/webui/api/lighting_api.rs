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

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
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
fn project_root(
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
) -> Result<(String, Vec<u8>), axum::response::Response> {
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
        return Ok((filename, bytes.to_vec()));
    }
    Err((
        StatusCode::BAD_REQUEST,
        Json(json!({"error": "No file uploaded"})),
    )
        .into_response())
}

/// POST /api/lighting/gdtf/inspect — parses an uploaded GDTF archive and
/// returns its modes, for the mode picker. Writes nothing.
pub(super) async fn inspect_gdtf(mut multipart: axum::extract::Multipart) -> impl IntoResponse {
    let (_, bytes) = first_multipart_file(&mut multipart).await?;
    let description = lighting::gdtf::parse_archive(&bytes).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Not a parseable GDTF: {}", e)})),
        )
            .into_response()
    })?;
    let modes: Vec<serde_json::Value> = lighting::gdtf::mode_summaries(&description)
        .into_iter()
        .map(|summary| {
            json!({
                "name": summary.name,
                "channel_count": summary.channel_count,
                "footprint": summary.footprint,
            })
        })
        .collect();
    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(json!({
                "fixture": description.name,
                "manufacturer": description.manufacturer,
                "modes": modes,
            })),
        )
            .into_response(),
    )
}

/// Query parameters for the GDTF import endpoint.
#[derive(serde::Deserialize)]
pub(super) struct GdtfImportQuery {
    mode: String,
    name: Option<String>,
}

/// POST /api/lighting/gdtf/import?mode=...&name=... — imports one mode of an
/// uploaded GDTF archive through the shared importer (the same one behind
/// the CLI and MCP): archive into lighting/library/, a referential .fixture
/// definition, and a warmed expansion cache. Returns the import report.
pub(super) async fn import_gdtf(
    State(state): State<WebUiState>,
    Query(query): Query<GdtfImportQuery>,
    mut multipart: axum::extract::Multipart,
) -> impl IntoResponse {
    let (filename, bytes) = first_multipart_file(&mut multipart).await?;
    let project = project_root(&state.config_path)?;
    // The inner Result survives spawn_blocking_io so an import refusal (bad
    // mode, name collision, archive conflict) surfaces as the caller error
    // it is, not a 500.
    let report = super::helpers::spawn_blocking_io("import gdtf", move || {
        Ok::<_, String>(
            lighting::import::import_gdtf_bytes(
                &bytes,
                &filename,
                &query.mode,
                query.name.as_deref(),
                &project,
                DEFAULT_FIXTURE_TYPES_DIR,
            )
            .map_err(|e| e.to_string()),
        )
    })
    .await?
    .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e}))).into_response())?;
    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(serde_json::to_value(&report).unwrap_or_default()),
        )
            .into_response(),
    )
}

/// GET /api/lighting — lists available .light files from the songs directory.
pub(super) async fn get_lighting_files(State(state): State<WebUiState>) -> impl IntoResponse {
    let songs_path = state.songs_path.clone();
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
    light_files.sort_by(|a, b| {
        a.get("path")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .cmp(b.get("path").and_then(|v| v.as_str()).unwrap_or(""))
    });
    (StatusCode::OK, Json(json!({"files": light_files}))).into_response()
}

/// Recursively finds .light files under a directory.
pub(crate) fn find_light_files(
    base: &std::path::Path,
    dir: &std::path::Path,
    results: &mut Vec<serde_json::Value>,
) -> Result<(), std::io::Error> {
    // codeql[rust/path-injection] dir is always state.songs_path, set at startup.
    if !dir.is_dir() {
        return Ok(());
    }
    // codeql[rust/path-injection] dir is always state.songs_path, set at startup.
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            find_light_files(base, &path, results)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("light") {
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
            results.push(json!({
                "name": name,
                "path": relative,
            }));
        }
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

    let root = match VerifiedRoot::new(&state.songs_path) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    let safe = match SafePath::resolve(&state.songs_path.join(&name), &root) {
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
        std::fs::read_to_string(&path)
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

/// PUT /api/lighting/:name — validates and atomically writes a lighting DSL file.
pub(super) async fn put_lighting_file(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    body: String,
) -> impl IntoResponse {
    use super::super::safe_path::{SafePath, SafePathError, VerifiedRoot};

    let root = match VerifiedRoot::new(&state.songs_path) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };

    // Resolve the file path under the songs root. For existing files, SafePath::resolve
    // canonicalizes and verifies containment. For new files, resolve the parent directory
    // and join the filename to it.
    let candidate = root.as_path().join(&name);
    let verified_path = match SafePath::resolve(&candidate, &root) {
        Ok(p) => p.as_path().to_path_buf(),
        Err(_) => {
            // File doesn't exist — resolve the parent and join the filename.
            let (parent, filename) = match (candidate.parent(), candidate.file_name()) {
                (Some(p), Some(f)) => (p, f),
                _ => return SafePathError::InvalidName.into_response(),
            };
            let safe_parent = match SafePath::resolve(parent, &root) {
                Ok(p) => p,
                Err(e) => return e.into_response(),
            };
            safe_parent.as_path().join(filename)
        }
    };

    // Validate against the tempo the owning song loads this file with. A show
    // inheriting its song's tempo is valid content that a tempo-less parse
    // rejects, so writing it through the UI would fail for no real reason.
    let tempo = song_tempo_for_path(&state, &verified_path);
    if let Err(errors) = config_io::validate_light_show(&body, tempo.as_ref()) {
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

    let root = match VerifiedRoot::new(&state.songs_path) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };
    let safe = match SafePath::resolve(&state.songs_path.join(&name), &root) {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Lighting file not found: {}", name)})),
            )
                .into_response();
        }
    };

    if safe.as_path().extension().and_then(|e| e.to_str()) != Some("light") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Can only delete .light files"})),
        )
            .into_response();
    }

    // codeql[rust/path-injection] safe is verified via SafePath::resolve against songs_path root.
    let path = safe.as_path().to_path_buf();
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
    let tempo = query
        .song
        .as_deref()
        .and_then(|name| state.player.songs().get(name).ok())
        .and_then(|song| song.lighting_tempo_map());
    match config_io::validate_light_show(&body, tempo.as_ref()) {
        Ok(()) => (StatusCode::OK, Json(json!({"valid": true}))).into_response(),
        Err(errors) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"valid": false, "errors": errors})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Fixture Type & Venue CRUD endpoints
// ---------------------------------------------------------------------------

/// Default directory for fixture type definitions, relative to project root.
const DEFAULT_FIXTURE_TYPES_DIR: &str = "lighting/fixture_types";

/// Default directory for venue definitions, relative to project root.
const DEFAULT_VENUES_DIR: &str = "lighting/venues";

/// Fixture type files: `.fixture` (rich channels, GDTF-referential types) and
/// the v1 `.light`, loaded as peers — the same pair the lighting system reads.
const FIXTURE_TYPE_EXTENSIONS: &[&str] = &["light", "fixture"];

/// The file a fixture type of this name lives in, if one exists: `.fixture`
/// first, then `.light`.
fn existing_fixture_type_file(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let stem = sanitize_filename(name);
    ["fixture", "light"]
        .iter()
        .map(|extension| dir.join(format!("{stem}.{extension}")))
        .find(|path| path.is_file())
}

/// Venue files: `.venue` (positions, focus points, MVR provenance) and the
/// v1 `.light`, loaded as peers.
const VENUE_EXTENSIONS: &[&str] = &["light", "venue"];

/// The file a venue of this name lives in, if one exists: `.venue` first,
/// then `.light`.
fn existing_venue_file(dir: &std::path::Path, name: &str) -> Option<std::path::PathBuf> {
    let stem = sanitize_filename(name);
    ["venue", "light"]
        .iter()
        .map(|extension| dir.join(format!("{stem}.{extension}")))
        .find(|path| path.is_file())
}

/// Whether a venue uses syntax only `.venue` files carry.
fn needs_venue_extension(venue: &lighting::types::Venue) -> bool {
    venue.source().is_some()
        || !venue.focus_points().is_empty()
        || venue
            .fixtures()
            .values()
            .any(|f| f.position().is_some() || f.rotation().is_some())
}

/// Resolves a lighting directory path relative to the project root.
/// Uses the provided override (from query param) or falls back to the default.
/// Returns an error response if the project root cannot be canonicalized or the
/// resolved path would escape it.
#[allow(clippy::result_large_err)]
fn resolve_lighting_dir(
    config_path: &std::path::Path,
    override_dir: Option<&str>,
    default: &str,
) -> Result<std::path::PathBuf, axum::response::Response> {
    use super::super::safe_path::{SafePath, VerifiedRoot};

    let root = VerifiedRoot::new(&project_root(config_path)?).map_err(|e| e.into_response())?;

    let relative = match override_dir {
        Some(d) if !d.is_empty() => d,
        _ => default,
    };

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
    let canonical_root = root.canonicalize().map_err(|_| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": "Failed to resolve project root"})),
        )
            .into_response()
    })?;
    for (name, fixture_type) in types {
        let Some(source) = fixture_type.source() else {
            continue;
        };
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
        if !canonical.starts_with(&canonical_root) {
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
    }
    Ok(())
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
}

/// Validates that a fixture type or venue name is safe for use as a filename.
#[allow(clippy::result_large_err)]
fn validate_lighting_name(name: &str) -> Result<(), axum::response::Response> {
    use super::super::safe_path::SafePath;
    if SafePath::validate_name(name).is_err() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Invalid name"})),
        )
            .into_response());
    }
    Ok(())
}

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

    let content_type = std::path::Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .and_then(|ext| {
            ASSET_TYPES
                .iter()
                .find(|(known, _)| known.eq_ignore_ascii_case(ext))
        })
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
    if !dir.is_dir() {
        return Ok::<_, axum::response::Response>(
            (StatusCode::OK, Json(json!({"fixture_types": {}}))).into_response(),
        );
    }
    let all = super::helpers::spawn_blocking_io("load fixture types", move || {
        let mut all = std::collections::HashMap::new();
        // Which file each name came from, so a name claimed twice can name
        // both files rather than one silently winning.
        let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut duplicates: Vec<FileError> = Vec::new();
        // Both extensions, as the lighting system loads them. A type's file
        // travels with it: the form a type is in decides how it may be
        // edited, and a referential type parsed from a file has no expanded
        // channels to show — only the system's expansion has those.
        let mut errors =
            load_light_files_from_dir(&dir, FIXTURE_TYPE_EXTENSIONS, |content, path| {
                let types = lighting::parser::parse_fixture_types(content)?;
                let file = crate::util::filename_display(path).to_string();
                let extension = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or_default()
                    .to_string();
                for (name, fixture_type) in types {
                    // Last-wins would show one file and hide the other, while the
                    // lighting system registers the name twice. Report both files
                    // and keep the first; the rest of this file still lists.
                    if let Some(previous) = seen.get(&name) {
                        duplicates.push(FileError {
                            file: file.clone(),
                            error: format!(
                                "fixture type \"{name}\" is defined in both {previous} and {file}"
                            ),
                        });
                        continue;
                    }
                    seen.insert(name.clone(), file.clone());
                    all.insert(
                        name,
                        json!({
                            "referential": fixture_type.source().is_some(),
                            "rich": fixture_type.uses_rich_channels(),
                            "fixture_type": fixture_type,
                            "file": file,
                            "extension": extension,
                        }),
                    );
                }
                Ok(())
            })
            .map_err(|e| e.to_string())?;
        errors.append(&mut duplicates);
        Ok::<_, String>((all, errors))
    })
    .await?;
    let (all, errors) = all;
    Ok((
        StatusCode::OK,
        Json(json!({"fixture_types": all, "errors": errors})),
    )
        .into_response())
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
    let Some(file_path) = existing_fixture_type_file(&dir, &name) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Fixture type not found: {}", name)})),
        )
            .into_response());
    };
    let fp = file_path.clone();
    let content = super::helpers::spawn_blocking_io("read fixture type", move || {
        std::fs::read_to_string(&fp)
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
            Json(json!({
                "referential": ft.source().is_some(),
                "rich": ft.uses_rich_channels(),
                "fixture_type": ft,
                "dsl": content,
                "file": crate::util::filename_display(&file_path),
                "extension": file_path.extension().and_then(|e| e.to_str()).unwrap_or_default(),
            })),
        )
            .into_response()),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Fixture type '{}' not found in file", name)})),
        )
            .into_response()),
    }
}

/// PUT /api/lighting/fixture-types/:name — creates or updates a fixture type.
///
/// Accepts either JSON (structured) or plain text (raw DSL).
pub(super) async fn put_fixture_type(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
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

    let stem = sanitize_filename(&name);
    // Read against the resolved directory rather than the created one: every
    // refusal below should land before anything is made on disk.
    let existing_extension = existing_fixture_type_file(&dir, &name).and_then(|path| {
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_string)
    });

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
                     lighting/fixture_types/{}.fixture by hand or through `mtrack import-gdtf`",
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
    let dir = super::helpers::ensure_configured_dir(&dir, &state).await?;

    // A type is one file: whichever form it is saved in, the other one is
    // retired, or the loader would register the name twice.
    let file_path = dir.join(format!("{stem}.{extension}"));
    let stale_twin = FIXTURE_TYPE_EXTENSIONS
        .iter()
        .map(|extension| dir.join(format!("{stem}.{extension}")))
        .find(|path| path != &file_path && path.is_file());
    let fp = file_path;
    let dsl_owned = dsl;
    super::helpers::spawn_blocking_io("write fixture type", move || {
        config_io::staged_write(&fp, &dsl_owned)?;
        if let Some(twin) = stale_twin {
            // Best effort: the save is durable already, and a leftover twin
            // surfaces as a duplicate fixture type name at load.
            if let Err(e) = std::fs::remove_file(&twin) {
                tracing::warn!(file = %twin.display(), error = %e, "could not retire the fixture type's stale twin");
            }
        }
        Ok::<(), String>(())
    })
    .await?;

    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(json!({"status": "saved", "name": name})),
        )
            .into_response(),
    )
}

/// DELETE /api/lighting/fixture-types/:name — deletes a fixture type file.
pub(super) async fn delete_fixture_type(
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
    let Some(file_path) = existing_fixture_type_file(&dir, &name) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Fixture type not found: {}", name)})),
        )
            .into_response());
    };
    let fp = file_path;
    super::helpers::spawn_blocking_io("delete fixture type", move || std::fs::remove_file(&fp))
        .await?;
    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(json!({"status": "deleted", "name": name})),
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
        let mut out: Vec<serde_json::Value> = names
            .into_iter()
            .map(|name| {
                let fixtures = guard.resolve_logical_group_graceful(&name);
                json!({"name": name, "fixtures": fixtures})
            })
            .collect();
        out.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        out
    })
    .await;

    match groups {
        Ok(groups) => Json(json!({"groups": groups})).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to resolve groups: {e}")})),
        )
            .into_response(),
    }
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

    let groups: Vec<serde_json::Value> = names
        .into_iter()
        .map(|name| json!({"name": name, "fixtures": []}))
        .collect();
    Json(json!({"groups": groups, "resolved": false})).into_response()
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
            (StatusCode::OK, Json(json!({"venues": {}}))).into_response(),
        );
    }
    let all = super::helpers::spawn_blocking_io("load venues", move || {
        let mut all = std::collections::HashMap::new();
        let errors = load_light_files_from_dir(&dir, VENUE_EXTENSIONS, |content, _path| {
            match lighting::parser::parse_venues(content) {
                Ok(venues) => {
                    all.extend(venues);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        })
        .map_err(|e| e.to_string())?;
        Ok::<_, String>((all, errors))
    })
    .await?;
    let (all, errors) = all;
    Ok((
        StatusCode::OK,
        Json(json!({"venues": all, "errors": errors})),
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
    let Some(file_path) = existing_venue_file(&dir, &name) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Venue not found: {}", name)})),
        )
            .into_response());
    };
    let fp = file_path.clone();
    let content =
        super::helpers::spawn_blocking_io("read venue file", move || std::fs::read_to_string(&fp))
            .await?;
    let venues = lighting::parser::parse_venues(&content).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to parse venue: {}", e)})),
        )
            .into_response()
    })?;
    match venues.get(&name) {
        Some(v) => Ok((StatusCode::OK, Json(json!({"venue": v, "dsl": content}))).into_response()),
        None => Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Venue '{}' not found in file", name)})),
        )
            .into_response()),
    }
}

/// PUT /api/lighting/venues/:name — creates or updates a venue.
pub(super) async fn put_venue(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let dir = resolve_lighting_dir(&state.config_path, query.dir.as_deref(), DEFAULT_VENUES_DIR)?;

    let content_type = headers
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let dsl = if content_type.contains("application/json") {
        let json_body: serde_json::Value = serde_json::from_slice(&body).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Invalid JSON: {}", e)})),
            )
                .into_response()
        })?;
        venue_json_to_dsl(&name, &json_body)
            .map_err(|e| (StatusCode::BAD_REQUEST, Json(json!({"error": e}))).into_response())?
    } else {
        String::from_utf8(body.to_vec()).map_err(|_| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Invalid UTF-8"})),
            )
                .into_response()
        })?
    };

    // Validate the DSL parses correctly
    let venues = lighting::parser::parse_venues(&dsl).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Invalid venue DSL: {}", e)})),
        )
            .into_response()
    })?;
    require_declared_name("venue", &name, &venues)?;
    let typed = venues.values().any(needs_venue_extension);

    let dir = super::helpers::ensure_configured_dir(&dir, &state).await?;
    // The extension is the version marker: a venue using positions, focus
    // points or MVR provenance is a `.venue`; anything else stays `.light`.
    // An existing file keeps its extension unless the content outgrows it,
    // in which case the `.light` twin is retired after the durable write.
    let stem = sanitize_filename(&name);
    let existing = existing_venue_file(&dir, &name);
    let file_path = match &existing {
        Some(path) if !typed || path.extension().is_some_and(|e| e == "venue") => path.clone(),
        _ => dir.join(format!("{stem}.{}", if typed { "venue" } else { "light" })),
    };
    let stale_twin = existing.filter(|path| path != &file_path);
    let fp = file_path;
    let dsl_owned = dsl;
    super::helpers::spawn_blocking_io("write venue", move || {
        config_io::staged_write(&fp, &dsl_owned)?;
        if let Some(twin) = stale_twin {
            // Best effort: the save is durable already, and a leftover
            // `.light` twin surfaces as a duplicate venue name at load.
            if let Err(e) = std::fs::remove_file(&twin) {
                tracing::warn!(file = %twin.display(), error = %e, "could not retire the venue's .light twin");
            }
        }
        Ok::<(), String>(())
    })
    .await?;

    let reloaded = reload_if_current_venue(&state, &name).await;

    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(json!({"status": "saved", "name": name, "reloaded": reloaded})),
        )
            .into_response(),
    )
}

/// After a venue file changed on disk: if it is the venue the running engine
/// is playing against, re-read it and push fresh stage metadata to every
/// web client. Returns whether that happened. A reload failure is logged,
/// not returned — the save itself is durable and the loader will say the
/// same thing at next startup.
async fn reload_if_current_venue(state: &WebUiState, name: &str) -> bool {
    let is_current = state
        .player
        .broadcast_handles()
        .and_then(|h| h.lighting_system)
        .is_some_and(|system| system.lock().current_venue() == Some(name));
    if !is_current {
        return false;
    }
    let player = state.player.clone();
    match tokio::task::spawn_blocking(move || player.reload_current_venue()).await {
        Ok(Ok(())) => true,
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
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let dir = resolve_lighting_dir(&state.config_path, query.dir.as_deref(), DEFAULT_VENUES_DIR)?;
    let Some(file_path) = existing_venue_file(&dir, &name) else {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Venue not found: {}", name)})),
        )
            .into_response());
    };
    let fp = file_path;
    super::helpers::spawn_blocking_io("delete venue", move || std::fs::remove_file(&fp)).await?;
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

/// Reads every file of the given extensions from a directory, calling the
/// processor with each one's content and path.
fn load_light_files_from_dir(
    dir: &std::path::Path,
    extensions: &[&str],
    mut processor: impl FnMut(&str, &std::path::Path) -> Result<(), Box<dyn std::error::Error>>,
) -> Result<Vec<FileError>, Box<dyn std::error::Error>> {
    let mut errors = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let extension = path.extension().and_then(|e| e.to_str());
        if path.is_file() && extension.is_some_and(|e| extensions.contains(&e)) {
            let content = std::fs::read_to_string(&path)?;
            // Per-file, not fatal: a directory is a set of independent files,
            // and one that no longer parses must not hide the rest. The caller
            // reports them so the UI can say which file and why.
            if let Err(e) = processor(&content, &path) {
                errors.push(FileError {
                    file: crate::util::filename_display(&path).to_string(),
                    error: e.to_string(),
                });
            }
        }
    }
    Ok(errors)
}

/// A file in a lighting directory that could not be parsed, reported alongside
/// the ones that could.
#[derive(serde::Serialize)]
struct FileError {
    file: String,
    error: String,
}

/// Converts a name to a safe filename (lowercase, spaces to underscores).
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            ' ' => '_',
            c if c.is_alphanumeric() || c == '_' || c == '-' => c,
            _ => '_',
        })
        .collect::<String>()
        .to_lowercase()
}

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
    for (i, (ch_name, ch_offset)) in entries.iter().enumerate() {
        let offset = ch_offset
            .as_u64()
            .ok_or(format!("Channel '{}' offset must be a number", ch_name))?;
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

/// Converts a JSON venue definition to DSL format.
fn venue_json_to_dsl(name: &str, json: &serde_json::Value) -> Result<String, String> {
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

    let mut by_name = std::collections::HashMap::new();
    for fix in fixtures {
        let fix_name = fix
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or("Fixture missing 'name'")?;
        let fix_type = fix
            .get("fixture_type")
            .and_then(|v| v.as_str())
            .ok_or("Fixture missing 'fixture_type'")?;
        let universe = fix
            .get("universe")
            .and_then(|v| v.as_u64())
            .ok_or("Fixture missing 'universe'")?;
        let start_channel = fix
            .get("start_channel")
            .and_then(|v| v.as_u64())
            .ok_or("Fixture missing 'start_channel'")?;
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
            u16::try_from(universe).map_err(|_| "Fixture 'universe' out of range")?,
            u16::try_from(start_channel).map_err(|_| "Fixture 'start_channel' out of range")?,
            tags,
        )
        .with_position(optional_vec3(fix, "position")?)
        .with_rotation(optional_vec3(fix, "rotation")?);
        if by_name.insert(fix_name.to_string(), fixture).is_some() {
            return Err(format!("Fixture '{fix_name}' listed more than once"));
        }
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

    let venue = Venue::new(name.to_string(), by_name)
        .with_focus_points(focus_points)
        .with_source(source);
    Ok(format!("{venue}\n"))
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
        std::fs::write(state.songs_path.join("show.light"), "content").unwrap();
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
        std::fs::write(state.songs_path.join("z_show.light"), "content").unwrap();
        std::fs::write(state.songs_path.join("a_show.light"), "content").unwrap();
        std::fs::write(state.songs_path.join("m_show.light"), "content").unwrap();
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

    #[tokio::test]
    async fn get_lighting_file_success() {
        let (state, _dir) = test_state();
        std::fs::write(state.songs_path.join("show.light"), "light content").unwrap();
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
        std::os::unix::fs::symlink(&secret_file, state.songs_path.join("evil.light")).unwrap();

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
        let file = state.songs_path.join("unreadable.light");
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
    async fn put_lighting_file_valid() {
        let (state, _dir) = test_state();
        let file_path = state.songs_path.join("new.light");
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
    async fn get_lighting_files_missing_dir() {
        let (mut state, _dir) = test_state();
        state.songs_path = std::path::PathBuf::from("/nonexistent/songs");
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
        std::os::unix::fs::symlink(outside_dir.path(), state.songs_path.join("escape")).unwrap();

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
        let paths: Vec<&str> = results
            .iter()
            .map(|r| r["path"].as_str().unwrap())
            .collect();
        assert!(paths.contains(&"song1/show.light"));
        assert!(paths.contains(&"top.light"));
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
        assert_eq!(results[0]["name"].as_str().unwrap(), "my_show");
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
        assert_eq!(results[0]["path"].as_str().unwrap(), "a/b/c/deep.light");
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
        assert_eq!(results[0]["name"].as_str().unwrap(), "show");
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
        let file = state.songs_path.join("broken.light");
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
        let sub = state.songs_path.join("readonly");
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

        let (state, _dir) = test_state();
        let sub = state.songs_path.join("unreadable_dir");
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

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["error"]
            .as_str()
            .unwrap()
            .contains("Failed to scan for lighting files"));
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
        format!(
            "fixture_type \"{name}\" from gdtf(\"library/synth.gdtf\", mode \"8: RGBS\") {{\n}}\n"
        )
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
        assert_eq!(sanitize_filename("hello world"), "hello_world");
        assert_eq!(sanitize_filename("My-Fixture_01"), "my-fixture_01");
        assert_eq!(sanitize_filename("a/b\\c.d!e"), "a_b_c_d_e");
        assert_eq!(sanitize_filename("UPPER"), "upper");
        assert_eq!(sanitize_filename(""), "");
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
    // Unit tests: load_light_files_from_dir
    // -----------------------------------------------------------------------

    #[test]
    fn load_light_files_from_dir_processes_light_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.light"), sample_fixture_type_dsl("TypeA")).unwrap();
        std::fs::write(dir.path().join("b.txt"), "not a light file").unwrap();

        let mut count = 0;
        load_light_files_from_dir(dir.path(), &["light"], |_content, _path| {
            count += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn load_light_files_from_dir_empty() {
        let dir = tempfile::tempdir().unwrap();
        let mut count = 0;
        load_light_files_from_dir(dir.path(), &["light"], |_content, _path| {
            count += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(count, 0);
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
    async fn inspect_gdtf_lists_modes_and_writes_nothing() {
        let (state, dir) = test_state();
        let app = router().with_state(state);
        let (content_type, body) = multipart_body("synth.gdtf", &synthetic_gdtf_bytes());

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/lighting/gdtf/inspect")
                    .header("content-type", content_type)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let parsed: serde_json::Value =
            serde_json::from_str(&response_body(response).await).unwrap();
        assert_eq!(parsed["fixture"], "Synth Brick");
        assert!(
            parsed["modes"]
                .as_array()
                .unwrap()
                .iter()
                .any(|m| m["name"] == "8: RGBS"),
            "{parsed}"
        );
        assert!(
            !dir.path().join("lighting").exists(),
            "inspect must not write"
        );
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
                    .uri("/lighting/gdtf/import?mode=8%3A%20RGBS&name=Brick")
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

        // A bad mode is the caller's error, not a server fault.
        let (content_type, body) = multipart_body("synth.gdtf", &synthetic_gdtf_bytes());
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/lighting/gdtf/import?mode=Nope")
                    .header("content-type", content_type)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let parsed: serde_json::Value =
            serde_json::from_str(&response_body(response).await).unwrap();
        assert!(
            parsed["error"]
                .as_str()
                .unwrap()
                .contains("no mode matching"),
            "{parsed}"
        );
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
    async fn put_fixture_type_refuses_a_gdtf_archive_outside_the_project() {
        let (state, _dir) = test_state();
        let rel = "ft_escaping_archive";
        let app = router().with_state(state);
        let dsl = "fixture_type \"Brick\" from gdtf(\"../../etc/passwd\", mode \"8: RGBS\") {\n}\n";

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
}
