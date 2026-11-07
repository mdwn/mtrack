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

//! The web UI's venue and fixture type writers: the one rule's plan and
//! write ([`crate::lighting::writes`]) with their refusals answered as the
//! web API answers them, and the listings' owners. Both kinds go through
//! here, so a venue and a fixture type are saved, renamed and deleted alike.

// A refusal is the response itself, as across the web API.
#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::lighting;
use crate::lighting::files::{self, Editable, Kind, Owner, ReadOnly, Refusal};
use crate::webui::wire::fixture_types::LightingOwner;

type Response = axum::response::Response;

/// An owner as the listings carry it.
pub(super) fn wire_owner(kind: Kind, dir: &Path, owner: &Owner) -> LightingOwner {
    LightingOwner {
        file: lighting::project_files::display_in(dir, &owner.path),
        editable: owner.editable == Editable::Editable,
        reason: owner.reason(kind, dir),
    }
}

/// The 409 for a write to a name whose file is not its alone: the reason,
/// the file, and what else is there (`others`), or the other files
/// declaring it (`files`), or why it does not parse (`parse_error`).
pub(super) fn read_only_response(kind: Kind, dir: &Path, owner: &Owner) -> Response {
    let shown = |p: &Path| lighting::project_files::display_in(dir, p);
    let mut body = json!({
        "error": owner.reason(kind, dir).unwrap_or_default(),
        "file": shown(&owner.path),
        "read_only": true,
    });
    if let Editable::ReadOnly(why) = &owner.editable {
        match why {
            ReadOnly::Duplicate { files } => {
                body["files"] = json!(files.iter().map(|f| shown(f)).collect::<Vec<_>>());
            }
            ReadOnly::Shared { others } => body["others"] = json!(others),
            ReadOnly::Mixed { others } => {
                body["others"] = json!(others.iter().map(ToString::to_string).collect::<Vec<_>>());
            }
            ReadOnly::Unparseable { error } => body["parse_error"] = json!(error),
        }
    }
    (StatusCode::CONFLICT, Json(body)).into_response()
}

pub(super) use crate::lighting::writes::{Plan, Save};
use crate::lighting::writes::{Refused, WriteError};

