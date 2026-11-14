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

//! A GDTF fixture's settings (`GetFixtureTypeSettings`,
//! `SaveFixtureTypeSettings`), called as the service: the tests of the
//! routes they replaced. A save is posted as the page posted it, a JSON
//! object of `name` (the name the type holds after), `write`, `movement`,
//! `strobe_curve` and `venue_versions`, and its revision as `if_match`.

use std::path::Path as FsPath;

use serde_json::{json, Value};
use tonic::{Code, Request, Status};

use super::{json as read, Project};
use crate::controller::grpc::config_service::CURRENT_VERSION_KEY;
use crate::proto::player::v1::{self as proto, lighting_service_server::LightingService};
use crate::webui::api::lighting_api::content_version;

const TYPE_FILE: &str = "# Imported from synth.gdtf.\n# Keep this.\nfixture_type \"Brick\"\n  \
                         from gdtf(\"library/synth.gdtf\")\n{\n}\n";

impl Project {
    pub async fn settings(&self, name: &str, dir: Option<&str>) -> Result<Value, Status> {
        self.server
            .get_fixture_type_settings(Request::new(proto::GetFixtureTypeSettingsRequest {
                name: name.to_string(),
                dir: dir.map(str::to_string),
            }))
            .await
            .map(|r| read::fixture_settings(&r.into_inner()))
    }

    pub async fn save_settings(
        &self,
        request: proto::SaveFixtureTypeSettingsRequest,
    ) -> Result<proto::FixtureTypeSettingsSaved, Status> {
        self.server
            .save_fixture_type_settings(Request::new(request))
            .await
            .map(|r| r.into_inner())
    }
}

/// A project: the synthetic archive, `Brick` in `types/brick.fixture`,
/// a comment-heavy `.venue` and a `.light` venue using it, and a venue
/// that does not.
fn write_project(root: &FsPath) {
    let library = root.join("library");
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(
        library.join("synth.gdtf"),
        crate::testutil::lighting::synthetic_gdtf(),
    )
    .unwrap();
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::write(root.join("types/brick.fixture"), TYPE_FILE).unwrap();
    std::fs::create_dir_all(root.join("venues")).unwrap();
    std::fs::write(
        root.join("venues/house.venue"),
        "# The house rig.\n# Measured 2026-09.\nvenue \"house\" {\n  # front truss\n  \
         fixture \"B1\" Brick mode \"8: RGBS\" @ 1:1 tags [\"wash\"]  # left\n  \
         fixture \"B2\" Brick mode \"Mover 16bit\" @ 1:40\n  \
         fixture \"P\" Other @ 1:100\n  focus \"center\" (0, 1, 1)\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("venues/club.light"),
        "venue \"club\" {\n  fixture \"C1\" Brick mode \"8: RGBS\" @ 2:1\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("venues/other.light"),
        "venue \"other\" {\n  fixture \"X\" Other @ 1:1\n}\n",
    )
    .unwrap();
}

fn project() -> Project {
    let p = super::project();
    write_project(p.dir.path());
    p
}

fn curve(name: &Value) -> i32 {
    match name.as_str() {
        None => proto::FixtureStrobeCurve::Unspecified as i32,
        Some("period") => proto::FixtureStrobeCurve::Period as i32,
        Some("linear") => proto::FixtureStrobeCurve::Linear as i32,
        Some("declared") => proto::FixtureStrobeCurve::Declared as i32,
        Some(other) => panic!("no curve {other}"),
    }
}

/// The request a page makes for `name`, from the JSON it used to post.
fn request(
    name: &str,
    body: &Value,
    if_match: Option<&str>,
) -> proto::SaveFixtureTypeSettingsRequest {
    let movement = &body["movement"];
    proto::SaveFixtureTypeSettingsRequest {
        name: name.to_string(),
        dir: Some("types".to_string()),
        venues_dir: Some("venues".to_string()),
        expected_revision: if_match.map(str::to_string),
        new_name: body["name"].as_str().unwrap().to_string(),
        movement: Some(proto::FixtureMovementLimits {
            max_pan_speed: movement["max_pan_speed"].as_f64(),
            max_tilt_speed: movement["max_tilt_speed"].as_f64(),
        }),
        strobe_curve: curve(&body["strobe_curve"]),
        write: body["write"].as_bool().unwrap(),
        venue_revisions: body["venue_versions"]
            .as_object()
            .map(|m| {
                m.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string()))
                    .collect()
            })
            .unwrap_or_default(),
    }
}

