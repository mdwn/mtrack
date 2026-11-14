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

//! The fixture type calls (`ListFixtureTypes`, `GetFixtureType`,
//! `SaveFixtureType`, `DeleteFixtureType`, `GetFixtureTypeGdtfDetails`),
//! called as the service: each held by a test that was the REST route's.
//! The answers are read back as the JSON the routes sent
//! ([`super::json`]), so the assertions are the routes' own.

use std::path::Path;

use serde_json::{json, Value};
use tonic::{Code, Request, Status};

use super::{json as read, Project};
use crate::proto::player::v1::{self as proto, lighting_service_server::LightingService};

/// A hand-written type, in the form a `.light` holds.
pub(super) fn par(name: &str) -> String {
    format!(
        "fixture_type \"{name}\" {{\n  channels: 3\n  channel_map: {{\n    \"red\": 1,\n    \
         \"green\": 2,\n    \"blue\": 3\n  }}\n}}"
    )
}

/// The rich channel form, which only a `.fixture` file may hold.
pub(super) fn rich(name: &str) -> String {
    format!("fixture_type \"{name}\" {{\n  channel \"pan\" @ 1 fine 2 range -270deg..270deg\n}}\n")
}

/// A GDTF-referential type. It parses without the archive.
pub(super) fn brick(name: &str) -> String {
    format!("fixture_type \"{name}\" from gdtf(\"library/synth.gdtf\") {{\n}}\n")
}

pub(super) fn venue(name: &str) -> String {
    format!("venue \"{name}\" {{\n  fixture \"Spot1\" GenericPar @ 1:1\n}}\n")
}

pub(super) fn synthetic_gdtf_bytes() -> Vec<u8> {
    crate::testutil::lighting::synthetic_gdtf()
}

/// A save of the text `dsl` as `name` in `dir`.
pub(super) fn save(name: &str, dir: &str, dsl: impl Into<String>) -> proto::SaveFixtureTypeRequest {
    proto::SaveFixtureTypeRequest {
        name: name.to_string(),
        dir: Some(dir.to_string()),
        body: Some(proto::save_fixture_type_request::Body::Dsl(dsl.into())),
        ..Default::default()
    }
}

/// A save of the channel-map form.
pub(super) fn save_form(
    name: &str,
    dir: &str,
    channels: &[(&str, u32)],
) -> proto::SaveFixtureTypeRequest {
    proto::SaveFixtureTypeRequest {
        name: name.to_string(),
        dir: Some(dir.to_string()),
        body: Some(proto::save_fixture_type_request::Body::ChannelMap(
            proto::FixtureTypeChannelMap {
                channels: channels.iter().map(|(n, o)| (n.to_string(), *o)).collect(),
                ..Default::default()
            },
        )),
        ..Default::default()
    }
}

impl Project {
    pub async fn types(&self, dir: Option<&str>, venues: Option<&str>) -> Result<Value, Status> {
        self.server
            .list_fixture_types(Request::new(proto::ListFixtureTypesRequest {
                dir: dir.map(str::to_string),
                venues_dir: venues.map(str::to_string),
            }))
            .await
            .map(|r| read::fixture_type_list(&r.into_inner()))
    }

    pub async fn the_type(&self, name: &str, dir: Option<&str>) -> Result<Value, Status> {
        self.server
            .get_fixture_type(Request::new(proto::GetFixtureTypeRequest {
                name: name.to_string(),
                dir: dir.map(str::to_string),
            }))
            .await
            .map(|r| read::fixture_type_info(&r.into_inner(), false))
    }

    pub async fn type_gdtf(
        &self,
        name: &str,
        dir: Option<&str>,
        venues: Option<&str>,
    ) -> Result<Value, Status> {
        self.server
            .get_fixture_type_gdtf_details(Request::new(proto::GetFixtureTypeGdtfDetailsRequest {
                name: name.to_string(),
                dir: dir.map(str::to_string),
                venues_dir: venues.map(str::to_string),
            }))
            .await
            .map(|r| read::gdtf_details(&r.into_inner()))
    }

    pub async fn put_type(&self, request: proto::SaveFixtureTypeRequest) -> Result<Value, Status> {
        self.server
            .save_fixture_type(Request::new(request))
            .await
            .map(|r| read::fixture_type_saved(&r.into_inner()))
    }

    pub async fn delete_type(&self, name: &str, dir: Option<&str>) -> Result<Value, Status> {
        self.server
            .delete_fixture_type(Request::new(proto::DeleteFixtureTypeRequest {
                name: name.to_string(),
                dir: dir.map(str::to_string),
            }))
            .await
            .map(|r| read::fixture_type_deleted(&r.into_inner()))
    }

