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

/// GET /api/lighting — lists available .light files from the songs directory.
pub(super) async fn get_lighting_files(State(state): State<WebUiState>) -> impl IntoResponse {
    let mut light_files: Vec<serde_json::Value> = Vec::new();
    if let Err(e) = find_light_files(&state.songs_path, &state.songs_path, &mut light_files) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to scan for lighting files: {}", e)})),
        )
            .into_response();
    }
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
    // Prevent path traversal
    if name.contains("..") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Invalid path"})),
        )
            .into_response();
    }

    let file_path = state.songs_path.join(&name);

    // Ensure the resolved path is still under songs_path
    match file_path.canonicalize() {
        Ok(canonical) => {
            let base = match state.songs_path.canonicalize() {
                Ok(b) => b,
                Err(e) => {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"error": format!("Failed to resolve base path: {}", e)})),
                    )
                        .into_response();
                }
            };
            if !canonical.starts_with(&base) {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "Path outside allowed directory"})),
                )
                    .into_response();
            }
        }
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Lighting file not found: {}", name)})),
            )
                .into_response();
        }
    }

    // codeql[rust/path-injection] file_path is validated via canonicalize + starts_with above.
    match std::fs::read_to_string(&file_path) {
        Ok(content) => (
            StatusCode::OK,
            [("content-type", "text/plain; charset=utf-8")],
            content,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to read lighting file: {}", e)})),
        )
            .into_response(),
    }
}

/// PUT /api/lighting/:name — validates and atomically writes a lighting DSL file.
pub(super) async fn put_lighting_file(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    body: String,
) -> impl IntoResponse {
    if name.contains("..") {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Invalid path"})),
        )
            .into_response();
    }

    let file_path = state.songs_path.join(&name);

    // Validate path is within songs directory
    // For new files the parent must exist and be within the base
    let parent = match file_path.parent() {
        Some(p) => p,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Invalid file path"})),
            )
                .into_response();
        }
    };
    if let Err(e) = config_io::validate_path_within(&state.songs_path, parent) {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": e}))).into_response();
    }

    // Validate the DSL content
    if let Err(errors) = config_io::validate_light_show(&body) {
        return (StatusCode::BAD_REQUEST, Json(json!({"errors": errors}))).into_response();
    }

    match config_io::atomic_write(&file_path, &body) {
        Ok(()) => (StatusCode::OK, Json(json!({"status": "saved"}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response(),
    }
}

/// POST /api/lighting/validate — validates lighting DSL content without saving.
pub(super) async fn validate_lighting(body: String) -> impl IntoResponse {
    match config_io::validate_light_show(&body) {
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
    let project_root = config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    // Canonicalize the project root so all derived paths are anchored to a
    // verified absolute base, preventing path traversal via the dir override.
    let root_canonical = project_root.canonicalize().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to resolve project root: {}", e)})),
        )
            .into_response()
    })?;
    let relative = match override_dir {
        Some(d) if !d.is_empty() => d,
        _ => default,
    };
    // Reject absolute overrides — directories must be relative to the project root.
    if std::path::Path::new(relative).is_absolute() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Directory must be a relative path"})),
        )
            .into_response());
    }
    let resolved = root_canonical.join(relative);
    // If the directory already exists, canonicalize and verify containment.
    if resolved.exists() {
        let canonical = resolved.canonicalize().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to resolve directory: {}", e)})),
            )
                .into_response()
        })?;
        if !canonical.starts_with(&root_canonical) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Directory must be within the project root"})),
            )
                .into_response());
        }
        Ok(canonical)
    } else {
        // Directory doesn't exist yet — verify no ".." components escape the root
        // by checking lexically. The directory will be created later if needed.
        let mut normalized = root_canonical.clone();
        for component in std::path::Path::new(relative).components() {
            match component {
                std::path::Component::Normal(c) => normalized.push(c),
                std::path::Component::ParentDir => {
                    normalized.pop();
                    if !normalized.starts_with(&root_canonical) {
                        return Err((
                            StatusCode::BAD_REQUEST,
                            Json(json!({"error": "Directory must be within the project root"})),
                        )
                            .into_response());
                    }
                }
                std::path::Component::CurDir => {}
                _ => {
                    return Err((
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": "Invalid directory path"})),
                    )
                        .into_response());
                }
            }
        }
        Ok(normalized)
    }
}

