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

//! Bring-your-own MVR corpus checks (venue-exchange design §12, tier 2).
//!
//! Real venue files come from consoles and pre-viz tools, not from anywhere
//! CI can fetch. gdtf.eu publishes ten sample exports (Basic_Festival,
//! Circle_Stage, Midsize_w_GP, Messy_Patch, Key_Arena_Remake_MVR,
//! Template_Stage1, Demoshow_grandMA3, Demostage_MVR, Simple_Show,
//! Capture_Demo) under `https://www.gdtf.eu/mvr_files/`; all ten import
//! with one TODO between them as of 2026-09-19. Drop `.mvr` files into
//! `tests/mvr-corpus/` (gitignored) and run:
//!
//! ```sh
//! cargo test --test mvr_corpus -- --ignored --nocapture
//! ```
//!
//! Every file must parse; every fixture's GDTF reference should resolve to
//! an embedded archive that itself parses. Scene warnings are printed for
//! eyeballing, not asserted — they are console-quirk facts, not code facts.

use mtrack::lighting::{gdtf, mvr};

#[test]
#[ignore = "bring-your-own corpus: put .mvr files in tests/mvr-corpus/ and run with --ignored"]
fn every_corpus_file_parses_and_references_resolve() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mvr-corpus");
    assert!(
        dir.is_dir(),
        "no corpus at {}: create it and drop .mvr files in (see module docs)",
        dir.display()
    );
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("readable corpus dir")
        .filter_map(|entry| {
            let path = entry.expect("dir entry").path();
            (path.extension().is_some_and(|e| e == "mvr")).then_some(path)
        })
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "corpus dir {} holds no .mvr files",
        dir.display()
    );

    for path in files {
        let bytes = std::fs::read(&path).expect("readable mvr");
        let scene = mvr::parse_archive(&bytes)
            .unwrap_or_else(|e| panic!("{}: failed to parse: {e}", path.display()));
        let embedded = mvr::list_gdtf_entries(&bytes).expect("entry listing");
        println!(
            "{}: {} fixtures, {} embedded GDTFs",
            path.display(),
            scene.fixtures.len(),
            embedded.len()
        );
        for warning in &scene.warnings {
            println!("  [scene] {warning}");
        }
        assert!(
            !scene.fixtures.is_empty(),
            "{}: no fixtures found",
            path.display()
        );

        for fixture in &scene.fixtures {
            let Some(spec) = fixture.gdtf_spec.as_deref() else {
                println!("  [{}] no GDTF reference", fixture.name);
                continue;
            };
            // The importer's own resolution: extension, path prefix and
            // manufacturer prefix tolerated, ambiguity reported.
            let entry = match mvr::resolve_gdtf_entry(&embedded, spec) {
                Ok(entry) => entry.map(str::to_string),
                Err(e) => {
                    println!("  [{}] {e}", fixture.name);
                    continue;
                }
            };
            let Some(entry) = entry else {
                println!(
                    "  [{}] GDTF reference {spec:?} matches no embedded entry",
                    fixture.name
                );
                continue;
            };
            let gdtf_bytes = mvr::read_gdtf_entry(&bytes, &entry)
                .unwrap_or_else(|e| panic!("{}: {entry}: {e}", path.display()));
            let description = gdtf::parse_archive(&gdtf_bytes)
                .unwrap_or_else(|e| panic!("{}: {entry}: failed to parse: {e}", path.display()));
            println!(
                "  [{}] {} @ {:?} → \"{}\" mode {:?}",
                fixture.name,
                entry,
                fixture.addresses.first(),
                description.name,
                fixture.gdtf_mode
            );
        }
    }
}

