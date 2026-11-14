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
    extract::State,
    http::{header, StatusCode, Uri},
    response::{Html, IntoResponse, Response},
    routing::{any, get},
    Router,
};
use rust_embed::Embed;
use std::path::PathBuf;
use std::sync::Arc;
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
    /// Canonicalized path to the player config file (mtrack.yaml).
    pub config_path: PathBuf,
    /// The hub's waveform cache.
    pub waveform_cache: ws_state::WaveformCache,
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
}

/// Canonicalizes a path, falling back to the original if canonicalization fails
/// (e.g. the path does not exist yet).
fn canonicalize_or_keep(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}

/// Parameters for constructing a [`WebUiState`].
pub struct WebUiStateParams {
    pub player: Arc<Player>,
    pub config_path: PathBuf,
}

impl WebUiState {
    /// Creates a new WebUiState, canonicalizing all path fields.
    pub fn new(params: WebUiStateParams) -> Self {
        WebUiState {
            waveform_cache: params.player.watch_hub().waveform_cache().clone(),
            player: params.player,
            config_path: canonicalize_or_keep(params.config_path),
        }
    }
}

/// Handle to a running web UI server, returned so callers can shut it down.
///
/// When dropped, the server is aborted. The pollers that feed `Watch`
/// are the player's hub's, started by whoever runs the player.
pub struct WebUiHandle {
    _shutdown_tx: tokio::sync::oneshot::Sender<()>,
    server_handle: tokio::task::JoinHandle<()>,
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

/// Starts the unified web server with dashboard, gRPC-Web and file routes.
///
/// The server serves:
/// - `/` — Web UI SPA (dashboard, config editor, etc.)
/// - `/player.v1.PlayerService/*` — gRPC-Web endpoints for player control
/// - `/grpc.health.v1.Health/*` — the standard gRPC health service, over gRPC-Web
/// - `/lighting-assets/*` — the lighting asset store (models, thumbnails)
/// - `/uploads/{id}`, `/downloads/{id}` — file bytes (see `transfers`)
///
/// There is no REST API: nothing is served under `/api`, and a request for
/// it gets what any unknown path gets.
/// - Static assets from the embedded Svelte dist directory
pub async fn start(
    webui_state: WebUiState,
    address: String,
    port: u16,
) -> Result<WebUiHandle, Box<dyn std::error::Error + Send + Sync>> {
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

    // Build the app router
    let app = Router::new()
        // File bytes (the id was issued under the lock policy) and the
        // asset store's reads: not behind the lock, which the gRPC services
        // enforce for themselves.
        .merge(super::transfer_routes::router())
        .merge(super::lighting_asset_route::router())
        .route("/player.v1.PlayerService/{method}", any(grpc_web_handler))
        .route(
            "/player.v1.ConfigService/{method}",
            any(grpc_config_handler),
        )
        .route(
            "/player.v1.LightingService/{method}",
            any(grpc_lighting_handler),
        )
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

/// Handles gRPC-Web requests for `ConfigService`: the editor pages' calls,
/// behind the same lock as `PlayerService`.
async fn grpc_config_handler(
    State(state): State<WebUiState>,
    request: axum::extract::Request,
) -> Response {
    serve_grpc_web(
        crate::controller::grpc::config_service(state.player.clone()),
        request,
    )
    .await
}

/// Handles gRPC-Web requests for `LightingService`: the lighting pages'
/// calls, behind the same lock as `PlayerService`.
async fn grpc_lighting_handler(
    State(state): State<WebUiState>,
    request: axum::extract::Request,
) -> Response {
    serve_grpc_web(
        crate::controller::grpc::lighting_service(state.player.clone()),
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

    /// Nothing is served under `/api`: every route that was is answered as
    /// any unknown path is, whether that is the SPA's shell (a path without
    /// an extension) or a plain 404 (one with).
    #[tokio::test]
    async fn nothing_is_served_under_api() {
        let (_handle, base_url, _dir) = start_test_server().await;
        let client = reqwest::Client::new();

        let ask = |method: &'static str, path: String| {
            let client = client.clone();
            let url = format!("{base_url}{path}");
            async move {
                let resp = client
                    .request(method.parse().unwrap(), url)
                    .send()
                    .await
                    .unwrap();
                let status = resp.status();
                let content_type = resp.headers().get("content-type").cloned();
                (status, content_type, resp.bytes().await.unwrap())
            }
        };

        for (method, path) in [
            ("GET", "/api/songs"),
            ("PUT", "/api/lock"),
            ("GET", "/api/playlist"),
            ("GET", "/api/playlists/all_songs"),
            ("GET", "/api/config/samples"),
            ("POST", "/api/songs/Song A/tracks"),
            ("GET", "/api/lighting/show.light"),
            ("GET", "/api/lighting/venues/Hall"),
            ("GET", "/api/lighting/assets/abc/rig.json"),
            ("GET", "/api/downloads/abc"),
            ("PUT", "/api/uploads/abc"),
            ("GET", "/api/ws"),
        ] {
            let at_api = ask(method, path.to_string()).await;
            let elsewhere = ask(method, path.replacen("/api", "/nowhere", 1)).await;
            assert_eq!(
                at_api, elsewhere,
                "{method} {path} is answered differently from an unknown path"
            );
            let json = at_api
                .1
                .as_ref()
                .is_some_and(|t| t.to_str().unwrap().contains("json"));
            assert!(!json, "{method} {path} still answers JSON");
        }
    }

    /// The song list, the playlists and the lock are gRPC-Web calls on the
    /// web port: each answers, and the lock toggle answers the lock as it now
    /// stands.
    #[tokio::test]
    async fn the_song_list_playlists_and_lock_are_grpc_web_calls() {
        use crate::proto::player::v1::{
            ListPlaylistsRequest, ListPlaylistsResponse, ListSongsRequest, ListSongsResponse,
            SetLockRequest, SetLockResponse,
        };
        use prost::Message;
        let (_handle, base_url, _dir) = start_test_server().await;
        let client = reqwest::Client::new();
        let call = |method: &'static str, request: Vec<u8>| {
            let client = client.clone();
            let base_url = base_url.clone();
            async move {
                let mut frame = vec![0u8];
                frame.extend_from_slice(&(request.len() as u32).to_be_bytes());
                frame.extend_from_slice(&request);
                let resp = client
                    .post(format!("{base_url}/player.v1.PlayerService/{method}"))
                    .header("content-type", "application/grpc-web+proto")
                    .header("x-grpc-web", "1")
                    .body(frame)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(resp.status(), 200, "{method}");
                let body = resp.bytes().await.unwrap().to_vec();
                let len = u32::from_be_bytes(body[1..5].try_into().unwrap()) as usize;
                let trailers = String::from_utf8_lossy(&body[5 + len..]).to_string();
                assert!(trailers.contains("grpc-status:0"), "{method}: {trailers}");
                body[5..5 + len].to_vec()
            }
        };

        let songs = ListSongsResponse::decode(
            &call(
                "ListSongs",
                ListSongsRequest {
                    start: 0,
                    detailed: true,
                }
                .encode_to_vec(),
            )
            .await[..],
        )
        .unwrap();
        assert_eq!(songs.total as usize, songs.songs.len());
        let playlists = ListPlaylistsResponse::decode(
            &call(
                "ListPlaylists",
                ListPlaylistsRequest { detailed: true }.encode_to_vec(),
            )
            .await[..],
        )
        .unwrap();
        assert!(playlists.names.contains(&"all_songs".to_string()));
        assert_eq!(playlists.playlists.len(), playlists.names.len());
        for locked in [true, false] {
            let answer = SetLockResponse::decode(
                &call("SetLock", SetLockRequest { locked }.encode_to_vec()).await[..],
            )
            .unwrap();
            assert_eq!(answer.locked, locked);
        }
    }

    /// `ConfigService` is served by the web port's gRPC-Web beside
    /// `PlayerService`, behind the same lock: a write is refused while the
    /// player is locked, a read reaches its handler (which has no profiles
    /// directory to read).
    #[tokio::test]
    async fn config_service_is_a_grpc_web_service_behind_the_lock() {
        use prost::Message;
        use tonic_health::pb::{health_check_response::ServingStatus, HealthCheckRequest};
        let (_handle, base_url, _dir) = start_test_server().await;
        let client = reqwest::Client::new();
        let frame = |message: Vec<u8>| {
            let mut frame = vec![0u8];
            frame.extend_from_slice(&(message.len() as u32).to_be_bytes());
            frame.extend_from_slice(&message);
            frame
        };
        // Headers and body as one lowercase text: a refusal is in the
        // headers or the trailer frame.
        let call = |method: &'static str, message: Vec<u8>| {
            let client = client.clone();
            let base_url = base_url.clone();
            let body = frame(message);
            async move {
                let resp = client
                    .post(format!("{base_url}/player.v1.ConfigService/{method}"))
                    .header("content-type", "application/grpc-web+proto")
                    .header("x-grpc-web", "1")
                    .body(body)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(resp.status(), 200, "{method}");
                let headers = format!("{:?}", resp.headers()).to_lowercase();
                let body = String::from_utf8_lossy(&resp.bytes().await.unwrap()).to_lowercase();
                format!("{headers} {body}")
            }
        };

        // The message is percent-encoded in the header.
        assert!(
            call(
                "UpdateSamples",
                crate::proto::player::v1::UpdateSamplesRequest {
                    samples_json: "{}".to_string(),
                    max_sample_voices: None,
                    expected_checksum: String::new(),
                }
                .encode_to_vec()
            )
            .await
            .contains("locked"),
            "a write while locked"
        );
        // FAILED_PRECONDITION is status 9, for the lock and for no profiles
        // directory alike: the message tells them apart.
        let read = call(
            "ListProfileFiles",
            crate::proto::player::v1::ListProfileFilesRequest {}.encode_to_vec(),
        )
        .await;
        assert!(
            read.contains("no%20profiles%20directory%20configured") && !read.contains("locked"),
            "{read}"
        );

        // Served as such.
        let msg = HealthCheckRequest {
            service: "player.v1.ConfigService".to_string(),
        }
        .encode_to_vec();
        let resp = client
            .post(format!("{base_url}/grpc.health.v1.Health/Check"))
            .header("content-type", "application/grpc-web+proto")
            .header("x-grpc-web", "1")
            .body(frame(msg))
            .send()
            .await
            .unwrap();
        let body = resp.bytes().await.unwrap();
        let len = u32::from_be_bytes(body[1..5].try_into().unwrap()) as usize;
        let answer = tonic_health::pb::HealthCheckResponse::decode(&body[5..5 + len]).unwrap();
        assert_eq!(answer.status, ServingStatus::Serving as i32);
    }

