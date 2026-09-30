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
pub(crate) mod helpers;
pub(crate) mod lighting_api;
pub(crate) mod mvr_api;
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

/// The routes that stay open while the player is locked, by method and the
/// route's own pattern (never the request path: a venue named `lock` matches
/// `/lighting/venues/{name}`, not the lock toggle). These are the lock toggle,
/// validation, playlist activation (a playback control, not an edit), the
/// controller restart, and the pure reads that happen to be POSTs (a preview
/// evaluation and the upload inspectors, which write nothing).
const LOCKED_ALLOWLIST: &[(&str, &str)] = &[
    ("PUT", "/lock"),
    ("POST", "/config/validate"),
    ("POST", "/lighting/validate"),
    ("POST", "/playlists/{name}/activate"),
    ("POST", "/controllers/restart"),
    ("POST", "/lighting/evaluate"),
    ("POST", "/lighting/gdtf/inspect"),
    ("POST", "/lighting/mvr/inspect"),
];

/// True when a request may proceed while the player is locked.
fn allowed_while_locked(method: &axum::http::Method, pattern: Option<&str>) -> bool {
    if method == axum::http::Method::GET {
        return true;
    }
    let Some(pattern) = pattern else {
        return false;
    };
    let pattern = pattern.strip_prefix("/api").unwrap_or(pattern);
    LOCKED_ALLOWLIST
        .iter()
        .any(|(m, p)| *m == method.as_str() && *p == pattern)
}

/// Middleware that rejects mutating requests (non-GET) when the player is locked,
/// except the exact routes in [`LOCKED_ALLOWLIST`].
/// Applied at the server layer where the state is available.
pub(crate) async fn lock_guard(
    State(state): State<WebUiState>,
    request: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let pattern = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|p| p.as_str().to_string());

    if allowed_while_locked(request.method(), pattern.as_deref()) {
        return next.run(request).await;
    }

    if state.player.is_locked() {
        return (
            StatusCode::LOCKED,
            Json(json!({"error": "Player is locked. Unlock to make changes."})),
        )
            .into_response();
    }

    next.run(request).await
}

/// The API router with the lock guard applied to every route it matches.
pub(crate) fn guarded_router(state: &WebUiState) -> Router<WebUiState> {
    router().route_layer(axum::middleware::from_fn_with_state(
        state.clone(),
        lock_guard,
    ))
}

