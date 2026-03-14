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
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde_json::json;

use std::path::PathBuf;

use tracing::warn;

use super::config_io;
use super::server::WebUiState;
use super::state as ws_state;
use crate::{audio, calibrate, config, lighting, midi, songs};

/// Builds the API router for config read/write endpoints.
///
/// Playback control is handled via gRPC-Web (PlayerService), not REST.
pub fn router() -> Router<WebUiState> {
    Router::new()
        .route("/config", get(get_config_raw).put(put_config))
        .route("/config/parsed", get(get_config_parsed))
        .route("/config/validate", post(validate_config))
        .route("/songs", get(get_songs))
        .route("/songs/{name}", get(get_song).post(post_song).put(put_song))
        .route("/songs/{name}/tracks/{filename}", put(upload_track_single))
        .route("/songs/{name}/tracks", post(upload_tracks_multipart))
        .route("/songs/{name}/waveform", get(get_song_waveform))
        .route("/songs/{name}/files", get(get_song_files))
        .route("/songs/{name}/import", post(import_file_to_song))
        .route("/browse", get(browse_directory))
        .route("/browse/create-song", post(create_song_in_directory))
        .route("/playlist", get(get_playlist).put(put_playlist))
        .route("/playlist/validate", post(validate_playlist))
        .route("/playlists", get(get_playlists))
        .route(
            "/playlists/{name}",
            get(get_playlist_by_name)
                .put(put_playlist_by_name)
                .delete(delete_playlist_by_name),
        )
        .route("/playlists/{name}/activate", post(activate_playlist))
        .route("/lighting", get(get_lighting_files))
        .route(
            "/lighting/{name}",
            get(get_lighting_file).put(put_lighting_file),
        )
        .route("/lighting/validate", post(validate_lighting))
        .route("/config/store", get(get_config_store))
        .route("/config/audio", put(put_config_audio))
        .route("/config/midi", put(put_config_midi))
        .route("/config/dmx", put(put_config_dmx))
        .route("/config/controllers", put(put_config_controllers))
        .route("/config/samples", put(put_config_samples))
        .route("/samples/upload/{filename}", put(upload_sample_file))
        .route("/config/profiles", post(post_config_profile))
        .route(
            "/config/profiles/{index}",
            put(put_config_profile).delete(delete_config_profile),
        )
        .route("/devices/audio", get(get_audio_devices))
        .route("/devices/midi", get(get_midi_devices))
        .route("/calibrate/start", post(post_calibrate_start))
        .route("/calibrate/capture", post(post_calibrate_capture))
        .route("/calibrate/stop", post(post_calibrate_stop))
        .route("/calibrate", delete(delete_calibrate))
        .route("/lighting/fixture-types", get(get_fixture_types))
        .route(
            "/lighting/fixture-types/{name}",
            get(get_fixture_type)
                .put(put_fixture_type)
                .delete(delete_fixture_type),
        )
        .route("/lighting/venues", get(get_venues))
        .route(
            "/lighting/venues/{name}",
            get(get_venue).put(put_venue).delete(delete_venue),
        )
}

