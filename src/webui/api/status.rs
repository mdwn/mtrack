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

use axum::{extract::State, response::IntoResponse, Json};
use serde_json::json;

use super::super::server::WebUiState;
use crate::build_info;
use crate::webui::wire::status::{BuildInfo, ControllersRestarted, Status};

/// GET /api/status — returns build info, hardware status, and controller status.
pub(super) async fn get_status(State(state): State<WebUiState>) -> impl IntoResponse {
    let hardware = state.player.hardware_status();
    let controllers = state.player.controller_statuses();
    Json(Status {
        build: BuildInfo {
            version: build_info::VERSION.to_string(),
            git_hash: build_info::GIT_HASH.to_string(),
            build_time: build_info::BUILD_TIME.to_string(),
            ui_build: super::super::server::ui_build(),
        },
        hardware,
        controllers,
        locked: state.player.is_locked(),
    })
}

/// POST /api/controllers/restart — restarts all controllers from current config.
///
/// Rejected during playback to avoid disrupting active control surfaces.
pub(super) async fn restart_controllers(State(state): State<WebUiState>) -> impl IntoResponse {
    if state.player.is_playing().await {
        return (
            axum::http::StatusCode::CONFLICT,
            Json(json!({"error": "Cannot restart controllers during playback"})),
        )
            .into_response();
    }
    match state.player.reload_controllers().await {
        Ok(()) => {
            let restarted = ControllersRestarted {
                status: "restarted".to_string(),
                controllers: state.player.controller_statuses(),
            };
            (axum::http::StatusCode::OK, Json(restarted)).into_response()
        }
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("{}", e)})),
        )
            .into_response(),
    }
}

/// GET /api/lock — returns the current lock state.
pub(super) async fn get_lock(State(state): State<WebUiState>) -> impl IntoResponse {
    Json(json!({"locked": state.player.is_locked()}))
}

/// PUT /api/lock — sets the lock state.
pub(super) async fn put_lock(
    State(state): State<WebUiState>,
    Json(body): Json<LockRequest>,
) -> impl IntoResponse {
    state.player.set_locked(body.locked);
    Json(json!({"locked": state.player.is_locked()}))
}

#[derive(serde::Deserialize)]
pub(super) struct LockRequest {
    locked: bool,
}

#[cfg(test)]
mod test {
    use super::super::super::api;
    use super::super::test_helpers::*;
    use axum::body::Body;
    use http::StatusCode;
    use tower::ServiceExt;

