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

//! MVR import and export (lighting UI design §11): the browser's import
//! wizard and export dialog. Every function is a thin wrapper over the
//! library functions behind the CLI and the MCP tools
//! (`import::inspect_mvr_bytes`, `import::import_mvr_bytes`,
//! `export::export_mvr_bytes`), so the surfaces cannot disagree. The gRPC
//! services call them with a file already uploaded (`transfers`).
//!
//! The server holds nothing between calls: the wizard uploads the file at
//! each step. Every write goes through a directory proven to be inside the
//! project (the same containment the GDTF import and the venue save use).

use crate::webui::wire::mvr::{
    AimPoint, AimPoints, MvrExportSummary, MvrImportAnswer, MvrInspection, MvrKept,
};

use axum::http::StatusCode;
use serde_json::json;

use super::lighting_api::{
    archive_file_name, content_version, lighting_dir, project_root, reload_if_current_venue,
    resolve_lighting_dir, stale_venue_response, validate_lighting_name, venue_error_of,
    LightingDir, VENUE_WRITES,
};
use super::lighting_writes::Answer;
use crate::lighting;
use crate::lighting::export::MvrExportOptions;
use crate::lighting::import::MvrImportOptions;
use crate::lighting::types::Vec3;

/// What an upload of an MVR announces: the name the archive is kept under
/// (see [`archive_file_name`]), which must be an `.mvr`.
pub(crate) fn check_mvr_upload(file_name: &str) -> Result<String, Answer> {
    let Some(name) = archive_file_name(file_name) else {
        return Err(Answer::bad("No file uploaded"));
    };
    if !name.to_ascii_lowercase().ends_with(".mvr") {
        return Err(Answer::bad(format!(
            "{name} is not an .mvr file; choose the venue's MVR export"
        )));
    }
    Ok(name)
}

/// The point, in millimeters, an import takes as the stage's origin.
pub(crate) fn origin_mm(x: f64, y: f64, z: f64) -> Result<Vec3, Answer> {
    if [x, y, z].iter().all(|v| v.is_finite()) {
        Ok([x, y, z])
    } else {
        Err(Answer::bad(
            "origin must be three numbers in millimeters, \"x,y,z\"",
        ))
    }
}

/// A directory the caller named, or this host's, proven to stay in the
/// project. Returns the project-relative string the library functions take.
fn checked_dir(
    config_path: &std::path::Path,
    requested: Option<&str>,
    which: LightingDir,
) -> Result<String, Answer> {
    // The config read once: the directory, then proven to stay in the
    // project as named.
    let dir = lighting_dir(config_path, requested, which)?;
    resolve_lighting_dir(config_path, Some(&dir), which)?;
    Ok(dir)
}

/// What an inspection or an import is asked for. A text left blank is
/// not given.
#[derive(Default)]
pub(crate) struct MvrAsk {
    pub name: Option<String>,
    pub origin_mm: Option<Vec3>,
    pub venues_dir: Option<String>,
    pub fixture_types_dir: Option<String>,
    /// The hand edits a merge keeps.
    pub keep: lighting::mvr_types::MvrKeep,
}

/// A text that is given: trimmed, and not empty.
fn given(text: Option<&str>) -> Option<&str> {
    text.map(str::trim).filter(|t| !t.is_empty())
}

/// The import options an ask has, directories checked.
fn import_options(config_path: &std::path::Path, ask: &MvrAsk) -> Result<MvrImportOptions, Answer> {
    Ok(MvrImportOptions {
        name: given(ask.name.as_deref()).map(str::to_string),
        origin_mm: ask.origin_mm,
        fixture_types_dir: checked_dir(
            config_path,
            given(ask.fixture_types_dir.as_deref()),
            LightingDir::FixtureTypes,
        )?,
        venues_dir: checked_dir(
            config_path,
            given(ask.venues_dir.as_deref()),
            LightingDir::Venues,
        )?,
        keep: ask.keep.clone(),
    })
}

