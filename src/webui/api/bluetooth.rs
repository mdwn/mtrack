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

use std::time::{Duration, Instant};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use super::super::server::WebUiState;
use crate::controller::ble::{BleError, BleHandle, PairingState};
use crate::webui::wire::bluetooth::{
    BluetoothPhone, BluetoothStatus, PairingRequest, PairingWindow,
};

/// How long a pairing window stays open when the request names no time.
const DEFAULT_WINDOW: Duration = Duration::from_secs(120);
/// The shortest and longest window a request may ask for, in seconds.
const WINDOW_RANGE: std::ops::RangeInclusive<u32> = 10..=600;

/// The Bluetooth controller as it is now: configured when the running config
/// has one, running when its handle answers. Its failure text is the
/// controller status's own.
async fn status_of(state: &WebUiState) -> BluetoothStatus {
    let statuses = state.player.controller_statuses();
    let ble = statuses.iter().find(|s| s.kind == "ble");
    let handle = state.player.ble_handle();
    let mut status = BluetoothStatus {
        configured: ble.is_some() || handle.is_some(),
        running: false,
        error: None,
        pairing: PairingWindow {
            open: false,
            seconds_left: 0,
        },
        devices: Vec::new(),
    };
    let Some(handle) = handle.as_ref() else {
        status.error = ble.and_then(|s| s.error.clone());
        return status;
    };
    let Ok(window) = handle.pairing_state() else {
        // A controller that is retrying says why itself; one that stopped
        // for good says it through its status.
        status.error = handle
            .failure()
            .or_else(|| ble.and_then(|s| s.error.clone()));
        return status;
    };
    status.running = true;
    if let PairingState::Open { until } = window {
        let left = until.saturating_duration_since(Instant::now());
        status.pairing = PairingWindow {
            open: !left.is_zero(),
            seconds_left: u32::try_from(left.as_secs_f64().ceil() as u64).unwrap_or(u32::MAX),
        };
    }
    match handle.paired_devices().await {
        Ok(phones) => {
            status.devices = phones
                .into_iter()
                .map(|p| BluetoothPhone {
                    address: p.address,
                    name: p.name,
                    connected: p.connected,
                })
                .collect();
        }
        Err(e) => status.error = Some(e.to_string()),
    }
    status
}

/// The running controller's handle, or the 409 that says there is none.
#[allow(clippy::result_large_err)]
fn running_handle(state: &WebUiState) -> Result<BleHandle, Response> {
    state
        .player
        .ble_handle()
        .ok_or_else(|| refusal(&BleError::NotRunning))
}

/// A refusal as a response: 409 when Bluetooth is not running, 400 for an
/// address that is none, 404 for one that is not a paired phone, and 502
/// when the radio stack refused.
fn refusal(e: &BleError) -> Response {
    let (status, message) = match e {
        BleError::NotRunning => (StatusCode::CONFLICT, e.to_string()),
        BleError::BadAddress(_) => (
            StatusCode::BAD_REQUEST,
            "Not a Bluetooth address".to_string(),
        ),
        BleError::NotPaired(_) => (
            StatusCode::NOT_FOUND,
            "Not a phone paired with this player".to_string(),
        ),
        BleError::Adapter(_) => (StatusCode::BAD_GATEWAY, e.to_string()),
    };
    (status, Json(json!({"error": message}))).into_response()
}

/// GET /api/bluetooth — whether Bluetooth runs, the pairing window and the
/// paired phones.
pub(super) async fn get_bluetooth(State(state): State<WebUiState>) -> impl IntoResponse {
    Json(status_of(&state).await)
}

/// POST /api/bluetooth/pairing — opens the pairing window (for `seconds`,
/// 120 by default, kept between 10 and 600) or closes it. Refused while the
/// player is locked: who may control the player is not for a locked player
/// to change.
pub(super) async fn post_pairing(
    State(state): State<WebUiState>,
    Json(body): Json<PairingRequest>,
) -> Response {
    let handle = match running_handle(&state) {
        Ok(handle) => handle,
        Err(response) => return response,
    };
    let result = if body.open {
        let seconds = body.seconds.map_or(DEFAULT_WINDOW.as_secs() as u32, |s| {
            s.clamp(*WINDOW_RANGE.start(), *WINDOW_RANGE.end())
        });
        handle
            .open_pairing(Duration::from_secs(u64::from(seconds)))
            .await
    } else {
        handle.close_pairing().await
    };
    match result {
        Ok(()) => Json(status_of(&state).await).into_response(),
        Err(e) => refusal(&e),
    }
}

