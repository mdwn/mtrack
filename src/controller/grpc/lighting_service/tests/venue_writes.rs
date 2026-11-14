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

//! The one rule for venue files, held through `SaveVenue`, `DeleteVenue` and
//! `ListVenues`: a new name is claimed at its canonical `.venue` file, an
//! existing one is written where its file is and only when that file is the
//! venue's alone, and a refused write leaves every file as it was. Each is a
//! test that was the REST route's, answered by status where the route
//! answered by HTTP status (`400` is `INVALID_ARGUMENT`, `404` `NOT_FOUND`,
//! `409` `FAILED_PRECONDITION`, a stale revision `ABORTED`).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};
use tonic::Code;

use super::venues::{save_dsl, save_form};
use super::Project;

/// A project with `files` (path, text) under `rel`.
fn project(rel: &str, files: &[(&str, &str)]) -> (Project, PathBuf) {
    let p = super::project();
    let d = p.make_dir(rel);
    for (path, text) in files {
        let path = d.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    (p, d)
}

/// Every file under `dir` (relative) and its text: what a refused write must
/// leave exactly as it was.
fn snapshot(dir: &Path) -> Vec<(String, String)> {
    let mut out: Vec<_> = walk(dir)
        .into_iter()
        .map(|p| {
            (
                p.strip_prefix(dir).unwrap().display().to_string(),
                std::fs::read_to_string(&p).unwrap_or_default(),
            )
        })
        .collect();
    out.sort();
    out
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(walk(&path));
        } else {
            out.push(path);
        }
    }
    out
}

fn venue(name: &str) -> String {
    format!("venue \"{name}\" {{\n  fixture \"A\" Par @ 1:1\n}}\n")
}

fn par(name: &str) -> String {
    format!("fixture_type \"{name}\" {{\n  channels: 1\n  channel_map: {{ \"dimmer\": 1 }}\n}}\n")
}

/// The structured venue the UI sends: one fixture, in `mode` if given.
fn venue_json(mode: Option<&str>) -> Value {
    let mut fixture =
        json!({"name": "A", "fixture_type": "Par", "universe": 1, "start_channel": 1});
    if let Some(mode) = mode {
        fixture["mode"] = json!(mode);
    }
    json!({ "fixtures": [fixture] })
}

const V: &str = "v_writes";

/// A save of the structured venue as `name`, renaming `from` when given.
async fn put(
    p: &Project,
    name: &str,
    from: Option<&str>,
    mode: Option<&str>,
) -> Result<Value, tonic::Status> {
    let mut request = save_form(name, V, &venue_json(mode));
    request.from = from.map(str::to_string);
    p.put_venue(request).await
}

// --- Saves of a new name.

#[tokio::test]
async fn a_new_venue_is_claimed_at_its_canonical_venue_file() {
    let (p, d) = project(V, &[]);
    put(&p, "Club (Main)", None, None).await.unwrap();
    assert_eq!(snapshot(&d).len(), 1);
    assert!(std::fs::read_to_string(d.join("club_main.venue"))
        .unwrap()
        .contains("venue \"Club (Main)\""));
}

#[tokio::test]
async fn a_new_name_whose_file_is_taken_is_refused() {
    // `club_main.venue` holds "Club Main"; "club main" has its stem.
    let (p, d) = project(V, &[("club_main.venue", &venue("Club Main"))]);
    let before = snapshot(&d);
    let status = p
        .put_venue(save_dsl("club main", V, &venue("club main")))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert!(
        status
            .message()
            .contains("club_main.venue is taken by \"Club Main\""),
        "{status:?}"
    );
    assert_eq!(snapshot(&d), before);
}

// --- Saves of an editable name.

#[tokio::test]
async fn an_editable_venue_is_rewritten_where_it_lives() {
    let text = "# mine\nvenue \"House\" {\n  fixture \"A\" Par @ 1:1  # keep\n}\n";
    let (p, d) = project(V, &[("tour/2024 rig.light", text)]);
    put(&p, "House", None, None).await.unwrap();
    let files = snapshot(&d);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0, "tour/2024 rig.light");
    assert!(files[0].1.contains("# keep"), "{}", files[0].1);
}

#[tokio::test]
async fn a_light_venue_needing_the_new_format_keeps_its_own_stem() {
    let (p, d) = project(V, &[("tour/old.light", &venue("Big Room"))]);
    put(&p, "Big Room", None, Some("8: RGBS")).await.unwrap();
    let names: Vec<String> = snapshot(&d).into_iter().map(|(f, _)| f).collect();
    // The file keeps its name, as a rename elsewhere does.
    assert_eq!(names, ["tour/old.venue"]);
}

