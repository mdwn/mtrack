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

//! Testing one fixture from its page (lighting UI design §15): pick a mode,
//! a universe of the running profile and an address, and send it a colour,
//! a level, a strobe rate or a pose — or raw channel values — through the
//! running DMX engine.
//!
//! The controls resolve to DMX through the effects engine
//! ([`lighting::fixture_test`]), and the engine lays the result over its
//! output for a few seconds at a time ([`crate::dmx::engine::TestOutput`]):
//! the page keeps it alive while it is open, and it goes dark by itself
//! when nothing does.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use super::super::server::WebUiState;
use super::lighting_api::{
    canonical_project_root, project_root, resolve_lighting_dir, system_from_files,
    DEFAULT_FIXTURE_TYPES_DIR, DEFAULT_VENUES_DIR,
};
use crate::lighting;
use crate::lighting::fixture_test::{self, TestControls};
use crate::lighting::system::TypeInModeError;
use crate::lighting::types::FixtureType;

/// The directories a test works in, as the page passes them from the
/// running profile.
#[derive(serde::Deserialize, Default)]
pub(super) struct TestQuery {
    dir: Option<String>,
    venues_dir: Option<String>,
    fixture_type: Option<String>,
}

/// A test request.
#[derive(serde::Deserialize)]
pub(super) struct TestRequest {
    fixture_type: String,
    #[serde(default)]
    mode: Option<String>,
    universe: u16,
    address: u16,
    #[serde(default)]
    controls: serde_json::Map<String, serde_json::Value>,
    #[serde(default)]
    raw: BTreeMap<String, serde_json::Value>,
}

fn refuse(status: StatusCode, reason: &str, error: impl Into<String>) -> Response {
    (
        status,
        Json(json!({"error": error.into(), "reason": reason})),
    )
        .into_response()
}

/// Why testing is not possible right now, if it is not.
async fn unavailable(state: &WebUiState) -> Option<(&'static str, String)> {
    if state.player.is_locked() {
        return Some((
            "locked",
            "The player is locked. Unlock it to test a fixture.".to_string(),
        ));
    }
    if state.player.is_playing().await {
        return Some((
            "playing",
            "A song is playing. Stop it to test a fixture.".to_string(),
        ));
    }
    let Some(engine) = state.player.dmx_engine() else {
        return Some((
            "no_dmx",
            "The running profile has no DMX output, so there is nothing to send to.".to_string(),
        ));
    };
    if engine.configured_universes().is_empty() {
        return Some((
            "no_universes",
            "The running profile's DMX section names no universes.".to_string(),
        ));
    }
    None
}

/// The project root and the fixture types and venues directories a test
/// request reads.
#[allow(clippy::result_large_err)]
fn test_dirs(
    state: &WebUiState,
    query: &TestQuery,
) -> Result<(PathBuf, PathBuf, PathBuf), Response> {
    let types_dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        DEFAULT_FIXTURE_TYPES_DIR,
    )?;
    let venues_dir = resolve_lighting_dir(
        &state.config_path,
        query.venues_dir.as_deref(),
        DEFAULT_VENUES_DIR,
    )?;
    let root = canonical_project_root(&project_root(&state.config_path)?)?;
    Ok((root, types_dir, venues_dir))
}

/// A fixture type in a mode, loaded from the project's files, with the
/// type's GDTF-ness. Kept briefly so a slider's stream of updates does not
/// reload the project for every value.
#[derive(Clone)]
struct Prepared {
    fixture_type: FixtureType,
}

type PrepKey = (PathBuf, PathBuf, String, Option<String>);

static PREPARED: std::sync::LazyLock<parking_lot::Mutex<Option<(PrepKey, Instant, Prepared)>>> =
    std::sync::LazyLock::new(|| parking_lot::Mutex::new(None));

/// How long a prepared type is reused.
const PREPARED_TTL: Duration = Duration::from_secs(10);