    /// The directory `rel` of the project, made.
    pub fn make_dir(&self, rel: &str) -> std::path::PathBuf {
        let dir = self.dir.path().join(rel);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

/// A fixture types directory holding one file, named as `import-gdtf` used
/// to name it (`astera_pixelbrick.fixture`), which is not the name's own
/// stem.
fn imported_dir(project: &Path, rel: &str, name: &str, dsl: String) -> std::path::PathBuf {
    let stem = crate::lighting::files::stem(name).replace('-', "_");
    assert_ne!(
        stem,
        crate::lighting::files::stem(name),
        "the stems must differ"
    );
    let dir = project.join(rel);
    std::fs::create_dir(&dir).unwrap();
    let file = dir.join(format!("{stem}.fixture"));
    std::fs::write(&file, dsl).unwrap();
    file
}

fn names(list: &Value) -> Vec<String> {
    let mut names: Vec<String> = list["fixture_types"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    names.sort();
    names
}

// ---- ListFixtureTypes ----

#[tokio::test]
async fn an_empty_directory_lists_no_types() {
    let p = super::project();
    p.make_dir("ft_empty");
    let parsed = p.types(Some("ft_empty"), None).await.unwrap();
    assert_eq!(parsed["fixture_types"].as_object().unwrap().len(), 0);
}

#[tokio::test]
async fn a_directory_that_is_not_there_lists_no_types() {
    let p = super::project();
    let parsed = p.types(Some("nonexistent_dir"), None).await.unwrap();
    assert_eq!(parsed["fixture_types"].as_object().unwrap().len(), 0);
}

#[tokio::test]
async fn the_listing_gives_each_type_its_file_form_and_kind() {
    let p = super::project();
    let dir = p.make_dir("ft_test");
    std::fs::write(dir.join("led_par.light"), par("LED_Par")).unwrap();
    std::fs::write(dir.join("mover.fixture"), rich("Mover")).unwrap();
    std::fs::write(dir.join("brick.fixture"), brick("Brick")).unwrap();

    let parsed = p.types(Some("ft_test"), None).await.unwrap();
    let par = &parsed["fixture_types"]["LED_Par"];
    assert!(par["fixture_type"].is_object(), "{parsed}");
    assert_eq!(par["file"], "led_par.light");
    assert_eq!(par["extension"], "light");
    assert_eq!(par["referential"], false);
    assert_eq!(par["rich"], false);

    let mover = &parsed["fixture_types"]["Mover"];
    assert_eq!(mover["file"], "mover.fixture");
    assert_eq!(mover["extension"], "fixture");
    assert_eq!(mover["rich"], true);
    assert_eq!(mover["referential"], false);

    // A referential type has no channels of its own here; only the
    // lighting system's expansion resolves them from the archive.
    let brick = &parsed["fixture_types"]["Brick"];
    assert_eq!(brick["referential"], true);
    assert_eq!(brick["extension"], "fixture");
    assert_eq!(
        brick["fixture_type"]["channels"].as_object().unwrap().len(),
        0
    );
}

#[tokio::test]
async fn a_listing_names_each_channel_with_its_offset_and_the_strobe_figures() {
    let p = super::project();
    let dir = p.make_dir("ft_channels");
    std::fs::write(
        dir.join("strobe.light"),
        "fixture_type \"Strobe\" {\n  channels: 2\n  channel_map: { \"dimmer\": 1, \"strobe\": 2 }\n  \
         max_strobe_frequency: 25\n  min_strobe_frequency: 0.5\n  strobe_dmx_offset: 10\n}\n",
    )
    .unwrap();
    let parsed = p.types(Some("ft_channels"), None).await.unwrap();
    let strobe = &parsed["fixture_types"]["Strobe"];
    assert_eq!(strobe["fixture_type"]["name"], "Strobe");
    assert_eq!(
        strobe["fixture_type"]["channels"],
        json!({"dimmer": 1, "strobe": 2})
    );
    assert_eq!(strobe["fixture_type"]["max_strobe_frequency"], 25.0);
    assert_eq!(strobe["fixture_type"]["min_strobe_frequency"], 0.5);
    assert_eq!(strobe["fixture_type"]["strobe_dmx_offset"], 10);
    assert_eq!(strobe["footprint"], 2);
    // Owned by its file, which it has to itself.
    assert_eq!(
        strobe["owner"],
        json!({"file": "strobe.light", "editable": true, "reason": null})
    );
}

#[tokio::test]
async fn several_files_list_every_type() {
    let p = super::project();
    let dir = p.make_dir("ft_multi");
    std::fs::write(dir.join("a.light"), par("TypeA")).unwrap();
    std::fs::write(dir.join("b.light"), par("TypeB")).unwrap();
    let parsed = p.types(Some("ft_multi"), None).await.unwrap();
    assert_eq!(names(&parsed), ["TypeA", "TypeB"]);
}

#[tokio::test]
async fn a_name_defined_twice_is_reported_against_both_files() {
    // Last-wins would show one file and hide the other, while the
    // lighting system registers the name twice.
    let p = super::project();
    let dir = p.make_dir("ft_dupes");
    std::fs::write(dir.join("a.light"), par("Mover")).unwrap();
    std::fs::write(dir.join("b.fixture"), rich("Mover")).unwrap();
    std::fs::write(dir.join("c.light"), par("Par")).unwrap();

    let parsed = p.types(Some("ft_dupes"), None).await.unwrap();
    let errors = parsed["errors"].as_array().unwrap();
    assert_eq!(errors.len(), 1, "{parsed}");
    let message = errors[0]["error"].as_str().unwrap();
    assert!(message.contains("defined in both"), "{message}");
    assert!(message.contains("a.light"), "{message}");
    assert!(message.contains("b.fixture"), "{message}");
    // The rest of the directory still lists, including the duplicate
    // under one of its two files.
    assert!(parsed["fixture_types"]["Par"].is_object(), "{parsed}");
    assert!(parsed["fixture_types"]["Mover"].is_object(), "{parsed}");
}

#[tokio::test]
async fn the_listing_gives_each_name_its_owner() {
    let p = super::project();
    let dir = p.make_dir("ft_owners");
    std::fs::write(dir.join("par.light"), par("Par")).unwrap();
    std::fs::write(dir.join("all.fixture"), format!("{}{}", par("A"), par("B"))).unwrap();
    let list = p.types(Some("ft_owners"), None).await.unwrap();
    assert_eq!(list["fixture_types"]["Par"]["owner"]["editable"], true);
    assert_eq!(list["fixture_types"]["A"]["owner"]["editable"], false);
    assert_eq!(
        list["fixture_types"]["A"]["owner"]["reason"],
        "Defined in all.fixture alongside fixture type \"B\"; edit that file by hand."
    );
}

/// A project with the synthetic archive in `library/`, a `Brick` type
/// referring to it in `ft_rel`, and a venues directory `v_rel` patching
/// three Bricks (one in a second venue) beside a file that will not parse.
fn project_with_bricks(project: &Path, ft_rel: &str, v_rel: &str) {
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
        // One fixture in another mode, spelled loosely (the archive's
        // own spelling comes back), and one in a mode it does not have.
        "venue \"club\" {\n  fixture \"Solo\" Brick mode \"mover 16BIT\" @ 2:1\n  \
         fixture \"Odd\" Brick mode \"Nope\" @ 2:40\n}\n",
    )
    .unwrap();
    std::fs::write(venues.join("broken.light"), "venue {{{").unwrap();
}

/// A fixture types directory with types at the top and in
/// subdirectories: a native type in each, a GDTF record in `rig/`, a
/// second declaration of `Top` further down, and an unrecorded archive
/// in the library.
fn project_with_nested_types(project: &Path, ft_rel: &str) {
    let library = project.join("library");
    std::fs::create_dir_all(&library).unwrap();
    std::fs::write(library.join("synth.gdtf"), synthetic_gdtf_bytes()).unwrap();
    let shelf = project.join("lighting/library");
    std::fs::create_dir_all(&shelf).unwrap();
    std::fs::write(shelf.join("loose.gdtf"), synthetic_gdtf_bytes()).unwrap();
    let ft = project.join(ft_rel);
    std::fs::create_dir_all(ft.join("rig/old")).unwrap();
    std::fs::write(ft.join("top.light"), par("Top")).unwrap();
    std::fs::write(ft.join("rig/wash.light"), par("Wash")).unwrap();
    std::fs::write(ft.join("rig/brick.fixture"), brick("Brick")).unwrap();
    std::fs::write(ft.join("rig/old/top.light"), par("Top")).unwrap();
}

#[tokio::test]
async fn each_gdtf_type_is_summarised_and_a_missing_archive_is_survived() {
    let p = super::project();
    project_with_bricks(p.dir.path(), "ft_list_gdtf", "v_list_gdtf");
    std::fs::write(
        p.dir.path().join("ft_list_gdtf").join("lost.fixture"),
        "fixture_type \"Lost\" from gdtf(\"library/gone.gdtf\") {\n}\n",
    )
    .unwrap();
    std::fs::write(
        p.dir.path().join("ft_list_gdtf").join("par.light"),
        par("Par"),
    )
    .unwrap();

    let parsed = p
        .types(Some("ft_list_gdtf"), Some("v_list_gdtf"))
        .await
        .unwrap();
    let types = &parsed["fixture_types"];
    let brick = &types["Brick"]["gdtf"];
    assert_eq!(brick["fixture"], "Synth Brick");
    assert_eq!(brick["manufacturer"], "mtrack synthetic");
    assert!(brick["modes"].as_u64().unwrap() > 0, "{brick}");
    assert!(brick.get("mode").is_none(), "{brick}");
    assert_eq!(
        brick["beam"],
        serde_json::json!({"type": "Spot", "angle": 12.0})
    );
    assert_eq!(brick["used_by"], 4);
    assert!(
        types["Brick"]["footprint"].is_null(),
        "a GDTF type's footprint is each fixture's mode's: {parsed}"
    );
    assert!(types["Lost"]["footprint"].is_null(), "unknown is null");
    assert!(
        types["Par"]["footprint"].as_u64().unwrap() > 0,
        "a native type's own footprint: {parsed}"
    );
    assert_eq!(
        brick["in_use"],
        serde_json::json!([
            {"mode": "8: RGBS", "count": 2},
            {"mode": "Mover 16bit", "count": 1},
            {"mode": "Nope", "count": 1},
        ]),
        "most-used first, the archive's spelling, an unknown mode as written"
    );
    assert!(brick["thumbnail"].is_null(), "no rig in the store yet");
    assert!(
        types["Lost"]["gdtf"].is_null(),
        "a missing archive is that card's null: {parsed}"
    );
    assert_eq!(types["Lost"]["referential"], true);
    assert!(
        types["Par"].get("gdtf").is_none(),
        "a native type carries no summary"
    );
    assert!(
        !p.dir.path().join("lighting/.cache").exists(),
        "a listing writes nothing"
    );
}

#[tokio::test]
async fn an_unused_gdtf_type_is_listed_and_drawn_in_its_first_mode() {
    let p = super::project();
    project_with_bricks(p.dir.path(), "ft_unused", "v_unused");
    std::fs::write(
        p.dir.path().join("ft_unused").join("bare.fixture"),
        "fixture_type \"Bare\" from gdtf(\"library/synth.gdtf\") {\n}\n",
    )
    .unwrap();

    let parsed = p.types(Some("ft_unused"), Some("v_unused")).await.unwrap();
    let bare = &parsed["fixture_types"]["Bare"];
    assert_eq!(bare["referential"], true, "{parsed}");
    assert!(bare["footprint"].is_null(), "{bare}");
    assert!(bare.get("default_mode").is_none(), "{bare}");
    assert_eq!(bare["gdtf"]["in_use"], serde_json::json!([]));

    let parsed = p
        .type_gdtf("Bare", Some("ft_unused"), Some("v_unused"))
        .await
        .unwrap();
    assert_eq!(parsed["venues"], serde_json::json!([]));
    assert!(
        parsed["rig"].is_string(),
        "drawn in the first mode: {parsed}"
    );
}

#[tokio::test]
async fn a_type_in_a_subdirectory_lists_opens_saves_in_place_and_deletes() {
    let p = super::project();
    project_with_nested_types(p.dir.path(), "ft_nested");
    let ft = p.dir.path().join("ft_nested");

    let list = p.types(Some("ft_nested"), None).await.unwrap();
    let types = &list["fixture_types"];
    assert_eq!(types["Wash"]["file"], "rig/wash.light", "{list}");
    assert_eq!(types["Brick"]["file"], "rig/brick.fixture", "{list}");
    // The second `Top` is reported the way any duplicate is, against
    // both files; the first in path order keeps the name.
    assert_eq!(types["Top"]["file"], "rig/old/top.light", "{list}");
    let errors = list["errors"].as_array().unwrap();
    assert!(
        errors.iter().any(|e| e["file"] == "top.light"
            && e["error"]
                .as_str()
                .unwrap()
                .contains("\"Top\" is defined in both rig/old/top.light and top.light")),
        "{list}"
    );

    p.the_type("Wash", Some("ft_nested")).await.unwrap();
    p.the_type("Brick", Some("ft_nested")).await.unwrap();
    p.type_gdtf("Brick", Some("ft_nested"), None).await.unwrap();

    p.put_type(save_form("Wash", "ft_nested", &[("dimmer", 1)]))
        .await
        .unwrap();
    let saved = std::fs::read_to_string(ft.join("rig/wash.light")).unwrap();
    assert!(saved.contains("dimmer"), "{saved}");
    assert!(
        !ft.join("wash.light").exists(),
        "saved beside, not in place"
    );

    p.delete_type("Wash", Some("ft_nested")).await.unwrap();
    assert!(!ft.join("rig/wash.light").exists());
    assert!(ft.join("rig/brick.fixture").exists());
}

#[tokio::test]
async fn the_engine_and_the_fixture_types_list_read_the_same_tree() {
    let p = super::project();
    project_with_nested_types(p.dir.path(), "ft_agree");

    let mut system = crate::lighting::system::LightingSystem::new();
    system
        .load(
            &crate::config::Lighting::new(
                None,
                None,
                Some(crate::config::lighting::Directories::new(
                    Some("ft_agree".into()),
                    None,
                )),
            ),
            p.dir.path(),
        )
        .unwrap();
    let mut engine: Vec<String> = system
        .fixture_types_iter()
        .chain(system.gdtf_types_iter())
        .map(|(name, _)| name.clone())
        .collect();
    engine.sort();

    let list = p.types(Some("ft_agree"), None).await.unwrap();
    let web = names(&list);
    assert_eq!(engine, web);
    assert_eq!(web, ["Brick", "Synth Brick", "Top", "Wash"]);
}

#[tokio::test]
async fn the_engine_and_the_listing_resolve_the_same_directories() {
    use crate::config::lighting::{Directories, Lighting};
    /// `None`: no lighting section; `Some(None)`: no `directories`;
    /// else (fixture types, venues) as configured.
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
        std::fs::write(root.join(types).join("par.light"), par("Par")).unwrap();
        std::fs::write(
            root.join(venues).join("v.light"),
            "venue \"v\" {\n  fixture \"A\" Par @ 1:1\n}\n",
        )
        .unwrap();

        // The engine (a profile with a lighting section) loads them.
        if let Some(config) = &section {
            let mut system = crate::lighting::system::LightingSystem::new();
            system.load(config, root).unwrap();
            assert_eq!(
                system.get_current_venue_fixtures().map(|f| f.len()).ok(),
                Some(1),
                "{case}: the engine finds the venue and its type"
            );
        }

        // The listing, given what the UI sends for this profile.
        let configured = section.as_ref().and_then(|l| l.directories());
        let list = p
            .types(configured.and_then(|d| d.fixture_types()), None)
            .await
            .unwrap();
        assert!(
            list.to_string().contains("\"Par\""),
            "{case}: the listing has the type: {list}"
        );
    }
}

// ---- GetFixtureType ----

#[tokio::test]
async fn a_type_is_read_with_its_file_and_text() {
    let p = super::project();
    let dir = p.make_dir("ft_get");
    std::fs::write(dir.join("led_par.light"), par("LED_Par")).unwrap();
    let parsed = p.the_type("LED_Par", Some("ft_get")).await.unwrap();
    assert!(parsed["fixture_type"].is_object());
    assert!(parsed["dsl"].is_string());
    assert_eq!(parsed["file"], "led_par.light");
    assert_eq!(parsed["extension"], "light");
    assert_eq!(parsed["referential"], false);
    assert_eq!(parsed["rich"], false);
}

#[tokio::test]
async fn a_fixture_file_is_read() {
    let p = super::project();
    let dir = p.make_dir("ft_get_fixture");
    std::fs::write(dir.join("brick.fixture"), brick("Brick")).unwrap();
    let parsed = p.the_type("Brick", Some("ft_get_fixture")).await.unwrap();
    assert_eq!(parsed["file"], "brick.fixture");
    assert_eq!(parsed["extension"], "fixture");
    assert_eq!(parsed["referential"], true);
    assert!(
        parsed["dsl"].as_str().unwrap().contains("from gdtf("),
        "{parsed}"
    );
}

#[tokio::test]
async fn a_type_is_found_in_an_imported_file_by_its_declaration() {
    let p = super::project();
    imported_dir(
        p.dir.path(),
        "ft_get_imported",
        "Astera-PixelBrick",
        brick("Astera-PixelBrick"),
    );
    let parsed = p
        .the_type("Astera-PixelBrick", Some("ft_get_imported"))
        .await
        .unwrap();
    assert_eq!(parsed["file"], "astera_pixelbrick.fixture");
    assert_eq!(parsed["referential"], true);
}

#[tokio::test]
async fn a_type_that_is_not_there_is_not_found() {
    let p = super::project();
    p.make_dir("ft_notfound");
    let status = p
        .the_type("nonexistent", Some("ft_notfound"))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
    assert!(
        status
            .message()
            .contains("Fixture type not found: nonexistent"),
        "{}",
        status.message()
    );
}

#[tokio::test]
async fn a_gdtf_in_the_library_with_no_record_is_a_fixture_with_no_file() {
    let p = super::project();
    let shelf = p.make_dir("lighting/library");
    std::fs::write(shelf.join("loose.gdtf"), synthetic_gdtf_bytes()).unwrap();
    let list = p.types(Some("ft_none"), None).await.unwrap();
    let loose = &list["fixture_types"]["Synth Brick"];
    assert_eq!(loose["referential"], true, "{list}");
    assert!(loose["file"].is_null(), "{loose}");
    assert!(loose["owner"].is_null(), "{loose}");
    assert_eq!(loose["form_editable"], false);
    let one = p.the_type("Synth Brick", Some("ft_none")).await.unwrap();
    assert_eq!(one["referential"], true, "{one}");
    assert!(one["file"].is_null() && one["extension"].is_null() && one["owner"].is_null());
    assert!(one["dsl"].is_null(), "{one}");
}

// ---- SaveFixtureType ----

#[tokio::test]
async fn text_is_saved_as_a_new_fixture_file() {
    let p = super::project();
    let parsed = p
        .put_type(save("MyFixture", "ft_put_raw", par("MyFixture")))
        .await
        .unwrap();
    assert_eq!(parsed["status"], "saved");
    assert_eq!(parsed["name"], "MyFixture");
    assert!(parsed["rename"].is_null() && parsed["venue_error"].is_null());
    assert!(p.dir.path().join("ft_put_raw/myfixture.fixture").exists());
}

#[tokio::test]
async fn the_channel_map_form_is_saved_as_valid_text() {
    let p = super::project();
    let parsed = p
        .put_type(save_form(
            "JSONFixture",
            "ft_put_json",
            &[("red", 1), ("green", 2), ("blue", 3)],
        ))
        .await
        .unwrap();
    assert_eq!(parsed["status"], "saved");
    let file = p.dir.path().join("ft_put_json/jsonfixture.fixture");
    let content = std::fs::read_to_string(&file).unwrap();
    let types = crate::lighting::parser::parse_fixture_types(&content).unwrap();
    assert!(types.contains_key("JSONFixture"));
}

#[tokio::test]
async fn the_channel_map_form_carries_the_strobe_figures() {
    let p = super::project();
    let mut request = save_form("Strobes", "ft_form_strobe", &[("dimmer", 1), ("strobe", 2)]);
    let Some(proto::save_fixture_type_request::Body::ChannelMap(form)) = &mut request.body else {
        unreachable!()
    };
    form.max_strobe_frequency = Some(25.0);
    form.min_strobe_frequency = Some(0.5);
    form.strobe_dmx_offset = Some(10);
    p.put_type(request).await.unwrap();
    let list = p.types(Some("ft_form_strobe"), None).await.unwrap();
    let strobes = &list["fixture_types"]["Strobes"]["fixture_type"];
    assert_eq!(strobes["max_strobe_frequency"], 25.0);
    assert_eq!(strobes["min_strobe_frequency"], 0.5);
    assert_eq!(strobes["strobe_dmx_offset"], 10);
}

#[tokio::test]
async fn a_channel_map_form_with_a_bad_row_is_refused_with_the_rows_own_words() {
    let p = super::project();
    let refused = |request| async {
        let status = p.put_type(request).await.unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument);
        status.message().to_string()
    };
    assert_eq!(
        refused(save_form("Bad", "ft_bad_form", &[("dimmer", 0)])).await,
        "Channel 'dimmer' offset must be from 1 to 65535"
    );
    assert_eq!(
        refused(save_form("Bad", "ft_bad_form", &[("", 1)])).await,
        "A channel has no name"
    );
    assert!(!p.dir.path().join("ft_bad_form").exists());
}

