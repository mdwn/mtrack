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

use tracing::warn;

use super::library::Library;
use super::types::{FixtureType, Venue};

/// Fixture type files: `.fixture` (rich channels, GDTF records) and the v1
/// `.light`, read as peers. Where both forms of one stem could be meant, the
/// first is preferred.
pub const FIXTURE_TYPE_EXTENSIONS: &[&str] = &["fixture", "light"];

/// Venue files: `.venue` and the v1 `.light`, read as peers. Where both
/// forms of one stem could be meant, the first is preferred.
pub const VENUE_EXTENSIONS: &[&str] = &["venue", "light"];

/// Every file under `dir`, at any depth, with one of `extensions` (lowercase,
/// matched in any case: `songs::has_extension`), in path order: the one walk
/// for a project's lighting files (the venue and fixture type directories,
/// and the songs' `.light` shows).
///
/// - Hidden directories are left out, as the song loader leaves them out
///   (`songs::is_hidden`).
/// - A link is followed only to what is inside `dir`, by the web UI's
///   containment rule (`SafePath::resolve`): one leading out of it, or to
///   nothing, is left out, so nothing listed here is a file the tools that
///   read it would refuse.
/// - A missing `dir` has none; one that cannot be read is an error, as the
///   song loader's is. A subdirectory that cannot be read is passed over,
///   with a warning.
pub fn files_under(dir: &Path, extensions: &[&str]) -> std::io::Result<Vec<PathBuf>> {
    use crate::webui::safe_path::{SafePath, VerifiedRoot};
    // Not `exists`, which says no to a directory it cannot reach: only one
    // that is not there has no files.
    if !dir.try_exists()? {
        return Ok(Vec::new());
    }
    let entries = std::fs::read_dir(dir)?;
    let root = VerifiedRoot::new(dir).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "the directory cannot be resolved",
        )
    })?;
    let inside = |path: &Path| {
        let is_link = path
            .symlink_metadata()
            .is_ok_and(|m| m.file_type().is_symlink());
        !is_link || SafePath::resolve(path, &root).is_ok()
    };
    let mut out = Vec::new();
    let mut pending = vec![entries];
    while let Some(entries) = pending.pop() {
        for path in entries.filter_map(|e| e.ok().map(|e| e.path())) {
            if !inside(&path) {
                warn!(link = %path.display(), "A link among the lighting files leads out of their directory, or to nothing; passing over it");
                continue;
            }
            if path.is_dir() {
                if crate::songs::is_hidden(&path) {
                    continue;
                }
                match std::fs::read_dir(&path) {
                    Ok(entries) => pending.push(entries),
                    Err(e) => {
                        warn!(dir = %path.display(), error = %e, "Cannot read a directory of lighting files; passing over it")
                    }
                }
            } else if crate::songs::has_extension(&path, extensions) {
                out.push(path);
            }
        }
    }
    out.sort();
    Ok(out)
}

/// The stem a venue or fixture type file takes for a name, as a new file is
/// named and an existing one is found at it: lowercase, with spaces and
/// anything but letters, digits, `_` and `-` as `_` (`Club-2` is `club-2`).
pub fn name_stem(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            ' ' => '_',
            c if c.is_alphanumeric() || c == '_' || c == '-' => c,
            _ => '_',
        })
        .collect::<String>()
        .to_lowercase()
}

/// The venue file at `name`'s stem ([`name_stem`]) directly in `dir`, in
/// any case (`util::files_at_stem`), `.venue` before `.light`: the file a
/// save of the venue would write when no file declares it.
pub fn venue_file_at_stem(dir: &Path, name: &str) -> Option<PathBuf> {
    crate::util::files_at_stem(dir, &name_stem(name), VENUE_EXTENSIONS)
        .into_iter()
        .next()
}

/// What a lighting file declares, or why it does not read.
pub type Declares = Result<Vec<String>, String>;

/// The fixture types the file at `path` declares ([`Declares`]).
pub fn fixture_types_declared(path: &Path) -> Declares {
    let text = crate::util::read_to_string(path).map_err(|e| e.to_string())?;
    super::parser::parse_fixture_types(&text)
        .map(|types| types.into_keys().collect())
        .map_err(|e| e.to_string())
}

/// The venues the file at `path` declares ([`Declares`]).
pub fn venues_declared(path: &Path) -> Declares {
    let bytes = crate::util::read_file(path).map_err(|e| e.to_string())?;
    let text = std::str::from_utf8(&bytes).map_err(|_| "not valid UTF-8".to_string())?;
    super::parser::parse_venues(text)
        .map(|venues| venues.into_keys().collect())
        .map_err(|e| e.to_string())
}

/// Why a save or delete of a name may not act on a file ([`may_act_on`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    /// The file's name.
    pub file: String,
    /// What is wrong with it: `it holds "Other"`, `it does not read: …`.
    pub why: String,
    /// Its parse error, when it does not read: the one case a person may
    /// choose to replace it.
    pub parse_error: Option<String>,
}

