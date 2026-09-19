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

//! MVR export (venue-exchange design §16.4): a venue mtrack owns, handed
//! to a console or a pre-viz tool as a `.mvr` — the patch with positions,
//! rotations and focus points, and every fixture type's GDTF embedded.
//!
//! A venue that was imported round-trips: the stage origin it recorded is
//! reapplied, so a re-import of the export merges into the same venue
//! with nothing changed. A venue written by hand exports the first time.
//! A native fixture type (a `.light` or hand-written `.fixture`) has no
//! GDTF to embed, so one is generated — one mode, the channel definitions,
//! no models — enough for the console to patch it (see [`gdtf`]).
//!
//! Everything is resolved before anything is written; a refused export
//! leaves the project untouched.

pub mod gdtf;

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::Serialize;
use sha2::{Digest, Sha256};

use super::import::fixture_filename_stem;
use super::parser::{parse_fixture_types, parse_venues};
use super::types::{Fixture, FixtureType, Vec3, Venue};

/// How an export is run.
#[derive(Clone, Debug)]
pub struct MvrExportOptions {
    /// The venue to export, by name.
    pub venue: String,
    /// The `.mvr` file name, under the project's `lighting/export/`
    /// directory — a bare name, `.mvr` extension, no directories, so an
    /// export can only ever land there. Defaults to `<venue stem>.mvr`.
    pub output: Option<String>,
    /// Fixture types directory, relative to the project.
    pub fixture_types_dir: String,
    /// Venues directory, relative to the project.
    pub venues_dir: String,
    /// Put each fixture on an MVR layer named after its first tag (untagged
    /// fixtures on a layer named after the venue). Off, one layer holds
    /// everything.
    pub layers_from_tags: bool,
}

impl MvrExportOptions {
    /// Options for a venue with the default directories.
    pub fn for_venue(venue: &str) -> MvrExportOptions {
        MvrExportOptions {
            venue: venue.to_string(),
            output: None,
            fixture_types_dir: "lighting/fixture_types".to_string(),
            venues_dir: "lighting/venues".to_string(),
            layers_from_tags: false,
        }
    }
}

/// What an export produced, for reporting.
#[derive(Debug, Serialize)]
pub struct MvrExport {
    /// The venue's name.
    pub venue: String,
    /// The `.mvr` written, project-relative.
    pub output: String,
    /// Fixtures in the scene.
    pub fixtures: usize,
    /// Focus points in the scene.
    pub focus_points: usize,
    /// GDTF archives embedded from the library, by entry name.
    pub embedded_gdtfs: Vec<String>,
    /// GDTF archives generated for native fixture types: entry name →
    /// fixture type.
    pub generated_gdtfs: BTreeMap<String, String>,
    /// What was skipped or approximated.
    pub warnings: Vec<String>,
}

/// Exports a venue to a `.mvr` in the project. All validation runs before
/// the write.
pub fn export_mvr(options: &MvrExportOptions, project: &Path) -> Result<MvrExport, Box<dyn Error>> {
    let (bytes, mut report) = export_mvr_bytes(options, project)?;
    let output = output_path(options)?;
    // The export directory is the one place an export writes. It is
    // created if missing and then proven to be inside the project (a
    // symlink planted there could point anywhere; the canonical path says).
    let export_dir = project.join(EXPORT_DIR);
    super::import::create_dir(&export_dir)?;
    let canonical_project = project.canonicalize().map_err(|e| {
        format!(
            "cannot resolve project directory {}: {e}",
            project.display()
        )
    })?;
    let canonical_dir = export_dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve {}: {e}", export_dir.display()))?;
    if !canonical_dir.starts_with(&canonical_project) {
        return Err(format!(
            "{} resolves outside the project ({}); refusing to write there",
            export_dir.display(),
            canonical_dir.display()
        )
        .into());
    }
    let path = canonical_dir.join(&output);
    if path.is_symlink() {
        return Err(format!(
            "{} is a symlink; refusing to write through it",
            path.display()
        )
        .into());
    }
    super::import::write(&path, &bytes)?;
    report.output = format!("{EXPORT_DIR}/{output}");
    Ok(report)
}

/// Where exports land, relative to the project.
pub const EXPORT_DIR: &str = "lighting/export";

