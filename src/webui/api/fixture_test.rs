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

use axum::http::StatusCode;
use serde_json::json;

use super::lighting_api::{
    canonical_project_root, project_root, resolve_lighting_dir, system_from_files, LightingDir,
};
use super::lighting_writes::Answer;
use crate::lighting;
use crate::lighting::fixture_test::{self, TestControls};
use crate::lighting::patch::{self, PatchSpan};
use crate::lighting::system::TypeInModeError;
use crate::lighting::types::FixtureType;
use crate::player::Player;
use crate::webui::wire::fixture_test::{
    TestAnswer, TestChannel, TestMode, TestOlad, TestOptions, TestUnavailable, TestUniverse,
    TestWarning,
};

/// The directories a test works in, as the page passes them from the
/// running profile: each, when left out, the one the engine reads on this
/// host.
#[derive(Default, Clone)]
pub(crate) struct TestDirs {
    pub dir: Option<String>,
    pub venues_dir: Option<String>,
}

/// The controls a test asks for, as they come in: a colour not yet read,
/// and numbers not yet checked. One left out is not touched.
#[derive(Default, Clone)]
pub(crate) struct ControlInput {
    /// `#rrggbb`.
    pub color: Option<String>,
    /// 0..1.
    pub dimmer: Option<f64>,
    /// `Some(None)` is off, `Some(Some(hz))` a rate.
    pub strobe: Option<Option<f64>>,
    /// Degrees.
    pub pan: Option<f64>,
    pub tilt: Option<f64>,
    /// Further channel levels by channel name, 0..1.
    pub levels: BTreeMap<String, f64>,
}

/// A test to send.
#[derive(Clone)]
pub(crate) struct TestInput {
    pub fixture_type: String,
    pub mode: Option<String>,
    pub universe: u16,
    pub address: u16,
    pub controls: ControlInput,
    /// Raw channel values, by offset or channel name.
    pub raw: BTreeMap<String, u32>,
}

/// A refusal of a test: the status the web API gave it, why in a word
/// (`reason`) and in a sentence (`error`).
fn refuse(status: StatusCode, reason: impl serde::Serialize, error: impl Into<String>) -> Answer {
    Answer {
        status,
        body: json!({"error": error.into(), "reason": reason}),
    }
}

/// Why testing is not possible right now, if it is not.
async fn unavailable(player: &Player) -> Option<(TestUnavailable, String)> {
    if player.is_locked() {
        return Some((
            TestUnavailable::Locked,
            "The player is locked. Unlock it to test a fixture.".to_string(),
        ));
    }
    if player.is_playing().await {
        return Some((
            TestUnavailable::Playing,
            "A song is playing. Stop it to test a fixture.".to_string(),
        ));
    }
    let Some(engine) = player.dmx_engine() else {
        return Some((
            TestUnavailable::NoDmx,
            "The running profile has no DMX output, so there is nothing to send to.".to_string(),
        ));
    };
    if engine.configured_universes().is_empty() {
        return Some((
            TestUnavailable::NoUniverses,
            "The running profile's DMX section names no universes.".to_string(),
        ));
    }
    None
}

/// The project root and the fixture types and venues directories a test
/// request reads.
fn test_dirs(
    config_path: &std::path::Path,
    query: &TestDirs,
) -> Result<(PathBuf, PathBuf, PathBuf), Answer> {
    let types_dir =
        resolve_lighting_dir(config_path, query.dir.as_deref(), LightingDir::FixtureTypes)?;
    let venues_dir = resolve_lighting_dir(
        config_path,
        query.venues_dir.as_deref(),
        LightingDir::Venues,
    )?;
    let root = canonical_project_root(&project_root(config_path)?)?;
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

/// The colour `#rrggbb` (the `#` optional) reads as.
fn parse_color(text: &str) -> Result<[u8; 3], String> {
    let invalid = || "color must be \"#rrggbb\"".to_string();
    let hex = text.strip_prefix('#').unwrap_or(text);
    if hex.len() != 6 || !hex.is_char_boundary(2) || !hex.is_char_boundary(4) {
        return Err(invalid());
    }
    let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| invalid());
    Ok([byte(0)?, byte(2)?, byte(4)?])
}