/// GET /api/config — returns the raw YAML content of the player config file.
async fn get_config_raw(State(state): State<WebUiState>) -> impl IntoResponse {
    // codeql[rust/path-injection] config_path is set at startup, not from user input.
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
///
/// Uses the player's song registry rather than rescanning disk, so the API
/// is always consistent with what the player knows about.
async fn get_songs(State(state): State<WebUiState>) -> impl IntoResponse {
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
async fn get_song(State(state): State<WebUiState>, Path(name): Path<String>) -> impl IntoResponse {
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
async fn import_file_to_song(
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
struct ImportFileRequest {
    path: String,
    filename: Option<String>,
}

/// GET /api/songs/:name/waveform — returns waveform peaks for a song.
///
/// Uses the shared waveform cache; computes on demand if not cached.
async fn get_song_waveform(
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
async fn get_song_files(
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

/// GET /api/browse?path=... — lists files and directories at a filesystem path.
///
/// Restricted to the directory containing mtrack.yaml (the config root).
/// If no `path` query parameter is provided, defaults to the config root.
/// Returns entries sorted: directories first, then files alphabetically.
async fn browse_directory(
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
struct BrowseParams {
    #[serde(default)]
    path: String,
}

/// POST /api/browse/create-song — auto-generates a song.yaml in a project-relative directory.
///
/// Expects a JSON body with `path` (project-relative directory, e.g. "/songs/Afar")
/// and an optional `name` override. The backend scans the directory for audio/MIDI/lighting
/// files and generates the song config automatically, including per-channel track splitting
/// for stereo and multichannel audio files.
async fn create_song_in_directory(
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
    // channel-aware track splitting (stereo → L/R, multichannel → per-channel).
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
struct CreateSongInDirRequest {
    path: String,
    name: Option<String>,
}

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
async fn get_playlist(State(state): State<WebUiState>) -> impl IntoResponse {
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

/// Helper to convert a ConfigError into an HTTP response.
fn config_store_error_response(e: config::ConfigError) -> axum::response::Response {
    match e {
        config::ConfigError::StaleChecksum { .. } => {
            (StatusCode::CONFLICT, Json(json!({"error": e.to_string()}))).into_response()
        }
        config::ConfigError::InvalidProfileIndex { .. } => (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
        _ => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response(),
    }
}

/// Helper to convert a ConfigSnapshot into a JSON response.
fn config_snapshot_response(
    snapshot: config::store::ConfigSnapshot,
    status: StatusCode,
) -> axum::response::Response {
    (
        status,
        Json(json!({"yaml": snapshot.yaml, "checksum": snapshot.checksum})),
    )
        .into_response()
}

/// Returns the config store from the player, or an error response.
#[allow(clippy::result_large_err)]
fn require_config_store(
    state: &WebUiState,
) -> Result<std::sync::Arc<crate::config::ConfigStore>, axum::response::Response> {
    state.player.config_store().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "config store not available"})),
        )
            .into_response()
    })
}

/// Returns a 409 Conflict response if the player is currently playing.
async fn reject_if_playing(state: &WebUiState) -> Option<axum::response::Response> {
    if state.player.is_playing().await {
        Some(
            (
                StatusCode::CONFLICT,
                Json(json!({"error": "Cannot modify hardware config during playback"})),
            )
                .into_response(),
        )
    } else {
        None
    }
}

/// Reloads hardware from the updated config. Non-blocking — spawns async
/// device discovery and returns immediately. The broadcast channel is already
/// stored on the Player and will be wired when the DMX engine comes up.
async fn reload_hardware_after_mutation(state: &WebUiState) {
    if let Err(e) = state.player.reload_hardware().await {
        warn!("Hardware reload failed: {}", e);
    }
}

/// GET /api/config/store — returns config YAML + checksum via the ConfigStore.
async fn get_config_store(State(state): State<WebUiState>) -> impl IntoResponse {
    let store = match require_config_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match store.read_yaml().await {
        Ok((yaml, checksum)) => (
            StatusCode::OK,
            Json(json!({"yaml": yaml, "checksum": checksum})),
        )
            .into_response(),
        Err(e) => config_store_error_response(e),
    }
}

/// PUT /api/config/audio — update audio config section.
async fn put_config_audio(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_playing(&state).await {
        return resp;
    }

    let checksum = match body.get("expected_checksum").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing expected_checksum"})),
            )
                .into_response()
        }
    };

    let audio: Option<config::Audio> = match body.get("audio") {
        Some(v) if !v.is_null() => match serde_json::from_value(v.clone()) {
            Ok(a) => Some(a),
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("invalid audio: {}", e)})),
                )
                    .into_response()
            }
        },
        _ => None,
    };

    let store = match require_config_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match store.update_audio(audio, &checksum).await {
        Ok(snapshot) => {
            reload_hardware_after_mutation(&state).await;
            config_snapshot_response(snapshot, StatusCode::OK)
        }
        Err(e) => config_store_error_response(e),
    }
}

/// PUT /api/config/midi — update MIDI config section.
async fn put_config_midi(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_playing(&state).await {
        return resp;
    }

    let checksum = match body.get("expected_checksum").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing expected_checksum"})),
            )
                .into_response()
        }
    };

    let midi: Option<config::Midi> = match body.get("midi") {
        Some(v) if !v.is_null() => match serde_json::from_value(v.clone()) {
            Ok(m) => Some(m),
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("invalid midi: {}", e)})),
                )
                    .into_response()
            }
        },
        _ => None,
    };

    let store = match require_config_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match store.update_midi(midi, &checksum).await {
        Ok(snapshot) => {
            reload_hardware_after_mutation(&state).await;
            config_snapshot_response(snapshot, StatusCode::OK)
        }
        Err(e) => config_store_error_response(e),
    }
}

/// PUT /api/config/dmx — update DMX config section.
async fn put_config_dmx(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_playing(&state).await {
        return resp;
    }

    let checksum = match body.get("expected_checksum").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing expected_checksum"})),
            )
                .into_response()
        }
    };

    let dmx: Option<config::Dmx> = match body.get("dmx") {
        Some(v) if !v.is_null() => match serde_json::from_value(v.clone()) {
            Ok(d) => Some(d),
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("invalid dmx: {}", e)})),
                )
                    .into_response()
            }
        },
        _ => None,
    };

    let store = match require_config_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match store.update_dmx(dmx, &checksum).await {
        Ok(snapshot) => {
            reload_hardware_after_mutation(&state).await;
            config_snapshot_response(snapshot, StatusCode::OK)
        }
        Err(e) => config_store_error_response(e),
    }
}