/// [`export_mvr`] without the write: the archive bytes and the report
/// (whose `output` names where [`export_mvr`] would put them).
pub fn export_mvr_bytes(
    options: &MvrExportOptions,
    project: &Path,
) -> Result<(Vec<u8>, MvrExport), Box<dyn Error>> {
    let output = output_path(options)?;
    let venues = load_venues(&project.join(&options.venues_dir))?;
    let venue = venues.get(&options.venue).ok_or_else(|| {
        let mut names: Vec<&String> = venues.keys().collect();
        names.sort();
        format!(
            "no venue named \"{}\" in {}; venues there: {}",
            options.venue,
            options.venues_dir,
            if names.is_empty() {
                "none".to_string()
            } else {
                names
                    .iter()
                    .map(|n| format!("\"{n}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            }
        )
    })?;
    let fixture_types = load_fixture_types(&project.join(&options.fixture_types_dir))?;

    let mut warnings = Vec::new();
    let mut fixtures: Vec<&Fixture> = venue.fixtures().values().collect();
    fixtures.sort_by(|a, b| {
        (a.universe(), a.start_channel(), a.name()).cmp(&(
            b.universe(),
            b.start_channel(),
            b.name(),
        ))
    });

    // --- Every fixture type resolves to a GDTF entry: the library archive
    // for a referential type, a generated one for a native type. Resolved
    // before anything is rendered so a missing archive refuses the export.
    // Entries are keyed by what they came from — the archive's project
    // path, or the native type — never by the bare file name alone: two
    // library archives called Spot.gdtf in different directories are two
    // entries, the second's name disambiguated.
    let mut entries: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut entry_of_source: HashMap<String, String> = HashMap::new();
    let mut gdtf_of: HashMap<&str, (String, String)> = HashMap::new();
    let mut embedded = Vec::new();
    let mut generated = BTreeMap::new();
    for fixture in &fixtures {
        let type_name = fixture.fixture_type();
        if gdtf_of.contains_key(type_name) {
            continue;
        }
        let fixture_type = fixture_types.get(type_name).ok_or_else(|| {
            format!(
                "fixture \"{}\" uses fixture type \"{type_name}\", which {} does not define",
                fixture.name(),
                options.fixture_types_dir
            )
        })?;
        let (entry, mode) = match fixture_type.source() {
            Some(source) => {
                let source_key = format!("archive:{}", source.path);
                let entry = match entry_of_source.get(&source_key) {
                    Some(entry) => entry.clone(),
                    None => {
                        let bytes = std::fs::read(project.join(&source.path)).map_err(|e| {
                            format!(
                                "fixture type \"{type_name}\" references {}, which cannot be read: {e}",
                                source.path
                            )
                        })?;
                        let wanted = Path::new(&source.path)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .ok_or_else(|| {
                                format!("GDTF path \"{}\" has no file name", source.path)
                            })?;
                        let entry =
                            place(wanted.clone(), source_key, &entries, &mut entry_of_source);
                        if entry != wanted {
                            warnings.push(format!(
                                "two library archives are called {wanted}; {} is embedded as {entry}",
                                source.path
                            ));
                        }
                        entries.insert(entry.clone(), bytes);
                        embedded.push(entry.clone());
                        entry
                    }
                };
                (entry, source.mode.clone())
            }
            None => {
                let source_key = format!("native:{type_name}");
                let entry = match entry_of_source.get(&source_key) {
                    Some(entry) => entry.clone(),
                    None => {
                        let wanted = format!("mtrack_{}.gdtf", fixture_filename_stem(type_name));
                        let entry = place(wanted, source_key, &entries, &mut entry_of_source);
                        entries.insert(entry.clone(), gdtf::generate(fixture_type)?);
                        generated.insert(entry.clone(), type_name.to_string());
                        warnings.push(format!(
                            "fixture type \"{type_name}\" has no GDTF; a minimal one ({entry}) was \
                             generated with its channels and no models"
                        ));
                        entry
                    }
                };
                (entry, gdtf::MODE_NAME.to_string())
            }
        };
        gdtf_of.insert(type_name, (entry, mode));
    }

    let origin_mm = venue
        .source()
        .map(|s| {
            [
                s.origin[0] * 1000.0,
                s.origin[1] * 1000.0,
                s.origin[2] * 1000.0,
            ]
        })
        .unwrap_or([0.0; 3]);
    let unplaced = fixtures.iter().filter(|f| f.position().is_none()).count();
    if unplaced > 0 {
        warnings.push(format!(
            "{unplaced} fixture(s) have no position in the venue; exported without a transform"
        ));
    }

    let scene = render_scene(
        venue,
        &fixtures,
        &gdtf_of,
        &origin_mm,
        options.layers_from_tags,
    );
    let bytes = build_archive(&scene, &entries)?;
    Ok((
        bytes,
        MvrExport {
            venue: venue.name().to_string(),
            output,
            fixtures: fixtures.len(),
            focus_points: venue.focus_points().len(),
            embedded_gdtfs: embedded,
            generated_gdtfs: generated,
            warnings,
        },
    ))
}

/// The output's bare file name: `.mvr`, no directories, nothing hidden.
fn output_path(options: &MvrExportOptions) -> Result<String, Box<dyn Error>> {
    let output = match &options.output {
        Some(output) => output.trim().to_string(),
        None => format!("{}.mvr", fixture_filename_stem(&options.venue)),
    };
    let bare = Path::new(&output).file_name() == Some(std::ffi::OsStr::new(output.as_str()));
    if !bare || output.starts_with('.') || output.contains('\\') || output.contains('\0') {
        return Err(format!(
            "output \"{output}\" must be a bare file name; exports land in {EXPORT_DIR}/"
        )
        .into());
    }
    if !output.to_ascii_lowercase().ends_with(".mvr") || output.len() <= 4 {
        return Err(format!("output \"{output}\" must end in .mvr").into());
    }
    Ok(output)
}

/// A zip entry name for a source: the wanted name, or the first free
/// `(n)` variant when another source already took it.
fn place(
    wanted: String,
    source_key: String,
    entries: &BTreeMap<String, Vec<u8>>,
    entry_of_source: &mut HashMap<String, String>,
) -> String {
    let mut name = wanted.clone();
    let mut n = 1;
    while entries.contains_key(&name) {
        n += 1;
        let stem = wanted.strip_suffix(".gdtf").unwrap_or(&wanted);
        name = format!("{stem} ({n}).gdtf");
    }
    entry_of_source.insert(source_key, name.clone());
    name
}

/// Every venue in a directory (`.venue` and `.light` files alike).
fn load_venues(dir: &Path) -> Result<HashMap<String, Venue>, Box<dyn Error>> {
    let mut venues = HashMap::new();
    for path in lighting_files(dir, &["venue", "light"])? {
        let content = std::fs::read_to_string(&path)?;
        match parse_venues(&content) {
            Ok(parsed) => venues.extend(parsed),
            Err(e) => {
                return Err(format!("{}: {e}", path.display()).into());
            }
        }
    }
    Ok(venues)
}

/// Every fixture type in a directory (`.fixture` and `.light` files alike).
fn load_fixture_types(dir: &Path) -> Result<HashMap<String, FixtureType>, Box<dyn Error>> {
    let mut types = HashMap::new();
    for path in lighting_files(dir, &["fixture", "light"])? {
        let content = std::fs::read_to_string(&path)?;
        match parse_fixture_types(&content) {
            Ok(parsed) => types.extend(parsed),
            Err(e) => {
                return Err(format!("{}: {e}", path.display()).into());
            }
        }
    }
    Ok(types)
}

fn lighting_files(dir: &Path, extensions: &[&str]) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read directory {}: {e}", dir.display()))?;
    let mut files: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| extensions.contains(&e))
        })
        .collect();
    files.sort();
    Ok(files)
}

