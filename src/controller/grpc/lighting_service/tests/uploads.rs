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

//! The GDTF and MVR import and export calls (`ImportGdtf`, `InspectMvr`,
//! `ImportMvr`, `ExportMvr`), called as the service, with the bytes sent as
//! the web server stages them: each held by a test that was the REST route's
//! (the GDTF import, the MVR inspector and import, the MVR export download).
//! The answers are read back as the JSON the routes sent ([`super::json`]).

use serde_json::{json, Value};
use tonic::{Code, Request, Status};

use super::types::synthetic_gdtf_bytes;
use super::{json as read, Project};
use crate::proto::player::v1::{self as proto, lighting_service_server::LightingService};
use crate::webui::api::mvr_api::test::synthetic_mvr;
use crate::webui::transfers::Purpose;

/// The kind of refusal a status says (`x-import-kind`), if it does.
fn kind_of(status: &Status) -> Option<String> {
    status
        .metadata()
        .get(super::super::IMPORT_KIND_KEY)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string)
}

fn options(name: Option<&str>, origin: Option<[f64; 3]>) -> proto::MvrOptions {
    proto::MvrOptions {
        name: name.map(str::to_string),
        origin_mm: origin.map(|o| proto::LightingVec3 {
            x: o[0],
            y: o[1],
            z: o[2],
        }),
        ..Default::default()
    }
}

/// The options the old tests sent as `name=kellys`, `origin=0,1000,0`.
fn kellys_options() -> proto::MvrOptions {
    options(Some("kellys"), Some([0.0, 1000.0, 0.0]))
}

fn keep(fixtures: &[(&str, &[&str])]) -> proto::MvrKeep {
    proto::MvrKeep {
        fixtures: fixtures
            .iter()
            .map(|(name, fields)| {
                (
                    name.to_string(),
                    proto::MvrKeepFields {
                        fields: fields.iter().map(|f| f.to_string()).collect(),
                    },
                )
            })
            .collect(),
        focus_points: vec![],
    }
}

impl Project {
    fn transfers(&self) -> &crate::webui::transfers::Transfers {
        self.state.player.transfers()
    }