#[tokio::test]
async fn a_name_that_is_not_a_plain_name_is_refused() {
    let p = super::project();
    for name in ["../evil", "..", "a/b", "", "lock", "validate"] {
        let status = p
            .put_type(save(name, "ft_test", par("Evil")))
            .await
            .unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument, "{name:?}");
    }
    assert!(!p.dir.path().join("ft_test").exists());
}

#[tokio::test]
async fn text_that_does_not_parse_is_refused_with_the_parsers_message() {
    let p = super::project();
    let status = p
        .put_type(save("BadDSL", "ft_bad", "invalid {{{ content"))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(
        status.message().starts_with("Invalid fixture type DSL: "),
        "{}",
        status.message()
    );
    assert!(!p.dir.path().join("ft_bad").exists());
}

#[tokio::test]
async fn a_save_with_no_body_is_the_empty_text_and_does_not_parse() {
    let p = super::project();
    let status = p
        .put_type(proto::SaveFixtureTypeRequest {
            name: "Nothing".to_string(),
            dir: Some("ft_nobody".to_string()),
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(!p.dir.path().join("ft_nobody").exists());
}

#[tokio::test]
async fn a_new_rich_type_is_a_fixture_file() {
    let p = super::project();
    p.put_type(save("Mover", "ft_rich", rich("Mover")))
        .await
        .unwrap();
    // A new type is always born a `.fixture`, rich or not.
    assert!(p.dir.path().join("ft_rich/mover.fixture").exists());
    assert!(!p.dir.path().join("ft_rich/mover.light").exists());
}

#[tokio::test]
async fn rich_channel_syntax_is_accepted_as_a_fixture_file() {
    let p = super::project();
    let mut request = save("Mover", "ft_rich_ok", rich("Mover"));
    request.file_form = proto::FixtureTypeFile::Fixture as i32;
    p.put_type(request).await.unwrap();
    assert!(p.dir.path().join("ft_rich_ok/mover.fixture").exists());
    assert!(!p.dir.path().join("ft_rich_ok/mover.light").exists());
}

#[tokio::test]
async fn the_form_refuses_a_rich_type_and_leaves_its_file() {
    let p = super::project();
    let dir = p.make_dir("ft_form_refusal");
    let file = dir.join("mover.fixture");
    std::fs::write(&file, rich("Mover")).unwrap();
    let status = p
        .put_type(save_form("Mover", "ft_form_refusal", &[("dimmer", 1)]))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(status.message().contains("as text"), "{}", status.message());
    // The rich definition is untouched.
    assert!(std::fs::read_to_string(&file).unwrap().contains("fine 2"));
}

#[tokio::test]
async fn a_body_that_renames_the_declaration_is_refused() {
    // The file is keyed on the name sent, so a body that renames the
    // declaration would write `mover.light` holding "Rover" — a file no
    // later read, save or delete could reach.
    let p = super::project();
    let dir = p.make_dir("ft_rename");
    let file = dir.join("mover.light");
    std::fs::write(&file, par("Mover")).unwrap();
    let status = p
        .put_type(save("Mover", "ft_rename", par("Rover")))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(
        status.message().contains("declares no fixture type"),
        "{}",
        status.message()
    );
    assert!(status.message().contains("Rover"), "{}", status.message());
    // The original is untouched, and no orphan was written.
    assert!(std::fs::read_to_string(&file).unwrap().contains("Mover"));
    assert!(!dir.join("rover.light").exists());
}

#[tokio::test]
async fn a_form_that_names_no_known_file_form_is_refused_for_either_body() {
    let p = super::project();
    for mut request in [
        save("Mover", "ft_bad_ext", par("Mover")),
        save_form("Mover", "ft_bad_ext", &[("dimmer", 1)]),
    ] {
        // A number no `FixtureTypeFile` has: a typed client cannot send
        // one, a hand-made one can, and is told which.
        request.file_form = 99;
        let status = p.put_type(request).await.unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument);
        assert!(
            status
                .message()
                .contains("Unknown fixture type extension: 99"),
            "{}",
            status.message()
        );
    }
    // Nothing was created — not even the directory.
    assert!(!p.dir.path().join("ft_bad_ext").exists());
}

#[tokio::test]
async fn asking_for_a_fixture_file_moves_a_light_file() {
    // The only way out of v1: an explicit form wins over the existing
    // file's extension, and the `.light` is retired behind it.
    let p = super::project();
    let dir = p.make_dir("ft_convert");
    std::fs::write(dir.join("mover.light"), par("Mover")).unwrap();
    let mut request = save("Mover", "ft_convert", rich("Mover"));
    request.file_form = proto::FixtureTypeFile::Fixture as i32;
    p.put_type(request).await.unwrap();
    assert!(dir.join("mover.fixture").exists());
    assert!(!dir.join("mover.light").exists());
}

#[tokio::test]
async fn a_save_with_no_form_keeps_the_existing_extension() {
    let p = super::project();
    let dir = p.make_dir("ft_keep_ext");
    std::fs::write(dir.join("mover.fixture"), rich("Mover")).unwrap();
    p.put_type(save("Mover", "ft_keep_ext", rich("Mover")))
        .await
        .unwrap();
    assert!(dir.join("mover.fixture").exists());
    assert!(!dir.join("mover.light").exists());
}

#[tokio::test]
async fn an_imported_file_is_saved_back_where_the_type_lives() {
    let p = super::project();
    let file = imported_dir(
        p.dir.path(),
        "ft_put_imported",
        "Astera-PixelBrick",
        rich("Astera-PixelBrick"),
    );
    let edited = rich("Astera-PixelBrick").replace("270", "180");
    let mut request = save("Astera-PixelBrick", "ft_put_imported", edited.clone());
    request.file_form = proto::FixtureTypeFile::Fixture as i32;
    p.put_type(request).await.unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), edited);
    // One type, one file: no second file at this API's own stem.
    assert_eq!(
        std::fs::read_dir(p.dir.path().join("ft_put_imported"))
            .unwrap()
            .count(),
        1
    );
}