/// Query parameters for lighting endpoints — allows overriding the directory.
#[derive(serde::Deserialize, Default)]
pub(super) struct LightingDirQuery {
    dir: Option<String>,
}

/// Validates that a fixture type or venue name is safe for use as a filename.
#[allow(clippy::result_large_err)]
fn validate_lighting_name(name: &str) -> Result<(), axum::response::Response> {
    if name.is_empty()
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Invalid name"})),
        )
            .into_response());
    }
    Ok(())
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
    let mut all = std::collections::HashMap::new();
    if let Err(e) =
        load_light_files_from_dir(&dir, |content| match lighting::parser::parse_fixture_types(
            content,
        ) {
            Ok(types) => {
                all.extend(types);
                Ok(())
            }
            Err(e) => Err(e),
        })
    {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to load fixture types: {}", e)})),
        )
            .into_response());
    }
    Ok((StatusCode::OK, Json(json!({"fixture_types": all}))).into_response())
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
    let file_path = dir.join(format!("{}.light", sanitize_filename(&name)));
    if !file_path.is_file() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Fixture type not found: {}", name)})),
        )
            .into_response());
    }
    let content = std::fs::read_to_string(&file_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to read file: {}", e)})),
        )
            .into_response()
    })?;
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
            Json(json!({"fixture_type": ft, "dsl": content})),
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

    let dsl = if content_type.contains("application/json") {
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
    lighting::parser::parse_fixture_types(&dsl).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Invalid fixture type DSL: {}", e)})),
        )
            .into_response()
    })?;

    // Ensure directory exists
    ensure_lighting_dir(&dir)?;

    let file_path = dir.join(format!("{}.light", sanitize_filename(&name)));
    std::fs::write(&file_path, &dsl).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to write file: {}", e)})),
        )
            .into_response()
    })?;

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
    let file_path = dir.join(format!("{}.light", sanitize_filename(&name)));
    if !file_path.is_file() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Fixture type not found: {}", name)})),
        )
            .into_response());
    }
    std::fs::remove_file(&file_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to delete file: {}", e)})),
        )
            .into_response()
    })?;
    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(json!({"status": "deleted", "name": name})),
        )
            .into_response(),
    )
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
    let mut all = std::collections::HashMap::new();
    if let Err(e) =
        load_light_files_from_dir(&dir, |content| {
            match lighting::parser::parse_venues(content) {
                Ok(venues) => {
                    all.extend(venues);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        })
    {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to load venues: {}", e)})),
        )
            .into_response());
    }
    Ok((StatusCode::OK, Json(json!({"venues": all}))).into_response())
}

/// GET /api/lighting/venues/:name — returns a single venue.
pub(super) async fn get_venue(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let dir = resolve_lighting_dir(&state.config_path, query.dir.as_deref(), DEFAULT_VENUES_DIR)?;
    let file_path = dir.join(format!("{}.light", sanitize_filename(&name)));
    if !file_path.is_file() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Venue not found: {}", name)})),
        )
            .into_response());
    }
    let content = std::fs::read_to_string(&file_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to read file: {}", e)})),
        )
            .into_response()
    })?;
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
    lighting::parser::parse_venues(&dsl).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Invalid venue DSL: {}", e)})),
        )
            .into_response()
    })?;

    ensure_lighting_dir(&dir)?;

    let file_path = dir.join(format!("{}.light", sanitize_filename(&name)));
    std::fs::write(&file_path, &dsl).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to write file: {}", e)})),
        )
            .into_response()
    })?;

    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(json!({"status": "saved", "name": name})),
        )
            .into_response(),
    )
}