/// PUT /api/config/controllers — update controllers config.
async fn put_config_controllers(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_playing(&state).await {
        return resp;
    }

    let checksum = match body.get("expected_checksum").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing expected_checksum"})),
            )
                .into_response()
        }
    };

    let controllers: Vec<config::Controller> = match body.get("controllers") {
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(c) => c,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("invalid controllers: {}", e)})),
                )
                    .into_response()
            }
        },
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing controllers field"})),
            )
                .into_response()
        }
    };

    let store = match require_config_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match store.update_controllers(controllers, &checksum).await {
        Ok(snapshot) => {
            reload_hardware_after_mutation(&state).await;
            config_snapshot_response(snapshot, StatusCode::OK)
        }
        Err(e) => config_store_error_response(e),
    }
}

/// PUT /api/config/samples — replace all sample definitions.
async fn put_config_samples(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_playing(&state).await {
        return resp;
    }

    let checksum = match body.get("expected_checksum").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing expected_checksum"})),
            )
                .into_response()
        }
    };

    let samples: std::collections::HashMap<String, config::SampleDefinition> =
        match body.get("samples") {
            Some(v) => match serde_json::from_value(v.clone()) {
                Ok(s) => s,
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        Json(json!({"error": format!("invalid samples: {}", e)})),
                    )
                        .into_response()
                }
            },
            None => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "missing samples field"})),
                )
                    .into_response()
            }
        };

    let store = match require_config_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match store.update_samples(samples, &checksum).await {
        Ok(snapshot) => {
            reload_hardware_after_mutation(&state).await;
            config_snapshot_response(snapshot, StatusCode::OK)
        }
        Err(e) => config_store_error_response(e),
    }
}

/// POST /api/config/profiles — add a new profile.
async fn post_config_profile(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_playing(&state).await {
        return resp;
    }

    let checksum = match body.get("expected_checksum").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing expected_checksum"})),
            )
                .into_response()
        }
    };

    let profile: config::Profile = match body.get("profile") {
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(p) => p,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("invalid profile: {}", e)})),
                )
                    .into_response()
            }
        },
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing profile field"})),
            )
                .into_response()
        }
    };

    let store = match require_config_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match store.add_profile(profile, &checksum).await {
        Ok(snapshot) => {
            reload_hardware_after_mutation(&state).await;
            config_snapshot_response(snapshot, StatusCode::CREATED)
        }
        Err(e) => config_store_error_response(e),
    }
}

/// PUT /api/config/profiles/:index — update profile at index.
async fn put_config_profile(
    State(state): State<WebUiState>,
    Path(index): Path<usize>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_playing(&state).await {
        return resp;
    }

    let checksum = match body.get("expected_checksum").and_then(|v| v.as_str()) {
        Some(c) => c.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing expected_checksum"})),
            )
                .into_response()
        }
    };

    let profile: config::Profile = match body.get("profile") {
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(p) => p,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("invalid profile: {}", e)})),
                )
                    .into_response()
            }
        },
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing profile field"})),
            )
                .into_response()
        }
    };

    let store = match require_config_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match store.update_profile(index, profile, &checksum).await {
        Ok(snapshot) => {
            reload_hardware_after_mutation(&state).await;
            config_snapshot_response(snapshot, StatusCode::OK)
        }
        Err(e) => config_store_error_response(e),
    }
}

/// DELETE /api/config/profiles/:index?expected_checksum=... — remove profile at index.
async fn delete_config_profile(
    State(state): State<WebUiState>,
    Path(index): Path<usize>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_playing(&state).await {
        return resp;
    }

    let checksum = match params.get("expected_checksum") {
        Some(c) => c.to_string(),
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing expected_checksum query parameter"})),
            )
                .into_response()
        }
    };

    let store = match require_config_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match store.remove_profile(index, &checksum).await {
        Ok(snapshot) => {
            reload_hardware_after_mutation(&state).await;
            config_snapshot_response(snapshot, StatusCode::OK)
        }
        Err(e) => config_store_error_response(e),
    }
}

/// GET /api/devices/audio — lists available audio devices.
async fn get_audio_devices() -> impl IntoResponse {
    match tokio::task::spawn_blocking(|| audio::list_device_info().map_err(|e| e.to_string())).await
    {
        Ok(Ok(devices)) => (StatusCode::OK, Json(json!(devices))).into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("failed to list audio devices: {}", e)})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("task failed: {}", e)})),
        )
            .into_response(),
    }
}

/// GET /api/devices/midi — lists available MIDI devices.
async fn get_midi_devices() -> impl IntoResponse {
    match tokio::task::spawn_blocking(|| midi::list_device_info().map_err(|e| e.to_string())).await
    {
        Ok(Ok(devices)) => (StatusCode::OK, Json(json!(devices))).into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("failed to list MIDI devices: {}", e)})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("task failed: {}", e)})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Calibration endpoints
// ---------------------------------------------------------------------------