#[tokio::test]
async fn a_gdtf_archive_that_is_not_there_is_refused_at_the_save() {
    // The lighting system resolves the archive against the project at
    // load; a save is the last moment the text is still in front of the
    // person who can fix the path.
    let p = super::project();
    let mut request = save("Brick", "ft_missing_archive", brick("Brick"));
    request.file_form = proto::FixtureTypeFile::Fixture as i32;
    let status = p.put_type(request).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(status.message().contains("GDTF archive"));
    assert!(status.message().contains("library/synth.gdtf"));
    assert!(!p.dir.path().join("ft_missing_archive").exists());
}

#[tokio::test]
async fn a_referential_type_whose_archive_is_there_is_saved() {
    let p = super::project();
    let library = p.make_dir("library");
    std::fs::write(library.join("synth.gdtf"), synthetic_gdtf_bytes()).unwrap();
    let mut request = save("Brick", "ft_present_archive", brick("Brick"));
    request.file_form = proto::FixtureTypeFile::Fixture as i32;
    p.put_type(request).await.unwrap();
    assert!(p
        .dir
        .path()
        .join("ft_present_archive/brick.fixture")
        .exists());
}

#[tokio::test]
async fn a_gdtf_archive_outside_the_project_is_refused_at_the_save() {
    let p = super::project();
    let dsl = "fixture_type \"Brick\" from gdtf(\"../../etc/passwd\") {\n}\n";
    let mut request = save("Brick", "ft_escaping_archive", dsl);
    request.file_form = proto::FixtureTypeFile::Fixture as i32;
    let status = p.put_type(request).await.unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(!p.dir.path().join("ft_escaping_archive").exists());
}