/// Loads `type_name` in `mode` from the project, or the copy prepared a
/// moment ago.
fn prepare(
    root: &std::path::Path,
    types_dir: &std::path::Path,
    venues_dir: &std::path::Path,
    type_name: &str,
    mode: Option<&str>,
) -> Result<Prepared, TypeInModeError> {
    let key: PrepKey = (
        types_dir.to_path_buf(),
        venues_dir.to_path_buf(),
        type_name.to_string(),
        mode.map(str::to_string),
    );
    if let Some((k, at, prepared)) = PREPARED.lock().as_ref() {
        if *k == key && at.elapsed() < PREPARED_TTL {
            return Ok(prepared.clone());
        }
    }
    let system = system_from_files(root, types_dir, venues_dir).map_err(TypeInModeError::Mode)?;
    let fixture_type = system.type_in_mode(type_name, mode, root)?;
    let prepared = Prepared { fixture_type };
    *PREPARED.lock() = Some((key, Instant::now(), prepared.clone()));
    Ok(prepared)
}

/// Reads the request's controls: `color` as `#rrggbb`, `dimmer` 0..1,
/// `strobe` Hz or null for off, `pan`/`tilt` degrees, any other name a
/// channel level 0..1.
fn controls_from(map: &serde_json::Map<String, serde_json::Value>) -> Result<TestControls, String> {
    let mut controls = TestControls::default();
    for (key, value) in map {
        let number = || {
            value
                .as_f64()
                .filter(|v| v.is_finite())
                .ok_or_else(|| format!("control \"{key}\" must be a number"))
        };
        match key.as_str() {
            "color" => {
                let text = value
                    .as_str()
                    .ok_or("color must be \"#rrggbb\"".to_string())?;
                let hex = text.strip_prefix('#').unwrap_or(text);
                if hex.len() != 6 {
                    return Err("color must be \"#rrggbb\"".to_string());
                }
                let byte = |i: usize| {
                    u8::from_str_radix(&hex[i..i + 2], 16)
                        .map_err(|_| "color must be \"#rrggbb\"".to_string())
                };
                controls.color = Some([byte(0)?, byte(2)?, byte(4)?]);
            }
            "dimmer" => controls.dimmer = Some(number()?.clamp(0.0, 1.0)),
            "strobe" => {
                controls.strobe = Some(if value.is_null() {
                    None
                } else {
                    Some(number()?.max(0.0))
                })
            }
            "pan" => controls.pan = Some(number()?),
            "tilt" => controls.tilt = Some(number()?),
            other => {
                controls
                    .levels
                    .insert(other.to_string(), number()?.clamp(0.0, 1.0));
            }
        }
    }
    Ok(controls)
}

/// Reads raw values: keys are 1-based offsets or channel names.
fn raw_from(
    raw: &BTreeMap<String, serde_json::Value>,
    names: &BTreeMap<u16, String>,
) -> Result<BTreeMap<u16, u8>, String> {
    let mut out = BTreeMap::new();
    for (key, value) in raw {
        let offset = match key.parse::<u16>() {
            Ok(offset) if names.contains_key(&offset) => offset,
            _ => names
                .iter()
                .find(|(_, name)| *name == key)
                .map(|(offset, _)| *offset)
                .ok_or_else(|| format!("this mode has no channel \"{key}\""))?,
        };
        let byte = value
            .as_u64()
            .filter(|v| *v <= 255)
            .ok_or_else(|| format!("raw value for \"{key}\" must be 0 to 255"))?;
        out.insert(offset, byte as u8);
    }
    Ok(out)
}

/// The current venue's fixtures the span runs into, by name.
fn venue_overlaps(state: &WebUiState, universe: u16, from: u16, to: u16) -> Vec<String> {
    let Some(system) = state
        .player
        .broadcast_handles()
        .and_then(|h| h.lighting_system)
    else {
        return Vec::new();
    };
    let system = system.lock();
    let Some(venue) = system.get_current_venue() else {
        return Vec::new();
    };
    let mut names: Vec<String> = system
        .patch_spans(venue)
        .into_iter()
        .filter(|span| span.universe == universe && span.footprint > 0)
        .filter(|span| {
            let end = span.address.saturating_add(span.footprint - 1);
            span.address <= to && end >= from
        })
        .map(|span| span.fixture)
        .collect();
    names.sort();
    names
}

