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
//! Both transports are needed: the web port is the only place a player whose
//! gRPC port never opened can be observed (its hardware status is read from
//! `Watch` over gRPC-Web, as the web UI reads it), while gRPC carries
//! playback control and the config mutations the persistence checks depend
//! on.

use std::time::{Duration, Instant};

use mtrack::proto::player::v1::config_service_client::ConfigServiceClient;
use mtrack::proto::player::v1::player_service_client::PlayerServiceClient;
use mtrack::proto::player::v1::{
    HardwareSnapshot, HardwareStatus, SetLockRequest, SetLockResponse, StatusRequest,
    StatusResponse, SubsystemStatus,
};
use tonic::transport::Channel;

use crate::server::Server;

/// A connected client pair for one server.
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    /// Absent for clients built with [`Client::connect_http_only`].
    grpc: Option<PlayerServiceClient<Channel>>,
    /// The editor service, on the same channel as `grpc`.
    config: Option<ConfigServiceClient<Channel>>,
}

impl Client {
    /// Connects to a running server.
    pub async fn connect(server: &Server) -> Result<Client, Box<dyn std::error::Error>> {
        // The gRPC controller can finish binding slightly after the web API
        // starts answering, so the first connect is retried rather than
        // treated as a failure.
        let deadline = Instant::now() + Duration::from_secs(15);
        let channel = loop {
            let connecting = Channel::from_shared(server.grpc_url())
                .map_err(|e| format!("bad gRPC address {}: {e}", server.grpc_url()))?
                .connect()
                .await;
            match connecting {
                Ok(channel) => break channel,
                Err(e) if Instant::now() >= deadline => {
                    return Err(
                        format!("could not reach gRPC at {}: {e}", server.grpc_url()).into(),
                    )
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(200)).await,
            }
        };
        let channel_for_player = channel.clone();

        let client = Client {
            http: reqwest::Client::new(),
            base_url: server.base_url(),
            config: Some(ConfigServiceClient::new(channel)),
            grpc: Some(PlayerServiceClient::new(channel_for_player)),
        };

        // The player boots locked so a live show cannot be altered by accident.
        // The suite exists to alter things, so it unlocks on connect.
        client.set_locked(false).await?;
        Ok(client)
    }

