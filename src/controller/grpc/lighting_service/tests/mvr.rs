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

//! The MVR export calls (`GetMvrExportSummary`, `KeepMvrExport`) and the
//! aim points (`AddVenueAimPoints`), on a venue imported from a synthetic
//! MVR (the import itself is still an HTTP route), and the venue save and
//! read of such a venue: each held by a test that was the REST route's.

use serde_json::{json, Value};
use tonic::{Code, Request, Status};

use super::venues::{form, save_form};
use super::{json as read, Project};
use crate::proto::player::v1::{self as proto, lighting_service_server::LightingService};

impl Project {
    pub async fn export_summary(
        &self,
        request: proto::GetMvrExportSummaryRequest,
    ) -> Result<Value, Status> {
        self.server
            .get_mvr_export_summary(Request::new(request))
            .await
            .map(|r| read::export_summary(&r.into_inner()))
    }

    /// A keep, answered as `{"kept": path}` or `{"existing": file}`.
    pub async fn keep(&self, request: proto::KeepMvrExportRequest) -> Result<Value, Status> {
        use proto::keep_mvr_export_response::Outcome;
        let reply = self
            .server
            .keep_mvr_export(Request::new(request))
            .await?
            .into_inner();
        Ok(match reply.outcome.unwrap() {
            Outcome::Kept(path) => json!({"kept": path}),
            Outcome::Existing(file) => json!({"existing": file}),
        })
    }

    pub async fn aim(&self, request: proto::AddVenueAimPointsRequest) -> Result<Value, Status> {
        self.server
            .add_venue_aim_points(Request::new(request))
            .await
            .map(|r| read::aim_points(&r.into_inner()))
    }

    /// The synthetic MVR imported as `kellys`.
    pub async fn imported(&self) {
        crate::webui::api::mvr_api::test::import_kellys(self.state.clone()).await;
    }

    pub fn kellys(&self) -> std::path::PathBuf {
        self.dir.path().join("lighting/venues/kellys.venue")
    }
}

fn summary_of(venue: &str) -> proto::GetMvrExportSummaryRequest {
    proto::GetMvrExportSummaryRequest {
        venue: venue.to_string(),
        ..Default::default()
    }
}

fn keep_of(venue: &str, file: Option<&str>, overwrite: bool) -> proto::KeepMvrExportRequest {
    proto::KeepMvrExportRequest {
        venue: venue.to_string(),
        file: file.map(str::to_string),
        overwrite,
        ..Default::default()
    }
}

fn aim_of(venue: &str, revision: Option<&str>) -> proto::AddVenueAimPointsRequest {
    proto::AddVenueAimPointsRequest {
        name: venue.to_string(),
        expected_revision: revision.map(str::to_string),
        ..Default::default()
    }
}

// ---- GetMvrExportSummary ----

#[tokio::test]
async fn the_summary_says_what_is_linked_and_writes_nothing() {
    let p = super::project();
    p.imported().await;

    let body = p.export_summary(summary_of("kellys")).await.unwrap();
    assert_eq!(body["file_name"], "kellys.mvr");
    assert_eq!(body["venue"], "kellys");
    // "Lost" is a TODO comment, not a fixture of the venue.
    assert_eq!(body["fixtures"], 2);
    assert_eq!(body["positioned_fixtures"], 2);
    assert_eq!(body["focus_points"], 1);
    assert_eq!(body["embedded_gdtfs"].as_array().unwrap().len(), 1);
    let fixed = body["fixed_fixtures"].as_array().unwrap();
    assert_eq!(fixed.len(), 2);
    assert!(fixed.iter().all(|f| f["focus"].is_null()), "{body}");
    assert!(
        !p.dir.path().join("lighting/export").exists(),
        "a summary must not write"
    );

    // A file name is the name a download would carry.
    let named = p
        .export_summary(proto::GetMvrExportSummaryRequest {
            file: Some("tour.mvr".to_string()),
            layers_from_tags: true,
            ..summary_of("kellys")
        })
        .await
        .unwrap();
    assert_eq!(named["file_name"], "tour.mvr");
    // A blank file name is the default.
    let blank = p
        .export_summary(proto::GetMvrExportSummaryRequest {
            file: Some("  ".to_string()),
            ..summary_of("kellys")
        })
        .await
        .unwrap();
    assert_eq!(blank["file_name"], "kellys.mvr");
    assert!(!p.dir.path().join("lighting/export").exists());
}

