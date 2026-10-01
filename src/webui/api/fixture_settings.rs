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

//! A GDTF fixture type's own settings — its name, its default mode and its
//! movement limits — as the fixture page's form edits them (lighting UI
//! design §12.4). The type's file is patched, not regenerated
//! ([`lighting::fixture_patch`]). A rename rewrites every venue line that
//! names the type, planned in full before anything is written; a new default
//! says which venue fixtures take it and what they would newly overlap, so
//! the page can ask before it saves.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path as FsPath, PathBuf};

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use super::super::config_io;
use super::super::server::WebUiState;
use super::lighting_api::{
    canonical_project_root, content_version, existing_fixture_type_file, if_match_version,
    locate_fixture_type_file, project_root, resolve_lighting_dir, system_from_files,
    validate_lighting_name, venue_error_after_save, DEFAULT_FIXTURE_TYPES_DIR, DEFAULT_VENUES_DIR,
    VENUE_EXTENSIONS, VENUE_WRITES,
};
use crate::lighting;
use crate::lighting::fixture_patch::{current_settings, patch_fixture_type, FixtureSettings};
use crate::lighting::patch::{venue_overlaps, venue_overruns, PatchSpan};
use crate::lighting::types::{Fixture, MovementLimits, Venue};

/// The directories a settings request works in.
#[derive(serde::Deserialize, Default)]
pub(super) struct SettingsQuery {
    dir: Option<String>,
    venues_dir: Option<String>,
}

/// The body of a settings plan or save.
#[derive(serde::Deserialize)]
pub(super) struct SettingsRequest {
    name: String,
    default_mode: Option<String>,
    #[serde(default)]
    movement: MovementLimits,
    /// `false` answers what the save would do and writes nothing.
    write: bool,
    /// The venue files' versions the plan was made against, by file name; a
    /// save refuses when any has changed since.
    #[serde(default)]
    venue_versions: HashMap<String, String>,
}

/// A refusal; a 409 carries `conflict` so a client reloads and asks again,
/// as it does for a venue save.
fn error(status: StatusCode, message: String) -> Response {
    let conflict = status == StatusCode::CONFLICT;
    (
        status,
        Json(json!({"error": message, "conflict": conflict})),
    )
        .into_response()
}

/// GET /api/lighting/fixture-types/:name/settings — the settings a GDTF
/// type's file holds, for the form: `{name, default_mode, movement, file,
/// version}`. A native type is a 404 here (it is edited as before).
pub(super) async fn get_settings(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<SettingsQuery>,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        DEFAULT_FIXTURE_TYPES_DIR,
    )?;
    let Some((path, _)) = locate_fixture_type_file(&dir, &name).await? else {
        return Err(error(
            StatusCode::NOT_FOUND,
            format!("Fixture type not found: {name}"),
        ));
    };
    let read = path.clone();
    let content = super::helpers::spawn_blocking_io("read fixture type", move || {
        std::fs::read_to_string(&read)
    })
    .await?;
    let Some(settings) = current_settings(&content, &name) else {
        return Err(error(
            StatusCode::NOT_FOUND,
            format!("fixture type \"{name}\" is not a GDTF type"),
        ));
    };
    Ok::<_, Response>(
        Json(json!({
            "name": settings.name,
            "default_mode": settings.default_mode,
            "movement": settings.movement,
            "file": crate::util::filename_display(&path),
            "version": content_version(content.as_bytes()),
        }))
        .into_response(),
    )
}

/// One venue file's planned rewrite for a rename.
struct VenueRewrite {
    path: PathBuf,
    file: String,
    content: String,
    /// (venue, lines changed).
    venues: Vec<(String, usize)>,
}

/// What a settings save would do, worked out from the files.
struct Plan {
    type_path: PathBuf,
    type_file: String,
    type_version: String,
    patched: String,
    rewrites: Vec<VenueRewrite>,
    /// Every venue file read, by file name, with its version.
    venue_versions: BTreeMap<String, String>,
    default_change: Option<serde_json::Value>,
    overlaps: Vec<serde_json::Value>,
    overruns: Vec<serde_json::Value>,
    config_references: Vec<String>,
}

/// A planning refusal: the status and the message.
type Refusal = (StatusCode, String);