/// DELETE /api/lighting/venues/:name — deletes a venue file.
pub(super) async fn delete_venue(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<LightingDirQuery>,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let dir = resolve_lighting_dir(&state.config_path, query.dir.as_deref(), DEFAULT_VENUES_DIR)?;
    let file_path = dir.join(format!("{}.light", sanitize_filename(&name)));
    if !file_path.is_file() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Venue not found: {}", name)})),
        )
            .into_response());
    }
    std::fs::remove_file(&file_path).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to delete file: {}", e)})),
        )
            .into_response()
    })?;
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

/// Reads all .light files from a directory, calling the processor for each.
fn load_light_files_from_dir(
    dir: &std::path::Path,
    mut processor: impl FnMut(&str) -> Result<(), Box<dyn std::error::Error>>,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("light") {
            let content = std::fs::read_to_string(&path)?;
            processor(&content)?;
        }
    }
    Ok(())
}

/// Ensures a lighting directory exists, creating it if necessary.
#[allow(clippy::result_large_err)]
fn ensure_lighting_dir(dir: &std::path::Path) -> Result<(), axum::response::Response> {
    if !dir.exists() {
        std::fs::create_dir_all(dir).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to create directory: {}", e)})),
            )
                .into_response()
        })?;
    }
    Ok(())
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
    let fixtures = json
        .get("fixtures")
        .and_then(|v| v.as_array())
        .ok_or("Missing 'fixtures' array")?;

    let mut dsl = format!("venue \"{name}\" {{\n");

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

        dsl.push_str(&format!(
            "  fixture \"{fix_name}\" {fix_type} @ {universe}:{start_channel}"
        ));

        if let Some(tags) = fix.get("tags").and_then(|v| v.as_array()) {
            let tag_strs: Vec<String> = tags
                .iter()
                .filter_map(|t| t.as_str())
                .map(|t| format!("\"{t}\""))
                .collect();
            if !tag_strs.is_empty() {
                dsl.push_str(&format!(" tags [{}]", tag_strs.join(", ")));
            }
        }
        dsl.push('\n');
    }

    if let Some(groups) = json.get("groups").and_then(|v| v.as_object()) {
        for (group_name, group_fixtures) in groups {
            if let Some(fixture_list) = group_fixtures.as_array() {
                let names: Vec<&str> = fixture_list.iter().filter_map(|v| v.as_str()).collect();
                dsl.push_str(&format!(
                    "  group \"{group_name}\" = {}\n",
                    names.join(", ")
                ));
            }
        }
    }

    dsl.push_str("}\n");
    Ok(dsl)
}

#[cfg(test)]
mod test {
    use super::super::router;
    use super::super::test_helpers::*;
    use super::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use tower::ServiceExt;

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

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
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
    lights: static color: "red"
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
        let content = "show \"test\" {\n    @00:00.000\n    lights: static color: \"red\"\n}\n";
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

        let content = "show \"test\" {\n    @00:00.000\n    lights: static color: \"red\"\n}\n";
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

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
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

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["error"].as_str().unwrap().contains("Invalid path"));
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
                        "show \"test\" {\n    @00:00.000\n    lights: static color: \"red\"\n}\n",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["error"].as_str().unwrap().contains("Invalid path"));
    }

    #[tokio::test]
    async fn put_lighting_file_write_failure_returns_500() {
        use std::os::unix::fs::PermissionsExt;

        let (state, _dir) = test_state();
        let sub = state.songs_path.join("readonly");
        std::fs::create_dir(&sub).unwrap();
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o555)).unwrap();

        let content = "show \"test\" {\n    @00:00.000\n    lights: static color: \"red\"\n}\n";
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
}