/// Builds the API router for config read/write endpoints.
///
/// Playback control is handled via gRPC-Web (PlayerService), not REST.
pub fn router() -> Router<WebUiState> {
    // Upload routes get unlimited body size for large audio/MIDI files.
    let upload_routes = Router::new()
        .route(
            "/songs/{name}/tracks/{filename}",
            put(songs_api::upload_track_single),
        )
        .route(
            "/songs/{name}/tracks",
            post(songs_api::upload_tracks_multipart),
        )
        .route("/samples/upload/{filename}", put(upload_sample_file))
        .layer(axum::extract::DefaultBodyLimit::disable());

    // GDTF and MVR uploads get a bounded raise instead of a disable: the archive
    // layer refuses anything past 256MB anyway, so the transport should too.
    let gdtf_routes = Router::new()
        .route("/lighting/gdtf/inspect", post(lighting_api::inspect_gdtf))
        .route("/lighting/gdtf/import", post(lighting_api::import_gdtf))
        .route("/lighting/mvr/inspect", post(mvr_api::inspect_mvr))
        .route("/lighting/mvr/import", post(mvr_api::import_mvr))
        .layer(axum::extract::DefaultBodyLimit::max(272 * 1024 * 1024));

    // All other routes use the default body limit.
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
                .put(songs_api::put_song)
                .delete(songs_api::delete_song),
        )
        .route("/songs/{name}/waveform", get(songs_api::get_song_waveform))
        .route(
            "/songs/{name}/tempo-guess",
            get(songs_api::get_song_tempo_guess),
        )
        .route("/songs/{name}/files", get(songs_api::get_song_files))
        .route("/songs/{name}/import", post(songs_api::import_file_to_song))
        .route("/browse", get(browse::browse_directory))
        .route(
            "/browse/create-song",
            post(browse::create_song_in_directory),
        )
        .route("/browse/bulk-import", post(browse::bulk_import))
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
            get(lighting_api::get_lighting_file)
                .put(lighting_api::put_lighting_file)
                .delete(lighting_api::delete_lighting_file),
        )
        .route("/lighting/validate", post(lighting_api::validate_lighting))
        .route("/lighting/readiness", get(lighting_api::get_readiness))
        .route("/lighting/fit", get(lighting_api::get_fit))
        .route("/lighting/evaluate", post(lighting_api::evaluate_lighting))
        .route("/config/store", get(config_api::get_config_store))
        .route("/config/audio", put(config_api::put_config_audio))
        .route("/config/midi", put(config_api::put_config_midi))
        .route("/config/dmx", put(config_api::put_config_dmx))
        .route(
            "/config/controllers",
            put(config_api::put_config_controllers),
        )
        .route("/config/samples", put(config_api::put_config_samples))
        .route("/config/metronome", put(config_api::put_config_metronome))
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
        .route("/controllers/restart", post(status::restart_controllers))
        .route("/lock", get(status::get_lock).put(status::put_lock))
        .route("/devices/audio", get(devices::get_audio_devices))
        .route(
            "/devices/audio/probe",
            post(devices::post_probe_audio_device),
        )
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
        .route("/lighting/groups", get(lighting_api::get_lighting_groups))
        .route("/lighting/mvr/export", get(mvr_api::export_mvr))
        .route("/lighting/mvr/export/summary", get(mvr_api::export_summary))
        .route("/lighting/mvr/export/keep", post(mvr_api::keep_export))
        .route(
            "/lighting/venues/{name}/aim-points",
            post(mvr_api::add_aim_points),
        )
        .route(
            "/lighting/assets/{*path}",
            get(lighting_api::get_lighting_asset),
        )
        .route("/lighting/venues", get(lighting_api::get_venues))
        .route(
            "/lighting/venues/{name}",
            get(lighting_api::get_venue)
                .put(lighting_api::put_venue)
                .delete(lighting_api::delete_venue),
        )
        .merge(upload_routes)
        .merge(gdtf_routes)
}

