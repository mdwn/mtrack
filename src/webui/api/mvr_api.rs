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

//! MVR import and export over HTTP (lighting UI design §11): the browser's
//! import wizard and export dialog. Every handler is a thin wrapper over the
//! library functions behind the CLI and the MCP tools
//! (`import::inspect_mvr_bytes`, `import::import_mvr_bytes`,
//! `export::export_mvr_bytes`), so the three surfaces cannot disagree.
//!
//! The server holds nothing between requests: the wizard uploads the file at
//! each step. Every write goes through a directory proven to be inside the
//! project (the same containment the GDTF import and the venue save use).

use std::collections::HashMap;

use axum::{
    extract::{Path, Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use serde_json::json;

use super::super::config_io;
use super::super::server::WebUiState;
use super::lighting_api::{
    project_root, reload_if_current_venue, resolve_lighting_dir, validate_lighting_name,
    DEFAULT_FIXTURE_TYPES_DIR, DEFAULT_VENUES_DIR,
};
use crate::lighting;
use crate::lighting::export::{MvrExportOptions, EXPORT_DIR};
use crate::lighting::import::MvrImportOptions;
use crate::lighting::types::Vec3;

/// The most a text field of an upload may hold.
const MAX_FIELD_BYTES: usize = 4096;

/// The `keep` field lists a name and fields per hand-edited fixture, so it
/// is allowed to be far longer than the other text fields.
const MAX_KEEP_BYTES: usize = 1024 * 1024;

fn bad_request(message: impl Into<String>) -> Response {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({"error": message.into()})),
    )
        .into_response()
}

/// An uploaded MVR and the text fields sent with it.
struct MvrForm {
    file_name: String,
    bytes: Vec<u8>,
    fields: HashMap<String, String>,
}

impl MvrForm {
    fn field(&self, key: &str) -> Option<&str> {
        self.fields
            .get(key)
            .map(|v| v.trim())
            .filter(|v| !v.is_empty())
    }
}

/// Reads a multipart body: the first file, and every other field as text.
/// The file name keeps only its final component, as the GDTF upload does —
/// it becomes a path in the library.
#[allow(clippy::result_large_err)]
async fn read_form(multipart: &mut axum::extract::Multipart) -> Result<MvrForm, Response> {
    let mut file: Option<(String, Vec<u8>)> = None;
    let mut fields = HashMap::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| bad_request(format!("Failed to read multipart field: {e}")))?
    {
        if let Some(raw) = field.file_name().map(|f| f.to_string()) {
            let name = std::path::Path::new(&raw)
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.is_empty() || file.is_some() {
                continue;
            }
            let bytes = field
                .bytes()
                .await
                .map_err(|e| bad_request(format!("Failed to read file data: {e}")))?;
            file = Some((name, bytes.to_vec()));
        } else if let Some(key) = field.name().map(str::to_string) {
            let text = field
                .text()
                .await
                .map_err(|e| bad_request(format!("Failed to read field {key}: {e}")))?;
            let limit = if key == "keep" {
                MAX_KEEP_BYTES
            } else {
                MAX_FIELD_BYTES
            };
            if text.len() > limit {
                return Err(bad_request(format!("Field {key} is too long")));
            }
            fields.insert(key, text);
        }
    }
    let Some((file_name, bytes)) = file else {
        return Err(bad_request("No file uploaded"));
    };
    if !file_name.to_ascii_lowercase().ends_with(".mvr") {
        return Err(bad_request(format!(
            "{file_name} is not an .mvr file; choose the venue's MVR export"
        )));
    }
    Ok(MvrForm {
        file_name,
        bytes,
        fields,
    })
}

/// Parses `"x,y,z"` millimeters.
#[allow(clippy::result_large_err)]
fn parse_origin(text: &str) -> Result<Vec3, Response> {
    let parts: Vec<f64> = text
        .split(',')
        .map(|p| p.trim().parse::<f64>().ok().filter(|v| v.is_finite()))
        .collect::<Option<_>>()
        .unwrap_or_default();
    match parts.as_slice() {
        [x, y, z] => Ok([*x, *y, *z]),
        _ => Err(bad_request(
            "origin must be three numbers in millimeters, \"x,y,z\"",
        )),
    }
}

/// A directory the caller named, or the default, proven to stay in the
/// project. Returns the project-relative string the library functions take.
#[allow(clippy::result_large_err)]
fn checked_dir(
    state: &WebUiState,
    requested: Option<&str>,
    default: &'static str,
) -> Result<String, Response> {
    resolve_lighting_dir(&state.config_path, requested, default)?;
    Ok(match requested {
        Some(dir) if !dir.is_empty() => dir.to_string(),
        _ => default.to_string(),
    })
}

