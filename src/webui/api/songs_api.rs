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
    extract::{Multipart, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use super::super::config_io;
use super::super::server::WebUiState;
use super::super::state as ws_state;
use crate::{config, songs};

/// GET /api/songs — returns a list of all songs with metadata.
///
/// Uses the player's song registry rather than rescanning disk, so the API
/// is always consistent with what the player knows about.
pub(super) async fn get_songs(State(state): State<WebUiState>) -> impl IntoResponse {
    let all_songs = state.player.songs();
    let song_list: Vec<serde_json::Value> = all_songs
        .sorted_list()
        .iter()
        .map(|song| {
            // Compute the song's directory path relative to the songs root,
            // so the frontend can construct lighting API paths.
            let base_dir = song
                .base_path()
                .strip_prefix(&state.songs_path)
                .unwrap_or(std::path::Path::new(""))
                .to_string_lossy()
                .to_string();

            // Collect DSL lighting show file paths relative to the songs root.
            let lighting_files: Vec<String> = song
                .dsl_lighting_shows()
                .iter()
                .filter_map(|show| {
                    show.file_path()
                        .strip_prefix(&state.songs_path)
                        .ok()
                        .map(|p| p.to_string_lossy().to_string())
                })
                .collect();

            // Collect legacy MIDI DMX file paths relative to the songs root.
            let legacy_lighting_files: Vec<String> = song
                .light_shows()
                .iter()
                .filter_map(|show| {
                    show.dmx_file_path()
                        .strip_prefix(&state.songs_path)
                        .ok()
                        .map(|p| p.to_string_lossy().to_string())
                })
                .collect();

            json!({
                "name": song.name(),
                "duration_ms": song.duration().as_millis() as u64,
                "duration_display": song.duration_string(),
                "num_channels": song.num_channels(),
                "sample_format": format!("{}", song.sample_format()),
                "track_count": song.tracks().len(),
                "tracks": song.tracks().iter().map(|t| t.name().to_string()).collect::<Vec<_>>(),
                "has_midi": song.midi_playback().is_some(),
                "has_lighting": !song.light_shows().is_empty() || !song.dsl_lighting_shows().is_empty(),
                "base_dir": base_dir,
                "lighting_files": lighting_files,
                "legacy_lighting_files": legacy_lighting_files,
            })
        })
        .collect();
    (StatusCode::OK, Json(json!({"songs": song_list}))).into_response()
}

/// GET /api/songs/:name — returns a single song's config YAML.
pub(super) async fn get_song(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let all_songs = state.player.songs();
    match all_songs.get(&name) {
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
    }
}

/// POST /api/songs/:name/import — copies a file from the server filesystem into a song directory.
///
/// Expects a JSON body with `path` (absolute filesystem path) and optional `filename` override.
/// The file extension must be supported (audio, MIDI, or .light).
/// The source path must resolve to within the project root (the directory containing mtrack.yaml)
/// to prevent arbitrary file reads.
pub(super) async fn import_file_to_song(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Json(body): Json<ImportFileRequest>,
) -> impl IntoResponse {
    // Determine the project root (same as the browse endpoint).
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
    let project_root = match config_canonical.parent() {
        Some(p) => p.to_path_buf(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Unable to determine project root"})),
            )
                .into_response();
        }
    };

    // Canonicalize the source path to resolve symlinks and verify it exists.
    // codeql[rust/path-injection] Path is canonicalized and verified against project_root below.
    let source_canonical = match std::path::Path::new(&body.path).canonicalize() {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Source file does not exist or is invalid"})),
            )
                .into_response();
        }
    };
    if !source_canonical.starts_with(&project_root) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Source path is outside the project directory"})),
        )
            .into_response();
    }
    if !source_canonical.is_file() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Source path is not a file"})),
        )
            .into_response();
    }

    let filename = body
        .filename
        .as_deref()
        .or_else(|| source_canonical.file_name().and_then(|n| n.to_str()))
        .unwrap_or("unknown");

    if let Err(e) = validate_track_filename(filename) {
        return *e;
    }

    let song_dir = match ensure_song_dir(&state.songs_path, &name) {
        Ok(d) => d,
        Err(e) => return *e,
    };

    // codeql[rust/path-injection] dest_path is under song_dir (verified by ensure_song_dir),
    // filename is validated by validate_track_filename (no .. or / allowed).
    let dest_path = song_dir.join(filename);
    if let Err(e) = std::fs::copy(&source_canonical, &dest_path) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to copy file: {}", e)})),
        )
            .into_response();
    }

    if let Err(e) = ensure_song_config(&song_dir) {
        return *e;
    }

    state.player.reload_songs(
        &state.songs_path,
        state.playlists_dir.as_deref(),
        state.legacy_playlist_path.as_deref(),
    );

    (
        StatusCode::OK,
        Json(json!({
            "status": "imported",
            "file": filename,
            "song": name,
        })),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
pub(super) struct ImportFileRequest {
    path: String,
    filename: Option<String>,
}

/// GET /api/songs/:name/waveform — returns waveform peaks for a song.
///
/// Uses the shared waveform cache; computes on demand if not cached.
pub(super) async fn get_song_waveform(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    // Check cache first
    {
        let cache = state.waveform_cache.lock();
        if let Some(cached) = cache.get(&name) {
            let tracks: Vec<serde_json::Value> = cached
                .iter()
                .map(|(track_name, peaks)| json!({ "name": track_name, "peaks": peaks }))
                .collect();
            return (
                StatusCode::OK,
                Json(json!({ "song_name": name, "tracks": tracks })),
            )
                .into_response();
        }
    }

    // Cache miss — look up song from the player's registry
    let all_songs = state.player.songs();
    let song = match all_songs.get(&name) {
        Ok(s) => s,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Song not found: {}", name)})),
            )
                .into_response();
        }
    };

    let track_infos: Vec<ws_state::TrackInfo> = song
        .tracks()
        .iter()
        .map(|t| {
            (
                t.name().to_string(),
                t.file().to_path_buf(),
                t.file_channel(),
            )
        })
        .collect();

    let cache = state.waveform_cache.clone();
    let song_name = name.clone();
    let peaks_result = tokio::task::spawn_blocking(move || {
        let peaks = ws_state::compute_waveform_peaks(&track_infos);
        cache.lock().insert(song_name, peaks.clone());
        peaks
    })
    .await;

    match peaks_result {
        Ok(peaks) => {
            let tracks: Vec<serde_json::Value> = peaks
                .iter()
                .map(|(track_name, p)| json!({ "name": track_name, "peaks": p }))
                .collect();
            (
                StatusCode::OK,
                Json(json!({ "song_name": name, "tracks": tracks })),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to compute waveform: {}", e)})),
        )
            .into_response(),
    }
}