async fn post(
    p: &Project,
    name: &str,
    body: Value,
    if_match: Option<&str>,
) -> Result<Value, Status> {
    p.save_settings(request(name, &body, if_match))
        .await
        .map(|s| read::settings_saved(&s))
}

fn text(root: &FsPath, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap()
}

#[tokio::test]
async fn the_form_reads_the_settings_and_a_native_type_has_none() {
    let p = project();
    std::fs::write(
        p.dir.path().join("types/par.light"),
        "fixture_type \"Par\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n",
    )
    .unwrap();
    let body = p.settings("Brick", Some("types")).await.unwrap();
    assert_eq!(body["name"], "Brick");
    assert!(body["movement"]["max_pan_speed"].is_null(), "{body}");
    assert_eq!(body["version"], content_version(TYPE_FILE.as_bytes()));
    let status = p.settings("Par", Some("types")).await.unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
    assert_eq!(status.message(), "fixture type \"Par\" is not a GDTF type");
    let status = p.settings("Nothing", Some("types")).await.unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
    assert_eq!(status.message(), "Fixture type not found: Nothing");
}

#[tokio::test]
async fn a_rename_rewrites_every_venue_line_and_keeps_every_comment() {
    let p = project();
    let root = p.dir.path();
    let before_house = text(root, "venues/house.venue");
    let before_other = text(root, "venues/other.light");
    let body = json!({"name": "PixelBrick", "write": false});

    // The plan writes nothing and says what it would touch.
    let plan = post(&p, "Brick", body.clone(), None).await.unwrap();
    assert_eq!(plan["rename"]["lines"], 3, "{plan}");
    assert_eq!(
        plan["rename"]["venues"],
        json!([
            {"venue": "club", "file": "club.light", "lines": 1},
            {"venue": "house", "file": "house.venue", "lines": 2},
        ])
    );
    assert_eq!(text(root, "types/brick.fixture"), TYPE_FILE);

    let mut save = body.clone();
    save["write"] = json!(true);
    save["venue_versions"] = plan["venue_versions"].clone();
    let saved = post(
        &p,
        "Brick",
        save,
        Some(&content_version(TYPE_FILE.as_bytes())),
    )
    .await
    .unwrap();
    assert!(saved["venue_error"].is_null(), "{saved}");
    // The record was at its own name's place, so it moves to the new
    // name's, its comments kept.
    assert_eq!(
        text(root, "types/pixelbrick.fixture"),
        TYPE_FILE.replace("\"Brick\"", "\"PixelBrick\"")
    );
    assert!(!root.join("types/brick.fixture").exists());
    let house = text(root, "venues/house.venue");
    assert_eq!(
        house,
        before_house
            .replace("\"B1\" Brick mode", "\"B1\" PixelBrick mode")
            .replace("\"B2\" Brick mode", "\"B2\" PixelBrick mode"),
        "only the type changes, comments and layout stay"
    );
    assert!(text(root, "venues/club.light")
        .contains("fixture \"C1\" PixelBrick mode \"8: RGBS\" @ 2:1"));
    assert_eq!(text(root, "venues/other.light"), before_other);
}

