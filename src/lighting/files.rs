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

//! The one rule for venue and fixture type files: what a new one is called
//! ([`stem`], [`canonical_path`]), which file owns a name and whether it can
//! be edited there ([`Record::owner`]), and how a new one is claimed
//! ([`claim_new`]).
//!
//! The files stay the source of truth: any layout in the configured
//! directories loads ([`super::project_files`]). Only what is created has a
//! fixed place, `<dir>/<stem>.venue` or `<dir>/<stem>.fixture`. Which file
//! declared a name is what the loader recorded ([`Record`]); a file that
//! does not parse is reported, never searched for names.

use std::path::{Path, PathBuf};

use super::project_files::{display_in, FIXTURE_TYPE_EXTENSIONS, VENUE_EXTENSIONS};

/// The two kinds of lighting file this rule is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Kind {
    Venue,
    FixtureType,
}

impl Kind {
    /// The extension a new file of this kind is written with.
    pub fn extension(self) -> &'static str {
        match self {
            Kind::Venue => "venue",
            Kind::FixtureType => "fixture",
        }
    }

    /// Every extension a file of this kind loads from.
    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Kind::Venue => VENUE_EXTENSIONS,
            Kind::FixtureType => FIXTURE_TYPE_EXTENSIONS,
        }
    }

    /// The kind as messages name it.
    pub fn noun(self) -> &'static str {
        match self {
            Kind::Venue => "venue",
            Kind::FixtureType => "fixture type",
        }
    }
}

/// A venue or fixture type name as a file stem: lowercase, letters, digits
/// and `-` kept, any run of anything else one `_`, no `_` at either end,
/// and `untitled` when nothing is left. "Astera-PixelBrick" is
/// `astera-pixelbrick`; "Club (Main)" is `club_main`.
pub fn stem(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() || c == '-' {
            out.push(c);
        } else if !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_matches('_');
    if out.is_empty() {
        "untitled".to_string()
    } else {
        out.to_string()
    }
}

/// Where a new `kind` named `name` is written in `dir`:
/// `dir/<stem>.venue` or `dir/<stem>.fixture`.
pub fn canonical_path(kind: Kind, dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{}.{}", stem(name), kind.extension()))
}

/// One file the loader read, and what it declared of its kind (names in
/// order), or why it did not parse.
#[derive(Clone, Debug, PartialEq)]
pub struct FileRead {
    pub file: PathBuf,
    pub declared: Result<Vec<String>, String>,
}

/// Every file of one kind the loader read, in path order: the record a
/// name's [`Owner`] is taken from, so nothing is read again to find it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Record {
    files: Vec<FileRead>,
}

/// The file that declares a name, and whether it can be edited there.
#[derive(Clone, Debug, PartialEq)]
pub struct Owner {
    pub path: PathBuf,
    pub editable: Editable,
}

/// Whether an [`Owner`]'s file can be rewritten as this one name's.
#[derive(Clone, Debug, PartialEq)]
pub enum Editable {
    /// The file parses and declares this one block of its kind, and nothing
    /// else of that kind.
    Editable,
    /// The file is not this name's alone; it is edited by hand.
    ReadOnly(ReadOnly),
}

/// Why an [`Owner`]'s file is read-only.
#[derive(Clone, Debug, PartialEq)]
pub enum ReadOnly {
    /// The file also declares these (in name order).
    Shared { others: Vec<String> },
    /// The file does not parse. A name is never found in such a file, so
    /// only [`Record::listing`] gives this, for the file itself.
    Unparseable { error: String },
    /// These files all declare the name, in path order; the first keeps it.
    Duplicate { files: Vec<PathBuf> },
}

/// One entry of [`Record::listing`]: a name and its owner, or (no name) a
/// file that does not parse.
#[derive(Clone, Debug, PartialEq)]
pub struct Listed {
    pub name: Option<String>,
    pub owner: Owner,
}

impl Record {
    /// Records one file, after the ones before it in path order.
    pub fn push(&mut self, file: PathBuf, declared: Result<Vec<String>, String>) {
        self.files.push(FileRead { file, declared });
    }

    /// The files read, in path order.
    pub fn files(&self) -> &[FileRead] {
        &self.files
    }

