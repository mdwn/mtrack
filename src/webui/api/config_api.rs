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

use tracing::warn;

use super::super::config_io;
use super::super::server::WebUiState;
use crate::config;

/// GET /api/config — returns the raw YAML content of the player config file.
pub(super) async fn get_config_raw(State(state): State<WebUiState>) -> impl IntoResponse {
    // codeql[rust/path-injection] config_path is set at startup, not from user input.
    let path = state.config_path.clone();
    match super::helpers::spawn_blocking_io("read config", move || std::fs::read_to_string(&path))
        .await
    {
        Ok(content) => (
            StatusCode::OK,
            [("content-type", "text/yaml; charset=utf-8")],
            content,
        )
            .into_response(),
        Err(e) => e,
    }
}

/// GET /api/config/parsed — returns the player config as JSON.
pub(super) async fn get_config_parsed(State(state): State<WebUiState>) -> impl IntoResponse {
    let path = state.config_path.clone();
    let player_config = match super::helpers::spawn_blocking_io("parse config", move || {
        config::Player::deserialize(&path).map_err(|e| e.to_string())
    })
    .await
    {
        Ok(c) => c,
        Err(e) => return e,
    };
    match serde_json::to_value(&player_config) {
        Ok(json_val) => (StatusCode::OK, Json(json_val)).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to serialize config: {}", e)})),
        )
            .into_response(),
    }
}

/// PUT /api/config — validates and atomically writes the player config.
pub(super) async fn put_config(State(state): State<WebUiState>, body: String) -> impl IntoResponse {
    if let Err(errors) = config_io::validate_player_config(&body) {
        return (StatusCode::BAD_REQUEST, Json(json!({"errors": errors}))).into_response();
    }

    let path = state.config_path.clone();
    match super::helpers::spawn_blocking_io("write config", move || {
        config_io::atomic_write(&path, &body)
    })
    .await
    {
        Ok(()) => (StatusCode::OK, Json(json!({"status": "saved"}))).into_response(),
        Err(e) => e,
    }
}