#[tokio::test]
async fn a_venue_file_that_does_not_parse_blocks_a_rename_and_nothing_is_written() {
    let p = project();
    let root = p.dir.path();
    std::fs::write(root.join("venues/broken.venue"), "venue \"b\" {{{").unwrap();
    let house = text(root, "venues/house.venue");
    let status = post(
        &p,
        "Brick",
        json!({"name": "PixelBrick", "write": true}),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(status.code(), Code::Aborted, "{status:?}");
    assert!(status.message().contains("broken.venue"), "{status:?}");
    assert_eq!(text(root, "types/brick.fixture"), TYPE_FILE);
    assert_eq!(text(root, "venues/house.venue"), house);
}

#[tokio::test]
async fn stale_revisions_are_refused_before_anything_is_written() {
    let p = project();
    let root = p.dir.path();
    let body = json!({"name": "PixelBrick", "write": true});
    let status = post(&p, "Brick", body.clone(), Some("not-it"))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::Aborted);
    // It says the revision the record is at, to reload against.
    assert_eq!(
        status.metadata().get(CURRENT_VERSION_KEY).unwrap(),
        content_version(TYPE_FILE.as_bytes()).as_str()
    );
    let mut stale = body.clone();
    stale["venue_versions"] = json!({"house.venue": "old"});
    let refused = post(&p, "Brick", stale, None).await.unwrap_err();
    assert_eq!(refused.code(), Code::Aborted);
    assert!(refused.message().contains("house.venue"));
    // A venue file's staleness has no one revision to report.
    assert!(refused.metadata().get(CURRENT_VERSION_KEY).is_none());
    assert_eq!(text(root, "types/brick.fixture"), TYPE_FILE);
}

#[tokio::test]
async fn an_empty_expected_revision_is_not_checked() {
    let p = project();
    let saved = post(
        &p,
        "Brick",
        json!({"name": "Brick", "write": true,
               "movement": {"max_pan_speed": 90, "max_tilt_speed": null}}),
        Some(""),
    )
    .await
    .unwrap();
    assert_ne!(saved["version"], content_version(TYPE_FILE.as_bytes()));
}

#[tokio::test]
async fn a_rename_onto_another_type_is_refused() {
    let p = project();
    std::fs::write(
        p.dir.path().join("types/other.light"),
        "fixture_type \"Other\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n",
    )
    .unwrap();
    let status = post(&p, "Brick", json!({"name": "Other", "write": false}), None)
        .await
        .unwrap_err();
    // Every conflict is `ABORTED`, as every one was a `conflict` that
    // reloaded the form.
    assert_eq!(status.code(), Code::Aborted, "{status:?}");
    assert_eq!(
        status.message(),
        "fixture type \"Other\" is already declared in other.light; choose another name"
    );
}

#[tokio::test]
async fn movement_limits_are_patched_into_the_record() {
    let p = project();
    let saved = post(
        &p,
        "Brick",
        json!({"name": "Brick", "write": true,
               "movement": {"max_pan_speed": 240, "max_tilt_speed": null}}),
        Some(&content_version(TYPE_FILE.as_bytes())),
    )
    .await
    .unwrap();
    assert!(saved["rename"].is_null(), "{saved}");
    assert_eq!(
        text(p.dir.path(), "types/brick.fixture"),
        "# Imported from synth.gdtf.\n# Keep this.\nfixture_type \"Brick\"\n  \
         from gdtf(\"library/synth.gdtf\")\n{\n  \
         movement { max_pan_speed: 240deg/s }\n}\n"
    );
    let now = p.settings("Brick", Some("types")).await.unwrap();
    assert_eq!(now["movement"]["max_pan_speed"], 240.0);
    assert!(now["movement"]["max_tilt_speed"].is_null());
    assert_eq!(now["version"], saved["version"]);
}

/// A project whose fixture is its archive alone: `lighting/library/` in
/// the config's directory, no record, and a venue using its derived
/// name. The archive is copied twice under two file names so one name
/// is shared — "Synth Brick" and "Synth Brick (twin)".
fn library_project() -> Project {
    let p = super::project();
    let root = p.dir.path();
    let library = root.join("lighting/library");
    std::fs::create_dir_all(&library).unwrap();
    let archive = crate::testutil::lighting::synthetic_gdtf();
    std::fs::write(library.join("synth.gdtf"), &archive).unwrap();
    std::fs::write(library.join("twin.gdtf"), &archive).unwrap();
    std::fs::create_dir_all(root.join("types")).unwrap();
    std::fs::create_dir_all(root.join("venues")).unwrap();
    std::fs::write(
        root.join("venues/house.venue"),
        "venue \"house\" {\n  fixture \"B1\" \"Synth Brick\" mode \"8: RGBS\" @ 1:1\n}\n",
    )
    .unwrap();
    p
}