/// The profile's universes, with olad's patch state for each (null when
/// olad was not reachable).
fn universes_with_olad(
    engine: &crate::dmx::engine::Engine,
) -> (Vec<serde_json::Value>, Option<bool>) {
    let configured = engine.configured_universes();
    let report = crate::dmx::patch_check::probe_universes(engine.ola_http_port(), &configured);
    let reachable = report.reachable;
    let names = engine.universe_names();
    let universes = configured
        .iter()
        .map(|u| {
            json!({
                "universe": u,
                "name": names.get(u),
                "patched": reachable.then(|| !report.unpatched.contains(u)),
            })
        })
        .collect();
    (universes, Some(reachable))
}

/// GET /api/lighting/fixture-test/options?fixture_type= — what the test panel
/// needs: the type's modes, each with the controls it supports and its
/// channels; the running profile's universes with olad's patch state; and
/// whether a test can run now.
pub(super) async fn get_options(
    State(state): State<WebUiState>,
    Query(query): Query<TestQuery>,
) -> Response {
    let Some(type_name) = query.fixture_type.clone().filter(|n| !n.is_empty()) else {
        return refuse(StatusCode::BAD_REQUEST, "type", "fixture_type is required");
    };
    let (root, types_dir, venues_dir) = match test_dirs(&state, &query) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let why_not = unavailable(&state).await;
    let engine = state.player.dmx_engine();
    let name = type_name.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let system = system_from_files(&root, &types_dir, &venues_dir)?;
        let gdtf = system.gdtf_archive(&name).map(str::to_string);
        let mode_names: Vec<Option<String>> = match &gdtf {
            Some(archive) => {
                let bytes = std::fs::read(root.join(archive)).map_err(|e| e.to_string())?;
                let description =
                    lighting::gdtf::parse_archive(&bytes).map_err(|e| e.to_string())?;
                description
                    .modes
                    .into_iter()
                    .map(|m| Some(m.name))
                    .collect()
            }
            None => {
                if system.fixture_types_iter().all(|(n, _)| *n != name) {
                    return Ok(None);
                }
                vec![None]
            }
        };
        let modes: Vec<serde_json::Value> = mode_names
            .into_iter()
            .map(
                |mode| match system.type_in_mode(&name, mode.as_deref(), &root) {
                    Ok(fixture_type) => {
                        let info =
                            lighting::system::fixture_info_for(&name, 1, 1, &name, &fixture_type);
                        let channels: Vec<serde_json::Value> = fixture_test::channel_names(&info)
                            .into_iter()
                            .map(|(offset, name)| json!({"offset": offset, "name": name}))
                            .collect();
                        json!({
                            "name": mode,
                            "drivable": true,
                            "error": null,
                            "footprint": fixture_test::footprint(&info),
                            "controls": fixture_test::control_support(&info),
                            "channels": channels,
                        })
                    }
                    Err(e) => json!({
                        "name": mode,
                        "drivable": false,
                        "error": e.to_string(),
                        "footprint": null,
                        "controls": null,
                        "channels": [],
                    }),
                },
            )
            .collect();
        let default_mode = modes
            .iter()
            .find(|m| m["drivable"] == true)
            .map(|m| m["name"].clone())
            .unwrap_or(serde_json::Value::Null);
        let (universes, reachable) = match &engine {
            Some(engine) => universes_with_olad(engine),
            None => (Vec::new(), None),
        };
        let olad_port = engine.as_ref().map(|e| e.ola_http_port());
        Ok::<_, String>(Some(json!({
            "fixture_type": name,
            "gdtf": gdtf.is_some(),
            "modes": modes,
            "default_mode": default_mode,
            "universes": universes,
            "olad": {"reachable": reachable, "port": olad_port},
        })))
    })
    .await;
    match outcome {
        Ok(Ok(Some(mut body))) => {
            body["available"] = json!(why_not.is_none());
            body["unavailable"] = json!(why_not.as_ref().map(|(r, _)| r));
            body["unavailable_message"] = json!(why_not.map(|(_, m)| m));
            (StatusCode::OK, Json(body)).into_response()
        }
        Ok(Ok(None)) => refuse(
            StatusCode::NOT_FOUND,
            "type",
            format!("no fixture type named \"{type_name}\""),
        ),
        Ok(Err(e)) => refuse(StatusCode::INTERNAL_SERVER_ERROR, "files", e),
        Err(e) => refuse(StatusCode::INTERNAL_SERVER_ERROR, "files", e.to_string()),
    }
}

