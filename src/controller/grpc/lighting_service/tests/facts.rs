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

//! `GetLightingReadiness`, `GetLightingFit` and `EvaluateSongLighting`,
//! called as the service. Each answer is read back as the JSON the REST
//! routes sent ([`super::json`]), so what those routes said is asserted as
//! it was.

use serde_json::{json, Value};
use tonic::{Code, Request, Status};

use super::{json as read, serve, Project};
use crate::proto::player::v1::{self as proto, lighting_service_server::LightingService};
use crate::webui::api::test_helpers::{test_state_with_dmx, test_state_with_registry};

pub(super) const TYPE_PAR: &str =
    "fixture_type \"Par\" {\n  channels: 2\n  channel_map: { \"dimmer\": 1, \"red\": 2 }\n}\n";

/// A project on disk plus a player running a DMX engine over it.
pub(super) struct Rig {
    pub p: Project,
    _project: tempfile::TempDir,
    /// The port olad's web server is looked for on, where nothing listens.
    _olad: crate::testutil::net::ClosedPort,
}

/// Writes one song directory per (name, show file body) and loads them.
pub(super) fn song_registry(
    root: &std::path::Path,
    songs: &[(&str, &str)],
) -> std::sync::Arc<crate::songs::Songs> {
    let songs_dir = root.join("songs");
    std::fs::create_dir_all(&songs_dir).unwrap();
    for (name, show) in songs {
        let dir = songs_dir.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        crate::testutil::write_wav(dir.join("kick.wav"), vec![vec![0i32; 4410]], 44100).unwrap();
        std::fs::write(dir.join("show.light"), show).unwrap();
        std::fs::write(
            dir.join("song.yaml"),
            format!(
                "kind: song\nname: {name}\ntracks:\n  - name: kick\n    file: kick.wav\n\
                 lighting:\n  - file: show.light\n"
            ),
        )
        .unwrap();
    }
    crate::songs::get_all_songs(&songs_dir).unwrap()
}

/// `venue` is the venue file's body (`None` for no current venue), `songs`
/// are (directory, show file body) pairs, `universes` the configured
/// outputs. olad's web server is pointed at a port nothing listens on so the
/// probe never meets a real daemon.
pub(super) fn rig(
    types: &[(&str, &str)],
    venue: Option<&str>,
    songs: &[(&str, &str)],
    universes: &[u16],
) -> Rig {
    rig_with_library(types, venue, songs, universes, &[])
}

/// [`rig`], with these archives in `lighting/library/` before it loads.
pub(super) fn rig_with_library(
    types: &[(&str, &str)],
    venue: Option<&str>,
    songs: &[(&str, &str)],
    universes: &[u16],
    library: &[(&str, &[u8])],
) -> Rig {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    std::fs::create_dir_all(root.join("lighting/library")).unwrap();
    for (file, bytes) in library {
        std::fs::write(root.join("lighting/library").join(file), bytes).unwrap();
    }
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::create_dir_all(root.join("venues")).unwrap();
    for (file, body) in types {
        std::fs::write(root.join("types").join(file), body).unwrap();
    }
    if let Some(venue) = venue {
        std::fs::write(root.join("venues/v.light"), venue).unwrap();
    }
    let songs = song_registry(root, songs);

    let olad = crate::testutil::net::ClosedPort::bind();
    let dead_port = olad.port();
    let universe_yaml: String = universes
        .iter()
        .map(|u| format!("  - universe: {u}\n    name: u{u}\n"))
        .collect();
    let current_venue = if venue.is_some() {
        "  current_venue: v\n"
    } else {
        ""
    };
    let dmx_yaml = format!(
        "ola_http_port: {dead_port}\nuniverses:\n{universe_yaml}lighting:\n  directories:\n    \
         fixture_types: types\n    venues: venues\n{current_venue}  groups:\n    washes:\n      \
         name: washes\n      constraints:\n        - AllOf: [\"wash\"]\n"
    );
    let dmx: crate::config::Dmx = ::config::Config::builder()
        .add_source(::config::File::from_str(
            &dmx_yaml,
            ::config::FileFormat::Yaml,
        ))
        .build()
        .unwrap()
        .try_deserialize()
        .unwrap();
    let engine = crate::dmx::engine::Engine::new(
        &dmx,
        dmx.lighting(),
        Some(root),
        crate::dmx::ola_client::OlaClientFactory::create_mock_client(),
    )
    .unwrap();
    let (state, state_dir) = test_state_with_dmx(songs, std::sync::Arc::new(engine));
    Rig {
        p: serve(state, state_dir),
        _project: project,
        _olad: olad,
    }
}

