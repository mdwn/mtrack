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

//! A GDTF in the library is a fixture (venue-exchange design §22.2).
//!
//! Every `.gdtf` in `<project>/lighting/library/` that no fixture type file
//! points at is a fixture type in its own right: named from the archive's
//! own fixture name (the importer's rule), with no movement limits. Nothing
//! is written beside it; a `.fixture` record exists only for what the GDTF
//! does not say (another name, a mover's speed limits). Every venue fixture
//! of it states its own mode.
//!
//! [`unrecorded_types`] is the one place those types are worked out, through
//! the project's one reader of type files ([`super::project_files`]); the
//! lighting system's loader and every web endpoint that lists, opens or
//! deletes a fixture type ask it, so the engine and the API cannot disagree
//! about what the library holds.
//!
//! Parsing stays at the boundary (§3): an archive's name is read once per
//! content and kept in `lighting/.cache/`; while an archive's length and
//! modification time are unchanged it is not opened again.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use super::gdtf;

/// The library, relative to the project. A profile's fixture types
/// directory override does not move it.
pub const LIBRARY_DIR: &str = "lighting/library";

/// The name index's file in the project's cache, and its format version.
const INDEX_FILE: &str = "lighting/.cache/library-names-v1.json";
const INDEX_VERSION: u32 = 1;

/// A fixture type that is an archive alone.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LibraryType {
    /// The type's name: the archive's fixture name made a type name, with
    /// the archive's file stem added on a collision.
    pub name: String,
    /// The archive, project-relative (`lighting/library/<file>`).
    pub archive: String,
    /// The archive's file name.
    pub file_name: String,
    /// The archive's own fixture name and manufacturer.
    pub fixture: String,
    pub manufacturer: String,
    /// The name it would have had, when another fixture already has it.
    pub renamed_from: Option<String>,
}

/// Something about the library a person should know: two archives stating
/// one name (`gdtf-name-collision`), or an archive that is not a readable
/// GDTF (`gdtf-unreadable`). Neither stops the other fixtures.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LibraryFinding {
    pub kind: &'static str,
    /// The archive's file name.
    pub file: String,
    pub message: String,
}

/// What [`unrecorded_types`] found.
#[derive(Clone, Debug, Default)]
pub struct Library {
    /// By name order of the archive files.
    pub types: Vec<LibraryType>,
    pub findings: Vec<LibraryFinding>,
}

impl Library {
    /// The type of this name, if the library has one.
    pub fn get(&self, name: &str) -> Option<&LibraryType> {
        self.types.iter().find(|t| t.name == name)
    }
}

/// The name index: per file, what it was when last read; per content hash,
/// the names it states.
#[derive(Default, Deserialize, Serialize)]
struct Index {
    version: u32,
    files: BTreeMap<String, Seen>,
    names: BTreeMap<String, Names>,
}

#[derive(Clone, Deserialize, Serialize, PartialEq)]
struct Seen {
    len: u64,
    modified_ns: Option<u128>,
    sha256: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct Names {
    fixture: String,
    manufacturer: String,
}

fn modified_ns(meta: &std::fs::Metadata) -> Option<u128> {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
}

fn load_index(project: &Path) -> Index {
    std::fs::read(project.join(INDEX_FILE))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Index>(&bytes).ok())
        .filter(|index| index.version == INDEX_VERSION)
        .unwrap_or_default()
}

fn save_index(project: &Path, index: &Index) {
    let path = project.join(INDEX_FILE);
    let write = || -> Result<(), Box<dyn std::error::Error>> {
        let dir = path.parent().expect("index path has a directory");
        std::fs::create_dir_all(dir)?;
        let mut tmp = tempfile::NamedTempFile::new_in(dir)?;
        std::io::Write::write_all(&mut tmp, &serde_json::to_vec_pretty(index)?)?;
        tmp.persist(&path)?;
        Ok(())
    };
    // The index is a cache: failing to write it costs a parse next load.
    if let Err(e) = write() {
        warn!(error = %e, "Could not write the GDTF library's name index");
    }
}

