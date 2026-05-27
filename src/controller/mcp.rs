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
//! MCP (Model Context Protocol) controller.
//!
//! Exposes mtrack's player and config store to MCP-compatible clients (such as
//! Claude Desktop or Claude Code) over a streamable HTTP transport. The
//! transport is mounted at `/mcp` on a dedicated listener; we keep this
//! separate from the webui's listener so the surfaces can be enabled,
//! disabled, and bound independently.

use std::{
    collections::HashMap,
    error::Error,
    io,
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use http_body_util::BodyExt;
use parking_lot::Mutex;
use rmcp::transport::streamable_http_server::{
    session::{local::LocalSessionManager, SessionManager},
    StreamableHttpServerConfig, StreamableHttpService,
};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{info, span, warn, Level};

use crate::{config, player::Player};

mod service;

#[cfg(test)]
mod integration_test;

pub use service::McpServer;

/// Lower bound on how often the eviction sweeper runs, regardless of TTL.
const MIN_SWEEP_INTERVAL: Duration = Duration::from_secs(5);

/// A controller that exposes mtrack to MCP clients over streamable HTTP.
pub struct Driver {
    player: Arc<Player>,
    addr: SocketAddr,
    bearer_token: Option<String>,
    idle_timeout: Option<Duration>,
}

impl Driver {
    pub fn new(
        config: config::McpController,
        player: Arc<Player>,
    ) -> Result<Arc<Self>, Box<dyn Error>> {
        // Parse `bind_address` as an `IpAddr` directly rather than building a
        // `host:port` string. `SocketAddr`'s `FromStr` requires IPv6 literals
        // to be bracketed (`[::1]:port`), so the textual form would reject
        // `bind_address: "::1"` even though it is a perfectly valid IPv6
        // literal.
        let ip: IpAddr = config
            .bind_address()
            .parse()
            .map_err(|e| format!("invalid bind_address `{}`: {e}", config.bind_address()))?;
        let addr = SocketAddr::new(ip, config.port());
        Ok(Arc::new(Driver {
            player,
            addr,
            bearer_token: config.bearer_token().map(str::to_owned),
            idle_timeout: config.idle_session_timeout(),
        }))
    }
}

impl super::Driver for Driver {
    fn monitor_events(&self) -> JoinHandle<Result<(), io::Error>> {
        let addr = self.addr;
        let player = self.player.clone();
        let bearer_token = self.bearer_token.clone();
        let idle_timeout = self.idle_timeout;

        tokio::spawn(async move {
            {
                let _enter = span!(Level::INFO, "MCP Server").entered();
                let auth = if bearer_token.is_some() {
                    " (bearer-token auth)"
                } else {
                    ""
                };
                match idle_timeout {
                    Some(ttl) => info!(
                        "Starting MCP server on {addr}{auth}; idle session TTL {}s",
                        ttl.as_secs()
                    ),
                    None => info!("Starting MCP server on {addr}{auth}; idle eviction disabled"),
                }
            }

            let cancel = CancellationToken::new();
            // `CancellationToken` does not cancel on Drop — that's what
            // `DropGuard` is for. Without this, aborting `monitor_events`
            // (e.g. via `Controller::shutdown` or a controller reload) would
            // drop the token silently, leaving the sweeper task and the
            // axum graceful-shutdown future running with their captured
            // Arcs forever.
            let _cancel_guard = cancel.clone().drop_guard();
            let cancel_for_service = cancel.child_token();
            let cancel_for_sweeper = cancel.child_token();

            // Share the session manager between the HTTP service (which
            // creates/closes sessions in response to requests) and our own
            // sweeper (which closes idle sessions on a timer).
            let session_manager = Arc::new(LocalSessionManager::default());

            // Per-session last-activity timestamps, updated from middleware.
            let activity: ActivityMap = Arc::new(Mutex::new(HashMap::new()));

            let factory_player = player.clone();
            let service = StreamableHttpService::new(
                move || Ok(McpServer::new(factory_player.clone())),
                session_manager.clone(),
                StreamableHttpServerConfig::default().with_cancellation_token(cancel_for_service),
            );

            let mut app = axum::Router::new().nest_service("/mcp", service);

            // Activity-tracking middleware. We only register it when eviction
            // is enabled — otherwise every request pays for a hashmap touch
            // we'd never look at.
            if idle_timeout.is_some() {
                app = app.layer(axum::middleware::from_fn_with_state(
                    activity.clone(),
                    track_session_activity,
                ));
            }

            if let Some(token) = bearer_token {
                app = app.layer(axum::middleware::from_fn_with_state(
                    Arc::new(token),
                    require_bearer_token,
                ));
            }

            // Eviction sweeper: periodically closes sessions whose last
            // recorded activity is older than the configured TTL. Cancels
            // when the HTTP server shuts down.
            if let Some(ttl) = idle_timeout {
                let manager = session_manager.clone();
                let activity = activity.clone();
                tokio::spawn(async move {
                    let interval = sweep_interval(ttl);
                    loop {
                        tokio::select! {
                            _ = cancel_for_sweeper.cancelled() => break,
                            _ = tokio::time::sleep(interval) => {
                                sweep_idle_sessions(&manager, &activity, ttl).await;
                            }
                        }
                    }
                });
            }

            let listener = tokio::net::TcpListener::bind(addr).await?;
            axum::serve(listener, app)
                .with_graceful_shutdown(async move { cancel.cancelled().await })
                .await
                .map_err(io::Error::other)
        })
    }
}

/// Per-session last-activity map keyed by the `mcp-session-id` header value.
/// Cloned cheaply between the activity-tracking middleware and the sweeper.
type ActivityMap = Arc<Mutex<HashMap<Arc<str>, std::time::Instant>>>;

/// Bumps the activity timestamp for whichever session id appears on the
/// request or on the response. The response side matters for `initialize`,
/// which is the only path that mints a fresh session id.
///
/// The response body is also wrapped so each emitted frame refreshes the
/// session's activity. Without that wrapper, a passive SSE subscriber that
/// only listens (no further inbound POSTs) would look idle to the sweeper
/// and get evicted while server-pushed notifications were still flowing
/// over its GET stream.
async fn track_session_activity(
    axum::extract::State(activity): axum::extract::State<ActivityMap>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let now = std::time::Instant::now();
    let req_session = req
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(Arc::<str>::from);
    if let Some(ref id) = req_session {
        activity.lock().insert(id.clone(), now);
    }
    let response = next.run(req).await;
    let resp_session = response
        .headers()
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .map(Arc::<str>::from);
    if let Some(ref id) = resp_session {
        activity.lock().insert(id.clone(), now);
    }

    // Resolve which session this response belongs to. Requests carry the id
    // on the request; `initialize` mints it on the response.
    let Some(session_id) = resp_session.or(req_session) else {
        return response;
    };
    let (parts, body) = response.into_parts();
    let activity_for_frames = activity.clone();
    let mapped = body.map_frame(move |frame| {
        activity_for_frames
            .lock()
            .insert(session_id.clone(), std::time::Instant::now());
        frame
    });
    axum::response::Response::from_parts(parts, axum::body::Body::new(mapped))
}

/// Closes sessions whose last activity is older than `ttl`. Holds the mutex
/// only long enough to snapshot the list of stale ids; the actual eviction
/// calls run outside the lock so a slow `close_session` doesn't stall request
/// handling.
async fn sweep_idle_sessions(
    manager: &Arc<LocalSessionManager>,
    activity: &ActivityMap,
    ttl: Duration,
) {
    let now = std::time::Instant::now();
    let stale: Vec<Arc<str>> = {
        let map = activity.lock();
        map.iter()
            .filter(|(_, &last)| now.duration_since(last) >= ttl)
            .map(|(id, _)| id.clone())
            .collect()
    };
    if stale.is_empty() {
        return;
    }
    for id in &stale {
        if let Err(e) = manager.close_session(id).await {
            warn!(session = %id, error = ?e, "Failed to close idle MCP session");
        }
    }
    let mut map = activity.lock();
    for id in &stale {
        map.remove(id);
    }
    let ids: Vec<&str> = stale.iter().map(|id| id.as_ref()).collect();
    info!(count = stale.len(), sessions = ?ids, "Evicted idle MCP sessions");
}

/// Picks a sweep interval such that idle sessions are evicted within roughly
/// one TTL of going quiet, capped to once per minute on the high end and
/// floored at [`MIN_SWEEP_INTERVAL`] on the low end.
fn sweep_interval(ttl: Duration) -> Duration {
    let target = ttl / 4;
    target.clamp(MIN_SWEEP_INTERVAL, Duration::from_secs(60))
}

/// Axum middleware enforcing `Authorization: Bearer <token>` when configured.
/// Rejects missing, malformed, or mismatching tokens with `401 Unauthorized`.
/// Constant-time comparison defends against timing-based token recovery.
async fn require_bearer_token(
    axum::extract::State(expected): axum::extract::State<Arc<String>>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, axum::http::StatusCode> {
    // RFC 7235 §2.1: authentication scheme names are case-insensitive
    // ("bearer xyz" is just as valid as "Bearer xyz"). The token itself is
    // case-sensitive and compared in constant time.
    let header = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or(axum::http::StatusCode::UNAUTHORIZED)?;
    let (scheme, provided) = header
        .split_once(char::is_whitespace)
        .ok_or(axum::http::StatusCode::UNAUTHORIZED)?;
    let provided = provided.trim_start();
    if !scheme.eq_ignore_ascii_case("Bearer") {
        return Err(axum::http::StatusCode::UNAUTHORIZED);
    }

    if !constant_time_eq(provided.as_bytes(), expected.as_bytes()) {
        return Err(axum::http::StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(req).await)
}

/// Constant-time byte-slice equality. Always inspects every byte of the
/// shorter input regardless of mismatch so an attacker can't time-distinguish
/// "wrong prefix" from "wrong suffix".
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod sweep_interval_tests {
    use super::*;

    #[test]
    fn short_ttl_clamped_to_min() {
        assert_eq!(sweep_interval(Duration::from_secs(1)), MIN_SWEEP_INTERVAL);
    }

    #[test]
    fn long_ttl_clamped_to_one_minute() {
        assert_eq!(
            sweep_interval(Duration::from_secs(3600)),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn mid_ttl_picks_quarter() {
        assert_eq!(
            sweep_interval(Duration::from_secs(120)),
            Duration::from_secs(30)
        );
    }
}