/// The venue files of a directory (both extensions), sorted, each with its
/// text; one that cannot be read is an error naming it.
fn venue_files(dir: &FsPath) -> Result<Vec<(PathBuf, String)>, Refusal> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| VENUE_EXTENSIONS.contains(&e))
        })
        .collect();
    paths.sort();
    paths
        .into_iter()
        .map(|p| {
            std::fs::read_to_string(&p)
                .map(|text| (p.clone(), text))
                .map_err(|e| {
                    (
                        StatusCode::CONFLICT,
                        format!(
                            "venue file {} cannot be read ({e}); nothing was written",
                            crate::util::filename_display(&p)
                        ),
                    )
                })
        })
        .collect()
}

/// A venue with every fixture of type `from` re-typed `to`, and how many.
fn retyped(venue: &Venue, from: &str, to: &str) -> (Venue, usize) {
    let mut count = 0;
    let fixtures = venue
        .fixtures()
        .iter()
        .map(|(name, f)| {
            let f = if f.fixture_type() == from {
                count += 1;
                Fixture::new(
                    f.name().to_string(),
                    to.to_string(),
                    f.universe(),
                    f.start_channel(),
                    f.tags().to_vec(),
                )
                .with_mode(f.mode().map(str::to_string))
                .with_position(f.position())
                .with_rotation(f.rotation())
            } else {
                f.clone()
            };
            (name.clone(), f)
        })
        .collect();
    let renamed = Venue::new(venue.name().to_string(), fixtures)
        .with_focus_points(venue.focus_points().clone())
        .with_source(venue.source().cloned());
    (renamed, count)
}