#[tokio::test]
async fn a_type_in_two_files_is_read_only() {
    let p = super::project();
    let dir = p.make_dir("ft_twin");
    // A GDTF import can land a `.fixture` beside a hand-written `.light`
    // of the same name: the name is declared twice, so neither file is
    // its alone, and the save is refused rather than guessing which.
    std::fs::write(dir.join("mover.light"), par("Mover")).unwrap();
    std::fs::write(dir.join("mover.fixture"), rich("Mover")).unwrap();
    let status = p
        .put_type(save("Mover", "ft_twin", rich("Mover")))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    // The refusal names both files, in its message: the editor shows the
    // message, and nothing else of the old answer's body.
    assert!(
        status.message().contains("mover.fixture") && status.message().contains("mover.light"),
        "{}",
        status.message()
    );
    assert!(dir.join("mover.fixture").exists());
    assert!(dir.join("mover.light").exists());
}

#[tokio::test]
async fn a_type_in_a_shared_file_is_read_but_not_deleted_or_form_saved() {
    let p = super::project();
    let dir = p.make_dir("ft_shared");
    let file = dir.join("house.light");
    let both = format!("{}\n{}\n", par("ParA"), par("ParB"));
    std::fs::write(&file, &both).unwrap();

    let parsed = p.the_type("ParB", Some("ft_shared")).await.unwrap();
    assert_eq!(parsed["file"], "house.light");

    let status = p.delete_type("ParB", Some("ft_shared")).await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert!(
        status.message().contains("\"ParA\""),
        "{}",
        status.message()
    );

    let status = p
        .put_type(save_form("ParB", "ft_shared", &[("red", 1)]))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);

    assert_eq!(std::fs::read_to_string(&file).unwrap(), both);
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
}