/// Reads the staged MVR `file`: what an import would do (`report`, the MCP
/// `inspect_mvr` plan) and the file seen from above (`scene`: every
/// fixture's and focus point's position in the file's millimeters, the
/// scenery and, when the scenery carries a stage floor, its bounds,
/// `deck_mm`, else null). Writes nothing. A file the importer refuses (not
/// an MVR, over the caps) is a 400 with the parser's reason; a venue name
/// (or an embedded GDTF's record) the one rule for lighting files refuses is
/// a 409 with the reason, as the import answers it. The staged file is
/// deleted whether this succeeds or not.
pub(crate) async fn inspect_mvr(
    config_path: &std::path::Path,
    file: crate::webui::transfers::Claimed,
    ask: &MvrAsk,
) -> Result<MvrInspection, Answer> {
    check_mvr_upload(&file.file_name)?;
    let options = import_options(config_path, ask)?;
    let project = project_root(config_path)?;
    let file_name = file.file_name.clone();
    let outcome = super::helpers::blocking_answer("inspect MVR", move || {
        let bytes = file.read().map_err(|e| e.to_string())?;
        Ok::<_, String>((|| {
            let report =
                lighting::import::inspect_mvr_bytes(&bytes, &file.file_name, &options, &project)
                    .map_err(|e| super::lighting_writes::import_error(e.as_ref()))?;
            let scene = lighting::import::scene_view::scene_view(&bytes)
                .map_err(|e| (StatusCode::BAD_REQUEST, json!({"error": e.to_string()})))?;
            Ok::<_, (StatusCode, serde_json::Value)>((report, scene))
        })())
    })
    .await?
    .map_err(|(status, body)| Answer { status, body })?;
    let (report, scene) = outcome;
    Ok(MvrInspection {
        file_name,
        report,
        scene,
    })
}

/// Imports the staged MVR `file`, or, with `write` false, plans the import
/// (what `import-mvr` prints without `--write`). `write` true performs it
/// and answers the report, `reloaded` saying whether the running engine
/// re-read the venue it is playing. Importing never makes the venue
/// current. A venue whose file is not its alone, or whose new file is
/// taken, is a 409 (the venue save's rule); anything else refused is a 400.
/// The staged file is deleted whether this succeeds or not.
pub(crate) async fn import_mvr(
    player: &std::sync::Arc<crate::player::Player>,
    config_path: &std::path::Path,
    file: crate::webui::transfers::Claimed,
    ask: &MvrAsk,
    write: bool,
) -> Result<MvrImportAnswer, Answer> {
    check_mvr_upload(&file.file_name)?;
    let options = import_options(config_path, ask)?;
    let project = project_root(config_path)?;
    let venues_path = project.join(&options.venues_dir);
    if write {
        // Directories the import writes into are created, and proven to be
        // inside the project, before anything is written.
        let project_dir = crate::util::project_dir_of(config_path);
        for dir in [
            options.fixture_types_dir.as_str(),
            options.venues_dir.as_str(),
            "lighting/library",
        ] {
            if let Err(response) =
                super::helpers::ensure_dir_within(&project.join(dir), &project_dir).await
            {
                return Err(Answer::of_response(response).await);
            }
        }
    }
    let outcome = super::helpers::blocking_answer("import MVR", move || {
        let bytes = file.read().map_err(|e| e.to_string())?;
        // A re-import merges into the venue file it reads: held from that
        // read to its write, or a venue save between the two is lost (#578).
        let _guard = write.then(|| VENUE_WRITES.blocking_lock());
        Ok::<_, String>(
            if write {
                lighting::import::import_mvr_bytes(&bytes, &file.file_name, &options, &project)
                    .map(Imported::Report)
            } else {
                lighting::import::inspect_mvr_bytes(&bytes, &file.file_name, &options, &project)
                    .map(Imported::Plan)
            }
            .map_err(|e| super::lighting_writes::import_error(e.as_ref())),
        )
    })
    .await?
    .map_err(|(status, body)| Answer { status, body })?;
    let report = match outcome {
        Imported::Plan(plan) => {
            return Ok(MvrImportAnswer {
                write: false,
                plan: Some(plan),
                report: None,
                reloaded: None,
                venue_error: None,
            });
        }
        Imported::Report(report) => report,
    };
    let venue = report.plan.venue_name.clone();
    let reloaded = reload_if_current_venue(player, &venue).await;
    let venue_error = venue_error_of(player, config_path, &venue, &venues_path).await;
    Ok(MvrImportAnswer {
        write: true,
        plan: None,
        report: Some(report),
        reloaded: Some(reloaded),
        venue_error: Some(venue_error),
    })
}