    /// Announces an upload through `BeginUpload` and sends its bytes, as the
    /// web UI does; the id.
    async fn send(
        &self,
        purpose: proto::UploadPurpose,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<String, Status> {
        let config =
            crate::controller::grpc::config_service::ConfigServer::new(self.state.player.clone());
        use crate::proto::player::v1::config_service_server::ConfigService;
        let begun = config
            .begin_upload(Request::new(proto::BeginUploadRequest {
                purpose: purpose.into(),
                file_name: file_name.to_string(),
                size: Some(bytes.len() as u64),
                song: None,
            }))
            .await?
            .into_inner();
        let receive = self.transfers().receive(&begun.upload_id).unwrap();
        std::fs::write(receive.path(), bytes).unwrap();
        receive.finish(bytes.len() as u64);
        Ok(begun.upload_id)
    }

    /// An upload registered with no checks, for the claim to meet.
    fn unchecked(&self, purpose: Purpose, file_name: &str, bytes: &[u8]) -> String {
        let id = self
            .transfers()
            .begin_upload(purpose, file_name.to_string(), bytes.len() as u64)
            .unwrap();
        let receive = self.transfers().receive(&id).unwrap();
        std::fs::write(receive.path(), bytes).unwrap();
        receive.finish(bytes.len() as u64);
        id
    }

    async fn gdtf(&self, file: &str, bytes: &[u8], name: Option<&str>) -> Result<Value, Status> {
        let id = self
            .send(proto::UploadPurpose::GdtfImport, file, bytes)
            .await?;
        self.import_gdtf_id(&id, name).await
    }

    async fn import_gdtf_id(&self, id: &str, name: Option<&str>) -> Result<Value, Status> {
        self.server
            .import_gdtf(Request::new(proto::ImportGdtfRequest {
                upload_id: id.to_string(),
                name: name.map(str::to_string),
            }))
            .await
            .map(|r| read::gdtf_import(&r.into_inner()))
    }

    async fn inspect(
        &self,
        file: &str,
        bytes: &[u8],
        options: proto::MvrOptions,
    ) -> Result<Value, Status> {
        let id = self
            .send(proto::UploadPurpose::MvrInspect, file, bytes)
            .await?;
        self.inspect_id(&id, Some(options)).await
    }

    async fn inspect_id(
        &self,
        id: &str,
        options: Option<proto::MvrOptions>,
    ) -> Result<Value, Status> {
        self.server
            .inspect_mvr(Request::new(proto::InspectMvrRequest {
                upload_id: id.to_string(),
                options,
            }))
            .await
            .map(|r| read::mvr_inspection(&r.into_inner()))
    }

    async fn import(
        &self,
        file: &str,
        bytes: &[u8],
        write: bool,
        options: proto::MvrOptions,
    ) -> Result<Value, Status> {
        let id = self
            .send(proto::UploadPurpose::MvrImport, file, bytes)
            .await?;
        self.import_id(&id, write, Some(options)).await
    }

    async fn import_id(
        &self,
        id: &str,
        write: bool,
        options: Option<proto::MvrOptions>,
    ) -> Result<Value, Status> {
        self.server
            .import_mvr(Request::new(proto::ImportMvrRequest {
                upload_id: id.to_string(),
                write,
                options,
            }))
            .await
            .map(|r| read::mvr_import_answer(&r.into_inner()))
    }

    async fn export(
        &self,
        request: proto::ExportMvrRequest,
    ) -> Result<proto::ExportMvrResponse, Status> {
        self.server
            .export_mvr(Request::new(request))
            .await
            .map(|r| r.into_inner())
    }

    fn staged(&self) -> usize {
        std::fs::read_dir(self.transfers().staging_dir().unwrap())
            .unwrap()
            .count()
    }
}

fn export_of(venue: &str) -> proto::ExportMvrRequest {
    proto::ExportMvrRequest {
        venue: venue.to_string(),
        ..Default::default()
    }
}

// ---- ImportGdtf (POST /lighting/gdtf/import) ----

#[tokio::test]
async fn a_gdtf_is_imported_through_the_shared_importer() {
    let p = super::project();
    // A traversal-shaped filename must land as its final component only.
    let parsed = p
        .gdtf("../synth.gdtf", &synthetic_gdtf_bytes(), Some("Brick"))
        .await
        .unwrap();
    assert_eq!(parsed["type_name"], "Brick");
    assert!(p.dir.path().join("lighting/library/synth.gdtf").exists());
    assert!(
        !p.dir.path().parent().unwrap().join("synth.gdtf").exists(),
        "traversal filename must not escape the library"
    );
    assert!(p
        .dir
        .path()
        .join("lighting/fixture_types/brick.fixture")
        .exists());
    assert!(p.dir.path().join("lighting/.cache").is_dir());
    assert_eq!(p.staged(), 0, "the staged archive went with the claim");

    // A name another fixture has is a conflict naming its file, not a
    // server fault; so is a name whose file another type has.
    let other = crate::lighting::gdtf::build_zip(&[(
        "description.xml",
        crate::lighting::gdtf::SYNTHETIC_DESCRIPTION
            .replace("Synth Brick", "Other Brick")
            .as_bytes(),
    )]);
    let refused = p
        .gdtf("other.gdtf", &other, Some("Brick"))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused}");
    assert_eq!(
        refused.message(),
        "fixture type \"Brick\" is already declared in brick.fixture; choose another name"
    );
    assert_eq!(kind_of(&refused).as_deref(), Some("name"));
    p.gdtf("other.gdtf", &other, Some("BRICK 1")).await.unwrap();
    let refused = p
        .gdtf("other.gdtf", &other, Some("brick_1"))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused}");
    assert_eq!(
        refused.message(),
        "brick_1.fixture is taken by \"BRICK 1\"; choose another name"
    );
}

/// #673: the import's name check and its pinned record go to the fixture
/// types directory this host's profile sets, as the CLI and MCP put them,
/// not the default one the player does not read; and the other calls that
/// default a directory read it there too.
#[tokio::test]
async fn an_import_goes_to_the_hosts_configured_fixture_types_dir() {
    let p = super::project();
    p.write_config(
        "songs: \".\"\nprofiles:\n  - hostname: not-this-host-x9\n    dmx:\n      \
         universes: []\n      lighting:\n        directories:\n          \
         fixture_types: other/types\n  - dmx:\n      universes: []\n      \
         lighting:\n        directories:\n          fixture_types: rig/types\n",
    );
    std::fs::create_dir_all(p.dir.path().join("rig/types")).unwrap();
    p.gdtf("synth.gdtf", &synthetic_gdtf_bytes(), Some("Brick"))
        .await
        .unwrap();
    assert!(p.dir.path().join("rig/types/brick.fixture").exists());
    assert!(!p
        .dir
        .path()
        .join("lighting/fixture_types/brick.fixture")
        .exists());
    assert!(!p.dir.path().join("other/types/brick.fixture").exists());
    // Taken there: another archive under the name is a conflict.
    let other = crate::lighting::gdtf::build_zip(&[(
        "description.xml",
        crate::lighting::gdtf::SYNTHETIC_DESCRIPTION
            .replace("Synth Brick", "Other Brick")
            .as_bytes(),
    )]);
    let refused = p
        .gdtf("other.gdtf", &other, Some("Brick"))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused}");
}

