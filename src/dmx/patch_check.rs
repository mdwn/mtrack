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

//! Tells the operator when olad will eat everything mtrack streams.
//!
//! olad drops frames for a universe that has no output port patched to it, and
//! says nothing: the streaming client connects, every write succeeds, and the
//! rig stays dark. Nothing on the DMX path can detect this — the streaming
//! protocol carries no reply — so we ask olad's web server instead, off the
//! output path entirely, and only to produce a warning.

use std::{
    io::{ErrorKind, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    thread,
    time::{Duration, Instant},
};

use tracing::{debug, warn};

/// olad's web server is local to the daemon; the streaming port may one day be
/// remote, this never is.
const OLAD_HOST: &str = "127.0.0.1";

/// Long enough for a loopback connect, short enough that an olad wedged on
/// startup cannot hold the probe thread.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(1);

/// The whole exchange is bounded by wall clock: per-read timeouts alone let a
/// server that dribbles a byte at a time keep the socket open forever.
const PROBE_DEADLINE: Duration = Duration::from_secs(2);

/// olad's answer is a few hundred bytes. Anything past this is not olad, and we
/// would rather stop reading than grow a buffer for whatever is.
const MAX_RESPONSE_BYTES: usize = 64 * 1024;

/// Why a universe's patch state could not be read.
///
/// The two cases are worth telling apart: an unreachable web server means every
/// remaining universe would fail the same way, while a single unreadable answer
/// says nothing about the next universe.
#[derive(Debug)]
enum ProbeError {
    /// olad's web server never answered — likely not running, or not listening
    /// where we looked.
    Unreachable(String),
    /// olad answered, but not in a shape we could draw a conclusion from.
    Unreadable(String),
}

/// Warns about every configured universe olad would silently drop frames for.
///
/// Runs detached: the probe must never delay engine startup or DMX output.
///
/// The thread is deliberately never joined — it holds no reference to the
/// engine, ends on its own within seconds, and the only thing a shutdown could
/// gain by waiting for it is a warning nobody is left to read.
pub fn warn_unpatched_universes(http_port: u16, universes: Vec<u16>) {
    // The same universe can appear under several devices; ask about each one
    // once, and in a settled order.
    let mut universes = universes;
    universes.sort_unstable();
    universes.dedup();

    if universes.is_empty() {
        return;
    }

    let spawned = thread::Builder::new()
        .name("olad-patch-check".to_string())
        .spawn(move || {
            for universe in universes {
                match probe_universe(OLAD_HOST, http_port, universe) {
                    Ok(true) => warn!(
                        universe,
                        "Universe {universe} is configured under dmx.universes but has no output \
                         port patched in olad — olad will silently drop every frame mtrack \
                         streams there. Patch one (ola_patch -d <device> -p <port> -u {universe}, \
                         or olad's web UI on :{http_port})."
                    ),
                    Ok(false) => {}
                    // Nothing answered, so nothing will: stop, and say so once
                    // rather than once per universe. olad's web server is
                    // optional and this check is advisory, so debug is loud
                    // enough.
                    Err(ProbeError::Unreachable(e)) => {
                        debug!(
                            "could not reach olad's web server on {OLAD_HOST}:{http_port} to \
                             verify patch state: {e}"
                        );
                        return;
                    }
                    // This universe alone is unreadable. The next one may not
                    // be, and it may be the one the operator is about to run.
                    Err(ProbeError::Unreadable(e)) => debug!(
                        universe,
                        "could not verify olad patch state for universe {universe}: {e}"
                    ),
                }
            }
        });

    if let Err(e) = spawned {
        debug!("could not verify olad patch state: {e}");
    }
}

/// Asks olad whether a universe has an output port patched to it.
///
/// A hand-rolled HTTP/1.0 GET: the main crate carries no runtime HTTP client,
/// and one request against the loopback does not justify adding one. HTTP/1.0
/// with `Connection: close` means olad closes the socket at the end of the
/// body, so reading to EOF is the whole response without parsing a length.
fn probe_universe(host: &str, http_port: u16, universe: u16) -> Result<bool, ProbeError> {
    let deadline = Instant::now() + PROBE_DEADLINE;

    let address = (host, http_port)
        .to_socket_addrs()
        .map_err(|e| ProbeError::Unreachable(format!("{host}:{http_port}: {e}")))?
        .next()
        .ok_or_else(|| {
            ProbeError::Unreachable(format!("{host}:{http_port} resolved to no address"))
        })?;

    let mut stream = TcpStream::connect_timeout(&address, CONNECT_TIMEOUT)
        .map_err(|e| ProbeError::Unreachable(format!("connect: {e}")))?;

    let request = format!(
        "GET /json/universe_info?id={universe} HTTP/1.0\r\n\
         Host: {host}:{http_port}\r\n\
         Connection: close\r\n\r\n"
    );
    stream
        .set_write_timeout(Some(remaining(deadline)?))
        .map_err(|e| ProbeError::Unreadable(format!("write timeout: {e}")))?;
    stream
        .write_all(request.as_bytes())
        .map_err(|e| ProbeError::Unreadable(format!("write: {e}")))?;

    let mut raw = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        stream
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(|e| ProbeError::Unreadable(format!("read timeout: {e}")))?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => raw.extend_from_slice(&chunk[..read]),
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(ProbeError::Unreadable(format!("read: {e}"))),
        }
        if raw.len() > MAX_RESPONSE_BYTES {
            return Err(ProbeError::Unreadable(format!(
                "answer exceeded {MAX_RESPONSE_BYTES} bytes"
            )));
        }
    }

    let (status, body) =
        parse_http_response(&String::from_utf8_lossy(&raw)).map_err(ProbeError::Unreadable)?;
    unpatched_from_response(status, &body).map_err(ProbeError::Unreadable)
}