/// Every embedded GDTF of the corpus distills a rig for every mode (design
/// §16.2), and the Spiider — the one pixel mover with glTF meshes — comes
/// out with its axes, its 20 cells (19 pixels and the flower) and their
/// beams.
#[test]
#[ignore = "bring-your-own corpus: put .mvr files in tests/mvr-corpus/ and run with --ignored"]
fn every_embedded_gdtf_distills_a_rig_per_mode() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mvr-corpus");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("readable corpus dir")
        .filter_map(|entry| {
            let path = entry.expect("dir entry").path();
            (path.extension().is_some_and(|e| e == "mvr")).then_some(path)
        })
        .collect();
    files.sort();
    let mut spiider_checked = false;
    let mut rigs = 0;
    for path in files {
        let bytes = std::fs::read(&path).expect("readable mvr");
        for entry in mvr::list_gdtf_entries(&bytes).expect("entry listing") {
            let gdtf_bytes = mvr::read_gdtf_entry(&bytes, &entry)
                .unwrap_or_else(|e| panic!("{}: {entry}: {e}", path.display()));
            let description = gdtf::parse_archive(&gdtf_bytes)
                .unwrap_or_else(|e| panic!("{entry}: failed to parse: {e}"));
            let files = gdtf::list_model_files(&gdtf_bytes).expect("model listing");
            for mode in &description.modes {
                let rig = gdtf::distill_rig(&description, &mode.name, &files)
                    .unwrap_or_else(|e| panic!("{entry} mode {:?}: {e}", mode.name));
                rigs += 1;
                let has_pan = mode
                    .channels
                    .iter()
                    .any(|c| c.logical_channels.iter().any(|l| l.attribute == "Pan"));
                assert_eq!(
                    rig.pan.is_some(),
                    has_pan,
                    "{entry} mode {:?}: a pan channel means a pan node",
                    mode.name
                );
                if description.name == "Robin Spiider" && mode.name == "Mode 8 - Pixel RGBW " {
                    spiider_checked = true;
                    let name = |i: Option<usize>| i.map(|i| rig.nodes[i].name.as_str());
                    assert_eq!(name(rig.pan), Some("Yoke"));
                    assert_eq!(name(rig.tilt), Some("Head"));
                    let cell_names: Vec<&str> = rig
                        .nodes
                        .iter()
                        .filter(|n| matches!(n.role, gdtf::RigRole::Cell { .. }))
                        .map(|n| n.name.as_str())
                        .collect();
                    assert_eq!(cell_names.len(), 20, "{:?}", rig.nodes);
                    assert!(rig.beams.len() >= 20, "{} beams", rig.beams.len());
                    let meshes = rig
                        .nodes
                        .iter()
                        .filter(|n| matches!(n.shape, gdtf::RigShape::Model { .. }))
                        .count();
                    assert!(
                        meshes >= 6,
                        "{meshes} meshes: base, yoke, head, three lens kinds"
                    );

                    // The distiller's own cells (design §17.2): the pixel
                    // mode puts RGBW on the three lens templates and
                    // references them nineteen times with Break offsets;
                    // expanded per reference, the nineteen lenses gang into
                    // one group of cells named for the references. The
                    // flower is a differently shaped section and stays out.
                    let distilled = gdtf::distill(&description, &mode.name, "Robin Spiider")
                        .unwrap_or_else(|e| panic!("{entry} mode {:?}: {e}", mode.name));
                    let cells = distilled.fixture_type.cells();
                    let names: Vec<&str> = cells.iter().map(|c| c.name.as_str()).collect();
                    assert_eq!(cells.len(), 19, "{names:?}\n{:?}", distilled.warnings);
                    assert_eq!(names[0], "P1 Zone1");
                    assert_eq!(names[18], "P19 Zone3");
                    // Offsets follow the Break table: P3 Zone2 sits four
                    // bytes after P2 Zone 2, P19 Zone3 at the mode's end.
                    let red = |name: &str| {
                        cells.iter().find(|c| c.name == name).unwrap().channels["red"].offset
                    };
                    assert_eq!(red("P2 Zone 2") + 4, red("P3 Zone2"));
                    assert_eq!(red("P19 Zone3"), 107);
                    // The whole-fixture view still fans out: the group's
                    // owner mirrors the other eighteen.
                    let owner_red = distilled
                        .fixture_type
                        .channel_defs()
                        .iter()
                        .find(|(_, d)| d.offset == red("P1 Zone1"))
                        .map(|(n, d)| (n.clone(), d.mirrors.len()))
                        .unwrap();
                    assert_eq!(owner_red.1, 18, "{owner_red:?}");
                    for cell in cells {
                        assert!(
                            cell_names.contains(&cell.name.as_str()),
                            "distilled cell {:?} is not among the rig's Cell-role nodes {cell_names:?}",
                            cell.name
                        );
                        for channel in ["red", "green", "blue", "white"] {
                            assert!(
                                cell.channels.contains_key(channel),
                                "cell {:?} is missing channel {channel:?}: {:?}",
                                cell.name,
                                cell.channels.keys().collect::<Vec<_>>()
                            );
                        }
                    }
                }
            }
        }
    }
    println!("{rigs} rigs distilled");
    assert!(
        spiider_checked,
        "the Demoshow/Basic_Festival corpus files carry the Robin Spiider"
    );
}

