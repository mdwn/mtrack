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

use crate::webui::wire::config::{ConfigReload, ConfigSaved, StoreSnapshot};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde_json::json;

use tracing::warn;

use super::super::server::WebUiState;
use crate::config;

/// Helper to convert a ConfigError into an HTTP response.
fn config_store_error_response(e: config::ConfigError) -> axum::response::Response {
    super::helpers::fault_response(e.kind(), e.to_string())
}

/// Helper to convert a ConfigSnapshot into a JSON response, with what became
/// of the hardware reload.
fn config_snapshot_response(
    snapshot: config::store::ConfigSnapshot,
    status: StatusCode,
    reload: ConfigReload,
) -> axum::response::Response {
    let snapshot = StoreSnapshot {
        yaml: snapshot.yaml,
        checksum: snapshot.checksum,
    };
    let saved = ConfigSaved { snapshot, reload };
    (status, Json(saved)).into_response()
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

/// Applies a saved config change to the running hardware, as every
/// surface does after a save (see [`crate::player::ConfigApply`]), and
/// returns what the reply says of it: `applied`, and a `message` when it is
/// not. A save is made whatever the player is doing: while a song plays, is
/// paused or fading out, or an auto play waits out its gap, the hardware
/// reload waits until the player is idle, and the save answers with
/// `applied: false` and a message saying so. It used to be refused with a
/// 409, though the gRPC and MCP config changes saved and deferred.
/// Non-blocking: device discovery runs in the background.
pub(super) async fn apply_config_change(state: &WebUiState) -> ConfigReload {
    config_apply_fields(&state.player.apply_config_change().await)
}

/// What became of a saved config change, as its reply says.
fn config_apply_fields(outcome: &crate::player::ConfigChange) -> ConfigReload {
    // The reload brings the controllers in line with the config itself,
    // restarting only those it changed, or whose MIDI device it reopened.
    // Forcing them all here as well tore down every OSC, gRPC and MCP
    // client for a change that was not theirs.
    ConfigReload {
        applied: outcome.applied(),
        // Which it was, for the web UI to say so: a save deferred until the
        // player is idle reads otherwise as one made.
        reload: outcome.kind().to_string(),
        message: outcome.message(),
        // A song reload that failed beside it, carried rather than hiding
        // the hardware's outcome.
        songs_error: outcome.songs_error.clone(),
        // The deferred reload generation a deferral waits for, for the web
        // UI to clear its notice once the playback updates' started
        // reaches it.
        awaits_reload: outcome.awaits,
        awaits_process: outcome
            .awaits
            .map(|_| crate::player::process_id().to_string()),
    }
}

/// GET /api/config/store — returns config YAML + checksum via the ConfigStore.
pub(super) async fn get_config_store(State(state): State<WebUiState>) -> impl IntoResponse {
    let store = match require_config_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match store.read_yaml().await {
        Ok((yaml, checksum)) => {
            (StatusCode::OK, Json(StoreSnapshot { yaml, checksum })).into_response()
        }
        Err(e) => config_store_error_response(e),
    }
}

/// Extracts the expected_checksum from a JSON body, or returns a 400 error.
#[allow(clippy::result_large_err)]
fn extract_checksum(body: &serde_json::Value) -> Result<String, axum::response::Response> {
    body.get("expected_checksum")
        .and_then(|v| v.as_str())
        .map(|c| c.to_string())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "missing expected_checksum"})),
            )
                .into_response()
        })
}