#[tokio::test]
async fn a_saved_type_is_read_back() {
    let p = super::project();
    p.put_type(save("RoundTrip", "ft_roundtrip", par("RoundTrip")))
        .await
        .unwrap();
    let parsed = p.the_type("RoundTrip", Some("ft_roundtrip")).await.unwrap();
    assert!(parsed["fixture_type"].is_object());
}

// ---- DeleteFixtureType ----

#[tokio::test]
async fn a_type_is_deleted_with_its_file() {
    let p = super::project();
    let dir = p.make_dir("ft_del");
    let file = dir.join("todelete.light");
    std::fs::write(&file, par("ToDelete")).unwrap();
    let parsed = p.delete_type("ToDelete", Some("ft_del")).await.unwrap();
    assert_eq!(parsed["status"], "deleted");
    assert!(!file.exists());
}

#[tokio::test]
async fn a_fixture_file_is_deleted() {
    let p = super::project();
    let dir = p.make_dir("ft_del_fixture");
    let file = dir.join("brick.fixture");
    std::fs::write(&file, brick("Brick")).unwrap();
    p.delete_type("Brick", Some("ft_del_fixture"))
        .await
        .unwrap();
    assert!(!file.exists());
}

#[tokio::test]
async fn an_imported_file_is_deleted_by_its_declaration() {
    let p = super::project();
    let file = imported_dir(
        p.dir.path(),
        "ft_del_imported",
        "Astera-PixelBrick",
        brick("Astera-PixelBrick"),
    );
    p.delete_type("Astera-PixelBrick", Some("ft_del_imported"))
        .await
        .unwrap();
    assert!(!file.exists());
}