/// Server-side state for an in-progress calibration session.
pub(crate) struct CalibrationSession {
    device: cpal::Device,
    stream_config: cpal::StreamConfig,
    stream_format: cpal::SampleFormat,
    num_device_channels: u16,
    target_channel: u16,
    sample_rate: u32,
    #[allow(dead_code)]
    device_name: String,
    noise_floor: calibrate::NoiseFloorStats,
    // Phase 2 state
    hit_buffer: Option<std::sync::Arc<calibrate::CaptureBuffer>>,
    hit_stream: Option<cpal::Stream>,
}

#[derive(serde::Deserialize)]
struct CalibrateStartRequest {
    device: String,
    channel: u16,
    #[serde(default = "default_noise_duration")]
    duration: f32,
    sample_rate: Option<u32>,
    /// "int" or "float"
    sample_format: Option<String>,
    bits_per_sample: Option<u16>,
}

fn default_noise_duration() -> f32 {
    3.0
}

/// POST /api/calibrate/start — begins noise floor measurement.
///
/// Blocks for `duration` seconds while capturing audio, then returns
/// the noise floor stats for the target channel.
async fn post_calibrate_start(
    State(state): State<WebUiState>,
    Json(body): Json<CalibrateStartRequest>,
) -> impl IntoResponse {
    use cpal::traits::StreamTrait;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    if body.channel < 1 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "channel must be >= 1"})),
        )
            .into_response();
    }

    let duration = body.duration.clamp(0.5, 30.0);
    let device_name = body.device.clone();
    let target_channel = body.channel;
    let sample_rate_opt = body.sample_rate;
    let sample_format_opt: Option<crate::audio::format::SampleFormat> =
        body.sample_format.as_deref().and_then(|s| s.parse().ok());
    let bits_per_sample_opt = body.bits_per_sample;

    let result = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let device =
            audio::find_input_device(&device_name).map_err(|e| format!("Device not found: {e}"))?;

        let cal_config = calibrate::CalibrationConfig {
            device_name: device_name.clone(),
            sample_rate: sample_rate_opt,
            noise_floor_duration_secs: duration,
            sample_format: sample_format_opt,
            bits_per_sample: bits_per_sample_opt,
        };

        let (channels, sample_rate, stream_format) =
            calibrate::resolve_stream_params(&device, &cal_config)
                .map_err(|e| format!("Failed to resolve stream params: {e}"))?;

        if target_channel > channels {
            return Err(format!(
                "Channel {target_channel} exceeds device's {channels} channels"
            ));
        }

        let stream_config = cpal::StreamConfig {
            channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let expected_samples = (duration * sample_rate as f32) as usize + 1024;
        let buffer = Arc::new(calibrate::CaptureBuffer {
            channels: (0..channels)
                .map(|_| parking_lot::Mutex::new(Vec::with_capacity(expected_samples)))
                .collect(),
            active: AtomicBool::new(true),
        });

        let stream = calibrate::build_capture_stream(
            &device,
            &stream_config,
            buffer.clone(),
            channels,
            stream_format,
        )
        .map_err(|e| format!("Failed to build capture stream: {e}"))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start stream: {e}"))?;
        std::thread::sleep(std::time::Duration::from_secs_f32(duration));
        buffer.active.store(false, Ordering::Relaxed);
        drop(stream);

        // Extract target channel samples and compute noise floor
        let ch_idx = (target_channel - 1) as usize;
        let samples = std::mem::take(&mut *buffer.channels[ch_idx].lock());
        let noise_floor = calibrate::analyze_noise_floor(&samples, sample_rate);

        Ok((
            device,
            stream_config,
            stream_format,
            channels,
            sample_rate,
            device_name,
            noise_floor,
        ))
    })
    .await;

    match result {
        Ok(Ok((
            device,
            stream_config,
            stream_format,
            channels,
            sample_rate,
            device_name,
            noise_floor,
        ))) => {
            let response = json!({
                "peak": noise_floor.peak,
                "rms": noise_floor.rms,
                "low_freq_energy": noise_floor.low_freq_energy,
                "channel": target_channel,
                "sample_rate": sample_rate,
                "device_channels": channels,
            });

            let session = CalibrationSession {
                device,
                stream_config,
                stream_format,
                num_device_channels: channels,
                target_channel,
                sample_rate,
                device_name,
                noise_floor,
                hit_buffer: None,
                hit_stream: None,
            };

            // Replace any existing session
            *state.calibration.lock() = Some(session);

            (StatusCode::OK, Json(response)).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(json!({"error": e}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Task failed: {e}")})),
        )
            .into_response(),
    }
}

/// POST /api/calibrate/capture — starts hit capture phase.
async fn post_calibrate_capture(State(state): State<WebUiState>) -> impl IntoResponse {
    use cpal::traits::StreamTrait;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let mut guard = state.calibration.lock();
    let session = match guard.as_mut() {
        Some(s) => s,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "No calibration session — call /calibrate/start first"})),
            )
                .into_response()
        }
    };

    // Stop any existing capture
    session.hit_stream = None;
    session.hit_buffer = None;

    let hit_capacity = (60.0 * session.sample_rate as f32) as usize;
    let buffer = Arc::new(calibrate::CaptureBuffer {
        channels: (0..session.num_device_channels)
            .map(|_| parking_lot::Mutex::new(Vec::with_capacity(hit_capacity)))
            .collect(),
        active: AtomicBool::new(true),
    });

    let stream = match calibrate::build_capture_stream(
        &session.device,
        &session.stream_config,
        buffer.clone(),
        session.num_device_channels,
        session.stream_format,
    ) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to build capture stream: {e}")})),
            )
                .into_response()
        }
    };

    if let Err(e) = stream.play() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to start capture: {e}")})),
        )
            .into_response();
    }

    session.hit_buffer = Some(buffer);
    session.hit_stream = Some(stream);

    (StatusCode::OK, Json(json!({"status": "capturing"}))).into_response()
}

