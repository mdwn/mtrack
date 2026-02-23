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
    response::{Html, IntoResponse},
    routing::get,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{error, info};

use rust_embed::Embed;

#[derive(Embed)]
#[folder = "src/simulator/web/"]
struct WebAssets;

/// Shared state for the axum handlers.
#[derive(Clone)]
struct AppState {
    broadcast_tx: broadcast::Sender<String>,
    metadata_json: Arc<String>,
}

/// Runs the HTTP + WebSocket server.
pub async fn run(
    broadcast_tx: broadcast::Sender<String>,
    metadata_json: String,
    port: u16,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) {
    let state = AppState {
        broadcast_tx,
        metadata_json: Arc::new(metadata_json),
    };

    let app = Router::new()
        .route("/", get(index_handler))
        .route("/ws", get(ws_handler))
        .with_state(state);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    info!("Lighting simulator listening on http://{}", addr);

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => {
            error!("Failed to bind simulator to {}: {}", addr, e);
            return;
        }
    };

    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = shutdown_rx.await;
        })
        .await
        .unwrap_or_else(|e| error!("Simulator server error: {}", e));
}

/// Serves the embedded index.html.
async fn index_handler() -> impl IntoResponse {
    match WebAssets::get("index.html") {
        Some(content) => Html(
            std::str::from_utf8(content.data.as_ref())
                .unwrap_or("<h1>Error loading simulator</h1>")
                .to_string(),
        ),
        None => Html("<h1>Simulator assets not found</h1>".to_string()),
    }
}

/// Handles WebSocket upgrade requests.
async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

/// Handles a single WebSocket connection.
async fn handle_ws(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    // Send metadata on connect
    if sender
        .send(Message::Text((*state.metadata_json).clone()))
        .await
        .is_err()
    {
        return;
    }

    // Subscribe to state broadcasts
    let mut rx = state.broadcast_tx.subscribe();

    // Spawn a task to forward broadcasts to this client
    let mut send_task = tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(msg) => {
                    if sender.send(Message::Text(msg)).await.is_err() {
                        break;
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    // Client fell behind; skip dropped frames and continue
                    continue;
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    });

    // Spawn a task to handle incoming messages (we just need to drain them)
    let mut recv_task = tokio::spawn(async move {
        while let Some(Ok(msg)) = receiver.next().await {
            if let Message::Close(_) = msg {
                break;
            }
        }
    });

    // Wait for either task to finish
    tokio::select! {
        _ = &mut send_task => {
            recv_task.abort();
        }
        _ = &mut recv_task => {
            send_task.abort();
        }
    }
}
