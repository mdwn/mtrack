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
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use std::path::PathBuf;

use super::super::config_io;
use super::super::server::WebUiState;
use crate::config;

/// Resolves the path for the legacy `/api/playlist` endpoints.
/// Prefers `legacy_playlist_path`, falls back to `playlists_dir/playlist.yaml`.
fn resolve_legacy_playlist_path(state: &WebUiState) -> Option<PathBuf> {
    if let Some(ref p) = state.legacy_playlist_path {
        return Some(p.clone());
    }
    state
        .playlists_dir
        .as_ref()
        .map(|d| d.join("playlist.yaml"))
}

/// GET /api/playlist — returns the playlist config as JSON (backward compat).
pub(super) async fn get_playlist(State(state): State<WebUiState>) -> impl IntoResponse {
    let Some(path) = resolve_legacy_playlist_path(&state) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "No playlist configured"})),
        )
            .into_response();
    };
    match config::Playlist::deserialize(&path) {
        Ok(playlist) => match serde_json::to_value(&playlist) {
            Ok(json_val) => (StatusCode::OK, Json(json_val)).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to serialize playlist: {}", e)})),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to parse playlist: {}", e)})),
        )
            .into_response(),
    }
}

/// PUT /api/playlist — validates and atomically writes the playlist (backward compat).
pub(super) async fn put_playlist(
    State(state): State<WebUiState>,
    body: String,
) -> impl IntoResponse {
    let Some(path) = resolve_legacy_playlist_path(&state) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "No playlist configured"})),
        )
            .into_response();
    };
    if let Err(errors) = config_io::validate_playlist(&body) {
        return (StatusCode::BAD_REQUEST, Json(json!({"errors": errors}))).into_response();
    }

    match config_io::atomic_write(&path, &body) {
        Ok(()) => (StatusCode::OK, Json(json!({"status": "saved"}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response(),
    }
}

/// POST /api/playlist/validate — validates playlist YAML without saving.
pub(super) async fn validate_playlist(body: String) -> impl IntoResponse {
    match config_io::validate_playlist(&body) {
        Ok(()) => (StatusCode::OK, Json(json!({"valid": true}))).into_response(),
        Err(errors) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"valid": false, "errors": errors})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Playlist CRUD endpoints
// ---------------------------------------------------------------------------

/// Validates a playlist name for use in file paths.
#[allow(clippy::result_large_err)]
fn validate_playlist_name(name: &str) -> Result<(), axum::response::Response> {
    if name.is_empty()
        || name == "all_songs"
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Invalid playlist name"})),
        )
            .into_response());
    }
    Ok(())
}

/// Returns the playlists directory, or an error response if not configured.
#[allow(clippy::result_large_err)]
fn require_playlists_dir(state: &WebUiState) -> Result<PathBuf, axum::response::Response> {
    state.playlists_dir.clone().ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "No playlists directory configured"})),
        )
            .into_response()
    })
}

/// Resolves a playlist file path within the playlists directory, verifying
/// that the result does not escape the directory via symlinks or other tricks.
/// The playlists directory must exist before calling this function.
/// Returns the validated path or an error response.
#[allow(clippy::result_large_err)]
fn resolve_playlist_path(
    playlists_dir: &std::path::Path,
    name: &str,
    ext: &str,
) -> Result<PathBuf, axum::response::Response> {
    // Canonicalize the directory itself (must exist) so the resulting path
    // is anchored to a verified absolute base.
    let dir_canonical = playlists_dir.canonicalize().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to resolve playlists dir: {}", e)})),
        )
            .into_response()
    })?;
    let file_path = dir_canonical.join(format!("{}.{}", name, ext));
    // If the file already exists, canonicalize it and verify it stayed inside.
    // This catches symlink escapes.
    if file_path.exists() {
        let canonical = file_path.canonicalize().map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to resolve path: {}", e)})),
            )
                .into_response()
        })?;
        if !canonical.starts_with(&dir_canonical) {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Invalid playlist path"})),
            )
                .into_response());
        }
        Ok(canonical)
    } else {
        // File doesn't exist yet. Verify the parent of the constructed path
        // is still the canonical directory (defense in depth beyond name validation).
        let parent = file_path.parent().ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Invalid playlist path"})),
            )
                .into_response()
        })?;
        if parent != dir_canonical {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Invalid playlist path"})),
            )
                .into_response());
        }
        Ok(file_path)
    }
}

/// GET /api/playlists — list all playlists with name, song count, and active status.
pub(super) async fn get_playlists(State(state): State<WebUiState>) -> impl IntoResponse {
    let playlists = state.player.list_playlists();
    let active = {
        let active_playlist = state.player.get_playlist();
        active_playlist.name().to_string()
    };
    let items: Vec<serde_json::Value> = playlists
        .iter()
        .map(|name| {
            let playlist_map = state.player.playlists_snapshot();
            let song_count = playlist_map.get(name).map(|p| p.songs().len()).unwrap_or(0);
            json!({
                "name": name,
                "song_count": song_count,
                "is_active": name == &active,
            })
        })
        .collect();
    (StatusCode::OK, Json(json!(items))).into_response()
}