/// The time left before `deadline`, or an error once it has passed.
///
/// A zero timeout means "block forever" to the socket options, so an expired
/// deadline has to be refused rather than passed along.
fn remaining(deadline: Instant) -> Result<Duration, ProbeError> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(ProbeError::Unreadable(format!(
            "olad did not finish answering within {PROBE_DEADLINE:?}"
        )));
    }
    Ok(left)
}

/// Splits a raw HTTP response into its status code and body.
///
/// Kept pure so every shape olad can answer with is testable without a socket.
fn parse_http_response(raw: &str) -> Result<(u16, String), String> {
    let (head, body) = raw
        .split_once("\r\n\r\n")
        .or_else(|| raw.split_once("\n\n"))
        .ok_or_else(|| "response had no blank line between head and body".to_string())?;

    let status_line = head.lines().next().unwrap_or_default();
    let mut fields = status_line.split_whitespace();
    let version = fields.next().unwrap_or_default();
    if !version.starts_with("HTTP/") {
        return Err(format!("not an HTTP status line: {status_line:?}"));
    }
    let status = fields
        .next()
        .ok_or_else(|| format!("status line carried no code: {status_line:?}"))?
        .parse::<u16>()
        .map_err(|_| format!("status line carried no numeric code: {status_line:?}"))?;

    Ok((status, body.to_string()))
}

/// Decides a universe's patch state from olad's status code and body.
fn unpatched_from_response(status: u16, body: &str) -> Result<bool, String> {
    if status == 200 {
        return unpatched_from_universe_info(body);
    }

    // olad garbage-collects a universe as soon as nothing is patched to it and
    // no client holds it, so a universe olad has never heard of is precisely
    // the universe nobody patched — the case this whole check exists for. It
    // says so with a 500 and an HTML body, not a 404 and not JSON:
    // "<b>500 Server Error</b><p>Universe doesn't exist</p>".
    if body.to_ascii_lowercase().contains("doesn't exist") {
        return Ok(true);
    }

    Err(format!("olad answered {status}"))
}

/// Answers whether a `/json/universe_info` body describes a universe with no
/// output port.
///
/// Kept apart from the socket so every shape olad can answer with is testable.
fn unpatched_from_universe_info(body: &str) -> Result<bool, String> {
    let info: serde_json::Value =
        serde_json::from_str(body.trim()).map_err(|e| format!("universe_info JSON: {e}"))?;
    let info = info
        .as_object()
        .ok_or_else(|| "universe_info was not a JSON object".to_string())?;

    // Some builds answer 200 with an error field rather than a 500.
    if let Some(error) = info.get("error").and_then(|error| error.as_str()) {
        if !error.is_empty() {
            return Ok(true);
        }
    }

    if let Some(ports) = info.get("output_ports").and_then(|ports| ports.as_array()) {
        return Ok(ports.is_empty());
    }
    if let Some(count) = info
        .get("output_port_count")
        .and_then(|count| count.as_u64())
    {
        return Ok(count == 0);
    }

    Err("universe_info had neither output_ports nor output_port_count".to_string())
}

#[cfg(test)]
mod test {
    use std::net::TcpListener;

    use super::*;

    const PATCHED_BODY: &str = r#"{"id":1,"name":"Universe 1","input_ports":[],"output_ports":[{"device":"Dummy Device","description":"Dummy Port","id":"2-0","is_output":true}]}"#;
    const UNPATCHED_BODY: &str =
        r#"{"id":3,"name":"Universe 3","input_ports":[],"output_ports":[]}"#;
    const OLAD_MISSING_UNIVERSE: &str = "<b>500 Server Error</b><p>Universe doesn't exist</p>";

