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

/// GET /api/status — returns build info and hardware status.
pub(super) async fn get_status(State(state): State<WebUiState>) -> impl IntoResponse {
    let hardware = state.player.hardware_status();
    Json(json!({
        "build": {
            "version": build_info::VERSION,
            "git_hash": build_info::GIT_HASH,
            "build_time": build_info::BUILD_TIME,
        },
        "hardware": hardware,
    }))
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
}
