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

pub(crate) mod browse;
pub(crate) mod config_api;
pub(crate) mod devices;
pub(crate) mod lighting_api;
pub(crate) mod playlists;
pub(crate) mod profiles;
pub(crate) mod songs_api;
pub(crate) mod status;

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post, put},
    Json, Router,
};
use serde_json::json;

use super::server::WebUiState;
use crate::songs as songs_crate;

/// Builds the API router for config read/write endpoints.
///
/// Playback control is handled via gRPC-Web (PlayerService), not REST.
pub fn router() -> Router<WebUiState> {
    Router::new()
        .route(
            "/config",
            get(config_api::get_config_raw).put(config_api::put_config),
        )
        .route("/config/parsed", get(config_api::get_config_parsed))
        .route("/config/validate", post(config_api::validate_config))
        .route("/songs", get(songs_api::get_songs))
        .route(
            "/songs/{name}",
            get(songs_api::get_song)
                .post(songs_api::post_song)
                .put(songs_api::put_song),
        )
        .route(
            "/songs/{name}/tracks/{filename}",
            put(songs_api::upload_track_single),
        )
        .route(
            "/songs/{name}/tracks",
            post(songs_api::upload_tracks_multipart),
        )
        .route("/songs/{name}/waveform", get(songs_api::get_song_waveform))
        .route("/songs/{name}/files", get(songs_api::get_song_files))
        .route("/songs/{name}/import", post(songs_api::import_file_to_song))
        .route("/browse", get(browse::browse_directory))
        .route(
            "/browse/create-song",
            post(browse::create_song_in_directory),
        )
        .route(
            "/playlist",
            get(playlists::get_playlist).put(playlists::put_playlist),
        )
        .route("/playlist/validate", post(playlists::validate_playlist))
        .route("/playlists", get(playlists::get_playlists))
        .route(
            "/playlists/{name}",
            get(playlists::get_playlist_by_name)
                .put(playlists::put_playlist_by_name)
                .delete(playlists::delete_playlist_by_name),
        )
        .route(
            "/playlists/{name}/activate",
            post(playlists::activate_playlist),
        )
        .route("/lighting", get(lighting_api::get_lighting_files))
        .route(
            "/lighting/{name}",
            get(lighting_api::get_lighting_file).put(lighting_api::put_lighting_file),
        )
        .route("/lighting/validate", post(lighting_api::validate_lighting))
        .route("/config/store", get(config_api::get_config_store))
        .route("/config/audio", put(config_api::put_config_audio))
        .route("/config/midi", put(config_api::put_config_midi))
        .route("/config/dmx", put(config_api::put_config_dmx))
        .route(
            "/config/controllers",
            put(config_api::put_config_controllers),
        )
        .route("/config/samples", put(config_api::put_config_samples))
        .route("/samples/upload/{filename}", put(upload_sample_file))
        .route("/config/profiles", post(config_api::post_config_profile))
        .route(
            "/config/profiles/{index}",
            put(config_api::put_config_profile).delete(config_api::delete_config_profile),
        )
        .route("/profiles", get(profiles::get_profiles))
        .route(
            "/profiles/{filename}",
            get(profiles::get_profile)
                .put(profiles::put_profile)
                .delete(profiles::delete_profile_file),
        )
        .route("/status", get(status::get_status))
        .route("/devices/audio", get(devices::get_audio_devices))
        .route("/devices/midi", get(devices::get_midi_devices))
        .route("/calibrate/start", post(devices::post_calibrate_start))
        .route("/calibrate/capture", post(devices::post_calibrate_capture))
        .route("/calibrate/stop", post(devices::post_calibrate_stop))
        .route("/calibrate", delete(devices::delete_calibrate))
        .route(
            "/lighting/fixture-types",
            get(lighting_api::get_fixture_types),
        )
        .route(
            "/lighting/fixture-types/{name}",
            get(lighting_api::get_fixture_type)
                .put(lighting_api::put_fixture_type)
                .delete(lighting_api::delete_fixture_type),
        )
        .route("/lighting/venues", get(lighting_api::get_venues))
        .route(
            "/lighting/venues/{name}",
            get(lighting_api::get_venue)
                .put(lighting_api::put_venue)
                .delete(lighting_api::delete_venue),
        )
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
    if !songs_crate::is_supported_audio_extension(ext) {
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

#[cfg(test)]
pub(super) mod test_helpers {
    use super::*;
    use http_body_util::BodyExt;

    /// Creates a WebUiState with a test player and temp directories.
    /// The player's song registry contains "Song A" (in-memory only, not on disk).
    pub fn test_state() -> (WebUiState, tempfile::TempDir) {
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
    pub fn test_state_with_registry(
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
            profiles_dir: None,
            waveform_cache: super::super::state::new_waveform_cache(),
            calibration: std::sync::Arc::new(parking_lot::Mutex::new(None)),
        };

        (state, dir)
    }

    pub async fn response_body(response: axum::response::Response) -> String {
        let body = response.into_body();
        let bytes = body.collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    /// Creates a minimal valid WAV file for upload tests.
    pub fn create_test_wav() -> Vec<u8> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wav");
        crate::testutil::write_wav(path.clone(), vec![vec![0_i32; 4410]], 44100).unwrap();
        std::fs::read(&path).unwrap()
    }

    /// Helper: create a test state with a config store for mutation tests.
    pub fn test_state_with_store() -> (WebUiState, tempfile::TempDir) {
        let (state, dir) = test_state();
        let path = state.config_path.clone();
        let cfg = crate::config::Player::deserialize(&path).unwrap();
        let store = std::sync::Arc::new(crate::config::ConfigStore::new(cfg, path));
        state.player.set_config_store(store);
        (state, dir)
    }
}

#[cfg(test)]
mod test {
    use super::test_helpers::*;
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

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
