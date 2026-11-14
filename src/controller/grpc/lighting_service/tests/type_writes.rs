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

//! What a fixture type save, rename and delete do to the files, by the one
//! rule for type files, called as the service: the type halves of the tests
//! the venue and type routes shared.

use std::path::{Path, PathBuf};

use serde_json::json;
use tonic::Code;

use super::types::{brick, par, rich, save, save_form, synthetic_gdtf_bytes};
use super::Project;
use crate::proto::player::v1 as proto;

const V: &str = "v_writes";
const T: &str = "t_writes";

/// A project with `files` (path, text) under `rel`.
fn project(rel: &str, files: &[(&str, &str)]) -> (Project, PathBuf) {
    let p = super::project();
    let d = p.dir.path().join(rel);
    std::fs::create_dir_all(&d).unwrap();
    for (path, text) in files {
        let path = d.join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    (p, d)
}

/// Every file under `dir` (relative) and its text: what a refused write
/// must leave exactly as it was.
fn snapshot(dir: &Path) -> Vec<(String, String)> {
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

fn venue(name: &str) -> String {
    format!("venue \"{name}\" {{\n  fixture \"A\" Par @ 1:1\n}}\n")
}

fn with_form(
    mut request: proto::SaveFixtureTypeRequest,
    form: i32,
) -> proto::SaveFixtureTypeRequest {
    request.file_form = form;
    request
}

const LIGHT: i32 = proto::FixtureTypeFile::Light as i32;
const FIXTURE: i32 = proto::FixtureTypeFile::Fixture as i32;

// --- Saves of a new name.

#[tokio::test]
async fn a_new_type_is_claimed_at_its_canonical_fixture_file_in_either_form() {
    let (p, d) = project(T, &[]);
    p.put_type(save_form("Wash Par", T, &[("dimmer", 1)]))
        .await
        .unwrap();
    // The `light` form names the form an existing `.light` keeps; a new
    // type is a `.fixture` all the same.
    p.put_type(with_form(save("Spot", T, par("Spot")), LIGHT))
        .await
        .unwrap();
    let names: Vec<String> = snapshot(&d).into_iter().map(|(f, _)| f).collect();
    assert_eq!(names, ["spot.fixture", "wash_par.fixture"]);
}

#[tokio::test]
async fn a_new_name_whose_file_is_taken_is_refused() {
    let (p, d) = project(T, &[("wash_par.fixture", &par("Wash Par"))]);
    let before = snapshot(&d);
    let status = p
        .put_type(save("wash par", T, par("wash par")))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert!(
        status
            .message()
            .contains("wash_par.fixture is taken by \"Wash Par\""),
        "{}",
        status.message()
    );
    assert_eq!(snapshot(&d), before);
}

#[tokio::test]
async fn a_new_type_may_not_take_an_unrecorded_library_name() {
    let (p, d) = project(T, &[]);
    let library = p.dir.path().join("lighting/library");
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(library.join("synth.gdtf"), synthetic_gdtf_bytes()).unwrap();
    let unrecorded = crate::lighting::library::unrecorded_types(p.dir.path(), Some(&d));
    let name = unrecorded.types[0].name.clone();
    let status = p.put_type(save(&name, T, par(&name))).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert!(
        status.message().contains("synth.gdtf"),
        "{}",
        status.message()
    );
    assert!(snapshot(&d).is_empty());
}

// --- Saves of an editable name.

#[tokio::test]
async fn an_editable_type_is_rewritten_where_it_lives() {
    let (p, d) = project(T, &[("old/par-v1.light", &par("Par"))]);
    p.put_type(save_form("Par", T, &[("dimmer", 1)]))
        .await
        .unwrap();
    let names: Vec<String> = snapshot(&d).into_iter().map(|(f, _)| f).collect();
    assert_eq!(names, ["old/par-v1.light"]);
}

#[tokio::test]
async fn a_light_type_needing_the_new_format_moves_to_its_stem_fixture() {
    let (p, d) = project(T, &[("mover.light", &par("Mover"))]);
    p.put_type(with_form(save("Mover", T, rich("Mover")), LIGHT))
        .await
        .unwrap();
    assert_eq!(snapshot(&d), [("mover.fixture".to_string(), rich("Mover"))]);

    // Asked for, a plain type moves too.
    let (p, d) = project(T, &[("wash.light", &par("Wash"))]);
    p.put_type(with_form(save("Wash", T, par("Wash")), FIXTURE))
        .await
        .unwrap();
    let names: Vec<String> = snapshot(&d).into_iter().map(|(f, _)| f).collect();
    assert_eq!(names, ["wash.fixture"]);
}

#[tokio::test]
async fn a_light_type_upgrade_whose_target_exists_is_refused() {
    let (p, d) = project(
        T,
        &[
            ("mover.light", &par("Mover")),
            ("mover.fixture", &par("Other")),
        ],
    );
    let before = snapshot(&d);
    let status = p
        .put_type(save("Mover", T, rich("Mover")))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert_eq!(snapshot(&d), before);
}

#[tokio::test]
async fn a_fixture_file_is_not_written_back_as_light() {
    let (p, d) = project(T, &[("wash.fixture", &par("Wash"))]);
    let before = snapshot(&d);
    let status = p
        .put_type(with_form(save("Wash", T, par("Wash")), LIGHT))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert_eq!(snapshot(&d), before);
}

#[tokio::test]
async fn the_channel_map_form_saves_only_a_type_it_can_say_all_of() {
    let plain = par("Wash");
    let with = |line: &str| plain.replace("\n}", &format!("\n  {line}\n}}"));
    for (what, text) in [
        ("a comment", format!("# measured on the rig\n{plain}")),
        ("a strobe curve", with("strobe_curve: linear")),
        ("special cases", with("special_cases: [\"Wash\"]")),
        (
            "movement limits",
            with("movement { max_pan_speed: 90deg/s }"),
        ),
        ("rich channels", rich("Wash")),
    ] {
        let (p, d) = project(T, &[("wash.fixture", &text)]);
        let before = snapshot(&d);
        let list = p.types(Some(T), None).await.unwrap();
        assert_eq!(
            list["fixture_types"]["Wash"]["form_editable"], false,
            "{what}"
        );
        let status = p
            .put_type(save_form("Wash", T, &[("dimmer", 1)]))
            .await
            .unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument, "{what}");
        assert!(status.message().contains("as text"), "{}", status.message());
        assert_eq!(snapshot(&d), before, "{what}");
    }
    // A plain type, in either extension, is the form's to open and save.
    for file in ["wash.fixture", "wash.light"] {
        let (p, _) = project(T, &[(file, &plain)]);
        let list = p.types(Some(T), None).await.unwrap();
        assert_eq!(
            list["fixture_types"]["Wash"]["form_editable"], true,
            "{file}"
        );
        p.put_type(save_form("Wash", T, &[("dimmer", 1)]))
            .await
            .unwrap_or_else(|e| panic!("{file}: {e:?}"));
    }
}

// --- Saves of a read-only name.

#[tokio::test]
async fn a_type_sharing_its_file_is_read_only() {
    let shared = format!("{}{}", par("Par"), par("Wash"));
    let (p, d) = project(T, &[("all.light", &shared)]);
    let before = snapshot(&d);
    let status = p.put_type(save("Wash", T, par("Wash"))).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    // The refusal says which file and what else is in it, in its message:
    // the one thing the editor ever showed of the old answer's body.
    assert_eq!(
        status.message(),
        "Defined in all.light alongside fixture type \"Par\"; edit that file by hand."
    );
    let status = p.delete_type("Wash", Some(T)).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert_eq!(snapshot(&d), before);
}

#[tokio::test]
async fn a_type_declared_in_a_file_with_a_venue_is_read_only() {
    let mixed = format!("{}{}", par("Par"), venue("House"));
    let (p, d) = project(T, &[("house.light", &mixed)]);
    let before = snapshot(&d);
    let status = p.put_type(save("Par", T, par("Par"))).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert!(
        status.message().contains("venue \"House\""),
        "{}",
        status.message()
    );
    assert_eq!(snapshot(&d), before);
}

// --- Text bodies.

#[tokio::test]
async fn text_declares_this_one_type_and_nothing_else() {
    let (p, d) = project(T, &[]);
    for body in [
        format!("{}{}", par("Par"), par("Wash")),
        format!("{}{}", par("Par"), venue("House")),
        format!("{}{}", par("Par"), par("Par")),
        par("Elsewhere"),
    ] {
        let status = p.put_type(save("Par", T, body.clone())).await.unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument, "{body}");
    }
    assert!(snapshot(&d).is_empty());
}

