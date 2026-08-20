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
//! CI can fetch. Drop `.mvr` files into `tests/mvr-corpus/` (gitignored)
//! and run:
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
            // Console exports sometimes omit the extension in the reference.
            let entry = embedded
                .iter()
                .find(|name| {
                    name.as_str() == spec
                        || name.strip_suffix(".gdtf") == Some(spec)
                        || name.rsplit('/').next() == Some(spec)
                })
                .cloned();
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
