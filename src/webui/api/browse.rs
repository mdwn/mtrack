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
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use super::super::server::WebUiState;
use crate::songs;

/// GET /api/browse?path=... — lists files and directories at a filesystem path.
///
/// Restricted to the directory containing mtrack.yaml (the config root).
/// If no `path` query parameter is provided, defaults to the config root.
/// Returns entries sorted: directories first, then files alphabetically.
pub(super) async fn browse_directory(
    State(state): State<WebUiState>,
    Query(params): Query<BrowseParams>,
) -> impl IntoResponse {
    // The browsable root is the directory containing mtrack.yaml.
    // Canonicalize config_path first to handle relative paths (e.g., "mtrack.yaml").
    let config_canonical = match state.config_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to resolve config path: {}", e)})),
            )
                .into_response();
        }
    };

    let root_canonical = match config_canonical.parent() {
        Some(p) => p.to_path_buf(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Unable to determine config root directory"})),
            )
                .into_response();
        }
    };

    // Resolve the requested path. Paths from the frontend are project-relative
    // (e.g., "/" = project root, "/songs" = songs subdirectory).
    let requested = if params.path.is_empty() || params.path == "/" {
        root_canonical.clone()
    } else {
        // Strip leading "/" and join with root to get absolute path.
        let relative = params.path.strip_prefix('/').unwrap_or(&params.path);
        root_canonical.join(relative)
    };

    // Canonicalize the requested path and verify it's under the root before
    // touching the filesystem, preventing path traversal attacks.
    let dir_canonical = match requested.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Not a directory: {}", params.path)})),
            )
                .into_response();
        }
    };

    if !dir_canonical.starts_with(&root_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({
                "error": "Access denied: path is outside the project directory",
            })),
        )
            .into_response();
    }

    if !dir_canonical.is_dir() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Not a directory: {}", params.path)})),
        )
            .into_response();
    }

    // Convert an absolute path to a project-relative path (e.g., "/songs/foo").
    let to_relative = |abs: &std::path::Path| -> String {
        let suffix = abs
            .strip_prefix(&root_canonical)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        if suffix.is_empty() {
            "/".to_string()
        } else {
            format!("/{suffix}")
        }
    };

    let mut entries: Vec<serde_json::Value> = Vec::new();
    match std::fs::read_dir(&dir_canonical) {
        Ok(iter) => {
            for entry in iter.flatten() {
                let path = entry.path();
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                // Skip hidden files
                if name.starts_with('.') {
                    continue;
                }
                let is_dir = path.is_dir();
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let file_type = if is_dir {
                    "directory"
                } else if songs::is_supported_audio_extension(&ext) {
                    "audio"
                } else if ext == "mid" {
                    "midi"
                } else if ext == "light" {
                    "lighting"
                } else {
                    "other"
                };
                entries.push(json!({
                    "name": name,
                    "path": to_relative(&path),
                    "type": file_type,
                    "is_dir": is_dir,
                }));
            }
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to read directory: {}", e)})),
            )
                .into_response();
        }
    }

    // Sort: directories first, then alphabetically by name
    entries.sort_by(|a, b| {
        let a_dir = a.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false);
        let b_dir = b.get("is_dir").and_then(|v| v.as_bool()).unwrap_or(false);
        b_dir.cmp(&a_dir).then_with(|| {
            a.get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase()
                .cmp(
                    &b.get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_lowercase(),
                )
        })
    });

    (
        StatusCode::OK,
        Json(json!({
            "path": to_relative(&dir_canonical),
            "root": root_canonical.to_string_lossy(),
            "entries": entries,
        })),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
pub(super) struct BrowseParams {
    #[serde(default)]
    path: String,
}

/// POST /api/browse/create-song — auto-generates a song.yaml in a project-relative directory.
///
/// Expects a JSON body with `path` (project-relative directory, e.g. "/songs/Afar")
/// and an optional `name` override. The backend scans the directory for audio/MIDI/lighting
/// files and generates the song config automatically, including per-channel track splitting
/// for stereo and multichannel audio files.
pub(super) async fn create_song_in_directory(
    State(state): State<WebUiState>,
    Json(body): Json<CreateSongInDirRequest>,
) -> impl IntoResponse {
    // Resolve the project root (same logic as browse_directory).
    let config_canonical = match state.config_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to resolve config path: {}", e)})),
            )
                .into_response();
        }
    };
    let root_canonical = match config_canonical.parent() {
        Some(p) => p.to_path_buf(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Unable to determine config root directory"})),
            )
                .into_response();
        }
    };

    // Resolve the target directory from the project-relative path.
    let relative = body.path.strip_prefix('/').unwrap_or(&body.path);
    let target_dir = if relative.is_empty() {
        root_canonical.clone()
    } else {
        root_canonical.join(relative)
    };

    let dir_canonical = match target_dir.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Directory not found: {}", body.path)})),
            )
                .into_response();
        }
    };

    if !dir_canonical.starts_with(&root_canonical) {
        return (
            StatusCode::FORBIDDEN,
            Json(json!({"error": "Access denied: path is outside the project directory"})),
        )
            .into_response();
    }

    if !dir_canonical.is_dir() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Path is not a directory"})),
        )
            .into_response();
    }

    let config_path = dir_canonical.join("song.yaml");
    if config_path.exists() {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "song.yaml already exists in this directory"})),
        )
            .into_response();
    }

    // Use Song::initialize to scan the directory and build the config with proper
    // channel-aware track splitting (stereo -> L/R, multichannel -> per-channel).
    let mut song_config = match songs::Song::initialize(&dir_canonical) {
        Ok(song) => song.get_config(),
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Failed to scan directory: {}", e)})),
            )
                .into_response();
        }
    };

    // Apply name override if provided.
    if let Some(ref name) = body.name {
        let trimmed = name.trim();
        if !trimmed.is_empty() {
            song_config.set_name(trimmed);
        }
    }

    match song_config.save(&config_path) {
        Ok(()) => {
            // Refresh the player's all-songs playlist so newly imported songs appear.
            state.player.reload_songs(
                &state.songs_path,
                state.playlists_dir.as_deref(),
                state.legacy_playlist_path.as_deref(),
            );
            (
                StatusCode::CREATED,
                Json(json!({"status": "created", "path": config_path.to_string_lossy()})),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to save song config: {}", e)})),
        )
            .into_response(),
    }
}

