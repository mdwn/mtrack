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

//! The venue calls (`ListVenues`, `GetVenue`, `SaveVenue`, `DeleteVenue`,
//! `GetVenuePatch`, `GetVenueScene`), called as the service: each held by a
//! test that was the REST route's. The answers are read back as the JSON the
//! routes sent ([`super::json`]), so the assertions are the routes' own.

use serde_json::{json, Value};
use tonic::{Code, Request, Status};

use super::types::{brick, par, synthetic_gdtf_bytes};
use super::{json as read, Project};
use crate::lighting;
use crate::proto::player::v1::{self as proto, lighting_service_server::LightingService};

/// A venue of two fixtures, as the file holds it.
pub(super) fn venue_dsl(name: &str) -> String {
    format!(
        r#"venue "{name}" {{
  fixture "Spot1" GenericPar @ 1:1
  fixture "Spot2" GenericPar @ 1:5
}}"#
    )
}

/// The structured venue a save asks for, from the JSON the web UI used to
/// send: `fixtures` rows (`name`, `fixture_type`, `universe`,
/// `start_channel`, and optionally `tags`, `position`, `rotation`, `mode`,
/// `beam_angle`), `focus_points` and `source`.
pub(super) fn form(json: &Value) -> proto::LightingVenueForm {
    let point = |v: &Value| proto::LightingVec3 {
        x: v[0].as_f64().unwrap(),
        y: v[1].as_f64().unwrap(),
        z: v[2].as_f64().unwrap(),
    };
    let optional = |v: &Value| (!v.is_null()).then(|| point(v));
    proto::LightingVenueForm {
        fixtures: json["fixtures"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| proto::LightingFixture {
                name: f["name"].as_str().unwrap().to_string(),
                fixture_type: f["fixture_type"].as_str().unwrap().to_string(),
                universe: f["universe"].as_u64().unwrap() as u32,
                start_channel: f["start_channel"].as_u64().unwrap() as u32,
                tags: f["tags"]
                    .as_array()
                    .map(|t| t.iter().map(|t| t.as_str().unwrap().to_string()).collect())
                    .unwrap_or_default(),
                position: optional(&f["position"]),
                rotation: optional(&f["rotation"]),
                mode: f["mode"].as_str().map(str::to_string),
                beam_angle: f["beam_angle"].as_f64(),
            })
            .collect(),
        focus_points: json["focus_points"]
            .as_object()
            .map(|points| {
                points
                    .iter()
                    .map(|(name, p)| (name.clone(), point(p)))
                    .collect()
            })
            .unwrap_or_default(),
        source: (!json["source"].is_null()).then(|| proto::LightingVenueSource {
            mvr: json["source"]["mvr"].as_str().unwrap().to_string(),
            origin: optional(&json["source"]["origin"]),
        }),
    }
}

/// A save of the text `dsl` as `name` in `dir`.
pub(super) fn save_dsl(name: &str, dir: &str, dsl: &str) -> proto::SaveVenueRequest {
    proto::SaveVenueRequest {
        name: name.to_string(),
        dir: Some(dir.to_string()),
        body: Some(proto::save_venue_request::Body::Dsl(dsl.to_string())),
        ..Default::default()
    }
}

/// A save of the structured venue `json` as `name` in `dir`.
pub(super) fn save_form(name: &str, dir: &str, json: &Value) -> proto::SaveVenueRequest {
    proto::SaveVenueRequest {
        name: name.to_string(),
        dir: Some(dir.to_string()),
        body: Some(proto::save_venue_request::Body::Form(form(json))),
        ..Default::default()
    }
}

impl Project {
    pub async fn venues(&self, dir: Option<&str>) -> Result<Value, Status> {
        self.server
            .list_venues(Request::new(proto::ListVenuesRequest {
                dir: dir.map(str::to_string),
            }))
            .await
            .map(|r| read::venue_list(&r.into_inner()))
    }

    pub async fn venue(&self, name: &str, dir: Option<&str>) -> Result<Value, Status> {
        self.server
            .get_venue(Request::new(proto::GetVenueRequest {
                name: name.to_string(),
                dir: dir.map(str::to_string),
            }))
            .await
            .map(|r| read::venue_detail(&r.into_inner()))
    }

    pub async fn put_venue(&self, request: proto::SaveVenueRequest) -> Result<Value, Status> {
        self.server
            .save_venue(Request::new(request))
            .await
            .map(|r| read::venue_saved(&r.into_inner()))
    }

    pub async fn remove_venue(&self, name: &str, dir: Option<&str>) -> Result<(), Status> {
        self.server
            .delete_venue(Request::new(proto::DeleteVenueRequest {
                name: name.to_string(),
                dir: dir.map(str::to_string),
            }))
            .await
            .map(|r| assert_eq!(r.into_inner().name, name))
    }

    pub async fn patch(
        &self,
        name: &str,
        dir: Option<&str>,
        types: Option<&str>,
    ) -> Result<Value, Status> {
        self.server
            .get_venue_patch(Request::new(proto::GetVenuePatchRequest {
                name: name.to_string(),
                dir: dir.map(str::to_string),
                fixture_types_dir: types.map(str::to_string),
            }))
            .await
            .map(|r| read::venue_patch(&r.into_inner()))
    }

    pub async fn scene(
        &self,
        name: &str,
        dir: Option<&str>,
        types: Option<&str>,
    ) -> Result<Value, Status> {
        self.server
            .get_venue_scene(Request::new(proto::GetVenueSceneRequest {
                name: name.to_string(),
                dir: dir.map(str::to_string),
                fixture_types_dir: types.map(str::to_string),
            }))
            .await
            .map(|r| read::venue_scene(&r.into_inner()))
    }
}

/// A native type, as a `.light` holds it.
fn fixture_type_dsl(name: &str) -> String {
    par(name)
}

// ---- ListVenues ----

