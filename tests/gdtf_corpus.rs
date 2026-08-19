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

//! Bring-your-own GDTF corpus checks (venue-exchange design §12, tier 2).
//!
//! Manufacturer GDTF files are not committed and never fetched by CI —
//! vendor URLs rot and block non-browser clients. Drop real `.gdtf` files
//! into `tests/gdtf-corpus/` (gitignored) and run:
//!
//! ```sh
//! cargo test --test gdtf_corpus -- --ignored --nocapture
//! ```
//!
//! Every file must parse, and every non-pixel mode must distill; pixel
//! modes must refuse with the multi-instance error rather than
//! half-import. Warnings are printed for eyeballing, not asserted — they
//! are manufacturer-file facts, not code facts.

use mtrack::lighting::gdtf;

fn corpus_files() -> Vec<std::path::PathBuf> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/gdtf-corpus");
    assert!(
        dir.is_dir(),
        "no corpus at {}: create it and drop .gdtf files in (see module docs)",
        dir.display()
    );
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("readable corpus dir")
        .filter_map(|entry| {
            let path = entry.expect("dir entry").path();
            (path.extension().is_some_and(|e| e == "gdtf")).then_some(path)
        })
        .collect();
    files.sort();
    assert!(
        !files.is_empty(),
        "corpus dir {} holds no .gdtf files",
        dir.display()
    );
    files
}

/// The PixelBrick proof, against the real manufacturer file: distilling
/// mode "8: RGBS" must agree with the hand-written v1 definition this
/// project has used in production. Skips loudly when the corpus doesn't
/// hold the file.
#[test]
#[ignore = "bring-your-own corpus: put .gdtf files in tests/gdtf-corpus/ and run with --ignored"]
fn pixelbrick_rgbs_matches_the_hand_written_definition() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/gdtf-corpus/pb15.gdtf");
    if !path.is_file() {
        println!(
            "SKIPPED: {} not present — drop the Astera PB15 PixelBrick GDTF there to run this",
            path.display()
        );
        return;
    }
    let bytes = std::fs::read(&path).expect("readable gdtf");
    let description = gdtf::parse_archive(&bytes).expect("parses");
    let distilled = gdtf::distill(&description, "8: RGBS", "Astera-PixelBrick").expect("distills");
    let ft = &distilled.fixture_type;

    assert_eq!(ft.channels().get("red"), Some(&1));
    assert_eq!(ft.channels().get("green"), Some(&2));
    assert_eq!(ft.channels().get("blue"), Some(&3));
    assert_eq!(ft.channels().get("strobe"), Some(&4));
    assert_eq!(ft.channels().len(), 4);
    assert_eq!(ft.footprint(), 4);
    // The three numbers hand-transcribed from the manual, recovered from
    // the manufacturer's own file.
    assert_eq!(ft.strobe_dmx_offset(), Some(7));
    assert_eq!(ft.min_strobe_frequency(), Some(0.4));
    assert_eq!(ft.max_strobe_frequency(), Some(25.0));
}

#[test]
#[ignore = "bring-your-own corpus: put .gdtf files in tests/gdtf-corpus/ and run with --ignored"]
fn every_corpus_file_parses_and_every_mode_distills_or_refuses() {
    for path in corpus_files() {
        let bytes = std::fs::read(&path).expect("readable gdtf");
        let description = gdtf::parse_archive(&bytes)
            .unwrap_or_else(|e| panic!("{}: failed to parse: {e}", path.display()));
        let summaries = gdtf::mode_summaries(&description);
        assert!(
            !summaries.is_empty(),
            "{}: no DMX modes found",
            path.display()
        );
        println!(
            "{}: \"{}\" by {}, {} modes",
            path.display(),
            description.name,
            description.manufacturer,
            summaries.len()
        );

        for summary in &summaries {
            match gdtf::distill(&description, &summary.name, &description.name) {
                Ok(distilled) => {
                    assert!(
                        !distilled.fixture_type.channels().is_empty(),
                        "{}: mode {:?} distilled to zero channels",
                        path.display(),
                        summary.name
                    );
                    // A skipped channel (no logical channel) still counts in
                    // the summary's raw footprint, so distillation can only
                    // shrink it, never exceed it.
                    assert!(
                        distilled.fixture_type.footprint() <= summary.footprint,
                        "{}: mode {:?} distilled footprint {} exceeds the summary's {}",
                        path.display(),
                        summary.name,
                        distilled.fixture_type.footprint(),
                        summary.footprint
                    );
                    for warning in &distilled.warnings {
                        println!("  [{}] {warning}", summary.name);
                    }
                }
                Err(e) => {
                    // The only acceptable refusal is the multi-instance one;
                    // anything else is a parser/distiller bug to look at.
                    let message = e.to_string();
                    assert!(
                        message.contains("multi-instance"),
                        "{}: mode {:?} failed for a non-instancing reason: {message}",
                        path.display(),
                        summary.name
                    );
                    println!("  [{}] refused: multi-instance", summary.name);
                }
            }
        }
    }
}