/// An import's outcome: the plan, or what the import did.
enum Imported {
    Plan(lighting::import::MvrPlan),
    Report(lighting::import::MvrImport),
}

/// What an export is asked for.
#[derive(Default)]
pub(crate) struct ExportAsk {
    pub venue: String,
    /// One layer per first tag.
    pub layers_from_tags: bool,
    /// The archive's file name (bare, `.mvr`); default `<venue>.mvr`.
    pub file: Option<String>,
    pub venues_dir: Option<String>,
    pub fixture_types_dir: Option<String>,
}

fn export_options(
    config_path: &std::path::Path,
    ask: &ExportAsk,
) -> Result<MvrExportOptions, Answer> {
    validate_lighting_name(&ask.venue)?;
    Ok(MvrExportOptions {
        output: ask.file.clone().filter(|f| !f.trim().is_empty()),
        layers_from_tags: ask.layers_from_tags,
        fixture_types_dir: checked_dir(
            config_path,
            ask.fixture_types_dir.as_deref(),
            LightingDir::FixtureTypes,
        )?,
        venues_dir: checked_dir(config_path, ask.venues_dir.as_deref(), LightingDir::Venues)?,
        ..MvrExportOptions::for_venue(&ask.venue)
    })
}

/// What an export would contain, computed without writing or sending the
/// archive: the export report (fixtures, positioned fixtures, focus points,
/// GDTFs embedded and generated, warnings, and every positioned fixed
/// fixture with the focus point it links to) plus `file_name`, the name the
/// download would carry. The dialog reads it before the user downloads.
pub(crate) async fn export_summary(
    config_path: &std::path::Path,
    ask: &ExportAsk,
) -> Result<MvrExportSummary, Answer> {
    let options = export_options(config_path, ask)?;
    let project = project_root(config_path)?;
    let (_, report) = super::helpers::blocking_answer("summarize MVR export", move || {
        Ok::<_, String>(
            lighting::export::export_mvr_bytes(&options, &project).map_err(|e| e.to_string()),
        )
    })
    .await?
    .map_err(Answer::bad)?;
    Ok(MvrExportSummary {
        file_name: report.output.clone(),
        export: report,
    })
}

/// Builds the export archive: its bytes and the name it carries, the
/// report's output without its directories. Nothing is written; keeping a
/// copy in the project is [`keep_export`]. A name that is not a bare `.mvr`
/// and a venue that is not there are a 400.
pub(crate) async fn build_export(
    config_path: &std::path::Path,
    ask: &ExportAsk,
) -> Result<(Vec<u8>, String), Answer> {
    let options = export_options(config_path, ask)?;
    let project = project_root(config_path)?;
    let (bytes, report) = super::helpers::blocking_answer("export MVR", move || {
        Ok::<_, String>(
            lighting::export::export_mvr_bytes(&options, &project).map_err(|e| e.to_string()),
        )
    })
    .await?
    .map_err(Answer::bad)?;
    let file_name = report
        .output
        .rsplit('/')
        .next()
        .unwrap_or(&report.output)
        .to_string();
    Ok((bytes, file_name))
}

