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

            // Collect MIDI DMX file paths relative to the songs root.
            let midi_dmx_files: Vec<String> = song
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
                "midi_dmx_files": midi_dmx_files,
            })
        })
        .collect();
    let failure_list: Vec<serde_json::Value> = all_songs
        .failures()
        .iter()
        .map(|f| {
            let base_dir = f
                .base_path()
                .strip_prefix(&state.songs_path)
                .unwrap_or(std::path::Path::new(""))
                .to_string_lossy()
                .to_string();
            json!({
                "name": f.name(),
                "error": f.error(),
                "base_dir": base_dir,
                "failed": true,
            })
        })
        .collect();

    (
        StatusCode::OK,
        Json(json!({"songs": song_list, "failures": failure_list})),
    )
        .into_response()
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
        Err(_) => {
            // Check if this is a failed song — serve its raw config for editing.
            if let Some(failure) = all_songs.failures().iter().find(|f| f.name() == name) {
                let config_path = failure.base_path().join("song.yaml");
                let alt_config_path = failure.base_path().join("song.yml");
                let yaml_path = if config_path.exists() {
                    Some(config_path)
                } else if alt_config_path.exists() {
                    Some(alt_config_path)
                } else {
                    None
                };

                return match yaml_path {
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
                    None => (
                        StatusCode::NOT_FOUND,
                        Json(json!({"error": format!("No config file found for failed song: {}", name)})),
                    )
                        .into_response(),
                };
            }

            (
                StatusCode::NOT_FOUND,
                Json(json!({"error": format!("Song not found: {}", name)})),
            )
                .into_response()
        }
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
    use super::super::safe_path::{SafePath, VerifiedRoot};

    // Determine the project root (parent of mtrack.yaml).
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
        Some(p) => match VerifiedRoot::new(p) {
            Ok(r) => r,
            Err(e) => return e.into_response(),
        },
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": "Unable to determine project root"})),
            )
                .into_response();
        }
    };

    // Resolve the source file path under the project root.
    let source = match SafePath::resolve(std::path::Path::new(&body.path), &project_root) {
        Ok(p) => p,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Source file does not exist or is outside the project directory"})),
            )
                .into_response();
        }
    };
    if !source.is_file() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "Source path is not a file"})),
        )
            .into_response();
    }

    let filename = body
        .filename
        .as_deref()
        .or_else(|| source.file_name().and_then(|n| n.to_str()))
        .unwrap_or("unknown");

    if let Err(e) = validate_track_filename(filename) {
        return *e;
    }

    let song_dir = match resolve_or_create_song_dir(&state.player, &state.songs_path, &name) {
        Ok(d) => d,
        Err(e) => return *e,
    };

    // codeql[rust/path-injection] dest_path is under song_dir (verified by resolve_or_create_song_dir),
    // filename is validated by validate_track_filename (no .. or / allowed).
    let dest_path = song_dir.join_filename(filename);
    if let Err(e) = std::fs::copy(&source, &dest_path) {
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

/// DELETE /api/songs/:name — removes a song by deleting its song.yaml.
///
/// The song's audio, MIDI, and lighting files are left in place. Only the
/// song.yaml config file is removed, which unregisters the song from the player.
pub(super) async fn delete_song(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    // Reject deletion if the song is currently playing.
    if state.player.is_playing().await {
        if let Some(current) = state.player.get_playlist().current() {
            if current.name() == name {
                return (
                    StatusCode::CONFLICT,
                    Json(json!({"error": "Cannot delete a song that is currently playing"})),
                )
                    .into_response();
            }
        }
    }

    // Resolve the song directory through the registry with path verification.
    let song_dir = match resolve_song_dir(&state.player, &state.songs_path, &name) {
        Ok(p) => p,
        Err(e) => return *e,
    };

    let config_path = song_dir.join_filename("song.yaml");
    if !config_path.exists() {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": "song.yaml not found for this song"})),
        )
            .into_response();
    }

    if let Err(e) = std::fs::remove_file(&config_path) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to delete song.yaml: {}", e)})),
        )
            .into_response();
    }

    // Remove the song from any playlist files on disk so they stay valid.
    remove_song_from_playlists(
        &name,
        state.playlists_dir.as_deref(),
        state.legacy_playlist_path.as_deref(),
    );

    // Refresh the player's song state.
    state.player.reload_songs(
        &state.songs_path,
        state.playlists_dir.as_deref(),
        state.legacy_playlist_path.as_deref(),
    );

    (
        StatusCode::OK,
        Json(json!({"status": "deleted", "name": name})),
    )
        .into_response()
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
    use super::super::safe_path::{SafePath, VerifiedRoot};

    let root = match VerifiedRoot::new(&state.songs_path) {
        Ok(r) => r,
        Err(e) => return e.into_response(),
    };

    // Validate the YAML before creating any directories so we don't leave
    // orphan directories on disk when validation fails.
    if let Err(e) = validate_song_body(&body) {
        return e;
    }

    let song_dir = match SafePath::create_dir_nested(&name, &root) {
        Ok(p) => p,
        Err(e) => return e.into_response(),
    };

    let config_path = song_dir.join_filename("song.yaml");
    if config_path.exists() {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": format!("Song already exists: {}", name)})),
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
    // Look up the song in the registry to get its actual path (handles nested dirs).
    let song_dir = match resolve_song_dir(&state.player, &state.songs_path, &name) {
        Ok(p) => p,
        Err(e) => return *e,
    };

    let config_path = song_dir.join_filename("song.yaml");

    // Validate the YAML before writing to disk.
    if let Err(e) = validate_song_body(&body) {
        return e;
    }

    match config_io::atomic_write(&config_path, &body) {
        Ok(()) => {
            state.player.reload_songs(
                &state.songs_path,
                state.playlists_dir.as_deref(),
                state.legacy_playlist_path.as_deref(),
            );
            (StatusCode::OK, Json(json!({"status": "saved"}))).into_response()
        }
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
    let song_dir =
        resolve_or_create_song_dir(&state.player, &state.songs_path, &name).map_err(|e| *e)?;

    let file_path = song_dir.join_filename(&filename);
    let replaced = file_path.exists();
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
            "status": if replaced { "replaced" } else { "uploaded" },
            "replaced": replaced,
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
    let song_dir =
        resolve_or_create_song_dir(&state.player, &state.songs_path, &name).map_err(|e| *e)?;

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

        let file_path = song_dir.join_filename(&filename);
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

use super::super::safe_path::{SafePath, VerifiedRoot};

/// Deserializes and validates a song config YAML body. Returns the parsed
/// config on success, or an error response with all validation issues.
#[allow(clippy::result_large_err)]
fn validate_song_body(body: &str) -> Result<config::Song, axum::response::Response> {
    let tmp = tempfile::Builder::new()
        .suffix(".yaml")
        .tempfile()
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to create temp file: {}", e)})),
            )
                .into_response()
        })?;
    std::fs::write(tmp.path(), body).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to write temp file: {}", e)})),
        )
            .into_response()
    })?;
    let song_config = config::Song::deserialize(tmp.path()).map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(json!({"errors": [format!("{}", e)]})),
        )
            .into_response()
    })?;
    if let Err(errors) = song_config.validate() {
        return Err((StatusCode::BAD_REQUEST, Json(json!({"errors": errors}))).into_response());
    }
    Ok(song_config)
}