// --- Scene rendering -------------------------------------------------------

/// The scene's XML. Fixtures carry name, the FixtureID their name ends in
/// (else their ordinal), GDTF reference and mode, address, and transform;
/// focus points their transform. UUIDs are derived from the venue and the
/// object's name, so an export is stable across runs and a console that
/// merges by UUID sees the same objects each time.
fn render_scene(
    venue: &Venue,
    fixtures: &[&Fixture],
    gdtf_of: &HashMap<&str, (String, String)>,
    origin_mm: &Vec3,
    layers_from_tags: bool,
) -> String {
    let mut layers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut place = |layer: String, xml: String| {
        if !layers.contains_key(&layer) {
            order.push(layer.clone());
        }
        layers.entry(layer).or_default().push(xml);
    };

    let ids = fixture_ids(fixtures);
    for (index, fixture) in fixtures.iter().enumerate() {
        let (entry, mode) = &gdtf_of[fixture.fixture_type()];
        let layer = if layers_from_tags {
            fixture
                .tags()
                .first()
                .cloned()
                .unwrap_or_else(|| venue.name().to_string())
        } else {
            venue.name().to_string()
        };
        let mut xml = String::new();
        xml.push_str(&format!(
            "          <Fixture name=\"{}\" uuid=\"{}\">\n",
            escape(fixture.name()),
            uuid(venue.name(), "fixture", fixture.name())
        ));
        if let Some(position) = fixture.position() {
            let rotation = fixture.rotation().unwrap_or([0.0; 3]);
            xml.push_str(&format!(
                "            <Matrix>{}</Matrix>\n",
                matrix_text(&position, &rotation, origin_mm)
            ));
        }
        xml.push_str(&format!(
            "            <GDTFSpec>{}</GDTFSpec>\n            <GDTFMode>{}</GDTFMode>\n",
            escape(entry),
            escape(mode)
        ));
        xml.push_str(&format!(
            "            <Addresses>\n              <Address break=\"0\">{}.{}</Address>\n            </Addresses>\n",
            fixture.universe(),
            fixture.start_channel()
        ));
        xml.push_str(&format!(
            "            <FixtureID>{}</FixtureID>\n            <UnitNumber>0</UnitNumber>\n",
            ids[index]
        ));
        xml.push_str("          </Fixture>\n");
        place(layer, xml);
    }

    for (name, point) in venue.focus_points() {
        let xml = format!(
            "          <FocusPoint name=\"{}\" uuid=\"{}\">\n            <Matrix>{}</Matrix>\n          </FocusPoint>\n",
            escape(name),
            uuid(venue.name(), "focus", name),
            matrix_text(point, &[0.0; 3], origin_mm)
        );
        place(venue.name().to_string(), xml);
    }

    let mut out = String::new();
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    out.push_str(&format!(
        "<GeneralSceneDescription verMajor=\"1\" verMinor=\"6\" provider=\"mtrack\" providerVersion=\"{}\">\n",
        escape(env!("CARGO_PKG_VERSION"))
    ));
    out.push_str("  <UserData/>\n  <Scene>\n    <Layers>\n");
    for layer in order {
        out.push_str(&format!(
            "      <Layer name=\"{}\" uuid=\"{}\">\n        <ChildList>\n",
            escape(&layer),
            uuid(venue.name(), "layer", &layer)
        ));
        for xml in &layers[&layer] {
            out.push_str(xml);
        }
        out.push_str("        </ChildList>\n      </Layer>\n");
    }
    out.push_str("    </Layers>\n  </Scene>\n</GeneralSceneDescription>\n");
    out
}