// --- Deletes.

#[tokio::test]
async fn delete_removes_an_editable_file_and_finds_no_other() {
    let (p, d) = project(
        T,
        &[
            ("x/par.light", &par("Par")),
            ("broken.fixture", "fixture_type \"Gone\" {"),
        ],
    );
    p.delete_type("Par", Some(T)).await.unwrap();
    assert!(!d.join("x/par.light").exists());
    // A name only a broken file mentions is not there.
    let status = p.delete_type("Gone", Some(T)).await.unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
    assert!(d.join("broken.fixture").exists());
    let status = p.delete_type("Par", Some(T)).await.unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
}

// --- Renames.

#[tokio::test]
async fn a_rename_moves_a_canonical_file_to_the_new_canonical_file() {
    let (p, d) = project(T, &[("par.light", &par("Par"))]);
    let mut request = save("Wash", T, par("Wash"));
    request.from = Some("Par".to_string());
    p.put_type(request).await.unwrap();
    assert_eq!(snapshot(&d), [("wash.fixture".to_string(), par("Wash"))]);
}

#[tokio::test]
async fn a_rename_of_a_type_nothing_declares_is_not_found() {
    let (p, d) = project(T, &[("par.light", &par("Par"))]);
    let before = snapshot(&d);
    let mut request = save("Wash", T, par("Wash"));
    request.from = Some("Nowhere".to_string());
    let status = p.put_type(request).await.unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
    assert_eq!(snapshot(&d), before);
}