#[tokio::test]
async fn a_light_upgrade_whose_target_exists_is_refused() {
    let (p, d) = project(
        V,
        &[("rig.light", &venue("Rig")), ("rig.venue", &venue("Other"))],
    );
    let before = snapshot(&d);
    let status = put(&p, "Rig", None, Some("8: RGBS")).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert!(
        status.message().contains("rig.venue is there"),
        "{status:?}"
    );
    assert_eq!(snapshot(&d), before);

    // Nor beside the same stem in another case.
    let (p, d) = project(
        V,
        &[
            ("tour/rig.light", &venue("Rig")),
            ("tour/RIG.venue", &venue("Other")),
        ],
    );
    let before = snapshot(&d);
    let status = put(&p, "Rig", None, Some("8: RGBS")).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition, "{status:?}");
    assert!(status.message().contains("tour/RIG.venue"), "{status:?}");
    assert_eq!(snapshot(&d), before);
}

// --- Saves of a read-only name.

#[tokio::test]
async fn a_venue_sharing_its_file_is_read_only() {
    let shared = format!("{}{}", venue("Arena"), venue("Club"));
    let (p, d) = project(V, &[("tour/all.light", &shared)]);
    let before = snapshot(&d);
    let structured = put(&p, "Club", None, None).await.unwrap_err();
    let text = p
        .put_venue(save_dsl("Club", V, &venue("Club")))
        .await
        .unwrap_err();
    for status in [structured, text] {
        assert_eq!(status.code(), Code::FailedPrecondition);
        // The file and what else it holds are in the message.
        assert_eq!(
            status.message(),
            "Defined in tour/all.light alongside venue \"Arena\"; edit that file by hand."
        );
    }
    let status = p.remove_venue("Club", Some(V)).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert_eq!(snapshot(&d), before);
}

#[tokio::test]
async fn a_venue_in_a_file_with_other_items_is_read_only() {
    let mixed = format!("{}{}", par("Par"), venue("House"));
    let (p, d) = project(V, &[("house.light", &mixed)]);
    let before = snapshot(&d);
    let status = put(&p, "House", None, None).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert!(
        status.message().contains("fixture_type \"Par\""),
        "{status:?}"
    );
    assert_eq!(snapshot(&d), before);
}

#[tokio::test]
async fn a_venue_declared_twice_is_read_only() {
    let (p, d) = project(
        V,
        &[
            ("a.venue", &venue("House")),
            ("b/house.venue", &venue("House")),
        ],
    );
    let before = snapshot(&d);
    let status = put(&p, "House", None, None).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    // Both files are named in the message.
    assert!(status.message().contains("a.venue"), "{status:?}");
    assert!(status.message().contains("b/house.venue"), "{status:?}");
    let status = p.remove_venue("House", Some(V)).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert_eq!(snapshot(&d), before);
}

// --- DSL bodies.

#[tokio::test]
async fn a_dsl_body_declares_this_one_block_and_nothing_else() {
    let (p, d) = project(V, &[("house.venue", &venue("House"))]);
    let before = snapshot(&d);
    let show = "show \"S\" {\n  @00:00.000\n  front: static color: \"red\"\n}\n";
    for body in [
        format!("{}{}", venue("House"), venue("Annex")),
        format!("{}{}", venue("House"), par("Par")),
        format!("{}{show}", venue("House")),
        format!("{}{}", venue("House"), venue("House")),
        venue("Elsewhere"),
    ] {
        let status = p.put_venue(save_dsl("House", V, &body)).await.unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument, "{body}: {status:?}");
    }
    assert_eq!(snapshot(&d), before);
}

// --- Deletes.

#[tokio::test]
async fn delete_removes_an_editable_file_and_finds_no_other() {
    let (p, d) = project(
        V,
        &[
            ("tour/rig.venue", &venue("House")),
            ("broken.venue", "venue \"Gone\" {"),
        ],
    );
    p.remove_venue("House", Some(V)).await.unwrap();
    assert!(!d.join("tour/rig.venue").exists());
    // A name only a broken file mentions is not there.
    let status = p.remove_venue("Gone", Some(V)).await.unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
    assert!(d.join("broken.venue").exists());
}

// --- Renames.

#[tokio::test]
async fn a_rename_moves_a_canonical_file_to_the_new_canonical_file() {
    let text = "# mine\nvenue \"House\" {\n  fixture \"A\" Par @ 1:1  # keep\n}\n";
    let (p, d) = project(V, &[("house.venue", text)]);
    put(&p, "Big Room", Some("House"), None).await.unwrap();
    let files = snapshot(&d);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0, "big_room.venue");
    assert!(
        files[0].1.starts_with("# mine\nvenue \"Big Room\" {"),
        "{}",
        files[0].1
    );
    assert!(files[0].1.contains("# keep"));
}

#[tokio::test]
async fn a_rename_elsewhere_renames_the_block_and_keeps_the_file_name() {
    let (p, d) = project(V, &[("tour/2024.venue", &venue("House"))]);
    put(&p, "Arena", Some("House"), None).await.unwrap();
    let files = snapshot(&d);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0, "tour/2024.venue");
    assert!(files[0].1.contains("venue \"Arena\""), "{}", files[0].1);

    // A new name with the old one's stem keeps the file too.
    let (p, d) = project(V, &[("house.venue", &venue("house"))]);
    put(&p, "House", Some("house"), None).await.unwrap();
    let files = snapshot(&d);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].0, "house.venue");
    assert!(files[0].1.contains("venue \"House\""));
}

