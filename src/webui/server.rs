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
    extract::{
        ws::{Message, WebSocket},
        State, WebSocketUpgrade,
    },
    http::{header, StatusCode, Uri},
    response::{Html, IntoResponse, Response},
    routing::{any, get},
    Router,
};
use futures_util::{SinkExt, StreamExt};
use rust_embed::Embed;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, watch};
use tonic_web::GrpcWebLayer;
use tower::{Layer, ServiceExt};
use tracing::{debug, error, info};

use crate::controller::grpc::PlayerServer;
use crate::player::Player;
use crate::proto::player::v1::player_service_server::PlayerServiceServer;
use crate::state::StateSnapshot;

use super::state as ws_state;

#[derive(Embed)]
#[folder = "src/webui/svelte/dist/"]
struct WebUiAssets;

/// Configuration for the web server (address + port).
#[derive(Debug, Clone)]
pub struct WebConfig {
    pub port: u16,
    pub address: String,
}

impl Default for WebConfig {
    fn default() -> Self {
        Self {
            port: 8080,
            address: "127.0.0.1".to_string(),
        }
    }
}

/// Shared state for the web UI.
#[derive(Clone)]
pub struct WebUiState {
    pub player: Arc<Player>,
    pub state_rx: watch::Receiver<Arc<StateSnapshot>>,
    pub broadcast_tx: broadcast::Sender<String>,
    /// Resolved path to the player config file (mtrack.yaml).
    pub config_path: PathBuf,
    /// Resolved path to the songs directory.
    pub songs_path: PathBuf,
    /// Resolved path to the playlist file.
    pub playlist_path: PathBuf,
    /// Fixture metadata JSON (sent to each WebSocket client on connect).
    pub metadata_json: Arc<String>,
    /// Shared waveform cache (sent to each WebSocket client on connect).
    pub waveform_cache: ws_state::WaveformCache,
}

/// Handle to a running web UI server, returned so callers can shut it down.
///
/// When dropped, the server and background tasks are aborted.
pub struct WebUiHandle {
    _shutdown_tx: tokio::sync::oneshot::Sender<()>,
    server_handle: tokio::task::JoinHandle<()>,
    playback_poller_handle: tokio::task::JoinHandle<()>,
    state_poller_handle: tokio::task::JoinHandle<()>,
    log_poller_handle: tokio::task::JoinHandle<()>,
    waveform_poller_handle: tokio::task::JoinHandle<()>,
    waveform_prewarmer_handle: tokio::task::JoinHandle<()>,
}

impl Drop for WebUiHandle {
    fn drop(&mut self) {
        self.server_handle.abort();
        self.playback_poller_handle.abort();
        self.state_poller_handle.abort();
        self.log_poller_handle.abort();
        self.waveform_poller_handle.abort();
        self.waveform_prewarmer_handle.abort();
    }
}

/// Starts the unified web server with dashboard, gRPC-Web, and REST API.
///
/// The server serves:
/// - `/` — Web UI SPA (dashboard, config editor, etc.)
/// - `/ws` — WebSocket for real-time state streaming (playback, fixtures, logs)
/// - `/player.v1.PlayerService/*` — gRPC-Web endpoints for player control
/// - `/api/*` — REST endpoints for config/song/playlist/lighting CRUD
/// - Static assets from the embedded Svelte dist directory
pub async fn start(
    webui_state: WebUiState,
    address: String,
    port: u16,
) -> Result<WebUiHandle, Box<dyn std::error::Error + Send + Sync>> {
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // Start background pollers that feed the broadcast channel
    let playback_poller_handle = tokio::spawn(ws_state::playback_poller(
        webui_state.player.clone(),
        webui_state.broadcast_tx.clone(),
    ));
    let state_poller_handle = tokio::spawn(ws_state::state_poller(
        webui_state.state_rx.clone(),
        webui_state.broadcast_tx.clone(),
    ));
    let log_poller_handle = tokio::spawn(ws_state::log_poller(webui_state.broadcast_tx.clone()));
    let waveform_poller_handle = tokio::spawn(ws_state::waveform_poller(
        webui_state.player.clone(),
        webui_state.broadcast_tx.clone(),
        webui_state.waveform_cache.clone(),
    ));
    let waveform_prewarmer_handle = tokio::spawn(ws_state::waveform_prewarmer(
        webui_state.player.clone(),
        webui_state.waveform_cache.clone(),
    ));

    // Build the app router
    let api_router = super::api::router();
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .nest("/api", api_router)
        .route("/player.v1.PlayerService/{method}", any(grpc_web_handler))
        .route("/", get(index_handler))
        .fallback(get(static_handler))
        .with_state(webui_state);

    let ip: std::net::IpAddr = match address.parse() {
        Ok(ip) => ip,
        Err(e) => {
            return Err(format!("Invalid web UI address '{}': {}", address, e).into());
        }
    };
    let addr = std::net::SocketAddr::from((ip, port));
    info!("Web UI listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
            .unwrap_or_else(|e| error!("Web UI server error: {}", e));
    });

    Ok(WebUiHandle {
        _shutdown_tx: shutdown_tx,
        server_handle,
        playback_poller_handle,
        state_poller_handle,
        log_poller_handle,
        waveform_poller_handle,
        waveform_prewarmer_handle,
    })
}