/// Validates that a filename has a supported audio extension (for sample uploads).
fn validate_sample_filename(filename: &str) -> Result<(), Box<axum::response::Response>> {
    use super::safe_path::SafePath;
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
    // `project_dir_of`, not `parent()`: the latter answers Some("") for a bare
    // filename, so the fallback never fires and the empty path fails to
    // canonicalize -- a 500 for every sample upload whenever mtrack was started
    // as `mtrack start mtrack.yaml`.
    let project_root = crate::util::project_dir_of(&state.config_path);
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

    /// Creates a WebUiState whose player holds an open mock audio device.
    ///
    /// The default test state holds none, which is the wrong shape for anything
    /// that behaves differently when a device is already in use.
    pub fn test_state_with_audio_device(name: &str) -> (WebUiState, tempfile::TempDir) {
        use crate::songs::{Song, Songs};
        use std::collections::HashMap;

        let mut map = HashMap::new();
        map.insert(
            "Song A".to_string(),
            std::sync::Arc::new(Song::new_for_test("Song A", &["kick", "snare"])),
        );
        test_state_inner(
            std::sync::Arc::new(Songs::new(map)),
            Some(std::sync::Arc::new(crate::audio::mock::Device::get(name))),
        )
    }

    /// Creates a WebUiState with the given song registry.
    pub fn test_state_with_registry(
        songs: std::sync::Arc<crate::songs::Songs>,
    ) -> (WebUiState, tempfile::TempDir) {
        test_state_inner(songs, None)
    }

    /// Creates a WebUiState whose player runs the given DMX engine.
    pub fn test_state_with_dmx(
        songs: std::sync::Arc<crate::songs::Songs>,
        dmx_engine: std::sync::Arc<crate::dmx::engine::Engine>,
    ) -> (WebUiState, tempfile::TempDir) {
        test_state_with_hardware(songs, None, Some(dmx_engine))
    }

    fn test_state_inner(
        songs: std::sync::Arc<crate::songs::Songs>,
        audio: Option<std::sync::Arc<dyn crate::audio::Device>>,
    ) -> (WebUiState, tempfile::TempDir) {
        test_state_with_hardware(songs, audio, None)
    }

    fn test_state_with_hardware(
        songs: std::sync::Arc<crate::songs::Songs>,
        audio: Option<std::sync::Arc<dyn crate::audio::Device>>,
        dmx_engine: Option<std::sync::Arc<crate::dmx::engine::Engine>>,
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
            audio,
            mappings: None,
            midi: None,
            dmx_engine,
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

        let state = WebUiState::new(super::super::server::WebUiStateParams {
            player,
            state_rx,
            broadcast_tx,
            config_path,
            songs_path,
            playlists_dir: Some(dir.path().to_path_buf()),
            legacy_playlist_path: Some(playlist_path),
            profiles_dir: None,
            waveform_cache: super::super::state::new_waveform_cache(),
        });

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

    fn locked_app(state: &WebUiState) -> Router {
        Router::new()
            .nest("/api", guarded_router(state))
            .with_state(state.clone())
    }

    async fn send(app: &Router, method: &str, uri: &str, body: &str) -> StatusCode {
        app.clone()
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn locked_guard_matches_routes_not_path_suffixes() {
        let (state, dir) = test_state();
        state.player.set_locked(true);
        let app = locked_app(&state);

        // A venue named `lock` used to ride the `/lock` suffix exemption.
        let status = send(
            &app,
            "PUT",
            "/api/lighting/venues/lock",
            r#"{"fixtures":[]}"#,
        )
        .await;
        assert_eq!(status, StatusCode::LOCKED);
        assert!(!dir.path().join("lighting/venues/lock.light").exists());
        for uri in ["/api/lighting/fixture-types/lock", "/api/songs/activate"] {
            assert_eq!(
                send(&app, "PUT", uri, "{}").await,
                StatusCode::LOCKED,
                "{uri}"
            );
        }
        // Ordinary edits stay locked.
        assert_eq!(
            send(&app, "PUT", "/api/config", "songs: songs\n").await,
            StatusCode::LOCKED
        );
    }

    #[tokio::test]
    async fn locked_guard_lets_the_allowlisted_routes_through() {
        let (state, _dir) = test_state();
        state.player.set_locked(true);
        let app = locked_app(&state);

        // The real toggle, then the validators, which answer rather than 423.
        assert_eq!(
            send(&app, "PUT", "/api/lock", r#"{"locked":true}"#).await,
            StatusCode::OK
        );
        assert_ne!(
            send(&app, "POST", "/api/lighting/validate", "").await,
            StatusCode::LOCKED
        );
        assert_ne!(
            send(&app, "POST", "/api/config/validate", "songs: songs\n").await,
            StatusCode::LOCKED
        );
        assert_ne!(
            send(&app, "POST", "/api/controllers/restart", "").await,
            StatusCode::LOCKED
        );
        assert_ne!(
            send(&app, "POST", "/api/playlists/all/activate", "").await,
            StatusCode::LOCKED
        );
        // The inspectors and the preview write nothing.
        for uri in [
            "/api/lighting/gdtf/inspect",
            "/api/lighting/mvr/inspect",
            "/api/lighting/evaluate",
        ] {
            assert_ne!(
                send(&app, "POST", uri, "").await,
                StatusCode::LOCKED,
                "{uri}"
            );
        }
    }
}