/// Reads the request's controls: `color` as `#rrggbb`, `dimmer` 0..1,
/// `strobe` Hz or off, `pan`/`tilt` degrees, any other name a channel level
/// 0..1. A number that is not finite is refused by the control's name.
fn controls_from(input: &ControlInput) -> Result<TestControls, String> {
    let finite = |key: &str, value: f64| {
        Some(value)
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("control \"{key}\" must be a number"))
    };
    let mut controls = TestControls::default();
    if let Some(color) = &input.color {
        controls.color = Some(parse_color(color)?);
    }
    if let Some(dimmer) = input.dimmer {
        controls.dimmer = Some(finite("dimmer", dimmer)?.clamp(0.0, 1.0));
    }
    if let Some(strobe) = input.strobe {
        controls.strobe = Some(match strobe {
            None => None,
            Some(hz) => Some(finite("strobe", hz)?.max(0.0)),
        });
    }
    if let Some(pan) = input.pan {
        controls.pan = Some(finite("pan", pan)?);
    }
    if let Some(tilt) = input.tilt {
        controls.tilt = Some(finite("tilt", tilt)?);
    }
    for (name, level) in &input.levels {
        controls
            .levels
            .insert(name.clone(), finite(name, *level)?.clamp(0.0, 1.0));
    }
    Ok(controls)
}

/// Reads raw values: keys are 1-based offsets or channel names.
fn raw_from(
    raw: &BTreeMap<String, u32>,
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
        let byte = u8::try_from(*value)
            .map_err(|_| format!("raw value for \"{key}\" must be 0 to 255"))?;
        out.insert(offset, byte);
    }
    Ok(out)
}

/// The current venue's fixtures the test's span runs into, by name, on
/// the addresses a universe has: past 512 nothing is sent.
fn venue_overlaps(player: &Player, test: &PatchSpan) -> Vec<String> {
    let Some(system) = player.broadcast_handles().and_then(|h| h.lighting_system) else {
        return Vec::new();
    };
    let system = system.lock();
    let Some(venue) = system.get_current_venue() else {
        return Vec::new();
    };
    let mut names: Vec<String> = system
        .patch_spans(venue)
        .into_iter()
        .filter(|span| {
            test.intersect(span)
                .is_some_and(|(from, _)| from <= patch::UNIVERSE_SIZE)
        })
        .map(|span| span.fixture)
        .collect();
    names.sort();
    names
}

/// The profile's universes, with olad's patch state for each (null when
/// olad was not reachable).
fn universes_with_olad(engine: &crate::dmx::engine::Engine) -> (Vec<TestUniverse>, Option<bool>) {
    let configured = engine.configured_universes();
    let report = engine.olad().probe(&configured);
    let reachable = report.reachable;
    let names = engine.universe_names();
    let universes = configured
        .iter()
        .map(|u| TestUniverse {
            universe: *u,
            name: names.get(u).cloned(),
            patched: reachable.then(|| !report.unpatched.contains(u)),
        })
        .collect();
    (universes, Some(reachable))
}