#[derive(serde::Deserialize)]
pub(super) struct CreateSongInDirRequest {
    path: String,
    name: Option<String>,
}

#[cfg(test)]
mod test {
    use super::super::router;
    use super::super::test_helpers::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn browse_directory_default_lists_project_root() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/browse")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed.get("path").is_some());
        assert!(parsed.get("root").is_some());
        assert!(parsed.get("entries").is_some());
        assert!(parsed["entries"].is_array());
    }

    #[tokio::test]
    async fn browse_directory_with_path() {
        let (state, _dir) = test_state();
        // Create a subdirectory inside the project root.
        let subdir = _dir.path().join("subdir");
        std::fs::create_dir(&subdir).unwrap();
        std::fs::write(subdir.join("file.txt"), "hello").unwrap();

        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/browse?path=/subdir")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let entries = parsed["entries"].as_array().unwrap();
        assert!(!entries.is_empty());
    }

    #[tokio::test]
    async fn browse_directory_path_traversal_rejected() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/browse?path=/../../../etc")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        // The server should reject this with FORBIDDEN or NOT_FOUND.
        let status = response.status();
        assert!(
            status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
            "expected 403 or 404, got {status}"
        );
    }

    #[tokio::test]
    async fn create_song_in_directory_success() {
        let (state, _dir) = test_state();
        // Create a directory with a WAV file inside the project root.
        let song_dir = _dir.path().join("songs").join("mysong");
        std::fs::create_dir_all(&song_dir).unwrap();
        let wav_bytes = create_test_wav();
        std::fs::write(song_dir.join("track.wav"), &wav_bytes).unwrap();

        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/create-song")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path": "/songs/mysong"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["status"], "created");
    }

    #[tokio::test]
    async fn create_song_in_directory_conflict() {
        let (state, _dir) = test_state();
        // Create a directory that already has song.yaml.
        let song_dir = _dir.path().join("songs").join("existing");
        std::fs::create_dir_all(&song_dir).unwrap();
        std::fs::write(song_dir.join("song.yaml"), "name: existing\ntracks: []\n").unwrap();

        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/create-song")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path": "/songs/existing"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn browse_directory_classifies_file_types() {
        let (state, _dir) = test_state();
        let subdir = _dir.path().join("typedir");
        std::fs::create_dir(&subdir).unwrap();
        let wav_bytes = create_test_wav();
        std::fs::write(subdir.join("track.wav"), &wav_bytes).unwrap();
        std::fs::write(subdir.join("notes.mid"), "midi data").unwrap();
        std::fs::write(subdir.join("show.light"), "light data").unwrap();
        std::fs::write(subdir.join("readme.txt"), "text data").unwrap();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/browse?path=/typedir")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let entries = parsed["entries"].as_array().unwrap();

        let find_entry = |name: &str| entries.iter().find(|e| e["name"] == name).unwrap();
        assert_eq!(find_entry("track.wav")["type"], "audio");
        assert_eq!(find_entry("notes.mid")["type"], "midi");
        assert_eq!(find_entry("show.light")["type"], "lighting");
        assert_eq!(find_entry("readme.txt")["type"], "other");
    }

    #[tokio::test]
    async fn create_song_in_directory_with_name_override() {
        let (state, _dir) = test_state();
        let song_dir = _dir.path().join("songs").join("override_test");
        std::fs::create_dir_all(&song_dir).unwrap();
        let wav_bytes = create_test_wav();
        std::fs::write(song_dir.join("track.wav"), &wav_bytes).unwrap();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/create-song")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"path": "/songs/override_test", "name": "Custom Name"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let config_content = std::fs::read_to_string(song_dir.join("song.yaml")).unwrap();
        assert!(
            config_content.contains("Custom Name"),
            "song.yaml should contain the custom name, got: {}",
            config_content
        );
    }

    #[tokio::test]
    async fn create_song_in_directory_nonexistent_path() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/browse/create-song")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"path": "/nonexistent/dir"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn browse_directory_sorts_dirs_first() {
        let (state, _dir) = test_state();
        let subdir = _dir.path().join("sorttest");
        std::fs::create_dir(&subdir).unwrap();
        // Create files and directories
        std::fs::write(subdir.join("aaa_file.txt"), "data").unwrap();
        std::fs::create_dir(subdir.join("zzz_dir")).unwrap();
        std::fs::write(subdir.join("bbb_file.wav"), &create_test_wav()).unwrap();
        std::fs::create_dir(subdir.join("aaa_dir")).unwrap();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/browse?path=/sorttest")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let entries = parsed["entries"].as_array().unwrap();

        // Directories should come first
        assert_eq!(entries.len(), 4);
        assert!(entries[0]["is_dir"].as_bool().unwrap());
        assert!(entries[1]["is_dir"].as_bool().unwrap());
        assert!(!entries[2]["is_dir"].as_bool().unwrap());
        assert!(!entries[3]["is_dir"].as_bool().unwrap());

        // Within groups, sorted alphabetically
        assert_eq!(entries[0]["name"], "aaa_dir");
        assert_eq!(entries[1]["name"], "zzz_dir");
    }
}