#[tokio::test]
async fn an_unrecorded_fixture_answers_its_derived_name_and_a_save_writes_its_record() {
    let p = library_project();
    let root = p.dir.path();
    let body = p.settings("Synth Brick", Some("types")).await.unwrap();
    assert_eq!(
        body,
        json!({
            "name": "Synth Brick",
            "movement": {"max_pan_speed": null, "max_tilt_speed": null},
            "strobe_curve": null,
            "strobe": {"steps": 0, "automatic": "declared"},
            "version": "unrecorded",
        })
    );

    // A plan writes nothing, and nothing to change writes nothing.
    let plan = post(
        &p,
        "Synth Brick",
        json!({"name": "Synth Brick", "write": true}),
        Some("unrecorded"),
    )
    .await
    .unwrap();
    assert_eq!(plan["version"], "unrecorded");
    assert_eq!(std::fs::read_dir(root.join("types")).unwrap().count(), 0);

    // A change writes the record the importer would have, then patches it.
    let saved = post(
        &p,
        "Synth Brick",
        json!({"name": "Synth Brick", "write": true,
               "movement": {"max_pan_speed": 120, "max_tilt_speed": null}}),
        Some("unrecorded"),
    )
    .await
    .unwrap();
    let record = text(root, "types/synth_brick.fixture");
    assert!(record.starts_with("# Imported from synth.gdtf"), "{record}");
    assert!(
        record
            .contains("fixture_type \"Synth Brick\"\n  from gdtf(\"lighting/library/synth.gdtf\")"),
        "{record}"
    );
    assert!(record.contains("max_pan_speed: 120deg/s"), "{record}");
    assert_eq!(saved["version"], content_version(record.as_bytes()));

    // "No record yet" is stale once there is one.
    let status = post(
        &p,
        "Synth Brick",
        json!({"name": "Synth Brick", "write": true}),
        Some("unrecorded"),
    )
    .await
    .unwrap_err();
    assert_eq!(status.code(), Code::Aborted);
    let body = p.settings("Synth Brick", Some("types")).await.unwrap();
    assert_ne!(body["version"], "unrecorded");
    assert_eq!(body["movement"]["max_pan_speed"], 120.0);
    // The refusal told what the record is at.
    assert_eq!(
        status.metadata().get(CURRENT_VERSION_KEY).unwrap(),
        body["version"].as_str().unwrap()
    );
}

#[tokio::test]
async fn the_strobe_curve_is_saved_in_the_record_and_read_back() {
    let p = library_project();
    let root = p.dir.path();
    // Automatic is the default: saving it writes no record.
    post(
        &p,
        "Synth Brick",
        json!({"name": "Synth Brick", "write": true, "strobe_curve": null}),
        Some("unrecorded"),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read_dir(root.join("types")).unwrap().count(), 0);

    post(
        &p,
        "Synth Brick",
        json!({"name": "Synth Brick", "write": true, "strobe_curve": "linear"}),
        Some("unrecorded"),
    )
    .await
    .unwrap();
    let record = text(root, "types/synth_brick.fixture");
    assert!(record.contains("  strobe_curve: linear\n"), "{record}");
    let body = p.settings("Synth Brick", Some("types")).await.unwrap();
    assert_eq!(body["strobe_curve"], "linear");
    let version = body["version"].as_str().unwrap().to_string();

    // A stale version is refused and writes nothing.
    let status = post(
        &p,
        "Synth Brick",
        json!({"name": "Synth Brick", "write": true, "strobe_curve": "period"}),
        Some("stale"),
    )
    .await
    .unwrap_err();
    assert_eq!(status.code(), Code::Aborted);
    assert_eq!(text(root, "types/synth_brick.fixture"), record);

    // Back to automatic: the statement goes; movement limits are kept
    // apart.
    post(
        &p,
        "Synth Brick",
        json!({"name": "Synth Brick", "write": true, "strobe_curve": null,
               "movement": {"max_pan_speed": 90, "max_tilt_speed": null}}),
        Some(&version),
    )
    .await
    .unwrap();
    let record = text(root, "types/synth_brick.fixture");
    assert!(!record.contains("strobe_curve"), "{record}");
    assert!(record.contains("max_pan_speed: 90deg/s"), "{record}");
    let body = p.settings("Synth Brick", Some("types")).await.unwrap();
    assert_eq!(body["strobe_curve"], Value::Null);
    assert_eq!(body["strobe"]["automatic"], "declared");
}

