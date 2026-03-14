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
#[allow_missing = true]
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
    /// Resolved path to the playlists directory (if configured).
    pub playlists_dir: Option<PathBuf>,
    /// Resolved path to the legacy playlist file (for backward compat).
    pub legacy_playlist_path: Option<PathBuf>,
    /// Shared waveform cache (sent to each WebSocket client on connect).
    pub waveform_cache: ws_state::WaveformCache,
    /// Active calibration session (at most one at a time).
    pub(crate) calibration: Arc<parking_lot::Mutex<Option<super::api::CalibrationSession>>>,
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
    config_watcher_handle: tokio::task::JoinHandle<()>,
    /// The local address the server is bound to.
    #[allow(dead_code)]
    local_addr: std::net::SocketAddr,
}

impl WebUiHandle {
    /// Returns the local address the server is listening on.
    #[allow(dead_code)]
    pub fn local_addr(&self) -> std::net::SocketAddr {
        self.local_addr
    }
}

impl Drop for WebUiHandle {
    fn drop(&mut self) {
        self.server_handle.abort();
        self.playback_poller_handle.abort();
        self.state_poller_handle.abort();
        self.log_poller_handle.abort();
        self.waveform_poller_handle.abort();
        self.waveform_prewarmer_handle.abort();
        self.config_watcher_handle.abort();
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
    let config_watcher_handle = tokio::spawn(ws_state::config_watcher(
        webui_state.player.clone(),
        webui_state.broadcast_tx.clone(),
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
    let local_addr = listener.local_addr()?;

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
        config_watcher_handle,
        local_addr,
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

    let grpc_svc = match state.player.config_store() {
        Some(store) => {
            PlayerServiceServer::new(PlayerServer::with_config_store(state.player.clone(), store))
        }
        None => PlayerServiceServer::new(PlayerServer::new(state.player.clone())),
    };
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

    // Send metadata on connect (fixture tags, types for stage layout).
    // Computed on-demand so it reflects the current DMX engine state
    // (which may have changed via hot-reload since startup).
    let metadata_json = match state.player.broadcast_handles() {
        Some(handles) => ws_state::build_metadata_json(handles.lighting_system.as_ref()),
        None => ws_state::build_metadata_json(None),
    };
    if sender
        .send(Message::Text(metadata_json.into()))
        .await
        .is_err()
    {
        return;
    }

    // Send cached waveform for the current song on connect
    let waveform_msg = {
        state
            .player
            .get_playlist()
            .current()
            .and_then(|current_song| {
                let song_name = current_song.name().to_string();
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn mime_type_html() {
        assert_eq!(mime_type_for_path("index.html"), "text/html; charset=utf-8");
    }

    #[test]
    fn mime_type_css() {
        assert_eq!(
            mime_type_for_path("styles/app.css"),
            "text/css; charset=utf-8"
        );
    }

    #[test]
    fn mime_type_js() {
        assert_eq!(
            mime_type_for_path("bundle.js"),
            "application/javascript; charset=utf-8"
        );
    }

    #[test]
    fn mime_type_json() {
        assert_eq!(mime_type_for_path("data.json"), "application/json");
    }

    #[test]
    fn mime_type_png() {
        assert_eq!(mime_type_for_path("logo.png"), "image/png");
    }

    #[test]
    fn mime_type_svg() {
        assert_eq!(mime_type_for_path("icon.svg"), "image/svg+xml");
    }

    #[test]
    fn mime_type_ico() {
        assert_eq!(mime_type_for_path("favicon.ico"), "image/x-icon");
    }

    #[test]
    fn mime_type_woff() {
        assert_eq!(mime_type_for_path("font.woff"), "font/woff");
    }

    #[test]
    fn mime_type_woff2() {
        assert_eq!(mime_type_for_path("font.woff2"), "font/woff2");
    }

    #[test]
    fn mime_type_unknown() {
        assert_eq!(mime_type_for_path("file.xyz"), "application/octet-stream");
    }

    #[test]
    fn mime_type_no_extension() {
        assert_eq!(mime_type_for_path("README"), "application/octet-stream");
    }

    #[test]
    fn web_config_default() {
        let config = WebConfig::default();
        assert_eq!(config.port, 8080);
        assert_eq!(config.address, "127.0.0.1");
    }

    use crate::player::PlayerDevices;
    use crate::playlist;
    use crate::songs::{Song, Songs};
    use std::collections::HashMap;

    fn test_webui_state() -> (WebUiState, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();

        let config_path = dir.path().join("mtrack.yaml");
        std::fs::write(&config_path, "songs: songs\n").unwrap();

        let playlist_path = dir.path().join("playlist.yaml");
        std::fs::write(&playlist_path, "songs:\n  - Song A\n").unwrap();

        let songs_path = dir.path().join("songs");
        std::fs::create_dir(&songs_path).unwrap();

        let mut map = HashMap::new();
        map.insert(
            "Song A".to_string(),
            Arc::new(Song::new_for_test("Song A", &["track1"])),
        );
        let songs = Arc::new(Songs::new(map));
        let playlist_config = crate::config::Playlist::new(&["Song A".to_string()]);
        let pl = playlist::Playlist::new("test", &playlist_config, songs.clone()).unwrap();
        let devices = PlayerDevices {
            audio: None,
            mappings: None,
            midi: None,
            dmx_engine: None,
            sample_engine: None,
            trigger_engine: None,
        };
        let mut playlists = std::collections::HashMap::new();
        playlists.insert(
            "all_songs".to_string(),
            playlist::from_songs(songs.clone()).unwrap(),
        );
        playlists.insert(pl.name().to_string(), pl);
        let player = Arc::new(
            crate::player::Player::new_with_devices(devices, playlists, "test".to_string(), None)
                .unwrap(),
        );

        let (broadcast_tx, _) = broadcast::channel(16);
        let (_state_tx, state_rx) =
            watch::channel(Arc::new(crate::state::StateSnapshot::default()));

        let state = WebUiState {
            player,
            state_rx,
            broadcast_tx,
            config_path,
            songs_path,
            playlists_dir: None,
            legacy_playlist_path: Some(playlist_path),
            waveform_cache: ws_state::new_waveform_cache(),
            calibration: Arc::new(parking_lot::Mutex::new(None)),
        };

        (state, dir)
    }

    async fn start_test_server() -> (WebUiHandle, String, tempfile::TempDir) {
        let (state, dir) = test_webui_state();
        let handle = start(state, "127.0.0.1".to_string(), 0)
            .await
            .expect("server should start");
        let base_url = format!("http://{}", handle.local_addr());
        (handle, base_url, dir)
    }

    #[tokio::test]
    async fn server_serves_index() {
        let (_handle, base_url, _dir) = start_test_server().await;

        let resp = reqwest::get(&base_url).await.unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.text().await.unwrap();
        assert!(body.contains("html"), "expected HTML response");
    }

    #[tokio::test]
    async fn server_serves_spa_route() {
        let (_handle, base_url, _dir) = start_test_server().await;

        let resp = reqwest::get(format!("{}/dashboard", base_url))
            .await
            .unwrap();
        // SPA routes should return index.html
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn server_static_not_found() {
        let (_handle, base_url, _dir) = start_test_server().await;

        let resp = reqwest::get(format!("{}/nonexistent.xyz", base_url))
            .await
            .unwrap();
        assert_eq!(resp.status(), 404);
    }

    #[tokio::test]
    async fn server_api_config() {
        let (_handle, base_url, _dir) = start_test_server().await;

        let resp = reqwest::get(format!("{}/api/config", base_url))
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let body = resp.text().await.unwrap();
        assert!(body.contains("songs:"));
    }

    #[tokio::test]
    async fn server_api_playlist() {
        let (_handle, base_url, _dir) = start_test_server().await;

        let resp = reqwest::get(format!("{}/api/playlist", base_url))
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn server_api_validate_config() {
        let (_handle, base_url, _dir) = start_test_server().await;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/api/config/validate", base_url))
            .body("songs: songs\n")
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn server_api_songs_empty() {
        let (_handle, base_url, _dir) = start_test_server().await;

        let resp = reqwest::get(format!("{}/api/songs", base_url))
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn server_api_lighting_empty() {
        let (_handle, base_url, _dir) = start_test_server().await;

        let resp = reqwest::get(format!("{}/api/lighting", base_url))
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
    }

    #[tokio::test]
    async fn server_local_addr() {
        let (handle, _, _dir) = start_test_server().await;
        let addr = handle.local_addr();
        assert_ne!(addr.port(), 0, "should have resolved to a real port");
    }

    #[tokio::test]
    async fn server_drop_shuts_down() {
        let (handle, base_url, _dir) = start_test_server().await;

        // Verify server is up
        let resp = reqwest::get(&base_url).await.unwrap();
        assert_eq!(resp.status(), 200);

        // Drop the handle
        drop(handle);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Server should be down — connection should fail
        let result = reqwest::get(&base_url).await;
        assert!(result.is_err(), "server should be shut down");
    }

    #[tokio::test]
    async fn server_serves_css_asset() {
        let css_file = WebUiAssets::iter()
            .find(|f| f.ends_with(".css"))
            .expect("no CSS asset found in embedded files");
        let (_handle, base_url, _dir) = start_test_server().await;

        let resp = reqwest::get(format!("{}/{}", base_url, css_file))
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let content_type = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            content_type.contains("css"),
            "expected CSS content type, got: {}",
            content_type
        );
    }

    #[tokio::test]
    async fn server_serves_js_asset() {
        let js_file = WebUiAssets::iter()
            .find(|f| f.ends_with(".js"))
            .expect("no JS asset found in embedded files");
        let (_handle, base_url, _dir) = start_test_server().await;

        let resp = reqwest::get(format!("{}/{}", base_url, js_file))
            .await
            .unwrap();
        assert_eq!(resp.status(), 200);
        let content_type = resp
            .headers()
            .get("content-type")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(
            content_type.contains("javascript"),
            "expected JS content type, got: {}",
            content_type
        );
    }

    #[tokio::test]
    async fn server_grpc_web_endpoint() {
        let (_handle, base_url, _dir) = start_test_server().await;

        // Send a gRPC-Web request to the PlayerService
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/player.v1.PlayerService/GetPlaylist", base_url))
            .header("content-type", "application/grpc-web+proto")
            .header("x-grpc-web", "1")
            .body(vec![0u8; 5]) // minimal gRPC frame (compressed flag + 4 byte length)
            .send()
            .await
            .unwrap();

        // Should get a response (even if the gRPC call fails, we exercise the handler)
        let status = resp.status().as_u16();
        assert!(
            status == 200 || status == 415 || status == 400,
            "unexpected status: {}",
            status
        );
    }

    #[tokio::test]
    async fn server_start_invalid_address() {
        let (state, _dir) = test_webui_state();
        let result = start(state, "not-an-ip".to_string(), 0).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn server_websocket_receives_waveform() {
        use futures_util::StreamExt;

        let (state, _dir) = test_webui_state();
        // Pre-populate waveform cache for current song "Song A"
        state.waveform_cache.lock().insert(
            "Song A".to_string(),
            vec![("track1".to_string(), vec![0.5, 0.8, 0.3])],
        );
        let handle = start(state, "127.0.0.1".to_string(), 0)
            .await
            .expect("server should start");
        let addr = handle.local_addr();
        let ws_url = format!("ws://{}/ws", addr);

        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .expect("WebSocket connect failed");

        let (_, mut read) = ws_stream.split();

        // First message: metadata
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), read.next())
            .await
            .unwrap();

        // Second message: waveform (from pre-populated cache)
        let msg = tokio::time::timeout(std::time::Duration::from_secs(2), read.next())
            .await
            .expect("timed out waiting for waveform message")
            .expect("stream ended")
            .expect("WS error");

        let text = msg.into_text().expect("expected text message");
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["type"], "waveform");
        assert_eq!(parsed["song_name"], "Song A");
        assert!(parsed["tracks"].is_array());
        let tracks = parsed["tracks"].as_array().unwrap();
        assert_eq!(tracks[0]["name"], "track1");
    }

    #[tokio::test]
    async fn server_websocket_receives_metadata() {
        use futures_util::StreamExt;

        let (handle, _, _dir) = start_test_server().await;
        let addr = handle.local_addr();
        let ws_url = format!("ws://{}/ws", addr);

        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .expect("WebSocket connect failed");

        let (_, mut read) = ws_stream.split();

        // First message should be the metadata JSON
        let msg = tokio::time::timeout(std::time::Duration::from_secs(2), read.next())
            .await
            .expect("timed out waiting for WS message")
            .expect("stream ended")
            .expect("WS error");

        let text = msg.into_text().expect("expected text message");
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(parsed["type"], "metadata");
    }

    #[tokio::test]
    async fn server_websocket_receives_playback() {
        use futures_util::StreamExt;

        let (handle, _, _dir) = start_test_server().await;
        let addr = handle.local_addr();
        let ws_url = format!("ws://{}/ws", addr);

        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .expect("WebSocket connect failed");

        let (_, mut read) = ws_stream.split();

        // Skip the metadata message
        let _ = tokio::time::timeout(std::time::Duration::from_secs(2), read.next())
            .await
            .unwrap();

        // Next messages should include a playback update (poller runs at 200ms)
        let msg = tokio::time::timeout(std::time::Duration::from_secs(3), read.next())
            .await
            .expect("timed out waiting for playback message")
            .expect("stream ended")
            .expect("WS error");

        let text = msg.into_text().expect("expected text message");
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        // Could be playback, state, or waveform — just verify it's valid JSON
        assert!(parsed["type"].is_string());
    }
}