/// The import options a form asks for, directories checked.
#[allow(clippy::result_large_err)]
fn import_options(state: &WebUiState, form: &MvrForm) -> Result<MvrImportOptions, Response> {
    Ok(MvrImportOptions {
        name: form.field("name").map(str::to_string),
        origin_mm: form.field("origin").map(parse_origin).transpose()?,
        fixture_types_dir: checked_dir(
            state,
            form.field("fixture_types_dir"),
            DEFAULT_FIXTURE_TYPES_DIR,
        )?,
        venues_dir: checked_dir(state, form.field("venues_dir"), DEFAULT_VENUES_DIR)?,
        keep: match form.field("keep") {
            Some(text) => serde_json::from_str(text).map_err(|e| {
                bad_request(format!(
                    "keep must be {{\"fixtures\": {{name: [fields]}}, \"focus_points\": [names]}}: {e}"
                ))
            })?,
            None => Default::default(),
        },
    })
}

/// POST /api/lighting/mvr/inspect — multipart: the file (and optionally
/// `name`, `origin`, `venues_dir`, `fixture_types_dir`). Answers
/// `{file_name, report, scene}`: `report` is the MCP `inspect_mvr` plan
/// (what an import would do), `scene` is the file seen from above — every
/// fixture's and focus point's position in the file's millimeters, the
/// scenery and, when the scenery carries a stage floor, its bounds
/// (`scene.deck_mm`, else null). Writes nothing. A file the importer
/// refuses (not an MVR, over the caps) is a 400 with the parser's reason.
pub(super) async fn inspect_mvr(
    State(state): State<WebUiState>,
    mut multipart: axum::extract::Multipart,
) -> impl IntoResponse {
    let form = read_form(&mut multipart).await?;
    let options = import_options(&state, &form)?;
    let project = project_root(&state.config_path)?;
    let file_name = form.file_name.clone();
    let outcome = super::helpers::spawn_blocking_io("inspect MVR", move || {
        Ok::<_, String>((|| {
            let report = lighting::import::inspect_mvr_bytes(
                &form.bytes,
                &form.file_name,
                &options,
                &project,
            )
            .map_err(|e| e.to_string())?;
            let scene =
                lighting::import::scene_view::scene_view(&form.bytes).map_err(|e| e.to_string())?;
            Ok::<_, String>((report, scene))
        })())
    })
    .await?
    .map_err(bad_request)?;
    let (report, scene) = outcome;
    Ok::<_, Response>(
        (
            StatusCode::OK,
            Json(json!({"file_name": file_name, "report": report, "scene": scene})),
        )
            .into_response(),
    )
}

