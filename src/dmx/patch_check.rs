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
    collections::HashMap,
    io::{ErrorKind, Read, Write},
    net::{TcpStream, ToSocketAddrs},
    sync::Mutex,
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

/// The whole probe — every universe, asked concurrently — is bounded by wall
/// clock: per-read timeouts alone let a server that dribbles a byte at a time
/// keep the socket open forever, and a wedged olad costs this once, not once
/// per universe.
const PROBE_DEADLINE: Duration = Duration::from_secs(2);

/// How long a report answers later probes. The hub and the fit view ask on
/// every load; olad's patch state changes when an operator runs `ola_patch`,
/// not between two page loads.
const CACHE_TTL: Duration = Duration::from_secs(10);

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

/// What olad's web server said about a set of universes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PatchReport {
    /// Whether olad's web server answered at all. `false` means nothing was
    /// learned about any universe.
    pub reachable: bool,
    /// Why it did not answer, when it did not.
    pub unreachable_reason: Option<String>,
    /// Universes olad has no output port patched to.
    pub unpatched: Vec<u16>,
    /// Universes whose answer could not be read, with why. Not a finding
    /// either way: olad answered, but not in a shape that settles the question.
    pub unreadable: Vec<(u16, String)>,
}

/// Asks olad's web server about each universe, once, in a settled order.
///
/// Blocking, and bounded: the universes are asked concurrently under one
/// [`PROBE_DEADLINE`], so N universes cost at most that, not N times it. An
/// unreachable server ends the report at the first universe (in order) that
/// saw it. Callers on an async runtime belong on the blocking pool.
///
/// The report is cached for [`CACHE_TTL`] per (port, universe set), so a
/// burst of readiness or fit requests makes one probe; concurrent callers
/// wait for the probe in flight rather than starting their own.
pub fn probe_universes(http_port: u16, universes: &[u16]) -> PatchReport {
    probe_cached(&CACHE, OLAD_HOST, http_port, universes, CACHE_TTL)
}

type Cache = Mutex<HashMap<(String, u16, Vec<u16>), (Instant, PatchReport)>>;

static CACHE: std::sync::LazyLock<Cache> = std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

fn probe_cached(
    cache: &Cache,
    host: &str,
    http_port: u16,
    universes: &[u16],
    ttl: Duration,
) -> PatchReport {
    let universes = settled(universes);
    let key = (host.to_string(), http_port, universes.clone());
    // Held across the probe on purpose: a second caller for the same rig
    // should wait for this answer, not send its own.
    let mut cache = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((at, report)) = cache.get(&key) {
        if at.elapsed() < ttl {
            return report.clone();
        }
    }
    let report = probe_universes_at(host, http_port, &universes);
    cache.retain(|_, (at, _)| at.elapsed() < ttl);
    cache.insert(key, (Instant::now(), report.clone()));
    report
}

/// The same universe can appear under several devices; ask about each one
/// once, and in a settled order.
fn settled(universes: &[u16]) -> Vec<u16> {
    let mut universes = universes.to_vec();
    universes.sort_unstable();
    universes.dedup();
    universes
}

fn probe_universes_at(host: &str, http_port: u16, universes: &[u16]) -> PatchReport {
    let universes = settled(universes);
    let deadline = Instant::now() + PROBE_DEADLINE;

    // One thread per universe, all under the same deadline.
    let answers: Vec<Result<bool, ProbeError>> = thread::scope(|scope| {
        let handles: Vec<_> = universes
            .iter()
            .map(|universe| {
                scope.spawn(move || probe_universe(host, http_port, *universe, deadline))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err(ProbeError::Unreadable("probe thread panicked".into())))
            })
            .collect()
    });

    let mut report = PatchReport {
        reachable: true,
        ..Default::default()
    };
    for (universe, answer) in universes.into_iter().zip(answers) {
        match answer {
            Ok(true) => report.unpatched.push(universe),
            Ok(false) => {}
            // Nothing answered, so nothing will: report no further.
            Err(ProbeError::Unreachable(e)) => {
                report.reachable = false;
                report.unreachable_reason = Some(e);
                break;
            }
            // This universe alone is unreadable. The next one may not be, and
            // it may be the one the operator is about to run.
            Err(ProbeError::Unreadable(e)) => report.unreadable.push((universe, e)),
        }
    }
    report
}