#[tokio::test]
async fn a_strobe_curve_that_names_none_is_refused() {
    let p = library_project();
    let mut request = request(
        "Synth Brick",
        &json!({"name": "Synth Brick", "write": true}),
        None,
    );
    // A number no `FixtureStrobeCurve` has: not the automatic one.
    request.strobe_curve = 99;
    let status = p.save_settings(request).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert_eq!(
        std::fs::read_dir(p.dir.path().join("types"))
            .unwrap()
            .count(),
        0
    );
}

#[tokio::test]
async fn renaming_an_unrecorded_fixture_pins_the_one_that_shares_its_name() {
    let p = library_project();
    let root = p.dir.path();
    p.settings("Synth Brick (twin)", Some("types"))
        .await
        .unwrap();
    let saved = post(
        &p,
        "Synth Brick",
        json!({"name": "House Brick", "write": true}),
        Some("unrecorded"),
    )
    .await
    .unwrap();
    assert_eq!(saved["rename"]["lines"], 1, "{saved}");
    assert!(text(root, "venues/house.venue")
        .contains("fixture \"B1\" \"House Brick\" mode \"8: RGBS\" @ 1:1"));
    let record = text(root, "types/house_brick.fixture");
    assert!(
        record
            .contains("fixture_type \"House Brick\"\n  from gdtf(\"lighting/library/synth.gdtf\")"),
        "{record}"
    );
    // Without its pin, the twin would take the freed name on the next load.
    let pin = text(root, "types/synth_brick_twin.fixture");
    assert!(
        pin.contains(
            "fixture_type \"Synth Brick (twin)\"\n  from gdtf(\"lighting/library/twin.gdtf\")"
        ),
        "{pin}"
    );
    let library = crate::lighting::library::unrecorded_types(root, Some(&root.join("types")));
    assert!(library.types.is_empty(), "{:?}", library.types);
}

#[tokio::test]
async fn a_rename_onto_a_library_fixture_s_name_is_refused() {
    let p = library_project();
    let status = post(
        &p,
        "Synth Brick",
        json!({"name": "Synth Brick (twin)", "write": false}),
        Some("unrecorded"),
    )
    .await
    .unwrap_err();
    assert_eq!(status.code(), Code::Aborted, "{status:?}");
}