/// Import → export → import (design §16.6, P2-3 exit): every corpus venue
/// exported by mtrack re-imports as a merge that changes nothing.
#[test]
#[ignore = "bring-your-own corpus: put .mvr files in tests/mvr-corpus/ and run with --ignored"]
fn every_corpus_venue_round_trips_through_export() {
    use mtrack::lighting::export::{export_mvr_bytes, MvrExportOptions};
    use mtrack::lighting::import::{import_mvr_bytes, inspect_mvr_bytes, MvrImportOptions};

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mvr-corpus");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("readable corpus dir")
        .filter_map(|entry| {
            let path = entry.expect("dir entry").path();
            (path.extension().is_some_and(|e| e == "mvr")).then_some(path)
        })
        .collect();
    files.sort();
    for path in files {
        let project = tempfile::tempdir().expect("temp project");
        let bytes = std::fs::read(&path).expect("readable mvr");
        let file_name = path.file_name().unwrap().to_string_lossy().into_owned();
        let options = MvrImportOptions::default();
        let report = import_mvr_bytes(&bytes, &file_name, &options, project.path())
            .unwrap_or_else(|e| panic!("{file_name}: import: {e}"));
        let venue = report.plan.venue_name.clone();
        let (exported, export) =
            export_mvr_bytes(&MvrExportOptions::for_venue(&venue), project.path())
                .unwrap_or_else(|e| panic!("{file_name}: export: {e}"));
        let before: Vec<_> = report
            .plan
            .fixtures
            .iter()
            .filter(|f| f.todo.is_none())
            .collect();
        assert_eq!(export.fixtures, before.len(), "{file_name}");
        assert!(
            export.generated_gdtfs.is_empty(),
            "{file_name}: {:?}",
            export.generated_gdtfs
        );

        let after = inspect_mvr_bytes(&exported, &file_name, &options, project.path())
            .unwrap_or_else(|e| panic!("{file_name}: re-import: {e}"));
        assert!(after.merge, "{file_name}");
        assert!(
            after.fixture_types.iter().all(|t| t.existing),
            "{file_name}: {:?}",
            after.fixture_types
        );
        assert!(
            after.removed_fixtures.is_empty(),
            "{file_name}: {:?}",
            after.removed_fixtures
        );
        let changed: Vec<String> = after
            .fixtures
            .iter()
            .filter(|f| f.change.as_deref().is_some_and(|c| c != "unchanged"))
            .map(|f| format!("{}: {:?}", f.name, f.change))
            .collect();
        assert!(changed.is_empty(), "{file_name}: {changed:?}");
        assert_eq!(after.fixtures.len(), before.len(), "{file_name}");
        println!(
            "{file_name}: {} fixtures, {} focus points, {} GDTFs round-trip",
            export.fixtures,
            export.focus_points,
            export.embedded_gdtfs.len()
        );
    }
}

/// Every corpus venue's scenery distills into the store (design §16.6,
/// P2-4): glTF meshes copied and drawn, `.3ds` reported and skipped.
#[test]
#[ignore = "bring-your-own corpus: put .mvr files in tests/mvr-corpus/ and run with --ignored"]
fn every_corpus_venue_distills_its_scenery() {
    use mtrack::lighting::distill::DistillCache;

    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mvr-corpus");
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("readable corpus dir")
        .filter_map(|entry| {
            let path = entry.expect("dir entry").path();
            (path.extension().is_some_and(|e| e == "mvr")).then_some(path)
        })
        .collect();
    files.sort();
    let mut drawn_somewhere = false;
    for path in files {
        let bytes = std::fs::read(&path).expect("readable mvr");
        let scene = mvr::parse_archive(&bytes).expect("parses");
        let store = tempfile::tempdir().expect("temp store");
        let cache = DistillCache::new(store.path().to_path_buf());
        let (rel, warnings) = cache
            .ensure_scenery(&bytes, &[0.0; 3], || Ok(&scene))
            .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let model = cache.scenery(&rel).expect("scenery reads back");
        let drawn: usize = model.objects.iter().map(|o| o.meshes.len()).sum();
        let skipped: usize = model.objects.iter().map(|o| o.skipped.len()).sum();
        if drawn > 0 {
            drawn_somewhere = true;
        }
        for object in &model.objects {
            for mesh in &object.meshes {
                let file = store
                    .path()
                    .join("assets")
                    .join(rel.rsplit_once('/').unwrap().0)
                    .join(&mesh.file);
                assert!(file.is_file(), "{}: {} missing", path.display(), mesh.file);
                let bytes = std::fs::read(&file).unwrap();
                assert!(
                    bytes.starts_with(b"glTF"),
                    "{}: {} is not a glb",
                    path.display(),
                    mesh.file
                );
            }
        }
        println!(
            "{}: {} objects, {} meshes drawn, {} skipped, formats {:?}",
            path.file_name().unwrap().to_string_lossy(),
            model.objects.len(),
            drawn,
            skipped,
            model.formats
        );
        for warning in &warnings {
            println!("  [scenery] {warning}");
        }
    }
    assert!(
        drawn_somewhere,
        "the corpus carries glTF scenery (Basic_Festival, Circle_Stage, Messy_Patch, Midsize_w_GP)"
    );
}