#[tokio::test]
async fn an_empty_directory_lists_no_venues() {
    let p = super::project();
    p.make_dir("v_empty");
    let parsed = p.venues(Some("v_empty")).await.unwrap();
    assert!(parsed["venues"].is_object());
    assert_eq!(parsed["venues"].as_object().unwrap().len(), 0);
}

#[tokio::test]
async fn a_directory_that_is_not_there_lists_no_venues() {
    let p = super::project();
    let parsed = p.venues(Some("nonexistent_venues")).await.unwrap();
    assert_eq!(parsed["venues"].as_object().unwrap().len(), 0);
    assert!(parsed["errors"].as_array().unwrap().is_empty());
    assert!(parsed["versions"].as_object().unwrap().is_empty());
}

#[tokio::test]
async fn the_listing_has_each_venue_in_its_file() {
    let p = super::project();
    let dir = p.make_dir("v_test");
    std::fs::write(dir.join("club.light"), venue_dsl("Club")).unwrap();
    let parsed = p.venues(Some("v_test")).await.unwrap();
    assert!(parsed["venues"]["Club"].is_object());
    assert_eq!(
        parsed["venues"]["Club"]["fixtures"]["Spot2"]["start_channel"],
        5
    );
    assert_eq!(
        parsed["owners"]["Club"],
        json!({"file": "club.light", "editable": true, "reason": null})
    );
}

#[tokio::test]
async fn several_files_list_every_venue() {
    let p = super::project();
    let dir = p.make_dir("v_multi");
    std::fs::write(dir.join("a.light"), venue_dsl("VenueA")).unwrap();
    std::fs::write(dir.join("b.light"), venue_dsl("VenueB")).unwrap();
    let parsed = p.venues(Some("v_multi")).await.unwrap();
    let venues = parsed["venues"].as_object().unwrap();
    assert_eq!(venues.len(), 2);
    assert!(venues.contains_key("VenueA"));
    assert!(venues.contains_key("VenueB"));
}

#[tokio::test]
async fn a_name_defined_twice_is_reported_against_both_files() {
    // The same shape as the fixture types: a venue directory holds both
    // extensions, so two files can claim one name.
    let p = super::project();
    let dir = p.make_dir("venue_dupes");
    std::fs::write(dir.join("a.light"), venue_dsl("MainHall")).unwrap();
    std::fs::write(dir.join("b.venue"), venue_dsl("MainHall")).unwrap();
    std::fs::write(dir.join("c.light"), venue_dsl("Club")).unwrap();
    let parsed = p.venues(Some("venue_dupes")).await.unwrap();
    let errors = parsed["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{parsed}");
    let message = errors[0]["error"].as_str().unwrap();
    assert!(message.contains("venue \"MainHall\""), "{message}");
    assert!(message.contains("defined in both"), "{message}");
    assert!(message.contains("a.light"), "{message}");
    assert!(message.contains("b.venue"), "{message}");
    // One bad pair does not empty the list.
    assert!(parsed["venues"]["Club"].is_object(), "{parsed}");
    assert!(parsed["venues"]["MainHall"].is_object(), "{parsed}");
}

#[tokio::test]
async fn the_list_carries_each_venues_revision() {
    let p = super::project();
    let dir = p.make_dir("v_files");
    std::fs::write(dir.join("house.venue"), HOUSE).unwrap();
    let got = p.venues(Some("v_files")).await.unwrap();
    assert_eq!(
        got["versions"]["House"].as_str().unwrap(),
        crate::webui::api::lighting_api::content_version(HOUSE.as_bytes())
    );
}

