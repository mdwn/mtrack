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

//! The fixture test calls (`StartFixtureTest`, `StopFixtureTest`,
//! `GetFixtureTestOptions`), called as the service.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::sync::Arc;

use serde_json::{json, Value};
use tonic::{Code, Request, Status};

use super::{json as read, serve, Project};
use crate::controller::grpc::lighting_service::REFUSAL_REASON_KEY;
use crate::proto::player::v1::{self as proto, lighting_service_server::LightingService};
use crate::testutil::net::ClosedPort;
use crate::webui::api::test_helpers::{
    test_state, test_state_with_audio_device, test_state_with_dmx,
};

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

fn write_project(root: &Path, venue: Option<&str>) {
    std::fs::create_dir_all(root.join("lighting/fixture_types")).unwrap();
    std::fs::create_dir_all(root.join("lighting/venues")).unwrap();
    std::fs::create_dir_all(root.join("lighting/library")).unwrap();
    std::fs::write(root.join("lighting/fixture_types/par.light"), PAR).unwrap();
    std::fs::write(
        root.join("lighting/library/synth.gdtf"),
        crate::testutil::lighting::synthetic_gdtf(),
    )
    .unwrap();
    if let Some(venue) = venue {
        std::fs::write(root.join("lighting/venues/house.light"), venue).unwrap();
    }
}

/// A project whose player runs a DMX engine on universe 1 (olad's web server
/// at `olad_port`), with the current venue `venue` when given.
fn dmx_project(
    olad_port: u16,
    venue: Option<&str>,
) -> (Project, tempfile::TempDir, Arc<crate::dmx::engine::Engine>) {
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
    let dmx: crate::config::Dmx = ::config::Config::builder()
        .add_source(::config::File::from_str(&yaml, ::config::FileFormat::Yaml))
        .build()
        .unwrap()
        .try_deserialize()
        .unwrap();
    let mut engine = crate::dmx::engine::Engine::new(
        &dmx,
        dmx.lighting(),
        Some(engine_project.path()),
        crate::dmx::ola_client::OlaClientFactory::create_mock_client(),
    )
    .unwrap();
    // The fake olad answers when its thread gets to it; on a loaded runner,
    // that can be past a probe's usual limits.
    engine.set_olad_limits(crate::dmx::patch_check::ProbeLimits::PATIENT);
    let engine = Arc::new(engine);
    let songs = Arc::new(crate::songs::Songs::new(std::collections::HashMap::new()));
    let (state, dir) = test_state_with_dmx(songs, engine.clone());
    write_project(dir.path(), venue);
    (serve(state, dir), engine_project, engine)
}

/// A test request as the page sends it: `controls` is a JSON object of the
/// controls it holds (`color`, `dimmer`, `strobe`, `pan`, `tilt`, and any
/// other name a level), `raw` one of raw values.
fn request(body: &Value) -> proto::StartFixtureTestRequest {
    let controls = &body["controls"];
    proto::StartFixtureTestRequest {
        fixture_type: body["fixture_type"].as_str().unwrap().to_string(),
        mode: body["mode"].as_str().map(str::to_string),
        universe: body["universe"].as_u64().unwrap() as u32,
        address: body["address"].as_u64().unwrap() as u32,
        controls: Some(proto::FixtureTestControls {
            color: controls["color"].as_str().map(str::to_string),
            dimmer: controls["dimmer"].as_f64(),
            strobe: controls
                .get("strobe")
                .map(|s| proto::FixtureTestStrobe { hertz: s.as_f64() }),
            pan: controls["pan"].as_f64(),
            tilt: controls["tilt"].as_f64(),
            levels: Default::default(),
        }),
        raw: body["raw"]
            .as_object()
            .map(|raw| {
                raw.iter()
                    .map(|(k, v)| (k.clone(), v.as_u64().unwrap() as u32))
                    .collect()
            })
            .unwrap_or_default(),
        dir: None,
        venues_dir: None,
    }
}

fn red_par() -> Value {
    json!({
        "fixture_type": "Par", "mode": null, "universe": 1, "address": 10,
        "controls": {"color": "#ff0000", "dimmer": 1},
    })
}