/// POST /api/lighting/mvr/import — multipart: the file, `name`, `origin`
/// ("x,y,z" millimeters), `write` ("true" / "false", required), and
/// optionally `venues_dir`, `fixture_types_dir` and `keep` (JSON,
/// `{"fixtures": {"<name>": ["position", ...]}, "focus_points": [...]}`: the
/// hand edits a merge keeps rather than overwrites; the plan lists them as
/// `overwrites`). `write=false` answers
/// `{write: false, plan}` (what `import-mvr` prints without `--write`);
/// `write=true` performs the import and answers `{write: true, report,
/// reloaded}`, `reloaded` saying whether the running engine re-read the
/// venue it is playing. Importing never makes the venue current.
pub(super) async fn import_mvr(
    State(state): State<WebUiState>,
    mut multipart: axum::extract::Multipart,
) -> impl IntoResponse {
    let form = read_form(&mut multipart).await?;
    let write = match form.field("write") {
        Some("true") => true,
        Some("false") => false,
        _ => return Err(bad_request("write must be \"true\" or \"false\"")),
    };
    let options = import_options(&state, &form)?;
    let project = project_root(&state.config_path)?;
    if write {
        // Directories the import writes into are created, and proven to be
        // inside the project, before anything is written.
        for dir in [
            options.fixture_types_dir.as_str(),
            options.venues_dir.as_str(),
            "lighting/library",
        ] {
            super::helpers::ensure_configured_dir(&project.join(dir), &state).await?;
        }
    }
    let outcome = super::helpers::spawn_blocking_io("import MVR", move || {
        Ok::<_, String>(
            if write {
                lighting::import::import_mvr_bytes(&form.bytes, &form.file_name, &options, &project)
                    .map(|report| serde_json::to_value(&report).unwrap_or_default())
            } else {
                lighting::import::inspect_mvr_bytes(
                    &form.bytes,
                    &form.file_name,
                    &options,
                    &project,
                )
                .map(|plan| serde_json::to_value(&plan).unwrap_or_default())
            }
            .map_err(|e| e.to_string()),
        )
    })
    .await?
    .map_err(bad_request)?;
    if !write {
        return Ok::<_, Response>(
            (
                StatusCode::OK,
                Json(json!({"write": false, "plan": outcome})),
            )
                .into_response(),
        );
    }
    let venue = outcome["venue_name"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let reloaded = reload_if_current_venue(&state, &venue).await;
    Ok((
        StatusCode::OK,
        Json(json!({"write": true, "report": outcome, "reloaded": reloaded})),
    )
        .into_response())
}

/// Query for the export endpoints.
#[derive(serde::Deserialize)]
pub(super) struct ExportQuery {
    venue: String,
    /// One layer per first tag.
    layers_from_tags: Option<bool>,
    /// The archive's file name (bare, `.mvr`); default `<venue>.mvr`.
    file: Option<String>,
    /// Also write the archive under `lighting/export/`.
    keep: Option<bool>,
    venues_dir: Option<String>,
    fixture_types_dir: Option<String>,
}

#[allow(clippy::result_large_err)]
fn export_options(state: &WebUiState, query: &ExportQuery) -> Result<MvrExportOptions, Response> {
    validate_lighting_name(&query.venue)?;
    Ok(MvrExportOptions {
        output: query.file.clone().filter(|f| !f.trim().is_empty()),
        layers_from_tags: query.layers_from_tags.unwrap_or(false),
        fixture_types_dir: checked_dir(
            state,
            query.fixture_types_dir.as_deref(),
            DEFAULT_FIXTURE_TYPES_DIR,
        )?,
        venues_dir: checked_dir(state, query.venues_dir.as_deref(), DEFAULT_VENUES_DIR)?,
        ..MvrExportOptions::for_venue(&query.venue)
    })
}

/// GET /api/lighting/mvr/export/summary?venue=&layers_from_tags=&file= —
/// what an export would contain, computed without writing or sending the
/// archive: the export report (fixtures, positioned fixtures, focus points,
/// GDTFs embedded and generated, warnings, and every positioned fixed
/// fixture with the focus point it links to) plus `file_name`, the name the
/// download would carry. The dialog reads it before the user downloads.
pub(super) async fn export_summary(
    State(state): State<WebUiState>,
    Query(query): Query<ExportQuery>,
) -> impl IntoResponse {
    let options = export_options(&state, &query)?;
    let project = project_root(&state.config_path)?;
    let (_, report) = super::helpers::spawn_blocking_io("summarize MVR export", move || {
        Ok::<_, String>(
            lighting::export::export_mvr_bytes(&options, &project).map_err(|e| e.to_string()),
        )
    })
    .await?
    .map_err(bad_request)?;
    let file_name = report.output.clone();
    let mut body = serde_json::to_value(&report).unwrap_or_default();
    body["file_name"] = json!(file_name);
    Ok::<_, Response>((StatusCode::OK, Json(body)).into_response())
}

/// GET /api/lighting/mvr/export?venue=&layers_from_tags=&file=&keep= — the
/// archive itself, `application/octet-stream` with a `Content-Disposition`
/// attachment carrying the file name. `keep=true` also writes the archive
/// under `lighting/export/` (the CLI's location) and names it in
/// `X-Mtrack-Kept`; without it nothing is written.
pub(super) async fn export_mvr(
    State(state): State<WebUiState>,
    Query(query): Query<ExportQuery>,
) -> impl IntoResponse {
    let options = export_options(&state, &query)?;
    let project = project_root(&state.config_path)?;
    let keep = query.keep.unwrap_or(false);
    let (bytes, report, kept) = super::helpers::spawn_blocking_io("export MVR", move || {
        Ok::<_, String>((|| {
            let (bytes, report) = lighting::export::export_mvr_bytes(&options, &project)
                .map_err(|e| e.to_string())?;
            let kept = if keep {
                Some(
                    lighting::export::export_mvr(&options, &project)
                        .map_err(|e| e.to_string())?
                        .output,
                )
            } else {
                None
            };
            Ok::<_, String>((bytes, report, kept))
        })())
    })
    .await?
    .map_err(bad_request)?;
    // `report.output` is `<EXPORT_DIR>`-relative only once kept; the file
    // name is what the download carries either way.
    let file_name = report
        .output
        .rsplit('/')
        .next()
        .unwrap_or(&report.output)
        .to_string();
    debug_assert!(!EXPORT_DIR.is_empty());
    let mut response = (StatusCode::OK, bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/octet-stream"),
    );
    if let Ok(value) = HeaderValue::from_str(&content_disposition(&file_name)) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    if let Some(kept) = kept.and_then(|k| HeaderValue::from_str(&k).ok()) {
        headers.insert("x-mtrack-kept", kept);
    }
    Ok::<_, Response>(response)
}

/// `attachment; filename="..."` with an ASCII fallback and, when the name
/// has anything else in it, the RFC 5987 `filename*` form.
fn content_disposition(file_name: &str) -> String {
    let ascii: String = file_name
        .chars()
        .map(|c| {
            if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if ascii == file_name {
        return format!("attachment; filename=\"{ascii}\"");
    }
    let mut encoded = String::new();
    for byte in file_name.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            encoded.push(byte as char);
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

/// Query for the aim-point action.
#[derive(serde::Deserialize, Default)]
pub(super) struct AimQuery {
    venues_dir: Option<String>,
    fixture_types_dir: Option<String>,
}

/// POST /api/lighting/venues/{name}/aim-points — for each fixed fixture
/// (no pan or tilt) with a position that the export would not link to a
/// focus point (its rest beam passes more than a degree from every one),
/// adds a focus point `<fixture> aim` where the rest beam meets the deck
/// (z = 0), or 3 m along the beam when it does not meet the deck within
/// 50 m. Fixtures with no position are skipped. The points are added to the
/// venue file in place — positions, rotations, tags, provenance and comments
/// untouched — and the running engine reloads the venue if it is playing it.
/// Answers `{created: [{name, fixture, point}], reloaded}`.
pub(super) async fn add_aim_points(
    State(state): State<WebUiState>,
    Path(name): Path<String>,
    Query(query): Query<AimQuery>,
) -> impl IntoResponse {
    validate_lighting_name(&name)?;
    let venues_dir = checked_dir(&state, query.venues_dir.as_deref(), DEFAULT_VENUES_DIR)?;
    let fixture_types_dir = checked_dir(
        &state,
        query.fixture_types_dir.as_deref(),
        DEFAULT_FIXTURE_TYPES_DIR,
    )?;
    let project = project_root(&state.config_path)?;
    let venue_name = name.clone();
    let created = super::helpers::spawn_blocking_io("add aim points", move || {
        Ok::<_, String>(add_aim_points_blocking(
            &project,
            &venue_name,
            &venues_dir,
            &fixture_types_dir,
        ))
    })
    .await?
    .map_err(bad_request)?;
    let reloaded = if created.is_empty() {
        false
    } else {
        reload_if_current_venue(&state, &name).await
    };
    let created: Vec<serde_json::Value> = created
        .into_iter()
        .map(|(point_name, fixture, point)| {
            json!({"name": point_name, "fixture": fixture, "point": point})
        })
        .collect();
    Ok::<_, Response>(
        (
            StatusCode::OK,
            Json(json!({"created": created, "reloaded": reloaded})),
        )
            .into_response(),
    )
}

/// The points created: (focus point name, fixture, position).
type Created = Vec<(String, String, Vec3)>;

fn add_aim_points_blocking(
    project: &std::path::Path,
    venue_name: &str,
    venues_dir: &str,
    fixture_types_dir: &str,
) -> Result<Created, String> {
    let options = MvrExportOptions {
        fixture_types_dir: fixture_types_dir.to_string(),
        venues_dir: venues_dir.to_string(),
        ..MvrExportOptions::for_venue(venue_name)
    };
    // The export's own analysis says which fixtures are fixed and which are
    // already linked, so this can never disagree with the summary.
    let (_, report) =
        lighting::export::export_mvr_bytes(&options, project).map_err(|e| e.to_string())?;
    let unlinked: Vec<&str> = report
        .fixed_fixtures
        .iter()
        .filter(|f| f.focus.is_none())
        .map(|f| f.name.as_str())
        .collect();
    if unlinked.is_empty() {
        return Ok(Vec::new());
    }

    // The file that holds the venue, whatever it is called.
    let dir = project.join(venues_dir);
    let mut found: Option<(std::path::PathBuf, String, lighting::types::Venue)> = None;
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("cannot read {venues_dir}: {e}"))?;
    let mut paths: Vec<std::path::PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e == "venue" || e == "light")
        })
        .collect();
    paths.sort();
    for path in paths {
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Ok(mut venues) = lighting::parser::parse_venues(&content) {
            if let Some(venue) = venues.remove(venue_name) {
                found = Some((path, content, venue));
                break;
            }
        }
    }
    let (path, content, venue) =
        found.ok_or_else(|| format!("no venue file in {venues_dir} defines \"{venue_name}\""))?;

    let mut taken: std::collections::HashSet<String> =
        venue.focus_points().keys().cloned().collect();
    let mut created: Created = Vec::new();
    for fixture_name in unlinked {
        let Some(fixture) = venue.fixtures().get(fixture_name) else {
            continue;
        };
        let Some(point) = lighting::export::aim_point(fixture) else {
            continue;
        };
        let base = format!("{fixture_name} aim");
        let mut point_name = base.clone();
        let mut n = 1;
        while taken.contains(&point_name) {
            n += 1;
            point_name = format!("{base} {n}");
        }
        taken.insert(point_name.clone());
        created.push((point_name, fixture_name.to_string(), point));
    }
    if created.is_empty() {
        return Ok(created);
    }

    // One textual-edit path: the venue with its new points, patched into the
    // file (which checks the result reads back before anything is written).
    let mut focus_points = venue.focus_points().clone();
    for (point_name, _, point) in &created {
        focus_points.insert(point_name.clone(), *point);
    }
    let desired = venue.clone().with_focus_points(focus_points);
    let updated = lighting::venue_patch::patch_venue(&content, venue_name, &desired)?;
    config_io::staged_write(&path, &updated)?;
    Ok(created)
}