/// GET /api/songs/:name/files — lists files in a song's directory.
///
/// Returns audio, MIDI, and lighting files with type classification.
/// Uses the same song lookup as other endpoints to resolve the correct base_path,
/// supporting songs in nested subdirectories.
pub(super) async fn get_song_files(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let all_songs = state.player.songs();
    let song = match all_songs.get(&name) {
        Ok(s) => s,
        Err(_) => {
            return (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Song not found: {}", name)})),
            )
                .into_response();
        }
    };

    let song_dir = song.base_path();

    let mut files: Vec<serde_json::Value> = Vec::new();
    match std::fs::read_dir(song_dir) {
        Ok(entries) => {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let filename = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                // Skip song config files
                if filename == "song.yaml" || filename == "song.yml" {
                    continue;
                }
                let ext = path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                let file_type = if songs::is_supported_audio_extension(&ext) {
                    "audio"
                } else if ext == "mid" {
                    "midi"
                } else if ext == "light" {
                    "lighting"
                } else {
                    "other"
                };
                files.push(json!({
                    "name": filename,
                    "type": file_type,
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

    files.sort_by(|a, b| {
        a.get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .cmp(b.get("name").and_then(|v| v.as_str()).unwrap_or(""))
    });

    (StatusCode::OK, Json(json!({"files": files}))).into_response()
}

/// POST /api/songs/:name — creates a new song with the given config YAML.
///
/// Creates the song directory and writes song.yaml. Returns 409 Conflict if the
/// song directory already exists.
pub(super) async fn post_song(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    body: String,
) -> impl IntoResponse {
    let song_dir = match ensure_song_dir(&state.songs_path, &name) {
        Ok(dir) => dir,
        Err(e) => return *e,
    };

    let config_path = song_dir.join("song.yaml");
    if config_path.exists() {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": format!("Song already exists: {}", name)})),
        )
            .into_response();
    }

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
        Ok(()) => {
            state.player.reload_songs(
                &state.songs_path,
                state.playlists_dir.as_deref(),
                state.legacy_playlist_path.as_deref(),
            );
            (
                StatusCode::CREATED,
                Json(json!({"status": "created", "song": name})),
            )
                .into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response(),
    }
}

/// PUT /api/songs/:name — validates and atomically writes a song config.
///
/// The song directory must already exist (created via POST or track upload).
pub(super) async fn put_song(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    body: String,
) -> impl IntoResponse {
    if name.is_empty()
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Invalid song name"})),
        )
            .into_response();
    }

    // Canonicalize songs_path and verify the song directory stays within it.
    let songs_canonical = match state.songs_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to resolve songs path: {}", e)})),
            )
                .into_response();
        }
    };
    let song_dir = songs_canonical.join(&name);
    if !song_dir.is_dir() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Song not found: {}", name)})),
        )
            .into_response();
    }
    // codeql[rust/path-injection] Path verified via canonicalize + starts_with.
    let song_dir = match song_dir.canonicalize() {
        Ok(p) if p.starts_with(&songs_canonical) => p,
        Ok(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Invalid song name"})),
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to resolve song path: {}", e)})),
            )
                .into_response();
        }
    };

    let config_path = song_dir.join("song.yaml");

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
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e}))).into_response(),
    }
}

