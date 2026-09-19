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
                    let cells = rig
                        .nodes
                        .iter()
                        .filter(|n| matches!(n.role, gdtf::RigRole::Cell { .. }))
                        .count();
                    assert_eq!(cells, 20, "{:?}", rig.nodes);
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