#[cfg(test)]
mod test {
    use super::super::router;
    use super::super::test_helpers::*;
    use super::*;
    use axum::body::Body;
    use tower::ServiceExt;

    const SYNTH_SCENE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<GeneralSceneDescription verMajor="1" verMinor="6"><Scene><Layers>
<Layer name="Front Truss"><ChildList>
<Fixture name="Brick 1"><Matrix>{1,0,0}{0,1,0}{0,0,1}{-2000,3500,4200}</Matrix>
<GDTFSpec>Astera_PB15.gdtf</GDTFSpec><GDTFMode>8: RGBS</GDTFMode>
<Addresses><Address break="0">1.1</Address></Addresses></Fixture>
<Fixture name="Brick 2"><Matrix>{1,0,0}{0,1,0}{0,0,1}{2000,3500,4200}</Matrix>
<GDTFSpec>Astera_PB15.gdtf</GDTFSpec><GDTFMode>8: RGBS</GDTFMode>
<Addresses><Address break="0">1.5</Address></Addresses></Fixture>
<Fixture name="Lost"><GDTFSpec>Missing.gdtf</GDTFSpec><GDTFMode>x</GDTFMode>
<Addresses><Address break="0">1.9</Address></Addresses></Fixture>
<SceneObject name="Main Stage"><Matrix>{1,0,0}{0,1,0}{0,0,1}{0,1000,0}</Matrix>
<Geometries><Geometry3D fileName="slab.glb"/></Geometries></SceneObject>
<FocusPoint name="Drummer"><Matrix>{1,0,0}{0,1,0}{0,0,1}{0,6300,1400}</Matrix></FocusPoint>
</ChildList></Layer></Layers></Scene></GeneralSceneDescription>"#;

