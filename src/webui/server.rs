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
use tokio::sync::broadcast;
use tonic_web::GrpcWebLayer;
use tower::{Layer, ServiceExt};
use tracing::{debug, error, info};

use crate::player::Player;

use super::state as ws_state;

#[derive(Embed)]
#[folder = "src/webui/svelte/dist/"]
#[allow_missing = true]
struct WebUiAssets;

/// The identity of the web UI this server serves — the hash of its frontend
/// inputs, which the UI build writes to `build-info.json` and also compiles
/// into the bundle. An open tab compares the two to know it is running an
/// older UI than the server now serves. `None` when the UI was built without
/// one (a plain `npm run build`) or there is no UI.
pub(crate) fn ui_build() -> Option<String> {
    let info = WebUiAssets::get("build-info.json")?;
    let parsed: serde_json::Value = serde_json::from_slice(&info.data).ok()?;
    parsed["ui_build"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

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
///
/// All path fields are canonicalized at construction time so that path
/// comparisons (e.g. `strip_prefix`) work consistently regardless of how
/// the paths were originally specified in the config file.
#[derive(Clone)]
pub struct WebUiState {
    pub player: Arc<Player>,
    pub broadcast_tx: broadcast::Sender<String>,
    /// Canonicalized path to the player config file (mtrack.yaml).
    pub config_path: PathBuf,
    /// Shared waveform cache (sent to each WebSocket client on connect).
    pub waveform_cache: ws_state::WaveformCache,
    /// Canonicalized path to the profiles directory (if configured).
    pub profiles_dir: Option<PathBuf>,
    /// Active calibration session (at most one at a time).
    pub(crate) calibration:
        Arc<parking_lot::Mutex<Option<super::api::devices::CalibrationSession>>>,
}

impl WebUiState {
    /// The player's song sources as last resolved: read on each call, so a
    /// runtime change to `songs:` or `playlists_dir:` reaches the web UI's
    /// file access as it reaches every surface's reload.
    fn song_sources(&self) -> crate::player::SongSources {
        self.player.song_sources().unwrap_or_default()
    }

    /// Canonicalized path to the songs directory, from the player's song
    /// sources, so the files the web UI edits are the ones it reloads.
    /// codeql[rust/path-injection] Resolved from the player config file; not user-controlled.
    pub fn songs_path(&self) -> PathBuf {
        canonicalize_or_keep(self.song_sources().songs)
    }

    /// Canonicalized path to the playlists directory, from the player's
    /// song sources.
    pub fn playlists_dir(&self) -> Option<PathBuf> {
        self.song_sources().playlists_dir.map(canonicalize_or_keep)
    }

    /// Canonicalized path to the legacy playlist file, if one is
    /// configured, from the player's song sources.
    pub fn legacy_playlist_path(&self) -> Option<PathBuf> {
        self.song_sources()
            .legacy_playlist
            .map(canonicalize_or_keep)
    }
}

/// Canonicalizes a path, falling back to the original if canonicalization fails
/// (e.g. the path does not exist yet).
fn canonicalize_or_keep(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

/// Parameters for constructing a [`WebUiState`].
pub struct WebUiStateParams {
    pub player: Arc<Player>,
    pub broadcast_tx: broadcast::Sender<String>,
    pub config_path: PathBuf,
    pub profiles_dir: Option<PathBuf>,
    pub waveform_cache: ws_state::WaveformCache,
}

impl WebUiState {
    /// Creates a new WebUiState, canonicalizing all path fields.
    pub fn new(params: WebUiStateParams) -> Self {
        WebUiState {
            player: params.player,
            broadcast_tx: params.broadcast_tx,
            config_path: canonicalize_or_keep(params.config_path),
            profiles_dir: params.profiles_dir.map(canonicalize_or_keep),
            waveform_cache: params.waveform_cache,
            calibration: Arc::new(parking_lot::Mutex::new(None)),
        }
    }
}

/// Handle to a running web UI server, returned so callers can shut it down.
///
/// When dropped, the server and background tasks are aborted.
pub struct WebUiHandle {
    _shutdown_tx: tokio::sync::oneshot::Sender<()>,
    server_handle: tokio::task::JoinHandle<()>,
    playback_poller_handle: tokio::task::JoinHandle<()>,
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
        self.log_poller_handle.abort();
        self.waveform_poller_handle.abort();
        self.waveform_prewarmer_handle.abort();
        self.config_watcher_handle.abort();
    }
}

/// Starts the web UI, or, when it cannot start, unlocks the player. The
/// web UI is how the lock is normally released (gRPC's and Bluetooth's
/// `SetLock` aside), so a player left locked without one would refuse every
/// gRPC and MCP config write for the whole run.
pub async fn start_or_unlock(
    webui_state: WebUiState,
    address: String,
    port: u16,
) -> Option<WebUiHandle> {
    let player = webui_state.player.clone();
    match start(webui_state, address, port).await {
        Ok(handle) => Some(handle),
        Err(e) => {
            tracing::warn!("Failed to start web UI: {}", e);
            player.set_locked(false);
            info!(
                "No web UI is running, so the player starts unlocked: there may be \
                 nothing else to unlock it with"
            );
            None
        }
    }
}

/// Starts the unified web server with dashboard, gRPC-Web, and REST API.
///
/// The server serves:
/// - `/` — Web UI SPA (dashboard, config editor, etc.)
/// - `/ws` — WebSocket for real-time state streaming (playback, fixtures, logs)
/// - `/player.v1.PlayerService/*` — gRPC-Web endpoints for player control
/// - `/grpc.health.v1.Health/*` — the standard gRPC health service, over gRPC-Web
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
    let api_router = super::api::guarded_router(&webui_state);
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .nest("/api", api_router)
        .route("/player.v1.PlayerService/{method}", any(grpc_web_handler))
        .route("/grpc.health.v1.Health/{method}", any(grpc_health_handler))
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

    // The same service, behind the same lock, as native gRPC.
    serve_grpc_web(
        crate::controller::grpc::player_service(state.player.clone()),
        request,
    )
    .await
}

/// Answers the standard `grpc.health.v1.Health` service over gRPC-Web. It is
/// not behind the lock: a health check must work on a locked player.
async fn grpc_health_handler(request: axum::extract::Request) -> Response {
    serve_grpc_web(crate::controller::grpc::health_service().await, request).await
}

/// Serves one gRPC-Web request with `service`, adapting between axum and
/// tonic body types.
async fn serve_grpc_web<S>(service: S, request: axum::extract::Request) -> Response
where
    S: tower::Service<
            http::Request<tonic::body::Body>,
            Response = http::Response<tonic::body::Body>,
            Error = std::convert::Infallible,
        > + Clone
        + Send
        + 'static,
    S::Future: Send + 'static,
{
    let grpc_web = GrpcWebLayer::new().layer(service);

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
        // The shell names the content-hashed bundle, so it is never reused
        // unchecked: a reload after an upgrade must fetch the new one.
        Some(content) => (
            [(header::CACHE_CONTROL, "no-cache")],
            Html(
                std::str::from_utf8(content.data.as_ref())
                    .unwrap_or("<h1>Error loading web UI</h1>")
                    .to_string(),
            ),
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
                .header(header::CACHE_CONTROL, "no-cache")
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
        Some("wasm") => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// Handles WebSocket upgrade requests for the dashboard.
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<WebUiState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

/// Handles a single WebSocket connection for the dashboard.
///
/// Sends the cached waveform on connect, then streams playback, log,
/// waveform and reload messages from the broadcast channel. The stage is not
/// here: it is the gRPC stream `WatchStage`.
async fn handle_ws(socket: WebSocket, state: WebUiState) {
    let (mut sender, mut receiver) = socket.split();

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

    fn test_webui_state() -> (WebUiState, tempfile::TempDir) {
        super::super::api::test_helpers::test_state_on_disk(
            crate::testutil::player::memory_player(&["Song A"]).playlists(),
            "test",
            None,
            None,
            "songs:\n  - Song A\n",
            false,
        )
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
        // A reload after an upgrade must not reuse the old shell.
        assert_eq!(resp.headers()["cache-control"], "no-cache");
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
        assert_eq!(resp.headers()["cache-control"], "no-cache");
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
    async fn server_api_playlist() {
        let (_handle, base_url, _dir) = start_test_server().await;

        let resp = reqwest::get(format!("{}/api/playlist", base_url))
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

    /// The standard health service answers over gRPC-Web, on a locked
    /// player, for the server as a whole.
    #[tokio::test]
    async fn the_health_service_answers_over_grpc_web_while_locked() {
        use prost::Message;
        use tonic_health::pb::{health_check_response::ServingStatus, HealthCheckRequest};
        let (_handle, base_url, _dir) = start_test_server().await;

        for (service, expected) in [
            ("", ServingStatus::Serving),
            ("player.v1.PlayerService", ServingStatus::Serving),
        ] {
            let msg = HealthCheckRequest {
                service: service.to_string(),
            }
            .encode_to_vec();
            let mut frame = vec![0u8];
            frame.extend_from_slice(&(msg.len() as u32).to_be_bytes());
            frame.extend_from_slice(&msg);

            let resp = reqwest::Client::new()
                .post(format!("{base_url}/grpc.health.v1.Health/Check"))
                .header("content-type", "application/grpc-web+proto")
                .header("x-grpc-web", "1")
                .body(frame)
                .send()
                .await
                .unwrap();
            assert_eq!(resp.status(), 200);
            let body = resp.bytes().await.unwrap();
            // The first frame is the message: flag 0, a 4 byte length, then
            // the response. A trailers frame (flag 0x80) follows it.
            assert_eq!(body[0], 0, "{service:?}");
            let len = u32::from_be_bytes(body[1..5].try_into().unwrap()) as usize;
            let answer = tonic_health::pb::HealthCheckResponse::decode(&body[5..5 + len]).unwrap();
            assert_eq!(answer.status, expected as i32, "{service:?}");
            let trailers = String::from_utf8_lossy(&body[5 + len..]).to_lowercase();
            assert!(trailers.contains("grpc-status:0"), "{trailers}");
        }
    }

    /// Only the web UI can release the lock, so a player whose web UI
    /// cannot start is left unlocked rather than refusing every write.
    #[tokio::test]
    async fn a_web_ui_that_cannot_start_leaves_the_player_unlocked() {
        let (state, _dir) = test_webui_state();
        let player = state.player.clone();
        assert!(player.is_locked());
        assert!(start_or_unlock(state, "not-an-ip".to_string(), 0)
            .await
            .is_none());
        assert!(!player.is_locked());

        let (state, _dir) = test_webui_state();
        let player = state.player.clone();
        let _handle = start_or_unlock(state, "127.0.0.1".to_string(), 0)
            .await
            .expect("server should start");
        assert!(player.is_locked(), "a running web UI keeps the lock");
    }

    /// A config write over the web UI's gRPC-Web route is refused while
    /// locked, the same as over native gRPC.
    #[tokio::test]
    async fn a_grpc_web_config_write_is_refused_while_locked() {
        use prost::Message;
        let (_handle, base_url, _dir) = start_test_server().await;
        let msg = crate::proto::player::v1::UpdateAudioRequest {
            audio_json: String::new(),
            expected_checksum: String::new(),
        }
        .encode_to_vec();
        let mut frame = vec![0u8];
        frame.extend_from_slice(&(msg.len() as u32).to_be_bytes());
        frame.extend_from_slice(&msg);

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{base_url}/player.v1.PlayerService/UpdateAudio"))
            .header("content-type", "application/grpc-web+proto")
            .header("x-grpc-web", "1")
            .body(frame)
            .send()
            .await
            .unwrap();
        let headers = format!("{:?}", resp.headers()).to_lowercase();
        let body = String::from_utf8_lossy(&resp.bytes().await.unwrap()).to_lowercase();
        let all = format!("{headers} {body}");
        // FAILED_PRECONDITION is status 9, in the headers or the trailer frame.
        assert!(
            all.contains("\"grpc-status\": \"9\"") || all.contains("grpc-status:9"),
            "{all}"
        );
        assert!(all.contains("locked"), "{all}");
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

        // First message: waveform (from pre-populated cache)
        let msg = tokio::time::timeout(crate::testutil::PATIENCE, read.next())
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
    async fn server_websocket_receives_playback() {
        use futures_util::StreamExt;

        let (handle, _, _dir) = start_test_server().await;
        let addr = handle.local_addr();
        let ws_url = format!("ws://{}/ws", addr);

        let (ws_stream, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .expect("WebSocket connect failed");

        let (_, mut read) = ws_stream.split();

        // The messages should include a playback update (poller runs at 200ms)
        let msg = tokio::time::timeout(crate::testutil::PATIENCE, read.next())
            .await
            .expect("timed out waiting for playback message")
            .expect("stream ended")
            .expect("WS error");

        let text = msg.into_text().expect("expected text message");
        let parsed: serde_json::Value = serde_json::from_str(&text).unwrap();
        // Could be playback or waveform — just verify it's valid JSON
        assert!(parsed["type"].is_string());
    }
}