    #[tokio::test]
    async fn get_status_returns_build_and_hardware() {
        let (state, _dir) = test_state();
        let app = api::router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("GET")
                    .uri("/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();

        // Build info is present.
        assert!(parsed["build"]["version"].is_string());
        assert!(parsed["build"]["git_hash"].is_string());
        assert!(parsed["build"]["build_time"].is_string());
        let ui_build = &parsed["build"]["ui_build"];
        assert!(ui_build.is_string() || ui_build.is_null(), "{ui_build}");

        // Hardware section is present with all subsystems.
        assert!(parsed["hardware"]["init_done"].is_boolean());
        // Profile fields are present (may be null in test state).
        assert!(!parsed["hardware"]["hostname"].is_object());
        assert!(!parsed["hardware"]["profile"].is_object());
        assert!(parsed["hardware"]["audio"]["status"].is_string());
        assert!(parsed["hardware"]["midi"]["status"].is_string());
        assert!(parsed["hardware"]["dmx"]["status"].is_string());
        assert!(parsed["hardware"]["trigger"]["status"].is_string());
    }

    #[tokio::test]
    async fn get_status_no_devices_shows_not_connected() {
        let (state, _dir) = test_state();
        let app = api::router().with_state(state);

        let response = app
            .oneshot(
                http::Request::builder()
                    .method("GET")
                    .uri("/status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let body = response_body(response).await;
        let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();

        // Test state has init_done=true and no devices, so all should be not_connected.
        assert_eq!(parsed["hardware"]["init_done"], true);
        assert_eq!(parsed["hardware"]["audio"]["status"], "not_connected");
        assert_eq!(parsed["hardware"]["midi"]["status"], "not_connected");
        assert_eq!(parsed["hardware"]["dmx"]["status"], "not_connected");
        assert_eq!(parsed["hardware"]["trigger"]["status"], "not_connected");
    }

    /// The status is what the `json!` it replaced built: that `json!`,
    /// kept here as the oracle.
    #[test]
    fn status_matches_the_old_json() {
        use crate::player::{
            AudioOutputHealth, ControllerStatus, HardwareStatusSnapshot, LightingVenueStatus,
            SubsystemStatus, TestOutputStatus,
        };
        use crate::webui::wire::status::*;
        use serde_json::json;

        let subsystem = |status: &str, error: Option<&str>| SubsystemStatus {
            status: status.to_string(),
            name: Some("dev".to_string()),
            error: error.map(String::from),
        };
        let hardware = HardwareStatusSnapshot {
            init_done: true,
            hostname: Some("pi".to_string()),
            profile: None,
            audio: subsystem("connected", None),
            audio_output: Some(AudioOutputHealth {
                status: crate::audio::health::OutputStatus::NeverStarted,
                callback_alive: false,
                writing_signal: false,
                since_last_callback_ms: None,
                since_last_signal_ms: Some(5),
                callbacks: 0,
                recoveries: 1,
                underruns: 2,
                last_error: None,
            }),
            midi: subsystem("failed", Some("gone")),
            dmx: subsystem("not_connected", None),
            trigger: subsystem("initializing", None),
            lighting_venue: LightingVenueStatus {
                name: None,
                status: "none".to_string(),
                error: None,
            },
            test_output: Some(TestOutputStatus {
                universe: 1,
                address: 10,
                footprint: 4,
                fixture_type: "par".to_string(),
                mode: None,
                expires_in_secs: 2.5,
            }),
        };
        let controllers = vec![ControllerStatus {
            kind: "osc".to_string(),
            status: "running".to_string(),
            detail: Some("port 9000".to_string()),
            error: None,
        }];
        let status = Status {
            build: BuildInfo {
                version: "1".to_string(),
                git_hash: "abc".to_string(),
                build_time: "now".to_string(),
                ui_build: None,
            },
            hardware: hardware.clone(),
            controllers: controllers.clone(),
            locked: true,
        };
        let old_subsystem = |status: &str, error: Option<&str>| {
            let mut s = json!({"status": status, "name": "dev"});
            if let Some(error) = error {
                s["error"] = json!(error);
            }
            s
        };
        let old_hardware = json!({
            "init_done": true,
            "hostname": "pi",
            "profile": null,
            "audio": old_subsystem("connected", None),
            "audio_output": {
                "status": "never_started",
                "callback_alive": false,
                "writing_signal": false,
                "since_last_callback_ms": null,
                "since_last_signal_ms": 5,
                "callbacks": 0,
                "recoveries": 1,
                "underruns": 2,
                "last_error": null,
            },
            "midi": old_subsystem("failed", Some("gone")),
            "dmx": old_subsystem("not_connected", None),
            "trigger": old_subsystem("initializing", None),
            "lighting_venue": {"name": null, "status": "none", "error": null},
            "test_output": {
                "universe": 1,
                "address": 10,
                "footprint": 4,
                "fixture_type": "par",
                "mode": null,
                "expires_in_secs": 2.5,
            },
        });
        let old_controllers = json!([{
            "kind": "osc",
            "status": "running",
            "detail": "port 9000",
            "error": null,
        }]);
        assert_eq!(
            json!(status),
            json!({
                "build": {
                    "version": "1",
                    "git_hash": "abc",
                    "build_time": "now",
                    "ui_build": null,
                },
                "hardware": old_hardware,
                "controllers": old_controllers,
                "locked": true,
            })
        );
        assert_eq!(
            json!(ControllersRestarted {
                status: "restarted".to_string(),
                controllers,
            }),
            json!({"status": "restarted", "controllers": old_controllers})
        );
    }
}