#[tokio::test]
async fn a_rename_onto_a_declared_name_or_taken_file_is_refused() {
    let (p, d) = project(
        T,
        &[
            ("par.light", &par("Par")),
            ("wash.light", &par("Wash")),
            ("spot.fixture", &par("Other Spot")),
        ],
    );
    let before = snapshot(&d);
    // Declared already.
    let mut request = save("Wash", T, par("Wash"));
    request.from = Some("Par".to_string());
    assert_eq!(
        p.put_type(request).await.unwrap_err().code(),
        Code::FailedPrecondition
    );
    // The new canonical file is taken.
    let mut request = save("Spot", T, par("Spot"));
    request.from = Some("Par".to_string());
    let status = p.put_type(request).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert!(
        status.message().contains("spot.fixture"),
        "{}",
        status.message()
    );
    assert_eq!(snapshot(&d), before);
}

#[tokio::test]
async fn an_editor_rename_of_a_type_retypes_every_venue_using_it() {
    let (p, d) = project(T, &[("par.fixture", &par("Par"))]);
    let venues = p.dir.path().join(V);
    std::fs::create_dir_all(venues.join("tour")).unwrap();
    std::fs::write(venues.join("club.venue"), venue("club")).unwrap();
    std::fs::write(venues.join("tour/2024.venue"), venue("arena")).unwrap();

    let mut request = save("Wash", T, par("Wash"));
    request.from = Some("Par".to_string());
    request.venues_dir = Some(V.to_string());
    let body = p.put_type(request).await.unwrap();

    assert_eq!(snapshot(&d), [("wash.fixture".to_string(), par("Wash"))]);
    let retyped = venue("club").replace("Par @", "Wash @");
    assert_eq!(
        std::fs::read_to_string(venues.join("club.venue")).unwrap(),
        retyped
    );
    assert!(std::fs::read_to_string(venues.join("tour/2024.venue"))
        .unwrap()
        .contains("\"A\" Wash @ 1:1"));
    assert_eq!(body["rename"]["lines"], 2, "{body}");
    assert_eq!(body["rename"]["from"], "Par");
    assert_eq!(body["rename"]["to"], "Wash");
    assert_eq!(
        body["rename"]["venues"],
        json!([
            {"venue": "club", "file": "club.venue", "lines": 1},
            {"venue": "arena", "file": "tour/2024.venue", "lines": 1},
        ])
    );
}