    /// The files declaring `name`, in path order.
    fn declaring<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a FileRead> {
        self.files.iter().filter(move |read| {
            read.declared
                .as_ref()
                .is_ok_and(|names| names.iter().any(|n| n == name))
        })
    }

    /// The file `name` loaded from: the first declaring it.
    pub fn file_of(&self, name: &str) -> Option<&Path> {
        self.files
            .iter()
            .find(|read| {
                read.declared
                    .as_ref()
                    .is_ok_and(|names| names.iter().any(|n| n == name))
            })
            .map(|read| read.file.as_path())
    }

    /// What a file declared, when it parsed.
    pub fn names_in(&self, file: &Path) -> Option<&[String]> {
        self.files
            .iter()
            .find(|read| read.file == file)
            .and_then(|read| read.declared.as_deref().ok())
    }

    /// The owner of `name`: the file it loaded from, editable only when it
    /// is that file's one block of its kind and no other file declares it.
    pub fn owner(&self, name: &str) -> Option<Owner> {
        let files: Vec<&FileRead> = self.declaring(name).collect();
        let first = files.first()?;
        let editable = if files.len() > 1 {
            Editable::ReadOnly(ReadOnly::Duplicate {
                files: files.iter().map(|read| read.file.clone()).collect(),
            })
        } else {
            let mut others: Vec<String> = first
                .declared
                .as_ref()
                .map(|names| names.iter().filter(|n| *n != name).cloned().collect())
                .unwrap_or_default();
            others.sort();
            if others.is_empty() {
                Editable::Editable
            } else {
                Editable::ReadOnly(ReadOnly::Shared { others })
            }
        };
        Some(Owner {
            path: first.file.clone(),
            editable,
        })
    }

    /// Every name with its owner, and every file that does not parse, in
    /// file order. A name declared twice is listed once, at its first file.
    pub fn listing(&self) -> Vec<Listed> {
        let mut out = Vec::new();
        for read in &self.files {
            match &read.declared {
                Err(error) => out.push(Listed {
                    name: None,
                    owner: Owner {
                        path: read.file.clone(),
                        editable: Editable::ReadOnly(ReadOnly::Unparseable {
                            error: error.clone(),
                        }),
                    },
                }),
                Ok(names) => {
                    for name in names {
                        if self.file_of(name) != Some(read.file.as_path()) {
                            continue;
                        }
                        if let Some(owner) = self.owner(name) {
                            out.push(Listed {
                                name: Some(name.clone()),
                                owner,
                            });
                        }
                    }
                }
            }
        }
        out
    }
}

/// Why [`claim_new`] refused a name.
#[derive(Debug)]
pub enum Refusal {
    /// A file in the directory already declares the name.
    Declared {
        kind: Kind,
        name: String,
        file: String,
    },
    /// The name's file, or one with its stem in any case, is there.
    Taken { file: String, by: Vec<String> },
    /// The directory could not be read, so nothing can be said to be free.
    Unreadable { dir: PathBuf, error: std::io::Error },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refusal::Declared { kind, name, file } => write!(
                f,
                "{} \"{name}\" is already declared in {file}; choose another name",
                kind.noun()
            ),
            Refusal::Taken { file, by } if by.is_empty() => {
                write!(f, "{file} already exists; choose another name")
            }
            Refusal::Taken { file, by } => write!(
                f,
                "{file} is taken by {}; choose another name",
                by.iter()
                    .map(|n| format!("\"{n}\""))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Refusal::Unreadable { dir, error } => {
                write!(f, "{} cannot be read: {error}", dir.display())
            }
        }
    }
}

impl std::error::Error for Refusal {}

/// A new name's file, free when it was claimed. [`Claim::write`] creates it
/// only if it is still not there.
#[derive(Debug)]
pub struct Claim {
    path: PathBuf,
}

impl Claim {
    /// The canonical path claimed.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Writes the new file with `create_new` ([`crate::util::write_new_file`]):
    /// one made since the claim is refused (`AlreadyExists`), never replaced.
    pub fn write(&self, contents: &[u8]) -> std::io::Result<()> {
        crate::util::write_new_file(&self.path, contents)
    }
}