#[tokio::test]
async fn deleting_a_type_that_is_not_there_is_not_found() {
    let p = super::project();
    p.make_dir("ft_del_nf");
    let status = p
        .delete_type("nonexistent", Some("ft_del_nf"))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
}

#[tokio::test]
async fn a_saved_type_is_deleted_and_then_not_found() {
    let p = super::project();
    p.put_type(save("Deletable", "ft_put_del", par("Deletable")))
        .await
        .unwrap();
    p.delete_type("Deletable", Some("ft_put_del"))
        .await
        .unwrap();
    let status = p
        .the_type("Deletable", Some("ft_put_del"))
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
}

#[tokio::test]
async fn deleting_a_hand_written_type_touches_no_archive() {
    let p = super::project();
    let types = p.make_dir("lighting/fixture_types");
    std::fs::write(types.join("par.light"), par("Par")).unwrap();
    let parsed = p.delete_type("Par", None).await.unwrap();
    assert!(parsed["archive_removed"].is_null());
    assert_eq!(parsed["archive_kept"], false);
}

#[tokio::test]
async fn deleting_a_gdtf_fixture_removes_its_archive_unless_another_uses_it() {
    let p = super::project();
    let library = p.make_dir("lighting/library");
    let archive = library.join("synth.gdtf");

    // A fixture that is its archive alone: deleting it removes the archive.
    std::fs::write(&archive, synthetic_gdtf_bytes()).unwrap();
    let parsed = p.delete_type("Synth Brick", None).await.unwrap();
    assert_eq!(
        parsed,
        serde_json::json!({
            "status": "deleted",
            "name": "Synth Brick",
            "archive_removed": "lighting/library/synth.gdtf",
            "archive_kept": false,
        })
    );
    assert!(!archive.exists());

    // Two records of one archive: the first delete keeps it.
    std::fs::write(&archive, synthetic_gdtf_bytes()).unwrap();
    let types = p.make_dir("lighting/fixture_types");
    std::fs::create_dir_all(p.dir.path().join("lighting/.cache")).unwrap();
    for (file, name) in [("a.fixture", "Brick A"), ("b.fixture", "Brick B")] {
        std::fs::write(
            types.join(file),
            format!(
                "fixture_type \"{name}\"\n  from gdtf(\"lighting/library/synth.gdtf\")\n{{\n}}\n"
            ),
        )
        .unwrap();
    }
    let parsed = p.delete_type("Brick A", None).await.unwrap();
    assert!(parsed["archive_removed"].is_null());
    assert_eq!(parsed["archive_kept"], true);
    assert!(archive.exists(), "Brick B still uses it");
    assert!(!types.join("a.fixture").exists());

    let parsed = p.delete_type("Brick B", None).await.unwrap();
    assert_eq!(parsed["archive_removed"], "lighting/library/synth.gdtf");
    assert_eq!(parsed["archive_kept"], false);
    assert!(!archive.exists());
    assert!(
        p.dir.path().join("lighting/.cache").is_dir(),
        "the cache is left alone"
    );
}

// ---- GetFixtureTypeGdtfDetails ----

#[tokio::test]
async fn the_details_answer_modes_and_a_rig_for_an_imported_file() {
    let p = super::project();
    let library = p.make_dir("library");
    std::fs::write(library.join("synth.gdtf"), synthetic_gdtf_bytes()).unwrap();
    // Named as `import-gdtf` names it, not by this API's own stem.
    imported_dir(
        p.dir.path(),
        "ft_gdtf_details",
        "Astera-PixelBrick",
        brick("Astera-PixelBrick"),
    );

    let parsed = p
        .type_gdtf("Astera-PixelBrick", Some("ft_gdtf_details"), None)
        .await
        .unwrap();
    assert_eq!(parsed["archive"], "library/synth.gdtf");
    assert!(
        parsed.get("mode").is_none(),
        "a type names no mode: {parsed}"
    );
    let modes = parsed["inspection"]["modes"].as_array().expect("modes");
    assert!(!modes.is_empty(), "{parsed}");
    assert!(modes.iter().any(|m| m["name"] == "8: RGBS"), "{parsed}");
    let rig = parsed["rig"].as_str().expect("a rig path");
    assert!(
        p.dir
            .path()
            .join("lighting/.cache")
            .join(crate::lighting::distill::ASSETS_DIR)
            .join(rig)
            .is_file(),
        "the rig must be in the asset store: {rig}"
    );
}

#[tokio::test]
async fn the_details_say_what_each_mode_lets_a_show_do() {
    let p = super::project();
    project_with_bricks(p.dir.path(), "ft_modes", "v_modes");
    let parsed = p
        .type_gdtf("Brick", Some("ft_modes"), Some("v_modes"))
        .await
        .unwrap();
    let modes = parsed["inspection"]["modes"].as_array().unwrap();
    let rgbs = modes.iter().find(|m| m["name"] == "8: RGBS").unwrap();
    assert!(rgbs["footprint"].as_u64().unwrap() > 0, "{rgbs}");
    assert!(rgbs.get("refused").is_none(), "{rgbs}");
    assert!(
        rgbs["capabilities"]
            .as_array()
            .unwrap()
            .contains(&json!("color")),
        "{rgbs}"
    );
    assert!(rgbs["cells"].is_number(), "{rgbs}");
    let channels = rgbs["channels"].as_array().unwrap();
    assert!(!channels.is_empty(), "{rgbs}");
    // By address offset.
    let offsets: Vec<u64> = channels.iter().map(|c| c[0].as_u64().unwrap()).collect();
    let mut sorted = offsets.clone();
    sorted.sort();
    assert_eq!(offsets, sorted);
}