/// PUT /api/songs/:name/tracks/:filename — uploads a single track file.
///
/// The request body is the raw file content. Creates the song directory and
/// song.yaml if they don't exist.
pub(super) async fn upload_track_single(
    State(state): State<WebUiState>,
    Path((name, filename)): Path<(String, String)>,
    body: Bytes,
) -> impl IntoResponse {
    validate_track_filename(&filename).map_err(|e| *e)?;
    let song_dir = ensure_song_dir(&state.songs_path, &name).map_err(|e| *e)?;

    let file_path = song_dir.join(&filename);
    if let Err(e) = std::fs::write(&file_path, &body) {
        return Err((
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to write file: {}", e)})),
        )
            .into_response());
    }

    ensure_song_config(&song_dir).map_err(|e| *e)?;
    state.player.reload_songs(
        &state.songs_path,
        state.playlists_dir.as_deref(),
        state.legacy_playlist_path.as_deref(),
    );

    Ok((
        StatusCode::OK,
        Json(json!({
            "status": "uploaded",
            "song": name,
            "file": filename,
        })),
    ))
}

/// POST /api/songs/:name/tracks — uploads multiple track files via multipart form.
///
/// Creates the song directory and song.yaml if they don't exist.
pub(super) async fn upload_tracks_multipart(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    mut multipart: Multipart,
) -> impl IntoResponse {
    let song_dir = ensure_song_dir(&state.songs_path, &name).map_err(|e| *e)?;

    let mut uploaded: Vec<String> = Vec::new();

    while let Some(field) = multipart.next_field().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": format!("Failed to read multipart field: {}", e)})),
        )
            .into_response()
    })? {
        let filename = match field.file_name() {
            Some(name) => name.to_string(),
            None => {
                continue;
            }
        };

        validate_track_filename(&filename).map_err(|e| *e)?;

        let data = field.bytes().await.map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Failed to read file data: {}", e)})),
            )
                .into_response()
        })?;

        let file_path = song_dir.join(&filename);
        std::fs::write(&file_path, &data).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to write file {}: {}", filename, e)})),
            )
                .into_response()
        })?;

        uploaded.push(filename);
    }

    if uploaded.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "No files uploaded"})),
        )
            .into_response());
    }

    ensure_song_config(&song_dir).map_err(|e| *e)?;
    state.player.reload_songs(
        &state.songs_path,
        state.playlists_dir.as_deref(),
        state.legacy_playlist_path.as_deref(),
    );

    Ok((
        StatusCode::OK,
        Json(json!({
            "status": "uploaded",
            "song": name,
            "files": uploaded,
        })),
    ))
}

// ---------------------------------------------------------------------------
// Song helper functions
// ---------------------------------------------------------------------------