/// Works out the whole save from the files. Nothing is written here.
#[allow(clippy::too_many_arguments)]
fn plan(
    root: &FsPath,
    types_dir: &FsPath,
    venues_dir: &FsPath,
    config_path: &FsPath,
    name: &str,
    request: &SettingsRequest,
    expected_type_version: Option<&str>,
) -> Result<Plan, Refusal> {
    let bad = |m: String| (StatusCode::BAD_REQUEST, m);
    let type_path = existing_fixture_type_file(types_dir, name).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            format!("Fixture type not found: {name}"),
        )
    })?;
    let content = std::fs::read_to_string(&type_path)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let type_version = content_version(content.as_bytes());
    let type_file = crate::util::filename_display(&type_path).to_string();
    if expected_type_version.is_some_and(|v| v != type_version) {
        return Err((
            StatusCode::CONFLICT,
            format!(
                "fixture type \"{name}\" changed since you loaded it ({type_file}); it has been \
                 reloaded, so make your change again"
            ),
        ));
    }
    let current = current_settings(&content, name)
        .ok_or_else(|| bad(format!("fixture type \"{name}\" is not a GDTF type")))?;
    let new_name = request.name.trim().to_string();
    let renaming = new_name != name;
    if renaming {
        validate_lighting_name(&new_name).map_err(|_| {
            bad(format!(
                "\"{new_name}\" cannot be a fixture type name (letters, digits, spaces, \
                 hyphens and underscores; not a route name)"
            ))
        })?;
        if existing_fixture_type_file(types_dir, &new_name).is_some() {
            return Err((
                StatusCode::CONFLICT,
                format!("there is already a fixture type named \"{new_name}\""),
            ));
        }
    }
    let settings = FixtureSettings {
        name: new_name.clone(),
        default_mode: request
            .default_mode
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_string),
        movement: request.movement,
    };
    let patched = patch_fixture_type(&content, name, &settings).map_err(bad)?;

    // Every venue file: read, versioned, and — for a rename — patched.
    let files = if venues_dir.is_dir() {
        venue_files(venues_dir)?
    } else {
        Vec::new()
    };
    let mut venue_versions = BTreeMap::new();
    let mut rewrites = Vec::new();
    let mut parsed: Vec<(String, Venue)> = Vec::new();
    for (path, text) in &files {
        let file = crate::util::filename_display(path).to_string();
        let version = content_version(text.as_bytes());
        if let Some(expected) = request.venue_versions.get(&file) {
            if *expected != version {
                return Err((
                    StatusCode::CONFLICT,
                    format!(
                        "venue file {file} changed since the plan was made; nothing was written \
                         — review the change again"
                    ),
                ));
            }
        }
        venue_versions.insert(file.clone(), version.clone());
        let venues = match lighting::parser::parse_venues(text) {
            Ok(venues) => venues,
            // A file that does not parse cannot be checked for the name, so
            // a rename cannot promise to have found every line.
            Err(e) if renaming => {
                return Err((
                    StatusCode::CONFLICT,
                    format!(
                        "venue file {file} does not parse ({e}), so the fixtures in it cannot be \
                         renamed; nothing was written — fix it by hand first"
                    ),
                ))
            }
            Err(_) => continue,
        };
        let mut names: Vec<&String> = venues.keys().collect();
        names.sort();
        let mut text_now = text.clone();
        let mut touched = Vec::new();
        for venue_name in names {
            let venue = &venues[venue_name];
            parsed.push((venue_name.clone(), venue.clone()));
            if !renaming {
                continue;
            }
            let (desired, count) = retyped(venue, name, &new_name);
            if count == 0 {
                continue;
            }
            text_now = lighting::venue_patch::patch_venue(&text_now, venue_name, &desired)
                .map_err(|e| {
                    (
                        StatusCode::CONFLICT,
                        format!(
                            "venue \"{venue_name}\" in {file} cannot be patched ({e}); nothing \
                             was written"
                        ),
                    )
                })?;
            touched.push((venue_name.clone(), count));
        }
        if !touched.is_empty() {
            rewrites.push(VenueRewrite {
                path: path.clone(),
                file,
                content: text_now,
                venues: touched,
            });
        }
    }

    // A new default: the fixtures that take it, and what they would newly
    // overlap or run past at its footprint.
    let mut default_change = None;
    let mut overlaps = Vec::new();
    let mut overruns = Vec::new();
    if settings.default_mode != current.default_mode {
        let takers: Vec<(String, Vec<&Fixture>)> = parsed
            .iter()
            .map(|(venue_name, venue)| {
                let fixtures: Vec<&Fixture> = venue
                    .fixtures_by_patch()
                    .into_iter()
                    .filter(|f| f.fixture_type() == name && f.mode().is_none())
                    .collect();
                (venue_name.clone(), fixtures)
            })
            .filter(|(_, f)| !f.is_empty())
            .collect();
        let count: usize = takers.iter().map(|(_, f)| f.len()).sum();
        let new_footprint = match &settings.default_mode {
            Some(mode) if count > 0 => {
                let declared = lighting::parser::parse_fixture_types(&content)
                    .ok()
                    .and_then(|t| t.get(name).cloned());
                declared.and_then(|d| {
                    lighting::system::LightingSystem::expand_mode(name, &d, mode, root)
                        .ok()
                        .map(|(t, _)| t.footprint())
                })
            }
            _ => None,
        };
        if count > 0 {
            let system = system_from_files(root, types_dir, venues_dir).ok();
            for (venue_name, fixtures) in &takers {
                let Some(venue) = parsed.iter().find(|(n, _)| n == venue_name).map(|(_, v)| v)
                else {
                    continue;
                };
                let changed: Vec<&str> = fixtures.iter().map(|f| f.name()).collect();
                let span = |f: &Fixture, footprint: Option<u16>| {
                    footprint.map(|footprint| PatchSpan {
                        fixture: f.name().to_string(),
                        universe: f.universe(),
                        address: f.start_channel(),
                        footprint,
                    })
                };
                let now = |f: &Fixture| {
                    system
                        .as_ref()
                        .and_then(|s| s.resolve_fixture_type(f).ok())
                        .map(|t| t.footprint())
                };
                let before: Vec<PatchSpan> = venue
                    .fixtures_by_patch()
                    .into_iter()
                    .filter_map(|f| span(f, now(f)))
                    .collect();
                let after: Vec<PatchSpan> = venue
                    .fixtures_by_patch()
                    .into_iter()
                    .filter_map(|f| {
                        if changed.contains(&f.name()) {
                            span(f, new_footprint)
                        } else {
                            span(f, now(f))
                        }
                    })
                    .collect();
                let was = venue_overlaps(&before);
                for o in venue_overlaps(&after) {
                    if was.contains(&o) {
                        continue;
                    }
                    overlaps.push(json!({
                        "venue": venue_name,
                        "a": o.first,
                        "b": o.second,
                        "a_gang": o.first_gang,
                        "b_gang": o.second_gang,
                        "universe": o.universe,
                        "from": o.from,
                        "to": o.to,
                        "message": o.to_string(),
                    }));
                }
                let was = venue_overruns(&before);
                for o in venue_overruns(&after) {
                    if !was.contains(&o) {
                        overruns.push(json!({
                            "venue": venue_name,
                            "fixture": o.fixture,
                            "message": o.to_string(),
                        }));
                    }
                }
            }
        }
        default_change = Some(json!({
            "from": current.default_mode,
            "to": settings.default_mode,
            "footprint": new_footprint,
            "count": count,
            "venues": takers
                .iter()
                .map(|(venue, f)| json!({
                    "venue": venue,
                    "fixtures": f.iter().map(|f| f.name()).collect::<Vec<_>>(),
                }))
                .collect::<Vec<_>>(),
        }));
    }

    // Inline fixtures in the player config name their type by string too;
    // the config is the user's to edit, so they are reported, not rewritten.
    let mut config_references = Vec::new();
    if renaming {
        if let Ok(config) = crate::config::Player::deserialize(config_path) {
            if let Some(lighting) = config.dmx().and_then(|d| d.lighting()) {
                for (fixture, spec) in lighting.fixtures() {
                    let type_name = spec.split('@').next().unwrap_or_default().trim();
                    if type_name.trim_matches('"') == name {
                        config_references.push(fixture.clone());
                    }
                }
            }
        }
        config_references.sort();
    }

    Ok(Plan {
        type_path,
        type_file,
        type_version,
        patched,
        rewrites,
        venue_versions,
        default_change,
        overlaps,
        overruns,
        config_references,
    })
}