#[tokio::test]
async fn an_import_with_no_name_is_one_step_and_says_what_the_user_has() {
    let p = super::project();
    let parsed = p
        .gdtf("synth.gdtf", &synthetic_gdtf_bytes(), None)
        .await
        .unwrap();
    assert_eq!(parsed["type_name"], "Synth Brick");
    assert_eq!(parsed["fixture"], "Synth Brick");
    assert_eq!(parsed["manufacturer"], "mtrack synthetic");
    assert_eq!(parsed["modes"], 2);
    assert_eq!(parsed["archive"], "lighting/library/synth.gdtf");
    assert_eq!(parsed["already_imported"], false);
    assert!(parsed["renamed_from"].is_null());
    assert_eq!(parsed["refused_modes"], json!([]));
    assert!(
        !p.dir.path().join("lighting/fixture_types").exists(),
        "an import is a copy; no record"
    );

    // The same archive again: nothing written, and it says so.
    let parsed = p
        .gdtf("copy.gdtf", &synthetic_gdtf_bytes(), None)
        .await
        .unwrap();
    assert_eq!(parsed["already_imported"], true);
    assert_eq!(parsed["type_name"], "Synth Brick");
    assert!(!p.dir.path().join("lighting/library/copy.gdtf").exists());

    // Another archive of the same fixture name gets its file stem.
    let other = crate::lighting::gdtf::build_zip(&[(
        "description.xml",
        crate::lighting::gdtf::SYNTHETIC_DESCRIPTION
            .replace("mtrack synthetic", "someone else")
            .as_bytes(),
    )]);
    let parsed = p.gdtf("brick-v2.gdtf", &other, None).await.unwrap();
    assert_eq!(parsed["type_name"], "Synth Brick (brick-v2)");
    assert_eq!(parsed["renamed_from"], "Synth Brick");
    assert_eq!(
        parsed["fixture_file"],
        "lighting/fixture_types/synth_brick_brick-v2.fixture"
    );
    let pin = std::fs::read_to_string(
        p.dir
            .path()
            .join("lighting/fixture_types/synth_brick_brick-v2.fixture"),
    )
    .unwrap_or_else(|e| panic!("the newcomer's name is pinned: {e}"));
    assert!(
        pin.contains("from gdtf(\"lighting/library/brick-v2.gdtf\")"),
        "{pin}"
    );

    // A different archive of a name the library already has: a conflict,
    // in the user's terms, and nothing written.
    let third = crate::lighting::gdtf::build_zip(&[(
        "description.xml",
        crate::lighting::gdtf::SYNTHETIC_DESCRIPTION
            .replace("Synth Brick", "Third Brick")
            .as_bytes(),
    )]);
    let refused = p.gdtf("synth.gdtf", &third, None).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused}");
    assert_eq!(kind_of(&refused).as_deref(), Some("library"));
    assert!(
        refused.message().contains("Rename your file"),
        "{}",
        refused.message()
    );
    assert!(
        !refused.message().contains(".fixture"),
        "{}",
        refused.message()
    );

    // The fixture page takes it with no record.
    let details = p
        .server
        .get_fixture_type_gdtf_details(Request::new(proto::GetFixtureTypeGdtfDetailsRequest {
            name: "Synth Brick".to_string(),
            dir: None,
            venues_dir: None,
        }))
        .await
        .unwrap()
        .into_inner();
    assert!(details.rig.is_some(), "{details:?}");
}

#[tokio::test]
async fn a_file_that_is_not_a_gdtf_is_refused_with_the_reason() {
    let p = super::project();
    let refused = p.gdtf("junk.gdtf", b"not a zip", None).await.unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument, "{refused}");
    assert!(!p.dir.path().join("lighting").exists());
    assert_eq!(p.staged(), 0, "a refused upload is spent too");
}