#[tokio::test]
async fn the_details_name_the_venue_fixtures_and_the_facts() {
    let p = super::project();
    project_with_bricks(p.dir.path(), "ft_gdtf_used", "v_gdtf_used");

    let parsed = p
        .type_gdtf("Brick", Some("ft_gdtf_used"), Some("v_gdtf_used"))
        .await
        .unwrap();
    assert_eq!(
        parsed["venues"],
        serde_json::json!([
            {"name": "club", "fixtures": [
                {"name": "Solo", "mode": "Mover 16bit"},
                {"name": "Odd", "mode": null},
            ]},
            {"name": "stage", "fixtures": [
                {"name": "Brick1", "mode": "8: RGBS"},
                {"name": "Brick2", "mode": "8: RGBS"},
            ]},
        ]),
        "by venue name, fixtures in patch order with the mode each is driven in, \
         other types left out"
    );
    assert_eq!(parsed["about"], "A brick that is not real.");
    // "8: RGBS" drives the Base tree, whose first beam is the head's lens.
    assert_eq!(parsed["beam"]["type"], "Spot");
    assert_eq!(parsed["beam"]["beam_angle"], 12.0);
    assert_eq!(parsed["beam"]["field_angle"], 20.0);
    assert_eq!(parsed["beam"]["luminous_flux"], 5000.0);
    assert_eq!(parsed["beam"]["color_temperature"], 6500.0);
    assert_eq!(parsed["beam"]["power"], 120.0);
}

#[tokio::test]
async fn the_details_with_no_venues_list_none() {
    let p = super::project();
    project_with_bricks(p.dir.path(), "ft_gdtf_nov", "v_gdtf_nov");
    let parsed = p
        .type_gdtf("Brick", Some("ft_gdtf_nov"), Some("no_such_dir"))
        .await
        .unwrap();
    assert_eq!(parsed["venues"], serde_json::json!([]));
}

#[tokio::test]
async fn the_details_of_a_native_type_are_not_found() {
    let p = super::project();
    let dir = p.make_dir("ft_gdtf_native");
    std::fs::write(dir.join("par.light"), par("Par")).unwrap();
    let status = p
        .type_gdtf("Par", Some("ft_gdtf_native"), None)
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
    assert_eq!(status.message(), "fixture type \"Par\" is not a GDTF type");
}

#[tokio::test]
async fn the_details_of_an_unknown_type_are_not_found() {
    let p = super::project();
    p.make_dir("ft_gdtf_unknown");
    let status = p
        .type_gdtf("Nothing", Some("ft_gdtf_unknown"), None)
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::NotFound);
    assert_eq!(status.message(), "Fixture type not found: Nothing");
}

#[tokio::test]
async fn the_details_of_a_type_whose_archive_is_missing_are_a_bad_argument() {
    let p = super::project();
    let dir = p.make_dir("ft_gdtf_missing");
    std::fs::write(dir.join("brick.fixture"), brick("Brick")).unwrap();
    let status = p
        .type_gdtf("Brick", Some("ft_gdtf_missing"), None)
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(
        status.message().contains("library/synth.gdtf"),
        "{}",
        status.message()
    );
}

#[tokio::test]
async fn the_details_of_an_archive_that_does_not_parse_say_so() {
    let p = super::project();
    let library = p.make_dir("library");
    std::fs::write(library.join("synth.gdtf"), b"not a zip").unwrap();
    let dir = p.make_dir("ft_gdtf_junk");
    std::fs::write(dir.join("brick.fixture"), brick("Brick")).unwrap();
    let status = p
        .type_gdtf("Brick", Some("ft_gdtf_junk"), None)
        .await
        .unwrap_err();
    assert_eq!(status.code(), Code::InvalidArgument);
    assert!(
        status.message().starts_with("Not a parseable GDTF: "),
        "{}",
        status.message()
    );
}

#[tokio::test]
async fn a_library_gdtf_with_no_record_has_details_drawn_in_its_first_mode() {
    let p = super::project();
    let shelf = p.make_dir("lighting/library");
    std::fs::write(shelf.join("synth.gdtf"), synthetic_gdtf_bytes()).unwrap();
    let parsed = p.type_gdtf("Synth Brick", None, None).await.unwrap();
    assert_eq!(parsed["archive"], "lighting/library/synth.gdtf");
    assert!(parsed.get("mode").is_none(), "{parsed}");
    assert!(parsed["rig"].is_string(), "{parsed}");
}

#[tokio::test]
async fn the_default_directories_are_the_hosts_profiles() {
    // #673: the listing with no directory named reads where this host's
    // profile says the types are, as the engine does.
    let p = super::project();
    p.write_config(
        "songs: \".\"\nprofiles:\n  - hostname: not-this-host-x9\n    dmx:\n      \
         universes: []\n      lighting:\n        directories:\n          \
         fixture_types: other/types\n  - dmx:\n      universes: []\n      \
         lighting:\n        directories:\n          fixture_types: rig/types\n",
    );
    let rig = p.make_dir("rig/types");
    let other = p.make_dir("other/types");
    std::fs::write(rig.join("par.light"), par("Rig")).unwrap();
    std::fs::write(other.join("par.light"), par("Other")).unwrap();
    let list = p.types(None, None).await.unwrap();
    assert_eq!(names(&list), ["Rig"]);
    let one = p.the_type("Rig", None).await.unwrap();
    assert_eq!(one["file"], "par.light");
    p.put_type(proto::SaveFixtureTypeRequest {
        name: "Made".to_string(),
        body: Some(proto::save_fixture_type_request::Body::Dsl(par("Made"))),
        ..Default::default()
    })
    .await
    .unwrap();
    assert!(rig.join("made.fixture").exists());
    p.delete_type("Made", None).await.unwrap();
    assert!(!rig.join("made.fixture").exists());
}

#[tokio::test]
async fn a_config_that_does_not_load_is_an_internal_error_not_the_defaults() {
    let p = super::project();
    p.write_config(
        "songs: songs\ndmx:\n  lighting:\n    groups:\n      wash:\n        name: wash\n",
    );
    let status = p.types(None, None).await.unwrap_err();
    assert_eq!(status.code(), Code::Internal);
    assert!(
        status.message().contains("the config does not load"),
        "{}",
        status.message()
    );
    // Naming the directory needs no config to read.
    p.types(Some("anywhere"), Some("anywhere")).await.unwrap();
}

// ---- Venues are venues' ----

#[tokio::test]
async fn a_venue_file_is_not_a_fixture_type() {
    let p = super::project();
    let dir = p.make_dir("ft_with_venue");
    std::fs::write(dir.join("club.light"), venue("club")).unwrap();
    let list = p.types(Some("ft_with_venue"), None).await.unwrap();
    assert!(names(&list).is_empty(), "{list}");
}