#[tokio::test]
async fn a_summary_of_a_name_that_is_not_a_bare_mvr_or_a_venue_that_is_not_there_is_refused() {
    let p = super::project();
    p.imported().await;
    let status = p
        .export_summary(proto::GetMvrExportSummaryRequest {
            file: Some("../x.mvr".to_string()),
            ..summary_of("kellys")
        })
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    let status = p.export_summary(summary_of("nope")).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
}

// ---- KeepMvrExport ----

#[tokio::test]
async fn keeping_an_export_will_not_replace_without_overwrite() {
    let p = super::project();
    p.imported().await;
    let kept = p.dir.path().join("lighting/export/tour.mvr");

    let body = p
        .keep(keep_of("kellys", Some("tour.mvr"), false))
        .await
        .unwrap();
    assert_eq!(body["kept"], "lighting/export/tour.mvr");
    assert!(crate::lighting::mvr::parse_archive(&std::fs::read(&kept).unwrap()).is_ok());

    // A second keep leaves the file alone and names it.
    std::fs::write(&kept, b"mine").unwrap();
    let body = p
        .keep(keep_of("kellys", Some("tour.mvr"), false))
        .await
        .unwrap();
    assert_eq!(body["existing"], "tour.mvr");
    assert_eq!(std::fs::read(&kept).unwrap(), b"mine");

    let body = p
        .keep(keep_of("kellys", Some("tour.mvr"), true))
        .await
        .unwrap();
    assert_eq!(body["kept"], "lighting/export/tour.mvr");
    assert_ne!(std::fs::read(&kept).unwrap(), b"mine");
}

#[tokio::test]
async fn keeping_an_export_of_a_venue_that_is_not_there_is_refused() {
    let p = super::project();
    let status = p.keep(keep_of("nope", None, false)).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(!p.dir.path().join("lighting/export").exists());
}

// ---- AddVenueAimPoints ----

#[tokio::test]
async fn aim_points_hit_the_deck_fall_back_to_three_meters_and_skip_the_linked() {
    let p = super::project();
    p.imported().await;
    let venue_path = p.kellys();
    // Brick 1 hangs at (-2, 2.5, 4.2) straight down: the deck is at its
    // feet. Tilt Brick 2 to point level (rotation about Y): its beam never
    // meets the deck. Both are fixed fixtures.
    let original = std::fs::read_to_string(&venue_path).unwrap();
    let tilted = original
        .lines()
        .map(|line| {
            if line.contains("\"Brick 2\"") {
                let (core, comment) = match line.split_once("  # ") {
                    Some((core, comment)) => (core, format!("  # {comment}")),
                    None => (line, String::new()),
                };
                let core = core.split(" rotation ").next().unwrap();
                format!("{core} rotation (0, 90, 0){comment}")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    std::fs::write(&venue_path, &tilted).unwrap();

    let body = p.aim(aim_of("kellys", None)).await.unwrap();
    let created = body["created"].as_array().unwrap();
    assert_eq!(created.len(), 2, "{body}");
    let by_fixture = |name: &str| created.iter().find(|c| c["fixture"] == name).unwrap();
    // Straight down from (-2, 2.5, 4.2) lands on the deck under it.
    assert_eq!(by_fixture("Brick 1")["name"], "Brick 1 aim");
    assert_eq!(by_fixture("Brick 1")["point"], json!([-2.0, 2.5, 0.0]));
    // Level beam: 3 m along it. R = Ry(90): the mounting's -Z becomes -X.
    let level = by_fixture("Brick 2")["point"].as_array().unwrap().clone();
    assert!(
        (level[0].as_f64().unwrap() - (2.0 - 3.0)).abs() < 1e-6,
        "{level:?}"
    );
    assert!((level[2].as_f64().unwrap() - 4.2).abs() < 1e-6, "{level:?}");

    // Comments and provenance are still in the file; the points read back
    // and the export now links both bricks.
    let after = std::fs::read_to_string(&venue_path).unwrap();
    assert!(after.contains("# TODO fixture \"Lost\""), "{after}");
    assert!(after.contains("imported from mvr("), "{after}");
    assert!(after.contains("focus \"Brick 1 aim\""), "{after}");
    let summary = p.export_summary(summary_of("kellys")).await.unwrap();
    assert!(
        summary["fixed_fixtures"]
            .as_array()
            .unwrap()
            .iter()
            .all(|f| f["focus"].is_string()),
        "{summary}"
    );

    // Nothing left to link: a second call creates nothing and leaves the
    // file alone.
    let body = p.aim(aim_of("kellys", None)).await.unwrap();
    assert!(body["created"].as_array().unwrap().is_empty(), "{body}");
    assert_eq!(std::fs::read_to_string(&venue_path).unwrap(), after);
}

#[tokio::test]
async fn aim_points_against_a_stale_revision_are_aborted_and_change_nothing() {
    let p = super::project();
    p.imported().await;
    let path = p.kellys();
    let before = std::fs::read_to_string(&path).unwrap();
    let current = crate::webui::api::lighting_api::content_version(before.as_bytes());

    let status = p
        .aim(aim_of("kellys", Some("not-the-revision")))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::Aborted);
    assert_eq!(
        status.metadata().get("x-current-version").unwrap(),
        current.as_str()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), before);

    let body = p.aim(aim_of("kellys", Some(&current))).await.unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        body["version"].as_str().unwrap(),
        crate::webui::api::lighting_api::content_version(after.as_bytes())
    );
}