async fn readiness(p: &Project) -> Value {
    let answer = p
        .server
        .get_lighting_readiness(Request::new(proto::GetLightingReadinessRequest {}))
        .await
        .unwrap()
        .into_inner();
    read::readiness(&answer)
}

// ---- GetLightingReadiness ----

#[tokio::test]
async fn readiness_without_dmx_has_no_venue_but_still_reports_shows() {
    let project = tempfile::tempdir().unwrap();
    let songs = song_registry(
        project.path(),
        &[
            (
                "Good",
                "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n}\n",
            ),
            // No `duration`: the song does not load, DMX or not.
            (
                "Bad",
                "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\"\n}\n",
            ),
        ],
    );
    let (state, dir) = test_state_with_registry(songs);
    let report = readiness(&serve(state, dir)).await;
    assert_eq!(report["dmx"], json!(false));
    assert_eq!(report["venue"], json!(null));
    assert_eq!(report["groups"], json!([]));
    assert_eq!(report["output"]["olad"], json!(null));
    assert_eq!(report["fixture_types"]["unresolved"], json!([]));

    let shows = report["shows"].as_array().unwrap();
    let bad = shows.iter().find(|s| s["song"] == json!("Bad")).unwrap();
    assert!(bad["error"].as_str().unwrap().contains("duration"));
    let good = shows.iter().find(|s| s["song"] == json!("Good")).unwrap();
    assert!(good.get("error").is_none());
    // Nothing that needs a venue is guessed at without one.
    for w in good["warnings"].as_array().unwrap() {
        assert!(
            !["empty-group", "unconfigured-universe", "capability-gap"]
                .contains(&w["kind"].as_str().unwrap()),
            "{w}"
        );
    }
}

#[tokio::test]
async fn readiness_puts_patch_findings_on_the_venue_not_under_every_song() {
    let rig = rig(
        &[("par.light", TYPE_PAR)],
        // Partly over each other: an overlap. And a gang at 1:10.
        Some(
            "venue \"v\" {\n  fixture \"A\" Par @ 1:1 tags [\"wash\"]\n  \
             fixture \"B\" Par @ 1:2\n  fixture \"G1\" Par @ 1:10\n  \
             fixture \"G2\" Par @ 1:10\n}\n",
        ),
        &[(
            "Esaweg",
            "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n}\n",
        )],
        &[1],
    );
    let report = readiness(&rig.p).await;
    assert!(report["venue_error"].is_null(), "{report}");
    let patch = report["patch_warnings"].as_array().unwrap();
    assert_eq!(patch.len(), 1, "the gang is quiet: {report}");
    assert_eq!(patch[0]["kind"], "patch-overlap");
    assert!(patch[0]["message"].as_str().unwrap().contains("\"A\""));
    for show in report["shows"].as_array().unwrap() {
        assert!(
            show["warnings"]
                .as_array()
                .unwrap()
                .iter()
                .all(|w| w["kind"] != "patch-overlap"),
            "{report}"
        );
    }
}

#[tokio::test]
async fn readiness_says_when_the_venue_will_not_load_and_which_fixture() {
    let rig = rig(
        &[("par.light", TYPE_PAR)],
        // A native type has no modes: this fails the whole venue.
        Some("venue \"v\" {\n  fixture \"A\" Par @ 1:1\n  fixture \"B\" Par mode \"x\" @ 1:5\n}\n"),
        &[],
        &[1],
    );
    let report = readiness(&rig.p).await;
    assert_eq!(report["venue_error"]["venue"], "v", "{report}");
    assert_eq!(report["venue_error"]["fixture"], "B", "{report}");
    assert!(
        report["venue_error"]["reason"]
            .as_str()
            .unwrap()
            .contains("not GDTF-sourced"),
        "{report}"
    );
    // And the fixture is listed as unresolved, with the same reason.
    assert_eq!(report["fixture_types"]["unresolved"][0]["fixture"], "B");
}