/// The name of the venue the player plays against: the engine's, else the
/// config's.
fn current_venue_name(state: &WebUiState) -> Option<String> {
    if let Some(system) = state
        .player
        .broadcast_handles()
        .and_then(|h| h.lighting_system)
    {
        return system.lock().current_venue().map(str::to_string);
    }
    // codeql[rust/path-injection] config_path is set at startup, not user input.
    let config = crate::config::Player::deserialize(&state.config_path).ok()?;
    config
        .dmx()
        .and_then(|d| d.lighting())
        .and_then(|l| l.current_venue())
        .map(str::to_string)
}

/// POST /api/lighting/fixture-types/:name/settings — plans (`write: false`)
/// or saves (`write: true`) a GDTF type's settings: `{name, default_mode,
/// movement: {max_pan_speed, max_tilt_speed}}`. The type's file is patched in
/// place; a rename rewrites every venue line that names the type, in every
/// venue file, or — when any file cannot be rewritten — nothing at all. An
/// `If-Match` header carries the type file's version and `venue_versions`
/// the venue files' from the plan; either having changed is a 409. A save
/// holds the venue write lock, writes the venue files then the type file,
/// and reloads the running engine's types and venues once. The answer says
/// what changed (or would): `{write, file, version, rename, default_change,
/// overlaps, overruns, venue_versions, config_references, reloaded,
/// venue_error}`.
pub(super) async fn post_settings(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<SettingsQuery>,
    headers: axum::http::HeaderMap,
    Json(request): Json<SettingsRequest>,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let types_dir = resolve_lighting_dir(
        &state.config_path,
        query.dir.as_deref(),
        DEFAULT_FIXTURE_TYPES_DIR,
    )?;
    let venues_dir = resolve_lighting_dir(
        &state.config_path,
        query.venues_dir.as_deref(),
        DEFAULT_VENUES_DIR,
    )?;
    let root = canonical_project_root(&project_root(&state.config_path)?)?;
    let expected = if_match_version(&headers);
    let config_path = state.config_path.clone();
    let write = request.write;
    let (vdir, current) = (venues_dir.clone(), name.clone());
    let outcome = super::helpers::spawn_blocking_io("save fixture settings", move || {
        // The same lock as a venue save: a rename's venue writes and a drag
        // on the stage plot never interleave.
        let _guard = write.then(|| VENUE_WRITES.lock().unwrap_or_else(|e| e.into_inner()));
        let plan = match plan(
            &root,
            &types_dir,
            &vdir,
            &config_path,
            &current,
            &request,
            expected.as_deref(),
        ) {
            Ok(plan) => plan,
            Err(refusal) => return Ok::<_, String>(Err(refusal)),
        };
        if write {
            for rewrite in &plan.rewrites {
                config_io::staged_write(&rewrite.path, &rewrite.content)
                    .map_err(|e| format!("writing {}: {e}", rewrite.file))?;
            }
            config_io::staged_write(&plan.type_path, &plan.patched)
                .map_err(|e| format!("writing {}: {e}", plan.type_file))?;
        }
        Ok(Ok((plan, request.name.trim().to_string())))
    })
    .await?;
    let (plan, new_name) = outcome.map_err(|(status, message)| error(status, message))?;

    let renamed = new_name != name;
    let mut reloaded = false;
    let mut venue_error = serde_json::Value::Null;
    if write {
        // One reload after every write: types and venues together, so the
        // engine never sees a venue naming a type it has not read yet.
        let player = state.player.clone();
        reloaded = match tokio::task::spawn_blocking(move || player.reload_fixture_types()).await {
            Ok(Ok(())) => state.player.dmx_engine().is_some(),
            Ok(Err(e)) => {
                tracing::warn!(fixture_type = %new_name, error = %e, "fixture type saved, but the running engine could not reload it");
                false
            }
            Err(_) => false,
        };
        if let Some(current) = current_venue_name(&state) {
            venue_error = venue_error_after_save(&state, &current, &venues_dir).await;
        }
    }
    let version = content_version(if write {
        plan.patched.as_bytes()
    } else {
        plan.type_version.as_bytes()
    });
    let rename = renamed.then(|| {
        let venues: Vec<serde_json::Value> = plan
            .rewrites
            .iter()
            .flat_map(|r| {
                r.venues.iter().map(
                    move |(venue, lines)| json!({"venue": venue, "file": r.file, "lines": lines}),
                )
            })
            .collect();
        let lines: usize = plan
            .rewrites
            .iter()
            .flat_map(|r| &r.venues)
            .map(|(_, n)| n)
            .sum();
        json!({"from": name, "to": new_name, "venues": venues, "lines": lines})
    });
    let mut venue_versions = plan.venue_versions.clone();
    if write {
        for r in &plan.rewrites {
            venue_versions.insert(r.file.clone(), content_version(r.content.as_bytes()));
        }
    }
    Ok::<_, Response>(
        Json(json!({
            "write": write,
            "file": plan.type_file,
            "version": version,
            "dsl": plan.patched,
            "rename": rename,
            "default_change": plan.default_change,
            "overlaps": plan.overlaps,
            "overruns": plan.overruns,
            "venue_versions": venue_versions,
            "config_references": plan.config_references,
            "reloaded": reloaded,
            "venue_error": venue_error,
        }))
        .into_response(),
    )
}