/// Warns about every configured universe olad would silently drop frames for.
///
/// Runs detached: the probe must never delay engine startup or DMX output.
///
/// The thread is deliberately never joined — it holds no reference to the
/// engine, ends on its own within seconds, and the only thing a shutdown could
/// gain by waiting for it is a warning nobody is left to read.
pub fn warn_unpatched_universes(http_port: u16, universes: Vec<u16>) {
    if universes.is_empty() {
        return;
    }

    let spawned = thread::Builder::new()
        .name("olad-patch-check".to_string())
        .spawn(move || {
            let report = probe_universes(http_port, &universes);
            for universe in &report.unpatched {
                warn!(
                    universe,
                    "Universe {universe} is configured under dmx.universes but has no output \
                     port patched in olad — olad will silently drop every frame mtrack \
                     streams there. Patch one (ola_patch -d <device> -p <port> -u {universe}, \
                     or olad's web UI on :{http_port})."
                );
            }
            for (universe, e) in &report.unreadable {
                debug!(
                    universe,
                    "could not verify olad patch state for universe {universe}: {e}"
                );
            }
            // olad's web server is optional and this check is advisory, so
            // debug is loud enough.
            if let Some(e) = &report.unreachable_reason {
                debug!(
                    "could not reach olad's web server on {OLAD_HOST}:{http_port} to \
                     verify patch state: {e}"
                );
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
fn probe_universe(
    host: &str,
    http_port: u16,
    universe: u16,
    deadline: Instant,
) -> Result<bool, ProbeError> {
    let address = (host, http_port)
        .to_socket_addrs()
        .map_err(|e| ProbeError::Unreachable(format!("{host}:{http_port}: {e}")))?
        .next()
        .ok_or_else(|| {
            ProbeError::Unreachable(format!("{host}:{http_port} resolved to no address"))
        })?;

    let mut stream =
        TcpStream::connect_timeout(&address, CONNECT_TIMEOUT.min(remaining(deadline)?))
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
        assert!(!probe_universe("127.0.0.1", port, 1, Instant::now() + PROBE_DEADLINE).unwrap());
    }

    #[test]
    fn probes_an_unpatched_universe() {
        let port = serve_once(http("HTTP/1.1 200 OK", UNPATCHED_BODY));
        assert!(probe_universe("127.0.0.1", port, 3, Instant::now() + PROBE_DEADLINE).unwrap());
    }

    #[test]
    fn probes_a_universe_olad_has_never_heard_of() {
        // olad's real answer for an unknown universe: a 500, in HTML.
        let port = serve_once(http("HTTP/1.1 500 Server Error", OLAD_MISSING_UNIVERSE));
        assert!(probe_universe("127.0.0.1", port, 7, Instant::now() + PROBE_DEADLINE).unwrap());
    }

    #[test]
    fn probes_something_that_is_not_olad() {
        let port = serve_once("not an http response at all".to_string());
        assert!(matches!(
            probe_universe("127.0.0.1", port, 1, Instant::now() + PROBE_DEADLINE),
            Err(ProbeError::Unreadable(_))
        ));
    }

    #[test]
    fn probes_an_error_status_that_is_not_a_missing_universe() {
        let port = serve_once(http("HTTP/1.1 503 Service Unavailable", "busy"));
        assert!(matches!(
            probe_universe("127.0.0.1", port, 1, Instant::now() + PROBE_DEADLINE),
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
            probe_universe("127.0.0.1", port, 1, Instant::now() + PROBE_DEADLINE),
            Err(ProbeError::Unreachable(_))
        ));
    }

    /// Serves every connection, answering by the universe id in the request
    /// line (the probes arrive concurrently, in no fixed order). Counts them.
    fn serve_by_universe(
        answers: HashMap<u16, String>,
    ) -> (u16, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = hits.clone();
        thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let answers = answers.clone();
                thread::spawn(move || {
                    let mut request = [0u8; 1024];
                    let read = stream.read(&mut request).unwrap_or(0);
                    let request = String::from_utf8_lossy(&request[..read]).into_owned();
                    let id: u16 = request
                        .split("id=")
                        .nth(1)
                        .and_then(|rest| rest.split_whitespace().next())
                        .and_then(|id| id.parse().ok())
                        .unwrap_or(0);
                    if let Some(answer) = answers.get(&id) {
                        let _ = stream.write_all(answer.as_bytes());
                    }
                });
            }
        });
        (port, hits)
    }

    #[test]
    fn a_report_lists_what_olad_said_about_each_universe() {
        // Three universes: patched, never-heard-of, garbage.
        let (port, _) = serve_by_universe(HashMap::from([
            (1, http("HTTP/1.1 200 OK", PATCHED_BODY)),
            (2, http("HTTP/1.1 500 Server Error", OLAD_MISSING_UNIVERSE)),
            (3, "not an http response at all".to_string()),
        ]));
        // Out of order and duplicated: asked once each, in a settled order.
        let report = probe_universes_at("127.0.0.1", port, &[3, 1, 2, 1]);
        assert!(report.reachable);
        assert_eq!(report.unpatched, vec![2]);
        assert_eq!(report.unreadable.len(), 1);
        assert_eq!(report.unreadable[0].0, 3);
    }

    /// A server that accepts and never answers, and the port it listens on.
    /// The listener is returned so it (and the accepted sockets) stay open.
    fn hung_server() -> (u16, thread::JoinHandle<()>, std::sync::mpsc::Sender<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        let (stop, stopped) = std::sync::mpsc::channel::<()>();
        let handle = thread::spawn(move || {
            listener.set_nonblocking(true).unwrap();
            let mut held = Vec::new();
            while stopped.try_recv().is_err() {
                if let Ok((stream, _)) = listener.accept() {
                    held.push(stream);
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
        (port, handle, stop)
    }

    #[test]
    fn a_hung_server_costs_one_deadline_not_one_per_universe() {
        let (port, handle, stop) = hung_server();
        let started = Instant::now();
        let report = probe_universes_at("127.0.0.1", port, &[1, 2, 3, 4]);
        let took = started.elapsed();
        stop.send(()).unwrap();
        handle.join().unwrap();

        assert!(report.reachable);
        assert_eq!(report.unreadable.len(), 4, "{report:?}");
        assert!(
            took < Duration::from_millis(3500),
            "4 universes took {took:?}; the deadline is shared, ~2s"
        );
    }

    #[test]
    fn a_second_probe_within_the_ttl_is_served_from_the_cache() {
        let (port, hits) = serve_by_universe(HashMap::from([
            (1, http("HTTP/1.1 200 OK", PATCHED_BODY)),
            (2, http("HTTP/1.1 200 OK", UNPATCHED_BODY)),
        ]));
        let cache: Cache = Mutex::new(HashMap::new());
        let ttl = Duration::from_secs(10);
        let first = probe_cached(&cache, "127.0.0.1", port, &[2, 1], ttl);
        let second = probe_cached(&cache, "127.0.0.1", port, &[1, 2, 2], ttl);
        assert_eq!(first, second);
        assert_eq!(first.unpatched, vec![2]);
        assert_eq!(
            hits.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "one probe = one request per universe, and the second probe made none"
        );

        // A different universe set is its own entry; an expired one is asked again.
        probe_cached(&cache, "127.0.0.1", port, &[1], ttl);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 3);
        probe_cached(&cache, "127.0.0.1", port, &[1], Duration::ZERO);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 4);
    }

    #[test]
    fn a_report_from_a_dead_web_server_is_unreachable_and_stops_early() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        drop(listener);
        let report = probe_universes_at("127.0.0.1", port, &[1, 2, 3]);
        assert!(!report.reachable);
        assert!(report.unreachable_reason.is_some());
        assert!(report.unpatched.is_empty() && report.unreadable.is_empty());
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