/// POST /api/lighting/fixture-test — starts or updates a test: resolves the
/// controls (and raw values over them) for the fixture at the given
/// universe and address, lays them over the engine's output, and answers
/// the frame, how long it holds, and anything worth knowing.
pub(super) async fn post_test(
    State(state): State<WebUiState>,
    Query(query): Query<TestQuery>,
    Json(request): Json<TestRequest>,
) -> Response {
    if let Some((reason, message)) = unavailable(&state).await {
        let status = if reason == "locked" {
            StatusCode::LOCKED
        } else {
            StatusCode::CONFLICT
        };
        return refuse(status, reason, message);
    }
    let Some(engine) = state.player.dmx_engine() else {
        return refuse(StatusCode::CONFLICT, "no_dmx", "no DMX engine");
    };
    if !engine.configured_universes().contains(&request.universe) {
        return refuse(
            StatusCode::BAD_REQUEST,
            "universe",
            format!(
                "universe {} is not one of the running profile's DMX universes",
                request.universe
            ),
        );
    }
    if !(1..=512).contains(&request.address) {
        return refuse(
            StatusCode::BAD_REQUEST,
            "address",
            "the address must be 1 to 512",
        );
    }
    let controls = match controls_from(&request.controls) {
        Ok(c) => c,
        Err(e) => return refuse(StatusCode::BAD_REQUEST, "controls", e),
    };
    let (root, types_dir, venues_dir) = match test_dirs(&state, &query) {
        Ok(d) => d,
        Err(r) => return r,
    };
    let (type_name, mode) = (request.fixture_type.clone(), request.mode.clone());
    let prepared = tokio::task::spawn_blocking(move || {
        prepare(&root, &types_dir, &venues_dir, &type_name, mode.as_deref())
    })
    .await;
    let prepared = match prepared {
        Ok(Ok(p)) => p,
        Ok(Err(TypeInModeError::Unknown(name))) => {
            return refuse(
                StatusCode::NOT_FOUND,
                "type",
                format!("no fixture type named \"{name}\""),
            )
        }
        Ok(Err(TypeInModeError::Mode(why))) => return refuse(StatusCode::BAD_REQUEST, "mode", why),
        Err(e) => return refuse(StatusCode::INTERNAL_SERVER_ERROR, "files", e.to_string()),
    };

    let info = lighting::system::fixture_info_for(
        "test",
        request.universe,
        request.address,
        &request.fixture_type,
        &prepared.fixture_type,
    );
    let names = fixture_test::channel_names(&info);
    let raw = match raw_from(&request.raw, &names) {
        Ok(r) => r,
        Err(e) => return refuse(StatusCode::BAD_REQUEST, "raw", e),
    };
    let frame = fixture_test::resolve_frame(&info, &controls, &raw);
    let footprint = fixture_test::footprint(&info);
    let channels: Vec<(u16, u8)> = frame
        .iter()
        .filter(|c| c.address <= 512)
        .map(|c| (c.address, c.value))
        .collect();
    let expiry = match engine.set_test_output(crate::dmx::engine::TestOutput {
        universe: request.universe,
        address: request.address,
        footprint,
        fixture_type: request.fixture_type.clone(),
        mode: request.mode.clone(),
        channels,
    }) {
        Ok(expiry) => expiry,
        Err(e) => return refuse(StatusCode::BAD_REQUEST, "universe", e),
    };

    let warnings = warnings(
        &state,
        &engine,
        &request,
        footprint,
        &fixture_test::unsupported(&fixture_test::control_support(&info), &controls),
    )
    .await;
    (
        StatusCode::OK,
        Json(json!({
            "active": true,
            "universe": request.universe,
            "address": request.address,
            "footprint": footprint,
            "fixture_type": request.fixture_type,
            "mode": request.mode,
            "expires_in_secs": expiry.as_secs_f64(),
            "frame": frame,
            "warnings": warnings,
        })),
    )
        .into_response()
}