#[tokio::test]
async fn aim_points_are_refused_for_a_venue_whose_file_is_not_its_alone() {
    let p = super::project();
    p.imported().await;
    let path = p.kellys();
    let shared = format!(
        "{}venue \"annex\" {{\n}}\n",
        std::fs::read_to_string(&path).unwrap()
    );
    std::fs::write(&path, &shared).unwrap();

    let status = p.aim(aim_of("kellys", None)).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    // The file and what else it holds, in the message.
    assert!(status.message().contains("kellys.venue"), "{status:?}");
    assert!(status.message().contains("annex"), "{status:?}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), shared);
}

#[tokio::test]
async fn aim_points_move_a_light_venue_to_its_venue_file() {
    let p = super::project();
    p.imported().await;
    let venues = p.dir.path().join("lighting/venues");
    let text = std::fs::read_to_string(venues.join("kellys.venue")).unwrap();
    std::fs::remove_file(venues.join("kellys.venue")).unwrap();
    std::fs::write(venues.join("kellys.light"), &text).unwrap();

    let body = p.aim(aim_of("kellys", None)).await.unwrap();
    assert!(!body["created"].as_array().unwrap().is_empty(), "{body}");
    assert!(!venues.join("kellys.light").exists());
    let after = std::fs::read_to_string(venues.join("kellys.venue")).unwrap();
    assert!(after.contains("focus \""), "{after}");
}

#[tokio::test]
async fn aim_points_for_a_venue_that_is_not_there_are_refused() {
    let p = super::project();
    p.imported().await;
    let status = p.aim(aim_of("nope", None)).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(status.message().contains("nope"), "{status:?}");
}

fn line_of(text: &str, fixture: &str) -> String {
    text.lines()
        .find(|l| l.contains(&format!("fixture \"{fixture}\"")))
        .unwrap_or_else(|| panic!("no line for {fixture} in {text}"))
        .to_string()
}

#[tokio::test]
async fn aim_points_keep_a_fixture_s_mode() {
    let p = super::project();
    p.imported().await;
    // A fixed mode: still a fixed fixture (a mover's mode would take it out
    // of the aim-point set).
    let path = p.kellys();
    let moded = std::fs::read_to_string(&path).unwrap();
    assert!(line_of(&moded, "Brick 1").contains("mode \"8: RGBS\""));

    p.aim(aim_of("kellys", None)).await.unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.contains("focus \"Brick 1 aim\""), "{after}");
    assert!(
        line_of(&after, "Brick 1").contains("mode \"8: RGBS\""),
        "{after}"
    );
}

// ---- A saved venue that was imported ----