/// The engine and the listing find the same venues: the first file to claim
/// a name keeps it, subdirectories included.
#[tokio::test]
async fn the_engine_and_the_venues_list_read_the_same_tree() {
    let p = super::project();
    project_with_nested_types(p.dir.path(), "ft_agree");
    let venues = p.dir.path().join("v_agree");
    std::fs::create_dir_all(venues.join("tour/old")).unwrap();
    std::fs::write(venues.join("house.light"), venue_dsl("house")).unwrap();
    std::fs::write(venues.join("tour/club.light"), venue_dsl("club")).unwrap();
    std::fs::write(venues.join("tour/old/house.light"), venue_dsl("house")).unwrap();

    let mut system = lighting::system::LightingSystem::new();
    system
        .load(
            &crate::config::Lighting::new(
                None,
                None,
                Some(crate::config::lighting::Directories::new(
                    Some("ft_agree".into()),
                    Some("v_agree".into()),
                )),
            ),
            p.dir.path(),
        )
        .unwrap();
    let mut engine_venues: Vec<String> =
        system.venues_iter().map(|(name, _)| name.clone()).collect();
    engine_venues.sort();

    let list = p.venues(Some("v_agree")).await.unwrap();
    let mut listed: Vec<String> = list["venues"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    listed.sort();
    assert_eq!(engine_venues, listed);
    assert_eq!(listed, ["club", "house"]);
}

// ---- GetVenue ----

#[tokio::test]
async fn a_venue_is_read_with_its_file_and_revision() {
    let p = super::project();
    let dir = p.make_dir("v_get");
    std::fs::write(dir.join("club.light"), venue_dsl("Club")).unwrap();
    let parsed = p.venue("Club", Some("v_get")).await.unwrap();
    assert!(parsed["venue"].is_object());
    assert_eq!(parsed["dsl"], venue_dsl("Club"));
    assert_eq!(
        parsed["version"].as_str().unwrap(),
        crate::webui::api::lighting_api::content_version(venue_dsl("Club").as_bytes())
    );
}

#[tokio::test]
async fn a_venue_that_is_not_there_is_not_found() {
    let p = super::project();
    p.make_dir("v_notfound");
    let status = p
        .venue("nonexistent", Some("v_notfound"))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
    assert!(status.message().contains("nonexistent"), "{status:?}");
}

// ---- SaveVenue ----

#[tokio::test]
async fn a_venue_saved_as_text_is_a_new_venue_file() {
    let p = super::project();
    let saved = p
        .put_venue(save_dsl("MyVenue", "v_put_raw", &venue_dsl("MyVenue")))
        .await
        .unwrap();
    assert_eq!(saved["status"], "saved");
    assert_eq!(saved["name"], "MyVenue");
    assert!(p.dir.path().join("v_put_raw/myvenue.venue").exists());
}

#[tokio::test]
async fn a_venue_saved_as_a_structured_venue_is_written_as_text_that_parses() {
    let p = super::project();
    let body = json!({
        "fixtures": [
            {"name": "Spot1", "fixture_type": "GenericPar", "universe": 1, "start_channel": 1},
            {"name": "Spot2", "fixture_type": "GenericPar", "universe": 1, "start_channel": 5},
        ]
    });
    let saved = p
        .put_venue(save_form("JSONVenue", "v_put_json", &body))
        .await
        .unwrap();
    assert_eq!(saved["status"], "saved");
    let path = p.dir.path().join("v_put_json/jsonvenue.venue");
    assert!(path.exists());
    let venues = lighting::parser::parse_venues(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!(venues.contains_key("JSONVenue"));
}

#[tokio::test]
async fn a_text_that_declares_another_venue_is_refused() {
    let p = super::project();
    let status = p
        .put_venue(save_dsl(
            "MyVenue",
            "venue_rename",
            &venue_dsl("OtherVenue"),
        ))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(status.message().contains("declares no venue"), "{status:?}");
    assert!(!p.dir.path().join("venue_rename/myvenue.light").exists());
}

#[tokio::test]
async fn a_text_that_does_not_parse_is_refused() {
    let p = super::project();
    let status = p
        .put_venue(save_dsl("BadDSL", "v_bad", "invalid {{{ content"))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn a_save_with_no_body_is_the_empty_text_and_does_not_parse() {
    let p = super::project();
    let status = p
        .put_venue(proto::SaveVenueRequest {
            name: "Empty".to_string(),
            dir: Some("v_none".to_string()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn a_name_that_is_not_a_plain_name_is_refused() {
    let p = super::project();
    let status = p
        .put_venue(save_dsl("../evil", "v_test", "content"))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn a_text_save_keeps_its_modes() {
    let p = super::project();
    let dsl = "venue \"Rig\" {\n  fixture \"B1\" Brick mode \"Mover 16bit\" @ 1:1\n}\n";
    p.put_venue(save_dsl("Rig", "v_put_mode_dsl", dsl))
        .await
        .unwrap();
    let path = p.dir.path().join("v_put_mode_dsl/rig.venue");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), dsl);
}

#[tokio::test]
async fn a_structured_save_with_geometry_lands_in_a_venue_file() {
    let p = super::project();
    let rel = "v_put_geometry";
    // First a plain venue, written by hand as a .light.
    let dir = p.make_dir(rel);
    std::fs::write(
        dir.join("geo.light"),
        "venue \"Geo\" {\n  fixture \"Spot1\" GenericPar @ 1:1\n}\n",
    )
    .unwrap();

    // Then the same venue gains a position and a focus point: it moves to
    // .venue and the .light twin is retired.
    let geometry = json!({
        "fixtures": [{
            "name": "Spot1", "fixture_type": "GenericPar", "universe": 1, "start_channel": 1,
            "tags": ["spot"], "position": [-2.0, 3.5, 4.2], "rotation": [0, 0, 180]
        }],
        "focus_points": {"drummer": [0.0, 2.8, 1.4]},
        "source": {"mvr": "lighting/library/geo.mvr", "origin": [0, -3.5, 0]}
    });
    p.put_venue(save_form("Geo", rel, &geometry)).await.unwrap();
    let venue_path = dir.join("geo.venue");
    assert!(venue_path.exists());
    assert!(!dir.join("geo.light").exists());
    let content = std::fs::read_to_string(&venue_path).unwrap();
    let venue = &lighting::parser::parse_venues(&content).unwrap()["Geo"];
    assert_eq!(venue.fixtures()["Spot1"].position(), Some([-2.0, 3.5, 4.2]));
    assert_eq!(venue.focus_points()["drummer"], [0.0, 2.8, 1.4]);
    assert_eq!(venue.source().unwrap().mvr, "lighting/library/geo.mvr");

    // GetVenue resolves the .venue and carries the geometry back.
    let body = p.venue("Geo", Some(rel)).await.unwrap();
    assert_eq!(
        body["venue"]["fixtures"]["Spot1"]["position"],
        json!([-2.0, 3.5, 4.2])
    );
    assert_eq!(
        body["venue"]["focus_points"]["drummer"],
        json!([0.0, 2.8, 1.4])
    );
    assert_eq!(body["venue"]["source"]["mvr"], "lighting/library/geo.mvr");

    // The listing sees it, and the delete finds it.
    let body = p.venues(Some(rel)).await.unwrap();
    assert!(body["venues"]["Geo"].is_object(), "{body}");
    p.remove_venue("Geo", Some(rel)).await.unwrap();
    assert!(!venue_path.exists());
}

#[tokio::test]
async fn a_structured_save_of_a_new_venue_writes_each_fixture_s_mode() {
    let p = super::project();
    let rel = "v_put_mode_new";
    let body = json!({
        "fixtures": [
            {"name": "B1", "fixture_type": "Brick", "universe": 1, "start_channel": 1,
             "mode": "Mover 16bit"},
            {"name": "B2", "fixture_type": "Brick", "universe": 1, "start_channel": 20},
        ]
    });
    p.put_venue(save_form("Rig", rel, &body)).await.unwrap();
    // A mode is `.venue` syntax: the file is born a .venue.
    let path = p.dir.path().join(rel).join("rig.venue");
    assert!(path.exists(), "a moded venue is a .venue");
    assert!(!p.dir.path().join(rel).join("rig.light").exists());
    let venue =
        &lighting::parser::parse_venues(&std::fs::read_to_string(&path).unwrap()).unwrap()["Rig"];
    assert_eq!(venue.fixtures()["B1"].mode(), Some("Mover 16bit"));
    assert_eq!(venue.fixtures()["B2"].mode(), None);
}

#[tokio::test]
async fn a_light_venue_that_gains_a_mode_moves_to_venue() {
    let p = super::project();
    let rel = "v_put_mode_ext";
    // A `.light` venue written by hand (a new one is a `.venue`).
    let dir = p.make_dir(rel);
    std::fs::write(
        dir.join("rig.light"),
        "venue \"Rig\" {\n  fixture \"B1\" Brick @ 1:1\n}\n",
    )
    .unwrap();
    let moded = json!({
        "fixtures": [{"name": "B1", "fixture_type": "Brick", "universe": 1,
                      "start_channel": 1, "mode": "8: RGBS"}]
    });
    p.put_venue(save_form("Rig", rel, &moded)).await.unwrap();
    let path = dir.join("rig.venue");
    assert!(path.exists());
    assert!(!dir.join("rig.light").exists());
    assert!(std::fs::read_to_string(&path)
        .unwrap()
        .contains("fixture \"B1\" Brick mode \"8: RGBS\" @ 1:1"));
}

#[tokio::test]
async fn a_structured_save_of_an_existing_venue_changes_or_clears_only_the_mode_it_is_given() {
    let p = super::project();
    let rel = "v_put_mode_patch";
    let dir = p.make_dir(rel);
    let path = dir.join("rig.venue");
    std::fs::write(
        &path,
        "# mine\nvenue \"Rig\" {\n  fixture \"B1\" Brick mode \"Mover 16bit\" @ 1:1  # keep\n  \
         fixture \"B2\" Brick @ 1:20\n}\n",
    )
    .unwrap();
    // The inspector's save: B2 gains a mode, B1 keeps its own.
    let body = json!({
        "fixtures": [
            {"name": "B1", "fixture_type": "Brick", "universe": 1, "start_channel": 1,
             "mode": "Mover 16bit"},
            {"name": "B2", "fixture_type": "Brick", "universe": 1, "start_channel": 20,
             "mode": "8: RGBS"},
        ]
    });
    p.put_venue(save_form("Rig", rel, &body)).await.unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("  fixture \"B1\" Brick mode \"Mover 16bit\" @ 1:1  # keep\n"),
        "{after}"
    );
    assert!(
        after.contains("fixture \"B2\" Brick mode \"8: RGBS\" @ 1:20"),
        "{after}"
    );

    // Back to the type default: the line loses its mode.
    let body = json!({
        "fixtures": [
            {"name": "B1", "fixture_type": "Brick", "universe": 1, "start_channel": 1,
             "mode": "Mover 16bit"},
            {"name": "B2", "fixture_type": "Brick", "universe": 1, "start_channel": 20},
        ]
    });
    p.put_venue(save_form("Rig", rel, &body)).await.unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("fixture \"B2\" Brick @ 1:20"), "{after}");
    assert!(after.contains("mode \"Mover 16bit\""), "{after}");
}

/// A structured save that must be refused, with its error: nothing is
/// written.
async fn refused_venue_save(fixtures: Value, rel: &str) -> String {
    let p = super::project();
    let status = p
        .put_venue(save_form("Rig", rel, &json!({ "fixtures": fixtures })))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(!p.dir.path().join(rel).exists(), "nothing is written");
    status.message().to_string()
}

#[tokio::test]
async fn a_venue_save_with_two_fixtures_of_one_name_is_refused_by_name() {
    let error = refused_venue_save(
        json!([
            {"name": "Spot", "fixture_type": "Par", "universe": 1, "start_channel": 1},
            {"name": "Spot", "fixture_type": "Par", "universe": 1, "start_channel": 5},
        ]),
        "v_dup_names",
    )
    .await;
    assert!(error.contains("'Spot'"), "{error}");
    assert!(error.contains("more than once"), "{error}");
}

#[tokio::test]
async fn a_venue_save_with_a_nameless_or_typeless_fixture_is_refused() {
    let error = refused_venue_save(
        json!([
            {"name": "  ", "fixture_type": "Par", "universe": 1, "start_channel": 1},
        ]),
        "v_no_name",
    )
    .await;
    assert!(error.contains("no name"), "{error}");
    let error = refused_venue_save(
        json!([
            {"name": "Spot", "fixture_type": "", "universe": 1, "start_channel": 1},
        ]),
        "v_no_type",
    )
    .await;
    assert!(error.contains("no fixture type"), "{error}");
    let error = refused_venue_save(
        json!([
            {"name": "Spot", "fixture_type": "Par", "universe": 0, "start_channel": 1},
        ]),
        "v_zero_universe",
    )
    .await;
    assert!(error.contains("universe"), "{error}");
    let error = refused_venue_save(
        json!([
            {"name": "Spot", "fixture_type": "Par", "universe": 1, "start_channel": 0},
        ]),
        "v_zero_channel",
    )
    .await;
    assert!(error.contains("start channel"), "{error}");
    // Out of the range a universe or an address has.
    let error = refused_venue_save(
        json!([
            {"name": "Spot", "fixture_type": "Par", "universe": 70000, "start_channel": 1},
        ]),
        "v_big_universe",
    )
    .await;
    assert!(error.contains("out of range"), "{error}");
}

#[tokio::test]
async fn a_blank_mode_is_refused_and_a_figure_that_is_not_a_number_is_too() {
    let p = super::project();
    for (fixture, what) in [
        (
            json!({"name": "A", "fixture_type": "Par", "universe": 1, "start_channel": 1,
                   "mode": "  "}),
            "'mode'",
        ),
        (
            json!({"name": "A", "fixture_type": "Par", "universe": 1, "start_channel": 1,
                   "beam_angle": 0.0}),
            "'beam_angle'",
        ),
    ] {
        let status = p
            .put_venue(save_form(
                "Rig",
                "v_bad_fig",
                &json!({"fixtures": [fixture]}),
            ))
            .await
            .unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument, "{what}");
        assert!(status.message().contains(what), "{status:?}");
    }
    assert!(!p.dir.path().join("v_bad_fig").exists());
}

// ---- A save against a file, and its revision ----

const HOUSE: &str = "// House rig, do not lose me.\nvenue \"House\" {\n  fixture \"A\" brick @ 1:1 position (1, 2, 3)  // keep me\n  // spare\n  fixture \"B\" brick @ 1:5\n}\n";

const REL: &str = "v_files";

fn house_body(x: f64) -> Value {
    json!({
        "fixtures": [
            {"name": "A", "fixture_type": "brick", "universe": 1, "start_channel": 1,
             "position": [x, 2.0, 3.0]},
            {"name": "B", "fixture_type": "brick", "universe": 1, "start_channel": 5},
        ]
    })
}

async fn put_house(p: &Project, revision: Option<&str>, x: f64) -> Result<Value, Status> {
    let mut request = save_form("House", REL, &house_body(x));
    request.expected_revision = revision.map(str::to_string);
    p.put_venue(request).await
}

#[tokio::test]
async fn a_venue_in_a_file_with_another_stem_is_patched_there() {
    let p = super::project();
    let d = p.make_dir(REL);
    std::fs::write(d.join("rig_2024.venue"), HOUSE).unwrap();

    put_house(&p, None, 9.0).await.unwrap();
    assert_eq!(
        std::fs::read_dir(&d).unwrap().count(),
        1,
        "no second file may be created"
    );
    let text = std::fs::read_to_string(d.join("rig_2024.venue")).unwrap();
    assert!(
        text.starts_with("// House rig, do not lose me.\n"),
        "{text}"
    );
    assert!(text.contains("// keep me"), "{text}");
    assert!(text.contains("\n  // spare\n"), "{text}");
    assert!(text.contains("position (9, 2, 3)"), "{text}");
}

#[tokio::test]
async fn a_broken_file_is_never_searched_and_left_byte_identical() {
    let p = super::project();
    let d = p.make_dir(REL);
    let broken = "// mine\nvenue \"House\" {\n  fixture \"A\" brick @ !!!\n}\n";
    std::fs::write(d.join("rig.venue"), broken).unwrap();

    // Nothing is guessed from a file that does not parse: "House" is
    // declared nowhere, so it is new, at its own canonical file.
    put_house(&p, None, 1.0).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(d.join("rig.venue")).unwrap(),
        broken
    );
    assert!(d.join("house.venue").exists());
}

#[tokio::test]
async fn a_broken_file_at_the_canonical_path_is_refused_and_left_byte_identical() {
    let p = super::project();
    let d = p.make_dir(REL);
    let broken = "// mine\nvenue \"House\" {\n  fixture \"A\" brick @ !!!\n}\n";
    std::fs::write(d.join("house.venue"), broken).unwrap();

    let status = put_house(&p, None, 1.0).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert!(status.message().contains("house.venue"), "{status:?}");
    assert_eq!(
        std::fs::read_to_string(d.join("house.venue")).unwrap(),
        broken
    );
    assert_eq!(std::fs::read_dir(&d).unwrap().count(), 1);
}

#[tokio::test]
async fn a_non_utf8_file_at_the_stem_is_refused() {
    let p = super::project();
    let d = p.make_dir(REL);
    let bytes: Vec<u8> = vec![0xff, 0xfe, b'v', b'e'];
    std::fs::write(d.join("house.venue"), &bytes).unwrap();

    let status = put_house(&p, None, 1.0).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert_eq!(std::fs::read(d.join("house.venue")).unwrap(), bytes);
}

#[tokio::test]
async fn a_new_venue_is_generated_and_answers_its_revision() {
    let p = super::project();
    p.make_dir(REL);
    let body = put_house(&p, None, 1.0).await.unwrap();
    assert!(body["version"].as_str().is_some_and(|v| v.len() == 64));
    assert!(p.dir.path().join(REL).join("house.venue").exists());
}

#[tokio::test]
async fn a_stale_revision_is_aborted_and_the_current_one_saves() {
    let p = super::project();
    let d = p.make_dir(REL);
    std::fs::write(d.join("house.venue"), HOUSE).unwrap();

    // GetVenue hands out the revision.
    let got = p.venue("House", Some(REL)).await.unwrap();
    let v1 = got["version"].as_str().unwrap().to_string();

    // Another writer changes the file.
    let saved = put_house(&p, Some(&v1), 4.0).await.unwrap();
    let v2 = saved["version"].as_str().unwrap().to_string();
    assert_ne!(v1, v2);
    let after_first = std::fs::read_to_string(d.join("house.venue")).unwrap();

    // The first reader saves against the revision it read: aborted, file
    // untouched, the current revision in the metadata.
    let status = put_house(&p, Some(&v1), 7.0).await.unwrap_err();
    assert_eq!(status.code(), Code::Aborted);
    assert_eq!(
        status.metadata().get("x-current-version").unwrap(),
        v2.as_str()
    );
    assert_eq!(
        std::fs::read_to_string(d.join("house.venue")).unwrap(),
        after_first
    );

    // The current revision is accepted.
    put_house(&p, Some(&v2), 7.0).await.unwrap();
    assert!(std::fs::read_to_string(d.join("house.venue"))
        .unwrap()
        .contains("position (7, 2, 3)"));
}

#[tokio::test]
async fn an_empty_expected_revision_is_not_checked() {
    let p = super::project();
    let d = p.make_dir(REL);
    std::fs::write(d.join("house.venue"), HOUSE).unwrap();
    put_house(&p, Some(""), 3.0).await.unwrap();
    assert!(std::fs::read_to_string(d.join("house.venue"))
        .unwrap()
        .contains("position (3, 2, 3)"));
}

#[tokio::test]
async fn a_text_save_is_checked_against_the_revision_too() {
    let p = super::project();
    let d = p.make_dir(REL);
    std::fs::write(d.join("house.venue"), HOUSE).unwrap();
    let mut request = save_dsl("House", REL, HOUSE);
    request.expected_revision = Some("not-the-revision".to_string());
    let status = p.put_venue(request).await.unwrap_err();
    assert_eq!(status.code(), Code::Aborted);
    assert_eq!(
        std::fs::read_to_string(d.join("house.venue")).unwrap(),
        HOUSE
    );
}

// ---- The venue a save leaves failing ----

/// A project whose config names `rig` as the current venue, with the
/// synthetic archive, a `Brick` type and a venues directory.
fn project_with_current_venue(p: &Project) {
    project_with_bricks(p.dir.path(), "lighting/fixture_types", "lighting/venues");
    p.write_config(
        "songs: songs\ndmx:\n  universes:\n    - universe: 1\n      name: main\n  \
         lighting:\n    current_venue: rig\n    directories:\n      \
         fixture_types: lighting/fixture_types\n      venues: lighting/venues\n",
    );
}

#[tokio::test]
async fn a_save_that_leaves_the_current_venue_failing_says_so_and_still_saves() {
    let p = super::project();
    project_with_current_venue(&p);
    let body = json!({
        "fixtures": [
            {"name": "B1", "fixture_type": "Brick", "universe": 1, "start_channel": 1,
             "mode": "8: RGBS"},
            {"name": "B2", "fixture_type": "Brick", "universe": 1, "start_channel": 20,
             "mode": "Nope"},
        ]
    });
    let request = proto::SaveVenueRequest {
        name: "rig".to_string(),
        body: Some(proto::save_venue_request::Body::Form(form(&body))),
        ..Default::default()
    };
    let parsed = p.put_venue(request).await.unwrap();
    assert_eq!(parsed["venue_error"]["venue"], "rig", "{parsed}");
    assert_eq!(parsed["venue_error"]["fixture"], "B2", "{parsed}");
    assert!(
        parsed["venue_error"]["reason"]
            .as_str()
            .unwrap()
            .contains("Nope"),
        "{parsed}"
    );
    assert!(p.dir.path().join("lighting/venues/rig.venue").exists());

    // Fixed: the error goes.
    let body = json!({
        "fixtures": [
            {"name": "B1", "fixture_type": "Brick", "universe": 1, "start_channel": 1,
             "mode": "8: RGBS"},
            {"name": "B2", "fixture_type": "Brick", "universe": 1, "start_channel": 20,
             "mode": "Mover 16bit"},
        ]
    });
    let request = proto::SaveVenueRequest {
        name: "rig".to_string(),
        body: Some(proto::save_venue_request::Body::Form(form(&body))),
        ..Default::default()
    };
    let parsed = p.put_venue(request).await.unwrap();
    assert!(parsed["venue_error"].is_null(), "{parsed}");
}

#[tokio::test]
async fn a_save_of_a_venue_that_is_not_current_reports_nothing() {
    let p = super::project();
    project_with_current_venue(&p);
    let body = json!({
        "fixtures": [{"name": "B1", "fixture_type": "Brick", "universe": 1,
                      "start_channel": 1, "mode": "Nope"}]
    });
    let request = proto::SaveVenueRequest {
        name: "other".to_string(),
        body: Some(proto::save_venue_request::Body::Form(form(&body))),
        ..Default::default()
    };
    let parsed = p.put_venue(request).await.unwrap();
    assert!(parsed["venue_error"].is_null(), "{parsed}");
    assert_eq!(parsed["reloaded"], false);
}

// ---- Venues in subdirectories, and a rename ----

#[tokio::test]
async fn a_venue_in_a_subdirectory_lists_opens_saves_in_place_and_deletes() {
    let p = super::project();
    let venues = p.make_dir("v_nested");
    std::fs::create_dir_all(venues.join("tour/old")).unwrap();
    std::fs::write(venues.join("house.light"), venue_dsl("house")).unwrap();
    std::fs::write(venues.join("tour/club.light"), venue_dsl("club")).unwrap();
    std::fs::write(venues.join("tour/old/house.light"), venue_dsl("house")).unwrap();

    let list = p.venues(Some("v_nested")).await.unwrap();
    assert!(list["venues"]["club"].is_object(), "{list}");
    let errors = list["errors"].as_array().unwrap();
    assert!(
        errors.iter().any(|e| e["file"] == "tour/old/house.light"
            && e["error"]
                .as_str()
                .unwrap()
                .contains("\"house\" is defined in both house.light and tour/old/house.light")),
        "{list}"
    );

    p.venue("club", Some("v_nested")).await.unwrap();

    let edited = venue_dsl("club").replace("@ 1:5", "@ 1:9");
    p.put_venue(save_dsl("club", "v_nested", &edited))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(venues.join("tour/club.light")).unwrap(),
        edited
    );
    assert!(
        !venues.join("club.light").exists(),
        "saved beside, not in place"
    );

    p.remove_venue("club", Some("v_nested")).await.unwrap();
    assert!(!venues.join("tour/club.light").exists());
}

#[tokio::test]
async fn a_rename_moves_the_file_to_the_new_name() {
    let p = super::project();
    let d = p.make_dir("v_rename");
    std::fs::write(d.join("old.venue"), venue_dsl("Old")).unwrap();
    let mut request = save_dsl("New", "v_rename", &venue_dsl("New"));
    request.from = Some("Old".to_string());
    p.put_venue(request).await.unwrap();
    assert!(d.join("new.venue").exists());
    assert!(!d.join("old.venue").exists());
    p.venue("New", Some("v_rename")).await.unwrap();
    assert_eq!(
        p.venue("Old", Some("v_rename")).await.unwrap_err().code(),
        Code::NotFound
    );
}

// ---- DeleteVenue ----

#[tokio::test]
async fn a_venue_is_deleted() {
    let p = super::project();
    let dir = p.make_dir("v_del");
    let file = dir.join("todelete.light");
    std::fs::write(&file, venue_dsl("ToDelete")).unwrap();
    p.remove_venue("ToDelete", Some("v_del")).await.unwrap();
    assert!(!file.exists());
}

#[tokio::test]
async fn deleting_a_venue_that_is_not_there_is_not_found() {
    let p = super::project();
    p.make_dir("v_del_nf");
    let status = p
        .remove_venue("nonexistent", Some("v_del_nf"))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
}

#[tokio::test]
async fn deleting_finds_an_imported_file_by_its_declaration() {
    let p = super::project();
    let dir = p.make_dir("v_del_imported");
    // Not the name's stem (`house-rig`).
    let file = dir.join("house_rig.light");
    std::fs::write(&file, venue_dsl("House-Rig")).unwrap();
    p.remove_venue("House-Rig", Some("v_del_imported"))
        .await
        .unwrap();
    assert!(!file.exists());
}

#[tokio::test]
async fn a_venue_in_a_shared_file_is_not_deleted() {
    let p = super::project();
    let dir = p.make_dir("v_del_shared");
    let shared = format!("{}\n{}", venue_dsl("A"), venue_dsl("B"));
    std::fs::write(dir.join("all.venue"), &shared).unwrap();
    let status = p.remove_venue("A", Some("v_del_shared")).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert!(status.message().contains("all.venue"), "{status:?}");
    assert_eq!(
        std::fs::read_to_string(dir.join("all.venue")).unwrap(),
        shared
    );
}

#[tokio::test]
async fn a_saved_venue_reads_back_and_a_deleted_one_is_gone() {
    let p = super::project();
    let rel = "v_roundtrip";
    p.put_venue(save_dsl("RoundTrip", rel, &venue_dsl("RoundTrip")))
        .await
        .unwrap();
    let parsed = p.venue("RoundTrip", Some(rel)).await.unwrap();
    assert!(parsed["venue"].is_object());

    p.remove_venue("RoundTrip", Some(rel)).await.unwrap();
    let status = p.venue("RoundTrip", Some(rel)).await.unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
}

// ---- GetVenuePatch ----

/// A project with the synthetic archive in `library/`, a `Brick` type
/// referring to it in `ft_rel`, and a venues directory `v_rel` patching
/// three Bricks (one in a second venue) beside a file that will not parse.
pub(super) fn project_with_bricks(project: &std::path::Path, ft_rel: &str, v_rel: &str) {
    let library = project.join("library");
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(library.join("synth.gdtf"), synthetic_gdtf_bytes()).unwrap();
    let ft = project.join(ft_rel);
    std::fs::create_dir_all(&ft).unwrap();
    std::fs::write(ft.join("brick.fixture"), brick("Brick")).unwrap();
    let venues = project.join(v_rel);
    std::fs::create_dir_all(&venues).unwrap();
    std::fs::write(
        venues.join("stage.light"),
        "venue \"stage\" {\n  fixture \"Brick2\" Brick mode \"8: RGBS\" @ 1:5\n  \
         fixture \"Brick1\" Brick mode \"8: RGBS\" @ 1:1\n  fixture \"Par\" GenericPar @ 1:20\n}\n",
    )
    .unwrap();
    std::fs::write(
        venues.join("club.venue"),
        // One fixture in another mode, spelled loosely (the archive's own
        // spelling comes back), and one in a mode it does not have.
        "venue \"club\" {\n  fixture \"Solo\" Brick mode \"mover 16BIT\" @ 2:1\n  \
         fixture \"Odd\" Brick mode \"Nope\" @ 2:40\n}\n",
    )
    .unwrap();
    std::fs::write(venues.join("broken.light"), "venue {{{").unwrap();
}

/// A fixture types directory with types at the top and in
/// subdirectories: a native type in each, a GDTF record in `rig/`, a second
/// declaration of `Top` further down, and an unrecorded archive in the
/// library.
fn project_with_nested_types(project: &std::path::Path, ft_rel: &str) {
    let library = project.join("library");
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(library.join("synth.gdtf"), synthetic_gdtf_bytes()).unwrap();
    let shelf = project.join("lighting/library");
    std::fs::create_dir_all(&shelf).unwrap();
    std::fs::write(shelf.join("loose.gdtf"), synthetic_gdtf_bytes()).unwrap();
    let ft = project.join(ft_rel);
    std::fs::create_dir_all(ft.join("rig/old")).unwrap();
    std::fs::write(ft.join("top.light"), fixture_type_dsl("Top")).unwrap();
    std::fs::write(ft.join("rig/wash.light"), fixture_type_dsl("Wash")).unwrap();
    std::fs::write(ft.join("rig/brick.fixture"), brick("Brick")).unwrap();
    std::fs::write(ft.join("rig/old/top.light"), fixture_type_dsl("Top")).unwrap();
}

#[tokio::test]
async fn the_patch_of_a_venue_comes_from_its_files_with_each_fixture_s_mode() {
    let p = super::project();
    project_with_bricks(p.dir.path(), "ft_patch", "v_patch");
    // A gang at 1:1, a partial overlap at 1:3, a fixture in a wider mode,
    // one whose mode does not load, and one past the universe end.
    std::fs::write(
        p.dir.path().join("v_patch").join("p.venue"),
        "venue \"p\" {\n  fixture \"G1\" Brick mode \"8: RGBS\" @ 1:1\n  \
         fixture \"G2\" Brick mode \"8: RGBS\" @ 1:1\n  \
         fixture \"Part\" Brick mode \"8: RGBS\" @ 1:3\n  \
         fixture \"Wide\" Brick mode \"Mover 16bit\" @ 1:100\n  \
         fixture \"Odd\" Brick mode \"Nope\" @ 1:200\n  fixture \"Bare\" Brick @ 1:300\n  \
         fixture \"End\" Brick mode \"8: RGBS\" @ 1:511\n}\n",
    )
    .unwrap();

    let parsed = p
        .patch("p", Some("v_patch"), Some("ft_patch"))
        .await
        .unwrap();
    let spans = parsed["spans"].as_array().unwrap();
    let span = |name: &str| spans.iter().find(|s| s["fixture"] == name).unwrap();
    let rgbs = span("G1")["footprint"].as_u64().unwrap();
    assert!(rgbs > 0, "{parsed}");
    assert_eq!(span("G1")["type"], "Brick");
    assert_eq!(span("G1")["mode"], "8: RGBS");
    assert_eq!(span("Wide")["mode"], "Mover 16bit");
    assert_ne!(span("Wide")["footprint"], span("G1")["footprint"]);
    assert!(
        span("Odd")["footprint"].is_null(),
        "never guessed: {parsed}"
    );
    assert!(
        span("Bare")["footprint"].is_null() && span("Bare")["mode"].is_null(),
        "a GDTF fixture with no mode has no footprint: {parsed}"
    );

    // The gang is quiet; the partial overlap names the gang once.
    let overlaps = parsed["overlaps"].as_array().unwrap();
    assert_eq!(overlaps.len(), 1, "{parsed}");
    assert_eq!(overlaps[0]["first_gang"], json!(["G1", "G2"]));
    assert_eq!(overlaps[0]["second"], "Part");
    let overruns = parsed["overruns"].as_array().unwrap();
    assert_eq!(overruns.len(), 1, "{parsed}");
    assert_eq!(overruns[0]["fixture"], "End");

    let status = p
        .patch("nowhere", Some("v_patch"), Some("ft_patch"))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
}

/// The engine and this service find a project's lighting files in the same
/// directories for every shape of config. The web UI sends a profile's
/// configured directory as `dir` and nothing otherwise; the engine resolves
/// through `config::lighting`. A first-run project (no `directories`) used
/// to have the UI save where the engine never read.
#[tokio::test]
async fn the_engine_and_the_service_resolve_the_same_directories() {
    use crate::config::lighting::{Directories, Lighting};
    /// `None`: no lighting section; `Some(None)`: no `directories`; else
    /// (fixture types, venues) as configured.
    type Section = Option<Option<(Option<&'static str>, Option<&'static str>)>>;
    let cases: [(&str, Section); 5] = [
        ("no lighting section", None),
        ("no directories", Some(None)),
        ("types set", Some(Some((Some("my/types"), None)))),
        ("venues set", Some(Some((None, Some("my/venues"))))),
        ("both set", Some(Some((Some("t2"), Some("v2"))))),
    ];
    for (case, lighting) in cases {
        let p = super::project();
        let root = p.dir.path();
        let section = lighting.map(|dirs| {
            Lighting::new(
                Some("v".to_string()),
                None,
                dirs.map(|(t, v)| Directories::new(t.map(str::to_string), v.map(str::to_string))),
            )
        });
        // Where the shared resolution says the files are.
        let types = crate::config::lighting::fixture_types_dir(section.as_ref());
        let venues = crate::config::lighting::venues_dir(section.as_ref());
        std::fs::create_dir_all(root.join(types)).unwrap();
        std::fs::create_dir_all(root.join(venues)).unwrap();
        std::fs::write(root.join(types).join("par.light"), fixture_type_dsl("Par")).unwrap();
        std::fs::write(
            root.join(venues).join("v.light"),
            "venue \"v\" {\n  fixture \"A\" Par @ 1:1\n}\n",
        )
        .unwrap();

        // The engine (a profile with a lighting section) loads them.
        if let Some(config) = &section {
            let mut system = lighting::system::LightingSystem::new();
            system.load(config, root).unwrap();
            assert_eq!(
                system.get_current_venue_fixtures().map(|f| f.len()).ok(),
                Some(1),
                "{case}: the engine finds the venue and its type"
            );
        }

        // The service, given what the UI sends for this profile.
        let configured = section.as_ref().and_then(|l| l.directories());
        let parsed = p
            .patch(
                "v",
                configured.and_then(|d| d.venues()),
                configured.and_then(|d| d.fixture_types()),
            )
            .await
            .unwrap_or_else(|e| panic!("{case}: {e:?}"));
        assert!(
            parsed["spans"][0]["footprint"].as_u64().is_some(),
            "{case}: the service finds the venue and its type: {parsed}"
        );
    }
}

// ---- GetVenueScene ----

#[tokio::test]
async fn the_scene_of_a_venue_comes_from_its_files_with_each_fixture_s_rig() {
    let p = super::project();
    project_with_bricks(p.dir.path(), "ft_scene", "v_scene");
    std::fs::write(
        p.dir.path().join("v_scene").join("s.venue"),
        "venue \"s\" {\n  fixture \"A\" Brick mode \"8: RGBS\" @ 1:1 position (-2, 2, 3) beam_angle 45\n  \
         fixture \"B\" Brick mode \"Mover 16bit\" @ 1:20\n  \
         fixture \"Odd\" Brick mode \"Nope\" @ 1:200\n}\n",
    )
    .unwrap();

    let parsed = p
        .scene("s", Some("v_scene"), Some("ft_scene"))
        .await
        .unwrap();
    assert_eq!(parsed["venue"]["name"], "s");
    assert!(parsed["venue"]["scenery"].is_null(), "{parsed}");
    let fixtures = &parsed["fixtures"];
    assert_eq!(fixtures.as_object().unwrap().len(), 3, "{parsed}");
    assert_eq!(fixtures["A"]["type"], "Brick");
    assert_eq!(fixtures["A"]["mode"], "8: RGBS");
    assert_eq!(fixtures["A"]["position"], json!([-2.0, 2.0, 3.0]));
    assert!(fixtures["B"]["position"].is_null(), "{parsed}");
    assert_eq!(fixtures["A"]["beam_angle"], 45.0);
    assert!(fixtures["B"]["beam_angle"].is_null(), "{parsed}");
    // Each (archive, mode) has its own rig, written to the store.
    let rig_a = fixtures["A"]["rig"].as_str().expect("a rig for A");
    let rig_b = fixtures["B"]["rig"].as_str().expect("a rig for B");
    assert_ne!(rig_a, rig_b);
    assert!(p
        .dir
        .path()
        .join("lighting/.cache/assets")
        .join(rig_a)
        .is_file());
    // A mode that does not load is drawn generically, not dropped.
    assert!(fixtures["Odd"]["rig"].is_null(), "{parsed}");

    let status = p
        .scene("nowhere", Some("v_scene"), Some("ft_scene"))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
}