/// Resolves an existing song directory by checking the player's registry first
/// (handles nested paths like `artist/album/song`), then falling back to a
/// direct path join. Returns an error response if the song isn't found.
pub(super) fn resolve_song_dir(
    player: &crate::player::Player,
    songs_path: &std::path::Path,
    name: &str,
) -> Result<SafePath, Box<axum::response::Response>> {
    let root = VerifiedRoot::new(songs_path).map_err(|e| Box::new(e.into_response()))?;

    // Check the registry first — handles songs in nested subdirectories.
    if let Some(song) = player.get_all_songs_playlist().get_song(name) {
        if let Ok(safe) = SafePath::resolve(song.base_path(), &root) {
            if safe.is_dir() {
                return Ok(safe);
            }
        }
    }

    // Check failures list — the song exists on disk but failed to load.
    let all_songs = player.songs();
    if let Some(failure) = all_songs.failures().iter().find(|f| f.name() == name) {
        if let Ok(safe) = SafePath::resolve(failure.base_path(), &root) {
            if safe.is_dir() {
                return Ok(safe);
            }
        }
    }

    // Fall back to direct join for songs not yet in registry but on disk.
    if let Ok(safe) = root.resolve(name) {
        if safe.is_dir() {
            return Ok(safe);
        }
    }

    Err(Box::new(
        (
            StatusCode::NOT_FOUND,
            Json(json!({"error": format!("Song not found: {}", name)})),
        )
            .into_response(),
    ))
}