/// Writes the archive under `lighting/export/` (the CLI's location) and
/// names it. An existing file is left alone, and named in
/// [`Kept::Exists`], unless `overwrite`. It writes, so the lock guards it.
pub(crate) async fn keep_export(
    config_path: &std::path::Path,
    ask: &ExportAsk,
    overwrite: bool,
) -> Result<Kept, Answer> {
    let options = export_options(config_path, ask)?;
    let project = project_root(config_path)?;
    // Built once: the bytes the download would carry are the bytes written.
    let result = super::helpers::blocking_answer("keep MVR export", move || {
        Ok::<_, String>(
            (|| {
                let (bytes, report) = lighting::export::export_mvr_bytes(&options, &project)?;
                lighting::export::write_export(&options, &project, &bytes, report, overwrite)
            })()
            .map_err(|e| match e.downcast::<lighting::export::ExportExists>() {
                Ok(exists) => KeepError::Exists(exists.0),
                Err(other) => KeepError::Failed(other.to_string()),
            }),
        )
    })
    .await?;
    match result {
        Ok(report) => Ok(Kept::Kept(MvrKept {
            kept: report.output,
        })),
        Err(KeepError::Exists(existing)) => Ok(Kept::Exists(existing)),
        Err(KeepError::Failed(message)) => Err(Answer::bad(message)),
    }
}

/// How a keep ended.
pub(crate) enum Kept {
    /// Written.
    Kept(MvrKept),
    /// A file of that name was already there and was left alone; its name.
    Exists(String),
}

/// Why a kept export was not written.
enum KeepError {
    Exists(String),
    Failed(String),
}

/// Adds the venue's aim points: for each fixed fixture (no pan or tilt)
/// with a position that the export would not link to a focus point (its rest
/// beam passes more than a degree from every one), a focus point
/// `<fixture> aim` where the rest beam meets the deck (z = 0), or 3 m along
/// the beam when it does not meet the deck within 50 m. Fixtures with no
/// position are skipped. The points are added to the venue file in place —
/// positions, rotations, tags, provenance and comments untouched — and the
/// running engine reloads the venue if it is playing it. An `expected`
/// version, when given, must be the venue file's current one (409,
/// `conflict`, otherwise).
pub(crate) async fn add_aim_points(
    player: &std::sync::Arc<crate::player::Player>,
    config_path: &std::path::Path,
    name: &str,
    venues_dir: Option<&str>,
    fixture_types_dir: Option<&str>,
    expected: Option<String>,
) -> Result<AimPoints, Answer> {
    validate_lighting_name(name)?;
    let venues_dir = checked_dir(config_path, venues_dir, LightingDir::Venues)?;
    let fixture_types_dir = checked_dir(config_path, fixture_types_dir, LightingDir::FixtureTypes)?;
    let project = project_root(config_path)?;
    let venues_path = project.join(&venues_dir);
    let venue_name = name.to_string();
    let outcome = super::helpers::blocking_answer("add aim points", move || {
        Ok::<_, String>(add_aim_points_blocking(
            &project,
            &venue_name,
            &venues_dir,
            &fixture_types_dir,
            expected.as_deref(),
        ))
    })
    .await?;
    let (created, version) = match outcome {
        Ok(done) => done,
        Err(AimError::Stale(current)) => return Err(stale_venue_response(name, Some(current))),
        Err(AimError::Message(e)) => return Err(Answer::bad(e)),
        Err(AimError::Refused(r)) => return Err(r),
    };
    let reloaded = if created.is_empty() {
        false
    } else {
        reload_if_current_venue(player, name).await
    };
    let venue_error = venue_error_of(player, config_path, name, &venues_path).await;
    let created = created
        .into_iter()
        .map(|(name, fixture, point)| AimPoint {
            name,
            fixture,
            point,
        })
        .collect();
    Ok(AimPoints {
        created,
        reloaded,
        version,
        venue_error,
    })
}

/// The points created: (focus point name, fixture, position).
type Created = Vec<(String, String, Vec3)>;

/// Why the aim points were not added.
enum AimError {
    Message(String),
    /// The file's version is not the one the client sent; carries the
    /// current one.
    Stale(String),
    /// The venue's file may not be written (it is not the venue's alone),
    /// or its upgrade to a `.venue` is refused: the save's own answer.
    Refused(super::lighting_writes::Answer),
}

