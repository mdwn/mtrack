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

    match std::fs::read_to_string(safe.as_path()) {
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

    // Validate the DSL content
    if let Err(errors) = config_io::validate_light_show(&body) {
        return (StatusCode::BAD_REQUEST, Json(json!({"errors": errors}))).into_response();
    }

    match config_io::atomic_write(&verified_path, &body) {
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
    use super::super::safe_path::{SafePath, VerifiedRoot};

    let project_root = config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let root = VerifiedRoot::new(project_root).map_err(|e| e.into_response())?;

    let relative = match override_dir {
        Some(d) if !d.is_empty() => d,
        _ => default,
    };

    SafePath::validate_relative(relative, &root).map_err(|e| e.into_response())
}

/// Query parameters for lighting endpoints — allows overriding the directory.
#[derive(serde::Deserialize, Default)]
pub(super) struct LightingDirQuery {
    dir: Option<String>,
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
                        "show \"test\" {\n    @00:00.000\n    lights: static color: \"red\"\n}\n",
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
    fn venue_json_to_dsl_with_groups() {
        let json = serde_json::json!({
            "fixtures": [
                {
                    "name": "L1",
                    "fixture_type": "Par",
                    "universe": 1,
                    "start_channel": 1
                },
                {
                    "name": "L2",
                    "fixture_type": "Par",
                    "universe": 1,
                    "start_channel": 5
                }
            ],
            "groups": {
                "front": ["L1", "L2"]
            }
        });
        let dsl = venue_json_to_dsl("Grouped", &json).unwrap();
        assert!(dsl.contains("group \"front\" = L1, L2"));
        // Verify the DSL actually parses.
        let venues = lighting::parser::parse_venues(&dsl).unwrap();
        let v = venues.get("Grouped").unwrap();
        assert!(v.groups().contains_key("front"));
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
        std::fs::write(
            dir.path().join("a.light"),
            &sample_fixture_type_dsl("TypeA"),
        )
        .unwrap();
        std::fs::write(dir.path().join("b.txt"), "not a light file").unwrap();

        let mut count = 0;
        load_light_files_from_dir(dir.path(), |_content| {
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
        load_light_files_from_dir(dir.path(), |_content| {
            count += 1;
            Ok(())
        })
        .unwrap();
        assert_eq!(count, 0);
    }

    // -----------------------------------------------------------------------
    // Unit tests: ensure_lighting_dir
    // -----------------------------------------------------------------------

    #[test]
    fn ensure_lighting_dir_creates_directory() {
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("new_subdir");
        assert!(!sub.exists());
        ensure_lighting_dir(&sub).unwrap();
        assert!(sub.is_dir());
    }

    #[test]
    fn ensure_lighting_dir_existing_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        ensure_lighting_dir(dir.path()).unwrap();
        assert!(dir.path().is_dir());
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
            &sample_fixture_type_dsl("LED_Par"),
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
        assert!(parsed["fixture_types"]["LED_Par"].is_object());
    }

    #[tokio::test]
    async fn get_fixture_type_success() {
        let (state, _dir) = test_state();
        let ft_dir = _dir.path().join("ft_get");
        std::fs::create_dir(&ft_dir).unwrap();
        std::fs::write(
            ft_dir.join("led_par.light"),
            &sample_fixture_type_dsl("LED_Par"),
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
        std::fs::write(&file_path, &sample_fixture_type_dsl("ToDelete")).unwrap();
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
        std::fs::write(v_dir.join("club.light"), &sample_venue_dsl("Club")).unwrap();
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
        std::fs::write(v_dir.join("club.light"), &sample_venue_dsl("Club")).unwrap();
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
        std::fs::write(&file_path, &sample_venue_dsl("ToDelete")).unwrap();
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
        std::fs::write(ft_dir.join("a.light"), &sample_fixture_type_dsl("TypeA")).unwrap();
        std::fs::write(ft_dir.join("b.light"), &sample_fixture_type_dsl("TypeB")).unwrap();
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
        std::fs::write(v_dir.join("a.light"), &sample_venue_dsl("VenueA")).unwrap();
        std::fs::write(v_dir.join("b.light"), &sample_venue_dsl("VenueB")).unwrap();
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
}