#[tokio::test]
async fn a_new_name_that_is_not_a_name_is_refused_in_the_users_terms() {
    let p = project();
    let status = post(
        &p,
        "Brick",
        json!({"name": "Up/Down", "write": false}),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(
        status
            .message()
            .starts_with("\"Up/Down\" cannot be a fixture type name"),
        "{}",
        status.message()
    );
}

const OTHER_TYPE: &str =
    "fixture_type \"Old\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n";

/// The files in `types`, by name.
fn type_files(root: &FsPath) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join("types"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[tokio::test]
async fn a_pin_whose_file_is_taken_in_any_case_is_refused_not_suffixed() {
    for taken in ["synth_brick.fixture", "Synth_Brick.fixture"] {
        let p = library_project();
        let root = p.dir.path();
        std::fs::write(root.join("types").join(taken), OTHER_TYPE).unwrap();

        let status = post(
            &p,
            "Synth Brick",
            json!({"name": "Synth Brick", "write": true,
                   "movement": {"max_pan_speed": 120, "max_tilt_speed": null}}),
            Some("unrecorded"),
        )
        .await
        .unwrap_err();

        assert_eq!(status.code(), Code::Aborted, "{status:?}");
        assert_eq!(
            status.message(),
            format!("{taken} is taken by \"Old\"; choose another name")
        );
        assert_eq!(type_files(root), [taken], "no synth_brick_2.fixture");
    }
}

#[tokio::test]
async fn the_settings_of_a_type_whose_file_is_not_its_alone_are_read_only() {
    let p = project();
    let shared = format!("{TYPE_FILE}{OTHER_TYPE}");
    std::fs::write(p.dir.path().join("types/brick.fixture"), &shared).unwrap();

    let status = post(
        &p,
        "Brick",
        json!({"name": "Brick", "write": true,
               "movement": {"max_pan_speed": 240, "max_tilt_speed": null}}),
        None,
    )
    .await
    .unwrap_err();

    assert_eq!(status.code(), Code::Aborted, "{status:?}");
    assert_eq!(
        status.message(),
        "Defined in brick.fixture alongside fixture type \"Old\"; edit that file by hand."
    );
    assert_eq!(text(p.dir.path(), "types/brick.fixture"), shared);
}

#[tokio::test]
async fn a_rename_is_refused_when_a_venue_using_it_is_not_its_file_s_alone() {
    let p = project();
    let root = p.dir.path();
    let shared = "venue \"arena\" {\n  fixture \"A1\" Brick mode \"8: RGBS\" @ 3:1\n}\n\
                  venue \"annex\" {\n}\n";
    std::fs::write(root.join("venues/all.venue"), shared).unwrap();
    let house = text(root, "venues/house.venue");

    let status = post(
        &p,
        "Brick",
        json!({"name": "PixelBrick", "write": true}),
        None,
    )
    .await
    .unwrap_err();

    assert_eq!(status.code(), Code::Aborted, "{status:?}");
    assert_eq!(
        status.message(),
        "fixture type \"Brick\" cannot be renamed, so nothing was written; venue \"arena\" \
         in all.venue uses it: Defined in all.venue alongside venue \"annex\"; edit that file \
         by hand."
    );
    assert_eq!(text(root, "venues/all.venue"), shared);
    assert_eq!(text(root, "venues/house.venue"), house);
    assert_eq!(text(root, "types/brick.fixture"), TYPE_FILE);
    assert_eq!(type_files(root), ["brick.fixture"]);

    // A settings change that renames nothing touches no venue, so the
    // shared file does not stand in its way.
    post(
        &p,
        "Brick",
        json!({"name": "Brick", "write": true,
               "movement": {"max_pan_speed": 240, "max_tilt_speed": null}}),
        None,
    )
    .await
    .unwrap();
    assert_eq!(text(root, "venues/all.venue"), shared);
}

/// The harness's record: "Astera-PixelBrick" in `astera_pixelbrick.fixture`,
/// which is not its own name's file (`astera-pixelbrick.fixture`).
const ASTERA: &str = "# Imported from synth.gdtf.\nfixture_type \"Astera-PixelBrick\"\n  \
                      from gdtf(\"library/synth.gdtf\")\n{\n}\n";

#[tokio::test]
async fn the_harness_record_is_edited_and_renamed_where_it_lives() {
    let p = project();
    let root = p.dir.path();
    std::fs::remove_file(root.join("types/brick.fixture")).unwrap();
    std::fs::write(root.join("types/astera_pixelbrick.fixture"), ASTERA).unwrap();
    std::fs::write(
        root.join("venues/house.venue"),
        "venue \"house\" {\n  fixture \"B1\" \"Astera-PixelBrick\" mode \"8: RGBS\" @ 1:1\n}\n",
    )
    .unwrap();

    post(
        &p,
        "Astera-PixelBrick",
        json!({"name": "Astera-PixelBrick", "write": true,
               "movement": {"max_pan_speed": 240, "max_tilt_speed": null}}),
        Some(&content_version(ASTERA.as_bytes())),
    )
    .await
    .unwrap();
    let edited = text(root, "types/astera_pixelbrick.fixture");
    assert!(edited.contains("max_pan_speed: 240deg/s"), "{edited}");

    post(
        &p,
        "Astera-PixelBrick",
        json!({"name": "PB15", "write": true,
               "movement": {"max_pan_speed": 240, "max_tilt_speed": null}}),
        Some(&content_version(edited.as_bytes())),
    )
    .await
    .unwrap();
    assert_eq!(type_files(root), ["astera_pixelbrick.fixture"]);
    assert!(text(root, "types/astera_pixelbrick.fixture").contains("fixture_type \"PB15\""));
    assert!(text(root, "venues/house.venue").contains("fixture \"B1\" PB15 mode"));
}

/// Every file under `root` (relative) and its text.
fn tree(root: &FsPath) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> =
        crate::lighting::project_files::files_under(root, &["light", "fixture", "venue", "gdtf"])
            .iter()
            .map(|p| {
                (
                    crate::lighting::project_files::display_in(root, p),
                    std::fs::read_to_string(p).unwrap_or_default(),
                )
            })
            .collect();
    out.sort();
    out
}

/// Makes `dir` read-only; `false` when that does not stop a write (the
/// tests run as root), and the test has nothing to show.
fn lock(dir: &FsPath) -> bool {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o555)).unwrap();
    let probe = dir.join(".probe");
    if std::fs::write(&probe, "").is_ok() {
        std::fs::remove_file(&probe).unwrap();
        unlock(dir);
        return false;
    }
    true
}

