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

//! The project's fixture types and venues, read one way for everyone.
//!
//! The lighting system walked its directories recursively while the web API
//! read only their top level, so a type in a subdirectory was a fixture to
//! the engine and nothing at all to the Fixtures page. Every caller now asks
//! here: [`fixture_types`] for the type files (records and hand-written
//! types) plus the library's unrecorded archives, [`venues`] for the venue
//! files. Files are read recursively in path order; when two files declare
//! one name, the first keeps it and the clash is reported against both.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use super::library::Library;
use super::types::{FixtureType, Venue};

/// Fixture type files: `.fixture` (rich channels, GDTF records) and the v1
/// `.light`, read as peers.
pub const FIXTURE_TYPE_EXTENSIONS: &[&str] = &["light", "fixture"];

/// Venue files: `.venue` and the v1 `.light`, read as peers.
pub const VENUE_EXTENSIONS: &[&str] = &["light", "venue"];

/// Every file under `dir`, at any depth, with one of `extensions`, in path
/// order. A missing directory has none.
pub fn files_under(dir: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
            if path.is_dir() {
                stack.push(path);
            } else if path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| extensions.contains(&e))
            {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

/// A file that could not be read or parsed, or a name two files declare.
#[derive(Clone, Debug, PartialEq)]
pub struct FileProblem {
    pub file: PathBuf,
    pub error: String,
}

/// One declaration and the file it is in.
#[derive(Clone, Debug)]
pub struct Declared<T> {
    pub name: String,
    pub file: PathBuf,
    pub item: T,
}

/// What a directory of files declares.
#[derive(Clone, Debug)]
pub struct Declarations<T> {
    /// In file order; a name appears once (its first file's).
    pub items: Vec<Declared<T>>,
    /// Files that would not read or parse, then names declared twice.
    pub problems: Vec<FileProblem>,
}

impl<T> Default for Declarations<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            problems: Vec::new(),
        }
    }
}

impl<T> Declarations<T> {
    /// The declaration of `name`, if any file has one.
    pub fn get(&self, name: &str) -> Option<&Declared<T>> {
        self.items.iter().find(|d| d.name == name)
    }

    /// The names every file declares, including the one the duplicate
    /// rule set aside — what a file holds, for a caller rewriting it whole.
    pub fn names_in(&self, file: &Path) -> Vec<String> {
        self.items
            .iter()
            .filter(|d| d.file == file)
            .map(|d| d.name.clone())
            .collect()
    }
}

/// How a file is shown in a problem: relative to the directory read, so a
/// file in a subdirectory says which.
pub fn display_in(dir: &Path, file: &Path) -> String {
    file.strip_prefix(dir)
        .unwrap_or(file)
        .to_string_lossy()
        .into_owned()
}

fn declarations<T>(
    dir: &Path,
    extensions: &[&str],
    kind: &str,
    parse: impl Fn(&str) -> Result<HashMap<String, T>, Box<dyn std::error::Error>>,
) -> Declarations<T> {
    let mut out = Declarations::default();
    let mut duplicates = Vec::new();
    let mut taken: HashMap<String, PathBuf> = HashMap::new();
    for file in files_under(dir, extensions) {
        let parsed = std::fs::read_to_string(&file)
            .map_err(|e| e.to_string())
            .and_then(|text| parse(&text).map_err(|e| e.to_string()));
        let declared = match parsed {
            Ok(declared) => declared,
            Err(error) => {
                out.problems.push(FileProblem { file, error });
                continue;
            }
        };
        // A file's own declarations in name order, so the result does not
        // depend on hash order.
        let declared: BTreeMap<String, T> = declared.into_iter().collect();
        for (name, item) in declared {
            if let Some(first) = taken.get(&name) {
                duplicates.push(FileProblem {
                    file: file.clone(),
                    error: format!(
                        "{kind} \"{name}\" is defined in both {} and {}",
                        display_in(dir, first),
                        display_in(dir, &file)
                    ),
                });
                continue;
            }
            taken.insert(name.clone(), file.clone());
            out.items.push(Declared {
                name,
                file: file.clone(),
                item,
            });
        }
    }
    out.problems.extend(duplicates);
    out
}

/// The fixture type files under `dir`, recursively.
pub fn type_files(dir: &Path) -> Declarations<FixtureType> {
    declarations(dir, FIXTURE_TYPE_EXTENSIONS, "fixture type", |text| {
        super::parser::parse_fixture_types(text)
    })
}

/// The venue files under `dir`, recursively.
pub fn venues(dir: &Path) -> Declarations<Venue> {
    declarations(dir, VENUE_EXTENSIONS, "venue", |text| {
        super::parser::parse_venues(text)
    })
}

/// The project's fixture types: every type file under `types_dir` and every
/// GDTF in the library no record points at.
#[derive(Clone, Debug, Default)]
pub struct ProjectTypes {
    pub files: Declarations<FixtureType>,
    pub library: Library,
}

/// Reads [`ProjectTypes`].
pub fn fixture_types(project: &Path, types_dir: Option<&Path>) -> ProjectTypes {
    let files = types_dir.map(type_files).unwrap_or_default();
    let names: HashSet<String> = files.items.iter().map(|d| d.name.clone()).collect();
    // A record set aside as a duplicate still points at its archive.
    let mut archives: HashSet<PathBuf> = HashSet::new();
    for file in types_dir
        .map(|dir| files_under(dir, FIXTURE_TYPE_EXTENSIONS))
        .unwrap_or_default()
    {
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(types) = super::parser::parse_fixture_types(&text) else {
            continue;
        };
        for fixture_type in types.values() {
            if let Some(source) = fixture_type.source() {
                if let Ok(canonical) = project.join(&source.path).canonicalize() {
                    archives.insert(canonical);
                }
            }
        }
    }
    let library = super::library::unrecorded_types_given(project, &names, &archives);
    ProjectTypes { files, library }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAR: &str =
        "fixture_type \"Par\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n";

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn a_type_in_a_subdirectory_is_read_and_a_duplicate_is_reported_against_both() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "a.light", PAR);
        write(dir.path(), "rig/b.light", &PAR.replace("Par", "Wash"));
        write(dir.path(), "rig/old/par.light", PAR);
        write(dir.path(), "rig/broken.light", "fixture_type {");
        let read = type_files(dir.path());
        let names: Vec<&str> = read.items.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["Par", "Wash"]);
        assert_eq!(
            read.get("Wash").unwrap().file,
            dir.path().join("rig/b.light")
        );
        assert_eq!(read.get("Par").unwrap().file, dir.path().join("a.light"));
        let errors: Vec<&str> = read.problems.iter().map(|p| p.error.as_str()).collect();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors[1].contains("defined in both a.light and rig/old/par.light"));
    }

    #[test]
    fn venues_are_read_recursively_too() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "house.light", "venue \"house\" {\n}\n");
        write(dir.path(), "tour/club.venue", "venue \"club\" {\n}\n");
        let read = venues(dir.path());
        let names: Vec<&str> = read.items.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["house", "club"]);
    }
}
