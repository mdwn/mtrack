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
    match super::helpers::spawn_blocking_io("read config", move || {
        crate::util::read_to_string(&path)
    })
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
    // Where the songs and playlists live is checked before the save, and the
    // songs directory made where startup would: a config whose songs cannot
    // be read is refused, leaving the config, the song paths and the songs
    // as they were, rather than saved with the player on songs from
    // somewhere else. Only a save that moves them is checked: one that
    // leaves them as they are is not refused for a location in use that has
    // since become unreadable.
    let mut made_songs_dir = None;
    if state.player.config_store().is_some() {
        let player = state.player.clone();
        let yaml = body.clone();
        let checked = tokio::task::spawn_blocking(move || {
            let config = config::Player::deserialize_from_str(&yaml).map_err(|e| e.to_string())?;
            player.prepare_song_sources(&config)
        })
        .await
        .unwrap_or_else(|e| Err(e.to_string()));
        match checked {
            Ok(made) => made_songs_dir = made,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": format!("the songs or playlists cannot be used: {e}")})),
                )
                    .into_response();
            }
        }
    }
    // A songs directory the check made for a save that then does not
    // happen goes again, while it is still empty.
    let unmake = move || {
        if let Some(dir) = made_songs_dir {
            let _ = std::fs::remove_dir(dir);
        }
    };

    // The store persists its own copy of this file on every mutation: the
    // copy is read back from the save before any mutation can land, or the
    // next one writes the copy from before the save over it.
    let store = state.player.config_store();
    let disk_write = match &store {
        Some(store) => Some(store.lock_for_disk_write().await),
        None => None,
    };
    let path = state.config_path.clone();
    let previous = match super::helpers::spawn_blocking_io("write config", move || {
        // A missing file has nothing to put back: a failed save removes it.
        let previous = match crate::util::read_file(&path) {
            Ok(previous) => Some(previous),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("failed to read the config before saving: {e}")),
        };
        config_io::staged_write(&path, &body)?;
        Ok(previous)
    })
    .await
    {
        Ok(previous) => previous,
        Err(e) => {
            unmake();
            return e;
        }
    };
    if let Some(disk_write) = disk_write {
        // The store keeps its own copy and checksum, and a failed reload
        // leaves the old ones: the file goes back as it was, still under the
        // store's lock, so a failed save changes nothing rather than waiting
        // for the next settings change to write over it.
        let path = state.config_path.clone();
        let restore = move || {
            match previous {
                Some(previous) => crate::util::write_file(&path, &previous),
                None => std::fs::remove_file(&path),
            }
            .map_err(|e| e.to_string())
        };
        if let Err(e) = disk_write.reload_or_restore(restore).await {
            warn!("Config reload after config write failed: {}", e);
            unmake();
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({
                    "error": format!("the config could not be loaded, and was not saved: {e}")
                })),
            )
                .into_response();
        }
    }
    // The running hardware was set up from the config before the save, and
    // matches it only once reloaded from the store's copy, which now holds it.
    let reload = apply_config_change(&state).await;
    (
        StatusCode::OK,
        Json(with_reload(json!({"status": "saved"}), reload)),
    )
        .into_response()
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
        config::ConfigError::InvalidProfileIndex { .. } | config::ConfigError::Validation(_) => (
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

/// Helper to convert a ConfigSnapshot into a JSON response, with what became
/// of the hardware reload.
fn config_snapshot_response(
    snapshot: config::store::ConfigSnapshot,
    status: StatusCode,
    reload: serde_json::Map<String, serde_json::Value>,
) -> axum::response::Response {
    (
        status,
        Json(with_reload(
            json!({"yaml": snapshot.yaml, "checksum": snapshot.checksum}),
            reload,
        )),
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

/// Applies a saved config change to the running hardware, as every
/// surface does after a save (see [`crate::player::ConfigApply`]), and
/// returns what the reply says of it: `applied`, and a `message` when it is
/// not. A save is made whatever the player is doing: while a song plays, is
/// paused or fading out, or an auto play waits out its gap, the hardware
/// reload waits until the player is idle, and the save answers with
/// `applied: false` and a message saying so. It used to be refused with a
/// 409, though the gRPC and MCP config changes saved and deferred.
/// Non-blocking: device discovery runs in the background.
pub(super) async fn apply_config_change(
    state: &WebUiState,
) -> serde_json::Map<String, serde_json::Value> {
    config_apply_fields(&state.player.apply_config_change().await)
}

/// The reply's fields for what became of a saved config change.
fn config_apply_fields(
    outcome: &crate::player::ConfigChange,
) -> serde_json::Map<String, serde_json::Value> {
    // The reload brings the controllers in line with the config itself,
    // restarting only those it changed, or whose MIDI device it reopened.
    // Forcing them all here as well tore down every OSC, gRPC and MCP
    // client for a change that was not theirs.
    let mut fields = serde_json::Map::new();
    fields.insert("applied".into(), json!(outcome.applied()));
    // Which it was, for the web UI to say so: a save deferred until the
    // player is idle reads otherwise as one made.
    fields.insert("reload".into(), json!(outcome.kind()));
    if let Some(message) = outcome.message() {
        fields.insert("message".into(), json!(message));
    }
    // A song reload that failed beside it, carried rather than hiding the
    // hardware's outcome.
    if let Some(songs_error) = &outcome.songs_error {
        fields.insert("songs_error".into(), json!(songs_error));
    }
    fields
}

/// Adds what became of a save's hardware reload to its reply.
pub(super) fn with_reload(
    mut body: serde_json::Value,
    reload: serde_json::Map<String, serde_json::Value>,
) -> serde_json::Value {
    if let Some(body) = body.as_object_mut() {
        body.extend(reload);
    }
    body
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
/// Handles: checksum extraction, deserialization, validation,
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
            let reload = apply_config_change(state).await;
            config_snapshot_response(snapshot, StatusCode::OK, reload)
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
            let reload = apply_config_change(&state).await;
            config_snapshot_response(snapshot, StatusCode::OK, reload)
        }
        Err(e) => config_store_error_response(e),
    }
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
        assert_eq!(result["applied"], serde_json::json!(true), "{result}");
    }

    /// A raw config save wrote the file and left the store's copy as it
    /// was: the next settings change persisted that copy over the save.
    #[tokio::test]
    async fn a_settings_change_after_a_raw_config_save_keeps_the_save() {
        let (state, _dir) = test_state_with_store();
        let config_path = state.config_path.clone();
        let response = router()
            .with_state(state.clone())
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config")
                    .body(Body::from("songs: other-songs\n"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

        let checksum = get_checksum(&state).await;
        let response = router()
            .with_state(state)
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
        let written = std::fs::read_to_string(&config_path).unwrap();
        assert!(
            written.contains("other-songs"),
            "the raw save was written over: {written}"
        );
    }

    /// A raw config save the store could not load back was written and left
    /// on disk, with the store's old copy and checksum: the next settings
    /// change wrote that copy over it, silently.
    #[tokio::test]
    async fn a_raw_config_save_that_cannot_be_loaded_changes_nothing() {
        let (state, dir) = test_state_with_store();
        let config_path = state.config_path.clone();
        let before = std::fs::read_to_string(&config_path).unwrap();
        let checksum = get_checksum(&state).await;
        // Valid by itself, but its profiles directory holds a broken profile.
        let profiles = dir.path().join("broken-profiles");
        std::fs::create_dir(&profiles).unwrap();
        std::fs::write(profiles.join("broken.yaml"), "audio: [[[").unwrap();

        let response = router()
            .with_state(state.clone())
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config")
                    .body(Body::from(
                        "songs: other-songs\nprofiles_dir: broken-profiles\n",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
        let error = parsed["error"].as_str().unwrap();
        assert!(error.contains("was not saved"), "{error}");
        assert_eq!(std::fs::read_to_string(&config_path).unwrap(), before);
        assert_eq!(get_checksum(&state).await, checksum);
    }

    /// A raw config save reloaded the store but not the hardware, which ran
    /// on the config from before it until a restart.
    #[tokio::test]
    async fn a_raw_config_save_reloads_the_hardware() {
        let (state, _dir) = test_state_with_store();
        assert!(state.player.audio_device().is_none());

        let response = router()
            .with_state(state.clone())
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config")
                    .body(Body::from(
                        "songs: songs\naudio:\n  device: mock-device\n  track_mappings:\n    kick: [1]\n",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
        assert_eq!(body["applied"], serde_json::json!(true), "{body}");
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            state.player.await_hardware_ready(),
        )
        .await
        .expect("the hardware never came up");
        assert!(
            state.player.audio_device().is_some(),
            "the saved audio device was not set up"
        );
    }

    /// Sets a config store on the state's player, from its config file.
    fn set_store(state: &crate::webui::server::WebUiState) {
        let path = state.config_path.clone();
        let cfg = crate::config::Player::deserialize(&path).unwrap();
        let store = std::sync::Arc::new(crate::config::ConfigStore::new(cfg, path));
        state.player.set_config_store(store);
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
        let reload = super::apply_config_change(&state).await;
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

        let reload = super::apply_config_change(&state).await;
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

    /// A save in an auto play's gap reset the hardware, and the song failed
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

        let response = router()
            .with_state(state.clone())
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
        let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
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

        let response = router()
            .with_state(state.clone())
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
        let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
        assert_eq!(body["status"], "saved");
        assert_eq!(body["applied"], serde_json::json!(false), "{body}");
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
        let reload = super::apply_config_change(&state).await;
        assert_eq!(reload["applied"], serde_json::json!(true), "{reload:?}");
    }

    /// Puts `yaml` as the whole config, and answers the reply's status and
    /// body.
    async fn put_whole_config(
        state: &crate::webui::server::WebUiState,
        yaml: &str,
    ) -> (StatusCode, serde_json::Value) {
        let response = router()
            .with_state(state.clone())
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config")
                    .body(Body::from(yaml.to_string()))
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

    /// A `songs:` naming a directory not there yet, inside the project, is
    /// made, as startup makes it, and the songs reload from it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_runtime_songs_change_to_a_new_directory_makes_it() {
        let (state, dir) = test_state();
        set_store(&state);
        let (status, body) = put_whole_config(&state, "songs: newdir\n").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["applied"], serde_json::json!(true), "{body}");
        assert!(
            dir.path().join("newdir").is_dir(),
            "the directory was not made"
        );
        assert!(state.songs_path().ends_with("newdir"));
    }

    /// A `songs:` that cannot be used (outside the project and not there)
    /// is refused before the save: the config, the song paths and the songs
    /// are left as they were. It used to be saved, said applied, with every
    /// song path after it pointing at the missing directory.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_songs_location_that_cannot_be_used_is_refused_before_the_save() {
        let (state, dir) = test_state();
        set_store(&state);
        let store = state.player.config_store().unwrap();
        let (_, checksum) = store.read_yaml().await.unwrap();
        let file = std::fs::read_to_string(&state.config_path).unwrap();
        let before = state.songs_path();
        let songs = state.player.songs().len();
        let missing = dir
            .path()
            .parent()
            .unwrap()
            .join("mtrack-no-such-songs-dir");
        let (status, body) =
            put_whole_config(&state, &format!("songs: {}\n", missing.display())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body["error"].as_str().unwrap().contains("songs"), "{body}");
        assert!(!missing.exists());
        assert_eq!(std::fs::read_to_string(&state.config_path).unwrap(), file);
        assert_eq!(store.read_yaml().await.unwrap().1, checksum);
        assert_eq!(state.songs_path(), before, "the song paths moved");
        assert_eq!(state.player.songs().len(), songs);
    }

    /// A save that leaves the songs where they are is not refused because
    /// the location in use has become unreadable: only a save that moves
    /// them is checked.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_leaving_the_songs_where_they_are_is_not_checked() {
        let (state, _dir) = test_state();
        set_store(&state);
        std::fs::remove_dir_all(state.songs_path()).unwrap();
        let (status, body) = put_whole_config(&state, "songs: songs\n").await;
        assert_eq!(status, StatusCode::OK, "{body}");
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

    /// A runtime change to `songs:` takes effect without a restart: the save
    /// reloads the new set, and the web UI's song files are read from it,
    /// as every surface's reload reads it.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_runtime_songs_change_reloads_the_new_set_over_the_web() {
        let (state, dir) = test_state();
        set_store(&state);
        let moved = dir.path().join("other").join("moved");
        std::fs::create_dir_all(&moved).unwrap();
        crate::testutil::write_wav(moved.join("kick.wav"), vec![vec![0_i32; 441]], 44100).unwrap();
        std::fs::write(
            moved.join("song.yaml"),
            "name: Moved Song\ntracks:\n- name: kick\n  file: kick.wav\n",
        )
        .unwrap();

        let response = router()
            .with_state(state.clone())
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config")
                    .body(Body::from("songs: other\n"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            state.player.songs().get("Moved Song").is_ok(),
            "the new set"
        );
        assert!(
            state.songs_path().ends_with("other"),
            "{:?}",
            state.songs_path()
        );

        // A song edit's reload reads it too.
        super::super::helpers::reload_songs(&state).await.unwrap();
        assert!(state.player.songs().get("Moved Song").is_ok());
    }

    /// The reply to a save says what became of it on the hardware, as the
    /// gRPC and MCP replies do: applied, deferred with a message, or saved
    /// and not applied because the reload failed, with the reason.
    #[test]
    fn a_saves_reply_says_what_became_of_it() {
        use crate::player::ConfigApply;
        let applied = super::config_apply_fields(&ConfigApply::Applied.into());
        assert_eq!(applied["applied"], serde_json::json!(true));
        assert!(!applied.contains_key("message"));
        let deferred = super::config_apply_fields(&ConfigApply::Deferred.into());
        assert_eq!(deferred["applied"], serde_json::json!(false));
        assert_eq!(deferred["reload"], "deferred");
        assert!(deferred["message"].as_str().unwrap().contains("idle"));
        let failed = super::config_apply_fields(&ConfigApply::Failed("no store".into()).into());
        assert_eq!(failed["applied"], serde_json::json!(false));
        assert_eq!(failed["reload"], "failed");
        let message = failed["message"].as_str().unwrap();
        assert!(
            message.contains("saved") && message.contains("no store"),
            "{message}"
        );
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

    /// A second MIDI controller was saved and applied, and mtrack then
    /// failed to start on the config it wrote. It is refused with a 400, and
    /// the file is left as it was.
    #[tokio::test]
    async fn put_config_controllers_refuses_a_config_that_would_not_load() {
        let (state, _dir) = test_state_with_store();
        let checksum = get_checksum(&state).await;
        let path = state.config_path.clone();
        let before = std::fs::read_to_string(&path).unwrap();
        let midi = crate::config::Controller::Midi(crate::config::MidiController::new(
            crate::config::midi::note_on(16, 0, 127),
            crate::config::midi::note_on(16, 1, 127),
            crate::config::midi::note_on(16, 2, 127),
            crate::config::midi::note_on(16, 3, 127),
            crate::config::midi::note_on(16, 4, 127),
            crate::config::midi::note_on(16, 5, 127),
        ));

        let app = router().with_state(state.clone());
        let response = app
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/config/controllers")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::to_string(&serde_json::json!({
                            "expected_checksum": checksum,
                            "controllers": [midi, midi]
                        }))
                        .unwrap(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = response_body(response).await;
        assert!(body.contains("only one can watch"), "{body}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before);
        assert_eq!(get_checksum(&state).await, checksum);
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