/// The MVR transform of a stage pose: the basis of `R = Rz·Ry·Rx` as
/// columns, the position back in scene millimeters with the venue's origin
/// restored — the import's [`Matrix::rotation_degrees`] and re-origin
/// undone.
///
/// [`Matrix::rotation_degrees`]: super::mvr::Matrix::rotation_degrees
fn matrix_text(position: &Vec3, rotation_deg: &Vec3, origin_mm: &Vec3) -> String {
    let (rx, ry, rz) = (
        rotation_deg[0].to_radians(),
        rotation_deg[1].to_radians(),
        rotation_deg[2].to_radians(),
    );
    let rotate = |v: [f64; 3]| -> [f64; 3] {
        let (cx, sx) = (rx.cos(), rx.sin());
        let v = [v[0], cx * v[1] - sx * v[2], sx * v[1] + cx * v[2]];
        let (cy, sy) = (ry.cos(), ry.sin());
        let v = [cy * v[0] + sy * v[2], v[1], -sy * v[0] + cy * v[2]];
        let (cz, sz) = (rz.cos(), rz.sin());
        [cz * v[0] - sz * v[1], sz * v[0] + cz * v[1], v[2]]
    };
    let u = rotate([1.0, 0.0, 0.0]);
    let v = rotate([0.0, 1.0, 0.0]);
    let w = rotate([0.0, 0.0, 1.0]);
    let o = [
        position[0] * 1000.0 + origin_mm[0],
        position[1] * 1000.0 + origin_mm[1],
        position[2] * 1000.0 + origin_mm[2],
    ];
    let group = |a: [f64; 3]| format!("{{{},{},{}}}", num(a[0]), num(a[1]), num(a[2]));
    format!("{}{}{}{}", group(u), group(v), group(w), group(o))
}