    fn synthetic_mvr(with_deck: bool) -> Vec<u8> {
        let gdtf = crate::lighting::gdtf::build_zip(&[(
            "description.xml",
            crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.as_bytes(),
        )]);
        let glb = crate::lighting::import::scene_view::test_glb([-3.0, 0.0, -4.0], [3.0, 0.5, 0.0]);
        let scene = if with_deck {
            SYNTH_SCENE.to_string()
        } else {
            SYNTH_SCENE.replace("Main Stage", "Lamp")
        };
        crate::lighting::gdtf::build_zip(&[
            ("GeneralSceneDescription.xml", scene.as_bytes()),
            ("Astera_PB15.gdtf", gdtf.as_slice()),
            ("slab.glb", glb.as_slice()),
        ])
    }

    fn multipart(file_name: &str, bytes: &[u8], fields: &[(&str, &str)]) -> (String, Vec<u8>) {
        let boundary = "mtrack-test-boundary";
        let mut body = Vec::new();
        for (key, value) in fields {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{key}\"\r\n\r\n{value}\r\n"
                )
                .as_bytes(),
            );
        }
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(
            format!("Content-Disposition: form-data; name=\"file\"; filename=\"{file_name}\"\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(b"Content-Type: application/octet-stream\r\n\r\n");
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        (format!("multipart/form-data; boundary={boundary}"), body)
    }

    async fn post(
        app: &axum::Router,
        uri: &str,
        file_name: &str,
        bytes: &[u8],
        fields: &[(&str, &str)],
    ) -> (StatusCode, serde_json::Value) {
        let (content_type, body) = multipart(file_name, bytes, fields);
        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri(uri)
                    .header("content-type", content_type)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let text = response_body(response).await;
        (status, serde_json::from_str(&text).unwrap_or_default())
    }

    async fn get(app: &axum::Router, uri: &str) -> axum::response::Response {
        app.clone()
            .oneshot(
                http::Request::builder()
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn inspect_reports_positions_and_deck_bounds_and_writes_nothing() {
        let (state, dir) = test_state();
        let app = router().with_state(state);
        let (status, body) = post(
            &app,
            "/lighting/mvr/inspect",
            "Kellys.mvr",
            &synthetic_mvr(true),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
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
            !dir.path().join("lighting").exists(),
            "inspect must not write"
        );

        // No object that says it is a deck: null, and the wizard falls back.
        let (status, body) = post(
            &app,
            "/lighting/mvr/inspect",
            "Kellys.mvr",
            &synthetic_mvr(false),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body["scene"]["deck_mm"].is_null(), "{body}");
        assert_eq!(body["scene"]["scenery"][0]["deck"], false);
    }

    #[tokio::test]
    async fn inspect_refuses_what_is_not_an_mvr_with_the_reason() {
        let (state, _dir) = test_state();
        let app = router().with_state(state);
        let (status, body) =
            post(&app, "/lighting/mvr/inspect", "junk.mvr", b"not a zip", &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .contains("not a readable MVR archive"),
            "{body}"
        );
        let (status, body) = post(
            &app,
            "/lighting/mvr/inspect",
            "rig.zip",
            &synthetic_mvr(true),
            &[],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains(".mvr"), "{body}");
        // A directory that escapes the project is refused, not followed.
        let (status, _) = post(
            &app,
            "/lighting/mvr/inspect",
            "rig.mvr",
            &synthetic_mvr(true),
            &[("venues_dir", "../elsewhere")],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn import_plans_without_writing_and_then_writes() {
        let (state, dir) = test_state();
        let app = router().with_state(state);
        let mvr = synthetic_mvr(true);
        let fields = [
            ("name", "kellys"),
            ("origin", "0,1000,0"),
            ("write", "false"),
        ];
        let (status, body) = post(&app, "/lighting/mvr/import", "Kellys.mvr", &mvr, &fields).await;
        assert_eq!(status, StatusCode::OK, "{body}");
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
            !dir.path().join("lighting").exists(),
            "a plan must not write"
        );

        let fields = [
            ("name", "kellys"),
            ("origin", "0,1000,0"),
            ("write", "true"),
        ];
        let (status, body) = post(&app, "/lighting/mvr/import", "Kellys.mvr", &mvr, &fields).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["write"], true);
        assert_eq!(body["report"]["venue_name"], "kellys");
        assert!(dir.path().join("lighting/venues/kellys.venue").exists());
        assert!(dir.path().join("lighting/library/Kellys.mvr").exists());
        let venue =
            std::fs::read_to_string(dir.path().join("lighting/venues/kellys.venue")).unwrap();
        assert!(venue.contains("# TODO fixture \"Lost\""), "{venue}");

        // The same again is a merge.
        let (_, body) = post(&app, "/lighting/mvr/import", "Kellys.mvr", &mvr, &fields).await;
        assert_eq!(body["report"]["merge"], true, "{body}");
    }

    #[tokio::test]
    async fn a_merge_plan_lists_hand_edits_and_the_keep_list_keeps_them() {
        let (state, dir) = test_state();
        let app = router().with_state(state);
        let mvr = synthetic_mvr(true);
        let write = [
            ("name", "kellys"),
            ("origin", "0,1000,0"),
            ("write", "true"),
        ];
        post(&app, "/lighting/mvr/import", "Kellys.mvr", &mvr, &write).await;

        // Hand-move Brick 1 (the MVR puts it at x = -2).
        let path = dir.path().join("lighting/venues/kellys.venue");
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

        let plan = [
            ("name", "kellys"),
            ("origin", "0,1000,0"),
            ("write", "false"),
        ];
        let (status, body) = post(&app, "/lighting/mvr/import", "Kellys.mvr", &mvr, &plan).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let brick1 = body["plan"]["fixtures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|f| f["name"] == "Brick 1")
            .unwrap();
        assert_eq!(brick1["overwrites"][0]["field"], "position", "{brick1}");
        assert_eq!(brick1["overwrites"][0]["mine"], "(-1, 2.5, 4.2)");

        let keep = r#"{"fixtures":{"Brick 1":["position"]}}"#;
        let fields = [
            ("name", "kellys"),
            ("origin", "0,1000,0"),
            ("write", "true"),
            ("keep", keep),
        ];
        let (status, body) = post(&app, "/lighting/mvr/import", "Kellys.mvr", &mvr, &fields).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let after = std::fs::read_to_string(&path).unwrap();
        assert!(after.starts_with("# mine\n"), "{after}");
        assert!(after.contains("position (-1, 2.5, 4.2)"), "{after}");

        // A keep list that is not JSON is refused before anything is written.
        let bad = [("write", "true"), ("keep", "nope")];
        let (status, body) = post(&app, "/lighting/mvr/import", "Kellys.mvr", &mvr, &bad).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("keep"), "{body}");
    }

    #[tokio::test]
    async fn import_needs_an_explicit_write_flag_and_a_sane_origin() {
        let (state, dir) = test_state();
        let app = router().with_state(state);
        let mvr = synthetic_mvr(true);
        let (status, _) = post(&app, "/lighting/mvr/import", "K.mvr", &mvr, &[]).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, body) = post(
            &app,
            "/lighting/mvr/import",
            "K.mvr",
            &mvr,
            &[("write", "true"), ("origin", "1,2")],
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().unwrap().contains("origin"), "{body}");
        assert!(!dir.path().join("lighting").exists());
    }

    /// Imports the synthetic MVR as `kellys` with the origin at the deck's
    /// front edge, so its bricks hang 4.2 m up over the deck.
    async fn imported(app: &axum::Router) {
        let (status, body) = post(
            app,
            "/lighting/mvr/import",
            "Kellys.mvr",
            &synthetic_mvr(true),
            &[
                ("name", "kellys"),
                ("origin", "0,1000,0"),
                ("write", "true"),
            ],
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    #[tokio::test]
    async fn export_summary_says_what_is_linked_and_download_sets_its_headers() {
        let (state, dir) = test_state();
        let app = router().with_state(state);
        imported(&app).await;

        let response = get(&app, "/lighting/mvr/export/summary?venue=kellys").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
        assert_eq!(body["file_name"], "kellys.mvr");
        // "Lost" is a TODO comment, not a fixture of the venue.
        assert_eq!(body["fixtures"], 2);
        assert_eq!(body["positioned_fixtures"], 2);
        assert_eq!(body["focus_points"], 1);
        assert_eq!(body["embedded_gdtfs"].as_array().unwrap().len(), 1);
        let fixed = body["fixed_fixtures"].as_array().unwrap();
        assert_eq!(fixed.len(), 2);
        assert!(fixed.iter().all(|f| f["focus"].is_null()), "{body}");
        assert!(
            !dir.path().join("lighting/export").exists(),
            "a summary must not write"
        );

        let response = get(
            &app,
            "/lighting/mvr/export?venue=kellys&layers_from_tags=true",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["content-type"],
            "application/octet-stream"
        );
        assert_eq!(
            response.headers()["content-disposition"],
            "attachment; filename=\"kellys.mvr\""
        );
        assert!(response.headers().get("x-mtrack-kept").is_none());
        use http_body_util::BodyExt;
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        assert!(
            crate::lighting::mvr::parse_archive(&bytes).is_ok(),
            "the download is an MVR"
        );
        assert!(
            !dir.path().join("lighting/export").exists(),
            "nothing is written unless asked"
        );

        // A named file, and keeping a copy in the project.
        let response = get(
            &app,
            "/lighting/mvr/export?venue=kellys&file=tour.mvr&keep=true",
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers()["content-disposition"],
            "attachment; filename=\"tour.mvr\""
        );
        assert_eq!(
            response.headers()["x-mtrack-kept"],
            "lighting/export/tour.mvr"
        );
        assert!(dir.path().join("lighting/export/tour.mvr").exists());

        // A name that is not a bare .mvr, and a venue that is not there.
        let response = get(&app, "/lighting/mvr/export?venue=kellys&file=..%2Fx.mvr").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let response = get(&app, "/lighting/mvr/export?venue=nope").await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn aim_points_hit_the_deck_fall_back_to_three_meters_and_skip_the_linked() {
        let (state, dir) = test_state();
        let app = router().with_state(state);
        imported(&app).await;
        let venue_path = dir.path().join("lighting/venues/kellys.venue");
        // Brick 1 hangs at (-2, 2.5, 4.2) straight down: the deck is at
        // its feet. Tilt Brick 2 to point level (rotation about Y): its
        // beam never meets the deck. Both are fixed fixtures.
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

        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/lighting/venues/kellys/aim-points")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
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

        // Comments and provenance are still in the file; the points read
        // back and the export now links both bricks.
        let after = std::fs::read_to_string(&venue_path).unwrap();
        assert!(after.contains("# TODO fixture \"Lost\""), "{after}");
        assert!(after.contains("imported from mvr("), "{after}");
        assert!(after.contains("focus \"Brick 1 aim\""), "{after}");
        let response = get(&app, "/lighting/mvr/export/summary?venue=kellys").await;
        let summary: serde_json::Value =
            serde_json::from_str(&response_body(response).await).unwrap();
        assert!(
            summary["fixed_fixtures"]
                .as_array()
                .unwrap()
                .iter()
                .all(|f| f["focus"].is_string()),
            "{summary}"
        );

        // Nothing left to link: a second call creates nothing and leaves
        // the file alone.
        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method("POST")
                    .uri("/lighting/venues/kellys/aim-points")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
        assert!(body["created"].as_array().unwrap().is_empty(), "{body}");
        assert_eq!(std::fs::read_to_string(&venue_path).unwrap(), after);
    }

    #[tokio::test]
    async fn tagging_an_imported_venue_through_the_venue_save_keeps_its_todo_lines() {
        let (state, dir) = test_state();
        let app = router().with_state(state);
        imported(&app).await;
        let path = dir.path().join("lighting/venues/kellys.venue");
        let before = std::fs::read_to_string(&path).unwrap();
        assert!(before.contains("# TODO fixture \"Lost\""), "{before}");

        // What the UI does: read the venue, change a tag, PUT it back as JSON.
        let response = get(&app, "/lighting/venues/kellys").await;
        let got: serde_json::Value = serde_json::from_str(&response_body(response).await).unwrap();
        let mut venue = got["venue"].clone();
        let fixtures: Vec<serde_json::Value> = venue["fixtures"]
            .as_object()
            .unwrap()
            .values()
            .cloned()
            .map(|mut f| {
                if f["name"] == "Brick 1" {
                    f["tags"] = json!(["wash"]);
                }
                f
            })
            .collect();
        venue["fixtures"] = json!(fixtures);
        let response = app
            .clone()
            .oneshot(
                http::Request::builder()
                    .method("PUT")
                    .uri("/lighting/venues/kellys")
                    .header("content-type", "application/json")
                    .body(Body::from(venue.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);

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

    #[test]
    fn an_aim_point_is_where_the_beam_meets_the_deck_or_three_meters_along_it() {
        use lighting::types::Fixture;
        let hung = |position: Vec3, rotation: Vec3| {
            Fixture::new("F".into(), "T".into(), 1, 1, vec![])
                .with_position(Some(position))
                .with_rotation(Some(rotation))
        };
        // Straight down.
        assert_eq!(
            lighting::export::aim_point(&hung([1.0, 2.0, 5.0], [0.0; 3])),
            Some([1.0, 2.0, 0.0])
        );
        // Tilted 45 degrees about X from 4 m up: lands 4 m along y... the
        // mounting's -Z tips toward +y for a negative rotation about X.
        let p = lighting::export::aim_point(&hung([0.0, 0.0, 4.0], [-45.0, 0.0, 0.0])).unwrap();
        assert!(
            (p[2]).abs() < 1e-9 && (p[1].abs() - 4.0).abs() < 1e-3,
            "{p:?}"
        );
        // Pointing up, or mounted on the deck: 3 m along the beam.
        let up = lighting::export::aim_point(&hung([0.0, 0.0, 2.0], [180.0, 0.0, 0.0])).unwrap();
        assert!((up[2] - 5.0).abs() < 1e-9, "{up:?}");
        let low = lighting::export::aim_point(&hung([0.0, 0.0, 0.0], [0.0; 3])).unwrap();
        assert_eq!(low, [0.0, 0.0, -3.0]);
        // Too shallow to meet the deck within 50 m.
        let far = lighting::export::aim_point(&hung([0.0, 0.0, 4.0], [89.0, 0.0, 0.0])).unwrap();
        assert!(far[2] > 0.9, "{far:?}");
        // No position, no point.
        let unplaced = Fixture::new("F".into(), "T".into(), 1, 1, vec![]);
        assert_eq!(lighting::export::aim_point(&unplaced), None);
    }

    #[test]
    fn content_disposition_is_safe_for_awkward_names() {
        assert_eq!(
            content_disposition("kellys.mvr"),
            "attachment; filename=\"kellys.mvr\""
        );
        let awkward = content_disposition("Bühne \"1\".mvr");
        assert!(awkward.contains("filename=\"B_hne _1_.mvr\""), "{awkward}");
        assert!(
            awkward.contains("filename*=UTF-8''B%C3%BChne%20%221%22.mvr"),
            "{awkward}"
        );
    }
}
