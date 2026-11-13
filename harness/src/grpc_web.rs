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
//! Calls of the player's gRPC service over gRPC-Web, on the web port.
//!
//! The web port answers while hardware initialisation is still running, and
//! for a player whose gRPC port never opens (a device that never resolves),
//! so it is the one way to call such a player: the lock, say, which a suite
//! that edits things must release first.

use prost::Message;

/// One gRPC-Web frame carrying `message`: a flag byte (0), the length (four
/// bytes, big endian) and the message.
pub fn frame(message: &[u8]) -> Vec<u8> {
    let mut out = vec![0u8];
    out.extend_from_slice(&(message.len() as u32).to_be_bytes());
    out.extend_from_slice(message);
    out
}

/// The message of the first data frame in `buffer`, once all of it is there.
pub fn first_message(buffer: &[u8]) -> Option<&[u8]> {
    if buffer.len() < 5 {
        return None;
    }
    let len = u32::from_be_bytes(buffer[1..5].try_into().ok()?) as usize;
    buffer.get(5..5 + len)
}

/// The `grpc-status` and `grpc-message` of a gRPC-Web answer, from its
/// headers (a call that failed before it answered) or, failing those, its
/// trailer frame.
fn status_of(headers: &reqwest::header::HeaderMap, body: &[u8]) -> (Option<i32>, String) {
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
    };
    if let Some(status) = header("grpc-status") {
        return (
            status.trim().parse().ok(),
            header("grpc-message").unwrap_or_default(),
        );
    }
    // The trailer frame: `name: value` lines.
    let mut at = 0;
    while at + 5 <= body.len() {
        let len = u32::from_be_bytes(body[at + 1..at + 5].try_into().unwrap()) as usize;
        let payload = body.get(at + 5..at + 5 + len).unwrap_or_default();
        if body[at] & 0x80 != 0 {
            let text = String::from_utf8_lossy(payload);
            let find = |name: &str| {
                text.lines()
                    .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
                    .map(|v| v.trim().to_string())
            };
            return (
                find("grpc-status").and_then(|s| s.parse().ok()),
                find("grpc-message").unwrap_or_default(),
            );
        }
        at += 5 + len;
    }
    (None, String::new())
}

/// Calls the unary method `method` of the player service (`SetLock`) at
/// `base_url` with `request`, and answers its response; the status as an
/// error when the call failed.
pub async fn unary<Req: Message, Resp: Message + Default>(
    http: &reqwest::Client,
    base_url: &str,
    method: &str,
    request: &Req,
) -> Result<Resp, String> {
    let response = http
        .post(format!("{base_url}/player.v1.PlayerService/{method}"))
        .header("content-type", "application/grpc-web+proto")
        .header("x-grpc-web", "1")
        .body(frame(&request.encode_to_vec()))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !response.status().is_success() {
        return Err(format!("{method}: HTTP {}", response.status()));
    }
    let headers = response.headers().clone();
    let body = response.bytes().await.map_err(|e| e.to_string())?;
    match status_of(&headers, &body) {
        (Some(0), _) => {}
        (Some(code), message) => {
            return Err(format!("{method}: gRPC status {code}: {message}"));
        }
        (None, _) => return Err(format!("{method}: the answer carries no gRPC status")),
    }
    let message = first_message(&body).ok_or_else(|| format!("{method}: no response message"))?;
    Resp::decode(message).map_err(|e| format!("{method}: the response did not parse: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_is_read_back() {
        let framed = frame(b"abc");
        assert_eq!(framed, [0, 0, 0, 0, 3, b'a', b'b', b'c']);
        assert_eq!(first_message(&framed), Some(&b"abc"[..]));
        assert_eq!(first_message(&framed[..6]), None, "not all of it yet");
    }

    #[test]
    fn the_status_is_in_the_headers_or_the_trailer_frame() {
        let mut headers = reqwest::header::HeaderMap::new();
        headers.insert("grpc-status", "9".parse().unwrap());
        headers.insert("grpc-message", "Player is locked".parse().unwrap());
        assert_eq!(
            status_of(&headers, b""),
            (Some(9), "Player is locked".to_string())
        );

        let mut body = frame(b"x");
        let trailers = b"grpc-status: 0\r\n";
        let mut trailer_frame = vec![0x80u8];
        trailer_frame.extend_from_slice(&(trailers.len() as u32).to_be_bytes());
        trailer_frame.extend_from_slice(trailers);
        body.extend(trailer_frame);
        assert_eq!(
            status_of(&reqwest::header::HeaderMap::new(), &body),
            (Some(0), String::new())
        );
    }
}