/// A number without float noise: six decimals, trailing zeros trimmed, no
/// negative zero.
fn num(value: f64) -> String {
    let rounded = (value * 1_000_000.0).round() / 1_000_000.0;
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    let text = format!("{rounded:.6}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text.is_empty() {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// The FixtureIDs, one per fixture and all distinct: the number a name
/// ends in (the import names repeated fixtures "Type ID", and people
/// number theirs), else the lowest number no other fixture has. The venue
/// does not keep a console's IDs for uniquely named fixtures, so those are
/// mtrack's to assign.
fn fixture_ids(fixtures: &[&Fixture]) -> Vec<u64> {
    let named: Vec<Option<u64>> = fixtures
        .iter()
        .map(|f| {
            f.name()
                .rsplit(' ')
                .next()
                .filter(|last| !last.is_empty() && last.len() < 10)
                .and_then(|last| last.parse::<u64>().ok())
        })
        .collect();
    let mut taken: std::collections::HashSet<u64> = std::collections::HashSet::new();
    let mut ids = vec![0u64; fixtures.len()];
    // Named numbers first (first come, first served), then the rest.
    for (i, id) in named.iter().enumerate() {
        if let Some(id) = id {
            if taken.insert(*id) {
                ids[i] = *id;
            }
        }
    }
    let mut next = 1;
    for id in ids.iter_mut() {
        if *id == 0 {
            while taken.contains(&next) {
                next += 1;
            }
            taken.insert(next);
            *id = next;
        }
    }
    ids
}

/// A stable UUID (version-5 shaped, SHA-256 derived) for a scene object.
fn uuid(venue: &str, kind: &str, name: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"mtrack-mvr\0");
    hasher.update(venue.as_bytes());
    hasher.update([0u8]);
    hasher.update(kind.as_bytes());
    hasher.update([0u8]);
    hasher.update(name.as_bytes());
    let h = hasher.finalize();
    format!(
        "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-5{:01X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        h[0],
        h[1],
        h[2],
        h[3],
        h[4],
        h[5],
        h[6] & 0x0F,
        h[7],
        (h[8] & 0x3F) | 0x80,
        h[9],
        h[10],
        h[11],
        h[12],
        h[13],
        h[14],
        h[15]
    )
}

/// XML-escapes text for an attribute or element body.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// The `.mvr`: the scene (deflated) and the GDTFs (stored — they are zips
/// already), each under its bare entry name.
fn build_archive(
    scene: &str,
    entries: &BTreeMap<String, Vec<u8>>,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let deflated: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let stored: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        writer.start_file("GeneralSceneDescription.xml", deflated)?;
        writer.write_all(scene.as_bytes())?;
        for (name, bytes) in entries {
            writer.start_file(name, stored)?;
            writer.write_all(bytes)?;
        }
        writer.finish()?;
    }
    Ok(cursor.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::import::{import_mvr_bytes, inspect_mvr_bytes, MvrImportOptions};
    use crate::lighting::mvr;

    /// The scene the tests seed from: two bricks and a mover of the
    /// synthetic GDTF, a focus point, one layer.
    const SCENE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<GeneralSceneDescription verMajor="1" verMinor="6"><Scene><Layers>
<Layer name="Front Truss"><ChildList>
<Fixture name="Brick 1"><FixtureID>101</FixtureID><Matrix>{1,0,0}{0,1,0}{0,0,1}{-2000,3500,4200}</Matrix>
<GDTFSpec>Astera_PB15.gdtf</GDTFSpec><GDTFMode>8: RGBS</GDTFMode>
<Addresses><Address break="0">1.1</Address></Addresses></Fixture>
<Fixture name="Brick 2"><Matrix>{-1,0,0}{0,-1,0}{0,0,1}{2000,3500,4200}</Matrix>
<GDTFSpec>Astera_PB15.gdtf</GDTFSpec><GDTFMode>8: RGBS</GDTFMode>
<Addresses><Address break="0">1.5</Address></Addresses></Fixture>
<Fixture name="Mover 1"><GDTFSpec>Astera_PB15.gdtf</GDTFSpec><GDTFMode>Mover 16bit</GDTFMode>
<Addresses><Address break="0">2.100</Address></Addresses></Fixture>
<FocusPoint name="Drummer"><Matrix>{1,0,0}{0,1,0}{0,0,1}{0,2800,1400}</Matrix></FocusPoint>
</ChildList></Layer>
</Layers></Scene></GeneralSceneDescription>"#;

    /// A project seeded from [`SCENE`].
    fn seeded_project() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(&project).unwrap();
        let gdtf = crate::lighting::gdtf::build_zip(&[(
            "description.xml",
            crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.as_bytes(),
        )]);
        let bytes = crate::lighting::gdtf::build_zip(&[
            ("GeneralSceneDescription.xml", SCENE.as_bytes()),
            ("Astera_PB15.gdtf", gdtf.as_slice()),
        ]);
        let options = MvrImportOptions {
            name: Some("kellys".to_string()),
            origin_mm: Some([0.0, -3500.0, 0.0]),
            ..MvrImportOptions::default()
        };
        import_mvr_bytes(&bytes, "kellys.mvr", &options, &project).unwrap();
        (dir, project)
    }

    #[test]
    fn an_imported_venue_round_trips_through_export() {
        let (_dir, project) = seeded_project();
        let before = inspect_mvr_bytes(
            &std::fs::read(project.join("lighting/library/kellys.mvr")).unwrap(),
            "kellys.mvr",
            &MvrImportOptions {
                name: Some("kellys".to_string()),
                ..MvrImportOptions::default()
            },
            &project,
        )
        .unwrap();

        let report = export_mvr(&MvrExportOptions::for_venue("kellys"), &project).unwrap();
        assert_eq!(report.output, "lighting/export/kellys.mvr");
        assert_eq!(report.fixtures, 3);
        assert_eq!(report.focus_points, 1);
        assert_eq!(report.embedded_gdtfs, vec!["Astera_PB15.gdtf".to_string()]);
        assert!(report.generated_gdtfs.is_empty());
        let exported = std::fs::read(project.join(&report.output)).unwrap();

        // The export parses as an MVR with the archive embedded intact...
        let scene = mvr::parse_archive(&exported).unwrap();
        assert!(scene.warnings.is_empty(), "{:?}", scene.warnings);
        assert_eq!(scene.fixtures.len(), 3);
        assert_eq!(scene.focus_points.len(), 1);
        let embedded = mvr::read_gdtf_entry(&exported, "Astera_PB15.gdtf").unwrap();
        assert_eq!(
            embedded,
            std::fs::read(project.join("lighting/library/Astera_PB15.gdtf")).unwrap()
        );
        let brick = scene.fixtures.iter().find(|f| f.name == "Brick 1").unwrap();
        assert_eq!(brick.fixture_id.as_deref(), Some("1"));
        assert_eq!(brick.gdtf_mode.as_deref(), Some("8: RGBS"));
        assert_eq!(brick.addresses, vec![(1, 1)]);
        // The origin was restored: the MVR millimeters come back as they went in.
        let o = brick.matrix.unwrap().o;
        assert!(
            (o[0] + 2000.0).abs() < 1e-6
                && (o[1] - 3500.0).abs() < 1e-6
                && (o[2] - 4200.0).abs() < 1e-6,
            "{o:?}"
        );

        // ...and re-importing it is a merge that changes nothing.
        let after = inspect_mvr_bytes(
            &exported,
            "kellys.mvr",
            &MvrImportOptions {
                name: Some("kellys".to_string()),
                ..MvrImportOptions::default()
            },
            &project,
        )
        .unwrap();
        assert!(after.merge);
        assert_eq!(after.origin, before.origin);
        assert!(
            after.fixture_types.iter().all(|t| t.existing),
            "{:?}",
            after.fixture_types
        );
        assert!(after.removed_fixtures.is_empty());
        assert_eq!(after.fixtures.len(), before.fixtures.len());
        for (a, b) in after.fixtures.iter().zip(&before.fixtures) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.fixture_type, b.fixture_type);
            assert_eq!(a.patch, b.patch);
            assert_eq!(a.position.is_some(), b.position.is_some());
            if let (Some(p), Some(q)) = (a.position, b.position) {
                assert!((0..3).all(|i| (p[i] - q[i]).abs() < 1e-6), "{p:?} vs {q:?}");
            }
            assert!(
                a.change.as_deref().is_none_or(|c| c == "unchanged"),
                "{}: {:?}",
                a.name,
                a.change
            );
        }
        assert!(after
            .focus_points
            .iter()
            .all(|f| f.change.as_deref().is_none_or(|c| c == "unchanged")));
    }

    #[test]
    fn a_native_type_gets_a_generated_gdtf_and_layers_follow_tags() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().to_path_buf();
        std::fs::create_dir_all(project.join("lighting/fixture_types")).unwrap();
        std::fs::create_dir_all(project.join("lighting/venues")).unwrap();
        std::fs::write(
            project.join("lighting/fixture_types/par.light"),
            "fixture_type \"Par\" {\n  channels: 4\n  channel_map: {\"red\": 1, \"green\": 2, \"blue\": 3, \"dimmer\": 4}\n}\n",
        )
        .unwrap();
        std::fs::write(
            project.join("lighting/venues/club.venue"),
            "venue \"Club\" {\n  fixture \"Left\" Par @ 1:1 tags [\"front\"] position (-2, 0.5, 3) rotation (0, 0, 180)\n  fixture \"Spare\" Par @ 1:9\n  focus \"drummer\" (0, 2.8, 1.4)\n}\n",
        )
        .unwrap();
        let options = MvrExportOptions {
            output: Some("club.mvr".to_string()),
            layers_from_tags: true,
            ..MvrExportOptions::for_venue("Club")
        };
        let report = export_mvr(&options, &project).unwrap();
        assert_eq!(report.output, "lighting/export/club.mvr");
        assert_eq!(
            report
                .generated_gdtfs
                .get("mtrack_par.gdtf")
                .map(String::as_str),
            Some("Par")
        );
        assert!(
            report.warnings.iter().any(|w| w.contains("no GDTF")),
            "{:?}",
            report.warnings
        );
        assert!(
            report.warnings.iter().any(|w| w.contains("no position")),
            "{:?}",
            report.warnings
        );

        let exported = std::fs::read(project.join("lighting/export/club.mvr")).unwrap();
        let scene = mvr::parse_archive(&exported).unwrap();
        let left = scene.fixtures.iter().find(|f| f.name == "Left").unwrap();
        assert_eq!(left.layer, "front");
        assert_eq!(left.gdtf_spec.as_deref(), Some("mtrack_par.gdtf"));
        assert_eq!(left.gdtf_mode.as_deref(), Some(gdtf::MODE_NAME));
        let (rotation, exact) = left.matrix.unwrap().rotation_degrees();
        assert!(exact);
        assert!((rotation[2].abs() - 180.0).abs() < 1e-6, "{rotation:?}");
        let spare = scene.fixtures.iter().find(|f| f.name == "Spare").unwrap();
        assert_eq!(spare.layer, "Club", "untagged: the venue's layer");
        assert!(spare.matrix.is_none());

        // The generated GDTF distills back to the same channels.
        let gdtf_bytes = mvr::read_gdtf_entry(&exported, "mtrack_par.gdtf").unwrap();
        let description = crate::lighting::gdtf::parse_archive(&gdtf_bytes).unwrap();
        let distilled =
            crate::lighting::gdtf::distill(&description, gdtf::MODE_NAME, "Par").unwrap();
        let mut channels: Vec<(String, u16)> = distilled
            .fixture_type
            .channels()
            .iter()
            .map(|(n, o)| (n.clone(), *o))
            .collect();
        channels.sort_by_key(|(_, o)| *o);
        assert_eq!(
            channels,
            vec![
                ("red".to_string(), 1),
                ("green".to_string(), 2),
                ("blue".to_string(), 3),
                ("dimmer".to_string(), 4)
            ]
        );

        // Re-importing the export into the same project resolves every
        // fixture to a type (a hand-written venue is never merged into, so
        // it seeds a new one).
        let plan = inspect_mvr_bytes(
            &exported,
            "club.mvr",
            &MvrImportOptions {
                name: Some("Club from export".to_string()),
                ..MvrImportOptions::default()
            },
            &project,
        )
        .unwrap();
        assert!(!plan.merge);
        assert!(
            plan.fixtures.iter().all(|f| f.todo.is_none()),
            "{:?}",
            plan.fixtures
        );
        let left = plan.fixtures.iter().find(|f| f.name == "Left").unwrap();
        assert_eq!(left.fixture_type.as_deref(), Some("Par"));
        assert_eq!(left.patch, Some((1, 1)));
        let p = left.position.unwrap();
        assert!(
            (p[0] + 2.0).abs() < 1e-6 && (p[1] - 0.5).abs() < 1e-6,
            "{p:?}"
        );
    }

    #[test]
    fn missing_venue_or_type_or_archive_refuses_before_writing() {
        let (_dir, project) = seeded_project();
        let err = export_mvr(&MvrExportOptions::for_venue("nope"), &project)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("no venue named \"nope\"") && err.contains("\"kellys\""),
            "{err}"
        );

        std::fs::remove_file(project.join("lighting/library/Astera_PB15.gdtf")).unwrap();
        let err = export_mvr(&MvrExportOptions::for_venue("kellys"), &project)
            .unwrap_err()
            .to_string();
        assert!(err.contains("cannot be read"), "{err}");
        assert!(!project.join("lighting/export").exists());
    }

    #[test]
    fn the_output_is_a_bare_mvr_name_under_the_export_directory() {
        let (_dir, project) = seeded_project();
        let venue_file = project.join("lighting/venues/kellys.venue");
        let venue_before = std::fs::read(&venue_file).unwrap();
        for output in [
            "../escape.mvr",
            "lighting/venues/kellys.venue",
            "kellys.venue",
            "/tmp/x.mvr",
            ".hidden.mvr",
            "sub/dir.mvr",
            ".mvr",
        ] {
            let err = export_mvr(
                &MvrExportOptions {
                    output: Some(output.to_string()),
                    ..MvrExportOptions::for_venue("kellys")
                },
                &project,
            )
            .unwrap_err()
            .to_string();
            assert!(
                err.contains("bare file name") || err.contains("end in .mvr"),
                "{output}: {err}"
            );
        }
        assert_eq!(std::fs::read(&venue_file).unwrap(), venue_before);
        let report = export_mvr(
            &MvrExportOptions {
                output: Some("Kelly's Rig.MVR".to_string()),
                ..MvrExportOptions::for_venue("kellys")
            },
            &project,
        )
        .unwrap();
        assert_eq!(report.output, "lighting/export/Kelly's Rig.MVR");
        assert!(project.join(&report.output).is_file());

        // A symlinked export directory pointing outside the project is
        // refused before anything is written through it.
        #[cfg(unix)]
        {
            let outside = tempfile::tempdir().unwrap();
            std::fs::remove_dir_all(project.join("lighting/export")).unwrap();
            std::os::unix::fs::symlink(outside.path(), project.join("lighting/export")).unwrap();
            let err = export_mvr(&MvrExportOptions::for_venue("kellys"), &project)
                .unwrap_err()
                .to_string();
            assert!(err.contains("outside the project"), "{err}");
            assert_eq!(std::fs::read_dir(outside.path()).unwrap().count(), 0);
        }
    }

    #[test]
    fn same_named_archives_in_different_directories_are_both_embedded() {
        let (_dir, project) = seeded_project();
        // A second archive with the same file name in another library
        // directory, and a type that uses it.
        let other_dir = project.join("lighting/library/other");
        std::fs::create_dir_all(&other_dir).unwrap();
        let other = crate::lighting::gdtf::build_zip(&[(
            "description.xml",
            crate::lighting::gdtf::SYNTHETIC_DESCRIPTION
                .replace("Synth Brick", "Other Brick")
                .as_bytes(),
        )]);
        std::fs::write(other_dir.join("Astera_PB15.gdtf"), &other).unwrap();
        std::fs::write(
            project.join("lighting/fixture_types/other_brick.fixture"),
            "fixture_type \"Other Brick\"\n  from gdtf(\"lighting/library/other/Astera_PB15.gdtf\", mode \"8: RGBS\")\n{\n}\n",
        )
        .unwrap();
        let venue_file = project.join("lighting/venues/kellys.venue");
        let mut venue = std::fs::read_to_string(&venue_file).unwrap();
        venue = venue.replace(
            "\n}",
            "\n  fixture \"Other 1\" \"Other Brick\" @ 3:1 position (0, 1, 2) rotation (0, 0, 0)\n}",
        );
        std::fs::write(&venue_file, venue).unwrap();

        let report = export_mvr(&MvrExportOptions::for_venue("kellys"), &project).unwrap();
        assert_eq!(
            report.embedded_gdtfs.len(),
            2,
            "{:?}",
            report.embedded_gdtfs
        );
        assert!(
            report
                .warnings
                .iter()
                .any(|w| w.contains("two library archives")),
            "{:?}",
            report.warnings
        );
        let exported = std::fs::read(project.join(&report.output)).unwrap();
        let scene = mvr::parse_archive(&exported).unwrap();
        let other_fixture = scene.fixtures.iter().find(|f| f.name == "Other 1").unwrap();
        let spec = other_fixture.gdtf_spec.clone().unwrap();
        assert_eq!(spec, "Astera_PB15 (2).gdtf");
        assert_eq!(mvr::read_gdtf_entry(&exported, &spec).unwrap(), other);
        let brick = scene.fixtures.iter().find(|f| f.name == "Brick 1").unwrap();
        assert_eq!(brick.gdtf_spec.as_deref(), Some("Astera_PB15.gdtf"));
    }

    #[test]
    fn fixture_ids_are_distinct() {
        let mk = |name: &str, address: u16| {
            Fixture::new(name.to_string(), "T".to_string(), 1, address, Vec::new())
        };
        let fixtures = [
            mk("Wash 3", 1),
            mk("Spot", 2),
            mk("Spot", 3),
            mk("Robe Spiider 12", 4),
            mk("Left 3", 5),
        ];
        let refs: Vec<&Fixture> = fixtures.iter().collect();
        let ids = fixture_ids(&refs);
        assert_eq!(ids, vec![3, 1, 2, 12, 4]);
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len());
    }

    #[test]
    fn matrices_round_trip_through_the_importer_math() {
        for rotation in [
            [0.0, 0.0, 0.0],
            [0.0, 0.0, 180.0],
            [90.0, 0.0, 0.0],
            [30.0, -45.0, 120.0],
        ] {
            let text = matrix_text(&[-2.0, 3.5, 4.2], &rotation, &[0.0, -3500.0, 0.0]);
            let scene =
                mvr::SYNTHETIC_SCENE.replace("{1,0,0}{0,1,0}{0,0,1}{-2000,3500,4200}", &text);
            let parsed = mvr::parse_scene(&scene).unwrap();
            let matrix = parsed.fixtures[0].matrix.unwrap();
            assert!(
                (matrix.o[1] - 0.0).abs() < 1e-6,
                "{text}: origin restored → {:?}",
                matrix.o
            );
            let (back, exact) = matrix.rotation_degrees();
            assert!(exact);
            // The same rotation (angles may differ by a full turn or gimbal
            // equivalence, so compare the frames they produce).
            let frame = |r: &Vec3| -> Vec<f64> {
                matrix_text(&[0.0; 3], r, &[0.0; 3])
                    .split(['{', '}', ','])
                    .filter(|t| !t.is_empty())
                    .map(|t| t.parse().unwrap())
                    .collect()
            };
            let (a, b) = (frame(&back), frame(&rotation));
            assert!(
                a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-4),
                "{rotation:?} → {back:?}"
            );
        }
        assert_eq!(num(-0.0), "0");
        assert_eq!(num(1.5), "1.5");
        assert_eq!(num(0.1 + 0.2), "0.3");
        assert_eq!(escape("a<b & \"c\""), "a&lt;b &amp; &quot;c&quot;");
        assert_eq!(uuid("v", "fixture", "x").len(), 36);
        assert_eq!(uuid("v", "fixture", "x"), uuid("v", "fixture", "x"));
        assert_ne!(uuid("v", "fixture", "x"), uuid("v", "focus", "x"));
    }
}