/// POST /api/calibrate/stop — stops capture and returns calibration results.
async fn post_calibrate_stop(State(state): State<WebUiState>) -> impl IntoResponse {
    use std::sync::atomic::Ordering;

    let mut guard = state.calibration.lock();
    let session = match guard.take() {
        Some(s) => s,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "No calibration session active"})),
            )
                .into_response()
        }
    };

    let hit_buffer = match session.hit_buffer {
        Some(b) => b,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Capture not started — call /calibrate/capture first"})),
            )
                .into_response()
        }
    };

    // Stop the stream
    hit_buffer.active.store(false, Ordering::Relaxed);
    drop(session.hit_stream);

    let ch_idx = (session.target_channel - 1) as usize;
    let samples = std::mem::take(&mut *hit_buffer.channels[ch_idx].lock());

    let hits = calibrate::detect_hits(&samples, &session.noise_floor, session.sample_rate);
    let calibration = calibrate::derive_channel_params(
        session.target_channel,
        &session.noise_floor,
        &hits,
        session.sample_rate,
    );

    (StatusCode::OK, Json(json!(calibration))).into_response()
}

/// DELETE /api/calibrate — cancels any in-progress calibration session.
async fn delete_calibrate(State(state): State<WebUiState>) -> impl IntoResponse {
    let mut guard = state.calibration.lock();
    if let Some(mut session) = guard.take() {
        // Stop any active capture
        if let Some(ref buffer) = session.hit_buffer {
            buffer
                .active
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
        session.hit_stream = None;
        session.hit_buffer = None;
    }
    (StatusCode::OK, Json(json!({"status": "cancelled"}))).into_response()
}

/// POST /api/songs/:name — creates a new song with the given config YAML.
///
/// Creates the song directory and writes song.yaml. Returns 409 Conflict if the
/// song directory already exists.
async fn post_song(
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
async fn put_song(
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

/// Ensures a song directory exists and returns its path.
/// Creates the directory if it doesn't exist. Returns an error response if the
/// song name is invalid or the directory can't be created.
fn ensure_song_dir(
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
fn ensure_song_config(song_dir: &std::path::Path) -> Result<(), Box<axum::response::Response>> {
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
fn validate_track_filename(filename: &str) -> Result<(), Box<axum::response::Response>> {
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

/// Validates that a filename has a supported audio extension (for sample uploads).
fn validate_sample_filename(filename: &str) -> Result<(), Box<axum::response::Response>> {
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
    if !songs::is_supported_audio_extension(ext) {
        return Err(Box::new(
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("Unsupported audio file type: .{}", ext)})),
            )
                .into_response(),
        ));
    }

    Ok(())
}

/// PUT /api/samples/upload/:filename — uploads a sample audio file.
///
/// The file is stored in a `samples/` directory next to the config file.
/// Returns the relative path `samples/{filename}` for use in sample definitions.
async fn upload_sample_file(
    State(state): State<WebUiState>,
    Path(filename): Path<String>,
    body: Bytes,
) -> impl IntoResponse {
    validate_sample_filename(&filename).map_err(|e| *e)?;

    // Canonicalize the project root first, then build the samples path from
    // the canonical root so that all filesystem operations use a verified base.
    let project_root = state
        .config_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."));
    let root_canonical = project_root.canonicalize().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to resolve project root: {}", e)})),
        )
            .into_response()
    })?;
    let samples_dir = root_canonical.join("samples");

    if !samples_dir.exists() {
        std::fs::create_dir_all(&samples_dir).map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to create samples directory: {}", e)})),
            )
                .into_response()
        })?;
    }

    let file_path = samples_dir.join(&filename);
    if !file_path.starts_with(&root_canonical) {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Invalid filename"})),
        )
            .into_response());
    }

    std::fs::write(&file_path, &body).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to write file: {}", e)})),
        )
            .into_response()
    })?;

    let relative_path = format!("samples/{}", filename);
    Ok::<_, axum::response::Response>(
        (
            StatusCode::OK,
            Json(json!({
                "status": "uploaded",
                "file": filename,
                "path": relative_path,
            })),
        )
            .into_response(),
    )
}

