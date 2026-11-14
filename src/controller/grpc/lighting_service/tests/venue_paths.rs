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

//! The venue and MVR export calls take a name and directories from the
//! caller, and none of them may lead outside the project: a name is a plain
//! name (and not one of the words the API keeps), a directory is a
//! project-relative path that, followed through its links, stays inside.
//! Each is tried with `..`, an absolute path, a backslash, a NUL and a link
//! out, on every call that takes it.

use tonic::{Code, Request, Status};

use super::types::{brick, par, synthetic_gdtf_bytes};
use super::venues::{save_dsl, venue_dsl};
use super::Project;
use crate::proto::player::v1::{self as proto, lighting_service_server::LightingService};

/// A project and, outside it, a folder holding a venue, a fixture type and a
/// GDTF the directories below lead to: by `..`, by an absolute path and by a
/// link.
struct Outside {
    p: Project,
    /// A folder outside the project.
    away: tempfile::TempDir,
}

impl Outside {
    fn new() -> Self {
        let p = super::project();
        let away = tempfile::tempdir().unwrap();
        std::fs::write(away.path().join("leak.venue"), venue_dsl("Leak")).unwrap();
        std::fs::write(away.path().join("leak.fixture"), par("LeakType")).unwrap();
        std::fs::create_dir_all(away.path().join("library")).unwrap();
        std::fs::write(
            away.path().join("library/synth.gdtf"),
            synthetic_gdtf_bytes(),
        )
        .unwrap();
        // A venue of the project's own, with a type, for the calls that read
        // the venue before they look at the other directory.
        std::fs::create_dir_all(p.dir.path().join("lighting/fixture_types")).unwrap();
        std::fs::write(
            p.dir.path().join("lighting/fixture_types/glint.fixture"),
            brick("Glint"),
        )
        .unwrap();
        std::fs::create_dir_all(p.dir.path().join("library")).unwrap();
        std::fs::write(
            p.dir.path().join("library/synth.gdtf"),
            synthetic_gdtf_bytes(),
        )
        .unwrap();
        std::fs::create_dir_all(p.dir.path().join("lighting/venues")).unwrap();
        std::fs::write(
            p.dir.path().join("lighting/venues/hall.venue"),
            "venue \"Hall\" {\n  fixture \"G\" Glint mode \"8: RGBS\" @ 1:1 position (0, 0, 4)\n}\n",
        )
        .unwrap();
        // A link to a folder out of the root, directly and with a folder
        // beneath it, and a link inside a folder of the project.
        std::os::unix::fs::symlink(away.path(), p.dir.path().join("escape")).unwrap();
        std::fs::create_dir_all(p.dir.path().join("inside")).unwrap();
        std::os::unix::fs::symlink(away.path(), p.dir.path().join("inside/escape")).unwrap();
        Self { p, away }
    }

    /// Every spelling of a directory that is not inside the project.
    fn dirs(&self) -> Vec<String> {
        vec![
            "..".to_string(),
            "../outside".to_string(),
            "a/../../outside".to_string(),
            "inside/../../outside".to_string(),
            self.away.path().display().to_string(),
            "/etc".to_string(),
            "/tmp/evil".to_string(),
            "escape".to_string(),
            "escape/sub".to_string(),
            "inside/escape".to_string(),
            "inside/escape/sub".to_string(),
        ]
    }

    /// Every spelling of a name that is not a plain name, and the words the
    /// API keeps for its own routes.
    fn names() -> Vec<&'static str> {
        vec![
            "..",
            "../evil",
            "../../etc/passwd",
            "a/b",
            "a/../b",
            "/etc/passwd",
            "/tmp/evil",
            "evil/",
            "",
            "back\\slash",
            "nul\0byte",
            "lock",
            "validate",
            "activate",
        ]
    }

    /// What is outside is as it was, and nothing new is.
    fn untouched(&self) {
        let mut found: Vec<String> = std::fs::read_dir(self.away.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        found.sort();
        assert_eq!(found, ["leak.fixture", "leak.venue", "library"]);
        assert_eq!(
            std::fs::read_to_string(self.away.path().join("leak.venue")).unwrap(),
            venue_dsl("Leak")
        );
        assert_eq!(
            std::fs::read_dir(self.away.path().join("library"))
                .unwrap()
                .count(),
            1
        );
    }
}

/// Refused as a path that is not the caller's to name: not allowed, or not a
/// valid name.
fn refused(status: &Status) -> bool {
    matches!(
        status.code(),
        Code::InvalidArgument | Code::PermissionDenied
    )
}

/// A directory under a link out that is not there: nothing is read from it
/// (it holds nothing), so a read of it may answer with nothing, as it always
/// has; a write is held by the tests below.
fn unmade(dir: &str) -> bool {
    dir.ends_with("/sub")
}