/// The name an archive is kept under is the final component of what was
/// sent, at the announcement; one with none is refused there and at the claim.
#[tokio::test]
async fn a_gdtf_name_is_reduced_to_its_final_component_or_refused() {
    let p = super::project();
    let config = crate::controller::grpc::config_service::ConfigServer::new(p.state.player.clone());
    use crate::proto::player::v1::config_service_server::ConfigService;
    for name in ["", "..", "../..", "a/..", "/", "."] {
        let refused = config
            .begin_upload(Request::new(proto::BeginUploadRequest {
                purpose: proto::UploadPurpose::GdtfImport.into(),
                file_name: name.to_string(),
                size: Some(1),
                song: None,
            }))
            .await;
        assert_eq!(
            refused.unwrap_err().code(),
            Code::InvalidArgument,
            "{name:?}"
        );
        // The library refuses what is not a bare file name, whatever came.
        let id = p.unchecked(Purpose::GdtfImport, name, &synthetic_gdtf_bytes());
        let refused = p.import_gdtf_id(&id, None).await;
        assert!(refused.is_err(), "claim accepted {name:?}");
    }
    assert!(!p.dir.path().join("lighting/library").exists());
    assert_eq!(p.staged(), 0);
}

// ---- InspectMvr (POST /lighting/mvr/inspect) ----

#[tokio::test]
async fn inspect_reports_positions_and_deck_bounds_and_writes_nothing() {
    let p = super::project();
    let body = p
        .inspect("Kellys.mvr", &synthetic_mvr(true), options(None, None))
        .await
        .unwrap();
    assert_eq!(body["file_name"], "Kellys.mvr");
    assert_eq!(body["report"]["venue_name"], "Kellys");
    assert_eq!(body["report"]["fixtures"].as_array().unwrap().len(), 3);
    let fixtures = body["scene"]["fixtures"].as_array().unwrap();
    assert_eq!(fixtures[0]["position_mm"], json!([-2000.0, 3500.0, 4200.0]));
    assert_eq!(fixtures[2]["position_mm"], json!(null), "Lost has none");
    assert_eq!(
        body["scene"]["deck_mm"],
        json!({"min": [-3000.0, 1000.0, 0.0], "max": [3000.0, 5000.0, 500.0]})
    );
    assert!(
        !p.dir.path().join("lighting").exists(),
        "inspect must not write"
    );

    // No object that says it is a deck: null, and the wizard falls back.
    let body = p
        .inspect("Kellys.mvr", &synthetic_mvr(false), options(None, None))
        .await
        .unwrap();
    assert!(body["scene"]["deck_mm"].is_null(), "{body}");
    assert_eq!(body["scene"]["scenery"][0]["deck"], false);
}

