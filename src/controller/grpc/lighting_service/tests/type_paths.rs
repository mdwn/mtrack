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

//! The fixture type calls take a name and directories from the caller, and
//! none of them may lead outside the project: a name is a plain name, a
//! directory is a project-relative path that, followed through its links,
//! stays inside. Each is tried with `..`, an absolute path and a link out,
//! on every call that takes it.

use tonic::{Code, Request, Status};

use super::types::{brick, par, save, synthetic_gdtf_bytes};
use super::Project;
use crate::proto::player::v1::{self as proto, lighting_service_server::LightingService};

/// A project and, outside it, a folder holding a fixture type and a GDTF
/// the directories below lead to: by `..`, by an absolute path and by a link.
struct Outside {
    p: Project,
    /// A folder outside the project.
    away: tempfile::TempDir,
}

impl Outside {
    fn new() -> Self {
        let p = super::project();
        let away = tempfile::tempdir().unwrap();
        std::fs::write(away.path().join("leak.light"), par("Leak")).unwrap();
        std::fs::create_dir_all(away.path().join("library")).unwrap();
        std::fs::write(
            away.path().join("library/synth.gdtf"),
            synthetic_gdtf_bytes(),
        )
        .unwrap();
        std::fs::write(away.path().join("gdtf.fixture"), brick("Away")).unwrap();
        // A GDTF fixture of the project's own, for the calls that read its
        // archive before they look at the venues directory.
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

    /// Every spelling of a name that is not a plain name.
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
        assert_eq!(found, ["gdtf.fixture", "leak.light", "library"]);
        assert_eq!(
            std::fs::read_to_string(self.away.path().join("leak.light")).unwrap(),
            par("Leak")
        );
        assert_eq!(
            std::fs::read_dir(self.away.path().join("library"))
                .unwrap()
                .count(),
            1
        );
    }
}

/// Refused as a path that is not the caller's to name: not found, not
/// allowed, or not a valid name.
fn refused(status: &Status) -> bool {
    matches!(
        status.code(),
        Code::InvalidArgument | Code::PermissionDenied
    )
}

// ---- A name ----

#[tokio::test]
async fn a_name_that_is_not_plain_is_refused_by_every_call_that_takes_one() {
    let o = Outside::new();
    let dir = Some("lighting/fixture_types");
    for name in Outside::names() {
        let get = o.p.the_type(name, dir).await.expect_err(name);
        assert_eq!(get.code(), Code::InvalidArgument, "get {name:?}");
        let gdtf = o.p.type_gdtf(name, dir, None).await.expect_err(name);
        assert_eq!(gdtf.code(), Code::InvalidArgument, "gdtf {name:?}");
        let delete = o.p.delete_type(name, dir).await.expect_err(name);
        assert_eq!(delete.code(), Code::InvalidArgument, "delete {name:?}");
        let put =
            o.p.put_type(save(name, "t", par("X")))
                .await
                .expect_err(name);
        assert_eq!(put.code(), Code::InvalidArgument, "save {name:?}");
        let settings = o.p.settings(name, dir).await.expect_err(name);
        assert_eq!(settings.code(), Code::InvalidArgument, "settings {name:?}");
        let planned =
            o.p.save_settings(proto::SaveFixtureTypeSettingsRequest {
                name: name.to_string(),
                new_name: name.to_string(),
                ..Default::default()
            })
            .await
            .expect_err(name);
        assert_eq!(planned.code(), Code::InvalidArgument, "plan {name:?}");
    }
    // Nothing was made, in the project or out of it.
    assert!(!o.p.dir.path().join("t").exists());
    o.untouched();
}

#[tokio::test]
async fn a_rename_source_that_is_not_plain_is_refused() {
    let o = Outside::new();
    for from in Outside::names() {
        if from.is_empty() {
            continue;
        }
        let mut request = save("Fine", "t", par("Fine"));
        request.from = Some(from.to_string());
        let status = o.p.put_type(request).await.expect_err(from);
        assert_eq!(status.code(), Code::InvalidArgument, "{from:?}");
    }
    assert!(!o.p.dir.path().join("t").exists());
    o.untouched();
}

#[tokio::test]
async fn a_new_name_that_is_not_new_enough_is_refused_and_an_old_one_is_not() {
    // The rules for a name being created are stricter than those for one
    // already there (a leading dot, a space at either end).
    let p = super::project();
    for name in [".hidden", " padded", "padded "] {
        let status = p.put_type(save(name, "t", par(name))).await.unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument, "{name:?}");
    }
    // The directory was made on the way, as it always was; no file was.
    let made = std::fs::read_dir(p.dir.path().join("t")).map(|d| d.count());
    assert_eq!(made.unwrap_or(0), 0);
}