/// The one rule for a venue or fixture type file that a save or delete of
/// `name` would act on (write over, rewrite or remove), wherever it is
/// asked: the web UI's saves and deletes, and the MVR import. Only a file
/// that declares `name`; one that holds only other names is refused, and so
/// is one that does not read, unless `replace_broken` names exactly that
/// file (a person who was shown its parse error and chose to replace it).
pub fn may_act_on(
    name: &str,
    file: &Path,
    declares: &Declares,
    replace_broken: Option<&str>,
) -> Result<(), Refusal> {
    let shown = crate::util::filename_display(file).to_string();
    let (why, parse_error) = match declares {
        Ok(names) if names.iter().any(|n| n == name) => return Ok(()),
        Ok(names) if names.is_empty() => ("it defines nothing".to_string(), None),
        Ok(names) => {
            let mut names: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
            names.sort();
            (format!("it holds {}", names.join(", ")), None)
        }
        Err(_) if replace_broken == Some(shown.as_str()) => return Ok(()),
        Err(e) => (format!("it does not read: {e}"), Some(e.clone())),
    };
    Err(Refusal {
        file: shown,
        why,
        parse_error,
    })
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
    let files = match files_under(dir, extensions) {
        Ok(files) => files,
        Err(e) => {
            out.problems.push(FileProblem {
                file: dir.to_path_buf(),
                error: format!("cannot read the directory: {e}"),
            });
            return out;
        }
    };
    for file in files {
        let parsed = crate::util::read_to_string(&file)
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
        // A directory that cannot be read is a problem of `files` already.
        .and_then(|dir| files_under(dir, FIXTURE_TYPE_EXTENSIONS).ok())
        .unwrap_or_default()
    {
        let Ok(text) = crate::util::read_to_string(&file) else {
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

    /// A name's stem, for the web UI's saves and the MVR import alike.
    #[test]
    fn name_stem_keeps_dashes() {
        assert_eq!(name_stem("Club-2"), "club-2");
        assert_eq!(name_stem("Main Hall"), "main_hall");
        assert_eq!(name_stem("a/b.c"), "a_b_c");
    }

    /// The one rule for a file a save or delete acts on: one declaring the
    /// name; a broken one only when the person named exactly it; never one
    /// that reads and holds other names.
    #[test]
    fn may_act_on_a_broken_file_only_when_named() {
        let file = Path::new("dir/Par.light");
        let broken: Declares = Err("line 2: expected }".to_string());
        let foreign: Declares = Ok(vec!["Spot".to_string()]);
        let own: Declares = Ok(vec!["Par".to_string(), "Spot".to_string()]);

        assert!(may_act_on("Par", file, &own, None).is_ok());
        let refused = may_act_on("Par", file, &broken, None).unwrap_err();
        assert_eq!(refused.file, "Par.light");
        assert_eq!(refused.parse_error.as_deref(), Some("line 2: expected }"));
        assert!(may_act_on("Par", file, &broken, Some("Par.light")).is_ok());
        assert!(may_act_on("Par", file, &broken, Some("Other.light")).is_err());
        let refused = may_act_on("Par", file, &foreign, Some("Par.light")).unwrap_err();
        assert_eq!(refused.parse_error, None);
        assert!(refused.why.contains("\"Spot\""), "{refused:?}");
    }

    /// A link is followed to what is inside the directory only: one out of
    /// it, or to nothing, is not listed, as the tools reading the listed
    /// files would refuse it.
    #[cfg(unix)]
    #[test]
    fn files_under_does_not_follow_a_link_out_of_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("secret.venue"), "").unwrap();
        std::fs::create_dir(dir.path().join("club")).unwrap();
        std::fs::write(dir.path().join("club/main.venue"), "").unwrap();
        let link =
            |to: &Path, at: &str| std::os::unix::fs::symlink(to, dir.path().join(at)).unwrap();
        link(outside.path(), "linked");
        link(&outside.path().join("secret.venue"), "out.venue");
        link(&dir.path().join("club"), "alias");
        link(&dir.path().join("gone.venue"), "dead.venue");

        let found = files_under(dir.path(), VENUE_EXTENSIONS).unwrap();
        let relative: Vec<_> = found.iter().map(|p| display_in(dir.path(), p)).collect();
        assert_eq!(relative, ["alias/main.venue", "club/main.venue"]);
    }

    /// A directory that cannot be read is an error; one below it that
    /// cannot be read is passed over; one that is not there has none.
    #[cfg(unix)]
    #[test]
    fn files_under_an_unreadable_directory() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.venue"), "").unwrap();
        let sub = dir.path().join("locked");
        std::fs::create_dir(&sub).unwrap();
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();
        let below = files_under(dir.path(), VENUE_EXTENSIONS);
        let root = files_under(&sub, VENUE_EXTENSIONS);
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();

        assert_eq!(below.unwrap(), [dir.path().join("a.venue")]);
        assert!(root.is_err());

        // A directory inside one that cannot be searched is not missing.
        std::fs::create_dir(sub.join("inner")).unwrap();
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o000)).unwrap();
        let unreachable = files_under(&sub.join("inner"), VENUE_EXTENSIONS);
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(unreachable.is_err());
        assert!(files_under(&dir.path().join("none"), VENUE_EXTENSIONS)
            .unwrap()
            .is_empty());
    }

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
