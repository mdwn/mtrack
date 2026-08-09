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
//! Talks to a running mtrack over the interfaces a real controller would use.
//!
//! Both transports are needed: the web API is the only place hardware and
//! controller status is exposed, while gRPC carries playback control and the
//! config mutations the persistence checks depend on.

use std::time::{Duration, Instant};

use mtrack::proto::player::v1::player_service_client::PlayerServiceClient;
use mtrack::proto::player::v1::{StatusRequest, StatusResponse};
use tonic::transport::Channel;

use crate::server::Server;

/// A connected client pair for one server.
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    /// Absent for clients built with [`Client::connect_http_only`].
    grpc: Option<PlayerServiceClient<Channel>>,
}

impl Client {
    /// Connects to a running server.
    pub async fn connect(server: &Server) -> Result<Client, Box<dyn std::error::Error>> {
        // The gRPC controller can finish binding slightly after the web API
        // starts answering, so the first connect is retried rather than
        // treated as a failure.
        let deadline = Instant::now() + Duration::from_secs(15);
        let grpc = loop {
            match PlayerServiceClient::connect(server.grpc_url()).await {
                Ok(client) => break client,
                Err(e) if Instant::now() >= deadline => {
                    return Err(
                        format!("could not reach gRPC at {}: {e}", server.grpc_url()).into(),
                    )
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        };

        let client = Client {
            http: reqwest::Client::new(),
            base_url: server.base_url(),
            grpc: Some(grpc),
        };

        // The player boots locked so a live show cannot be altered by accident.
        // The suite exists to alter things, so it unlocks on connect.
        client.set_locked(false).await?;
        Ok(client)
    }

    /// Sets the player's lock, which gates every state-altering endpoint.
    pub async fn set_locked(&self, locked: bool) -> Result<(), Box<dyn std::error::Error>> {
        let url = format!("{}/api/lock", self.base_url);
        let response = self
            .http
            .put(&url)
            .json(&serde_json::json!({ "locked": locked }))
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(format!("could not set lock={locked}: HTTP {}", response.status()).into());
        }
        Ok(())
    }

    /// Connects over HTTP only.
    ///
    /// Controllers are started as part of hardware initialization, so a player
    /// waiting on a device that never resolves never opens its gRPC port. The
    /// web API still answers, and is the only way to observe such a player.
    pub async fn connect_http_only(server: &Server) -> Result<Client, Box<dyn std::error::Error>> {
        let client = Client {
            http: reqwest::Client::new(),
            base_url: server.base_url(),
            grpc: None,
        };
        client.set_locked(false).await?;
        Ok(client)
    }

    /// A mutable handle to the gRPC client for calls without a wrapper here.
    pub fn grpc(&mut self) -> &mut PlayerServiceClient<Channel> {
        self.grpc
            .as_mut()
            .expect("this client was built without gRPC; use the HTTP API instead")
    }

    /// Fetches the player status over gRPC.
    pub async fn status(&mut self) -> Result<StatusResponse, Box<dyn std::error::Error>> {
        Ok(self.grpc().status(StatusRequest {}).await?.into_inner())
    }

    /// Fetches `GET /api/<path>` as JSON.
    pub async fn get_json(
        &self,
        path: &str,
    ) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
        let url = format!("{}/api/{}", self.base_url, path.trim_start_matches('/'));
        let response = self.http.get(&url).send().await?;
        let status = response.status();
        let body = response.text().await?;
        if !status.is_success() {
            return Err(format!("GET {url} returned {status}: {body}").into());
        }
        Ok(serde_json::from_str(&body)?)
    }

    /// Sends a request with a raw body and returns the status and body text.
    ///
    /// The lighting endpoints take and return DSL text rather than JSON, and
    /// several respond with a meaningful non-2xx, so the status is handed back
    /// rather than treated as an error.
    pub async fn send_text(
        &self,
        method: reqwest::Method,
        path: &str,
        body: String,
    ) -> Result<(reqwest::StatusCode, String), Box<dyn std::error::Error>> {
        let url = format!("{}/api/{}", self.base_url, path.trim_start_matches('/'));
        let response = self
            .http
            .request(method, &url)
            .header("content-type", "text/plain")
            .body(body)
            .send()
            .await?;
        let status = response.status();
        Ok((status, response.text().await?))
    }

    /// Fetches a path as raw text along with its status.
    pub async fn get_text(
        &self,
        path: &str,
    ) -> Result<(reqwest::StatusCode, String), Box<dyn std::error::Error>> {
        let url = format!("{}/api/{}", self.base_url, path.trim_start_matches('/'));
        let response = self.http.get(&url).send().await?;
        let status = response.status();
        Ok((status, response.text().await?))
    }

    /// The reported status of a hardware subsystem: `connected`,
    /// `initializing`, or `not_connected`.
    pub async fn subsystem_status(
        &self,
        subsystem: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let status = self.get_json("status").await?;
        Ok(status
            .get("hardware")
            .and_then(|h| h.get(subsystem))
            .and_then(|s| s.get("status"))
            .and_then(|s| s.as_str())
            .unwrap_or("missing")
            .to_string())
    }

    /// The device name the player reports for a subsystem. This is what proves
    /// a given profile actually took effect, since the status API's `profile`
    /// field carries the profile's hostname (or `"default"`), not a name.
    pub async fn subsystem_device(
        &self,
        subsystem: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let status = self.get_json("status").await?;
        Ok(status
            .get("hardware")
            .and_then(|h| h.get(subsystem))
            .and_then(|s| s.get("name"))
            .and_then(|n| n.as_str())
            .map(|s| s.to_string()))
    }

    /// The name of the active hardware profile.
    pub async fn active_profile(&self) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let status = self.get_json("status").await?;
        Ok(status
            .get("hardware")
            .and_then(|h| h.get("profile"))
            .and_then(|p| p.as_str())
            .map(|s| s.to_string()))
    }

    /// Waits until the player reports it is playing.
    pub async fn wait_until_playing(
        &mut self,
        timeout: Duration,
    ) -> Result<StatusResponse, Box<dyn std::error::Error>> {
        self.wait_for_status(timeout, "playing", |s| s.playing)
            .await
    }

    /// Waits until the player reports it has stopped.
    pub async fn wait_until_stopped(
        &mut self,
        timeout: Duration,
    ) -> Result<StatusResponse, Box<dyn std::error::Error>> {
        self.wait_for_status(timeout, "stopped", |s| !s.playing)
            .await
    }

    async fn wait_for_status(
        &mut self,
        timeout: Duration,
        what: &str,
        predicate: impl Fn(&StatusResponse) -> bool,
    ) -> Result<StatusResponse, Box<dyn std::error::Error>> {
        let deadline = Instant::now() + timeout;
        let mut last = None;
        while Instant::now() < deadline {
            let status = self.status().await?;
            if predicate(&status) {
                return Ok(status);
            }
            last = Some(status);
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err(
            format!("player never reported {what} within {timeout:?} (last status: {last:?})")
                .into(),
        )
    }
}

/// Converts a protobuf duration to a `Duration`, treating absence as zero.
pub fn elapsed_of(status: &StatusResponse) -> Duration {
    status
        .elapsed
        .and_then(|d| Duration::try_from(d).ok())
        .unwrap_or_default()
}