/// Claims the canonical file for a new `kind` named `name` in `dir`. Refused
/// when a file in `dir` (at any depth) declares the name, or when the
/// canonical file, or any file of the kind with the same stem in another
/// case, is there. There are no suffixes: a refused name is renamed by the
/// person saving it.
pub fn claim_new(kind: Kind, dir: &Path, name: &str) -> Result<Claim, Refusal> {
    let record = match kind {
        Kind::Venue => super::project_files::venues(dir).record,
        Kind::FixtureType => super::project_files::type_files(dir).record,
    };
    if let Some(file) = record.file_of(name) {
        return Err(Refusal::Declared {
            kind,
            name: name.to_string(),
            file: display_in(dir, file),
        });
    }
    let path = canonical_path(kind, dir, name);
    let wanted = stem(name);
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries.filter_map(|e| e.ok().map(|e| e.path())).collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(Refusal::Unreadable {
                dir: dir.to_path_buf(),
                error,
            })
        }
    };
    let same_stem = |entry: &PathBuf| {
        let (Some(file_stem), Some(extension)) = (
            entry.file_stem().and_then(|s| s.to_str()),
            entry.extension().and_then(|e| e.to_str()),
        ) else {
            return false;
        };
        file_stem.to_lowercase() == wanted
            && kind
                .extensions()
                .iter()
                .any(|e| e.eq_ignore_ascii_case(extension))
    };
    let taken = entries
        .into_iter()
        .find(same_stem)
        .or_else(|| path.symlink_metadata().is_ok().then(|| path.clone()));
    if let Some(file) = taken {
        return Err(Refusal::Taken {
            by: record
                .names_in(&file)
                .map(<[String]>::to_vec)
                .unwrap_or_default(),
            file: display_in(dir, &file),
        });
    }
    Ok(Claim { path })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, rel: &str, text: &str) {
        let path = dir.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn stem_keeps_letters_digits_and_dashes() {
        assert_eq!(stem("Astera-PixelBrick"), "astera-pixelbrick");
        assert_eq!(stem("Club (Main)"), "club_main");
        assert_eq!(stem("RGBW_Par"), "rgbw_par");
        assert_eq!(stem("PB15 PixelBrick"), "pb15_pixelbrick");
        assert_eq!(stem("built-in"), "built-in");
        assert_eq!(stem("a / b \\ c.d!e"), "a_b_c_d_e");
        assert_eq!(stem("  Kelly's  Basement  "), "kelly_s_basement");
        assert_eq!(stem("Café Bühne"), "café_bühne");
        assert_eq!(stem("-edge-"), "-edge-");
    }

    #[test]
    fn stem_of_nothing_is_untitled() {
        assert_eq!(stem(""), "untitled");
        assert_eq!(stem("///"), "untitled");
        assert_eq!(stem("  "), "untitled");
    }

    #[test]
    fn canonical_path_is_the_stem_with_the_kind_s_extension() {
        let dir = Path::new("lighting/venues");
        assert_eq!(
            canonical_path(Kind::Venue, dir, "Club (Main)"),
            dir.join("club_main.venue")
        );
        assert_eq!(
            canonical_path(Kind::FixtureType, dir, "Astera-PixelBrick"),
            dir.join("astera-pixelbrick.fixture")
        );
    }

    fn record() -> Record {
        let mut record = Record::default();
        record.push("a.venue".into(), Ok(vec!["solo".into()]));
        record.push(
            "b.venue".into(),
            Ok(vec!["club".into(), "bar".into(), "twice".into()]),
        );
        record.push("c.venue".into(), Err("expected venue_name".into()));
        record.push("d/twice.venue".into(), Ok(vec!["twice".into()]));
        record.push("e.venue".into(), Ok(vec![]));
        record
    }

    #[test]
    fn a_file_holding_only_the_name_is_editable() {
        assert_eq!(
            record().owner("solo"),
            Some(Owner {
                path: "a.venue".into(),
                editable: Editable::Editable,
            })
        );
    }

    #[test]
    fn a_shared_file_is_read_only_naming_the_others() {
        assert_eq!(
            record().owner("bar"),
            Some(Owner {
                path: "b.venue".into(),
                editable: Editable::ReadOnly(ReadOnly::Shared {
                    others: vec!["club".into(), "twice".into()],
                }),
            })
        );
    }

    #[test]
    fn a_name_two_files_declare_is_read_only_at_its_first_file() {
        assert_eq!(
            record().owner("twice"),
            Some(Owner {
                path: "b.venue".into(),
                editable: Editable::ReadOnly(ReadOnly::Duplicate {
                    files: vec!["b.venue".into(), "d/twice.venue".into()],
                }),
            })
        );
    }

    #[test]
    fn a_name_no_file_declares_has_no_owner() {
        // Not even the stem file of a name in a file that does not parse:
        // such a file is never searched.
        assert_eq!(record().owner("c"), None);
        assert_eq!(record().owner("missing"), None);
    }

    #[test]
    fn the_listing_gives_each_name_once_and_each_unparseable_file() {
        let listing = record().listing();
        let entries: Vec<(Option<&str>, &Path)> = listing
            .iter()
            .map(|l| (l.name.as_deref(), l.owner.path.as_path()))
            .collect();
        assert_eq!(
            entries,
            [
                (Some("solo"), Path::new("a.venue")),
                (Some("club"), Path::new("b.venue")),
                (Some("bar"), Path::new("b.venue")),
                (Some("twice"), Path::new("b.venue")),
                (None, Path::new("c.venue")),
            ]
        );
        assert_eq!(
            listing[4].owner.editable,
            Editable::ReadOnly(ReadOnly::Unparseable {
                error: "expected venue_name".into()
            })
        );
    }

    #[test]
    fn claim_new_gives_the_canonical_path_and_writes_it_once() {
        let dir = tempfile::tempdir().unwrap();
        let claim = claim_new(Kind::Venue, dir.path(), "Club (Main)").unwrap();
        assert_eq!(claim.path(), dir.path().join("club_main.venue"));
        claim.write(b"venue \"Club (Main)\" {\n}\n").unwrap();
        // A file made after the claim is not written over.
        let error = claim.write(b"ours").unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(claim.path()).unwrap(),
            "venue \"Club (Main)\" {\n}\n"
        );
    }

    #[test]
    fn claim_new_in_a_missing_directory_is_free() {
        let dir = tempfile::tempdir().unwrap();
        let claim = claim_new(Kind::FixtureType, &dir.path().join("none"), "Par").unwrap();
        assert_eq!(claim.path(), dir.path().join("none/par.fixture"));
    }

    #[test]
    fn claim_new_refuses_a_name_declared_anywhere() {
        let dir = tempfile::tempdir().unwrap();
        write(
            dir.path(),
            "rig/house.light",
            "fixture_type \"Par\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n",
        );
        let error = claim_new(Kind::FixtureType, dir.path(), "Par").unwrap_err();
        assert!(matches!(error, Refusal::Declared { .. }), "{error:?}");
        assert_eq!(
            error.to_string(),
            "fixture type \"Par\" is already declared in rig/house.light; choose another name"
        );
    }

    #[test]
    fn claim_new_refuses_a_taken_file_naming_what_it_holds() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "club_main.venue", "venue \"Club Main\" {\n}\n");
        let error = claim_new(Kind::Venue, dir.path(), "Club (Main)").unwrap_err();
        assert_eq!(
            error.to_string(),
            "club_main.venue is taken by \"Club Main\"; choose another name"
        );
    }

    #[test]
    fn claim_new_refuses_a_same_stem_file_in_any_case_or_extension_of_the_kind() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Club_Main.LIGHT", "");
        let error = claim_new(Kind::Venue, dir.path(), "club main").unwrap_err();
        assert!(
            matches!(&error, Refusal::Taken { file, by } if file == "Club_Main.LIGHT" && by.is_empty()),
            "{error:?}"
        );
        // A file that does not parse is not searched for names, but its
        // file name is still taken.
        write(dir.path(), "wash.venue", "venue {");
        let error = claim_new(Kind::Venue, dir.path(), "Wash").unwrap_err();
        assert_eq!(
            error.to_string(),
            "wash.venue already exists; choose another name"
        );
        // Another kind's extension is not this kind's file.
        write(dir.path(), "spot.fixture", "");
        assert!(claim_new(Kind::Venue, dir.path(), "Spot").is_ok());
    }
}
