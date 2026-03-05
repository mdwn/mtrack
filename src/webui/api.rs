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
    routing::{get, post},
    Json, Router,
};
use serde_json::json;

use super::config_io;
use super::server::WebUiState;
use crate::config;
use crate::songs;

/// Builds the API router for config read/write endpoints.
///
/// Playback control is handled via gRPC-Web (PlayerService), not REST.
pub fn router() -> Router<WebUiState> {
    Router::new()
        .route("/config", get(get_config_raw).put(put_config))
        .route("/config/parsed", get(get_config_parsed))
        .route("/config/validate", post(validate_config))
        .route("/songs", get(get_songs))
        .route("/songs/{name}", get(get_song).put(put_song))
        .route("/playlist", get(get_playlist).put(put_playlist))
        .route("/playlist/validate", post(validate_playlist))
        .route("/lighting", get(get_lighting_files))
        .route(
            "/lighting/{name}",
            get(get_lighting_file).put(put_lighting_file),
        )
        .route("/lighting/validate", post(validate_lighting))
}

/// GET /api/config — returns the raw YAML content of the player config file.
async fn get_config_raw(State(state): State<WebUiState>) -> impl IntoResponse {
    match std::fs::read_to_string(&state.config_path) {
        Ok(content) => (
            StatusCode::OK,
            [("content-type", "text/yaml; charset=utf-8")],
            content,
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to read config: {}", e)})),
        )
            .into_response(),
    }
}

/// GET /api/config/parsed — returns the player config as JSON.
async fn get_config_parsed(State(state): State<WebUiState>) -> impl IntoResponse {
    match config::Player::deserialize(&state.config_path) {
        Ok(player_config) => match serde_json::to_value(&player_config) {
            Ok(json_val) => (StatusCode::OK, Json(json_val)).into_response(),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to serialize config: {}", e)})),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to parse config: {}", e)})),
        )
            .into_response(),
    }
}

/// GET /api/songs — returns a list of all songs with metadata.
async fn get_songs(State(state): State<WebUiState>) -> impl IntoResponse {
    match songs::get_all_songs(&state.songs_path) {
        Ok(all_songs) => {
            let song_list: Vec<serde_json::Value> = all_songs
                .sorted_list()
                .iter()
                .map(|song| {
                    json!({
                        "name": song.name(),
                        "duration_ms": song.duration().as_millis() as u64,
                        "duration_display": song.duration_string(),
                        "num_channels": song.num_channels(),
                        "sample_format": format!("{}", song.sample_format()),
                        "track_count": song.tracks().len(),
                        "tracks": song.tracks().iter().map(|t| t.name().to_string()).collect::<Vec<_>>(),
                    })
                })
                .collect();
            (StatusCode::OK, Json(json!({"songs": song_list}))).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to load songs: {}", e)})),
        )
            .into_response(),
    }
}

/// GET /api/songs/:name — returns a single song's config YAML.
async fn get_song(State(state): State<WebUiState>, Path(name): Path<String>) -> impl IntoResponse {
    match songs::get_all_songs(&state.songs_path) {
        Ok(all_songs) => match all_songs.get(&name) {
            Ok(song) => {
                // Try to read the song's config YAML from its base_path
                let config_path = song.base_path().join("song.yaml");
                let alt_config_path = song.base_path().join("song.yml");
                let yaml_path = if config_path.exists() {
                    Some(config_path)
                } else if alt_config_path.exists() {
                    Some(alt_config_path)
                } else {
                    None
                };

                match yaml_path {
                    Some(path) => match std::fs::read_to_string(&path) {
                        Ok(content) => (
                            StatusCode::OK,
                            [("content-type", "text/yaml; charset=utf-8")],
                            content,
                        )
                            .into_response(),
                        Err(e) => (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({"error": format!("Failed to read song config: {}", e)})),
                        )
                            .into_response(),
                    },
                    None => {
                        // Return a JSON summary if no config file found
                        (
                            StatusCode::OK,
                            Json(json!({
                                "name": song.name(),
                                "duration_ms": song.duration().as_millis() as u64,
                                "duration_display": song.duration_string(),
                                "num_channels": song.num_channels(),
                                "sample_format": format!("{}", song.sample_format()),
                                "tracks": song.tracks().iter().map(|t| t.name().to_string()).collect::<Vec<_>>(),
                                "config_file": false,
                            })),
                        )
                            .into_response()
                    }
                }
            }
            Err(_) => (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Song not found: {}", name)})),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to load songs: {}", e)})),
        )
            .into_response(),
    }
}