#[tokio::test]
async fn a_rename_is_refused_for_a_taken_name_a_taken_file_or_a_read_only_one() {
    let (p, d) = project(
        V,
        &[
            ("house.venue", &venue("House")),
            ("arena.venue", &venue("Arena")),
            ("annex.venue", &venue("Other Annex")),
            ("all.venue", &format!("{}{}", venue("A1"), venue("A2"))),
        ],
    );
    let before = snapshot(&d);
    // Declared already.
    let status = put(&p, "Arena", Some("House"), None).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    // The new canonical file is taken.
    let status = put(&p, "Annex", Some("House"), None).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert!(status.message().contains("annex.venue"), "{status:?}");
    // Read-only.
    let status = put(&p, "A3", Some("A1"), None).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert!(status.message().contains("all.venue"), "{status:?}");
    // Nothing to rename.
    let status = put(&p, "A4", Some("Nowhere"), None).await.unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
    assert_eq!(snapshot(&d), before);
}

#[tokio::test]
async fn a_rename_at_a_stale_revision_moves_nothing() {
    let (p, d) = project(V, &[("house.venue", &venue("House"))]);
    let before = snapshot(&d);
    let mut request = save_form("Big Room", V, &venue_json(None));
    request.from = Some("House".to_string());
    request.expected_revision = Some("not-the-revision".to_string());
    let status = p.put_venue(request).await.unwrap_err();
    assert_eq!(status.code(), Code::Aborted);
    assert!(status.metadata().get("x-current-version").is_some());
    assert_eq!(snapshot(&d), before);
}

// --- Listings.

#[tokio::test]
async fn the_listings_give_each_names_owner() {
    let shared = format!("{}{}", venue("Arena"), venue("Club"));
    let (p, _) = project(
        V,
        &[
            ("house.venue", &venue("House")),
            ("tour/all.light", &shared),
            ("bad.venue", "venue {"),
        ],
    );
    let list = p.venues(Some(V)).await.unwrap();
    assert_eq!(
        list["owners"]["House"],
        json!({"file": "house.venue", "editable": true, "reason": null})
    );
    assert_eq!(list["owners"]["Club"]["editable"], false);
    assert_eq!(
        list["owners"]["Club"]["reason"],
        "Defined in tour/all.light alongside venue \"Arena\"; edit that file by hand."
    );
    // A file that does not parse is listed by its error, with no name.
    assert_eq!(list["errors"][0]["file"], "bad.venue");
}

// --- The harness's layout stays editable.

#[tokio::test]
async fn the_harness_layout_stays_editable_where_it_lives() {
    let p = super::project();
    let dir = p.dir.path();
    let venues = dir.join("lighting/venues");
    let types = dir.join("lighting/fixture_types");
    std::fs::create_dir_all(&venues).unwrap();
    std::fs::create_dir_all(&types).unwrap();
    std::fs::write(venues.join("built-in.venue"), venue("built-in")).unwrap();
    std::fs::write(venues.join("test.venue"), venue("test")).unwrap();
    // As the importer writes it: a record of its GDTF in the library.
    let library = dir.join("lighting/library");
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(
        library.join("pb15.gdtf"),
        crate::lighting::gdtf::build_zip(&[(
            "description.xml",
            crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.as_bytes(),
        )]),
    )
    .unwrap();
    let record = "# Imported from pb15.gdtf.\nfixture_type \"Astera-PixelBrick\"\n  \
                  from gdtf(\"lighting/library/pb15.gdtf\")\n{\n}\n";
    std::fs::write(types.join("astera_pixelbrick.fixture"), record).unwrap();

    // No directory named: the profile's, as the UI does it.
    let list = p.venues(None).await.unwrap();
    assert_eq!(list["owners"]["built-in"]["editable"], true);
    assert_eq!(list["owners"]["test"]["editable"], true);

    for name in ["built-in", "test"] {
        p.put_venue(proto_save(name, &venue_json(Some("8: RGBS"))))
            .await
            .unwrap();
    }
    // The type's own half of this (saved where it lives, not at its
    // canonical path) is held in `type_writes`.
    let mut names: Vec<String> = snapshot(&dir.join("lighting"))
        .into_iter()
        .map(|(f, _)| f)
        .filter(|f| !f.starts_with(".cache"))
        .collect();
    names.sort();
    assert_eq!(
        names,
        [
            "fixture_types/astera_pixelbrick.fixture",
            "library/pb15.gdtf",
            "venues/built-in.venue",
            "venues/test.venue"
        ]
    );
}

/// A save with no directory named, which is the profile's.
fn proto_save(name: &str, json: &Value) -> crate::proto::player::v1::SaveVenueRequest {
    let mut request = save_form(name, "", json);
    request.dir = None;
    request
}