/// POST /api/config/validate — validates player config YAML without saving.
pub(super) async fn validate_config(body: String) -> impl IntoResponse {
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
pub(super) async fn reject_if_playing(state: &WebUiState) -> Option<axum::response::Response> {
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

/// Reloads hardware and controllers from the updated config. Non-blocking —
/// spawns async device discovery and returns immediately. The broadcast channel
/// is already stored on the Player and will be wired when the DMX engine comes up.
pub(super) async fn reload_hardware_after_mutation(state: &WebUiState) {
    if let Err(e) = state.player.reload_hardware().await {
        warn!("Hardware reload failed: {}", e);
    }
    if let Err(e) = state.player.reload_controllers().await {
        warn!("Controller reload failed: {}", e);
    }
}

/// GET /api/config/store — returns config YAML + checksum via the ConfigStore.
pub(super) async fn get_config_store(State(state): State<WebUiState>) -> impl IntoResponse {
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

/// Trait for config section types that support validation.
trait Validatable {
    fn validate(&self) -> Result<(), Vec<String>>;
}
impl Validatable for config::Audio {
    fn validate(&self) -> Result<(), Vec<String>> {
        self.validate()
    }
}
impl Validatable for config::Midi {
    fn validate(&self) -> Result<(), Vec<String>> {
        self.validate()
    }
}
impl Validatable for config::Dmx {
    fn validate(&self) -> Result<(), Vec<String>> {
        self.validate()
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

/// Deserializes an optional config section from a JSON body field.
/// Returns `None` if the field is absent or null; returns a 400 error on
/// deserialization failure.
#[allow(clippy::result_large_err)]
fn deserialize_optional_section<T: serde::de::DeserializeOwned>(
    body: &serde_json::Value,
    field: &str,
) -> Result<Option<T>, axum::response::Response> {
    match body.get(field) {
        Some(v) if !v.is_null() => serde_json::from_value(v.clone()).map(Some).map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": format!("invalid {}: {}", field, e)})),
            )
                .into_response()
        }),
        _ => Ok(None),
    }
}

/// Shared preamble for optional, validatable config section updates.
///
/// Handles: reject_if_playing, checksum extraction, deserialization, validation,
/// config store lookup. Returns the deserialized value, checksum, and store on
/// success.
#[allow(clippy::result_large_err)]
async fn prepare_optional_update<T>(
    state: &WebUiState,
    body: &serde_json::Value,
    field: &str,
) -> Result<(Option<T>, String, std::sync::Arc<config::ConfigStore>), axum::response::Response>
where
    T: serde::de::DeserializeOwned + Validatable,
{
    if let Some(resp) = reject_if_playing(state).await {
        return Err(resp);
    }
    let checksum = extract_checksum(body)?;
    let value: Option<T> = deserialize_optional_section(body, field)?;
    if let Some(ref v) = value {
        if let Err(errors) = v.validate() {
            return Err((StatusCode::BAD_REQUEST, Json(json!({"errors": errors}))).into_response());
        }
    }
    let store = require_config_store(state)?;
    Ok((value, checksum, store))
}

/// Completes a config section update: calls the store updater, reloads hardware,
/// and returns the response.
async fn finish_config_update(
    state: &WebUiState,
    result: Result<config::store::ConfigSnapshot, config::ConfigError>,
) -> axum::response::Response {
    match result {
        Ok(snapshot) => {
            reload_hardware_after_mutation(state).await;
            config_snapshot_response(snapshot, StatusCode::OK)
        }
        Err(e) => config_store_error_response(e),
    }
}

/// PUT /api/config/audio — update audio config section.
pub(super) async fn put_config_audio(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let (audio, checksum, store) =
        match prepare_optional_update::<config::Audio>(&state, &body, "audio").await {
            Ok(t) => t,
            Err(e) => return e,
        };
    finish_config_update(&state, store.update_audio(audio, &checksum).await).await
}

/// PUT /api/config/midi — update MIDI config section.
pub(super) async fn put_config_midi(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let (midi, checksum, store) =
        match prepare_optional_update::<config::Midi>(&state, &body, "midi").await {
            Ok(t) => t,
            Err(e) => return e,
        };
    finish_config_update(&state, store.update_midi(midi, &checksum).await).await
}

/// PUT /api/config/dmx — update DMX config section.
pub(super) async fn put_config_dmx(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let (dmx, checksum, store) =
        match prepare_optional_update::<config::Dmx>(&state, &body, "dmx").await {
            Ok(t) => t,
            Err(e) => return e,
        };
    finish_config_update(&state, store.update_dmx(dmx, &checksum).await).await
}

/// PUT /api/config/controllers — update controllers config.
pub(super) async fn put_config_controllers(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_playing(&state).await {
        return resp;
    }
    let checksum = match extract_checksum(&body) {
        Ok(c) => c,
        Err(e) => return e,
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
pub(super) async fn put_config_samples(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_playing(&state).await {
        return resp;
    }

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
            reload_hardware_after_mutation(&state).await;
            config_snapshot_response(snapshot, StatusCode::OK)
        }
        Err(e) => config_store_error_response(e),
    }
}

/// POST /api/config/profiles — add a new profile.
pub(super) async fn post_config_profile(
    State(state): State<WebUiState>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if let Some(resp) = reject_if_playing(&state).await {
        return resp;
    }

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
            reload_hardware_after_mutation(&state).await;
            config_snapshot_response(snapshot, StatusCode::CREATED)
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
    if let Some(resp) = reject_if_playing(&state).await {
        return resp;
    }

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
    match store.update_profile(index, profile, &checksum).await {
        Ok(snapshot) => {
            reload_hardware_after_mutation(&state).await;
            config_snapshot_response(snapshot, StatusCode::OK)
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

#[cfg(test)]
mod test {
    use super::super::router;
    use super::super::test_helpers::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use tower::ServiceExt;

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
    async fn put_config_write_failure_returns_500() {
        let (mut state, _dir) = test_state();
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

    #[tokio::test]
    async fn put_config_audio_success() {
        let (state, _dir) = test_state_with_store();
        let checksum = get_checksum(&state).await;

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config/audio")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": checksum,
                            "audio": { "device": "test-device" }
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
        assert!(result["yaml"].is_string());
        assert!(result["checksum"].is_string());
        assert!(!result["checksum"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn put_config_audio_stale_checksum() {
        let (state, _dir) = test_state_with_store();

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config/audio")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": "wrong-checksum",
                            "audio": { "device": "test-device" }
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
    async fn put_config_audio_no_store() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config/audio")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": "abc",
                            "audio": { "device": "test" }
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
    async fn put_config_midi_success() {
        let (state, _dir) = test_state_with_store();
        let checksum = get_checksum(&state).await;

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config/midi")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": checksum,
                            "midi": { "device": "test-midi-device" }
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
        assert!(result["yaml"].is_string());
        assert!(!result["checksum"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn put_config_dmx_success() {
        let (state, _dir) = test_state_with_store();
        let checksum = get_checksum(&state).await;

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config/dmx")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": checksum,
                            "dmx": { "dim_speed_modifier": 1.0, "universes": [] }
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
        assert!(result["yaml"].is_string());
        assert!(!result["checksum"].as_str().unwrap().is_empty());
    }

    #[tokio::test]
    async fn put_config_controllers_success() {
        let (state, _dir) = test_state_with_store();
        let checksum = get_checksum(&state).await;

        let app = router().with_state(state);
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config/controllers")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": checksum,
                            "controllers": []
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
        assert!(result["yaml"].is_string());
        assert!(!result["checksum"].as_str().unwrap().is_empty());
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