#[tokio::test]
async fn inspect_refuses_what_is_not_an_mvr_with_the_reason() {
    let p = super::project();
    let refused = p
        .inspect("junk.mvr", b"not a zip", options(None, None))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    assert!(
        refused.message().contains("not a readable MVR archive"),
        "{}",
        refused.message()
    );
    // A name that is not an .mvr is refused when the upload is announced.
    let refused = p
        .inspect("rig.zip", &synthetic_mvr(true), options(None, None))
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    assert!(refused.message().contains(".mvr"), "{}", refused.message());
    // And again at the claim, for one announced another way.
    let id = p.unchecked(Purpose::MvrInspect, "rig.zip", &synthetic_mvr(true));
    let refused = p.inspect_id(&id, None).await.unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    assert!(refused.message().contains(".mvr"), "{}", refused.message());
    // A directory that escapes the project is refused, not followed.
    let refused = p
        .inspect(
            "rig.mvr",
            &synthetic_mvr(true),
            proto::MvrOptions {
                venues_dir: Some("../elsewhere".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::InvalidArgument);
    assert_eq!(p.staged(), 0);
}

/// The directories an inspection or import names stay in the project: a
/// path that leaves it, an absolute one, one with a backslash or a NUL, and
/// a directory that is a symlink out of the project.
#[tokio::test]
async fn the_directories_named_stay_in_the_project() {
    let p = super::project();
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(p.dir.path().join("lighting")).unwrap();
    std::os::unix::fs::symlink(elsewhere.path(), p.dir.path().join("lighting/linked")).unwrap();
    let mvr = synthetic_mvr(true);
    for dir in [
        "../elsewhere",
        "../../etc",
        "/etc",
        "/tmp",
        "lighting/../../out",
        "lighting\\venues",
        "lighting/ve\0nues",
        "lighting/linked",
    ] {
        for which in ["venues_dir", "fixture_types_dir"] {
            let named = || proto::MvrOptions {
                venues_dir: (which == "venues_dir").then(|| dir.to_string()),
                fixture_types_dir: (which == "fixture_types_dir").then(|| dir.to_string()),
                ..kellys_options()
            };
            let refused = p.inspect("k.mvr", &mvr, named()).await;
            assert!(refused.is_err(), "inspect accepted {which}={dir:?}");
            for write in [false, true] {
                let refused = p.import("k.mvr", &mvr, write, named()).await;
                assert!(refused.is_err(), "import({write}) accepted {which}={dir:?}");
            }
        }
    }
    assert_eq!(std::fs::read_dir(elsewhere.path()).unwrap().count(), 0);
    assert!(!p.dir.path().join("lighting/library").exists());
    assert!(!p.dir.path().join("lighting/venues").exists());
    assert_eq!(p.staged(), 0);
}

#[tokio::test]
async fn the_file_name_of_an_mvr_is_reduced_to_its_final_component() {
    let p = super::project();
    let body = p
        .inspect(
            "../../Kellys.mvr",
            &synthetic_mvr(true),
            options(None, None),
        )
        .await
        .unwrap();
    assert_eq!(body["file_name"], "Kellys.mvr");
    for name in ["", "..", "/", ".mvr/..", "a/.."] {
        let begun = p.send(proto::UploadPurpose::MvrInspect, name, b"x").await;
        assert_eq!(begun.unwrap_err().code(), Code::InvalidArgument, "{name:?}");
    }
}

// ---- ImportMvr (POST /lighting/mvr/import) ----

#[tokio::test]
async fn import_plans_without_writing_and_then_writes() {
    let p = super::project();
    let mvr = synthetic_mvr(true);
    let body = p
        .import("Kellys.mvr", &mvr, false, kellys_options())
        .await
        .unwrap();
    assert_eq!(body["write"], false);
    assert_eq!(body["plan"]["venue_name"], "kellys");
    assert_eq!(body["plan"]["merge"], false);
    assert_eq!(body["plan"]["origin"], json!([0.0, 1.0, 0.0]));
    assert!(body["plan"]["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .any(|f| f["name"] == "Lost" && f["todo"].is_string()));
    assert!(
        !p.dir.path().join("lighting").exists(),
        "a plan must not write"
    );

    let body = p
        .import("Kellys.mvr", &mvr, true, kellys_options())
        .await
        .unwrap();
    assert_eq!(body["write"], true);
    assert_eq!(body["report"]["venue_name"], "kellys");
    assert_eq!(body["reloaded"], false);
    assert!(body["venue_error"].is_null());
    assert!(body["report"]["written"].as_array().unwrap().len() >= 2);
    assert!(p.dir.path().join("lighting/venues/kellys.venue").exists());
    assert!(p.dir.path().join("lighting/library/Kellys.mvr").exists());
    let venue = std::fs::read_to_string(p.dir.path().join("lighting/venues/kellys.venue")).unwrap();
    assert!(venue.contains("# TODO fixture \"Lost\""), "{venue}");

    // The same again is a merge.
    let body = p
        .import("Kellys.mvr", &mvr, true, kellys_options())
        .await
        .unwrap();
    assert_eq!(body["report"]["merge"], true, "{body}");
}

#[tokio::test]
async fn a_merge_plan_lists_hand_edits_and_the_keep_list_keeps_them() {
    let p = super::project();
    let mvr = synthetic_mvr(true);
    p.import("Kellys.mvr", &mvr, true, kellys_options())
        .await
        .unwrap();

    // Hand-move Brick 1 (the MVR puts it at x = -2).
    let path = p.kellys();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("position (-2, 2.5, 4.2)"), "{text}");
    std::fs::write(
        &path,
        format!(
            "# mine\n{}",
            text.replace("position (-2, 2.5, 4.2)", "position (-1, 2.5, 4.2)")
        ),
    )
    .unwrap();

    let body = p
        .import("Kellys.mvr", &mvr, false, kellys_options())
        .await
        .unwrap();
    let brick1 = body["plan"]["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Brick 1")
        .unwrap();
    assert_eq!(brick1["overwrites"][0]["field"], "position", "{brick1}");
    assert_eq!(brick1["overwrites"][0]["mine"], "(-1, 2.5, 4.2)");

    let kept = proto::MvrOptions {
        keep: Some(keep(&[("Brick 1", &["position"])])),
        ..kellys_options()
    };
    p.import("Kellys.mvr", &mvr, true, kept).await.unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.starts_with("# mine\n"), "{after}");
    assert!(after.contains("position (-1, 2.5, 4.2)"), "{after}");
}

#[tokio::test]
async fn an_origin_must_be_three_finite_numbers() {
    let p = super::project();
    let mvr = synthetic_mvr(true);
    let id = p
        .send(proto::UploadPurpose::MvrImport, "K.mvr", &mvr)
        .await
        .unwrap();
    let inspect = p
        .send(proto::UploadPurpose::MvrInspect, "K.mvr", &mvr)
        .await
        .unwrap();
    for bad in [
        [f64::NAN, 0.0, 0.0],
        [0.0, f64::INFINITY, 0.0],
        [0.0, 0.0, f64::NEG_INFINITY],
    ] {
        let refused = p
            .import_id(&id, true, Some(options(None, Some(bad))))
            .await
            .unwrap_err();
        assert_eq!(refused.code(), Code::InvalidArgument);
        assert!(
            refused.message().contains("origin"),
            "{}",
            refused.message()
        );
        let refused = p
            .inspect_id(&inspect, Some(options(None, Some(bad))))
            .await
            .unwrap_err();
        assert_eq!(refused.code(), Code::InvalidArgument);
    }
    assert!(!p.dir.path().join("lighting").exists());
    // A request that is not valid is refused before the upload is claimed:
    // it is still good, with a valid one.
    p.import_id(&id, false, Some(kellys_options()))
        .await
        .unwrap();
    p.inspect_id(&inspect, None).await.unwrap();
    assert_eq!(p.staged(), 0);
}

#[tokio::test]
async fn a_re_import_into_a_shared_venue_file_is_a_conflict_and_writes_nothing() {
    let p = super::project();
    p.imported().await;
    let venues = p.dir.path().join("lighting/venues");
    let mut shared = std::fs::read_to_string(venues.join("kellys.venue")).unwrap();
    shared.push_str("venue \"Annex\" {\n}\n");
    std::fs::remove_file(venues.join("kellys.venue")).unwrap();
    std::fs::write(venues.join("all.venue"), &shared).unwrap();

    for write in [false, true] {
        let refused = p
            .import(
                "Kellys.mvr",
                &synthetic_mvr(true),
                write,
                options(Some("kellys"), None),
            )
            .await
            .unwrap_err();
        assert_eq!(refused.code(), Code::FailedPrecondition, "{refused}");
        assert_eq!(kind_of(&refused).as_deref(), Some("venue"));
        assert_eq!(
            refused.message(),
            "cannot import venue \"kellys\": Defined in all.venue alongside venue \
             \"Annex\"; edit that file by hand."
        );
    }
    // The inspection says so too, so the wizard can ask for another name.
    let refused = p
        .inspect(
            "Kellys.mvr",
            &synthetic_mvr(true),
            options(Some("kellys"), None),
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused}");
    assert_eq!(kind_of(&refused).as_deref(), Some("venue"));
    assert!(
        refused.message().contains("alongside venue \"Annex\""),
        "{}",
        refused.message()
    );
    assert_eq!(
        std::fs::read_to_string(venues.join("all.venue")).unwrap(),
        shared
    );
    assert!(!venues.join("kellys.venue").exists());
}

#[tokio::test]
async fn an_embedded_gdtf_s_taken_record_is_a_record_conflict_not_a_venue_one() {
    let p = super::project();
    let types = p.dir.path().join("lighting/fixture_types");
    std::fs::create_dir_all(&types).unwrap();
    let par = |name: &str| {
        format!("fixture_type \"{name}\" {{\n  channels: 1\n  channel_map: {{\"dimmer\": 1}}\n}}\n")
    };
    std::fs::write(types.join("synth_brick.light"), par("Synth Brick")).unwrap();
    std::fs::write(types.join("synth_brick_astera_pb15.fixture"), par("Other")).unwrap();

    let refused = p
        .inspect(
            "Kellys.mvr",
            &synthetic_mvr(true),
            options(Some("kellys"), None),
        )
        .await
        .unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition, "{refused}");
    assert_eq!(kind_of(&refused).as_deref(), Some("record"));
    assert!(
        refused
            .message()
            .starts_with("lighting/fixture_types/synth_brick_astera_pb15.fixture holds \"Other\""),
        "{}",
        refused.message()
    );
}

#[tokio::test]
async fn a_venues_directory_that_cannot_be_read_is_not_a_venue_refusal() {
    use std::os::unix::fs::PermissionsExt;
    let p = super::project();
    let venues = p.dir.path().join("lighting/venues");
    std::fs::create_dir_all(&venues).unwrap();
    // Written to and walked through, never listed.
    std::fs::set_permissions(&venues, std::fs::Permissions::from_mode(0o300)).unwrap();
    if std::fs::read_dir(&venues).is_ok() {
        std::fs::set_permissions(&venues, std::fs::Permissions::from_mode(0o755)).unwrap();
        return; // running as root
    }

    let refused = p
        .inspect(
            "Kellys.mvr",
            &synthetic_mvr(true),
            options(Some("kellys"), None),
        )
        .await
        .unwrap_err();
    std::fs::set_permissions(&venues, std::fs::Permissions::from_mode(0o755)).unwrap();

    assert_eq!(refused.code(), Code::Internal, "{refused}");
    assert_eq!(kind_of(&refused).as_deref(), Some("unreadable"));
    assert!(
        refused.message().contains("cannot be read"),
        "{}",
        refused.message()
    );
}

/// The imported venue with `mode "<mode>"` written onto one fixture's line
/// ... as `line_of` finds it.
fn with_line_mode(text: &str, fixture: &str, mode: &str) -> String {
    let needle = format!("fixture \"{fixture}\" ");
    text.lines()
        .map(|line| {
            let Some(at) = line.find(&needle) else {
                return line.to_string();
            };
            let from = at
                + line[at..]
                    .find(" mode \"")
                    .unwrap_or_else(|| panic!("no mode on {line}"))
                + " mode \"".len();
            let to = from + line[from..].find('"').unwrap();
            format!("{}{mode}{}", &line[..from], &line[to..])
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n"
}

fn line_of(text: &str, fixture: &str) -> String {
    text.lines()
        .find(|l| l.contains(&format!("fixture \"{fixture}\"")))
        .unwrap_or_else(|| panic!("no line for {fixture} in {text}"))
        .to_string()
}

#[tokio::test]
async fn a_merge_keeps_a_hand_set_mode_when_asked_and_keeps_other_modes_regardless() {
    let p = super::project();
    p.imported().await;
    let path = p.kellys();
    let text = std::fs::read_to_string(&path).unwrap();
    // Brick 1 hand-moded.
    std::fs::write(&path, with_line_mode(&text, "Brick 1", "Mover 16bit")).unwrap();

    let mvr = synthetic_mvr(true);
    let body = p
        .import("Kellys.mvr", &mvr, false, kellys_options())
        .await
        .unwrap();
    let brick1 = body["plan"]["fixtures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|f| f["name"] == "Brick 1")
        .unwrap()
        .clone();
    assert!(
        brick1["overwrites"]
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["field"] == "mode"),
        "{brick1}"
    );

    let kept = proto::MvrOptions {
        keep: Some(keep(&[("Brick 1", &["mode"])])),
        ..kellys_options()
    };
    p.import("Kellys.mvr", &mvr, true, kept).await.unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        line_of(&after, "Brick 1").contains("mode \"Mover 16bit\""),
        "{after}"
    );
}

#[tokio::test]
async fn blank_texts_are_not_given() {
    let p = super::project();
    let body = p
        .import(
            "Kellys.mvr",
            &synthetic_mvr(true),
            false,
            proto::MvrOptions {
                name: Some("  ".to_string()),
                venues_dir: Some("".to_string()),
                fixture_types_dir: Some("  ".to_string()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(body["plan"]["venue_name"], "Kellys");
}

// ---- The claim of an MVR or GDTF upload ----

#[tokio::test]
async fn an_mvr_or_gdtf_upload_is_good_for_its_purpose_alone_and_once() {
    let p = super::project();
    let mvr = synthetic_mvr(true);
    let inspect_id = p
        .send(proto::UploadPurpose::MvrInspect, "k.mvr", &mvr)
        .await
        .unwrap();
    // An inspection's upload cannot be imported, nor a GDTF's inspected.
    let refused = p.import_id(&inspect_id, true, None).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    let refused = p.import_gdtf_id(&inspect_id, None).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    assert!(!p.dir.path().join("lighting").exists());
    // Still good for its own call, once.
    p.inspect_id(&inspect_id, None).await.unwrap();
    assert_eq!(
        p.inspect_id(&inspect_id, None).await.unwrap_err().code(),
        Code::NotFound
    );
    assert_eq!(
        p.inspect_id("0123456789abcdef0123456789abcdef", None)
            .await
            .unwrap_err()
            .code(),
        Code::NotFound
    );
    assert_eq!(p.staged(), 0);
}

#[tokio::test]
async fn a_body_shorter_than_announced_is_refused_at_the_claim() {
    let p = super::project();
    let config = crate::controller::grpc::config_service::ConfigServer::new(p.state.player.clone());
    use crate::proto::player::v1::config_service_server::ConfigService;
    let begun = config
        .begin_upload(Request::new(proto::BeginUploadRequest {
            purpose: proto::UploadPurpose::MvrImport.into(),
            file_name: "k.mvr".to_string(),
            size: Some(1000),
            song: None,
        }))
        .await
        .unwrap()
        .into_inner();
    let receive = p.transfers().receive(&begun.upload_id).unwrap();
    std::fs::write(receive.path(), &synthetic_mvr(true)[..10]).unwrap();
    receive.finish(10);
    let refused = p.import_id(&begun.upload_id, true, None).await.unwrap_err();
    assert_eq!(refused.code(), Code::FailedPrecondition);
    assert!(!p.dir.path().join("lighting").exists());
    assert_eq!(p.staged(), 0);
}

// ---- ExportMvr (GET /lighting/mvr/export) ----

#[tokio::test]
async fn an_export_is_staged_for_download_and_writes_nothing() {
    let p = super::project();
    p.imported().await;

    let staged = p
        .export(proto::ExportMvrRequest {
            layers_from_tags: true,
            ..export_of("kellys")
        })
        .await
        .unwrap();
    assert_eq!(staged.file_name, "kellys.mvr");
    assert!(staged.path.starts_with("/downloads/"), "{}", staged.path);
    let id = staged.path.trim_start_matches("/downloads/");
    assert_eq!(id.len(), 32);
    let (file, name) = p.transfers().download(id).unwrap();
    assert_eq!(name, "kellys.mvr");
    let bytes = std::fs::read(file).unwrap();
    assert_eq!(staged.size, bytes.len() as u64);
    assert!(
        crate::lighting::mvr::parse_archive(&bytes).is_ok(),
        "the download is an MVR"
    );
    assert!(
        !p.dir.path().join("lighting/export").exists(),
        "nothing is written to the project unless asked"
    );

    let named = p
        .export(proto::ExportMvrRequest {
            file: Some("tour.mvr".to_string()),
            ..export_of("kellys")
        })
        .await
        .unwrap();
    assert_eq!(named.file_name, "tour.mvr");
    assert!(!p.dir.path().join("lighting/export").exists());
    // Each export is its own download.
    assert_ne!(named.path, staged.path);
}

#[tokio::test]
async fn an_export_that_cannot_be_made_is_refused_and_stages_nothing() {
    let p = super::project();
    p.imported().await;
    for request in [
        proto::ExportMvrRequest {
            file: Some("../x.mvr".to_string()),
            ..export_of("kellys")
        },
        proto::ExportMvrRequest {
            file: Some("/etc/x.mvr".to_string()),
            ..export_of("kellys")
        },
        proto::ExportMvrRequest {
            file: Some("x.txt".to_string()),
            ..export_of("kellys")
        },
        export_of("nope"),
        export_of("../kellys"),
        export_of(""),
        proto::ExportMvrRequest {
            venues_dir: Some("../elsewhere".to_string()),
            ..export_of("kellys")
        },
        proto::ExportMvrRequest {
            fixture_types_dir: Some("/etc".to_string()),
            ..export_of("kellys")
        },
    ] {
        let refused = p.export(request.clone()).await.unwrap_err();
        assert_eq!(
            refused.code(),
            Code::InvalidArgument,
            "{request:?}: {refused}"
        );
    }
    assert_eq!(p.staged(), 0);
}

#[tokio::test]
async fn staged_downloads_count_toward_the_cap() {
    let p = super::project();
    p.imported().await;
    for n in 0..32 {
        p.export(export_of("kellys"))
            .await
            .unwrap_or_else(|e| panic!("{n}: {e}"));
    }
    let refused = p.export(export_of("kellys")).await.unwrap_err();
    assert_eq!(refused.code(), Code::ResourceExhausted);
}