    /// `LightingService` is served by the web port's gRPC-Web beside the
    /// other services, behind the same lock: a write is refused while the
    /// player is locked, a read reaches its handler.
    #[tokio::test]
    async fn lighting_service_is_a_grpc_web_service_behind_the_lock() {
        use prost::Message;
        use tonic_health::pb::{health_check_response::ServingStatus, HealthCheckRequest};
        let (_handle, base_url, _dir) = start_test_server().await;
        let client = reqwest::Client::new();
        let frame = |message: Vec<u8>| {
            let mut frame = vec![0u8];
            frame.extend_from_slice(&(message.len() as u32).to_be_bytes());
            frame.extend_from_slice(&message);
            frame
        };
        // Headers and body as one lowercase text: a refusal is in the
        // headers or the trailer frame.
        let call = |method: &'static str, message: Vec<u8>| {
            let client = client.clone();
            let base_url = base_url.clone();
            let body = frame(message);
            async move {
                let resp = client
                    .post(format!("{base_url}/player.v1.LightingService/{method}"))
                    .header("content-type", "application/grpc-web+proto")
                    .header("x-grpc-web", "1")
                    .body(body)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(resp.status(), 200, "{method}");
                let headers = format!("{:?}", resp.headers()).to_lowercase();
                let body = String::from_utf8_lossy(&resp.bytes().await.unwrap()).to_lowercase();
                format!("{headers} {body}")
            }
        };
        use crate::proto::player::v1 as pb;