/// What the test panel needs for the fixture type `type_name`: the type's
/// modes, each with the controls it supports and its channels; the running
/// profile's universes with olad's patch state; and whether a test can run
/// now. 404 `type` for a name no fixture type has.
pub(crate) async fn test_options(
    player: &Player,
    config_path: &std::path::Path,
    type_name: &str,
    query: &TestDirs,
) -> Result<TestOptions, Answer> {
    if type_name.is_empty() {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "type",
            "fixture_type is required",
        ));
    }
    let type_name = type_name.to_string();
    let (root, types_dir, venues_dir) = test_dirs(config_path, query)?;
    let why_not = unavailable(player).await;
    let engine = player.dmx_engine();
    let name = type_name.clone();
    let outcome = tokio::task::spawn_blocking(move || {
        let system = system_from_files(&root, &types_dir, &venues_dir)?;
        let gdtf = system.gdtf_archive(&name).map(str::to_string);
        let mode_names: Vec<Option<String>> = match &gdtf {
            Some(archive) => {
                let bytes =
                    crate::util::read_file(&root.join(archive)).map_err(|e| e.to_string())?;
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
        let modes: Vec<TestMode> = mode_names
            .into_iter()
            .map(
                |mode| match system.type_in_mode(&name, mode.as_deref(), &root) {
                    Ok(fixture_type) => {
                        let info =
                            lighting::system::fixture_info_for(&name, 1, 1, &name, &fixture_type);
                        TestMode {
                            name: mode,
                            drivable: true,
                            error: None,
                            footprint: Some(fixture_test::footprint(&info)),
                            controls: Some(fixture_test::control_support(&info)),
                            channels: fixture_test::channel_names(&info)
                                .into_iter()
                                .map(|(offset, name)| TestChannel { offset, name })
                                .collect(),
                        }
                    }
                    Err(e) => TestMode {
                        name: mode,
                        drivable: false,
                        error: Some(e.to_string()),
                        footprint: None,
                        controls: None,
                        channels: Vec::new(),
                    },
                },
            )
            .collect();
        let default_mode = modes
            .iter()
            .find(|m| m.drivable)
            .and_then(|m| m.name.clone());
        let (universes, reachable) = match &engine {
            Some(engine) => universes_with_olad(engine),
            None => (Vec::new(), None),
        };
        let olad_port = engine.as_ref().map(|e| e.ola_http_port());
        Ok::<_, String>(Some(TestOptions {
            fixture_type: name,
            gdtf: gdtf.is_some(),
            modes,
            default_mode,
            universes,
            olad: TestOlad {
                reachable,
                port: olad_port,
            },
            available: why_not.is_none(),
            unavailable: why_not.as_ref().map(|(r, _)| *r),
            unavailable_message: why_not.map(|(_, m)| m),
        }))
    })
    .await;
    match outcome {
        Ok(Ok(Some(options))) => Ok(options),
        Ok(Ok(None)) => Err(refuse(
            StatusCode::NOT_FOUND,
            "type",
            format!("no fixture type named \"{type_name}\""),
        )),
        Ok(Err(e)) => Err(refuse(StatusCode::INTERNAL_SERVER_ERROR, "files", e)),
        Err(e) => Err(refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "files",
            e.to_string(),
        )),
    }
}

/// Starts or updates a test: resolves the controls (and raw values over
/// them) for the fixture at the given universe and address, lays them over
/// the engine's output, and answers the frame, how long it holds, and
/// anything worth knowing.
pub(crate) async fn start_test(
    player: &Player,
    config_path: &std::path::Path,
    query: &TestDirs,
    request: TestInput,
) -> Result<TestAnswer, Answer> {
    if let Some((reason, message)) = unavailable(player).await {
        let status = if reason == TestUnavailable::Locked {
            StatusCode::LOCKED
        } else {
            StatusCode::CONFLICT
        };
        return Err(refuse(status, reason, message));
    }
    let Some(engine) = player.dmx_engine() else {
        return Err(refuse(StatusCode::CONFLICT, "no_dmx", "no DMX engine"));
    };
    if !engine.configured_universes().contains(&request.universe) {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "universe",
            format!(
                "universe {} is not one of the running profile's DMX universes",
                request.universe
            ),
        ));
    }
    if !(1..=512).contains(&request.address) {
        return Err(refuse(
            StatusCode::BAD_REQUEST,
            "address",
            "the address must be 1 to 512",
        ));
    }
    let controls = controls_from(&request.controls)
        .map_err(|e| refuse(StatusCode::BAD_REQUEST, "controls", e))?;
    let (root, types_dir, venues_dir) = test_dirs(config_path, query)?;
    let (type_name, mode) = (request.fixture_type.clone(), request.mode.clone());
    let prepared = tokio::task::spawn_blocking(move || {
        prepare(&root, &types_dir, &venues_dir, &type_name, mode.as_deref())
    })
    .await;
    let prepared = match prepared {
        Ok(Ok(p)) => p,
        Ok(Err(TypeInModeError::Unknown(name))) => {
            return Err(refuse(
                StatusCode::NOT_FOUND,
                "type",
                format!("no fixture type named \"{name}\""),
            ))
        }
        Ok(Err(TypeInModeError::Mode(why))) => {
            return Err(refuse(StatusCode::BAD_REQUEST, "mode", why))
        }
        Err(e) => {
            return Err(refuse(
                StatusCode::INTERNAL_SERVER_ERROR,
                "files",
                e.to_string(),
            ))
        }
    };

    let info = lighting::system::fixture_info_for(
        "test",
        request.universe,
        request.address,
        &request.fixture_type,
        &prepared.fixture_type,
    );
    let names = fixture_test::channel_names(&info);
    let raw =
        raw_from(&request.raw, &names).map_err(|e| refuse(StatusCode::BAD_REQUEST, "raw", e))?;
    let frame = fixture_test::resolve_frame(&info, &controls, &raw);
    let footprint = fixture_test::footprint(&info);
    let channels: Vec<(u16, u8)> = frame
        .iter()
        .filter(|c| c.address <= 512)
        .map(|c| (c.address, c.value))
        .collect();
    let expiry = engine
        .set_test_output(crate::dmx::engine::TestOutput {
            universe: request.universe,
            address: request.address,
            footprint,
            fixture_type: request.fixture_type.clone(),
            mode: request.mode.clone(),
            channels,
        })
        .map_err(|e| refuse(StatusCode::BAD_REQUEST, "universe", e))?;

    let warnings = warnings(
        player,
        &engine,
        &request,
        footprint,
        &fixture_test::unsupported(&fixture_test::control_support(&info), &controls),
    )
    .await;
    Ok(TestAnswer {
        active: true,
        universe: request.universe,
        address: request.address,
        footprint,
        fixture_type: request.fixture_type,
        mode: request.mode,
        expires_in_secs: expiry.as_secs_f64(),
        frame,
        warnings,
    })
}