#[cfg(test)]
mod test {
    use super::super::router;
    use super::super::test_helpers::*;
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    const TYPE_FILE: &str = "# Imported from synth.gdtf.\n# Keep this.\nfixture_type \"Brick\"\n  \
                             from gdtf(\"library/synth.gdtf\", mode \"8: RGBS\")\n{\n}\n";

    /// A project: the synthetic archive, `Brick` in `types/brick.fixture`,
    /// a comment-heavy `.venue` and a `.light` venue using it, and a venue
    /// that does not.
    fn project(root: &FsPath) {
        let library = root.join("library");
        std::fs::create_dir_all(&library).unwrap();
        std::fs::write(
            library.join("synth.gdtf"),
            crate::lighting::gdtf::build_zip(&[(
                "description.xml",
                crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.as_bytes(),
            )]),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("types")).unwrap();
        std::fs::write(root.join("types/brick.fixture"), TYPE_FILE).unwrap();
        std::fs::create_dir_all(root.join("venues")).unwrap();
        std::fs::write(
            root.join("venues/house.venue"),
            "# The house rig.\n# Measured 2026-09.\nvenue \"house\" {\n  # front truss\n  \
             fixture \"B1\" Brick @ 1:1 tags [\"wash\"]  # left\n  \
             fixture \"B2\" Brick mode \"Mover 16bit\" @ 1:40\n  \
             fixture \"P\" Other @ 1:100\n  focus \"center\" (0, 1, 1)\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("venues/club.light"),
            "venue \"club\" {\n  fixture \"C1\" Brick @ 2:1\n}\n",
        )
        .unwrap();
        std::fs::write(
            root.join("venues/other.light"),
            "venue \"other\" {\n  fixture \"X\" Other @ 1:1\n}\n",
        )
        .unwrap();
    }