async fn start(p: &Project, body: &Value) -> Result<Value, Status> {
    start_request(p, request(body)).await
}

async fn start_request(p: &Project, req: proto::StartFixtureTestRequest) -> Result<Value, Status> {
    p.server
        .start_fixture_test(Request::new(req))
        .await
        .map(|r| read::test_answer(&r.into_inner()))
}

async fn stop(p: &Project) -> proto::StopFixtureTestResponse {
    p.server
        .stop_fixture_test(Request::new(proto::StopFixtureTestRequest {}))
        .await
        .unwrap()
        .into_inner()
}

async fn options(p: &Project, fixture_type: &str) -> Result<Value, Status> {
    p.server
        .get_fixture_test_options(Request::new(proto::GetFixtureTestOptionsRequest {
            fixture_type: fixture_type.to_string(),
            dir: None,
            venues_dir: None,
        }))
        .await
        .map(|r| read::test_options(&r.into_inner()))
}

/// The refusal's reason, in its metadata.
fn reason(status: &Status) -> Option<&str> {
    status
        .metadata()
        .get(REFUSAL_REASON_KEY)
        .map(|v| v.to_str().unwrap())
}

#[tokio::test]
async fn a_test_resolves_the_frame_lays_it_over_the_output_and_releases() {
    let closed = ClosedPort::bind();
    let (p, _engine_project, engine) = dmx_project(closed.port(), None);
    let body = start(&p, &red_par()).await.unwrap();
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
    let hardware = p.state.player.hardware_status();
    assert_eq!(hardware.test_output.expect("a live test").address, 10);

    // A raw value wins over its channel, by offset or by name.
    let mut raw = red_par();
    raw["raw"] = json!({"3": 42, "blue": 7});
    let body = start(&p, &raw).await.unwrap();
    assert_eq!(body["frame"][2]["value"], 42);
    assert_eq!(body["frame"][2]["raw"], true);
    assert_eq!(body["frame"][3]["value"], 7);

    assert!(!stop(&p).await.active);
    assert!(engine.test_output_status().is_none());
    assert!(p.state.player.hardware_status().test_output.is_none());
}