/// GET /api/playlist — returns the playlist config as JSON.
async fn get_playlist(State(state): State<WebUiState>) -> impl IntoResponse {
    match config::Playlist::deserialize(&state.playlist_path) {
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

/// GET /api/lighting — lists available .light files from the songs directory.
async fn get_lighting_files(State(state): State<WebUiState>) -> impl IntoResponse {
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
fn find_light_files(
    base: &std::path::Path,
    dir: &std::path::Path,
    results: &mut Vec<serde_json::Value>,
) -> Result<(), std::io::Error> {
    if !dir.is_dir() {
        return Ok(());
    }
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
async fn get_lighting_file(
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

// ---------------------------------------------------------------------------
// Write + Validate endpoints
// ---------------------------------------------------------------------------

/// PUT /api/config — validates and atomically writes the player config.
async fn put_config(State(state): State<WebUiState>, body: String) -> impl IntoResponse {
    if let Err(errors) = config_io::validate_player_config(&body) {
        return (StatusCode::BAD_REQUEST, Json(json!({"errors": errors}))).into_response();
    }

    match config_io::atomic_write(&state.config_path, &body) {
        Ok(()) => (StatusCode::OK, Json(json!({"status": "saved"}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response(),
    }
}

/// POST /api/config/validate — validates player config YAML without saving.
async fn validate_config(body: String) -> impl IntoResponse {
    match config_io::validate_player_config(&body) {
        Ok(()) => (StatusCode::OK, Json(json!({"valid": true}))).into_response(),
        Err(errors) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"valid": false, "errors": errors})),
        )
            .into_response(),
    }
}

/// PUT /api/songs/:name — validates and atomically writes a song config.
async fn put_song(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    body: String,
) -> impl IntoResponse {
    // Find the song to get its base path
    match songs::get_all_songs(&state.songs_path) {
        Ok(all_songs) => match all_songs.get(&name) {
            Ok(song) => {
                let config_path = song.base_path().join("song.yaml");

                // Validate the YAML can be deserialized as a song config
                let tmp = match tempfile::Builder::new().suffix(".yaml").tempfile() {
                    Ok(t) => t,
                    Err(e) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(json!({"error": format!("Failed to create temp file: {}", e)})),
                        )
                            .into_response();
                    }
                };
                if let Err(e) = std::fs::write(tmp.path(), &body) {
                    return (
                        StatusCode::INTERNAL_SERVER_ERROR,
                        Json(json!({"error": format!("Failed to write temp file: {}", e)})),
                    )
                        .into_response();
                }
                if let Err(e) = config::Song::deserialize(tmp.path()) {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"errors": [format!("{}", e)]})),
                    )
                        .into_response();
                }

                match config_io::atomic_write(&config_path, &body) {
                    Ok(()) => (StatusCode::OK, Json(json!({"status": "saved"}))).into_response(),
                    Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e})))
                        .into_response(),
                }
            }
            Err(_) => (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Song not found: {}", name)})),
            )
                .into_response(),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to load songs: {}", e)})),
        )
            .into_response(),
    }
}

/// PUT /api/playlist — validates and atomically writes the playlist.
async fn put_playlist(State(state): State<WebUiState>, body: String) -> impl IntoResponse {
    if let Err(errors) = config_io::validate_playlist(&body) {
        return (StatusCode::BAD_REQUEST, Json(json!({"errors": errors}))).into_response();
    }

    match config_io::atomic_write(&state.playlist_path, &body) {
        Ok(()) => (StatusCode::OK, Json(json!({"status": "saved"}))).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response(),
    }
}

/// POST /api/playlist/validate — validates playlist YAML without saving.
async fn validate_playlist(body: String) -> impl IntoResponse {
    match config_io::validate_playlist(&body) {
        Ok(()) => (StatusCode::OK, Json(json!({"valid": true}))).into_response(),
        Err(errors) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"valid": false, "errors": errors})),
        )
            .into_response(),
    }
}

/// PUT /api/lighting/:name — validates and atomically writes a lighting DSL file.
async fn put_lighting_file(
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
async fn validate_lighting(body: String) -> impl IntoResponse {
    match config_io::validate_light_show(&body) {
        Ok(()) => (StatusCode::OK, Json(json!({"valid": true}))).into_response(),
        Err(errors) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"valid": false, "errors": errors})),
        )
            .into_response(),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn find_light_files_discovers_files() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();

        // Create nested structure with .light files
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

        // Create a deeply nested structure
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
        // Non-dir path returns Ok(()) with empty results
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
}