    async fn post(
        state: WebUiState,
        name: &str,
        body: serde_json::Value,
        if_match: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        let mut request = http::Request::builder()
            .method("POST")
            .uri(format!(
                "/lighting/fixture-types/{name}/settings?dir=types&venues_dir=venues"
            ))
            .header("content-type", "application/json");
        if let Some(v) = if_match {
            request = request.header("if-match", v);
        }
        let response = router()
            .with_state(state)
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let text = response_body(response).await;
        (status, serde_json::from_str(&text).unwrap_or_default())
    }

    fn read(root: &FsPath, rel: &str) -> String {
        std::fs::read_to_string(root.join(rel)).unwrap()
    }

    #[tokio::test]
    async fn the_form_reads_the_settings_and_a_native_type_has_none() {
        let (state, dir) = test_state();
        project(dir.path());
        std::fs::write(
            dir.path().join("types/par.light"),
            "fixture_type \"Par\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n",
        )
        .unwrap();
        let get = |name: &str| {
            router().with_state(state.clone()).oneshot(
                http::Request::builder()
                    .uri(format!("/lighting/fixture-types/{name}/settings?dir=types"))
                    .body(Body::empty())
                    .unwrap(),
            )
        };
        let response = get("Brick").await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
        assert_eq!(body["default_mode"], "8: RGBS");
        assert_eq!(body["file"], "brick.fixture");
        assert!(body["movement"]["max_pan_speed"].is_null(), "{body}");
        assert_eq!(body["version"], content_version(TYPE_FILE.as_bytes()));
        assert_eq!(get("Par").await.unwrap().status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_rename_rewrites_every_venue_line_and_keeps_every_comment() {
        let (state, dir) = test_state();
        project(dir.path());
        let root = dir.path();
        let before_house = read(root, "venues/house.venue");
        let before_other = read(root, "venues/other.light");
        let body = json!({"name": "PixelBrick", "default_mode": "8: RGBS", "write": false});

        // The plan writes nothing and says what it would touch.
        let (status, plan) = post(state.clone(), "Brick", body.clone(), None).await;
        assert_eq!(status, StatusCode::OK, "{plan}");
        assert_eq!(plan["rename"]["lines"], 3, "{plan}");
        assert_eq!(
            plan["rename"]["venues"],
            json!([
                {"venue": "club", "file": "club.light", "lines": 1},
                {"venue": "house", "file": "house.venue", "lines": 2},
            ])
        );
        assert!(plan["default_change"].is_null());
        assert_eq!(read(root, "types/brick.fixture"), TYPE_FILE);

        let mut save = body.clone();
        save["write"] = json!(true);
        save["venue_versions"] = plan["venue_versions"].clone();
        let (status, saved) = post(
            state,
            "Brick",
            save,
            Some(&content_version(TYPE_FILE.as_bytes())),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert!(saved["venue_error"].is_null(), "{saved}");
        // The type keeps its file and its comments.
        assert_eq!(
            read(root, "types/brick.fixture"),
            TYPE_FILE.replace("\"Brick\"", "\"PixelBrick\"")
        );
        let house = read(root, "venues/house.venue");
        assert_eq!(
            house,
            before_house
                .replace("\"B1\" Brick @", "\"B1\" PixelBrick @")
                .replace("\"B2\" Brick mode", "\"B2\" PixelBrick mode"),
            "only the type changes, comments and layout stay"
        );
        assert!(read(root, "venues/club.light").contains("fixture \"C1\" PixelBrick @ 2:1"));
        assert_eq!(read(root, "venues/other.light"), before_other);
    }

    #[tokio::test]
    async fn a_venue_file_that_does_not_parse_blocks_a_rename_and_nothing_is_written() {
        let (state, dir) = test_state();
        project(dir.path());
        let root = dir.path();
        std::fs::write(root.join("venues/broken.venue"), "venue \"b\" {{{").unwrap();
        let house = read(root, "venues/house.venue");
        let (status, body) = post(
            state,
            "Brick",
            json!({"name": "PixelBrick", "default_mode": "8: RGBS", "write": true}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
        assert!(
            body["error"].as_str().unwrap().contains("broken.venue"),
            "{body}"
        );
        assert_eq!(read(root, "types/brick.fixture"), TYPE_FILE);
        assert_eq!(read(root, "venues/house.venue"), house);
    }

    #[tokio::test]
    async fn stale_versions_are_refused_before_anything_is_written() {
        let (state, dir) = test_state();
        project(dir.path());
        let root = dir.path();
        let body = json!({"name": "PixelBrick", "default_mode": "8: RGBS", "write": true});
        let (status, _) = post(state.clone(), "Brick", body.clone(), Some("not-it")).await;
        assert_eq!(status, StatusCode::CONFLICT);
        let mut stale = body.clone();
        stale["venue_versions"] = json!({"house.venue": "old"});
        let (status, refused) = post(state, "Brick", stale, None).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(refused["error"].as_str().unwrap().contains("house.venue"));
        assert_eq!(read(root, "types/brick.fixture"), TYPE_FILE);
    }

    #[tokio::test]
    async fn a_rename_onto_another_type_is_refused() {
        let (state, dir) = test_state();
        project(dir.path());
        std::fs::write(
            dir.path().join("types/other.light"),
            "fixture_type \"Other\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n",
        )
        .unwrap();
        let (status, body) = post(
            state,
            "Brick",
            json!({"name": "Other", "default_mode": "8: RGBS", "write": false}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
    }

    #[tokio::test]
    async fn a_new_default_says_who_takes_it_and_what_it_would_overlap() {
        let (state, dir) = test_state();
        project(dir.path());
        let root = dir.path();
        let (status, plan) = post(
            state.clone(),
            "Brick",
            json!({"name": "Brick", "default_mode": "Mover 16bit", "write": false}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{plan}");
        let change = &plan["default_change"];
        assert_eq!(change["from"], "8: RGBS");
        assert_eq!(change["to"], "Mover 16bit");
        // B2 names its own mode, so only B1 and C1 take the default.
        assert_eq!(change["count"], 2, "{plan}");
        assert_eq!(
            change["venues"],
            json!([
                {"venue": "club", "fixtures": ["C1"]},
                {"venue": "house", "fixtures": ["B1"]},
            ])
        );
        let footprint = change["footprint"]
            .as_u64()
            .expect("the new mode's footprint");
        // B1 at 1:1 runs into B2 at 1:40 only when the mover mode is that wide.
        let overlaps = plan["overlaps"].as_array().unwrap();
        assert_eq!(!overlaps.is_empty(), footprint >= 40, "{plan}");

        // Saved: only the mode string changes.
        let (status, saved) = post(
            state,
            "Brick",
            json!({"name": "Brick", "default_mode": "Mover 16bit", "write": true,
                   "movement": {"max_pan_speed": 240, "max_tilt_speed": null}}),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(
            read(root, "types/brick.fixture"),
            "# Imported from synth.gdtf.\n# Keep this.\nfixture_type \"Brick\"\n  \
             from gdtf(\"library/synth.gdtf\", mode \"Mover 16bit\")\n{\n  \
             movement { max_pan_speed: 240deg/s }\n}\n"
        );
    }

    #[tokio::test]
    async fn a_new_default_that_overlaps_names_the_fixtures() {
        let (state, dir) = test_state();
        project(dir.path());
        // Two default bricks back to back: any wider default overlaps.
        let wide = {
            let (_, plan) = post(
                state.clone(),
                "Brick",
                json!({"name": "Brick", "default_mode": "Mover 16bit", "write": false}),
                None,
            )
            .await;
            plan["default_change"]["footprint"].as_u64().unwrap()
        };
        std::fs::write(
            dir.path().join("venues/club.light"),
            "venue \"club\" {\n  fixture \"C1\" Brick @ 2:1\n  fixture \"C2\" Brick @ 2:5\n}\n",
        )
        .unwrap();
        let (_, plan) = post(
            state,
            "Brick",
            json!({"name": "Brick", "default_mode": "Mover 16bit", "write": false}),
            None,
        )
        .await;
        // 8: RGBS is 4 addresses, so C2 at 2:5 is adjacent; the mover mode
        // is wider and runs into it.
        assert!(wide > 4, "the mover mode is the wider: {wide}");
        let overlaps = plan["overlaps"].as_array().unwrap();
        assert_eq!(overlaps.len(), 1, "{plan}");
        assert_eq!(overlaps[0]["venue"], "club");
        assert_eq!(overlaps[0]["a"], "C1");
        assert_eq!(overlaps[0]["b"], "C2");
        assert!(overlaps[0]["message"].as_str().unwrap().contains("\"C2\""));
    }
}