#[tokio::test]
async fn a_strobe_and_a_level_are_controls_a_test_sends() {
    let closed = ClosedPort::bind();
    let (p, _engine_project, _engine) = dmx_project(closed.port(), None);
    // A strobe with no rate is off; the mode has no strobe channel, so the
    // test says so; a level the mode has no channel for is said too.
    let mut req = request(&red_par());
    req.controls = Some(proto::FixtureTestControls {
        strobe: Some(proto::FixtureTestStrobe { hertz: Some(4.0) }),
        levels: [("amber".to_string(), 0.5)].into(),
        ..Default::default()
    });
    let body = start_request(&p, req).await.unwrap();
    let mut unsupported: Vec<&str> = body["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|w| w["kind"] == "unsupported_control")
        .map(|w| w["control"].as_str().unwrap())
        .collect();
    unsupported.sort();
    assert_eq!(unsupported, vec!["amber", "strobe"]);

    let mut off = request(&red_par());
    off.controls = Some(proto::FixtureTestControls {
        strobe: Some(proto::FixtureTestStrobe { hertz: None }),
        ..Default::default()
    });
    let body = start_request(&p, off).await.unwrap();
    assert!(
        body["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["kind"] != "unsupported_control"),
        "an off strobe asks nothing of the mode: {body}"
    );
}

#[tokio::test]
async fn a_gdtf_fixture_in_a_mode_and_what_the_mode_cannot_do() {
    let closed = ClosedPort::bind();
    let (p, _engine_project, _engine) = dmx_project(closed.port(), None);
    let body = json!({
        "fixture_type": "Synth Brick", "mode": "8: RGBS", "universe": 1, "address": 1,
        "controls": {"color": "#ffffff", "dimmer": 1, "pan": 10},
    });
    let body = start(&p, &body).await.unwrap();
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
    let closed = ClosedPort::bind();
    let (p, _engine_project, engine) = dmx_project(closed.port(), None);
    let with = |key: &str, value: Value| {
        let mut b = red_par();
        b[key] = value;
        b
    };
    let refused = |status: Status, code: Code, why: &str| {
        assert_eq!(
            (status.code(), reason(&status)),
            (code, Some(why)),
            "{status:?}"
        );
        status
    };

    let status = start(&p, &with("universe", json!(9))).await.unwrap_err();
    refused(status, Code::InvalidArgument, "universe");
    // A universe past what a number of the DMX holds is no universe either.
    let mut huge = request(&red_par());
    huge.universe = 70_000;
    let status = start_request(&p, huge).await.unwrap_err();
    refused(status, Code::InvalidArgument, "universe");
    let status = start(&p, &with("fixture_type", json!("Nope")))
        .await
        .unwrap_err();
    refused(status, Code::NotFound, "type");
    let status = start(&p, &with("mode", json!("8: RGBS")))
        .await
        .unwrap_err();
    let status = refused(status, Code::InvalidArgument, "mode");
    assert!(status.message().contains("no modes"), "{status:?}");
    let gdtf = json!({"fixture_type": "Synth Brick", "mode": null, "universe": 1, "address": 1});
    let status = start(&p, &gdtf).await.unwrap_err();
    let status = refused(status, Code::InvalidArgument, "mode");
    assert!(
        status.message().contains("choose one of its modes"),
        "{status:?}"
    );
    let status = start(&p, &with("address", json!(0))).await.unwrap_err();
    refused(status, Code::InvalidArgument, "address");
    let mut huge = request(&red_par());
    huge.address = 70_000;
    let status = start_request(&p, huge).await.unwrap_err();
    refused(status, Code::InvalidArgument, "address");
    let mut bad = red_par();
    bad["controls"]["color"] = json!("red");
    let status = start(&p, &bad).await.unwrap_err();
    refused(status, Code::InvalidArgument, "controls");
    // A number that is not one is refused by the control's name.
    let mut nan = request(&red_par());
    nan.controls.as_mut().unwrap().dimmer = Some(f64::NAN);
    let status = start_request(&p, nan).await.unwrap_err();
    let status = refused(status, Code::InvalidArgument, "controls");
    assert!(
        status.message().contains("\"dimmer\" must be a number"),
        "{status:?}"
    );
    // A raw value past a byte.
    let mut raw = request(&red_par());
    raw.raw.insert("1".to_string(), 256);
    let status = start_request(&p, raw).await.unwrap_err();
    refused(status, Code::InvalidArgument, "raw");
    assert!(
        engine.test_output_status().is_none(),
        "nothing refused was sent"
    );

    // Locked: refused, and locking releases a running test.
    start(&p, &red_par()).await.unwrap();
    p.state.player.set_locked(true);
    assert!(
        engine.test_output_status().is_none(),
        "the lock released it"
    );
    let status = start(&p, &red_par()).await.unwrap_err();
    refused(status, Code::FailedPrecondition, "locked");
    // Releasing stays open while locked.
    assert!(!stop(&p).await.active);
    let options = options(&p, "Par").await.unwrap();
    assert_eq!(options["unavailable"], "locked");
    assert_eq!(options["available"], false);
}

#[tokio::test]
async fn without_dmx_or_while_playing_there_is_nothing_to_test() {
    let (state, dir) = test_state();
    write_project(dir.path(), None);
    let p = serve(state, dir);
    let status = start(&p, &red_par()).await.unwrap_err();
    assert_eq!(
        (status.code(), reason(&status)),
        (Code::FailedPrecondition, Some("no_dmx"))
    );
    let options = options(&p, "Par").await.unwrap();
    assert_eq!(options["unavailable"], "no_dmx");
    assert_eq!(options["universes"], json!([]));

    // A song playing comes first: stop it, then there is the DMX to set up.
    let (state, dir) = test_state_with_audio_device("mock-device");
    write_project(dir.path(), None);
    let p = serve(state, dir);
    p.state.player.play().await.unwrap();
    assert!(p.state.player.is_playing().await);
    let status = start(&p, &red_par()).await.unwrap_err();
    assert_eq!(
        (status.code(), reason(&status)),
        (Code::FailedPrecondition, Some("playing"))
    );
    p.state.player.stop().await;
}

#[tokio::test]
async fn the_warnings_name_the_venue_fixtures_and_the_unpatched_universe() {
    let olad = fake_olad(r#"{"id":1,"name":"Universe 1","input_ports":[],"output_ports":[]}"#);
    // "Edge" sits past 512, where the test at 510 runs on into, but nothing
    // is sent there: it is not lit over.
    let venue = "venue \"house\" {\n  fixture \"Front\" Par @ 1:11\n  \
        fixture \"Back\" Par @ 1:30\n  fixture \"Edge\" Par @ 1:513\n}\n";
    let (p, _engine_project, _engine) = dmx_project(olad, Some(venue));
    let mut body = red_par();
    body["address"] = json!(510);
    let body = start(&p, &body).await.unwrap();
    let kinds: Vec<&str> = body["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, vec!["past_universe_end", "universe_unpatched"]);
    // Only the channels up to 512 are in the override.
    assert_eq!(body["frame"].as_array().unwrap().len(), 4);

    let body = start(&p, &red_par()).await.unwrap();
    let overlap = body["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["kind"] == "venue_overlap")
        .expect("an overlap")
        .clone();
    assert_eq!(overlap["fixtures"], json!(["Front"]));

    let options = options(&p, "Par").await.unwrap();
    assert_eq!(options["universes"][0]["patched"], false);
    assert_eq!(options["olad"]["reachable"], true);
}

#[tokio::test]
async fn a_type_with_no_channels_lights_no_venue_fixture() {
    let venue = "venue \"house\" {\n  fixture \"Front\" Par @ 1:11\n}\n";
    let closed = ClosedPort::bind();
    let (p, _engine_project, _engine) = dmx_project(closed.port(), Some(venue));
    std::fs::write(
        p.dir.path().join("lighting/fixture_types/blank.light"),
        "fixture_type \"Blank\" {\n}\n",
    )
    .unwrap();
    let mut body = red_par();
    body["fixture_type"] = json!("Blank");
    body["address"] = json!(11);
    let body = start(&p, &body).await.unwrap();
    assert_eq!(body["footprint"], 0);
    assert!(
        body["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .all(|w| w["kind"] != "venue_overlap"),
        "{body}"
    );
}

#[tokio::test]
async fn the_options_describe_each_mode() {
    let closed = ClosedPort::bind();
    let (p, _engine_project, _engine) = dmx_project(closed.port(), None);
    let options_of = |name: &'static str| options(&p, name);
    let options = options_of("Synth Brick").await.unwrap();
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

    let native = options_of("Par").await.unwrap();
    assert_eq!(native["gdtf"], false);
    assert_eq!(native["modes"].as_array().unwrap().len(), 1);
    assert!(native["modes"][0]["name"].is_null());
    assert_eq!(native["modes"][0]["controls"]["dimmer"], "channel");
    let status = options_of("Nope").await.unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
    assert_eq!(reason(&status), Some("type"));
    // A request for no type at all names none.
    let status = options_of("").await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
}

/// The directories a page names reach the plain functions, and stay inside
/// the project.
#[tokio::test]
async fn a_directory_that_leaves_the_project_is_refused() {
    let closed = ClosedPort::bind();
    let (p, _engine_project, _engine) = dmx_project(closed.port(), None);
    // A link in the project that leads out of it.
    let away = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(away.path(), p.dir.path().join("escape")).unwrap();
    for dir in ["../elsewhere", "/etc", "escape"] {
        let status = p
            .server
            .get_fixture_test_options(Request::new(proto::GetFixtureTestOptionsRequest {
                fixture_type: "Par".to_string(),
                dir: Some(dir.to_string()),
                venues_dir: None,
            }))
            .await
            .unwrap_err();
        assert!(
            matches!(
                status.code(),
                Code::InvalidArgument | Code::PermissionDenied | Code::NotFound
            ),
            "{dir}: {status:?}"
        );
        let mut req = request(&red_par());
        req.venues_dir = Some(dir.to_string());
        let status = p
            .server
            .start_fixture_test(Request::new(req))
            .await
            .unwrap_err();
        assert!(
            matches!(
                status.code(),
                Code::InvalidArgument | Code::PermissionDenied | Code::NotFound
            ),
            "{dir}: {status:?}"
        );
    }
}