/// A call's result for `dir`: refused, or, for a directory under a link out
/// that is not there, nothing of what is out there.
fn held<T: std::fmt::Debug>(call: &str, dir: &str, result: Result<T, Status>) {
    match result {
        Err(status) => assert!(
            refused(&status) || (unmade(dir) && status.code() == Code::NotFound),
            "{call} {dir:?}: {status:?}"
        ),
        Ok(answer) => {
            assert!(unmade(dir), "{call} {dir:?} answered {answer:?}");
            let shown = format!("{answer:?}");
            assert!(
                !shown.contains("Leak"),
                "{call} {dir:?} read what is outside: {shown}"
            );
        }
    }
}

// ---- A name ----

#[tokio::test]
async fn a_name_that_is_not_plain_is_refused_by_every_call_that_takes_one() {
    let o = Outside::new();
    let dir = Some("lighting/venues");
    for name in Outside::names() {
        let get = o.p.venue(name, dir).await.expect_err(name);
        assert_eq!(get.code(), Code::InvalidArgument, "get {name:?}");
        let delete = o.p.remove_venue(name, dir).await.expect_err(name);
        assert_eq!(delete.code(), Code::InvalidArgument, "delete {name:?}");
        let put =
            o.p.put_venue(save_dsl(name, "t", "x"))
                .await
                .expect_err(name);
        assert_eq!(put.code(), Code::InvalidArgument, "save {name:?}");
        let patch = o.p.patch(name, dir, None).await.expect_err(name);
        assert_eq!(patch.code(), Code::InvalidArgument, "patch {name:?}");
        let scene = o.p.scene(name, dir, None).await.expect_err(name);
        assert_eq!(scene.code(), Code::InvalidArgument, "scene {name:?}");
        let aim =
            o.p.aim(proto::AddVenueAimPointsRequest {
                name: name.to_string(),
                ..Default::default()
            })
            .await
            .expect_err(name);
        assert_eq!(aim.code(), Code::InvalidArgument, "aim {name:?}");
        let summary =
            o.p.export_summary(proto::GetMvrExportSummaryRequest {
                venue: name.to_string(),
                ..Default::default()
            })
            .await
            .expect_err(name);
        assert_eq!(summary.code(), Code::InvalidArgument, "summary {name:?}");
        let keep =
            o.p.keep(proto::KeepMvrExportRequest {
                venue: name.to_string(),
                ..Default::default()
            })
            .await
            .expect_err(name);
        assert_eq!(keep.code(), Code::InvalidArgument, "keep {name:?}");
    }
    // Nothing was made, in the project or out of it.
    assert!(!o.p.dir.path().join("t").exists());
    assert!(!o.p.dir.path().join("lighting/export").exists());
    o.untouched();
}

#[tokio::test]
async fn a_rename_source_that_is_not_plain_is_refused() {
    let o = Outside::new();
    for from in Outside::names() {
        if from.is_empty() {
            continue;
        }
        let mut request = save_dsl("Fine", "t", &venue_dsl("Fine"));
        request.from = Some(from.to_string());
        let status = o.p.put_venue(request).await.expect_err(from);
        assert_eq!(status.code(), Code::InvalidArgument, "{from:?}");
    }
    assert!(!o.p.dir.path().join("t").exists());
    o.untouched();
}

#[tokio::test]
async fn a_new_name_that_is_not_new_enough_is_refused() {
    // The rules for a name being created are stricter than those for one
    // already there (a leading dot, a space at either end).
    let p = super::project();
    for name in [".hidden", " padded", "padded "] {
        let status = p
            .put_venue(save_dsl(name, "t", &venue_dsl(name)))
            .await
            .unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument, "{name:?}");
    }
    // The directory was made on the way, as it always was; no file was.
    let made = std::fs::read_dir(p.dir.path().join("t")).map(|d| d.count());
    assert_eq!(made.unwrap_or(0), 0);
}

// ---- A directory ----

#[tokio::test]
async fn a_directory_that_leaves_the_project_is_refused_by_every_call_that_takes_one() {
    let o = Outside::new();
    for dir in o.dirs() {
        let named = Some(dir.as_str());
        held("list", &dir, o.p.venues(named).await);
        held("get", &dir, o.p.venue("Leak", named).await);
        held("delete", &dir, o.p.remove_venue("Leak", named).await);
        held(
            "patch",
            &dir,
            o.p.patch("Hall", named, Some("lighting/fixture_types"))
                .await,
        );
        held(
            "patch, types",
            &dir,
            o.p.patch("Hall", Some("lighting/venues"), named).await,
        );
        held(
            "scene",
            &dir,
            o.p.scene("Hall", named, Some("lighting/fixture_types"))
                .await,
        );
        held(
            "scene, types",
            &dir,
            o.p.scene("Hall", Some("lighting/venues"), named).await,
        );
        let aim = |venues: Option<&str>, types: Option<&str>| proto::AddVenueAimPointsRequest {
            name: "Hall".to_string(),
            venues_dir: venues.map(str::to_string),
            fixture_types_dir: types.map(str::to_string),
            ..Default::default()
        };
        held("aim", &dir, o.p.aim(aim(named, None)).await);
        held("aim, types", &dir, o.p.aim(aim(None, named)).await);
        let summary =
            |venues: Option<&str>, types: Option<&str>| proto::GetMvrExportSummaryRequest {
                venue: "Hall".to_string(),
                venues_dir: venues.map(str::to_string),
                fixture_types_dir: types.map(str::to_string),
                ..Default::default()
            };
        held(
            "summary",
            &dir,
            o.p.export_summary(summary(named, None)).await,
        );
        held(
            "summary, types",
            &dir,
            o.p.export_summary(summary(None, named)).await,
        );
        let keep = |venues: Option<&str>, types: Option<&str>| proto::KeepMvrExportRequest {
            venue: "Hall".to_string(),
            venues_dir: venues.map(str::to_string),
            fixture_types_dir: types.map(str::to_string),
            ..Default::default()
        };
        held("keep", &dir, o.p.keep(keep(named, None)).await);
        held("keep, types", &dir, o.p.keep(keep(None, named)).await);
    }
    // The project's own venue was never touched, and nothing was written
    // out there or kept in the project.
    assert!(!o.p.dir.path().join("lighting/export").exists());
    o.untouched();
}