/// DELETE /api/bluetooth/devices/{address} — forgets a paired phone, which
/// must pair again.
pub(super) async fn delete_device(
    State(state): State<WebUiState>,
    Path(address): Path<String>,
) -> Response {
    let handle = match running_handle(&state) {
        Ok(handle) => handle,
        Err(response) => return response,
    };
    match handle.forget(&address).await {
        Ok(()) => Json(status_of(&state).await).into_response(),
        Err(e) => refusal(&e),
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::Router;
    use http::StatusCode;
    use tower::ServiceExt;

    use super::super::super::server::WebUiState;
    use super::super::test_helpers::*;
    use crate::controller::ble::fake::{device, started, trusted_device, Fake};
    use crate::controller::ble::BleHandle;
    use crate::controller::{Controller, Driver};

    /// A controller that does nothing but hold a Bluetooth handle.
    struct HandleOnly(BleHandle);

    impl Driver for HandleOnly {
        fn monitor_events(&self) -> tokio::task::JoinHandle<Result<(), std::io::Error>> {
            tokio::spawn(std::future::pending())
        }
        fn ble_handle(&self) -> Option<BleHandle> {
            Some(self.0.clone())
        }
    }

    fn app(state: &WebUiState) -> Router {
        Router::new()
            .nest("/api", super::super::guarded_router(state))
            .with_state(state.clone())
    }

    async fn call(
        state: &WebUiState,
        method: &str,
        uri: &str,
        body: &str,
    ) -> (StatusCode, serde_json::Value) {
        let response = app(state)
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = response_body(response).await;
        (status, serde_json::from_str(&body).unwrap_or_default())
    }

    /// A state whose player runs a Bluetooth controller; when
    /// `attached`, over a fake radio holding a phone, a connected phone
    /// and a pedal the owner trusts.
    async fn with_bluetooth(attached: bool) -> (WebUiState, tempfile::TempDir, Arc<Fake>) {
        let (state, dir) = test_state();
        state.player.set_locked(false);
        let handle = BleHandle::default();
        let (fake, pairing) = started(vec![
            device(1, true, true),
            device(2, true, false),
            trusted_device(3, true),
        ])
        .await;
        pairing.close().await.unwrap();
        if attached {
            handle.attach(pairing);
        }
        state
            .player
            .install_controller(Controller::new_from_drivers(vec![Arc::new(HandleOnly(
                handle,
            ))]));
        (state, dir, fake)
    }

    #[tokio::test]
    async fn without_a_bluetooth_controller_nothing_is_configured() {
        let (state, _dir) = test_state();
        state.player.set_locked(false);
        let (status, body) = call(&state, "GET", "/api/bluetooth", "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            body,
            serde_json::json!({
                "configured": false, "running": false, "error": null,
                "pairing": {"open": false, "seconds_left": 0}, "devices": [],
            })
        );
        for (method, uri, body) in [
            ("POST", "/api/bluetooth/pairing", r#"{"open":true}"#),
            ("DELETE", "/api/bluetooth/devices/00:00:00:00:00:01", ""),
        ] {
            assert_eq!(
                call(&state, method, uri, body).await.0,
                StatusCode::CONFLICT,
                "{method} {uri}"
            );
        }
    }

    /// A controller configured on a build without Bluetooth fails to start,
    /// and says why.
    #[cfg(not(all(target_os = "linux", feature = "ble")))]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_controller_that_failed_says_why() {
        let (state, _dir) = test_state();
        state.player.set_locked(false);
        state
            .player
            .start_controllers(vec![crate::config::Controller::Ble(
                crate::config::BleController::new(None),
            )]);
        let (status, body) = call(&state, "GET", "/api/bluetooth", "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["configured"], true, "{body}");
        assert_eq!(body["running"], false, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("Bluetooth"),
            "{body}"
        );
    }

    /// A controller that cannot reach the radio keeps retrying, and says
    /// why until it is running.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_controller_that_is_retrying_says_why() {
        let (state, _dir, _fake) = with_bluetooth(false).await;
        let handle = state.player.ble_handle().expect("handle");
        handle.failed("the radio is blocked");
        let (_, body) = call(&state, "GET", "/api/bluetooth", "").await;
        assert_eq!(body["running"], false, "{body}");
        assert_eq!(body["error"], "the radio is blocked", "{body}");

        let (_fake, pairing) = started(vec![]).await;
        handle.attach(pairing);
        let (_, body) = call(&state, "GET", "/api/bluetooth", "").await;
        assert_eq!(body["running"], true, "{body}");
        assert_eq!(body["error"], serde_json::Value::Null, "{body}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_configured_controller_that_is_not_running_is_closed_and_empty() {
        let (state, _dir, _fake) = with_bluetooth(false).await;
        let (status, body) = call(&state, "GET", "/api/bluetooth", "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["configured"], true);
        assert_eq!(body["running"], false);
        assert_eq!(body["pairing"]["open"], false);
        assert_eq!(body["devices"], serde_json::json!([]));
        let (status, _) = call(&state, "POST", "/api/bluetooth/pairing", r#"{"open":true}"#).await;
        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn it_lists_the_phones_but_not_the_owners_own_devices() {
        let (state, _dir, _fake) = with_bluetooth(true).await;
        let (status, body) = call(&state, "GET", "/api/bluetooth", "").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["running"], true);
        assert_eq!(body["error"], serde_json::Value::Null);
        assert_eq!(
            body["devices"],
            serde_json::json!([
                {"address": "00:00:00:00:00:01", "name": "phone 1", "connected": true},
                {"address": "00:00:00:00:00:02", "name": "phone 2", "connected": false},
            ])
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_window_opens_for_the_time_asked_clamped_and_closes() {
        let (state, _dir, fake) = with_bluetooth(true).await;
        let left = |body: &serde_json::Value| body["pairing"]["seconds_left"].as_u64().unwrap();

        let (status, body) =
            call(&state, "POST", "/api/bluetooth/pairing", r#"{"open":true}"#).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["pairing"]["open"], true);
        assert!((119..=120).contains(&left(&body)), "{body}");

        // The GET sees it too.
        let (_, body) = call(&state, "GET", "/api/bluetooth", "").await;
        assert_eq!(body["pairing"]["open"], true);

        let (_, body) = call(
            &state,
            "POST",
            "/api/bluetooth/pairing",
            r#"{"open":true,"seconds":1}"#,
        )
        .await;
        assert!((9..=10).contains(&left(&body)), "{body}");
        let (_, body) = call(
            &state,
            "POST",
            "/api/bluetooth/pairing",
            r#"{"open":true,"seconds":86400}"#,
        )
        .await;
        assert!((599..=600).contains(&left(&body)), "{body}");
        let (_, body) = call(
            &state,
            "POST",
            "/api/bluetooth/pairing",
            r#"{"open":true,"seconds":45}"#,
        )
        .await;
        assert!((44..=45).contains(&left(&body)), "{body}");

        let (status, body) = call(
            &state,
            "POST",
            "/api/bluetooth/pairing",
            r#"{"open":false}"#,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["pairing"]["open"], false);
        assert_eq!(body["pairing"]["seconds_left"], 0);
        assert_eq!(fake.pairable.lock().last(), Some(&false));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refusing_adapter_is_a_bad_gateway() {
        let (state, _dir, fake) = with_bluetooth(true).await;
        *fake.failing.lock() = true;
        let (status, body) =
            call(&state, "POST", "/api/bluetooth/pairing", r#"{"open":true}"#).await;
        assert_eq!(status, StatusCode::BAD_GATEWAY);
        assert_eq!(body["error"], "adapter gone");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn forgetting_a_phone_removes_it_and_answers_the_list() {
        let (state, _dir, fake) = with_bluetooth(true).await;
        let (status, body) = call(
            &state,
            "DELETE",
            "/api/bluetooth/devices/00:00:00:00:00:01",
            "",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(*fake.removed.lock(), vec![[0, 0, 0, 0, 0, 1]]);
        assert_eq!(body["devices"].as_array().unwrap().len(), 1);
        assert_eq!(body["devices"][0]["address"], "00:00:00:00:00:02");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn forgetting_what_is_not_a_phone_is_refused() {
        let (state, _dir, fake) = with_bluetooth(true).await;
        let (status, _) = call(&state, "DELETE", "/api/bluetooth/devices/nonsense", "").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        // Unknown, and the owner's own.
        for address in ["00:00:00:00:00:09", "00:00:00:00:00:03"] {
            let (status, _) = call(
                &state,
                "DELETE",
                &format!("/api/bluetooth/devices/{address}"),
                "",
            )
            .await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{address}");
        }
        assert!(fake.removed.lock().is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_locked_player_can_be_read_but_not_paired_with() {
        let (state, _dir, fake) = with_bluetooth(true).await;
        state.player.set_locked(true);
        assert_eq!(
            call(&state, "GET", "/api/bluetooth", "").await.0,
            StatusCode::OK
        );
        for (method, uri, body) in [
            ("POST", "/api/bluetooth/pairing", r#"{"open":true}"#),
            ("POST", "/api/bluetooth/pairing", r#"{"open":false}"#),
            ("DELETE", "/api/bluetooth/devices/00:00:00:00:00:01", ""),
        ] {
            assert_eq!(
                call(&state, method, uri, body).await.0,
                StatusCode::LOCKED,
                "{method} {uri}"
            );
        }
        assert!(fake.removed.lock().is_empty());
        assert_eq!(fake.pairable.lock().as_slice(), &[false, false]);
    }
}