#[tokio::test]
async fn readiness_reports_an_untagged_venue_as_groups_with_no_fixtures() {
    let rig = rig(
        &[("par.light", TYPE_PAR)],
        // A is tagged, B is not; a show that targets `washes` finds A and a
        // show that targets `movers` (declared nowhere) finds nothing.
        Some("venue \"v\" {\n  fixture \"A\" Par @ 1:1 tags [\"wash\"]\n  fixture \"B\" Par @ 1:3\n}\n"),
        &[(
            "Esaweg",
            "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n    \
             movers: static color: \"blue\", duration: 2s\n}\n",
        )],
        &[1],
    );
    let report = readiness(&rig.p).await;

    assert_eq!(report["dmx"], json!(true));
    assert_eq!(report["venue"]["name"], json!("v"));
    assert_eq!(report["venue"]["fixtures"], json!(2));
    assert_eq!(report["venue"]["placed"], json!(0));
    assert_eq!(report["venue"]["focus_points"], json!([]));
    assert_eq!(report["fixture_types"]["in_use"], json!(["Par"]));
    assert_eq!(report["fixture_types"]["unresolved"], json!([]));

    let groups = report["groups"].as_array().unwrap();
    let by_name = |name: &str| groups.iter().find(|g| g["name"] == json!(name)).unwrap();
    assert_eq!(by_name("washes")["fixtures"], json!(1));
    assert_eq!(by_name("movers")["fixtures"], json!(0));
    assert_eq!(by_name("movers")["songs"], json!(["Esaweg"]));

    let shows = report["shows"].as_array().unwrap();
    assert_eq!(shows.len(), 1);
    assert_eq!(shows[0]["song"], json!("Esaweg"));
    assert_eq!(shows[0]["files"], json!(["show.light"]));
    assert!(shows[0].get("error").is_none(), "{report}");
    let kinds: Vec<&str> = shows[0]["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|w| w["kind"].as_str())
        .collect();
    assert!(kinds.contains(&"empty-group"), "{kinds:?}");
    // The Groups check carries the empty group, not the song's.
    let checks = report["checks"].as_array().unwrap();
    let groups_check = checks.iter().find(|c| c["key"] == json!("groups")).unwrap();
    assert_eq!(groups_check["state"], json!("attention"), "{report}");
    // The checks carry their summaries, findings and fixes typed.
    assert_eq!(
        groups_check["summary"],
        json!([{"kind": "attention", "count": 1}])
    );
    let finding = &groups_check["findings"][0];
    assert_eq!(finding["what"]["kind"], json!("groupEmpty"), "{report}");
    assert_eq!(finding["what"]["group"], json!("movers"));
    assert_eq!(finding["fix"]["kind"], json!("fit"));
    assert_eq!(finding["fix"]["group"], json!("movers"));

    // Nothing listens where olad's web server should be.
    assert_eq!(report["output"]["universes"], json!([1]));
    assert_eq!(report["output"]["unconfigured"], json!([]));
    assert_eq!(report["output"]["olad"]["reachable"], json!(false));
}

#[tokio::test]
async fn readiness_reports_a_show_that_does_not_parse() {
    let rig = rig(
        &[("par.light", TYPE_PAR)],
        Some("venue \"v\" {\n  fixture \"A\" Par @ 1:1 tags [\"wash\"]\n}\n"),
        &[
            (
                "Good",
                "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n}\n",
            ),
            // No `duration`: the parser refuses it, and the song with it.
            (
                "Bad",
                "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\"\n}\n",
            ),
        ],
        &[1],
    );
    let report = readiness(&rig.p).await;
    let shows = report["shows"].as_array().unwrap();
    let bad = shows.iter().find(|s| s["song"] == json!("Bad")).unwrap();
    let error = bad["error"].as_str().expect("error present");
    assert!(error.contains("duration"), "{error}");
    let good = shows.iter().find(|s| s["song"] == json!("Good")).unwrap();
    assert!(good.get("error").is_none());
}

#[tokio::test]
async fn readiness_lists_a_universe_with_no_output() {
    let rig = rig(
        &[("par.light", TYPE_PAR)],
        Some("venue \"v\" {\n  fixture \"A\" Par @ 1:1\n  fixture \"B\" Par @ 4:1\n}\n"),
        &[(
            "Song",
            "show \"S\" {\n    @00:00.000\n    all: static color: \"red\", duration: 2s\n}\n",
        )],
        &[1],
    );
    let report = readiness(&rig.p).await;
    assert_eq!(report["output"]["universes"], json!([1, 4]));
    assert_eq!(report["output"]["unconfigured"], json!([4]));
    // Output carries it; no song repeats it.
    assert_eq!(report["shows"].as_array().unwrap().len(), 1);
    for show in report["shows"].as_array().unwrap() {
        for w in show["warnings"].as_array().unwrap() {
            assert_ne!(w["kind"], json!("unconfigured-universe"), "{report}");
        }
    }
}

#[tokio::test]
async fn readiness_says_why_a_fixture_type_did_not_load() {
    let rig = rig(
        &[
            ("par.light", TYPE_PAR),
            (
                "ghost.fixture",
                "fixture_type \"Ghost\"\n  from gdtf(\"lighting/library/missing.gdtf\")\n{ }\n",
            ),
        ],
        Some(
            "venue \"v\" {\n  fixture \"A\" Par @ 1:1\n  fixture \"G\" Ghost mode \"M\" @ 1:5\n  \
             fixture \"N\" Nope @ 1:9\n}\n",
        ),
        &[],
        &[1],
    );
    let report = readiness(&rig.p).await;
    assert_eq!(report["fixture_types"]["in_use"], json!(["Par"]));
    let unresolved = report["fixture_types"]["unresolved"].as_array().unwrap();
    assert_eq!(unresolved.len(), 2);
    assert_eq!(unresolved[0]["fixture"], json!("G"));
    assert_eq!(unresolved[0]["type"], json!("Ghost"));
    assert!(
        unresolved[0]["reason"]
            .as_str()
            .unwrap()
            .contains("missing.gdtf"),
        "the loader's own reason: {report}"
    );
    assert_eq!(unresolved[1]["fixture"], json!("N"));
    assert!(unresolved[1]["reason"]
        .as_str()
        .unwrap()
        .contains("no fixture type named 'Nope'"));
}

#[tokio::test]
async fn readiness_lists_the_library_s_name_collisions_and_unreadable_archives() {
    let archive = crate::testutil::lighting::synthetic_gdtf();
    let rig = rig_with_library(
        &[("par.light", TYPE_PAR)],
        Some(
            "venue \"v\" {\n  fixture \"A\" Par @ 1:1\n  \
             fixture \"B\" \"Synth Brick (b)\" mode \"8: RGBS\" @ 1:5\n}\n",
        ),
        &[],
        &[1],
        &[
            ("a.gdtf", archive.as_slice()),
            ("b.gdtf", archive.as_slice()),
            ("junk.gdtf", b"not a zip".as_slice()),
        ],
    );
    let report = readiness(&rig.p).await;
    assert!(report["venue_error"].is_null(), "{report}");
    let library = report["fixture_types"]["library"].as_array().unwrap();
    let kinds: Vec<(&str, &str)> = library
        .iter()
        .map(|f| (f["kind"].as_str().unwrap(), f["file"].as_str().unwrap()))
        .collect();
    assert_eq!(
        kinds,
        [
            ("gdtf-name-collision", "b.gdtf"),
            ("gdtf-unreadable", "junk.gdtf")
        ],
        "{report}"
    );
    // One finding per renamed archive, naming both.
    let collision = library[0]["message"].as_str().unwrap();
    assert!(
        collision.contains("a.gdtf") && collision.contains("\"Synth Brick (b)\""),
        "{collision}"
    );
}

#[tokio::test]
async fn readiness_without_a_current_venue_has_none() {
    let rig = rig(&[("par.light", TYPE_PAR)], None, &[], &[1]);
    let report = readiness(&rig.p).await;
    assert_eq!(report["dmx"], json!(true));
    assert_eq!(report["venue"], json!(null));
    assert_eq!(report["output"]["universes"], json!([]));
    // Every check says what it needs, and the Venue check is blocked on one.
    let checks = report["checks"].as_array().unwrap();
    let venue = checks.iter().find(|c| c["key"] == json!("venue")).unwrap();
    assert_eq!(venue["state"], json!("blocked"), "{report}");
    assert_eq!(venue["findings"][0]["what"], json!({"kind": "noVenue"}));
    assert_eq!(venue["findings"][0]["fix"], json!({"kind": "chooseVenue"}));
}

/// Every message of the answer is read back as the facts the plain function
/// gathered: the proto drops nothing the web UI's types carry.
#[tokio::test]
async fn the_readiness_answer_carries_every_fact() {
    let rig = rig(
        &[("par.light", TYPE_PAR)],
        Some("venue \"v\" {\n  fixture \"A\" Par @ 1:1 tags [\"wash\"]\n  fixture \"B\" Par @ 4:1\n}\n"),
        &[(
            "Song",
            "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n    \
             movers: static color: \"blue\", duration: 2s\n}\n",
        )],
        &[1],
    );
    let facts = crate::webui::api::lighting_api::readiness(rig.p.state.player.clone())
        .await
        .unwrap();
    assert_eq!(
        readiness(&rig.p).await,
        serde_json::to_value(&facts).unwrap()
    );
}

// ---- GetLightingFit ----

async fn fit(p: &Project) -> Value {
    let answer = p
        .server
        .get_lighting_fit(Request::new(proto::GetLightingFitRequest {}))
        .await
        .unwrap()
        .into_inner();
    read::fit(&answer)
}

const TYPE_MOVER: &str = "fixture_type \"Mover\" {\n  channels: 6\n  channel_map: { \"pan\": 1, \"tilt\": 2, \"red\": 3, \"green\": 4, \"blue\": 5, \"dimmer\": 6 }\n}\n";

const TYPE_WASH: &str = "fixture_type \"Wash\" {\n  channels: 4\n  channel_map: { \"red\": 1, \"green\": 2, \"blue\": 3, \"dimmer\": 4 }\n}\n";

#[tokio::test]
async fn fit_without_dmx_has_no_venue_and_no_groups() {
    let project = tempfile::tempdir().unwrap();
    let songs = song_registry(
        project.path(),
        &[(
            "Song",
            "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n}\n",
        )],
    );
    let (state, dir) = test_state_with_registry(songs);
    let report = fit(&serve(state, dir)).await;
    assert_eq!(report["venue"], json!(null));
    assert_eq!(report["groups"], json!([]));
    assert_eq!(report["focus_points_wanted"], json!([]));
}

#[tokio::test]
async fn fit_suggests_tags_for_an_untagged_group_and_lists_wanted_focus_points() {
    // `washes` needs the `wash` tag and is empty. The shows ask it to move
    // and to colour, which only the Mover does.
    let rig = rig(
        &[("par.light", TYPE_PAR), ("mover.light", TYPE_MOVER)],
        Some(
            "venue \"v\" {\n  fixture \"A\" Par @ 1:1\n  fixture \"B\" Par @ 1:3\n  \
             fixture \"M\" Mover @ 1:10 tags [\"other\"]\n  focus \"center\" (0, 3, 0)\n}\n",
        ),
        &[(
            "Esaweg",
            "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n    \
             washes: move focus: \"drummer\", duration: 1s\n    \
             washes: move focus: \"center\", duration: 1s\n}\n",
        )],
        &[1],
    );
    let report = fit(&rig.p).await;

    assert_eq!(report["venue"]["name"], json!("v"));
    assert_eq!(report["venue"]["fixtures"].as_array().unwrap().len(), 3);
    assert_eq!(report["venue"]["focus_points"], json!(["center"]));
    let m = report["venue"]["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == json!("M"))
        .unwrap();
    assert_eq!(m["tags"], json!(["other"]));
    assert_eq!(
        m["capabilities"],
        json!(["color", "pan_tilt", "strobe", "dimmer"])
    );

    let groups = report["groups"].as_array().unwrap();
    assert_eq!(groups.len(), 1);
    let washes = &groups[0];
    assert_eq!(washes["name"], json!("washes"));
    assert_eq!(washes["defined"], json!(true));
    assert_eq!(washes["needs"]["all_of"], json!(["wash"]));
    assert_eq!(washes["fixtures"], json!([]));
    assert_eq!(washes["songs"], json!(["Esaweg"]));
    assert_eq!(washes["wants"], json!(["move", "color"]));
    assert_eq!(washes["suggestion"]["fixtures"], json!(["M"]));
    assert_eq!(washes["suggestion"]["tags"], json!(["wash"]));
    assert_eq!(washes["suggestion"]["reason"]["count"], json!(1));
    assert_eq!(washes["suggestion"]["reason"]["type"], json!("Mover"));
    assert_eq!(
        washes["suggestion"]["reason"]["can"],
        json!(["move", "color"])
    );
    assert_eq!(washes["others"], json!([]));

    assert_eq!(
        report["focus_points_wanted"],
        json!([{"name": "drummer", "songs": ["Esaweg"]}])
    );
}

#[tokio::test]
async fn fit_offers_other_clusters_and_skips_groups_that_need_no_suggestion() {
    let rig = rig(
        &[("par.light", TYPE_PAR), ("mover.light", TYPE_MOVER)],
        Some(
            "venue \"v\" {\n  fixture \"A\" Par @ 1:1\n  fixture \"B\" Par @ 1:3\n  \
             fixture \"M\" Mover @ 1:10\n  fixture \"W\" Par @ 1:20 tags [\"wash\"]\n}\n",
        ),
        &[
            (
                "Untagged",
                "show \"S\" {\n    @00:00.000\n    movers: static color: \"red\", duration: 2s\n}\n",
            ),
            (
                "Tagged",
                "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n}\n",
            ),
        ],
        &[1],
    );
    let report = fit(&rig.p).await;
    let groups = report["groups"].as_array().unwrap();
    let by_name = |n: &str| groups.iter().find(|g| g["name"] == json!(n)).unwrap();
    // `movers` is declared nowhere in this rig: nothing to tag for.
    assert_eq!(by_name("movers")["defined"], json!(false));
    assert_eq!(by_name("movers")["suggestion"], json!(null));
    // `washes` already finds W.
    assert_eq!(by_name("washes")["fixtures"], json!(["W"]));
    assert_eq!(by_name("washes")["suggestion"], json!(null));
}

#[tokio::test]
async fn fit_lists_other_clusters_under_others() {
    // Three colour-capable candidates in two types: Pars win (two of them),
    // the Mover is the alternative.
    let rig = rig(
        &[("wash.light", TYPE_WASH), ("mover.light", TYPE_MOVER)],
        Some(
            "venue \"v\" {\n  fixture \"A\" Wash @ 1:1\n  fixture \"B\" Wash @ 1:7\n  \
             fixture \"M\" Mover @ 1:10\n}\n",
        ),
        &[(
            "Song",
            "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n}\n",
        )],
        &[1],
    );
    let report = fit(&rig.p).await;
    let washes = &report["groups"][0];
    assert_eq!(washes["suggestion"]["fixtures"], json!(["A", "B"]));
    assert_eq!(washes["others"][0]["type"], json!("Mover"));
    assert_eq!(washes["others"][0]["fixtures"], json!(["M"]));
}

#[tokio::test]
async fn fit_says_which_want_nothing_meets() {
    let rig = rig(
        &[("wash.light", TYPE_WASH)],
        Some("venue \"v\" {\n  fixture \"A\" Wash @ 1:1\n}\n"),
        &[(
            "Song",
            "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n    \
             washes: move pan: 45deg, tilt: 0deg, duration: 1s\n}\n",
        )],
        &[1],
    );
    let report = fit(&rig.p).await;
    let washes = &report["groups"][0];
    assert_eq!(washes["suggestion"], json!(null));
    assert_eq!(washes["unmet"], json!(["move"]));
    assert_eq!(washes["unmet_together"], json!(false));
}

#[tokio::test]
async fn fit_lists_unconfigured_universes_and_the_olad_port() {
    let rig = rig(
        &[("par.light", TYPE_PAR)],
        Some("venue \"v\" {\n  fixture \"A\" Par @ 1:1\n  fixture \"B\" Par @ 4:1\n}\n"),
        &[(
            "Song",
            "show \"S\" {\n    @00:00.000\n    all: static color: \"red\", duration: 2s\n}\n",
        )],
        &[1],
    );
    let report = fit(&rig.p).await;
    assert_eq!(report["output"]["unconfigured"], json!([4]));
    // Nothing listens where olad's web server should be.
    assert_eq!(report["output"]["reachable"], json!(false));
    assert_eq!(report["output"]["unpatched"], json!([]));
    assert!(report["output"]["ola_http_port"].as_u64().unwrap() > 0);
}

/// The proto drops nothing the fit report holds.
#[tokio::test]
async fn the_fit_answer_carries_every_fact() {
    let rig = rig(
        &[("par.light", TYPE_PAR), ("mover.light", TYPE_MOVER)],
        Some(
            "venue \"v\" {\n  fixture \"A\" Par @ 1:1\n  \
             fixture \"M\" Mover @ 1:10 position (1, 2, 3) tags [\"other\"]\n  \
             focus \"center\" (0, 3, 0)\n}\n",
        ),
        &[(
            "Esaweg",
            "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\", duration: 2s\n    \
             washes: move focus: \"drummer\", duration: 1s\n}\n",
        )],
        &[1],
    );
    let facts = crate::webui::api::lighting_api::fit(rig.p.state.player.clone())
        .await
        .unwrap();
    assert_eq!(fit(&rig.p).await, serde_json::to_value(&facts).unwrap());
}

// ---- EvaluateSongLighting ----

const PREVIEW_VENUE: &str = "venue \"v\" {\n  \
    fixture \"M\" Mover @ 1:1 tags [\"wash\"] position (0, 3.5, 4) rotation (0, 0, 180)\n  \
    fixture \"P\" Par @ 1:20\n  \
    focus \"center\" (0, 0, 0)\n}\n";

const PREVIEW_SHOW: &str = "show \"S\" {\n    @00:00.000\n    \
    washes: static color: \"red\", duration: 10s\n    \
    @00:01.000\n    washes: move focus: \"center\", duration: 4s\n}\n";

async fn evaluate(p: &Project, song: &str, times: &[f64]) -> Result<Value, Status> {
    p.server
        .evaluate_song_lighting(Request::new(proto::EvaluateSongLightingRequest {
            song: song.to_string(),
            times: times.to_vec(),
        }))
        .await
        .map(|r| read::preview(&r.into_inner()))
}

fn preview_rig() -> Rig {
    rig(
        &[("mover.light", TYPE_MOVER), ("par.light", TYPE_PAR)],
        Some(PREVIEW_VENUE),
        &[("Song", PREVIEW_SHOW)],
        &[1],
    )
}

#[tokio::test]
async fn evaluate_matches_evaluate_show_and_lists_untouched_fixtures() {
    let rig = preview_rig();
    let player = rig.p.state.player.clone();
    let body = evaluate(&rig.p, "Song", &[0.5, 3.0]).await.unwrap();
    let evaluations = body["evaluations"].as_array().unwrap();
    assert_eq!(evaluations.len(), 2);

    // The same shows evaluated directly, with the group resolved by hand.
    let song = player.songs().get("Song").unwrap();
    let shows = crate::lighting::evaluate::registered_shows(&song);
    let system = player
        .dmx_engine()
        .and_then(|dmx| dmx.broadcast_handles().lighting_system)
        .unwrap();
    let (fixtures, focus) = {
        let guard = system.lock();
        (
            guard.get_current_venue_fixtures().unwrap(),
            guard
                .get_current_venue()
                .unwrap()
                .focus_points()
                .iter()
                .map(|(n, p)| (n.clone(), *p))
                .collect::<std::collections::HashMap<_, _>>(),
        )
    };
    let direct = crate::lighting::evaluate::evaluate_show(
        shows,
        &fixtures,
        &focus,
        None,
        &[
            std::time::Duration::from_secs_f64(0.5),
            std::time::Duration::from_secs_f64(3.0),
        ],
        |mut effect| {
            effect.target_fixtures = vec!["M".to_string()];
            effect
        },
    );
    for (got, want) in evaluations.iter().zip(&direct) {
        let maps = crate::webui::state::stage_frame(&want.fixtures, &want.poses);
        let (poses, cells) = (json!(maps.poses), json!(maps.cells));
        assert_eq!(got["fixtures"], json!(maps.fixtures));
        assert_eq!(got["strobe_rates"], json!(maps.strobe_rates));
        // Pan and tilt are exact; the beam's direction and footprint are
        // trigonometry, which two evaluations agree on to the last bit or
        // two, not always the last.
        for (name, want_pose) in poses.as_object().unwrap() {
            let got_pose = &got["poses"][name];
            assert_eq!(got_pose["pan"], want_pose["pan"]);
            assert_eq!(got_pose["tilt"], want_pose["tilt"]);
            for key in ["aim", "floor"] {
                let (g, w) = (got_pose[key].as_array(), want_pose[key].as_array());
                assert_eq!(g.map(|a| a.len()), w.map(|a| a.len()), "{name} {key}");
                for (g, w) in g.into_iter().flatten().zip(w.into_iter().flatten()) {
                    assert!((g.as_f64().unwrap() - w.as_f64().unwrap()).abs() < 1e-9);
                }
            }
        }
        assert_eq!(
            got["poses"].as_object().unwrap().len(),
            poses.as_object().unwrap().len()
        );
        assert_eq!(got["cells"], json!(cells));
        assert_eq!(got["time"], json!(want.time.as_secs_f64()));
    }
    // The move is under way at 3 s: the mover has left rest.
    assert_ne!(
        evaluations[0]["poses"]["M"], evaluations[1]["poses"]["M"],
        "{body}"
    );
    assert_eq!(evaluations[1]["fixtures"]["M"]["red"], json!(255));

    let active = evaluations[1]["active_effects"].as_array().unwrap();
    let kinds: Vec<&str> = active.iter().filter_map(|e| e["kind"].as_str()).collect();
    assert!(
        kinds.contains(&"Static") && kinds.contains(&"Move"),
        "{kinds:?}"
    );
    assert!(active.iter().all(|e| e["groups"] == json!(["washes"])));
    let mv = active.iter().find(|e| e["kind"] == json!("Move")).unwrap();
    assert_eq!(mv["elapsed"], json!(2.0));
    assert_eq!(mv["duration"], json!(4.0));

    // P is in no group the show targets.
    assert_eq!(body["untouched"], json!(["P"]));
}

#[tokio::test]
async fn evaluate_answers_unknown_and_unloadable_songs() {
    let rig = rig(
        &[("mover.light", TYPE_MOVER), ("par.light", TYPE_PAR)],
        Some(PREVIEW_VENUE),
        &[
            ("Song", PREVIEW_SHOW),
            // No `duration`: the show does not parse, so the song does not load.
            (
                "Bad",
                "show \"S\" {\n    @00:00.000\n    washes: static color: \"red\"\n}\n",
            ),
        ],
        &[1],
    );
    let status = evaluate(&rig.p, "Nope", &[0.0]).await.unwrap_err();
    assert_eq!(status.code(), Code::NotFound, "{status:?}");

    let status = evaluate(&rig.p, "Bad", &[0.0]).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
    assert!(status.message().contains("duration"), "{status:?}");

    let status = evaluate(&rig.p, "Song", &[-1.0]).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn evaluate_refuses_times_it_cannot_hold_instead_of_panicking() {
    let rig = preview_rig();
    // Past what a Duration holds: used to panic the handler.
    let status = evaluate(&rig.p, "Song", &[1e300]).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument, "{status:?}");
    assert_eq!(status.message(), "time out of range");
    // Representable but longer than any song.
    let status = evaluate(&rig.p, "Song", &[1e6]).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    // Not a time at all.
    for bad in [f64::NAN, f64::INFINITY, -0.5] {
        let status = evaluate(&rig.p, "Song", &[bad]).await.unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument, "{bad}");
    }
    evaluate(&rig.p, "Song", &[2.5]).await.unwrap();
}

/// The proto drops nothing the preview holds.
#[tokio::test]
async fn the_preview_answer_carries_every_fact() {
    let rig = preview_rig();
    let facts =
        crate::webui::api::lighting_api::evaluate_song(&rig.p.state.player, "Song", &[0.5, 3.0])
            .await
            .unwrap();
    // Effect ids are counted across evaluations, so the two differ by it.
    let without_ids = |mut preview: Value| {
        for evaluation in preview["evaluations"].as_array_mut().unwrap() {
            for effect in evaluation["active_effects"].as_array_mut().unwrap() {
                effect.as_object_mut().unwrap().remove("id");
            }
        }
        preview
    };
    assert_eq!(
        without_ids(evaluate(&rig.p, "Song", &[0.5, 3.0]).await.unwrap()),
        without_ids(serde_json::to_value(&facts).unwrap())
    );
}