impl From<String> for AimError {
    fn from(e: String) -> Self {
        AimError::Message(e)
    }
}

/// Adds the points; returns them with the venue file's version afterwards.
#[allow(clippy::result_large_err)]
fn add_aim_points_blocking(
    project: &std::path::Path,
    venue_name: &str,
    venues_dir: &str,
    fixture_types_dir: &str,
    expected_version: Option<&str>,
) -> Result<(Created, String), AimError> {
    let options = MvrExportOptions {
        fixture_types_dir: fixture_types_dir.to_string(),
        venues_dir: venues_dir.to_string(),
        ..MvrExportOptions::for_venue(venue_name)
    };
    // Held from the analysis below, which reads the venue: a save linking a
    // fixture between it and the write would get an aim point it no longer
    // needs.
    let _guard = VENUE_WRITES.blocking_lock();
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

    // The venue's file, by the one rule for venue writes: the file the
    // loader read it from, written only when it is the venue's alone, and a
    // `.light` moved to a `.venue` (focus points need the newer format).
    let dir = project.join(venues_dir);
    if !dir.is_dir() {
        return Err(format!("cannot read {venues_dir}: not a directory").into());
    }
    let read = lighting::project_files::venues(&dir);
    let venue = read
        .get(venue_name)
        .map(|declared| declared.item.clone())
        .ok_or_else(|| format!("no venue file in {venues_dir} defines \"{venue_name}\""))?;
    let save = super::lighting_writes::Save {
        kind: lighting::files::Kind::Venue,
        dir: &dir,
        record: &read.record,
        library: &[],
        name: venue_name,
        from: None,
        needs_new_format: true,
    };
    let plan = super::lighting_writes::plan(&save).map_err(AimError::Refused)?;
    let content = plan
        .current
        .as_deref()
        .map(|bytes| String::from_utf8_lossy(bytes).into_owned())
        .ok_or_else(|| format!("no venue file in {venues_dir} defines \"{venue_name}\""))?;

    let current = content_version(content.as_bytes());
    if expected_version.is_some_and(|expected| expected != current) {
        return Err(AimError::Stale(current));
    }
    if unlinked.is_empty() {
        return Ok((Vec::new(), current));
    }

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
        return Ok((created, current));
    }

    // One textual-edit path: the venue with its new points, patched into the
    // file (which checks the result reads back before anything is written).
    let mut focus_points = venue.focus_points().clone();
    for (point_name, _, point) in &created {
        focus_points.insert(point_name.clone(), *point);
    }
    let desired = venue.clone().with_focus_points(focus_points);
    let updated = lighting::venue_patch::patch_venue(&content, venue_name, &desired)?;
    super::lighting_writes::write(&plan, &updated).map_err(AimError::Refused)?;
    Ok((created, content_version(updated.as_bytes())))
}

#[cfg(test)]
pub(crate) mod test {
    use super::*;

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

    pub(crate) fn synthetic_mvr(with_deck: bool) -> Vec<u8> {
        let gdtf = crate::testutil::lighting::synthetic_gdtf();
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

    /// The synthetic MVR imported as `kellys` with the origin at the deck's
    /// front edge, so its bricks hang 4.2 m up over the deck.
    pub(crate) async fn import_kellys(state: super::super::super::server::WebUiState) {
        let transfers = state.player.transfers();
        let staged = transfers.stage_for_test(
            crate::webui::transfers::Purpose::MvrImport,
            "Kellys.mvr",
            &synthetic_mvr(true),
        );
        let ask = MvrAsk {
            name: Some("kellys".to_string()),
            origin_mm: Some([0.0, 1000.0, 0.0]),
            ..Default::default()
        };
        import_mvr(&state.player, &state.config_path, staged, &ask, true)
            .await
            .unwrap_or_else(|e| panic!("{}", e.message()));
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
}