/// Resolves a song directory from the registry, falling back to creating it
/// if the song isn't registered (for uploads to new songs).
pub(super) fn resolve_or_create_song_dir(
    player: &crate::player::Player,
    songs_path: &std::path::Path,
    name: &str,
) -> Result<SafePath, Box<axum::response::Response>> {
    if let Ok(dir) = resolve_song_dir(player, songs_path, name) {
        return Ok(dir);
    }
    let root = VerifiedRoot::new(songs_path).map_err(|e| Box::new(e.into_response()))?;
    SafePath::create_dir(&root.as_safe_path(), name, &root).map_err(|e| Box::new(e.into_response()))
}

/// Removes a song name from all playlist YAML files on disk.
/// Silently skips files that can't be read or written.
fn remove_song_from_playlists(
    song_name: &str,
    playlists_dir: Option<&std::path::Path>,
    legacy_playlist_path: Option<&std::path::Path>,
) {
    let mut files_to_check: Vec<std::path::PathBuf> = Vec::new();

    if let Some(dir) = playlists_dir {
        if dir.is_dir() {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file()
                        && path
                            .extension()
                            .is_some_and(|ext| ext == "yaml" || ext == "yml")
                    {
                        files_to_check.push(path);
                    }
                }
            }
        }
    }

    if let Some(legacy) = legacy_playlist_path {
        if legacy.is_file() {
            files_to_check.push(legacy.to_path_buf());
        }
    }

    for path in &files_to_check {
        let Ok(playlist_config) = config::Playlist::deserialize(path) else {
            continue;
        };
        let songs = playlist_config.songs();
        if !songs.iter().any(|s| s == song_name) {
            continue;
        }
        // Rebuild without the deleted song
        let filtered: Vec<String> = songs.iter().filter(|s| *s != song_name).cloned().collect();
        let updated = config::Playlist::new(&filtered);
        if let Ok(yaml) = crate::util::to_yaml_string(&updated) {
            let _ = std::fs::write(path, yaml);
        }
    }
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
    if SafePath::validate_name(filename).is_err() {
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

    // ── import_file_to_song tests ────────────────────────────────────

    #[tokio::test]
    async fn import_file_to_song_success() {
        let (state, dir) = test_state();

        // Create a WAV file in the project root (outside songs/).
        let wav_bytes = create_test_wav();
        let source_path = dir.path().join("import_me.wav");
        std::fs::write(&source_path, &wav_bytes).unwrap();

        let app = router().with_state(state.clone());
        let body =
            serde_json::json!({ "path": source_path.canonicalize().unwrap().to_str().unwrap() });

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/songs/TestSong/import")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let resp_body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
        assert_eq!(parsed["status"], "imported");
        assert_eq!(parsed["file"], "import_me.wav");
        assert_eq!(parsed["song"], "TestSong");

        // Verify the file was copied into the song directory.
        assert!(state.songs_path.join("TestSong/import_me.wav").exists());
    }

    #[tokio::test]
    async fn import_file_to_song_outside_project_rejected() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let body = serde_json::json!({ "path": "/etc/hosts" });

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/songs/TestSong/import")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let resp_body = response_body(response).await;
        assert!(resp_body.contains("outside the project directory"));
    }

    #[tokio::test]
    async fn import_file_to_song_nonexistent_source() {
        let (state, dir) = test_state();
        let app = router().with_state(state);

        let nonexistent = dir.path().join("does_not_exist.wav");
        let body = serde_json::json!({ "path": nonexistent.to_str().unwrap() });

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/songs/TestSong/import")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let resp_body = response_body(response).await;
        assert!(resp_body.contains("does not exist"));
    }

    #[tokio::test]
    async fn import_file_to_song_renames_with_dmx_prefix() {
        let (state, dir) = test_state();

        // Create a .mid file in the project root.
        let source_path = dir.path().join("original.mid");
        let midi_bytes = std::fs::read(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/song.mid"),
        )
        .unwrap();
        std::fs::write(&source_path, &midi_bytes).unwrap();

        let app = router().with_state(state.clone());
        let body = serde_json::json!({
            "path": source_path.canonicalize().unwrap().to_str().unwrap(),
            "filename": "dmx_lightshow.mid"
        });

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/songs/TestSong/import")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let resp_body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&resp_body).unwrap();
        assert_eq!(parsed["file"], "dmx_lightshow.mid");

        // Verify the file was saved with the overridden name.
        assert!(state.songs_path.join("TestSong/dmx_lightshow.mid").exists());
    }

    #[tokio::test]
    async fn import_file_to_song_rejects_unsupported_extension() {
        let (state, dir) = test_state();

        // Create a source file (extension doesn't matter for the source, but
        // the override filename is what gets validated).
        let source_path = dir.path().join("notes.wav");
        std::fs::write(&source_path, &create_test_wav()).unwrap();

        let app = router().with_state(state);
        let body = serde_json::json!({
            "path": source_path.canonicalize().unwrap().to_str().unwrap(),
            "filename": "readme.txt"
        });

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/songs/TestSong/import")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let resp_body = response_body(response).await;
        assert!(resp_body.contains("Unsupported file type"));
    }

    #[tokio::test]
    async fn import_file_to_song_directory_rejected() {
        let (state, dir) = test_state();

        // Create a subdirectory in the project root.
        let subdir = dir.path().join("a_directory");
        std::fs::create_dir(&subdir).unwrap();

        let app = router().with_state(state);
        let body = serde_json::json!({ "path": subdir.canonicalize().unwrap().to_str().unwrap() });

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/songs/TestSong/import")
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_string(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let resp_body = response_body(response).await;
        assert!(resp_body.contains("not a file"));
    }

    // ── get_songs response field tests ───────────────────────────────

    #[tokio::test]
    async fn get_songs_includes_base_dir() {
        let (state, _dir) = test_state();

        // Create a song on disk so that it appears in the response.
        let song_dir = state.songs_path.join("BaseDirSong");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: BaseDirSong\ntracks:\n  - name: track1\n    file: track1.wav\n",
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
        let base_dir_song = songs
            .iter()
            .find(|s| s["name"] == "BaseDirSong")
            .expect("BaseDirSong should be in the response");
        assert!(
            base_dir_song.get("base_dir").is_some(),
            "Response should include base_dir field"
        );
        assert_eq!(base_dir_song["base_dir"], "BaseDirSong");
    }

    #[tokio::test]
    async fn get_songs_includes_lighting_files() {
        let (state, _dir) = test_state();

        // Create a song with a DSL lighting show.
        let song_dir = state.songs_path.join("LitSong");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();
        std::fs::write(song_dir.join("show.light"), "show \"Test\" {\n}\n").unwrap();
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: LitSong\ntracks:\n  - name: track1\n    file: track1.wav\nlighting:\n  - file: show.light\n",
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
        let lit_song = songs
            .iter()
            .find(|s| s["name"] == "LitSong")
            .expect("LitSong should be in the response");
        let lighting_files = lit_song["lighting_files"].as_array().unwrap();
        assert!(
            !lighting_files.is_empty(),
            "lighting_files should be populated"
        );
        assert!(
            lighting_files[0].as_str().unwrap().contains("show.light"),
            "lighting_files should contain the .light file path"
        );
    }

    #[tokio::test]
    async fn get_songs_includes_midi_dmx_files() {
        let (state, _dir) = test_state();

        // Create a song with a legacy dmx_*.mid light show.
        let song_dir = state.songs_path.join("LegacySong");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();

        // Copy a real MIDI file from assets to serve as the dmx file.
        let midi_source =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/song.mid");
        std::fs::copy(&midi_source, song_dir.join("dmx_show.mid")).unwrap();

        std::fs::write(
            song_dir.join("song.yaml"),
            "name: LegacySong\ntracks:\n  - name: track1\n    file: track1.wav\nlight_shows:\n  - universe_name: default\n    dmx_file: dmx_show.mid\n",
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
        let legacy_song = songs
            .iter()
            .find(|s| s["name"] == "LegacySong")
            .expect("LegacySong should be in the response");
        let legacy_files = legacy_song["midi_dmx_files"].as_array().unwrap();
        assert!(
            !legacy_files.is_empty(),
            "midi_dmx_files should be populated"
        );
        assert!(
            legacy_files[0].as_str().unwrap().contains("dmx_show.mid"),
            "midi_dmx_files should contain the dmx MIDI file path"
        );
    }

    #[tokio::test]
    async fn get_song_waveform_success() {
        let (state, _dir) = test_state();
        let song_dir = state.songs_path.join("WaveSong");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: WaveSong\ntracks:\n  - name: track1\n    file: track1.wav\n",
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
                    .uri("/songs/WaveSong/waveform")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["song_name"], "WaveSong");
        assert!(parsed["tracks"].is_array());
    }

    #[tokio::test]
    async fn get_song_waveform_not_found() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/songs/nonexistent/waveform")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn get_song_waveform_cached() {
        let (state, _dir) = test_state();
        let song_dir = state.songs_path.join("CachedSong");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: CachedSong\ntracks:\n  - name: track1\n    file: track1.wav\n",
        )
        .unwrap();

        state.player.reload_songs(
            &state.songs_path,
            state.playlists_dir.as_deref(),
            state.legacy_playlist_path.as_deref(),
        );

        // First call — computes and caches.
        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/songs/CachedSong/waveform")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        // Second call — served from cache.
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/songs/CachedSong/waveform")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["song_name"], "CachedSong");
        assert!(parsed["tracks"].is_array());
    }

    #[tokio::test]
    async fn get_song_files_success() {
        let (state, _dir) = test_state();
        let song_dir = state.songs_path.join("FilesSong");
        std::fs::create_dir(&song_dir).unwrap();
        crate::testutil::write_wav(song_dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
            .unwrap();
        // Copy a real MIDI file from assets.
        let midi_source =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/song.mid");
        std::fs::copy(&midi_source, song_dir.join("notes.mid")).unwrap();
        std::fs::write(song_dir.join("show.light"), "show \"Test\" {}\n").unwrap();
        std::fs::write(song_dir.join("readme.txt"), "hello").unwrap();
        std::fs::write(
            song_dir.join("song.yaml"),
            "name: FilesSong\ntracks:\n  - name: track1\n    file: track1.wav\n",
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
                    .uri("/songs/FilesSong/files")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let files = parsed["files"].as_array().unwrap();

        // Should have track1.wav, notes.mid, show.light, readme.txt (song.yaml is skipped)
        assert_eq!(files.len(), 4);

        // Verify type classification
        let find_file = |name: &str| files.iter().find(|f| f["name"] == name).unwrap();
        assert_eq!(find_file("track1.wav")["type"], "audio");
        assert_eq!(find_file("notes.mid")["type"], "midi");
        assert_eq!(find_file("show.light")["type"], "lighting");
        assert_eq!(find_file("readme.txt")["type"], "other");
    }

    #[tokio::test]
    async fn get_song_files_not_found() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .uri("/songs/nonexistent/files")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn upload_track_single_midi_file() {
        let (state, _dir) = test_state();
        let app = router().with_state(state.clone());

        let midi_bytes = std::fs::read(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/song.mid"),
        )
        .unwrap();

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/songs/MidiSong/tracks/notes.mid")
                    .body(Body::from(midi_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["status"], "uploaded");
        assert_eq!(parsed["file"], "notes.mid");
        assert!(state.songs_path.join("MidiSong/notes.mid").exists());
    }

    #[tokio::test]
    async fn upload_track_single_light_file() {
        let (state, _dir) = test_state();
        let app = router().with_state(state.clone());

        let light_content = b"show \"Test\" {}\n";

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/songs/LightSong/tracks/show.light")
                    .body(Body::from(light_content.as_slice()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(parsed["status"], "uploaded");
        assert_eq!(parsed["file"], "show.light");
        assert!(state.songs_path.join("LightSong/show.light").exists());
    }
}