/// Ensures a song directory exists and returns its path.
/// Creates the directory if it doesn't exist. Returns an error response if the
/// song name is invalid or the directory can't be created.
pub(super) fn ensure_song_dir(
    songs_path: &std::path::Path,
    name: &str,
) -> Result<std::path::PathBuf, Box<axum::response::Response>> {
    // Reject path traversal characters.
    if name.is_empty()
        || name.contains("..")
        || name.contains('/')
        || name.contains('\\')
        || name.contains('\0')
    {
        return Err(Box::new(
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Invalid song name"})),
            )
                .into_response(),
        ));
    }

    // Canonicalize the songs directory so all derived paths are anchored to a
    // verified absolute base.
    let songs_canonical = songs_path.canonicalize().map_err(|e| {
        Box::new(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to resolve songs path: {}", e)})),
            )
                .into_response(),
        )
    })?;

    let song_dir = songs_canonical.join(name);

    if !song_dir.exists() {
        std::fs::create_dir_all(&song_dir).map_err(|e| {
            Box::new(
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": format!("Failed to create song directory: {}", e)})),
                )
                    .into_response(),
            )
        })?;
    }

    // Canonicalize the result and verify containment within songs directory.
    // codeql[rust/path-injection] Path verified via canonicalize + starts_with.
    let song_canonical = song_dir.canonicalize().map_err(|e| {
        Box::new(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to resolve song directory: {}", e)})),
            )
                .into_response(),
        )
    })?;
    if !song_canonical.starts_with(&songs_canonical) {
        return Err(Box::new(
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Invalid song name"})),
            )
                .into_response(),
        ));
    }

    Ok(song_canonical)
}

/// Generates song.yaml for a song directory if one doesn't already exist.
/// If song.yaml already exists, it is left untouched so that manual edits
/// (track names, lighting config, etc.) are preserved.
pub(super) fn ensure_song_config(
    song_dir: &std::path::Path,
) -> Result<(), Box<axum::response::Response>> {
    let config_path = song_dir.join("song.yaml");
    if config_path.exists() {
        return Ok(());
    }

    let song = songs::Song::initialize(&song_dir.to_path_buf()).map_err(|e| {
        Box::new(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to initialize song: {}", e)})),
            )
                .into_response(),
        )
    })?;

    song.get_config().save(&config_path).map_err(|e| {
        Box::new(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to save song config: {}", e)})),
            )
                .into_response(),
        )
    })
}