    /// Sets the player's lock, which gates every state-altering endpoint.
    pub async fn set_locked(&self, locked: bool) -> Result<(), Box<dyn std::error::Error>> {
        // Over the web port's gRPC-Web, which answers for a player whose
        // gRPC port is not open as well.
        let answer: SetLockResponse = crate::grpc_web::unary(
            &self.http,
            &self.base_url,
            "SetLock",
            &SetLockRequest { locked },
        )
        .await
        .map_err(|e| format!("could not set lock={locked}: {e}"))?;
        if answer.locked != locked {
            return Err(format!("set lock={locked}, but the player says {}", answer.locked).into());
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
            config: None,
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

    /// A mutable handle to the editor service's gRPC client.
    pub fn config(&mut self) -> &mut ConfigServiceClient<Channel> {
        self.config
            .as_mut()
            .expect("this client was built without gRPC; use the HTTP API instead")
    }

    /// Calls a method of the editor service over the web port's gRPC-Web,
    /// which answers for a player whose gRPC port is not open as well (a
    /// client built with [`Client::connect_http_only`] has no other way).
    pub async fn config_call<Req: prost::Message, Resp: prost::Message + Default>(
        &self,
        method: &str,
        request: &Req,
    ) -> Result<Resp, crate::grpc_web::CallError> {
        crate::grpc_web::call(&self.http, &self.base_url, "ConfigService", method, request).await
    }

    /// Calls a method of the lighting service over the web port's
    /// gRPC-Web, as [`Client::config_call`] calls the editor service's.
    pub async fn lighting_call<Req: prost::Message, Resp: prost::Message + Default>(
        &self,
        method: &str,
        request: &Req,
    ) -> Result<Resp, crate::grpc_web::CallError> {
        crate::grpc_web::call(
            &self.http,
            &self.base_url,
            "LightingService",
            method,
            request,
        )
        .await
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

    /// POSTs JSON and returns the status alongside the parsed body.
    ///
    /// Separate from a plain request because axum's `Json` extractor rejects
    /// anything not sent as `application/json`, and because the status is part
    /// of what these endpoints are saying.
    pub async fn post_json(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<(reqwest::StatusCode, serde_json::Value), Box<dyn std::error::Error>> {
        let url = format!("{}/api/{}", self.base_url, path.trim_start_matches('/'));
        let response = self.http.post(&url).json(&body).send().await?;
        let status = response.status();
        let text = response.text().await?;
        let parsed = serde_json::from_str(&text).map_err(|e| {
            format!("POST {url} returned {status} with a non-JSON body: {e}: {text}")
        })?;
        Ok((status, parsed))
    }

    /// PUTs a JSON body and returns the status and parsed response.
    ///
    /// Separate from [`Self::post_json`] because several config endpoints are
    /// PUT-only, and they reject anything that is not `application/json`.
    pub async fn put_json(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<(reqwest::StatusCode, serde_json::Value), Box<dyn std::error::Error>> {
        let url = format!("{}/api/{}", self.base_url, path.trim_start_matches('/'));
        let response = self.http.put(&url).json(&body).send().await?;
        let status = response.status();
        let text = response.text().await?;
        let parsed = serde_json::from_str(&text).map_err(|e| {
            format!("PUT {url} returned {status} with a non-JSON body: {e}: {text}")
        })?;
        Ok((status, parsed))
    }

    /// The build, hardware and controllers as the player reports them: the
    /// hardware event `Watch` opens with.
    pub async fn hardware_status(&self) -> Result<HardwareStatus, Box<dyn std::error::Error>> {
        Ok(crate::watch::read_hardware(&self.http, &self.base_url).await?)
    }

    /// The hardware snapshot of [`Self::hardware_status`].
    pub async fn hardware(&self) -> Result<HardwareSnapshot, Box<dyn std::error::Error>> {
        Ok(self.hardware_status().await?.hardware.unwrap_or_default())
    }

    /// The reported status of a hardware subsystem: `connected`,
    /// `initializing`, or `not_connected`.
    pub async fn subsystem_status(
        &self,
        subsystem: &str,
    ) -> Result<String, Box<dyn std::error::Error>> {
        let hardware = self.hardware().await?;
        Ok(subsystem_of(&hardware, subsystem)
            .map(|s| s.status.clone())
            .unwrap_or_else(|| "missing".to_string()))
    }

    /// The device name the player reports for a subsystem. This is what proves
    /// a given profile actually took effect, since the status's `profile`
    /// field carries the profile's hostname (or `"default"`), not a name.
    pub async fn subsystem_device(
        &self,
        subsystem: &str,
    ) -> Result<Option<String>, Box<dyn std::error::Error>> {
        let hardware = self.hardware().await?;
        Ok(subsystem_of(&hardware, subsystem).and_then(|s| s.name.clone()))
    }

    /// The name of the active hardware profile.
    pub async fn active_profile(&self) -> Result<Option<String>, Box<dyn std::error::Error>> {
        Ok(self.hardware().await?.profile)
    }

    /// Waits until the player reports it is playing.
    pub async fn wait_until_playing(
        &mut self,
        timeout: Duration,
    ) -> Result<StatusResponse, crate::outcome::CheckError> {
        self.wait_for_status(timeout, "playing", false, |s| s.playing)
            .await
    }

    /// Waits until the player reports it has stopped, treating a timeout as
    /// *this check's* assertion.
    ///
    /// For checks whose whole point is that something stops. Separate from
    /// [`Self::wait_until_stopped`] so the self-test can tell a control that
    /// drove the assertion from one that merely failed during setup.
    pub async fn wait_until_stopped_asserting(
        &mut self,
        timeout: Duration,
    ) -> Result<StatusResponse, crate::outcome::CheckError> {
        self.wait_for_status(timeout, "stopped", true, |s| !s.playing)
            .await
    }

    /// Waits until the player reports it has stopped.
    pub async fn wait_until_stopped(
        &mut self,
        timeout: Duration,
    ) -> Result<StatusResponse, crate::outcome::CheckError> {
        self.wait_for_status(timeout, "stopped", false, |s| !s.playing)
            .await
    }

    async fn wait_for_status(
        &mut self,
        timeout: Duration,
        what: &str,
        assertion: bool,
        predicate: impl Fn(&StatusResponse) -> bool,
    ) -> Result<StatusResponse, crate::outcome::CheckError> {
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
        // A real defect in a normal run -- a player that accepts Play and never
        // plays is broken -- but by default *not* this check's assertion: most
        // call sites wait as setup before asserting something else. Call sites
        // where the wait is itself the assertion use the `_asserting` variants,
        // so the flag is chosen where the meaning is known rather than here.
        let message =
            format!("player never reported {what} within {timeout:?} (last status: {last:?})");
        Err(if assertion {
            crate::outcome::CheckError::assertion(message)
        } else {
            crate::outcome::CheckError::before_assertion(message)
        })
    }
}

/// One of the hardware snapshot's subsystems, by its name in the status.
pub fn subsystem_of<'a>(
    hardware: &'a HardwareSnapshot,
    subsystem: &str,
) -> Option<&'a SubsystemStatus> {
    match subsystem {
        "audio" => hardware.audio.as_ref(),
        "midi" => hardware.midi.as_ref(),
        "dmx" => hardware.dmx.as_ref(),
        "trigger" => hardware.trigger.as_ref(),
        _ => None,
    }
}

/// Converts a protobuf duration to a `Duration`, treating absence as zero.
pub fn elapsed_of(status: &StatusResponse) -> Duration {
    status
        .elapsed
        .and_then(|d| Duration::try_from(d).ok())
        .unwrap_or_default()
}