/// What a test should be told: fixtures of the current venue it is lit
/// over, a span past 512, olad not answering or not patched for the
/// universe, and controls the mode has no channel for.
async fn warnings(
    state: &WebUiState,
    engine: &Arc<crate::dmx::engine::Engine>,
    request: &TestRequest,
    footprint: u16,
    unsupported: &[String],
) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    let from = request.address;
    let to = from.saturating_add(footprint.saturating_sub(1));
    let overlaps = venue_overlaps(state, request.universe, from, to.min(512));
    if !overlaps.is_empty() {
        out.push(json!({
            "kind": "venue_overlap",
            "fixtures": overlaps,
            "message": format!(
                "These addresses belong to {} in the current venue; the test overrides {} while it runs.",
                overlaps.join(", "),
                if overlaps.len() == 1 { "it" } else { "them" },
            ),
        }));
    }
    if u32::from(from) + u32::from(footprint) - 1 > 512 && footprint > 0 {
        out.push(json!({
            "kind": "past_universe_end",
            "message": format!(
                "This fixture needs addresses {from}–{}, past the end of the universe (512); the channels past 512 are not sent.",
                u32::from(from) + u32::from(footprint) - 1
            ),
        }));
    }
    let port = engine.ola_http_port();
    let universe = request.universe;
    let report = tokio::task::spawn_blocking(move || {
        crate::dmx::patch_check::probe_universes(port, &[universe])
    })
    .await
    .ok();
    if let Some(report) = report {
        if !report.reachable {
            out.push(json!({
                "kind": "olad_unreachable",
                "message": format!(
                    "mtrack cannot reach olad (the DMX service) on port {port}, so nothing reaches the lights."
                ),
            }));
        } else if report.unpatched.contains(&universe) {
            out.push(json!({
                "kind": "universe_unpatched",
                "message": format!(
                    "olad has no output patched to universe {universe}: mtrack sends, but nothing leaves the computer."
                ),
            }));
        }
    }
    for control in unsupported {
        out.push(json!({
            "kind": "unsupported_control",
            "control": control,
            "message": format!("This mode has no channel for {control}."),
        }));
    }
    out
}

