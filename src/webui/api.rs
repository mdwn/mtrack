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

pub(crate) mod bluetooth;
pub(crate) mod browse;
pub(crate) mod devices;
pub(crate) mod fixture_settings;
pub(crate) mod fixture_test;
pub(crate) mod helpers;
pub(crate) mod lighting_api;
pub(crate) mod lighting_writes;
pub(crate) mod mvr_api;
pub(crate) mod playlists;
pub(crate) mod profiles;
pub(crate) mod sample_files;
pub(crate) mod songs_api;
pub(crate) mod status;
pub(crate) mod write_lock;

use axum::{extract::State, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde_json::json;

use super::server::WebUiState;
use crate::player::lock_policy::{self, Surface};

/// True when a request may proceed while the player is locked: every GET,
/// and the routes [`lock_policy::OPEN_WHILE_LOCKED`] lists, by method and
/// the route's own pattern (never the request path: a venue named `lock`
/// matches that route's pattern, not the lock toggle's).
fn allowed_while_locked(method: &axum::http::Method, pattern: Option<&str>) -> bool {
    let pattern = pattern.unwrap_or_default();
    let pattern = pattern.strip_prefix("/api").unwrap_or(pattern);
    lock_policy::open_while_locked(Surface::Web, &format!("{method} {pattern}"))
}

/// Middleware that rejects mutating requests (non-GET) when the player is locked,
/// except the exact routes [`lock_policy::OPEN_WHILE_LOCKED`] lists.
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

    if let Err(e) = state.player.require_unlocked() {
        return (StatusCode::LOCKED, Json(json!({"error": e.to_string()}))).into_response();
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

/// Builds the API router for what is left of the REST API.
///
/// Playback control and every call are gRPC-Web (`PlayerService`,
/// `ConfigService`, `LightingService`). File bytes go over `PUT /uploads/{id}`
/// and `GET /downloads/{id}` (see `transfers`), outside `/api`.
pub fn router() -> Router<WebUiState> {
    Router::new().route(
        "/lighting/assets/{*path}",
        get(lighting_api::get_lighting_asset),
    )
}

#[cfg(test)]
pub(crate) mod test_helpers {
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
            // An hour long: the tests that play it look while it plays, and a
            // zero-length song has often ended by then on a loaded machine.
            std::sync::Arc::new(Song::new_for_test_lasting(
                "Song A",
                &["kick", "snare"],
                std::time::Duration::from_secs(3600),
            )),
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
        let all_songs_playlist = crate::playlist::from_songs(songs).unwrap();
        let mut playlists = std::collections::HashMap::new();
        playlists.insert("all_songs".to_string(), all_songs_playlist);
        test_state_with_playlists(playlists, "all_songs", audio, dmx_engine)
    }

    fn test_state_with_playlists(
        playlists: std::collections::HashMap<String, std::sync::Arc<crate::playlist::Playlist>>,
        active: &str,
        audio: Option<std::sync::Arc<dyn crate::audio::Device>>,
        dmx_engine: Option<std::sync::Arc<crate::dmx::engine::Engine>>,
    ) -> (WebUiState, tempfile::TempDir) {
        test_state_on_disk(playlists, active, audio, dmx_engine, "songs: []\n", true)
    }

    /// A WebUiState whose player has `playlists`, `active` playing, in a
    /// temporary directory holding `mtrack.yaml`, an empty `songs/`, and
    /// the legacy `playlist.yaml` holding `legacy_playlist`; the directory
    /// the player's playlists directory too if `playlists_dir`.
    pub(in crate::webui) fn test_state_on_disk(
        playlists: std::collections::HashMap<String, std::sync::Arc<crate::playlist::Playlist>>,
        active: &str,
        audio: Option<std::sync::Arc<dyn crate::audio::Device>>,
        dmx_engine: Option<std::sync::Arc<crate::dmx::engine::Engine>>,
        legacy_playlist: &str,
        playlists_dir: bool,
    ) -> (WebUiState, tempfile::TempDir) {
        use crate::player::PlayerDevices;

        let dir = tempfile::tempdir().unwrap();

        // Create a minimal config file
        let config_path = dir.path().join("mtrack.yaml");
        std::fs::write(&config_path, "songs: songs\n").unwrap();

        // Create a minimal playlist file
        let playlist_path = dir.path().join("playlist.yaml");
        std::fs::write(&playlist_path, legacy_playlist).unwrap();

        // Create songs directory
        let songs_path = dir.path().join("songs");
        std::fs::create_dir(&songs_path).unwrap();

        // Mappings with the device, or the player counts no audio to play
        // and a song finishes the moment it starts, whatever its length.
        let mappings = audio
            .as_ref()
            .map(|_| std::sync::Arc::new(std::collections::HashMap::new()));
        let devices = PlayerDevices {
            audio,
            mappings,
            dmx_engine,
            ..crate::testutil::player::no_devices()
        };
        let player = std::sync::Arc::new(
            crate::player::Player::new_with_devices(devices, playlists, active.to_string(), None)
                .unwrap(),
        );

        player.set_song_sources(crate::player::SongSources {
            songs: songs_path,
            playlists_dir: playlists_dir.then(|| dir.path().to_path_buf()),
            legacy_playlist: Some(playlist_path),
        });
        let state = WebUiState::new(super::super::server::WebUiStateParams {
            player,
            config_path,
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
}

#[cfg(test)]
mod test {
    use super::test_helpers::*;
    use super::*;
    use axum::body::Body;
    use axum::routing::put;
    use tower::ServiceExt;

    /// The guard's own routes: a write, by a pattern a name can match the
    /// suffix of, and the one read the API keeps.
    fn locked_app(state: &WebUiState) -> Router {
        let routes = router()
            .route("/songs/{name}/probe/{file}", put(|| async { "written" }))
            .route("/lock", put(|| async { "written" }));
        Router::new()
            .nest(
                "/api",
                routes.route_layer(axum::middleware::from_fn_with_state(
                    state.clone(),
                    lock_guard,
                )),
            )
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
        let (state, _dir) = test_state();
        state.player.set_locked(true);
        let app = locked_app(&state);

        // A song named `lock` once rode the `/lock` suffix exemption.
        let status = send(&app, "PUT", "/api/songs/lock/probe/kick.wav", "RIFF").await;
        assert_eq!(status, StatusCode::LOCKED);
        // Ordinary edits stay locked.
        assert_eq!(
            send(&app, "PUT", "/api/lock", "{}").await,
            StatusCode::LOCKED
        );
        state.player.set_locked(false);
        assert_eq!(
            send(&app, "PUT", "/api/songs/lock/probe/kick.wav", "RIFF").await,
            StatusCode::OK
        );
    }

    #[tokio::test]
    async fn locked_guard_lets_the_reads_through() {
        let (state, _dir) = test_state();
        state.player.set_locked(true);
        let app = locked_app(&state);

        // The web table lists no write: the MVR inspector, its one entry,
        // is `ConfigService`'s `BeginUpload` and `LightingService`'s
        // `InspectMvr` now, held to the same table by the gRPC lock guard's
        // tests. A GET is a read.
        assert_ne!(
            send(&app, "GET", "/api/lighting/assets/x.json", "").await,
            StatusCode::LOCKED
        );
    }
}
