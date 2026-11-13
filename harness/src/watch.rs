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
//! Reads the player's hardware status over the web port's gRPC-Web, the way
//! the web UI does.
//!
//! The web port answers while hardware initialisation is still running, and
//! for a player whose gRPC port never opens (a device that never resolves),
//! so it is the one place to read the status of such a player. `Watch` opens
//! with the hardware status, so reading its first event is reading the
//! status.

use std::time::Duration;

use mtrack::proto::player::v1::watch_event::Event;
use mtrack::proto::player::v1::{HardwareStatus, WatchEvent, WatchRequest, WatchTopic};
use prost::Message;

/// How long the stream may take to say its first event.
const FIRST_EVENT: Duration = Duration::from_secs(5);

/// One gRPC-Web frame carrying `message`: a flag byte (0), the length (four
/// bytes, big endian) and the message.
fn frame(message: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8];
    out.extend_from_slice(&(message.len() as u32).to_be_bytes());
    out.extend_from_slice(message);
    out
}

/// The message of the first data frame in `buffer`, once all of it is there.
fn first_message(buffer: &[u8]) -> Option<&[u8]> {
    if buffer.len() < 5 {
        return None;
    }
    let len = u32::from_be_bytes(buffer[1..5].try_into().ok()?) as usize;
    buffer.get(5..5 + len)
}

/// The player's hardware status, as `Watch` opens with it.
pub async fn read_hardware(
    http: &reqwest::Client,
    base_url: &str,
) -> Result<HardwareStatus, String> {
    let request = WatchRequest {
        topics: vec![WatchTopic::Hardware as i32],
    }
    .encode_to_vec();
    let mut response = http
        .post(format!("{base_url}/player.v1.PlayerService/Watch"))
        .header("content-type", "application/grpc-web+proto")
        .header("x-grpc-web", "1")
        .body(frame(&request))
        .timeout(FIRST_EVENT)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("HTTP {}", response.status()));
    }

    let mut buffer = Vec::new();
    loop {
        if buffer.first().is_some_and(|flag| flag & 0x80 != 0) {
            // A trailer frame before any event: the stream failed.
            return Err(format!(
                "the stream ended without a hardware status: {}",
                String::from_utf8_lossy(&buffer[5.min(buffer.len())..])
            ));
        }
        if let Some(message) = first_message(&buffer) {
            let event = WatchEvent::decode(message)
                .map_err(|e| format!("the first event did not parse: {e}"))?;
            return match event.event {
                Some(Event::Hardware(hardware)) => Ok(hardware),
                other => Err(format!("the first event was not the hardware: {other:?}")),
            };
        }
        match response.chunk().await.map_err(|e| e.to_string())? {
            Some(chunk) => buffer.extend_from_slice(&chunk),
            None => return Err("the stream ended without a hardware status".to_string()),
        }
    }
}