#[tokio::test]
async fn an_editor_rename_is_refused_when_a_venue_using_it_is_read_only() {
    let (p, d) = project(T, &[("par.fixture", &par("Par"))]);
    let venues = p.dir.path().join(V);
    std::fs::create_dir_all(&venues).unwrap();
    std::fs::write(venues.join("club.venue"), venue("club")).unwrap();
    std::fs::write(
        venues.join("all.venue"),
        format!("{}{}", venue("arena"), venue("annex")),
    )
    .unwrap();
    let (types_before, venues_before) = (snapshot(&d), snapshot(&venues));

    let mut request = save("Wash", T, par("Wash"));
    request.from = Some("Par".to_string());
    request.venues_dir = Some(V.to_string());
    let status = p.put_type(request).await.unwrap_err();

    assert_eq!(status.code(), Code::FailedPrecondition);
    let error = status.message();
    assert!(
        error.starts_with("fixture type \"Par\" cannot be renamed, so nothing was written;"),
        "{error}"
    );
    assert!(
        error.contains("venue \"annex\" in all.venue uses it"),
        "{error}"
    );
    assert!(
        error.contains("venue \"arena\" in all.venue uses it"),
        "{error}"
    );
    assert_eq!(snapshot(&d), types_before);
    assert_eq!(snapshot(&venues), venues_before);
}

// --- The harness's layout stays editable.

#[tokio::test]
async fn the_harness_layout_stays_editable_where_it_lives() {
    let p = super::project();
    let types = p.dir.path().join("lighting/fixture_types");
    let venues = p.dir.path().join("lighting/venues");
    std::fs::create_dir_all(&types).unwrap();
    std::fs::create_dir_all(&venues).unwrap();
    std::fs::write(venues.join("built-in.venue"), venue("built-in")).unwrap();
    std::fs::write(venues.join("test.venue"), venue("test")).unwrap();
    // As the importer writes it: a record of its GDTF in the library.
    let library = p.dir.path().join("lighting/library");
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(library.join("pb15.gdtf"), synthetic_gdtf_bytes()).unwrap();
    let record = "# Imported from pb15.gdtf.\nfixture_type \"Astera-PixelBrick\"\n  \
                  from gdtf(\"lighting/library/pb15.gdtf\")\n{\n}\n";
    std::fs::write(types.join("astera_pixelbrick.fixture"), record).unwrap();

    let list = p.types(None, None).await.unwrap();
    assert_eq!(
        list["fixture_types"]["Astera-PixelBrick"]["owner"]["editable"],
        true
    );
    // A record is text, never the channel map's.
    assert_eq!(
        list["fixture_types"]["Astera-PixelBrick"]["form_editable"],
        false
    );

    // Not at its canonical path (`astera-pixelbrick.fixture`): saved
    // where it lives.
    p.put_type(proto::SaveFixtureTypeRequest {
        name: "Astera-PixelBrick".to_string(),
        body: Some(proto::save_fixture_type_request::Body::Dsl(
            record.replace("{\n}", "{\n  movement { max_pan_speed: 90deg/s }\n}"),
        )),
        ..Default::default()
    })
    .await
    .unwrap();
    assert!(
        std::fs::read_to_string(types.join("astera_pixelbrick.fixture"))
            .unwrap()
            .contains("max_pan_speed")
    );
    let mut names: Vec<String> = snapshot(&p.dir.path().join("lighting"))
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

#[tokio::test]
async fn a_brick_is_a_reference_the_form_does_not_express() {
    // The plain `brick` helper's text is a record, not a channel map.
    let (p, _) = project(T, &[("brick.fixture", &brick("Brick"))]);
    let list = p.types(Some(T), None).await.unwrap();
    assert_eq!(list["fixture_types"]["Brick"]["form_editable"], false);
}