/// What a test should be told: fixtures of the current venue it is lit
/// over, a span past 512, olad not answering or not patched for the
/// universe, and controls the mode has no channel for.
async fn warnings(
    player: &Player,
    engine: &Arc<crate::dmx::engine::Engine>,
    request: &TestInput,
    footprint: u16,
    unsupported: &[String],
) -> Vec<TestWarning> {
    let mut out = Vec::new();
    let span = PatchSpan {
        fixture: request.fixture_type.clone(),
        universe: request.universe,
        address: request.address,
        footprint,
    };
    let overlaps = venue_overlaps(player, &span);
    if !overlaps.is_empty() {
        out.push(TestWarning::VenueOverlap {
            message: format!(
                "These addresses belong to {} in the current venue; the test overrides {} while it runs.",
                overlaps.join(", "),
                if overlaps.len() == 1 { "it" } else { "them" },
            ),
            fixtures: overlaps,
        });
    }
    for overrun in patch::venue_overruns(std::slice::from_ref(&span)) {
        out.push(TestWarning::PastUniverseEnd {
            message: format!(
                "This fixture needs addresses {}–{}, past the end of the universe (512); the channels past 512 are not sent.",
                overrun.address, overrun.last
            ),
        });
    }
    let olad = engine.olad().clone();
    let port = olad.http_port();
    let universe = request.universe;
    let report = tokio::task::spawn_blocking(move || olad.probe(&[universe]))
        .await
        .ok();
    if let Some(report) = report {
        if !report.reachable {
            out.push(TestWarning::OladUnreachable {
                message: format!(
                    "mtrack cannot reach olad (the DMX service) on port {port}, so nothing reaches the lights."
                ),
            });
        } else if report.unpatched.contains(&universe) {
            out.push(TestWarning::UniverseUnpatched {
                message: format!(
                    "olad has no output patched to universe {universe}: mtrack sends, but nothing leaves the computer."
                ),
            });
        }
    }
    for control in unsupported {
        out.push(TestWarning::UnsupportedControl {
            control: control.clone(),
            message: format!("This mode has no channel for {control}."),
        });
    }
    out
}

/// Releases the test output now.
pub(crate) fn stop_test(player: &Player) {
    if let Some(engine) = player.dmx_engine() {
        engine.release_test_output();
    }
}
