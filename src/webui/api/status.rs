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
            let statuses = state.player.controller_statuses();
            (
                axum::http::StatusCode::OK,
                Json(json!({"status": "restarted", "controllers": statuses})),
            )
                .into_response()
        }
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("{}", e)})),
        )
            .into_response(),
    }
}