        // The test server's player is locked: a write is refused before its
        // handler, and the reads and the validation are not.
        assert!(
            call(
                "SaveLightShow",
                pb::SaveLightShowRequest {
                    name: "a.light".to_string(),
                    source: String::new(),
                }
                .encode_to_vec()
            )
            .await
            .contains("locked"),
            "a write while locked"
        );
        assert!(
            call(
                "StartFixtureTest",
                pb::StartFixtureTestRequest::default().encode_to_vec()
            )
            .await
            .contains("locked"),
            "a fixture test while locked"
        );
        // The fixture types: the writes are refused, the reads are not.
        for (method, message) in [
            (
                "SaveFixtureType",
                pb::SaveFixtureTypeRequest::default().encode_to_vec(),
            ),
            (
                "DeleteFixtureType",
                pb::DeleteFixtureTypeRequest::default().encode_to_vec(),
            ),
            (
                "SaveFixtureTypeSettings",
                pb::SaveFixtureTypeSettingsRequest::default().encode_to_vec(),
            ),
        ] {
            assert!(
                call(method, message).await.contains("locked"),
                "{method} while locked"
            );
        }
        let missing = call(
            "GetFixtureType",
            pb::GetFixtureTypeRequest {
                name: "Nowhere".to_string(),
                dir: None,
            }
            .encode_to_vec(),
        )
        .await;
        // This server's player has no config store: the call reached its
        // handler, which says so, where a locked one never gets there.
        assert!(
            missing.contains("config%20store%20not%20available") && !missing.contains("locked"),
            "{missing}"
        );
        let listed = call(
            "ListFixtureTypes",
            pb::ListFixtureTypesRequest::default().encode_to_vec(),
        )
        .await;
        assert!(!listed.contains("locked"), "{listed}");
        // The venues: the writes are refused, the reads are not.
        for (method, message) in [
            ("SaveVenue", pb::SaveVenueRequest::default().encode_to_vec()),
            (
                "DeleteVenue",
                pb::DeleteVenueRequest::default().encode_to_vec(),
            ),
            (
                "AddVenueAimPoints",
                pb::AddVenueAimPointsRequest::default().encode_to_vec(),
            ),
            (
                "KeepMvrExport",
                pb::KeepMvrExportRequest::default().encode_to_vec(),
            ),
        ] {
            assert!(
                call(method, message).await.contains("locked"),
                "{method} while locked"
            );
        }
        for (method, message) in [
            (
                "ListVenues",
                pb::ListVenuesRequest::default().encode_to_vec(),
            ),
            ("GetVenue", pb::GetVenueRequest::default().encode_to_vec()),
            (
                "GetVenuePatch",
                pb::GetVenuePatchRequest::default().encode_to_vec(),
            ),
            (
                "GetVenueScene",
                pb::GetVenueSceneRequest::default().encode_to_vec(),
            ),
            (
                "GetMvrExportSummary",
                pb::GetMvrExportSummaryRequest::default().encode_to_vec(),
            ),
        ] {
            let read = call(method, message).await;
            // This server's player has no config store, or the name is
            // refused first: the call reached its handler, not the lock.
            assert!(!read.contains("locked"), "{method}: {read}");
        }
        // FAILED_PRECONDITION (9) is for the lock and for a refusal alike:
        // the message tells them apart.
        let read = call(
            "GetLightShow",
            pb::GetLightShowRequest {
                name: "nowhere.light".to_string(),
            }
            .encode_to_vec(),
        )
        .await;
        assert!(
            read.contains("lighting%20file%20not%20found") && !read.contains("locked"),
            "{read}"
        );
        let validated = call(
            "ValidateLightShow",
            pb::ValidateLightShowRequest {
                source: "invalid {{{".to_string(),
                song: None,
                file: None,
            }
            .encode_to_vec(),
        )
        .await;
        assert!(
            validated.contains("grpc-status:0") || !validated.contains("locked"),
            "{validated}"
        );