/// PUT /api/songs/:name/tracks/:filename — uploads a single track file.
///
/// The request body is the raw file content. Creates the song directory and
/// song.yaml if they don't exist.
async fn upload_track_single(
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
async fn upload_tracks_multipart(
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

/// PUT /api/playlist — validates and atomically writes the playlist (backward compat).
async fn put_playlist(State(state): State<WebUiState>, body: String) -> impl IntoResponse {
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
async fn get_playlists(State(state): State<WebUiState>) -> impl IntoResponse {
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
async fn get_playlist_by_name(
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
struct PlaylistBody {
    songs: Vec<String>,
}

/// PUT /api/playlists/:name — create or update a playlist.
async fn put_playlist_by_name(
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
async fn delete_playlist_by_name(
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
async fn activate_playlist(
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
struct LightingDirQuery {
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
async fn get_fixture_types(
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
async fn get_fixture_type(
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
async fn put_fixture_type(
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
async fn delete_fixture_type(
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
async fn get_venues(
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
async fn get_venue(
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
async fn put_venue(
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
async fn delete_venue(
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
    use axum::body::Body;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// Creates a WebUiState with a test player and temp directories.
    /// The player's song registry contains "Song A" (in-memory only, not on disk).
    fn test_state() -> (WebUiState, tempfile::TempDir) {
        use crate::songs::{Song, Songs};
        use std::collections::HashMap;

        let mut map = HashMap::new();
        map.insert(
            "Song A".to_string(),
            std::sync::Arc::new(Song::new_for_test("Song A", &["kick", "snare"])),
        );
        let songs = std::sync::Arc::new(Songs::new(map));
        test_state_with_registry(songs)
    }

    /// Creates a WebUiState with the given song registry.
    fn test_state_with_registry(
        songs: std::sync::Arc<crate::songs::Songs>,
    ) -> (WebUiState, tempfile::TempDir) {
        use crate::player::PlayerDevices;
        use crate::playlist;
        use tokio::sync::{broadcast, watch};

        let dir = tempfile::tempdir().unwrap();

        // Create a minimal config file
        let config_path = dir.path().join("mtrack.yaml");
        std::fs::write(&config_path, "songs: songs\n").unwrap();

        // Create a minimal playlist file
        let playlist_path = dir.path().join("playlist.yaml");
        std::fs::write(&playlist_path, "songs: []\n").unwrap();

        // Create songs directory
        let songs_path = dir.path().join("songs");
        std::fs::create_dir(&songs_path).unwrap();

        let all_songs_playlist = playlist::from_songs(songs.clone()).unwrap();

        let devices = PlayerDevices {
            audio: None,
            mappings: None,
            midi: None,
            dmx_engine: None,
            sample_engine: None,
            trigger_engine: None,
        };
        let mut playlists = std::collections::HashMap::new();
        playlists.insert("all_songs".to_string(), all_songs_playlist);
        let player = std::sync::Arc::new(
            crate::player::Player::new_with_devices(
                devices,
                playlists,
                "all_songs".to_string(),
                None,
            )
            .unwrap(),
        );

        let (broadcast_tx, _) = broadcast::channel(16);
        let (_state_tx, state_rx) =
            watch::channel(std::sync::Arc::new(crate::state::StateSnapshot::default()));

        let state = WebUiState {
            player,
            state_rx,
            broadcast_tx,
            config_path,
            songs_path,
            playlists_dir: Some(dir.path().to_path_buf()),
            legacy_playlist_path: Some(playlist_path),
            waveform_cache: super::super::state::new_waveform_cache(),
            calibration: std::sync::Arc::new(parking_lot::Mutex::new(None)),
        };

        (state, dir)
    }

    async fn response_body(response: axum::response::Response) -> String {
        let body = response.into_body();
        let bytes = body.collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[tokio::test]
    async fn get_config_raw_returns_yaml() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        assert!(body.contains("songs:"));
    }

    #[tokio::test]
    async fn get_config_raw_missing_file() {
        let (mut state, _dir) = test_state();
        state.config_path = std::path::PathBuf::from("/nonexistent/config.yaml");
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/config")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn get_config_parsed_returns_json() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/config/parsed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed.is_object());
    }

    #[tokio::test]
    async fn get_config_parsed_invalid_config() {
        let (mut state, dir) = test_state();
        // Write invalid YAML to the config file
        let bad_config = dir.path().join("bad.yaml");
        std::fs::write(&bad_config, "invalid: [[[").unwrap();
        state.config_path = bad_config;
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/config/parsed")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn validate_config_valid() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/config/validate")
                    .body(Body::from("songs: songs\n"))
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
    async fn validate_config_invalid() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/config/validate")
                    .body(Body::from("not valid: [[["))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["valid"], false);
    }

    #[tokio::test]
    async fn put_config_valid() {
        let (state, _dir) = test_state();
        let config_path = state.config_path.clone();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config")
                    .body(Body::from("songs: songs\n"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            std::fs::read_to_string(&config_path).unwrap(),
            "songs: songs\n"
        );
    }

    #[tokio::test]
    async fn put_config_invalid() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config")
                    .body(Body::from("invalid: [[["))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

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
        // Verify sorted by path
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

        // Should be rejected (BAD_REQUEST or NOT_FOUND)
        assert_ne!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn get_lighting_file_symlink_escape() {
        let (state, _dir) = test_state();
        // Create a symlink inside songs_path that points outside
        let outside_dir = tempfile::tempdir().unwrap();
        let secret_file = outside_dir.path().join("secret.light");
        std::fs::write(&secret_file, "secret content").unwrap();
        // Create a symlink: songs_path/evil.light -> /outside/secret.light
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

        // Should be rejected because canonical path is outside songs_path
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

        // Restore permissions for cleanup
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
        // Create a song directory with a WAV file and song config
        let song_dir = state.songs_path.join("TestSong");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: TestSong\ntracks:\n  - name: track1\n    file: track1.wav\n",
        )
        .unwrap();

        // Reload the player's registry from disk so the new song is visible.
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
        // Create a song and load it into the registry, then remove the config
        // file so get_song falls back to a JSON summary.
        let song_dir = state.songs_path.join("NoConfig");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: NoConfig\ntracks:\n  - name: track1\n    file: track1.wav\n",
        )
        .unwrap();

        // Load the song into the player's registry while the config exists.
        state.player.reload_songs(
            &state.songs_path,
            state.playlists_dir.as_deref(),
            state.legacy_playlist_path.as_deref(),
        );

        // Now remove the config so get_song can't read it.
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

        // Song is in the registry but config file is gone → returns JSON summary
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
        // Use .yml extension
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
        // Create a song with a non-standard config filename
        let song_dir = state.songs_path.join("CustomSong");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();
        // Use a non-standard config filename (not song.yaml or song.yml)
        std::fs::write(
            song_dir.join("config.yaml"),
            "name: CustomSong\ntracks:\n  - name: track1\n    file: track1.wav\n",
        )
        .unwrap();

        // get_all_songs discovers config.yaml (any non-media-extension file)
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
        // Should return JSON summary with config_file: false
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
        // Only song.yml, no song.yaml
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
        // Should return the YAML content from song.yml
        assert!(body.contains("YmlOnlySong"));
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

        // find_light_files returns Ok(()) for non-dir path, so this returns empty
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn put_lighting_file_outside_base() {
        let (state, _dir) = test_state();
        // Create a symlinked subdirectory pointing outside
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

        // Parent resolves outside songs_path — should be rejected
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

    #[tokio::test]
    async fn get_config_raw_error_body_contains_message() {
        let (mut state, _dir) = test_state();
        state.config_path = std::path::PathBuf::from("/nonexistent/config.yaml");
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/config")
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
            .contains("Failed to read config"));
    }

    #[tokio::test]
    async fn get_config_parsed_error_body_contains_message() {
        let (mut state, dir) = test_state();
        let bad_config = dir.path().join("bad.yaml");
        std::fs::write(&bad_config, "invalid: [[[").unwrap();
        state.config_path = bad_config;
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/config/parsed")
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
            .contains("Failed to parse config"));
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
    async fn get_lighting_file_path_traversal_via_dotdot() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        // Use literal ".." in the path to test the explicit path traversal check
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
    async fn put_config_write_failure_returns_500() {
        let (mut state, _dir) = test_state();
        // Point config_path to a read-only directory
        state.config_path = std::path::PathBuf::from("/nonexistent/dir/config.yaml");
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config")
                    .body(Body::from("songs: songs\n"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["error"].is_string());
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
    async fn put_lighting_file_write_failure_returns_500() {
        use std::os::unix::fs::PermissionsExt;

        let (state, _dir) = test_state();
        // Create a subdir inside songs_path and make it read-only so writes fail
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

        // Restore permissions for cleanup
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
        // Create a subdirectory that cannot be read
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

        // Restore permissions for cleanup
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["error"]
            .as_str()
            .unwrap()
            .contains("Failed to scan for lighting files"));
    }

    #[tokio::test]
    async fn get_config_store_without_store_returns_503() {
        let (state, _dir) = test_state();
        // No config store set on player — should return SERVICE_UNAVAILABLE.
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/config/store")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn get_config_store_with_store_returns_yaml_and_checksum() {
        let (state, _dir) = test_state();

        // Set up a config store on the player.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store-config.yaml");
        std::fs::write(&path, "songs: songs\n").unwrap();
        let cfg = crate::config::Player::deserialize(&path).unwrap();
        let store = std::sync::Arc::new(crate::config::ConfigStore::new(cfg, path));
        state.player.set_config_store(store);

        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/config/store")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(parsed["yaml"].as_str().unwrap().contains("songs"));
        assert!(!parsed["checksum"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn upload_track_single_creates_song() {
        let (state, _dir) = test_state();
        let app = router().with_state(state.clone());

        // Create a valid WAV file in memory
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

        // Verify file and song.yaml were created
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
        // Build multipart body manually
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

        // Verify files were created
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

        // Create an existing song directory with one track
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

        // Verify directory and song.yaml exist
        assert!(state.songs_path.join("BrandNew").is_dir());
        assert!(state.songs_path.join("BrandNew/song.yaml").exists());
    }

    #[tokio::test]
    async fn post_song_conflict_if_exists() {
        let (state, _dir) = test_state();

        // Create song directory with config first
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

        // Create via POST
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

        // Update via PUT
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

        // Verify content was updated
        let content = std::fs::read_to_string(state.songs_path.join("MySong/song.yaml")).unwrap();
        assert!(content.contains("MySong Renamed"));
    }

    #[tokio::test]
    async fn post_song_then_upload_preserves_config() {
        let (state, _dir) = test_state();

        // Create song with custom track names via POST
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

        // Upload a track file
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

        // Verify song.yaml still has custom name (not overwritten by upload)
        let content =
            std::fs::read_to_string(state.songs_path.join("CustomSong/song.yaml")).unwrap();
        assert!(content.contains("My Custom Song"));
        assert!(content.contains("Lead Guitar"));
    }

    /// Creates a minimal valid WAV file for upload tests.
    fn create_test_wav() -> Vec<u8> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wav");
        crate::testutil::write_wav(path.clone(), vec![vec![0_i32; 4410]], 44100).unwrap();
        std::fs::read(&path).unwrap()
    }

    /// Helper: create a test state with a config store for mutation tests.
    fn test_state_with_store() -> (WebUiState, tempfile::TempDir) {
        let (state, dir) = test_state();
        let path = state.config_path.clone();
        let cfg = crate::config::Player::deserialize(&path).unwrap();
        let store = std::sync::Arc::new(crate::config::ConfigStore::new(cfg, path));
        state.player.set_config_store(store);
        (state, dir)
    }

    #[tokio::test]
    async fn put_config_samples_updates_samples() {
        let (state, _dir) = test_state_with_store();
        let app = router().with_state(state.clone());

        // Get the current checksum.
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/config/store")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let store_data: serde_json::Value = serde_json::from_str(&body).unwrap();
        let checksum = store_data["checksum"].as_str().unwrap();

        // Update samples.
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config/samples")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": checksum,
                            "samples": {
                                "kick": { "file": "samples/kick.wav" },
                                "snare": { "file": "samples/snare.wav", "retrigger": "polyphonic" }
                            }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let result: serde_json::Value = serde_json::from_str(&body).unwrap();
        // Should return updated yaml containing our samples.
        let yaml = result["yaml"].as_str().unwrap();
        assert!(yaml.contains("kick"));
        assert!(yaml.contains("snare"));
        assert!(!result["checksum"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn put_config_samples_missing_checksum() {
        let (state, _dir) = test_state_with_store();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config/samples")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "samples": {}
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        assert!(body.contains("missing expected_checksum"));
    }

    #[tokio::test]
    async fn put_config_samples_missing_samples_field() {
        let (state, _dir) = test_state_with_store();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config/samples")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": "abc"
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        assert!(body.contains("missing samples field"));
    }

    #[tokio::test]
    async fn put_config_samples_no_store_returns_503() {
        let (state, _dir) = test_state();
        // No config store set.
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config/samples")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": "abc",
                            "samples": {}
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn upload_sample_file_creates_samples_dir() {
        let (state, _dir) = test_state();
        let wav_bytes = create_test_wav();
        let app = router().with_state(state.clone());

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/samples/upload/kick.wav")
                    .body(Body::from(wav_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["status"], "uploaded");
        assert_eq!(parsed["file"], "kick.wav");
        assert_eq!(parsed["path"], "samples/kick.wav");

        // Verify file was created in samples/ directory next to config.
        let samples_dir = state.config_path.parent().unwrap().join("samples");
        assert!(samples_dir.join("kick.wav").exists());
    }

    #[tokio::test]
    async fn upload_sample_file_path_traversal_rejected() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/samples/upload/..%2F..%2Fetc%2Fpasswd")
                    .body(Body::from("bad"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn upload_sample_file_unsupported_extension() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/samples/upload/readme.txt")
                    .body(Body::from("data"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        assert!(body.contains("Unsupported audio file type"));
    }
}