/// The fixture name and manufacturer an archive states, from the index
/// while the file is unchanged, else read (once, loudly) and indexed.
fn names_of(
    path: &Path,
    file_name: &str,
    index: &mut Index,
    dirty: &mut bool,
) -> Result<Names, String> {
    let meta = std::fs::metadata(path).map_err(|e| format!("cannot read it: {e}"))?;
    let stat = (meta.len(), modified_ns(&meta));
    if let Some(seen) = index.files.get(file_name) {
        if (seen.len, seen.modified_ns) == stat {
            if let Some(names) = index.names.get(&seen.sha256) {
                return Ok(names.clone());
            }
        }
    }
    info!(
        archive = file_name,
        "Reading a GDTF in the library for its fixture name"
    );
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read it: {e}"))?;
    let sha256 = format!("{:x}", Sha256::digest(&bytes));
    let names = match index.names.get(&sha256) {
        Some(names) => names.clone(),
        None => {
            let description =
                gdtf::parse_archive(&bytes).map_err(|e| format!("not a readable GDTF: {e}"))?;
            if description.modes.is_empty() {
                return Err("the GDTF has no DMX modes".to_string());
            }
            let names = Names {
                fixture: description.name,
                manufacturer: description.manufacturer,
            };
            index.names.insert(sha256.clone(), names.clone());
            names
        }
    };
    index.files.insert(
        file_name.to_string(),
        Seen {
            len: stat.0,
            modified_ns: stat.1,
            sha256,
        },
    );
    *dirty = true;
    Ok(names)
}

/// The fixture types the library's unrecorded archives are: every `.gdtf`
/// in `lighting/library/` no type file in `types_dir` points at, by file
/// name order. A name is the archive's fixture name made a type name; when
/// a type file or an earlier archive has it, the archive takes
/// `Name (stem)` (an ordinal after that if even that is taken), and two
/// archives stating one name are reported. An archive that is not a
/// readable GDTF is reported and skipped.
pub fn unrecorded_types(project: &Path, types_dir: Option<&Path>) -> Library {
    super::project_files::fixture_types(project, types_dir).library
}

/// [`unrecorded_types`], given what the type files declare: the names they
/// take and the archives (canonical) they point at. The project's one
/// reader of type files ([`super::project_files::fixture_types`]) calls
/// this, so the library and the type files are read the same way.
pub fn unrecorded_types_given(
    project: &Path,
    record_names: &HashSet<String>,
    recorded: &HashSet<PathBuf>,
) -> Library {
    let library_dir = project.join(LIBRARY_DIR);
    let Ok(entries) = std::fs::read_dir(&library_dir) else {
        return Library::default();
    };
    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("gdtf"))
        })
        .collect();
    files.sort();
    if files.is_empty() {
        return Library::default();
    }

    let mut index = load_index(project);
    index.version = INDEX_VERSION;
    let mut dirty = false;
    let mut library = Library::default();
    // Name → the file that took it, for the collision report.
    let mut taken_by: BTreeMap<String, String> = BTreeMap::new();
    for path in files {
        if path
            .canonicalize()
            .is_ok_and(|canonical| recorded.contains(&canonical))
        {
            continue;
        }
        let file_name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let names = match names_of(&path, &file_name, &mut index, &mut dirty) {
            Ok(names) => names,
            Err(reason) => {
                warn!(
                    archive = file_name.as_str(),
                    "A file in the GDTF library is not a fixture"
                );
                library.findings.push(LibraryFinding {
                    kind: "gdtf-unreadable",
                    message: format!("{file_name} in the library is not a fixture: {reason}"),
                    file: file_name,
                });
                continue;
            }
        };
        let base = super::import::suggested_type_name(&names.fixture);
        let is_taken = |name: &str, taken_by: &BTreeMap<String, String>| {
            record_names.contains(name) || taken_by.contains_key(name)
        };
        let (name, renamed_from) = if !is_taken(&base, &taken_by) {
            (base, None)
        } else {
            let stem = super::import::suggested_type_name(
                path.file_stem().and_then(|s| s.to_str()).unwrap_or("gdtf"),
            );
            let with_stem = format!("{base} ({stem})");
            let mut candidate = with_stem.clone();
            let mut n = 2;
            while is_taken(&candidate, &taken_by) {
                candidate = format!("{with_stem} #{n}");
                n += 1;
            }
            if let Some(first) = taken_by.get(&base) {
                warn!(
                    archive = file_name.as_str(),
                    other = first.as_str(),
                    "Two GDTFs in the library state the same fixture name; the second \
                     takes its file name too"
                );
                library.findings.push(LibraryFinding {
                    kind: "gdtf-name-collision",
                    message: format!(
                        "{first} and {file_name} are both \"{base}\": {first} keeps the name \
                         and {file_name} is \"{candidate}\". Rename either on its page to \
                         choose."
                    ),
                    file: file_name.clone(),
                });
            }
            (candidate, Some(base))
        };
        taken_by.insert(name.clone(), file_name.clone());
        library.types.push(LibraryType {
            name,
            archive: format!("{LIBRARY_DIR}/{file_name}"),
            file_name,
            fixture: names.fixture,
            manufacturer: names.manufacturer,
            renamed_from,
        });
    }
    if dirty {
        save_index(project, &index);
    }
    library
}