/// Handles gRPC-Web requests by adapting between axum and tonic body types.
async fn grpc_web_handler(
    State(state): State<WebUiState>,
    request: axum::extract::Request,
) -> Response {
    let method = request.method().clone();
    let uri = request.uri().clone();
    let content_type = request
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("none")
        .to_string();

    debug!(
        "gRPC-Web request: {} {} (content-type: {})",
        method, uri, content_type
    );

    let grpc_svc = PlayerServiceServer::new(PlayerServer::new(state.player.clone()));
    let grpc_web = GrpcWebLayer::new().layer(grpc_svc);

    match grpc_web.oneshot(request).await {
        Ok(response) => {
            debug!("gRPC-Web response status: {}", response.status());
            let (parts, body) = response.into_parts();
            let body = axum::body::Body::new(body);
            http::Response::from_parts(parts, body)
        }
        Err(e) => {
            error!("gRPC-Web handler error: {}", e);
            Response::builder()
                .status(StatusCode::INTERNAL_SERVER_ERROR)
                .body(axum::body::Body::from(format!("{e}")))
                .unwrap()
        }
    }
}

/// Serves the embedded index.html for the SPA.
async fn index_handler() -> impl IntoResponse {
    match WebUiAssets::get("index.html") {
        Some(content) => Html(
            std::str::from_utf8(content.data.as_ref())
                .unwrap_or("<h1>Error loading web UI</h1>")
                .to_string(),
        )
        .into_response(),
        None => (StatusCode::NOT_FOUND, "Web UI assets not found").into_response(),
    }
}

/// Serves embedded static assets (CSS, JS, etc.).
async fn static_handler(uri: Uri) -> impl IntoResponse {
    let path = uri.path().trim_start_matches('/');

    // Try exact path first
    if let Some(content) = WebUiAssets::get(path) {
        let mime = mime_type_for_path(path);
        return Response::builder()
            .header(header::CONTENT_TYPE, mime)
            .body(axum::body::Body::from(content.data.to_vec()))
            .unwrap();
    }

    // For SPA routing: return index.html for non-file paths
    // (paths without extensions are assumed to be SPA routes)
    if !path.contains('.') {
        if let Some(content) = WebUiAssets::get("index.html") {
            return Response::builder()
                .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
                .body(axum::body::Body::from(content.data.to_vec()))
                .unwrap();
        }
    }

    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(axum::body::Body::from("Not found"))
        .unwrap()
}

/// Returns the MIME type for a file path based on its extension.
fn mime_type_for_path(path: &str) -> &'static str {
    match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "application/javascript; charset=utf-8",
        Some("json") => "application/json",
        Some("png") => "image/png",
        Some("svg") => "image/svg+xml",
        Some("ico") => "image/x-icon",
        Some("woff") => "font/woff",
        Some("woff2") => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Handles WebSocket upgrade requests for the dashboard.
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<WebUiState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

/// Handles a single WebSocket connection for the dashboard.
///
/// Sends fixture metadata on connect, then streams playback, fixture state,
/// and log messages from the broadcast channel.
async fn handle_ws(socket: WebSocket, state: WebUiState) {
    let (mut sender, mut receiver) = socket.split();

    // Send metadata on connect (fixture tags, types for stage layout)
    if sender
        .send(Message::Text((*state.metadata_json).clone().into()))
        .await
        .is_err()
    {
        return;
    }

    // Send cached waveform for the current song on connect
    let waveform_msg = {
        let song_name = state.player.get_playlist().current().name().to_string();
        state
            .waveform_cache
            .lock()
            .get(&song_name)
            .cloned()
            .map(|track_peaks| {
                let tracks_json: Vec<serde_json::Value> = track_peaks
                    .into_iter()
                    .map(|(name, peaks)| {
                        serde_json::json!({
                            "name": name,
                            "peaks": peaks,
                        })
                    })
                    .collect();
                serde_json::json!({
                    "type": "waveform",
                    "song_name": song_name,
                    "tracks": tracks_json,
                })
                .to_string()
            })
    };
    if let Some(msg) = waveform_msg {
        if sender.send(Message::Text(msg.into())).await.is_err() {
            return;
        }
    }

    // Subscribe to state broadcasts
    let mut rx = state.broadcast_tx.subscribe();

    // Spawn a task to forward broadcasts to this client
    let mut send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if sender.send(Message::Text(msg.into())).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Drain incoming messages (we only use server→client for now)
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Close(_) = msg {
                break;
            }
        }
    });

    tokio::select! {
        _ = &mut send_task => { recv_task.abort(); }
        _ = &mut recv_task => { send_task.abort(); }
    }
}