    /// Serves one canned raw response on loopback and returns its port.
    ///
    /// The request is read first so the probe is never writing into a socket
    /// that has already gone away.
    fn serve_once(response: String) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(response.as_bytes());
            }
        });
        port
    }

    fn http(status_line: &str, body: &str) -> String {
        format!(
            "{status_line}\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
    }

    #[test]
    fn parses_a_json_answer() {
        let (status, body) = parse_http_response(&http("HTTP/1.1 200 OK", PATCHED_BODY)).unwrap();
        assert_eq!(status, 200);
        assert_eq!(body, PATCHED_BODY);
    }

    #[test]
    fn parses_olads_html_error() {
        let raw = http("HTTP/1.1 500 Server Error", OLAD_MISSING_UNIVERSE);
        let (status, body) = parse_http_response(&raw).unwrap();
        assert_eq!(status, 500);
        assert!(body.contains("Universe doesn't exist"));
    }

    #[test]
    fn rejects_a_malformed_head() {
        assert!(parse_http_response("GARBAGE\r\n\r\nbody").is_err());
        assert!(parse_http_response("HTTP/1.1\r\n\r\nbody").is_err());
        assert!(parse_http_response("HTTP/1.1 nope OK\r\n\r\nbody").is_err());
    }

    #[test]
    fn rejects_a_response_with_no_blank_line() {
        assert!(parse_http_response("HTTP/1.1 200 OK\r\nContent-Type: text/plain").is_err());
        assert!(parse_http_response("").is_err());
    }

    #[test]
    fn probes_a_patched_universe() {
        let port = serve_once(http("HTTP/1.1 200 OK", PATCHED_BODY));
        assert!(!probe_universe("127.0.0.1", port, 1).unwrap());
    }

    #[test]
    fn probes_an_unpatched_universe() {
        let port = serve_once(http("HTTP/1.1 200 OK", UNPATCHED_BODY));
        assert!(probe_universe("127.0.0.1", port, 3).unwrap());
    }

    #[test]
    fn probes_a_universe_olad_has_never_heard_of() {
        // olad's real answer for an unknown universe: a 500, in HTML.
        let port = serve_once(http("HTTP/1.1 500 Server Error", OLAD_MISSING_UNIVERSE));
        assert!(probe_universe("127.0.0.1", port, 7).unwrap());
    }

    #[test]
    fn probes_something_that_is_not_olad() {
        let port = serve_once("not an http response at all".to_string());
        assert!(matches!(
            probe_universe("127.0.0.1", port, 1),
            Err(ProbeError::Unreadable(_))
        ));
    }

    #[test]
    fn probes_an_error_status_that_is_not_a_missing_universe() {
        let port = serve_once(http("HTTP/1.1 503 Service Unavailable", "busy"));
        assert!(matches!(
            probe_universe("127.0.0.1", port, 1),
            Err(ProbeError::Unreadable(_))
        ));
    }

    #[test]
    fn a_web_server_that_is_not_there_is_unreachable() {
        // Bind and drop: nothing is listening on the port by the time we ask,
        // which is what an olad without its web server looks like.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);
        assert!(matches!(
            probe_universe("127.0.0.1", port, 1),
            Err(ProbeError::Unreachable(_))
        ));
    }

    #[test]
    fn patched_universe_is_not_unpatched() {
        assert!(!unpatched_from_universe_info(PATCHED_BODY).unwrap());
    }

    #[test]
    fn no_output_ports_is_unpatched() {
        assert!(unpatched_from_universe_info(UNPATCHED_BODY).unwrap());
    }

    #[test]
    fn output_port_count_is_honored() {
        assert!(unpatched_from_universe_info(r#"{"id": 3, "output_port_count": 0}"#).unwrap());
        assert!(!unpatched_from_universe_info(r#"{"id": 3, "output_port_count": 2}"#).unwrap());
    }

    #[test]
    fn missing_universe_is_unpatched() {
        assert!(unpatched_from_universe_info(r#"{"error": "Universe doesn't exist"}"#).unwrap());
    }

    #[test]
    fn empty_error_is_not_an_error() {
        // A live universe answers with an empty error alongside its ports.
        let body = r#"{"error": "", "id": 1, "output_ports": [{"device": "Dummy Device"}]}"#;
        assert!(!unpatched_from_universe_info(body).unwrap());
    }

    #[test]
    fn malformed_body_is_an_error() {
        assert!(unpatched_from_universe_info("<html>oops</html>").is_err());
        assert!(unpatched_from_universe_info(r#"{"id": 1}"#).is_err());
    }

    #[test]
    fn an_html_error_is_unpatched_whatever_its_case() {
        assert!(unpatched_from_response(500, "<p>Universe Doesn't Exist</p>").unwrap());
        assert!(unpatched_from_response(500, "<p>universe doesn't exist</p>").unwrap());
        assert!(unpatched_from_response(500, "<p>something else</p>").is_err());
    }
}
