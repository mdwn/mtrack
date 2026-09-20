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

//! Bring-your-own corpus report for the strict GDTF reading (design §19.5):
//! every GDTF embedded in `tests/mvr-corpus/*.mvr` (gitignored) is run
//! through `gdtf::strict::check`, and the problems are printed per file.
//! Nothing is asserted about manufacturer files — they are facts, and
//! the wild is looser than the spec — but the run shows which rules real
//! files keep, which is how the checker itself is kept honest.
//!
//! ```sh
//! cargo test --test gdtf_strict_corpus -- --ignored --nocapture
//! ```

use std::collections::{BTreeMap, HashSet};

use mtrack::lighting::gdtf;
use mtrack::lighting::mvr;

#[test]
#[ignore = "bring-your-own corpus: put .mvr files in tests/mvr-corpus/ and run with --ignored"]
fn every_corpus_gdtf_is_read_strictly() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/mvr-corpus");
    assert!(dir.is_dir(), "no corpus at {}", dir.display());
    let mut files: Vec<_> = std::fs::read_dir(&dir)
        .expect("readable corpus dir")
        .filter_map(|e| {
            let p = e.expect("entry").path();
            (p.extension().is_some_and(|x| x == "mvr")).then_some(p)
        })
        .collect();
    files.sort();

    let mut seen: HashSet<String> = HashSet::new();
    let mut clean = 0usize;
    let mut by_rule: BTreeMap<String, (usize, String)> = BTreeMap::new();
    let mut total = 0usize;
    for path in files {
        let bytes = std::fs::read(&path).expect("readable mvr");
        for entry in mvr::list_gdtf_entries(&bytes).expect("gdtf entries") {
            if !seen.insert(entry.clone()) {
                continue;
            }
            let Ok(gdtf_bytes) = mvr::read_gdtf_entry(&bytes, &entry) else {
                continue;
            };
            let Ok(xml) = gdtf::read_description_xml(&gdtf_bytes) else {
                continue;
            };
            total += 1;
            let problems = gdtf::strict::check(&xml);
            if problems.is_empty() {
                clean += 1;
            } else {
                println!("{entry}: {} problem(s)", problems.len());
                for p in &problems {
                    // Group by the rule, not the instance: quoted names
                    // and numbers become placeholders.
                    let mut rule = String::new();
                    let mut in_quote = false;
                    for c in p.chars() {
                        match c {
                            '"' if in_quote => {
                                in_quote = false;
                                rule.push('_');
                            }
                            '"' => in_quote = true,
                            _ if in_quote => {}
                            c if c.is_ascii_digit() => rule.push('#'),
                            c => rule.push(c),
                        }
                    }
                    let entry_rule = by_rule.entry(rule).or_insert((0, p.clone()));
                    entry_rule.0 += 1;
                }
            }
        }
    }
    println!("{clean} of {total} archives pass every strict rule");
    for (rule, (n, example)) in &by_rule {
        println!("  {n:4}  {rule}\n        e.g. {example}");
    }
    assert!(total > 0, "the corpus holds no GDTFs");
}