/// The status every web answer to a refusal of the one rule takes: 404 for
/// a rename of a name no file declares, 500 for a file or directory that
/// cannot be read, 409 for the rest (a read-only file, a taken name or
/// file, a `.light` upgrade whose file is there).
pub(crate) fn refused_status(refused: &Refused) -> StatusCode {
    match refused {
        Refused::NotFound { .. } => StatusCode::NOT_FOUND,
        Refused::Claim(refusal) => claim_status(refusal),
        Refused::Unreadable { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        Refused::ReadOnly { .. } | Refused::UpgradeTaken { .. } => StatusCode::CONFLICT,
    }
}

/// [`refused_status`] for a name [`files::claim_new`],
/// [`files::claim_record_for`] or [`files::name_free`] refused.
pub(crate) fn claim_status(refusal: &Refusal) -> StatusCode {
    match refusal {
        Refusal::Unreadable { .. } => StatusCode::INTERNAL_SERVER_ERROR,
        _ => StatusCode::CONFLICT,
    }
}

/// The status of a failed write: 409 for a file to be made that appeared
/// since it was claimed, or one to be rewritten that was removed since it
/// was read; 500 otherwise.
pub(crate) fn write_status(e: &WriteError) -> StatusCode {
    // Both are a change made by someone else meanwhile.
    match e.cause() {
        WriteError::Exists { .. } | WriteError::Gone { .. } => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// An importer's error (GDTF or MVR, an inspection too) as the web API
/// answers it: the status, by [`refused_status`], [`claim_status`] and
/// [`write_status`] for a refusal of the one rule or a write, 409 for a
/// library archive of the same name with other bytes, and 400 for anything
/// else; and `{error, kind}`, `kind` saying which refusal it is so a client
/// can offer the right way on, never guessing from the status: `venue` (the
/// venue's name or file: another name may do), `record` (an embedded GDTF's
/// record file), `library` (an archive file name), `name` (a fixture type
/// name or its file), `write` (a file that could not be written), and
/// `unreadable` (a file or directory that could not be read, which no
/// other name gets past).
pub(crate) fn import_error(
    e: &(dyn std::error::Error + 'static),
) -> (StatusCode, serde_json::Value) {
    // An unreadable file or directory is no other name's to get past.
    let kind_of = |refusal: &Refusal, fallback: &'static str| match refusal {
        Refusal::Unreadable { .. } => "unreadable",
        _ => fallback,
    };
    let (status, kind) = if let Some(venue) = e.downcast_ref::<lighting::import::VenueRefused>() {
        let kind = match &venue.refused {
            Refused::Unreadable { .. } => "unreadable",
            Refused::Claim(refusal) => kind_of(refusal, "venue"),
            _ => "venue",
        };
        (refused_status(&venue.refused), Some(kind))
    } else if let Some(record) = e.downcast_ref::<lighting::import::RecordRefused>() {
        (
            claim_status(&record.refusal),
            Some(kind_of(&record.refusal, "record")),
        )
    } else if let Some(refusal) = e.downcast_ref::<Refusal>() {
        (claim_status(refusal), Some(kind_of(refusal, "name")))
    } else if let Some(write) = e.downcast_ref::<WriteError>() {
        (write_status(write), Some("write"))
    } else if e.downcast_ref::<lighting::import::LibraryClash>().is_some() {
        (StatusCode::CONFLICT, Some("library"))
    } else {
        (StatusCode::BAD_REQUEST, None)
    };
    let mut body = json!({"error": e.to_string()});
    if let Some(kind) = kind {
        body["kind"] = json!(kind);
    }
    (status, body)
}

/// [`Save::plan`], a refusal answered as the web API answers it, with the
/// status [`refused_status`] gives; a read-only file's 409 also carries the
/// file and what else is in it.
pub(super) fn plan(save: &Save<'_>) -> Result<Plan, Response> {
    save.plan().map_err(|refused| match refused {
        Refused::ReadOnly { kind, dir, owner } => read_only_response(kind, &dir, &owner),
        other => (
            refused_status(&other),
            Json(json!({"error": other.to_string()})),
        )
            .into_response(),
    })
}

/// [`crate::lighting::writes::write`], a failure answered with the status
/// [`write_status`] gives.
pub(super) fn write(plan: &Plan, content: &str) -> Result<(), Response> {
    crate::lighting::writes::write(plan, content)
        .map_err(|e| (write_status(&e), Json(json!({"error": e.to_string()}))).into_response())
}

/// The file a delete of `name` removes: its owner's, when that is the
/// name's alone. `Ok(None)`: no file declares it.
pub(super) fn deletable(
    kind: Kind,
    dir: &Path,
    record: &files::Record,
    name: &str,
) -> Result<Option<PathBuf>, Response> {
    match record.owner(name) {
        None => Ok(None),
        Some(owner) if owner.editable == Editable::Editable => Ok(Some(owner.path)),
        Some(owner) => Err(read_only_response(kind, dir, &owner)),
    }
}

/// The 400 for a text body that is not one file of exactly this name: it
/// must declare `name`, and nothing else, as an editable file holds.
pub(super) fn require_alone(kind: Kind, name: &str, text: &str) -> Result<(), Response> {
    let bad = |e: String| (StatusCode::BAD_REQUEST, Json(json!({"error": e}))).into_response();
    let contents = files::Contents::parse(kind, text)
        .map_err(|e| bad(format!("Invalid {} DSL: {e}", kind.noun())))?;
    let editable = contents.alone(name).ok_or_else(|| {
        let declared = files::unique(&contents.names);
        let found = if declared.is_empty() {
            "it declares none".to_string()
        } else {
            format!("it declares \"{}\"", declared.join("\", \""))
        };
        bad(format!(
            "the DSL declares no {noun} named \"{name}\" ({found}); rename the {noun} in the Name \
             field, not in the text",
            noun = kind.noun()
        ))
    })?;
    let also = match editable {
        Editable::Editable => return Ok(()),
        Editable::ReadOnly(ReadOnly::Duplicate { .. }) => {
            format!("declares {} \"{name}\" more than once", kind.noun())
        }
        Editable::ReadOnly(ReadOnly::Shared { others }) => format!(
            "also declares {} {}",
            kind.noun(),
            others
                .iter()
                .map(|n| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Editable::ReadOnly(ReadOnly::Mixed { others }) => format!(
            "also holds {}",
            others
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Editable::ReadOnly(ReadOnly::Unparseable { error }) => error,
    };
    Err(bad(format!(
        "the text {also}; a {noun} saved here is a file of that one {noun} and nothing else",
        noun = kind.noun()
    )))
}

#[cfg(test)]
mod tests {
    use super::super::router;
    use super::super::test_helpers::*;
    use axum::body::Body;
    use axum::http::StatusCode;
    use serde_json::{json, Value};
    use std::path::{Path, PathBuf};
    use tower::ServiceExt;

    /// A request to the lighting API, as the UI sends it.
    async fn call(
        app: &axum::Router,
        method: &str,
        uri: &str,
        content_type: &str,
        body: impl Into<Body>,
    ) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method(method)
                    .uri(uri)
                    .header("content-type", content_type)
                    .body(body.into())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let text = response_body(response).await;
        (status, serde_json::from_str(&text).unwrap_or_default())
    }

    /// A project with `files` (path, text) under `rel`, and its router.
    fn project(rel: &str, files: &[(&str, &str)]) -> (axum::Router, tempfile::TempDir, PathBuf) {
        let (state, dir) = test_state();
        let d = dir.path().join(rel);
        std::fs::create_dir_all(&d).unwrap();
        for (path, text) in files {
            let path = d.join(path);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        (router().with_state(state), dir, d)
    }

    /// Every file under `dir` (relative) and its text: what a refused write
    /// must leave exactly as it was.
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
        format!(
            "fixture_type \"{name}\" {{\n  channels: 1\n  channel_map: {{ \"dimmer\": 1 }}\n}}\n"
        )
    }

    fn rich(name: &str) -> String {
        format!(
            "fixture_type \"{name}\" {{\n  channel \"pan\" @ 1 fine 2 range -270deg..270deg\n}}\n"
        )
    }

    fn venue_json(mode: Option<&str>) -> Vec<u8> {
        let mut fixture =
            json!({"name": "A", "fixture_type": "Par", "universe": 1, "start_channel": 1});
        if let Some(mode) = mode {
            fixture["mode"] = json!(mode);
        }
        serde_json::to_vec(&json!({ "fixtures": [fixture] })).unwrap()
    }

    fn ft_json() -> Vec<u8> {
        serde_json::to_vec(&json!({"channels": {"dimmer": 1}})).unwrap()
    }

    const V: &str = "v_writes";
    const T: &str = "t_writes";

    fn venue_uri(name: &str, extra: &str) -> String {
        format!(
            "/lighting/venues/{}?dir={V}{extra}",
            name.replace(' ', "%20")
        )
    }

    fn type_uri(name: &str, extra: &str) -> String {
        format!(
            "/lighting/fixture-types/{}?dir={T}{extra}",
            name.replace(' ', "%20")
        )
    }

    // --- Saves of a new name.

    #[tokio::test]
    async fn a_new_venue_is_claimed_at_its_canonical_venue_file() {
        let (app, _dir, d) = project(V, &[]);
        let (status, _) = call(
            &app,
            "PUT",
            &venue_uri("Club (Main)", ""),
            "application/json",
            venue_json(None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(snapshot(&d).len(), 1);
        assert!(std::fs::read_to_string(d.join("club_main.venue"))
            .unwrap()
            .contains("venue \"Club (Main)\""));
    }

    #[tokio::test]
    async fn a_new_name_whose_file_is_taken_is_refused() {
        // `club_main.venue` holds "Club Main"; "club main" has its stem.
        let (app, _dir, d) = project(V, &[("club_main.venue", &venue("Club Main"))]);
        let before = snapshot(&d);
        let (status, body) = call(
            &app,
            "PUT",
            &venue_uri("club main", ""),
            "text/plain",
            venue("club main"),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        let error = body["error"].as_str().unwrap();
        assert!(
            error.contains("club_main.venue is taken by \"Club Main\""),
            "{error}"
        );
        assert_eq!(snapshot(&d), before);
    }

    #[tokio::test]
    async fn a_new_type_is_claimed_at_its_canonical_fixture_file_in_either_form() {
        let (app, _dir, d) = project(T, &[]);
        let (status, _) = call(
            &app,
            "PUT",
            &type_uri("Wash Par", ""),
            "application/json",
            ft_json(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        // `ext=light` names the form an existing `.light` keeps; a new type
        // is a `.fixture` all the same.
        let (status, _) = call(
            &app,
            "PUT",
            &type_uri("Spot", "&ext=light"),
            "text/plain",
            par("Spot"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let names: Vec<String> = snapshot(&d).into_iter().map(|(f, _)| f).collect();
        assert_eq!(names, ["spot.fixture", "wash_par.fixture"]);
    }

    #[tokio::test]
    async fn a_new_type_may_not_take_an_unrecorded_library_name() {
        let (app, dir, d) = project(T, &[]);
        let library = dir.path().join("lighting/library");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(
            library.join("synth.gdtf"),
            crate::lighting::gdtf::build_zip(&[(
                "description.xml",
                crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.as_bytes(),
            )]),
        )
        .unwrap();
        let unrecorded = crate::lighting::library::unrecorded_types(dir.path(), Some(&d));
        let name = unrecorded.types[0].name.clone();
        let (status, body) =
            call(&app, "PUT", &type_uri(&name, ""), "text/plain", par(&name)).await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("synth.gdtf"),
            "{body}"
        );
        assert!(snapshot(&d).is_empty());
    }

    // --- Saves of an editable name.

    #[tokio::test]
    async fn an_editable_venue_is_rewritten_where_it_lives() {
        let text = "# mine\nvenue \"House\" {\n  fixture \"A\" Par @ 1:1  # keep\n}\n";
        let (app, _dir, d) = project(V, &[("tour/2024 rig.light", text)]);
        let (status, _) = call(
            &app,
            "PUT",
            &venue_uri("House", ""),
            "application/json",
            venue_json(None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let files = snapshot(&d);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "tour/2024 rig.light");
        assert!(files[0].1.contains("# keep"), "{}", files[0].1);
    }

    #[tokio::test]
    async fn an_editable_type_is_rewritten_where_it_lives() {
        let (app, _dir, d) = project(T, &[("old/par-v1.light", &par("Par"))]);
        let (status, _) = call(
            &app,
            "PUT",
            &type_uri("Par", ""),
            "application/json",
            ft_json(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let names: Vec<String> = snapshot(&d).into_iter().map(|(f, _)| f).collect();
        assert_eq!(names, ["old/par-v1.light"]);
    }

    #[tokio::test]
    async fn a_light_venue_needing_the_new_format_keeps_its_own_stem() {
        let (app, _dir, d) = project(V, &[("tour/old.light", &venue("Big Room"))]);
        let (status, _) = call(
            &app,
            "PUT",
            &venue_uri("Big Room", ""),
            "application/json",
            venue_json(Some("8: RGBS")),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let names: Vec<String> = snapshot(&d).into_iter().map(|(f, _)| f).collect();
        // The file keeps its name, as a rename elsewhere does.
        assert_eq!(names, ["tour/old.venue"]);
    }

    #[tokio::test]
    async fn a_light_upgrade_whose_target_exists_is_refused() {
        let (app, _dir, d) = project(
            V,
            &[("rig.light", &venue("Rig")), ("rig.venue", &venue("Other"))],
        );
        let before = snapshot(&d);
        let (status, body) = call(
            &app,
            "PUT",
            &venue_uri("Rig", ""),
            "application/json",
            venue_json(Some("8: RGBS")),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("rig.venue is there"),
            "{body}"
        );
        assert_eq!(snapshot(&d), before);

        // Nor beside the same stem in another case.
        let (app, _dir, d) = project(
            V,
            &[
                ("tour/rig.light", &venue("Rig")),
                ("tour/RIG.venue", &venue("Other")),
            ],
        );
        let before = snapshot(&d);
        let (status, body) = call(
            &app,
            "PUT",
            &venue_uri("Rig", ""),
            "application/json",
            venue_json(Some("8: RGBS")),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("tour/RIG.venue"),
            "{body}"
        );
        assert_eq!(snapshot(&d), before);
    }

    #[tokio::test]
    async fn a_light_type_needing_the_new_format_moves_to_its_stem_fixture() {
        let (app, _dir, d) = project(T, &[("mover.light", &par("Mover"))]);
        let (status, _) = call(
            &app,
            "PUT",
            &type_uri("Mover", "&ext=light"),
            "text/plain",
            rich("Mover"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let files = snapshot(&d);
        assert_eq!(files, [("mover.fixture".to_string(), rich("Mover"))]);

        // Asked for, a plain type moves too.
        let (app, _dir, d) = project(T, &[("wash.light", &par("Wash"))]);
        let (status, _) = call(
            &app,
            "PUT",
            &type_uri("Wash", "&ext=fixture"),
            "text/plain",
            par("Wash"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let names: Vec<String> = snapshot(&d).into_iter().map(|(f, _)| f).collect();
        assert_eq!(names, ["wash.fixture"]);
    }

    #[tokio::test]
    async fn a_light_type_upgrade_whose_target_exists_is_refused() {
        let (app, _dir, d) = project(
            T,
            &[
                ("mover.light", &par("Mover")),
                ("mover.fixture", &par("Other")),
            ],
        );
        let before = snapshot(&d);
        let (status, _) = call(
            &app,
            "PUT",
            &type_uri("Mover", ""),
            "text/plain",
            rich("Mover"),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(snapshot(&d), before);
    }

    #[tokio::test]
    async fn a_fixture_file_is_not_written_back_as_light() {
        let (app, _dir, d) = project(T, &[("wash.fixture", &par("Wash"))]);
        let before = snapshot(&d);
        let (status, _) = call(
            &app,
            "PUT",
            &type_uri("Wash", "&ext=light"),
            "text/plain",
            par("Wash"),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(snapshot(&d), before);
    }

    #[tokio::test]
    async fn the_channel_map_form_saves_only_a_type_it_can_say_all_of() {
        let plain = par("Wash");
        let with = |line: &str| plain.replace("\n}\n", &format!("\n  {line}\n}}\n"));
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
            let (app, _dir, d) = project(T, &[("wash.fixture", &text)]);
            let before = snapshot(&d);
            let (_, list) = call(
                &app,
                "GET",
                &format!("/lighting/fixture-types?dir={T}"),
                "text/plain",
                Body::empty(),
            )
            .await;
            assert_eq!(
                list["fixture_types"]["Wash"]["form_editable"], false,
                "{what}"
            );
            let (status, body) = call(
                &app,
                "PUT",
                &type_uri("Wash", ""),
                "application/json",
                ft_json(),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{what}: {body}");
            assert!(
                body["error"].as_str().unwrap().contains("as text"),
                "{body}"
            );
            assert_eq!(snapshot(&d), before, "{what}");
        }
        // A plain type, in either extension, is the form's to open and save.
        for file in ["wash.fixture", "wash.light"] {
            let (app, _dir, _) = project(T, &[(file, &plain)]);
            let (_, list) = call(
                &app,
                "GET",
                &format!("/lighting/fixture-types?dir={T}"),
                "text/plain",
                Body::empty(),
            )
            .await;
            assert_eq!(
                list["fixture_types"]["Wash"]["form_editable"], true,
                "{file}"
            );
            let (status, _) = call(
                &app,
                "PUT",
                &type_uri("Wash", ""),
                "application/json",
                ft_json(),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{file}");
        }
    }

    // --- Saves of a read-only name.

    #[tokio::test]
    async fn a_venue_sharing_its_file_is_read_only() {
        let shared = format!("{}{}", venue("Arena"), venue("Club"));
        let (app, _dir, d) = project(V, &[("tour/all.light", &shared)]);
        let before = snapshot(&d);
        for (ctype, body) in [
            ("application/json", venue_json(None)),
            ("text/plain", venue("Club").into_bytes()),
        ] {
            let (status, reply) = call(&app, "PUT", &venue_uri("Club", ""), ctype, body).await;
            assert_eq!(status, StatusCode::CONFLICT);
            assert_eq!(reply["read_only"], true);
            assert_eq!(reply["file"], "tour/all.light");
            assert_eq!(reply["others"], json!(["Arena"]));
            assert_eq!(
                reply["error"],
                "Defined in tour/all.light alongside venue \"Arena\"; edit that file by hand."
            );
        }
        let (status, _) = call(
            &app,
            "DELETE",
            &venue_uri("Club", ""),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(snapshot(&d), before);
    }

    #[tokio::test]
    async fn a_venue_in_a_file_with_other_items_is_read_only() {
        let mixed = format!("{}{}", par("Par"), venue("House"));
        let (app, _dir, d) = project(V, &[("house.light", &mixed)]);
        let before = snapshot(&d);
        let (status, reply) = call(
            &app,
            "PUT",
            &venue_uri("House", ""),
            "application/json",
            venue_json(None),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(reply["others"], json!(["fixture_type \"Par\""]));
        assert_eq!(snapshot(&d), before);
    }

    #[tokio::test]
    async fn a_venue_declared_twice_is_read_only() {
        let (app, _dir, d) = project(
            V,
            &[
                ("a.venue", &venue("House")),
                ("b/house.venue", &venue("House")),
            ],
        );
        let before = snapshot(&d);
        let (status, reply) = call(
            &app,
            "PUT",
            &venue_uri("House", ""),
            "application/json",
            venue_json(None),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(reply["files"], json!(["a.venue", "b/house.venue"]));
        let (status, _) = call(
            &app,
            "DELETE",
            &venue_uri("House", ""),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(snapshot(&d), before);
    }

    #[tokio::test]
    async fn a_type_sharing_its_file_is_read_only() {
        let shared = format!("{}{}", par("Par"), par("Wash"));
        let (app, _dir, d) = project(T, &[("all.light", &shared)]);
        let before = snapshot(&d);
        let (status, reply) = call(
            &app,
            "PUT",
            &type_uri("Wash", ""),
            "text/plain",
            par("Wash"),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(reply["others"], json!(["Par"]));
        let (status, _) = call(
            &app,
            "DELETE",
            &type_uri("Wash", ""),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(snapshot(&d), before);
    }

    // --- DSL bodies.

    #[tokio::test]
    async fn a_dsl_body_declares_this_one_block_and_nothing_else() {
        let (app, _dir, d) = project(V, &[("house.venue", &venue("House"))]);
        let before = snapshot(&d);
        let show = "show \"S\" {\n  @00:00.000\n  front: static color: \"red\"\n}\n";
        for body in [
            format!("{}{}", venue("House"), venue("Annex")),
            format!("{}{}", venue("House"), par("Par")),
            format!("{}{show}", venue("House")),
            format!("{}{}", venue("House"), venue("House")),
            venue("Elsewhere"),
        ] {
            let (status, reply) = call(
                &app,
                "PUT",
                &venue_uri("House", ""),
                "text/plain",
                body.clone(),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body}: {reply}");
        }
        let (app_t, _dir_t, dt) = project(T, &[]);
        let (status, _) = call(
            &app_t,
            "PUT",
            &type_uri("Par", ""),
            "text/plain",
            format!("{}{}", par("Par"), par("Wash")),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(snapshot(&dt).is_empty());
        assert_eq!(snapshot(&d), before);
    }

    // --- Deletes.

    #[tokio::test]
    async fn delete_removes_an_editable_file_and_finds_no_other() {
        let (app, _dir, d) = project(
            V,
            &[
                ("tour/rig.venue", &venue("House")),
                ("broken.venue", "venue \"Gone\" {"),
            ],
        );
        let (status, _) = call(
            &app,
            "DELETE",
            &venue_uri("House", ""),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(!d.join("tour/rig.venue").exists());
        // A name only a broken file mentions is not there.
        let (status, _) = call(
            &app,
            "DELETE",
            &venue_uri("Gone", ""),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert!(d.join("broken.venue").exists());

        let (app, _dir, d) = project(T, &[("x/par.light", &par("Par"))]);
        let (status, _) = call(
            &app,
            "DELETE",
            &type_uri("Par", ""),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(snapshot(&d).is_empty());
        let (status, _) = call(
            &app,
            "DELETE",
            &type_uri("Par", ""),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    // --- Renames.

    #[tokio::test]
    async fn a_rename_moves_a_canonical_file_to_the_new_canonical_file() {
        let text = "# mine\nvenue \"House\" {\n  fixture \"A\" Par @ 1:1  # keep\n}\n";
        let (app, _dir, d) = project(V, &[("house.venue", text)]);
        let (status, _) = call(
            &app,
            "PUT",
            &venue_uri("Big Room", "&from=House"),
            "application/json",
            venue_json(None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let files = snapshot(&d);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "big_room.venue");
        assert!(
            files[0].1.starts_with("# mine\nvenue \"Big Room\" {"),
            "{}",
            files[0].1
        );
        assert!(files[0].1.contains("# keep"));

        // A type, renamed through its text.
        let (app, _dir, d) = project(T, &[("par.light", &par("Par"))]);
        let (status, _) = call(
            &app,
            "PUT",
            &type_uri("Wash", "&from=Par"),
            "text/plain",
            par("Wash"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(snapshot(&d), [("wash.fixture".to_string(), par("Wash"))]);
    }

    #[tokio::test]
    async fn a_rename_elsewhere_renames_the_block_and_keeps_the_file_name() {
        let (app, _dir, d) = project(V, &[("tour/2024.venue", &venue("House"))]);
        let (status, reply) = call(
            &app,
            "PUT",
            &venue_uri("Arena", "&from=House"),
            "application/json",
            venue_json(None),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{reply}");
        let files = snapshot(&d);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "tour/2024.venue");
        assert!(files[0].1.contains("venue \"Arena\""), "{}", files[0].1);

        // A new name with the old one's stem keeps the file too.
        let (app, _dir, d) = project(V, &[("house.venue", &venue("house"))]);
        let (status, _) = call(
            &app,
            "PUT",
            &venue_uri("House", "&from=house"),
            "application/json",
            venue_json(None),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let files = snapshot(&d);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "house.venue");
        assert!(files[0].1.contains("venue \"House\""));
    }

    #[tokio::test]
    async fn a_rename_is_refused_for_a_taken_name_a_taken_file_or_a_read_only_one() {
        let (app, _dir, d) = project(
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
        let (status, _) = call(
            &app,
            "PUT",
            &venue_uri("Arena", "&from=House"),
            "application/json",
            venue_json(None),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        // The new canonical file is taken.
        let (status, reply) = call(
            &app,
            "PUT",
            &venue_uri("Annex", "&from=House"),
            "application/json",
            venue_json(None),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(
            reply["error"].as_str().unwrap().contains("annex.venue"),
            "{reply}"
        );
        // Read-only.
        let (status, reply) = call(
            &app,
            "PUT",
            &venue_uri("A3", "&from=A1"),
            "application/json",
            venue_json(None),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(reply["read_only"], true);
        // Nothing to rename.
        let (status, _) = call(
            &app,
            "PUT",
            &venue_uri("A4", "&from=Nowhere"),
            "application/json",
            venue_json(None),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(snapshot(&d), before);
    }

    // --- Listings.

    #[tokio::test]
    async fn the_listings_give_each_names_owner() {
        let shared = format!("{}{}", venue("Arena"), venue("Club"));
        let (app, _dir, _) = project(
            V,
            &[
                ("house.venue", &venue("House")),
                ("tour/all.light", &shared),
                ("bad.venue", "venue {"),
            ],
        );
        let (status, list) = call(
            &app,
            "GET",
            &format!("/lighting/venues?dir={V}"),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            list["owners"]["House"],
            json!({"file": "house.venue", "editable": true, "reason": null})
        );
        assert_eq!(list["owners"]["Club"]["editable"], false);
        assert_eq!(
            list["owners"]["Club"]["reason"],
            "Defined in tour/all.light alongside venue \"Club\"; edit that file by hand."
                .replace("\"Club\"", "\"Arena\"")
        );
        // A file that does not parse is listed by its error, with no name.
        assert_eq!(list["errors"][0]["file"], "bad.venue");

        let (app, _dir, _) = project(
            T,
            &[
                ("par.light", &par("Par")),
                ("all.fixture", &format!("{}{}", par("A"), par("B"))),
            ],
        );
        let (_, list) = call(
            &app,
            "GET",
            &format!("/lighting/fixture-types?dir={T}"),
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(list["fixture_types"]["Par"]["owner"]["editable"], true);
        assert_eq!(list["fixture_types"]["A"]["owner"]["editable"], false);
        assert_eq!(
            list["fixture_types"]["A"]["owner"]["reason"],
            "Defined in all.fixture alongside fixture type \"B\"; edit that file by hand."
        );
    }

    #[tokio::test]
    async fn a_rename_at_a_stale_version_moves_nothing() {
        let (app, _dir, d) = project(V, &[("house.venue", &venue("House"))]);
        let before = snapshot(&d);
        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri(venue_uri("Big Room", "&from=House"))
                    .header("content-type", "application/json")
                    .header("if-match", "\"not-the-version\"")
                    .body(Body::from(venue_json(None)))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body: Value = serde_json::from_str(&response_body(response).await).unwrap();
        assert_eq!(body["conflict"], true);
        assert_eq!(snapshot(&d), before);
    }

    // --- The harness's layout stays editable.

    #[tokio::test]
    async fn the_harness_layout_stays_editable_where_it_lives() {
        let (state, dir) = test_state();
        let venues = dir.path().join("lighting/venues");
        let types = dir.path().join("lighting/fixture_types");
        std::fs::create_dir_all(&venues).unwrap();
        std::fs::create_dir_all(&types).unwrap();
        std::fs::write(venues.join("built-in.venue"), venue("built-in")).unwrap();
        std::fs::write(venues.join("test.venue"), venue("test")).unwrap();
        // As the importer writes it: a record of its GDTF in the library.
        let library = dir.path().join("lighting/library");
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
        let app = router().with_state(state);

        let (_, list) = call(&app, "GET", "/lighting/venues", "text/plain", Body::empty()).await;
        assert_eq!(list["owners"]["built-in"]["editable"], true);
        assert_eq!(list["owners"]["test"]["editable"], true);
        let (_, list) = call(
            &app,
            "GET",
            "/lighting/fixture-types",
            "text/plain",
            Body::empty(),
        )
        .await;
        assert_eq!(
            list["fixture_types"]["Astera-PixelBrick"]["owner"]["editable"],
            true
        );
        // A record is text, never the channel map's.
        assert_eq!(
            list["fixture_types"]["Astera-PixelBrick"]["form_editable"],
            false
        );

        for name in ["built-in", "test"] {
            let (status, _) = call(
                &app,
                "PUT",
                &format!("/lighting/venues/{name}"),
                "application/json",
                venue_json(Some("8: RGBS")),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }
        // Not at its canonical path (`astera-pixelbrick.fixture`): saved
        // where it lives.
        let (status, _) = call(
            &app,
            "PUT",
            "/lighting/fixture-types/Astera-PixelBrick",
            "text/plain",
            record.replace("{\n}", "{\n  movement { max_pan_speed: 90deg/s }\n}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            std::fs::read_to_string(types.join("astera_pixelbrick.fixture"))
                .unwrap()
                .contains("max_pan_speed")
        );
        let mut names: Vec<String> = snapshot(&dir.path().join("lighting"))
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

    // --- A type's rename in the editor is the fixture settings' rename.

    #[tokio::test]
    async fn an_editor_rename_of_a_type_retypes_every_venue_using_it() {
        let (app, dir, d) = project(T, &[("par.fixture", &par("Par"))]);
        let venues = dir.path().join(V);
        std::fs::create_dir_all(venues.join("tour")).unwrap();
        std::fs::write(venues.join("club.venue"), venue("club")).unwrap();
        std::fs::write(venues.join("tour/2024.venue"), venue("arena")).unwrap();

        let (status, body) = call(
            &app,
            "PUT",
            &type_uri("Wash", &format!("&from=Par&venues_dir={V}")),
            "text/plain",
            par("Wash"),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body}");
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
        let (app, dir, d) = project(T, &[("par.fixture", &par("Par"))]);
        let venues = dir.path().join(V);
        std::fs::create_dir_all(&venues).unwrap();
        std::fs::write(venues.join("club.venue"), venue("club")).unwrap();
        std::fs::write(
            venues.join("all.venue"),
            format!("{}{}", venue("arena"), venue("annex")),
        )
        .unwrap();
        let (types_before, venues_before) = (snapshot(&d), snapshot(&venues));

        let (status, body) = call(
            &app,
            "PUT",
            &type_uri("Wash", &format!("&from=Par&venues_dir={V}")),
            "text/plain",
            par("Wash"),
        )
        .await;

        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        let error = body["error"].as_str().unwrap();
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
}