fn unlock(dir: &FsPath) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test]
async fn a_venue_write_failing_puts_back_the_type_and_the_venues_written() {
    let p = project();
    let root = p.dir.path();
    // Written after club.light and house.venue, in path order.
    std::fs::create_dir_all(root.join("venues/locked")).unwrap();
    std::fs::write(
        root.join("venues/locked/tour.venue"),
        "venue \"tour\" {\n  fixture \"T1\" Brick mode \"8: RGBS\" @ 4:1\n}\n",
    )
    .unwrap();
    let before = tree(root);
    if !lock(&root.join("venues/locked")) {
        return;
    }

    let status = post(
        &p,
        "Brick",
        json!({"name": "PixelBrick", "write": true}),
        None,
    )
    .await
    .unwrap_err();
    unlock(&root.join("venues/locked"));

    assert_eq!(status.code(), Code::Internal, "{status:?}");
    let error = status.message();
    assert!(error.contains("locked/tour.venue"), "{error}");
    assert!(error.ends_with("; nothing was saved"), "{error}");
    assert_eq!(tree(root), before, "every file as it was");
}

#[tokio::test]
async fn a_type_write_failing_writes_no_venue() {
    let p = project();
    let root = p.dir.path();
    let before = tree(root);
    if !lock(&root.join("types")) {
        return;
    }

    let status = post(
        &p,
        "Brick",
        json!({"name": "PixelBrick", "write": true}),
        None,
    )
    .await
    .unwrap_err();
    unlock(&root.join("types"));

    assert_eq!(status.code(), Code::Internal, "{status:?}");
    let error = status.message();
    assert!(error.contains("pixelbrick.fixture"), "{error}");
    assert!(error.ends_with("; nothing was saved"), "{error}");
    assert_eq!(tree(root), before, "every file as it was");
}

#[tokio::test]
async fn an_old_record_that_cannot_be_removed_puts_everything_back() {
    let p = project();
    let root = p.dir.path();
    let before = tree(root);
    let old = root.join("types/brick.fixture");
    crate::lighting::writes::test_hook::refuse_removal_of(&old);
    if let Ok(canonical) = old.canonicalize() {
        crate::lighting::writes::test_hook::refuse_removal_of(&canonical);
    }

    let status = post(
        &p,
        "Brick",
        json!({"name": "PixelBrick", "write": true}),
        None,
    )
    .await
    .unwrap_err();

    // A rename leaves no stray "Brick" behind: the new record and every
    // venue rewritten are put back, and the answer says so.
    assert_eq!(status.code(), Code::Internal, "{status:?}");
    assert_eq!(
        status.message(),
        "cannot remove brick.fixture: refused by the test; nothing was saved"
    );
    assert_eq!(tree(root), before, "every file as it was");
}

#[tokio::test]
async fn a_light_record_moves_to_its_fixture_file_when_saved() {
    let p = project();
    let root = p.dir.path();
    std::fs::rename(
        root.join("types/brick.fixture"),
        root.join("types/brick.light"),
    )
    .unwrap();

    post(
        &p,
        "Brick",
        json!({"name": "Brick", "write": true,
               "movement": {"max_pan_speed": 240, "max_tilt_speed": null}}),
        None,
    )
    .await
    .unwrap();

    assert_eq!(type_files(root), ["brick.fixture"]);
    assert!(text(root, "types/brick.fixture").contains("max_pan_speed: 240deg/s"));
}

#[tokio::test]
async fn the_strobe_table_is_reported_for_a_fixture_with_a_strobe_function() {
    // The synthetic archive has none in hertz: the answer says so.
    let p = project();
    let body = p.settings("Brick", Some("types")).await.unwrap();
    assert_eq!(body["strobe"], json!({"steps": 0, "automatic": "declared"}));
}
