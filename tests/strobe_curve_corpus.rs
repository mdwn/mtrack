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

//! What the strobe curve's automatic rule chooses for every strobe function
//! in the bring-your-own corpora (`tests/gdtf-corpus/*.gdtf` and the GDTFs
//! embedded in `tests/mvr-corpus/*.mvr`). A GDTF fixture with no
//! `strobe_curve` in its record follows its file (`declared`): the
//! function's own table when it has one, linear in hertz between the
//! endpoints when it has none. Printed for eyeballing:
//!
//! ```sh
//! cargo test --test strobe_curve_corpus -- --ignored --nocapture
//! ```

use std::collections::BTreeMap;

use mtrack::lighting::{gdtf, mvr};

/// Every GDTF of both corpora, by (manufacturer, fixture name), first seen.
fn corpus_gdtfs() -> BTreeMap<(String, String), gdtf::Description> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests");
    let files = |dir: &str, ext: &str| -> Vec<std::path::PathBuf> {
        let mut files: Vec<_> = std::fs::read_dir(root.join(dir))
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|e| e == ext))
                    .collect()
            })
            .unwrap_or_default();
        files.sort();
        files
    };
    let mut out = BTreeMap::new();
    let mut add = |bytes: &[u8]| {
        if let Ok(description) = gdtf::parse_archive(bytes) {
            out.entry((description.manufacturer.clone(), description.name.clone()))
                .or_insert(description);
        }
    };
    for path in files("gdtf-corpus", "gdtf") {
        add(&std::fs::read(&path).expect("readable gdtf"));
    }
    for path in files("mvr-corpus", "mvr") {
        let bytes = std::fs::read(&path).expect("readable mvr");
        for entry in mvr::list_gdtf_entries(&bytes).expect("entry listing") {
            if let Ok(gdtf_bytes) = mvr::read_gdtf_entry(&bytes, &entry) {
                add(&gdtf_bytes);
            }
        }
    }
    out
}

#[test]
#[ignore = "bring-your-own corpus: tests/gdtf-corpus/ and tests/mvr-corpus/, run with --ignored"]
fn what_the_automatic_strobe_curve_chooses_across_the_corpus() {
    let gdtfs = corpus_gdtfs();
    assert!(
        !gdtfs.is_empty(),
        "no corpus: put .gdtf files in tests/gdtf-corpus/ or .mvr files in tests/mvr-corpus/"
    );
    println!("| fixture | strobe functions | Hz | steps | automatic curve |");
    println!("|---|---|---|---|---|");
    let (mut tabled, mut endpoints, mut none) = (0, 0, 0);
    for ((manufacturer, name), description) in &gdtfs {
        let tables = gdtf::strobe_tables(description);
        if tables.is_empty() {
            none += 1;
            println!("| {manufacturer} {name} | 0 | | | no strobe rate |");
            continue;
        }
        let mut steps: Vec<usize> = tables.iter().map(|t| t.steps).collect();
        steps.sort();
        steps.dedup();
        let mut hz: Vec<String> = tables
            .iter()
            .map(|t| format!("{}–{}", t.from_hz, t.to_hz))
            .collect();
        hz.sort();
        hz.dedup();
        let most = *steps.last().expect("non-empty");
        let choice = if most > 0 {
            tabled += 1;
            format!("declared: its {most}-step table")
        } else {
            endpoints += 1;
            "declared: linear between the endpoints".to_string()
        };
        println!(
            "| {manufacturer} {name} | {} | {} | {} | {choice} |",
            tables.len(),
            hz.join(", "),
            steps
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    println!(
        "\n{} fixtures: {tabled} with a table, {endpoints} with endpoints only, {none} with no \
         strobe rate",
        gdtfs.len()
    );
}