/// DELETE /api/lighting/fixture-test — releases the test now.
pub(super) async fn delete_test(State(state): State<WebUiState>) -> Response {
    if let Some(engine) = state.player.dmx_engine() {
        engine.release_test_output();
    }
    (StatusCode::OK, Json(json!({"active": false}))).into_response()
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::path::Path as FsPath;

    use axum::body::Body;
    use tower::ServiceExt;

    use super::super::super::api;
    use super::super::test_helpers::*;
    use super::*;

    const PAR: &str = "fixture_type \"Par\" {\n  channels: 4\n  \
        channel_map: { \"dimmer\": 1, \"red\": 2, \"green\": 3, \"blue\": 4 }\n}\n";

    /// A fake olad web server answering every universe query with `body`.
    fn fake_olad(body: &'static str) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for mut stream in listener.incoming().flatten() {
                let mut request = [0u8; 1024];
                let _ = stream.read(&mut request);
                let _ = stream.write_all(
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\n\r\n{body}",
                        body.len()
                    )
                    .as_bytes(),
                );
            }
        });
        port
    }

    fn dead_port() -> u16 {
        TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port()
    }

    fn write_project(root: &FsPath, venue: Option<&str>) {
        std::fs::create_dir_all(root.join("lighting/fixture_types")).unwrap();
        std::fs::create_dir_all(root.join("lighting/venues")).unwrap();
        std::fs::create_dir_all(root.join("lighting/library")).unwrap();
        std::fs::write(root.join("lighting/fixture_types/par.light"), PAR).unwrap();
        std::fs::write(
            root.join("lighting/library/synth.gdtf"),
            crate::lighting::gdtf::build_zip(&[(
                "description.xml",
                crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.as_bytes(),
            )]),
        )
        .unwrap();
        if let Some(venue) = venue {
            std::fs::write(root.join("lighting/venues/house.light"), venue).unwrap();
        }
    }

    /// A state whose player runs a DMX engine on universe 1 (olad's web
    /// server at `olad_port`), with the current venue `venue` when given.
    fn dmx_state(
        olad_port: u16,
        venue: Option<&str>,
    ) -> (
        WebUiState,
        tempfile::TempDir,
        tempfile::TempDir,
        Arc<crate::dmx::engine::Engine>,
    ) {
        let engine_project = tempfile::tempdir().unwrap();
        write_project(engine_project.path(), venue);
        let current = if venue.is_some() {
            "  current_venue: house\n"
        } else {
            ""
        };
        let yaml = format!(
            "ola_http_port: {olad_port}\nuniverses:\n  - universe: 1\n    name: main\nlighting:\n{current}  groups: {{}}\n"
        );
        let dmx: crate::config::Dmx = config::Config::builder()
            .add_source(config::File::from_str(&yaml, config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        let engine = Arc::new(
            crate::dmx::engine::Engine::new(
                &dmx,
                dmx.lighting(),
                Some(engine_project.path()),
                crate::dmx::ola_client::OlaClientFactory::create_mock_client(),
            )
            .unwrap(),
        );
        let songs = Arc::new(crate::songs::Songs::new(std::collections::HashMap::new()));
        let (state, dir) = test_state_with_dmx(songs, engine.clone());
        // A player starts locked, as a show machine should; a test edits.
        state.player.set_locked(false);
        write_project(dir.path(), venue);
        (state, dir, engine_project, engine)
    }

    async fn call(
        state: &WebUiState,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let request = http::Request::builder()
            .method(method)
            .uri(uri)
            .header("content-type", "application/json");
        let request = match body {
            Some(b) => request.body(Body::from(b.to_string())).unwrap(),
            None => request.body(Body::empty()).unwrap(),
        };
        let response = api::guarded_router(state)
            .with_state(state.clone())
            .oneshot(request)
            .await
            .unwrap();
        let status = response.status();
        let text = response_body(response).await;
        (status, serde_json::from_str(&text).unwrap_or(json!(text)))
    }

    fn red_par() -> serde_json::Value {
        json!({
            "fixture_type": "Par", "mode": null, "universe": 1, "address": 10,
            "controls": {"color": "#ff0000", "dimmer": 1},
        })
    }

    #[tokio::test]
    async fn a_test_resolves_the_frame_lays_it_over_the_output_and_releases() {
        let (state, _dir, _p, engine) = dmx_state(dead_port(), None);
        let (status, body) = call(&state, "POST", "/lighting/fixture-test", Some(red_par())).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["active"], true);
        assert_eq!(body["footprint"], 4);
        let frame: Vec<(u64, String, u64)> = body["frame"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["address"].as_u64().unwrap(),
                    c["name"].as_str().unwrap().to_string(),
                    c["value"].as_u64().unwrap(),
                )
            })
            .collect();
        assert_eq!(
            frame,
            vec![
                (10, "dimmer".into(), 255),
                (11, "red".into(), 255),
                (12, "green".into(), 0),
                (13, "blue".into(), 0)
            ]
        );
        assert!(body["expires_in_secs"].as_f64().unwrap() > 0.0);
        let live = engine.test_output_status().expect("live");
        assert_eq!((live.universe, live.address, live.footprint), (1, 10, 4));
        // olad is not there: said.
        let kinds: Vec<&str> = body["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, vec!["olad_unreachable"]);

        // Status says so, for every page.
        let (_, status_body) = call(&state, "GET", "/status", None).await;
        assert_eq!(status_body["hardware"]["test_output"]["address"], 10);

        // A raw value wins over its channel, by offset or by name.
        let mut raw = red_par();
        raw["raw"] = json!({"3": 42, "blue": 7});
        let (_, body) = call(&state, "POST", "/lighting/fixture-test", Some(raw)).await;
        assert_eq!(body["frame"][2]["value"], 42);
        assert_eq!(body["frame"][2]["raw"], true);
        assert_eq!(body["frame"][3]["value"], 7);

        let (status, body) = call(&state, "DELETE", "/lighting/fixture-test", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["active"], false);
        assert!(engine.test_output_status().is_none());
        let (_, status_body) = call(&state, "GET", "/status", None).await;
        assert!(status_body["hardware"]["test_output"].is_null());
    }

    #[tokio::test]
    async fn a_gdtf_fixture_in_a_mode_and_what_the_mode_cannot_do() {
        let (state, _dir, _p, _engine) = dmx_state(dead_port(), None);
        let body = json!({
            "fixture_type": "Synth Brick", "mode": "8: RGBS", "universe": 1, "address": 1,
            "controls": {"color": "#ffffff", "dimmer": 1, "pan": 10},
        });
        let (status, body) = call(&state, "POST", "/lighting/fixture-test", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let unsupported: Vec<&str> = body["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|w| w["kind"] == "unsupported_control")
            .map(|w| w["control"].as_str().unwrap())
            .collect();
        assert_eq!(unsupported, vec!["pan"]);
        assert!(body["frame"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "red" && c["value"] == 255));
    }

    #[tokio::test]
    async fn each_refusal_says_why() {
        let (state, _dir, _p, engine) = dmx_state(dead_port(), None);
        let post = |body: serde_json::Value| {
            let state = state.clone();
            async move { call(&state, "POST", "/lighting/fixture-test", Some(body)).await }
        };
        let with = |key: &str, value: serde_json::Value| {
            let mut b = red_par();
            b[key] = value;
            b
        };

        let (status, body) = post(with("universe", json!(9))).await;
        assert_eq!(
            (status, body["reason"].as_str()),
            (StatusCode::BAD_REQUEST, Some("universe"))
        );
        let (status, body) = post(with("fixture_type", json!("Nope"))).await;
        assert_eq!(
            (status, body["reason"].as_str()),
            (StatusCode::NOT_FOUND, Some("type"))
        );
        let (status, body) = post(with("mode", json!("8: RGBS"))).await;
        assert_eq!(
            (status, body["reason"].as_str()),
            (StatusCode::BAD_REQUEST, Some("mode"))
        );
        assert!(
            body["error"].as_str().unwrap().contains("no modes"),
            "{body}"
        );
        let gdtf =
            json!({"fixture_type": "Synth Brick", "mode": null, "universe": 1, "address": 1});
        let (status, body) = post(gdtf).await;
        assert_eq!(
            (status, body["reason"].as_str()),
            (StatusCode::BAD_REQUEST, Some("mode"))
        );
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("choose one of its modes"),
            "{body}"
        );
        let (status, body) = post(with("address", json!(0))).await;
        assert_eq!(
            (status, body["reason"].as_str()),
            (StatusCode::BAD_REQUEST, Some("address"))
        );
        let mut bad = red_par();
        bad["controls"]["color"] = json!("red");
        let (status, body) = post(bad).await;
        assert_eq!(
            (status, body["reason"].as_str()),
            (StatusCode::BAD_REQUEST, Some("controls"))
        );
        assert!(
            engine.test_output_status().is_none(),
            "nothing refused was sent"
        );

        // Locked: refused, and locking releases a running test.
        let (status, _) = post(red_par()).await;
        assert_eq!(status, StatusCode::OK);
        state.player.set_locked(true);
        assert!(
            engine.test_output_status().is_none(),
            "the lock released it"
        );
        let (status, _) = post(red_par()).await;
        assert_eq!(status, StatusCode::LOCKED);
        // Releasing stays open while locked.
        let (status, _) = call(&state, "DELETE", "/lighting/fixture-test", None).await;
        assert_eq!(status, StatusCode::OK);
        let (_, options) = call(
            &state,
            "GET",
            "/lighting/fixture-test/options?fixture_type=Par",
            None,
        )
        .await;
        assert_eq!(options["unavailable"], "locked");
        assert_eq!(options["available"], false);
    }

    #[tokio::test]
    async fn without_dmx_or_while_playing_there_is_nothing_to_test() {
        let (state, dir) = test_state();
        state.player.set_locked(false);
        write_project(dir.path(), None);
        let (status, body) = call(&state, "POST", "/lighting/fixture-test", Some(red_par())).await;
        assert_eq!(
            (status, body["reason"].as_str()),
            (StatusCode::CONFLICT, Some("no_dmx"))
        );
        let (_, options) = call(
            &state,
            "GET",
            "/lighting/fixture-test/options?fixture_type=Par",
            None,
        )
        .await;
        assert_eq!(options["unavailable"], "no_dmx");
        assert_eq!(options["universes"], json!([]));

        // A song playing comes first: stop it, then there is the DMX to set up.
        let (state, dir) = test_state_with_audio_device("mock-device");
        state.player.set_locked(false);
        write_project(dir.path(), None);
        state.player.play().await.unwrap();
        assert!(state.player.is_playing().await);
        let (status, body) = call(&state, "POST", "/lighting/fixture-test", Some(red_par())).await;
        assert_eq!(
            (status, body["reason"].as_str()),
            (StatusCode::CONFLICT, Some("playing"))
        );
        state.player.stop().await;
    }

    #[tokio::test]
    async fn the_warnings_name_the_venue_fixtures_and_the_unpatched_universe() {
        let olad = fake_olad(r#"{"id":1,"name":"Universe 1","input_ports":[],"output_ports":[]}"#);
        let venue =
            "venue \"house\" {\n  fixture \"Front\" Par @ 1:11\n  fixture \"Back\" Par @ 1:30\n}\n";
        let (state, _dir, _p, _engine) = dmx_state(olad, Some(venue));
        let mut body = red_par();
        body["address"] = json!(510);
        let (status, body) = call(&state, "POST", "/lighting/fixture-test", Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let kinds: Vec<&str> = body["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| w["kind"].as_str().unwrap())
            .collect();
        assert_eq!(kinds, vec!["past_universe_end", "universe_unpatched"]);
        // Only the channels up to 512 are in the override.
        assert_eq!(body["frame"].as_array().unwrap().len(), 4);

        let (_, body) = call(&state, "POST", "/lighting/fixture-test", Some(red_par())).await;
        let overlap = body["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["kind"] == "venue_overlap")
            .expect("an overlap")
            .clone();
        assert_eq!(overlap["fixtures"], json!(["Front"]));

        let (_, options) = call(
            &state,
            "GET",
            "/lighting/fixture-test/options?fixture_type=Par",
            None,
        )
        .await;
        assert_eq!(options["universes"][0]["patched"], false);
        assert_eq!(options["olad"]["reachable"], true);
    }

    #[tokio::test]
    async fn the_options_describe_each_mode() {
        let (state, _dir, _p, _engine) = dmx_state(dead_port(), None);
        let (status, options) = call(
            &state,
            "GET",
            "/lighting/fixture-test/options?fixture_type=Synth%20Brick",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{options}");
        assert_eq!(options["gdtf"], true);
        assert_eq!(options["available"], true);
        assert_eq!(options["default_mode"], "8: RGBS");
        let modes = options["modes"].as_array().unwrap();
        let rgbs = modes.iter().find(|m| m["name"] == "8: RGBS").unwrap();
        assert_eq!(rgbs["controls"]["color"], true);
        assert!(rgbs["controls"]["strobe"].is_object(), "{rgbs}");
        assert!(rgbs["controls"]["pan"].is_null());
        let mover = modes.iter().find(|m| m["name"] == "Mover 16bit").unwrap();
        assert!(
            mover["controls"]["pan"]["max"].as_f64().unwrap() > 0.0,
            "{mover}"
        );
        assert!(mover["channels"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c["name"] == "pan fine"));
        assert_eq!(options["universes"][0]["universe"], 1);
        assert_eq!(options["universes"][0]["name"], "main");
        assert!(
            options["universes"][0]["patched"].is_null(),
            "olad unreachable: unknown"
        );

        let (_, native) = call(
            &state,
            "GET",
            "/lighting/fixture-test/options?fixture_type=Par",
            None,
        )
        .await;
        assert_eq!(native["gdtf"], false);
        assert_eq!(native["modes"].as_array().unwrap().len(), 1);
        assert!(native["modes"][0]["name"].is_null());
        assert_eq!(native["modes"][0]["controls"]["dimmer"], "channel");
        let (status, _) = call(
            &state,
            "GET",
            "/lighting/fixture-test/options?fixture_type=Nope",
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