/// GET /api/playlists/:name — get a playlist's songs and available songs.
pub(super) async fn get_playlist_by_name(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let playlists = state.player.playlists_snapshot();
    let Some(pl) = playlists.get(&name) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Playlist '{}' not found", name)})),
        )
            .into_response();
    };
    let all_songs: Vec<String> = state
        .player
        .songs()
        .sorted_list()
        .iter()
        .map(|s| s.name().to_string())
        .collect();
    (
        StatusCode::OK,
        Json(json!({
            "name": name,
            "songs": pl.songs(),
            "available_songs": all_songs,
        })),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
pub(super) struct PlaylistBody {
    songs: Vec<String>,
}

/// PUT /api/playlists/:name — create or update a playlist.
pub(super) async fn put_playlist_by_name(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Json(body): Json<PlaylistBody>,
) -> impl IntoResponse {
    validate_playlist_name(&name)?;
    let playlists_dir = require_playlists_dir(&state)?;

    // Write the playlist YAML file.
    let playlist_config = config::Playlist::new(&body.songs);
    let yaml = match crate::util::to_yaml_string(&playlist_config) {
        Ok(y) => y,
        Err(e) => {
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to serialize playlist: {}", e)})),
            )
                .into_response());
        }
    };

    // Ensure the playlists directory exists.
    if let Err(e) = std::fs::create_dir_all(&playlists_dir) {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to create playlists directory: {}", e)})),
        )
            .into_response());
    }

    // codeql[rust/path-injection] name is validated by validate_playlist_name; path is
    // verified via canonicalize + starts_with containment in resolve_playlist_path.
    let file_path = resolve_playlist_path(&playlists_dir, &name, "yaml")?;
    if let Err(e) = config_io::atomic_write(&file_path, &yaml) {
        return Err((StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response());
    }

    // Reload songs to pick up the new playlist.
    state.player.reload_songs(
        &state.songs_path,
        state.playlists_dir.as_deref(),
        state.legacy_playlist_path.as_deref(),
    );

    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(json!({"status": "saved", "name": name})),
        )
            .into_response(),
    )
}

/// DELETE /api/playlists/:name — delete a playlist.
pub(super) async fn delete_playlist_by_name(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    validate_playlist_name(&name)?;
    let playlists_dir = require_playlists_dir(&state)?;

    // codeql[rust/path-injection] name is validated by validate_playlist_name; path is
    // verified via canonicalize + starts_with containment in resolve_playlist_path.
    let file_path = resolve_playlist_path(&playlists_dir, &name, "yaml")?;
    if !file_path.is_file() {
        // Also check .yml extension.
        let yml_path = resolve_playlist_path(&playlists_dir, &name, "yml")?;
        if yml_path.is_file() {
            std::fs::remove_file(&yml_path).map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("Failed to delete file: {}", e)})),
                )
                    .into_response()
            })?;
        } else {
            return Err((
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Playlist '{}' not found", name)})),
            )
                .into_response());
        }
    } else {
        std::fs::remove_file(&file_path).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to delete file: {}", e)})),
            )
                .into_response()
        })?;
    }

    // Reload songs to remove the playlist.
    state.player.reload_songs(
        &state.songs_path,
        state.playlists_dir.as_deref(),
        state.legacy_playlist_path.as_deref(),
    );

    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(json!({"status": "deleted", "name": name})),
        )
            .into_response(),
    )
}

/// POST /api/playlists/:name/activate — switch the active playlist.
pub(super) async fn activate_playlist(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    match state.player.switch_to_playlist(&name).await {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({"status": "activated", "name": name})),
        )
            .into_response(),
        Err(e) => {
            let status = if e.contains("not found") {
                StatusCode::NOT_FOUND
            } else {
                StatusCode::CONFLICT
            };
            (status, Json(json!({"error": e}))).into_response()
        }
    }
}

#[cfg(test)]
mod test {
    use super::super::router;
    use super::super::test_helpers::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn get_playlist_returns_json() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/playlist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_playlist_missing_file() {
        let (mut state, _dir) = test_state();
        state.legacy_playlist_path = Some(std::path::PathBuf::from("/nonexistent/playlist.yaml"));
        state.playlists_dir = None;
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/playlist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn validate_playlist_valid() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/playlist/validate")
                    .body(Body::from("songs:\n  - song1\n"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn validate_playlist_invalid() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/playlist/validate")
                    .body(Body::from("not valid: [[["))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn put_playlist_valid() {
        let (state, _dir) = test_state();
        let playlist_path = state.legacy_playlist_path.clone().unwrap();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/playlist")
                    .body(Body::from("songs:\n  - Song A\n"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            std::fs::read_to_string(&playlist_path).unwrap(),
            "songs:\n  - Song A\n"
        );
    }

    #[tokio::test]
    async fn put_playlist_invalid() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/playlist")
                    .body(Body::from("invalid yaml: [[["))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn get_playlist_error_body_contains_message() {
        let (mut state, _dir) = test_state();
        state.legacy_playlist_path = Some(std::path::PathBuf::from("/nonexistent/playlist.yaml"));
        state.playlists_dir = None;
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/playlist")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["error"]
            .as_str()
            .unwrap()
            .contains("Failed to parse playlist"));
    }

    #[tokio::test]
    async fn put_playlist_write_failure_returns_500() {
        let (mut state, _dir) = test_state();
        state.legacy_playlist_path =
            Some(std::path::PathBuf::from("/nonexistent/dir/playlist.yaml"));
        state.playlists_dir = None;
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/playlist")
                    .body(Body::from("songs:\n  - Song A\n"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["error"].is_string());
    }
}