/// Validates that a track filename has a supported extension.
pub(super) fn validate_track_filename(filename: &str) -> Result<(), Box<axum::response::Response>> {
    if filename.contains("..") || filename.contains('/') || filename.contains('\\') {
        return Err(Box::new(
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Invalid filename"})),
            )
                .into_response(),
        ));
    }

    let ext = std::path::Path::new(filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");
    if ext != "mid" && ext != "light" && !songs::is_supported_audio_extension(ext) {
        return Err(Box::new(
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Unsupported file type: .{}", ext)})),
            )
                .into_response(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod test {
    use super::super::router;
    use super::super::test_helpers::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn get_songs_empty_registry() {
        let songs = std::sync::Arc::new(crate::songs::Songs::new(std::collections::HashMap::new()));
        let (state, _dir) = test_state_with_registry(songs);
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/songs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["songs"].is_array());
        assert!(parsed["songs"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn get_songs_returns_registry_contents() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/songs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let songs = parsed["songs"].as_array().unwrap();
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0]["name"], "Song A");
    }

    #[tokio::test]
    async fn get_song_not_found() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/songs/nonexistent")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn put_song_missing_songs_dir() {
        let (mut state, _dir) = test_state();
        state.songs_path = std::path::PathBuf::from("/nonexistent/songs");
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/songs/anything")
                    .body(Body::from("name: test\n"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn put_song_not_found() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/songs/nonexistent")
                    .body(Body::from("name: test\n"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_songs_with_wav_files() {
        let (state, _dir) = test_state();
        let song_dir = state.songs_path.join("TestSong");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: TestSong\ntracks:\n  - name: track1\n    file: track1.wav\n",
        )
        .unwrap();

        state.player.reload_songs(
            &state.songs_path,
            state.playlists_dir.as_deref(),
            state.legacy_playlist_path.as_deref(),
        );

        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/songs")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let songs = parsed["songs"].as_array().unwrap();
        assert_eq!(songs.len(), 1);
        assert_eq!(songs[0]["name"], "TestSong");
        assert!(songs[0]["tracks"].is_array());
    }

    #[tokio::test]
    async fn get_song_with_config_file() {
        let (state, _dir) = test_state();
        let song_dir = state.songs_path.join("MySong");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();
        let song_yaml = "name: MySong\ntracks:\n  - name: track1\n    file: track1.wav\n";
        std::fs::write(song_dir.join("song.yaml"), song_yaml).unwrap();

        state.player.reload_songs(
            &state.songs_path,
            state.playlists_dir.as_deref(),
            state.legacy_playlist_path.as_deref(),
        );
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/songs/MySong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        assert!(body.contains("MySong"));
    }

    #[tokio::test]
    async fn get_song_no_config_file_returns_json_summary() {
        let (state, _dir) = test_state();
        let song_dir = state.songs_path.join("NoConfig");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: NoConfig\ntracks:\n  - name: track1\n    file: track1.wav\n",
        )
        .unwrap();

        state.player.reload_songs(
            &state.songs_path,
            state.playlists_dir.as_deref(),
            state.legacy_playlist_path.as_deref(),
        );

        std::fs::rename(song_dir.join("song.yaml"), song_dir.join("song.bak")).unwrap();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/songs/NoConfig")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["config_file"], false);
        assert_eq!(parsed["name"], "NoConfig");
    }

    #[tokio::test]
    async fn get_song_with_yml_extension() {
        let (state, _dir) = test_state();
        let song_dir = state.songs_path.join("YmlSong");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();
        let song_yaml = "name: YmlSong\ntracks:\n  - name: track1\n    file: track1.wav\n";
        std::fs::write(song_dir.join("song.yml"), song_yaml).unwrap();

        state.player.reload_songs(
            &state.songs_path,
            state.playlists_dir.as_deref(),
            state.legacy_playlist_path.as_deref(),
        );
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/songs/YmlSong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        assert!(body.contains("YmlSong"));
    }

    #[tokio::test]
    async fn put_song_valid() {
        let (state, _dir) = test_state();
        let song_dir = state.songs_path.join("EditSong");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: EditSong\ntracks:\n  - name: track1\n    file: track1.wav\n",
        )
        .unwrap();

        let new_yaml = "name: EditSong\ntracks:\n  - name: track1\n    file: track1.wav\n";
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/songs/EditSong")
                    .body(Body::from(new_yaml))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn put_song_invalid_yaml() {
        let (state, _dir) = test_state();
        let song_dir = state.songs_path.join("BadSong");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: BadSong\ntracks:\n  - name: track1\n    file: track1.wav\n",
        )
        .unwrap();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/songs/BadSong")
                    .body(Body::from("invalid yaml: [[["))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn get_song_no_config_yaml_returns_summary() {
        let (state, _dir) = test_state();
        let song_dir = state.songs_path.join("CustomSong");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();
        std::fs::write(
            song_dir.join("config.yaml"),
            "name: CustomSong\ntracks:\n  - name: track1\n    file: track1.wav\n",
        )
        .unwrap();

        state.player.reload_songs(
            &state.songs_path,
            state.playlists_dir.as_deref(),
            state.legacy_playlist_path.as_deref(),
        );
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/songs/CustomSong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["config_file"], false);
        assert_eq!(parsed["name"], "CustomSong");
    }

    #[tokio::test]
    async fn get_song_with_yml_config() {
        let (state, _dir) = test_state();
        let song_dir = state.songs_path.join("YmlOnlySong");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();
        std::fs::write(
            song_dir.join("song.yml"),
            "name: YmlOnlySong\ntracks:\n  - name: track1\n    file: track1.wav\n",
        )
        .unwrap();

        state.player.reload_songs(
            &state.songs_path,
            state.playlists_dir.as_deref(),
            state.legacy_playlist_path.as_deref(),
        );
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/songs/YmlOnlySong")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        assert!(body.contains("YmlOnlySong"));
    }

    #[tokio::test]
    async fn get_song_not_in_registry_returns_not_found() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/songs/anything")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["error"].as_str().unwrap().contains("Song not found"));
    }

    #[tokio::test]
    async fn get_song_not_found_body_contains_name() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/songs/FakeSong")
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
            .contains("Song not found: FakeSong"));
    }

    #[tokio::test]
    async fn put_song_songs_dir_failure_returns_500() {
        let (mut state, _dir) = test_state();
        state.songs_path = std::path::PathBuf::from("/nonexistent/songs");
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/songs/whatever")
                    .body(Body::from("name: whatever\n"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn put_song_not_found_body_contains_name() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/songs/DoesNotExist")
                    .body(Body::from("name: DoesNotExist\n"))
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
            .contains("Song not found: DoesNotExist"));
    }

    #[tokio::test]
    async fn upload_track_single_creates_song() {
        let (state, _dir) = test_state();
        let app = router().with_state(state.clone());

        let wav_bytes = create_test_wav();

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/songs/NewSong/tracks/track1.wav")
                    .body(Body::from(wav_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["status"], "uploaded");
        assert_eq!(parsed["song"], "NewSong");
        assert_eq!(parsed["file"], "track1.wav");

        assert!(state.songs_path.join("NewSong/track1.wav").exists());
        assert!(state.songs_path.join("NewSong/song.yaml").exists());
    }

    #[tokio::test]
    async fn upload_track_single_path_traversal_rejected() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/songs/..%2F..%2Fetc/tracks/passwd")
                    .body(Body::from("bad"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn upload_track_single_unsupported_extension() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/songs/TestSong/tracks/file.txt")
                    .body(Body::from("data"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        assert!(body.contains("Unsupported file type"));
    }

    #[tokio::test]
    async fn upload_tracks_multipart_creates_song() {
        let (state, _dir) = test_state();
        let app = router().with_state(state.clone());

        let wav_bytes = create_test_wav();
        let boundary = "----testboundary123";
        let mut body_bytes = Vec::new();
        body_bytes.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file1\"; filename=\"track1.wav\"\r\nContent-Type: application/octet-stream\r\n\r\n"
            )
            .as_bytes(),
        );
        body_bytes.extend_from_slice(&wav_bytes);
        body_bytes.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/songs/MultiSong/tracks")
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["status"], "uploaded");
        assert_eq!(parsed["song"], "MultiSong");
        assert_eq!(parsed["files"][0], "track1.wav");

        assert!(state.songs_path.join("MultiSong/track1.wav").exists());
        assert!(state.songs_path.join("MultiSong/song.yaml").exists());
    }

    #[tokio::test]
    async fn upload_tracks_multipart_empty_rejects() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let boundary = "----testboundary456";
        let body_bytes = format!("--{boundary}--\r\n");

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/songs/EmptySong/tracks")
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(Body::from(body_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        assert!(body.contains("No files uploaded"));
    }

    #[tokio::test]
    async fn upload_track_single_adds_to_existing_song() {
        let (state, _dir) = test_state();

        let song_dir = state.songs_path.join("ExistingSong");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();

        let app = router().with_state(state.clone());
        let wav_bytes = create_test_wav();

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/songs/ExistingSong/tracks/track2.wav")
                    .body(Body::from(wav_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(song_dir.join("track1.wav").exists());
        assert!(song_dir.join("track2.wav").exists());
        assert!(song_dir.join("song.yaml").exists());
    }

    #[tokio::test]
    async fn post_song_creates_song() {
        let (state, _dir) = test_state();
        let app = router().with_state(state.clone());

        let yaml = "name: Brand New Song\ntracks: []\n";

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/songs/BrandNew")
                    .body(Body::from(yaml))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["status"], "created");
        assert_eq!(parsed["song"], "BrandNew");

        assert!(state.songs_path.join("BrandNew").is_dir());
        assert!(state.songs_path.join("BrandNew/song.yaml").exists());
    }

    #[tokio::test]
    async fn post_song_conflict_if_exists() {
        let (state, _dir) = test_state();

        let song_dir = state.songs_path.join("Existing");
        std::fs::create_dir(&song_dir).unwrap();
        std::fs::write(song_dir.join("song.yaml"), "name: Existing\ntracks: []\n").unwrap();

        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/songs/Existing")
                    .body(Body::from("name: Existing\ntracks: []\n"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn post_song_invalid_yaml() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/songs/BadSong")
                    .body(Body::from("not valid: [[["))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn post_song_then_put_updates_config() {
        let (state, _dir) = test_state();

        let app = router().with_state(state.clone());
        let yaml = "name: MySong\ntracks: []\n";
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/songs/MySong")
                    .body(Body::from(yaml))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let app = router().with_state(state.clone());
        let updated_yaml = "name: MySong Renamed\ntracks: []\n";
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/songs/MySong")
                    .body(Body::from(updated_yaml))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let content = std::fs::read_to_string(state.songs_path.join("MySong/song.yaml")).unwrap();
        assert!(content.contains("MySong Renamed"));
    }

    #[tokio::test]
    async fn post_song_then_upload_preserves_config() {
        let (state, _dir) = test_state();

        let yaml = "name: My Custom Song\ntracks:\n  - name: Lead Guitar\n    file: guitar.wav\n";
        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/songs/CustomSong")
                    .body(Body::from(yaml))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let wav_bytes = create_test_wav();
        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/songs/CustomSong/tracks/guitar.wav")
                    .body(Body::from(wav_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let content =
            std::fs::read_to_string(state.songs_path.join("CustomSong/song.yaml")).unwrap();
        assert!(content.contains("My Custom Song"));
        assert!(content.contains("Lead Guitar"));
    }
}