        // Served as such.
        let msg = HealthCheckRequest {
            service: "player.v1.LightingService".to_string(),
        }
        .encode_to_vec();
        let resp = client
            .post(format!("{base_url}/grpc.health.v1.Health/Check"))
            .header("content-type", "application/grpc-web+proto")
            .header("x-grpc-web", "1")
            .body(frame(msg))
            .send()
            .await
            .unwrap();
        let body = resp.bytes().await.unwrap();
        let len = u32::from_be_bytes(body[1..5].try_into().unwrap()) as usize;
        let answer = tonic_health::pb::HealthCheckResponse::decode(&body[5..5 + len]).unwrap();
        assert_eq!(answer.status, ServingStatus::Serving as i32);
    }

    /// File bytes: `BeginUpload` over gRPC-Web answers an id and a path, the
    /// bytes go to `PUT <path>` (past the lock, over the default body
    /// limit), and the claiming call takes them; a staged download is
    /// fetched by `GET`. The web server serves both.
    #[tokio::test]
    async fn file_bytes_travel_by_put_and_get_past_the_lock() {
        use crate::proto::player::v1 as pb;
        use prost::Message;
        let (state, _dir) = test_webui_state();
        let player = state.player.clone();
        assert!(player.is_locked());
        let handle = start(state, "127.0.0.1".to_string(), 0).await.unwrap();
        let base_url = format!("http://{}", handle.local_addr());
        let client = reqwest::Client::new();
        let frame = |message: Vec<u8>| {
            let mut frame = vec![0u8];
            frame.extend_from_slice(&(message.len() as u32).to_be_bytes());
            frame.extend_from_slice(&message);
            frame
        };
        let call = |service: &'static str, method: &'static str, message: Vec<u8>| {
            let client = client.clone();
            let base_url = base_url.clone();
            let body = frame(message);
            async move {
                let resp = client
                    .post(format!("{base_url}/player.v1.{service}/{method}"))
                    .header("content-type", "application/grpc-web+proto")
                    .header("x-grpc-web", "1")
                    .body(body)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(resp.status(), 200, "{method}");
                let headers = format!("{:?}", resp.headers()).to_lowercase();
                let bytes = resp.bytes().await.unwrap();
                (headers, bytes)
            }
        };
        let begin = |purpose: pb::UploadPurpose, size: u64| {
            pb::BeginUploadRequest {
                purpose: purpose.into(),
                file_name: "Kellys.mvr".to_string(),
                size: Some(size),
                song: None,
            }
            .encode_to_vec()
        };

        // Locked: only the inspection may be announced.
        let (headers, body) = call(
            "ConfigService",
            "BeginUpload",
            begin(pb::UploadPurpose::MvrImport, 6),
        )
        .await;
        assert!(
            format!("{headers} {}", String::from_utf8_lossy(&body))
                .to_lowercase()
                .contains("locked"),
            "an import announced while locked"
        );
        let (headers, body) = call(
            "ConfigService",
            "BeginUpload",
            begin(pb::UploadPurpose::MvrInspect, 6),
        )
        .await;
        assert!(!headers.contains("grpc-status: 9"), "{headers}");
        let len = u32::from_be_bytes(body[1..5].try_into().unwrap()) as usize;
        let begun = pb::BeginUploadResponse::decode(&body[5..5 + len]).unwrap();
        assert_eq!(begun.path, format!("/uploads/{}", begun.upload_id));

        // The bytes: not behind the lock.
        let unknown = client
            .put(format!(
                "{base_url}/uploads/0123456789abcdef0123456789abcdef"
            ))
            .body("abcdef")
            .send()
            .await
            .unwrap();
        assert_eq!(unknown.status(), 404);
        let too_long = client
            .put(format!("{base_url}{}", begun.path))
            .body("abcdefg")
            .send()
            .await
            .unwrap();
        assert_eq!(too_long.status(), 413);
        // Spent: the announcement is gone with the refused bytes.
        let again = client
            .put(format!("{base_url}{}", begun.path))
            .body("abcdef")
            .send()
            .await
            .unwrap();
        assert_eq!(again.status(), 404);

        let (_, body) = call(
            "ConfigService",
            "BeginUpload",
            begin(pb::UploadPurpose::MvrInspect, 6),
        )
        .await;
        let len = u32::from_be_bytes(body[1..5].try_into().unwrap()) as usize;
        let begun = pb::BeginUploadResponse::decode(&body[5..5 + len]).unwrap();
        let put = client
            .put(format!("{base_url}{}", begun.path))
            .body("abcdef")
            .send()
            .await
            .unwrap();
        assert_eq!(put.status(), 204);
        let second = client
            .put(format!("{base_url}{}", begun.path))
            .body("abcdef")
            .send()
            .await
            .unwrap();
        assert_eq!(second.status(), 409);
        // The claim reaches its handler (this server has no config store to
        // inspect against) and spends the upload.
        let (headers, body) = call(
            "LightingService",
            "InspectMvr",
            pb::InspectMvrRequest {
                upload_id: begun.upload_id.clone(),
                options: None,
            }
            .encode_to_vec(),
        )
        .await;
        let said = format!("{headers} {}", String::from_utf8_lossy(&body)).to_lowercase();
        assert!(said.contains("config%20store%20not%20available"), "{said}");
        assert_eq!(
            std::fs::read_dir(player.transfers().staging_dir().unwrap())
                .unwrap()
                .count(),
            0
        );

        // A download: any number of times, as an attachment.
        let id = player
            .transfers()
            .stage_download("kellys.mvr".to_string(), b"archive")
            .unwrap();
        for _ in 0..2 {
            let got = client
                .get(format!("{base_url}/downloads/{id}"))
                .send()
                .await
                .unwrap();
            assert_eq!(got.status(), 200);
            assert_eq!(got.headers()["content-type"], "application/octet-stream");
            assert_eq!(
                got.headers()["content-disposition"],
                "attachment; filename=\"kellys.mvr\""
            );
            assert_eq!(got.bytes().await.unwrap(), "archive");
        }
        drop(handle);
    }

    /// The asset store is served at `/lighting-assets/`, on a locked player
    /// too (a read), and not at the old place under `/api`.
    #[tokio::test]
    async fn lighting_assets_are_served_while_locked_and_not_under_api() {
        let (state, dir) = test_webui_state();
        let store = dir.path().join("lighting/.cache/assets/abc");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("rig.json"), "{\"version\":1}").unwrap();
        state.player.set_locked(true);
        let handle = start(state, "127.0.0.1".to_string(), 0).await.unwrap();
        let base_url = format!("http://{}", handle.local_addr());

        let got = reqwest::get(format!("{base_url}/lighting-assets/abc/rig.json"))
            .await
            .unwrap();
        assert_eq!(got.status(), 200);
        assert_eq!(got.headers()["content-type"], "application/json");
        assert_eq!(got.text().await.unwrap(), "{\"version\":1}");

        let old = reqwest::get(format!("{base_url}/api/lighting/assets/abc/rig.json"))
            .await
            .unwrap();
        assert_eq!(old.status(), 404);
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

    /// One frame of a gRPC-Web response body: the message of the first
    /// data frame in `buffer`, taken off it, once it is all there.
    fn take_frame(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
        if buffer.len() < 5 {
            return None;
        }
        let len = u32::from_be_bytes(buffer[1..5].try_into().unwrap()) as usize;
        if buffer.len() < 5 + len {
            return None;
        }
        assert_eq!(buffer[0], 0, "a data frame");
        let message = buffer[5..5 + len].to_vec();
        buffer.drain(..5 + len);
        Some(message)
    }

    /// `Watch` streams over gRPC-Web from the web server's own port, on a
    /// locked player, from the player's hub, and with no gRPC
    /// controller anywhere.
    #[tokio::test]
    async fn watch_streams_over_grpc_web_while_locked() {
        use crate::proto::player::v1::{watch_event::Event, WatchEvent, WatchRequest, WatchTopic};
        use prost::Message;

        let (state, _dir) = test_webui_state();
        assert!(state.player.is_locked());
        let sender = state.player.watch_hub().sender();
        let handle = start(state, "127.0.0.1".to_string(), 0)
            .await
            .expect("server should start");

        let request = WatchRequest {
            topics: vec![WatchTopic::Hardware as i32, WatchTopic::Reload as i32],
        }
        .encode_to_vec();
        let mut frame = vec![0u8];
        frame.extend_from_slice(&(request.len() as u32).to_be_bytes());
        frame.extend_from_slice(&request);
        let mut response = reqwest::Client::new()
            .post(format!(
                "http://{}/player.v1.PlayerService/Watch",
                handle.local_addr()
            ))
            .header("content-type", "application/grpc-web+proto")
            .header("x-grpc-web", "1")
            .body(frame)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);

        let mut buffer = Vec::new();
        let mut next_event = async |buffer: &mut Vec<u8>| -> Event {
            loop {
                if let Some(message) = take_frame(buffer) {
                    return WatchEvent::decode(message.as_slice())
                        .unwrap()
                        .event
                        .expect("an event");
                }
                let chunk = tokio::time::timeout(crate::testutil::PATIENCE, response.chunk())
                    .await
                    .expect("the stream said nothing")
                    .unwrap()
                    .expect("the stream ended");
                buffer.extend_from_slice(&chunk);
            }
        };

        let Event::Hardware(hardware) = next_event(&mut buffer).await else {
            panic!("the hardware status comes first");
        };
        assert!(hardware.locked);
        assert_eq!(hardware.build.unwrap().version, crate::build_info::VERSION);
        assert_eq!(
            hardware.hardware.unwrap().audio.unwrap().status,
            "not_connected"
        );

        // The hub's news arrives on the same stream.
        sender
            .send(crate::webui::wire::messages::SocketMessage::Reload(
                crate::webui::wire::messages::ShowReload::Error {
                    error: "boom".to_string(),
                },
            ))
            .expect("the stream is a reader");
        let Event::Reload(reload) = next_event(&mut buffer).await else {
            panic!("a reload");
        };
        assert_eq!(reload.error.as_deref(), Some("boom"));
    }
}