// ---- A directory ----

/// A directory under a link out that is not there: nothing is read from it
/// (it holds nothing), so a read of it may answer with nothing, as it
/// always has; a write is held by the next test.
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
                !shown.contains("Leak") && !shown.contains("Away"),
                "{call} {dir:?} read what is outside: {shown}"
            );
        }
    }
}

#[tokio::test]
async fn a_directory_that_leaves_the_project_is_refused_by_every_call_that_takes_one() {
    let o = Outside::new();
    for dir in o.dirs() {
        let named = Some(dir.as_str());
        held("list", &dir, o.p.types(named, None).await);
        held("list, venues", &dir, o.p.types(None, named).await);
        held("get", &dir, o.p.the_type("Leak", named).await);
        held("gdtf", &dir, o.p.type_gdtf("Glint", named, None).await);
        held(
            "gdtf, venues",
            &dir,
            o.p.type_gdtf("Glint", None, named).await,
        );
        held("delete", &dir, o.p.delete_type("Leak", named).await);
        held("settings", &dir, o.p.settings("Glint", named).await);
        for write in [false, true] {
            let request =
                |dir: Option<&str>, venues: Option<&str>| proto::SaveFixtureTypeSettingsRequest {
                    name: "Glint".to_string(),
                    new_name: "Glint".to_string(),
                    write,
                    dir: dir.map(str::to_string),
                    venues_dir: venues.map(str::to_string),
                    ..Default::default()
                };
            held(
                "settings save",
                &dir,
                o.p.save_settings(request(named, None)).await,
            );
            held(
                "settings save, venues",
                &dir,
                o.p.save_settings(request(None, named)).await,
            );
        }
    }
    o.untouched();
}

#[tokio::test]
async fn a_save_never_writes_outside_the_project() {
    for dir in Outside::new().dirs() {
        let o = Outside::new();
        let status =
            o.p.put_type(save("Evil", &dir, par("Evil")))
                .await
                .expect_err(&dir);
        assert!(refused(&status), "{dir:?}: {status:?}");
        // The venues directory of a rename is where the venue lines are
        // rewritten; it is held inside too.
        let types = o.p.make_dir("t_inside");
        std::fs::write(types.join("par.light"), par("Par")).unwrap();
        let mut request = save("Wash", "t_inside", par("Wash"));
        request.from = Some("Par".to_string());
        request.venues_dir = Some(dir.clone());
        held("rename", &dir, o.p.put_type(request).await);
        o.untouched();
    }
    assert!(!std::path::Path::new("/tmp/evil/evil.fixture").exists());
}

#[tokio::test]
async fn a_save_never_makes_a_directory_beneath_a_link_out() {
    let o = Outside::new();
    for dir in ["escape/new", "inside/escape/new/deeper"] {
        let status =
            o.p.put_type(save("Evil", dir, par("Evil")))
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
    // A link in the project's own types directory to a file outside: the
    // type is found by what its file declares, and the delete removes the
    // link, never what it leads to.
    let types = o.p.make_dir("t_link");
    std::os::unix::fs::symlink(o.away.path().join("leak.light"), types.join("leak.light")).unwrap();
    let before = std::fs::read_to_string(o.away.path().join("leak.light")).unwrap();
    let outcome = o.p.delete_type("Leak", Some("t_link")).await;
    assert_eq!(
        std::fs::read_to_string(o.away.path().join("leak.light"))
            .unwrap_or_default()
            .as_str(),
        before,
        "what the link led to is as it was ({outcome:?})"
    );
}

// ---- Through the service's own request type ----

#[tokio::test]
async fn an_unset_directory_is_the_profiles_and_never_a_path_of_the_callers() {
    let o = Outside::new();
    // An unset and an empty directory are the profile's.
    for dir in [None, Some("")] {
        let reply =
            o.p.server
                .list_fixture_types(Request::new(proto::ListFixtureTypesRequest {
                    dir: dir.map(str::to_string),
                    venues_dir: dir.map(str::to_string),
                }))
                .await
                .unwrap()
                .into_inner();
        // The project's own `lighting/fixture_types`, and nothing else.
        let found: Vec<&String> = reply.fixture_types.keys().collect();
        assert_eq!(found, ["Glint"], "{dir:?}");
    }
}