/// PUT /api/config/samples — replace all sample definitions.
pub(super) async fn put_config_samples(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let checksum = match extract_checksum(&body) {
        Ok(c) => c,
        Err(e) => return e,
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

    let max_sample_voices = body
        .get("max_sample_voices")
        .and_then(|v| v.as_u64())
        .map(|v| v as u32);

    let store = match require_config_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match store
        .update_samples(samples, max_sample_voices, &checksum)
        .await
    {
        Ok(snapshot) => {
            let reload = apply_config_change(&state).await;
            config_snapshot_response(snapshot, StatusCode::OK, reload)
        }
        Err(e) => config_store_error_response(e),
    }
}

/// PUT /api/config/metronome — replace the player-wide metronome defaults.
/// `metronome` may be null to remove them entirely.
pub(super) async fn put_config_metronome(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let checksum = match extract_checksum(&body) {
        Ok(c) => c,
        Err(e) => return e,
    };

    let metronome: Option<config::MetronomeDefaults> = match body.get("metronome") {
        Some(serde_json::Value::Null) | None => None,
        Some(v) => match serde_json::from_value(v.clone()) {
            Ok(m) => Some(m),
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("invalid metronome defaults: {}", e)})),
                )
                    .into_response()
            }
        },
    };

    if let Some(defaults) = metronome.as_ref() {
        if let Err(e) = defaults.validate() {
            return (StatusCode::BAD_REQUEST, Json(json!({ "error": e }))).into_response();
        }
    }

    let store = match require_config_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };
    let default_enabled = metronome.as_ref().is_some_and(|m| m.enabled);
    match store.update_metronome(metronome, &checksum).await {
        Ok(snapshot) => {
            let mut change = state.player.apply_config_change().await;
            // The default-on flag is applied at song load; refresh both. A
            // reload that fails is answered with the save, its new checksum
            // included, and the reason beside the hardware's outcome, as any
            // save's failed reload is: a 500 left the UI the old checksum of
            // a config already saved.
            state.player.set_default_metronome(default_enabled);
            if let Err(e) = state.player.reload_songs().await {
                warn!("Reloading songs after a metronome change failed: {}", e);
                change.songs_error = Some(e.to_string());
            }
            config_snapshot_response(snapshot, StatusCode::OK, config_apply_fields(&change))
        }
        Err(e) => config_store_error_response(e),
    }
}

/// POST /api/config/profiles — add a new profile.
pub(super) async fn post_config_profile(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let checksum = match extract_checksum(&body) {
        Ok(c) => c,
        Err(e) => return e,
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

    if let Err(errors) = profile.validate() {
        return (StatusCode::BAD_REQUEST, Json(json!({"errors": errors}))).into_response();
    }

    let store = match require_config_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };
    match store.add_profile(profile, &checksum).await {
        Ok(snapshot) => {
            let reload = apply_config_change(&state).await;
            config_snapshot_response(snapshot, StatusCode::CREATED, reload)
        }
        Err(e) => config_store_error_response(e),
    }
}