#[tokio::test]
async fn a_save_never_writes_outside_the_project() {
    for dir in Outside::new().dirs() {
        let o = Outside::new();
        let status =
            o.p.put_venue(save_dsl("Evil", &dir, &venue_dsl("Evil")))
                .await
                .expect_err(&dir);
        assert!(refused(&status), "{dir:?}: {status:?}");
        o.untouched();
    }
    assert!(!std::path::Path::new("/tmp/evil/evil.venue").exists());
}

#[tokio::test]
async fn a_save_never_makes_a_directory_beneath_a_link_out() {
    let o = Outside::new();
    for dir in ["escape/new", "inside/escape/new/deeper"] {
        let status =
            o.p.put_venue(save_dsl("Evil", dir, &venue_dsl("Evil")))
                .await
                .expect_err(dir);
        assert!(refused(&status), "{dir:?}: {status:?}");
        assert!(!o.away.path().join("new").exists(), "{dir}");
    }
    o.untouched();
}

#[tokio::test]
async fn a_delete_never_removes_a_file_through_a_link_out() {
    let o = Outside::new();
    // A link in the project's own venues directory to a file outside: the
    // venue is found by what its file declares, and the delete removes the
    // link, never what it leads to.
    let venues = o.p.make_dir("v_link");
    std::os::unix::fs::symlink(o.away.path().join("leak.venue"), venues.join("leak.venue"))
        .unwrap();
    let before = std::fs::read_to_string(o.away.path().join("leak.venue")).unwrap();
    let outcome = o.p.remove_venue("Leak", Some("v_link")).await;
    assert_eq!(
        std::fs::read_to_string(o.away.path().join("leak.venue"))
            .unwrap_or_default()
            .as_str(),
        before,
        "what the link led to is as it was ({outcome:?})"
    );
}

#[tokio::test]
async fn an_export_file_name_never_leaves_the_export_directory() {
    let o = Outside::new();
    for file in [
        "../evil.mvr",
        "../../evil.mvr",
        "/tmp/evil.mvr",
        "a/b.mvr",
        "..",
        "evil.txt",
    ] {
        let kept =
            o.p.keep(proto::KeepMvrExportRequest {
                venue: "Hall".to_string(),
                file: Some(file.to_string()),
                overwrite: true,
                ..Default::default()
            })
            .await;
        match kept {
            Err(status) => assert_eq!(status.code(), Code::InvalidArgument, "{file:?}"),
            Ok(answer) => panic!("{file:?} was kept: {answer}"),
        }
        let summary =
            o.p.export_summary(proto::GetMvrExportSummaryRequest {
                venue: "Hall".to_string(),
                file: Some(file.to_string()),
                ..Default::default()
            })
            .await;
        if let Ok(answer) = &summary {
            // A summary writes nothing; it must not name a path outside the
            // export directory either.
            assert!(
                !answer["output"].as_str().unwrap().contains(".."),
                "{file:?}: {answer}"
            );
        }
    }
    assert!(!std::path::Path::new("/tmp/evil.mvr").exists());
    assert!(!o.p.dir.path().join("evil.mvr").exists());
    assert!(!o.p.dir.path().join("lighting/evil.mvr").exists());
    o.untouched();
}

// ---- Through the service's own request type ----

#[tokio::test]
async fn an_unset_directory_is_the_profiles_and_never_a_path_of_the_callers() {
    let o = Outside::new();
    // An unset and an empty directory are the profile's.
    for dir in [None, Some("")] {
        let reply =
            o.p.server
                .list_venues(Request::new(proto::ListVenuesRequest {
                    dir: dir.map(str::to_string),
                }))
                .await
                .unwrap()
                .into_inner();
        // The project's own `lighting/venues`, and nothing else.
        let found: Vec<&String> = reply.venues.keys().collect();
        assert_eq!(found, ["Hall"], "{dir:?}");
    }
}
