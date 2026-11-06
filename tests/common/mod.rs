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

//! Shared by the corpus tests. Each test crate uses only some of it.
#![allow(dead_code)]

use std::path::PathBuf;

/// How a corpus test takes a missing or empty corpus.
pub enum Listing {
    /// The corpus must be there and hold at least one file.
    NonEmpty,
    /// The corpus must be there; it may be empty.
    MayBeEmpty,
    /// A missing corpus, or an entry that cannot be read, is no file.
    Lenient,
}

/// The `.ext` files in `tests/<dir>`, sorted.
pub fn corpus_files(dir: &str, ext: &str, listing: Listing) -> Vec<PathBuf> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join(dir);
    let mut files: Vec<PathBuf> = match listing {
        Listing::Lenient => std::fs::read_dir(&dir)
            .map(|entries| {
                entries
                    .filter_map(|e| e.ok().map(|e| e.path()))
                    .filter(|p| p.extension().is_some_and(|x| x == ext))
                    .collect()
            })
            .unwrap_or_default(),
        Listing::NonEmpty | Listing::MayBeEmpty => {
            assert!(
                dir.is_dir(),
                "no corpus at {}: create it and drop .{ext} files in (see module docs)",
                dir.display()
            );
            std::fs::read_dir(&dir)
                .expect("readable corpus dir")
                .filter_map(|entry| {
                    let path = entry.expect("dir entry").path();
                    (path.extension().is_some_and(|x| x == ext)).then_some(path)
                })
                .collect()
        }
    };
    files.sort();
    if matches!(listing, Listing::NonEmpty) {
        assert!(
            !files.is_empty(),
            "corpus dir {} holds no .{ext} files",
            dir.display()
        );
    }
    files
}