/// PUT /api/config/profiles/:index — update profile at index.
pub(super) async fn put_config_profile(
    State(state): State<WebUiState>,
    Path(index): Path<usize>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let checksum = match extract_checksum(&body) {
        Ok(c) => c,
        Err(e) => return e,
    };

    let store = match require_config_store(&state) {
        Ok(s) => s,
        Err(e) => return e,
    };

    // Sections the body leaves out keep their current values (#523).
    let profile: config::Profile = match body.get("profile") {
        Some(v) => match store.profile_from_update(index, v.clone()).await {
            Ok(p) => p,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": e.to_string()})),
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

    if let Err(errors) = profile.validate() {
        return (StatusCode::BAD_REQUEST, Json(json!({"errors": errors}))).into_response();
    }

    match store.update_profile(index, profile, &checksum).await {
        Ok(snapshot) => {
            let reload = apply_config_change(&state).await;
            config_snapshot_response(snapshot, StatusCode::OK, reload)
        }
        Err(e) => config_store_error_response(e),
    }
}

/// DELETE /api/config/profiles/:index?expected_checksum=... — remove profile at index.
pub(super) async fn delete_config_profile(
    State(state): State<WebUiState>,
    Path(index): Path<usize>,
    axum::extract::Query(params): axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
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
            let reload = apply_config_change(&state).await;
            config_snapshot_response(snapshot, StatusCode::OK, reload)
        }
        Err(e) => config_store_error_response(e),
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
    async fn get_config_store_without_store_returns_503() {
        let (state, _dir) = test_state();
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
    async fn put_config_samples_updates_samples() {
        let (state, _dir) = test_state_with_store();
        let app = router().with_state(state.clone());

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

    /// Helper: GET /config/store to retrieve the current checksum.
    async fn get_checksum(state: &crate::webui::server::WebUiState) -> String {
        let app = router().with_state(state.clone());
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
        parsed["checksum"].as_str().unwrap().to_string()
    }

    /// Sets a config store on the state's player, from its config file.
    fn set_store(state: &crate::webui::server::WebUiState) {
        let path = state.config_path.clone();
        let cfg = crate::config::Player::deserialize(&path).unwrap();
        let store = std::sync::Arc::new(crate::config::ConfigStore::new(cfg, path));
        state.player.set_config_store(store);
    }

    /// Saves an empty set of samples, the one settings save that needs
    /// nothing but the store's checksum, and answers the reply's status and
    /// body.
    async fn save_samples(
        state: &crate::webui::server::WebUiState,
    ) -> (StatusCode, serde_json::Value) {
        let checksum = get_checksum(state).await;
        let response = router()
            .with_state(state.clone())
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config/samples")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"expected_checksum": checksum, "samples": {}})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        (
            status,
            serde_json::from_str(&response_body(response).await).unwrap(),
        )
    }

    /// A settings save in an auto play's gap reset the hardware, and the song failed
    /// to start when the gap ended (#578): the gap counts as playing. The
    /// save is made, and its reload waits for the song, as a save while it
    /// plays does; the reply says so.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_while_an_auto_play_waits_is_saved_and_waits() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("playlist.yaml");
        std::fs::write(
            &path,
            "songs:\n- Song 1\n- song: Song 3\n  auto_play: 10s\n",
        )
        .unwrap();
        let songs = crate::songs::get_all_songs(std::path::Path::new("assets/songs")).unwrap();
        let playlist = crate::playlist::Playlist::new(
            "playlist",
            &crate::config::Playlist::deserialize(&path).unwrap(),
            songs,
        )
        .unwrap();
        let (state, _dir) = test_state_with_playlist(playlist);
        set_store(&state);
        state.player.next().await;
        assert!(state.player.auto_play_status().is_some(), "waiting");

        let (status, body) = save_samples(&state).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["applied"], serde_json::json!(false), "{body}");
        assert!(body["message"].as_str().is_some_and(|m| m.contains("idle")));
        assert!(
            state.player.hardware_status().init_done,
            "the hardware was reset in the gap"
        );
        assert!(state.player.cancel_waiting_auto_play(), "still waiting");
    }

    /// A paused song is a show in progress, which a reload waits for, as for
    /// a playing one. A save then used to be refused with a 409, though the
    /// gRPC and MCP config changes saved and waited: it is saved, the reply
    /// says the reload waits, and it runs once the song is stopped.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_while_a_song_is_paused_is_saved_and_applied_once_stopped() {
        let (state, _dir) = test_state_with_audio_device("mock-device");
        set_store(&state);
        state.player.play().await.unwrap();
        assert!(state.player.pause().await.unwrap().is_some(), "paused");

        let (status, body) = save_samples(&state).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["applied"], serde_json::json!(false), "{body}");
        // The deferred reload generation the save waits for, for the web
        // UI to clear its notice by.
        assert_eq!(body["awaits_reload"], serde_json::json!(1), "{body}");
        assert_eq!(
            body["awaits_process"],
            serde_json::json!(crate::player::process_id().to_string())
        );
        assert!(state.player.is_paused(), "the pause was kept");
        assert!(
            state.player.audio_device().is_some(),
            "the hardware was reset under the pause"
        );
        state.player.stop().await;
        crate::testutil::eventually(
            || state.player.audio_device().is_none(),
            "the saved config was never applied",
        );
    }

    /// A play landing between a save's playback check and its hardware
    /// reload had the reload refused, and the save answered 200 while the
    /// rig ran the old config (#578). The reload runs once the song stops.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_raced_by_a_play_reloads_the_hardware_once_it_stops() {
        // The config holds no audio, so the reload shows as the device going.
        let (state, _dir) = test_state_with_audio_device("mock-device");
        set_store(&state);

        assert!(!state.player.is_playing().await);
        state.player.play().await.unwrap();
        let reload = serde_json::json!(super::apply_config_change(&state).await);
        // The save's reply says it has not been applied yet.
        assert_eq!(reload["applied"], serde_json::json!(false), "{reload:?}");
        assert!(reload["message"]
            .as_str()
            .is_some_and(|m| m.contains("idle")));
        assert!(
            state.player.audio_device().is_some(),
            "the hardware was reset under the song"
        );

        state.player.stop().await;
        crate::testutil::eventually(
            || state.player.audio_device().is_none(),
            "the saved config was never applied",
        );
    }

    /// A save restarted every controller after its reload had left the
    /// unchanged ones running: each OSC, gRPC and MCP client lost its
    /// connection over a change that was not its own.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_leaves_the_unchanged_controllers_running() {
        let (state, dir) = test_state_with_audio_device("mock-device");
        let grpc = crate::config::Controller::Grpc(crate::config::GrpcController::new(0));
        state
            .player
            .set_config_store(std::sync::Arc::new(crate::config::ConfigStore::new(
                crate::config::Player::new(
                    vec![grpc.clone()],
                    None,
                    None,
                    None,
                    std::collections::HashMap::new(),
                    "songs",
                ),
                dir.path().join("config.yaml"),
            )));
        state.player.start_controllers(vec![grpc]);
        let tasks = state.player.controller_tasks();
        assert_eq!(tasks.len(), 1);

        let reload = serde_json::json!(super::apply_config_change(&state).await);
        assert_eq!(reload["applied"], serde_json::json!(true), "{reload:?}");
        state.player.await_hardware_ready().await;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert!(
            !tasks[0].is_finished(),
            "an unchanged controller was restarted by a save"
        );
        assert_eq!(state.player.controller_statuses()[0].status, "running");
        state.player.shutdown_controllers();
    }

    /// A position cued by a seek while stopped read as a pause, and a save
    /// was refused as "playing or paused", though nothing plays and no
    /// lights are held.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_after_a_seek_while_stopped_is_not_refused() {
        let (state, _dir) = test_state_with_audio_device("mock-device");
        set_store(&state);
        state
            .player
            .seek_to(std::time::Duration::from_millis(500))
            .await
            .unwrap();
        assert!(!state.player.is_playing().await);
        let reload = serde_json::json!(super::apply_config_change(&state).await);
        assert_eq!(reload["applied"], serde_json::json!(true), "{reload:?}");
    }

    /// A metronome save whose song reload fails is still answered with the
    /// save and its new checksum, and the reason: a 500 left the UI the old
    /// checksum, and its next save was refused as stale.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_metronome_save_whose_song_reload_fails_answers_with_the_checksum() {
        let (state, _dir) = test_state();
        set_store(&state);
        let store = state.player.config_store().unwrap();
        let (_, checksum) = store.read_yaml().await.unwrap();
        // The songs directory gone from under the player.
        std::fs::remove_dir_all(state.songs_path()).unwrap();
        let response = router()
            .with_state(state.clone())
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config/metronome")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({
                            "expected_checksum": checksum,
                            "metronome": {"enabled": true},
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
        let (_, now) = store.read_yaml().await.unwrap();
        assert_eq!(body["checksum"], serde_json::json!(now), "{body}");
        assert_ne!(now, checksum);
        assert_eq!(body["applied"], serde_json::json!(false));
        // The hardware's outcome is carried beside the songs', not hidden.
        assert_eq!(body["reload"], "applied");
        assert!(body["songs_error"].is_string(), "{body}");
    }

    /// The reply to a save says what became of it on the hardware, as the
    /// gRPC and MCP replies do: applied, deferred with a message, or saved
    /// and not applied because the reload failed, with the reason.
    #[test]
    fn a_saves_reply_says_what_became_of_it() {
        use crate::player::ConfigApply;
        let applied = serde_json::json!(super::config_apply_fields(&ConfigApply::Applied.into()));
        assert_eq!(applied["applied"], serde_json::json!(true));
        assert!(applied.get("message").is_none());
        let deferred = serde_json::json!(super::config_apply_fields(&ConfigApply::Deferred.into()));
        assert_eq!(deferred["applied"], serde_json::json!(false));
        assert_eq!(deferred["reload"], "deferred");
        assert!(deferred["message"].as_str().unwrap().contains("idle"));
        let failed = serde_json::json!(super::config_apply_fields(
            &ConfigApply::Failed("no store".into()).into()
        ));
        assert_eq!(failed["applied"], serde_json::json!(false));
        assert_eq!(failed["reload"], "failed");
        let message = failed["message"].as_str().unwrap();
        assert!(
            message.contains("saved") && message.contains("no store"),
            "{message}"
        );
    }

    #[tokio::test]
    async fn post_config_profile_success() {
        let (state, _dir) = test_state_with_store();
        let checksum = get_checksum(&state).await;

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/config/profiles")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": checksum,
                            "profile": { "hostname": "test-host" }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response_body(response).await;
        let result: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(result["yaml"].as_str().unwrap().contains("test-host"));
        assert!(!result["checksum"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn post_config_profile_stale_checksum() {
        let (state, _dir) = test_state_with_store();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/config/profiles")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": "wrong-checksum",
                            "profile": { "hostname": "test-host" }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn put_config_profile_success() {
        let (state, _dir) = test_state_with_store();

        // First, add a profile so index 0 exists.
        let checksum = get_checksum(&state).await;
        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/config/profiles")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": checksum,
                            "profile": { "hostname": "original" }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response_body(response).await;
        let result: serde_json::Value = serde_json::from_str(&body).unwrap();
        let new_checksum = result["checksum"].as_str().unwrap();

        // Now update that profile at index 0.
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config/profiles/0")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": new_checksum,
                            "profile": { "hostname": "updated" }
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
        assert!(result["yaml"].as_str().unwrap().contains("updated"));
    }

    #[tokio::test]
    async fn put_config_profile_invalid_index() {
        let (state, _dir) = test_state_with_store();
        let checksum = get_checksum(&state).await;

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config/profiles/999")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": checksum,
                            "profile": { "hostname": "test" }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        let result: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(result["error"].as_str().unwrap().contains("index"));
    }

    #[tokio::test]
    async fn delete_config_profile_success() {
        let (state, _dir) = test_state_with_store();

        // First, add a profile so index 0 exists.
        let checksum = get_checksum(&state).await;
        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/config/profiles")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": checksum,
                            "profile": { "hostname": "to-delete" }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = response_body(response).await;
        let result: serde_json::Value = serde_json::from_str(&body).unwrap();
        let new_checksum = result["checksum"].as_str().unwrap();

        // Delete that profile.
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri(format!(
                        "/config/profiles/0?expected_checksum={}",
                        new_checksum
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let result: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(!result["yaml"].as_str().unwrap().contains("to-delete"));
        assert!(!result["checksum"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn delete_config_profile_stale_checksum() {
        let (state, _dir) = test_state_with_store();

        // Add a profile first.
        let checksum = get_checksum(&state).await;
        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/config/profiles")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": checksum,
                            "profile": { "hostname": "stale-test" }
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        // Delete with a wrong checksum.
        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("DELETE")
                    .uri("/config/profiles/0?expected_checksum=wrong-checksum")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
    }
}