/// The declared form of a library type: a GDTF-sourced fixture type with no
/// movement limits, as a record that said only `from gdtf("…")` would
/// declare it.
pub fn declared(library_type: &LibraryType) -> super::types::FixtureType {
    let mut fixture_type =
        super::types::FixtureType::new(library_type.name.clone(), std::collections::HashMap::new());
    fixture_type.set_source(super::types::GdtfSource {
        path: library_type.archive.clone(),
    });
    fixture_type
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gdtf_named(name: &str) -> Vec<u8> {
        gdtf::build_zip(&[(
            "description.xml",
            gdtf::SYNTHETIC_DESCRIPTION
                .replace("Synth Brick", name)
                .as_bytes(),
        )])
    }

    fn project_with(archives: &[(&str, Vec<u8>)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        let library = dir.path().join(LIBRARY_DIR);
        std::fs::create_dir_all(&library).unwrap();
        std::fs::create_dir_all(dir.path().join("lighting/fixture_types")).unwrap();
        for (file, bytes) in archives {
            std::fs::write(library.join(file), bytes).unwrap();
        }
        dir
    }

    fn types_dir(dir: &tempfile::TempDir) -> PathBuf {
        dir.path().join("lighting/fixture_types")
    }

    #[test]
    fn an_archive_alone_is_a_fixture_named_from_its_gdtf() {
        let dir = project_with(&[("pb15.gdtf", gdtf_named("PB15 &lt;Pixel&gt; Brick"))]);
        let library = unrecorded_types(dir.path(), Some(&types_dir(&dir)));
        assert_eq!(
            library.types,
            vec![LibraryType {
                name: "PB15 Pixel Brick".to_string(),
                archive: "lighting/library/pb15.gdtf".to_string(),
                file_name: "pb15.gdtf".to_string(),
                fixture: "PB15 <Pixel> Brick".to_string(),
                manufacturer: "mtrack synthetic".to_string(),
                renamed_from: None,
            }]
        );
        assert!(library.findings.is_empty());
        let declared = declared(&library.types[0]);
        assert_eq!(
            declared.source().unwrap().path,
            "lighting/library/pb15.gdtf"
        );
    }

    #[test]
    fn a_recorded_archive_is_not_a_second_fixture() {
        let dir = project_with(&[("pb15.gdtf", gdtf_named("Brick"))]);
        std::fs::write(
            types_dir(&dir).join("astera_pixelbrick.fixture"),
            "fixture_type \"Astera-PixelBrick\"\n  from gdtf(\"lighting/library/pb15.gdtf\")\n{\n}\n",
        )
        .unwrap();
        let library = unrecorded_types(dir.path(), Some(&types_dir(&dir)));
        assert!(library.types.is_empty(), "{:?}", library.types);
    }

    #[test]
    fn two_archives_of_one_name_the_first_file_keeps_it_and_both_are_reported() {
        let dir = project_with(&[
            ("b-brick.gdtf", gdtf_named("Brick")),
            ("a-brick.gdtf", gdtf_named("Brick")),
        ]);
        let library = unrecorded_types(dir.path(), Some(&types_dir(&dir)));
        let names: Vec<(&str, &str)> = library
            .types
            .iter()
            .map(|t| (t.file_name.as_str(), t.name.as_str()))
            .collect();
        assert_eq!(
            names,
            vec![
                ("a-brick.gdtf", "Brick"),
                ("b-brick.gdtf", "Brick (b-brick)")
            ]
        );
        assert_eq!(library.findings.len(), 1);
        assert_eq!(library.findings[0].kind, "gdtf-name-collision");
        assert!(library.findings[0].message.contains("a-brick.gdtf"));
        assert!(library.findings[0].message.contains("b-brick.gdtf"));

        // A third file added later that sorts first takes the name: the
        // rule is the file order, every load the same.
        std::fs::write(
            dir.path().join(LIBRARY_DIR).join("0-brick.gdtf"),
            gdtf_named("Brick"),
        )
        .unwrap();
        let library = unrecorded_types(dir.path(), Some(&types_dir(&dir)));
        let names: Vec<&str> = library.types.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["Brick", "Brick (a-brick)", "Brick (b-brick)"]);
        assert_eq!(library.findings.len(), 2);
    }

    #[test]
    fn a_name_a_record_or_hand_written_type_has_takes_the_archive_stem() {
        let dir = project_with(&[("brick.gdtf", gdtf_named("Brick"))]);
        std::fs::write(
            types_dir(&dir).join("brick.light"),
            "fixture_type \"Brick\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n",
        )
        .unwrap();
        let library = unrecorded_types(dir.path(), Some(&types_dir(&dir)));
        assert_eq!(library.types[0].name, "Brick (brick)");
        assert_eq!(library.types[0].renamed_from.as_deref(), Some("Brick"));
        assert!(
            library.findings.is_empty(),
            "a hand-written type is not a library collision"
        );
    }

    #[test]
    fn an_unreadable_archive_is_reported_and_stops_nothing() {
        let dir = project_with(&[
            ("broken.gdtf", b"not a zip".to_vec()),
            ("good.gdtf", gdtf_named("Good")),
        ]);
        let library = unrecorded_types(dir.path(), Some(&types_dir(&dir)));
        assert_eq!(library.types.len(), 1);
        assert_eq!(library.types[0].name, "Good");
        assert_eq!(library.findings.len(), 1);
        assert_eq!(library.findings[0].kind, "gdtf-unreadable");
        assert_eq!(library.findings[0].file, "broken.gdtf");
    }

    #[cfg(unix)]
    #[test]
    fn a_warm_index_names_an_archive_without_opening_it() {
        use std::os::unix::fs::PermissionsExt;
        let dir = project_with(&[("pb15.gdtf", gdtf_named("Brick"))]);
        // Cold: read and indexed.
        let cold = unrecorded_types(dir.path(), Some(&types_dir(&dir)));
        assert_eq!(cold.types[0].name, "Brick");
        assert!(dir.path().join(INDEX_FILE).is_file());

        // Unreadable now: a warm index must not need to open it.
        let archive = dir.path().join(LIBRARY_DIR).join("pb15.gdtf");
        std::fs::set_permissions(&archive, std::fs::Permissions::from_mode(0o000)).unwrap();
        let readable = std::fs::read(&archive).is_ok(); // root ignores modes
        let warm = unrecorded_types(dir.path(), Some(&types_dir(&dir)));
        std::fs::set_permissions(&archive, std::fs::Permissions::from_mode(0o644)).unwrap();
        if !readable {
            assert_eq!(warm.types.len(), 1, "{:?}", warm.findings);
            assert_eq!(warm.types[0].name, "Brick");
        }
    }

    #[test]
    fn a_changed_archive_is_read_again() {
        let dir = project_with(&[("pb15.gdtf", gdtf_named("Brick"))]);
        unrecorded_types(dir.path(), Some(&types_dir(&dir)));
        // Different content, different length: the index entry is stale.
        std::fs::write(
            dir.path().join(LIBRARY_DIR).join("pb15.gdtf"),
            gdtf_named("A Much Longer Brick Name"),
        )
        .unwrap();
        let library = unrecorded_types(dir.path(), Some(&types_dir(&dir)));
        assert_eq!(library.types[0].name, "A Much Longer Brick Name");
    }
}