/// The structured venue `GetVenue` answers, as the JSON a save is sent in
/// (what the UI does: read the venue, change a fixture, save it back).
fn editable(venue: &Value, change: impl Fn(&mut Value)) -> Value {
    let mut fixtures: Vec<Value> = venue["fixtures"]
        .as_object()
        .unwrap()
        .values()
        .cloned()
        .collect();
    fixtures.sort_by_key(|f| f["name"].as_str().unwrap().to_string());
    for fixture in &mut fixtures {
        change(fixture);
    }
    json!({
        "fixtures": fixtures,
        "focus_points": venue["focus_points"],
        "source": venue["source"],
    })
}

#[tokio::test]
async fn tagging_an_imported_venue_through_the_venue_save_keeps_its_todo_lines() {
    let p = super::project();
    p.imported().await;
    let path = p.kellys();
    let before = std::fs::read_to_string(&path).unwrap();
    assert!(before.contains("# TODO fixture \"Lost\""), "{before}");

    // What the UI does: read the venue, change a tag, save it back.
    let got = p.venue("kellys", None).await.unwrap();
    let venue = editable(&got["venue"], |f| {
        if f["name"] == "Brick 1" {
            f["tags"] = json!(["wash"]);
        }
    });
    p.put_venue(save_form("kellys", "lighting/venues", &venue))
        .await
        .unwrap();

    let after = std::fs::read_to_string(&path).unwrap();
    assert!(
        after.contains("tags (\"wash\")") || after.contains("\"wash\""),
        "{after}"
    );
    assert!(after.contains("# TODO fixture \"Lost\""), "{after}");
    assert!(after.contains("# Seeded from Kellys.mvr"), "{after}");
    assert!(after.contains("# layer \"Front Truss\""), "{after}");
    assert!(after.contains("imported from mvr("), "{after}");
    // Brick 2 was not touched, so its line is byte for byte as imported.
    let line = |text: &str| {
        text.lines()
            .find(|l| l.contains("\"Brick 2\""))
            .unwrap()
            .to_string()
    };
    assert_eq!(line(&before), line(&after));
}

#[tokio::test]
async fn a_structured_venue_save_keeps_every_fixture_s_mode() {
    let p = super::project();
    p.imported().await;
    let path = p.kellys();
    let text = std::fs::read_to_string(&path).unwrap();
    // Brick 2 hand-moded to another mode of the archive.
    let moded: String = text
        .lines()
        .map(|line| {
            if line.contains("fixture \"Brick 2\" ") {
                line.replace("mode \"8: RGBS\"", "mode \"Mover 16bit\"")
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert_ne!(moded, text);
    std::fs::write(&path, &moded).unwrap();

    // What the plan and the inspector do: read, change another fixture,
    // save the whole venue back.
    let got = p.venue("kellys", None).await.unwrap();
    assert_eq!(got["venue"]["fixtures"]["Brick 2"]["mode"], "Mover 16bit");
    let venue = editable(&got["venue"], |f| {
        if f["name"] == "Brick 1" {
            f["position"] = json!([-1.0, 2.5, 4.2]);
        }
    });
    p.put_venue(save_form("kellys", "lighting/venues", &venue))
        .await
        .unwrap();
    let after = std::fs::read_to_string(&path).unwrap();
    assert_eq!(line_of(&after, "Brick 2"), line_of(&moded, "Brick 2"));
    assert!(after.contains("position (-1, 2.5, 4.2)"), "{after}");
}

/// The structured form the tests build from JSON reads back as it was sent.
#[test]
fn a_form_is_what_the_json_said() {
    let f = form(&json!({
        "fixtures": [{"name": "A", "fixture_type": "T", "universe": 2, "start_channel": 7,
                      "tags": ["x"], "position": [1, 2, 3], "mode": "m", "beam_angle": 30}],
        "focus_points": {"p": [0, 1, 2]},
        "source": {"mvr": "a.mvr", "origin": [0, 0, 1]},
    }));
    assert_eq!(f.fixtures[0].universe, 2);
    assert_eq!(f.fixtures[0].position.as_ref().unwrap().y, 2.0);
    assert_eq!(f.fixtures[0].mode.as_deref(), Some("m"));
    assert_eq!(f.focus_points["p"].z, 2.0);
    assert_eq!(f.source.unwrap().origin.unwrap().z, 1.0);
}
