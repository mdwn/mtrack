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

use unicode_normalization::UnicodeNormalization;

use super::library::LibraryType;
use super::parser::{FileItem, ItemKind};
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

    /// The parser's item of this kind.
    pub fn item_kind(self) -> ItemKind {
        match self {
            Kind::Venue => ItemKind::Venue,
            Kind::FixtureType => ItemKind::FixtureType,
        }
    }
}

/// The longest stem, in bytes: with `.` and the longest extension
/// (`fixture`), a file name fits in 255 bytes.
pub const MAX_STEM_BYTES: usize = 255 - ".fixture".len();

/// A name or file stem folded for comparison: NFC, each character
/// lowercased on its own (so "Σ" is always "σ", never the final "ς"),
/// then NFC again. [`stem`] folds, and so does every comparison of a stem
/// with a file already there, so the two always agree.
pub fn fold(s: &str) -> String {
    s.nfc()
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .nfc()
        .collect()
}

/// Whether two names are the same name: equal once NFC-normalised. A name
/// stays as its file writes it; only the comparison normalises.
pub fn same_name(a: &str, b: &str) -> bool {
    a.nfc().eq(b.nfc())
}

/// A venue or fixture type name as a file stem:
/// - [`fold`]ed: NFC and lowercase, so a name typed on a Mac (decomposed)
///   and elsewhere gives one stem;
/// - letters, digits and `-` kept, with any combining mark that follows a
///   kept character ("n̈", and the dot "İ" lowercases to); any other run
///   one `_`; no `_` at either end;
/// - at most [`MAX_STEM_BYTES`], cut before a character and its marks
///   rather than through them;
/// - `untitled` when nothing is left.
///
/// "Astera-PixelBrick" is `astera-pixelbrick`; "Club (Main)" is
/// `club_main`.
pub fn stem(name: &str) -> String {
    use unicode_normalization::char::is_combining_mark;
    let mut out = String::with_capacity(name.len());
    let mut after_kept = false;
    for c in fold(name).chars() {
        // A mark is asked about first: some (the vowel sign "ि") are also
        // alphanumeric, and one after a separator is still not a
        // character of the name.
        let kept = if is_combining_mark(c) {
            after_kept
        } else {
            c.is_alphanumeric() || c == '-'
        };
        if kept {
            out.push(c);
            after_kept = true;
        } else {
            if !out.ends_with('_') {
                out.push('_');
            }
            after_kept = false;
        }
    }
    let mut cut = out.trim_matches('_');
    if cut.len() > MAX_STEM_BYTES {
        let mut end = MAX_STEM_BYTES;
        while !cut.is_char_boundary(end) {
            end -= 1;
        }
        // A cut just before a combining mark would part it from its base:
        // take the base and its marks off too.
        if cut[end..].chars().next().is_some_and(is_combining_mark) {
            let kept = &cut[..end];
            let marks = kept
                .char_indices()
                .rev()
                .take_while(|(_, c)| is_combining_mark(*c))
                .last()
                .map_or(end, |(i, _)| i);
            end = kept[..marks].char_indices().last().map_or(0, |(i, _)| i);
        }
        cut = cut[..end].trim_matches('_');
    }
    if cut.is_empty() {
        "untitled".to_string()
    } else {
        cut.to_string()
    }
}

/// Where a new `kind` named `name` is written in `dir`:
/// `dir/<stem>.venue` or `dir/<stem>.fixture`.
pub fn canonical_path(kind: Kind, dir: &Path, name: &str) -> PathBuf {
    file_in(kind, dir, &stem(name))
}

/// The file of `kind` with the stem `file_stem`, as written, in `folder`.
fn file_in(kind: Kind, folder: &Path, file_stem: &str) -> PathBuf {
    folder.join(format!("{file_stem}.{}", kind.extension()))
}

/// Where the `.light` file `light` of `kind`, under `dir`, moves when its
/// new text needs the newer format: beside it, with its own stem, as a
/// `.venue` or `.fixture` (`tour/old.light` to `tour/old.venue`). Refused
/// as a new name's file is when that file, or one of the kind with its stem
/// in another case, is there.
pub fn upgrade_path(kind: Kind, dir: &Path, light: &Path) -> Result<PathBuf, Refusal> {
    let folder = light.parent().unwrap_or(dir);
    let own = light
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    let path = file_in(kind, folder, own);
    file_free(kind, &read_record(kind, dir), dir, &path, Some(light))?;
    Ok(path)
}

/// Refuses `path` when it, or a file of `kind` in its folder whose stem
/// folds to the same, is there (`except` aside). Files are shown relative
/// to `dir`.
fn file_free(
    kind: Kind,
    record: &Record,
    dir: &Path,
    path: &Path,
    except: Option<&Path>,
) -> Result<(), Refusal> {
    let folder = path.parent().unwrap_or(dir);
    let wanted = path
        .file_stem()
        .and_then(|s| s.to_str())
        .map(fold)
        .unwrap_or_default();
    let entries: Vec<PathBuf> = match std::fs::read_dir(folder) {
        Ok(entries) => entries.filter_map(|e| e.ok().map(|e| e.path())).collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => {
            return Err(Refusal::Unreadable {
                dir: folder.to_path_buf(),
                error,
            })
        }
    };
    let taken = entries
        .into_iter()
        .filter(|entry| Some(entry.as_path()) != except)
        .find(|entry| has_stem_of_kind(entry, kind, &wanted))
        .or_else(|| path.symlink_metadata().is_ok().then(|| path.to_path_buf()));
    match taken {
        Some(file) => Err(Refusal::Taken {
            by: record.names_in(&file).map(unique).unwrap_or_default(),
            file: display_in(dir, &file),
        }),
        None => Ok(()),
    }
}

/// What a file that parsed holds: the names of its kind, in file order (a
/// name declared twice is here twice), and every other top-level item.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contents {
    pub names: Vec<String>,
    pub others: Vec<FileItem>,
}

impl Contents {
    /// What a file's top-level `items` (in file order) hold, for `kind`.
    pub fn of(kind: Kind, items: Vec<FileItem>) -> Self {
        let (names, others): (Vec<_>, Vec<_>) = items
            .into_iter()
            .partition(|item| item.kind == kind.item_kind());
        Contents {
            names: names.into_iter().filter_map(|item| item.name).collect(),
            others,
        }
    }

    /// What `text`, the whole of a file of `kind`, holds; why it does not
    /// parse otherwise.
    pub fn parse(kind: Kind, text: &str) -> Result<Self, String> {
        let items = match kind {
            Kind::Venue => super::parser::parse_venues_listed(text).map(|(_, items)| items),
            Kind::FixtureType => {
                super::parser::parse_fixture_types_listed(text).map(|(_, items)| items)
            }
        };
        items
            .map(|items| Contents::of(kind, items))
            .map_err(|e| e.to_string())
    }

    /// Whether these contents are `name`'s alone, as an [`Owner`]'s file
    /// must be to be [`Editable::Editable`]: the rule a file's new text is
    /// held to before it is written. `None` when they do not declare it.
    pub fn alone(&self, name: &str) -> Option<Editable> {
        let mut record = Record::default();
        record.push(PathBuf::new(), Ok(self.clone()));
        record.owner(name).map(|owner| owner.editable)
    }
}

/// One file the loader read, and what it holds, or why it did not parse.
#[derive(Clone, Debug, PartialEq)]
pub struct FileRead {
    pub file: PathBuf,
    pub declared: Result<Contents, String>,
}

impl FileRead {
    /// How many blocks of its kind named `name` the file holds.
    fn blocks_of(&self, name: &str) -> usize {
        self.declared
            .as_ref()
            .map(|c| c.names.iter().filter(|n| *n == name).count())
            .unwrap_or(0)
    }
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

impl Owner {
    /// Why the owner's file is read-only, as the editor and a refused write
    /// say it, with files shown relative to `dir`; `None` when editable.
    pub fn reason(&self, kind: Kind, dir: &Path) -> Option<String> {
        let Editable::ReadOnly(why) = &self.editable else {
            return None;
        };
        let file = display_in(dir, &self.path);
        let quoted = |names: &[String]| {
            names
                .iter()
                .map(|n| format!("\"{n}\""))
                .collect::<Vec<_>>()
                .join(", ")
        };
        Some(match why {
            ReadOnly::Duplicate { files } => {
                let shown: Vec<String> =
                    unique(&files.iter().map(|f| display_in(dir, f)).collect::<Vec<_>>());
                if shown.len() == 1 {
                    format!(
                        "Defined {} times in {file}; edit that file by hand.",
                        files.len()
                    )
                } else {
                    format!(
                        "Defined more than once, in {}; edit those files by hand.",
                        shown.join(", ")
                    )
                }
            }
            ReadOnly::Shared { others } => {
                let noun = if others.len() == 1 {
                    kind.noun().to_string()
                } else {
                    format!("{}s", kind.noun())
                };
                format!(
                    "Defined in {file} alongside {noun} {}; edit that file by hand.",
                    quoted(others)
                )
            }
            ReadOnly::Mixed { others } => format!(
                "Defined in {file} alongside {}; edit that file by hand.",
                others
                    .iter()
                    .map(ToString::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            ReadOnly::Unparseable { error } => {
                format!("{file} does not parse ({error}); edit that file by hand.")
            }
        })
    }

    /// Whether the owner's file is where a new file of `name` would be
    /// written: in `dir` itself, with `name`'s [`stem`] (in any case) and
    /// one of the kind's extensions. Only such a file follows a rename.
    pub fn at_canonical_place(&self, kind: Kind, dir: &Path, name: &str) -> bool {
        self.path.parent() == Some(dir) && has_stem_of_kind(&self.path, kind, &stem(name))
    }
}

/// Whether `path` is a file of `kind` whose stem folds to `wanted`.
fn has_stem_of_kind(path: &Path, kind: Kind, wanted: &str) -> bool {
    let (Some(file_stem), Some(extension)) = (
        path.file_stem().and_then(|s| s.to_str()),
        path.extension().and_then(|e| e.to_str()),
    ) else {
        return false;
    };
    fold(file_stem) == wanted
        && kind
            .extensions()
            .iter()
            .any(|e| e.eq_ignore_ascii_case(extension))
}

/// Whether an [`Owner`]'s file can be rewritten as this one name's.
#[derive(Clone, Debug, PartialEq)]
pub enum Editable {
    /// The file parses and holds this one block and nothing else: no other
    /// venue or fixture type, show, sequence, tempo or include.
    Editable,
    /// The file is not this name's alone; it is edited by hand.
    ReadOnly(ReadOnly),
}

/// Why an [`Owner`]'s file is read-only. When several apply, the first
/// here is given.
#[derive(Clone, Debug, PartialEq)]
pub enum ReadOnly {
    /// The name has more than one block: one entry per block, in path
    /// order, so a file declaring it twice is here twice. The first file
    /// keeps the name; within one file the last block is the one loaded.
    Duplicate { files: Vec<PathBuf> },
    /// The file also declares these of the same kind (in name order).
    Shared { others: Vec<String> },
    /// The file also holds these items of other kinds, in file order
    /// (`fixture_type "T"`, `show "S"`, `tempo`).
    Mixed { others: Vec<FileItem> },
    /// The file does not parse. A name is never found in such a file, so
    /// only [`Record::listing`] gives this, for the file itself.
    Unparseable { error: String },
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
    pub fn push(&mut self, file: PathBuf, declared: Result<Contents, String>) {
        self.files.push(FileRead { file, declared });
    }

    /// The files read, in path order.
    pub fn files(&self) -> &[FileRead] {
        &self.files
    }

    /// The file `name` loaded from: the first declaring it.
    pub fn file_of(&self, name: &str) -> Option<&Path> {
        self.files
            .iter()
            .find(|read| read.blocks_of(name) > 0)
            .map(|read| read.file.as_path())
    }

    /// The first file declaring `name` or a name [`same_name`] as it: what
    /// a new name must not take. [`Self::file_of`] is the exact name's.
    pub fn file_declaring_same(&self, name: &str) -> Option<&Path> {
        self.files
            .iter()
            .find(|read| {
                read.declared
                    .as_ref()
                    .is_ok_and(|c| c.names.iter().any(|n| same_name(n, name)))
            })
            .map(|read| read.file.as_path())
    }

    /// The names of its kind a file declared, in file order, when it parsed.
    pub fn names_in(&self, file: &Path) -> Option<&[String]> {
        self.files
            .iter()
            .find(|read| read.file == file)
            .and_then(|read| read.declared.as_ref().ok())
            .map(|contents| contents.names.as_slice())
    }

    /// The owner of `name`: the file it loaded from, editable only when it
    /// is that file's one item and no other block declares the name.
    pub fn owner(&self, name: &str) -> Option<Owner> {
        let blocks: Vec<&FileRead> = self
            .files
            .iter()
            .flat_map(|read| std::iter::repeat_n(read, read.blocks_of(name)))
            .collect();
        let first = blocks.first()?;
        let contents = first.declared.as_ref().ok()?;
        let mut others: Vec<String> = contents
            .names
            .iter()
            .filter(|n| *n != name)
            .cloned()
            .collect();
        others.sort();
        others.dedup();
        let editable = if blocks.len() > 1 {
            Editable::ReadOnly(ReadOnly::Duplicate {
                files: blocks.iter().map(|read| read.file.clone()).collect(),
            })
        } else if !others.is_empty() {
            Editable::ReadOnly(ReadOnly::Shared { others })
        } else if !contents.others.is_empty() {
            Editable::ReadOnly(ReadOnly::Mixed {
                others: contents.others.clone(),
            })
        } else {
            Editable::Editable
        };
        Some(Owner {
            path: first.file.clone(),
            editable,
        })
    }

    /// Every name with its owner, and every file that does not parse, in
    /// file order. A name with several blocks is listed once, at its first.
    pub fn listing(&self) -> Vec<Listed> {
        let mut out = Vec::new();
        let mut seen = std::collections::HashSet::new();
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
                Ok(contents) => {
                    for name in &contents.names {
                        if !seen.insert(name.as_str()) {
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

    /// Names a file declares more than once, with the file: the loader
    /// keeps the last block, and says so.
    pub fn repeats(&self) -> Vec<(&Path, &str)> {
        let mut out = Vec::new();
        for read in &self.files {
            let Ok(contents) = &read.declared else {
                continue;
            };
            let mut seen = std::collections::HashSet::new();
            let mut said = std::collections::HashSet::new();
            for name in &contents.names {
                if !seen.insert(name.as_str()) && said.insert(name.as_str()) {
                    out.push((read.file.as_path(), name.as_str()));
                }
            }
        }
        out
    }
}

/// Why [`claim_new`] or [`claim_record_for`] refused a name.
#[derive(Debug)]
pub enum Refusal {
    /// A file in the directory already declares the name.
    Declared {
        kind: Kind,
        name: String,
        file: String,
    },
    /// An unrecorded GDTF in the library is a fixture type of that name.
    Library { name: String, archive: String },
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
            Refusal::Library { name, archive } => write!(
                f,
                "\"{name}\" is the GDTF library type in {archive}; choose another name"
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

/// Claims the canonical file for a new `kind` named `name` in `dir`.
/// Refused when:
/// - a file in `dir` (at any depth) declares the name;
/// - for a fixture type, one of `library` (the library's unrecorded types,
///   [`super::project_files::ProjectTypes::library`]) has the name; venues
///   pass none;
/// - the canonical file, or any file of the kind with the same stem in
///   another case or normalisation, is there.
///
/// There are no suffixes: a refused name is renamed by the person saving it.
pub fn claim_new(
    kind: Kind,
    dir: &Path,
    name: &str,
    library: &[LibraryType],
) -> Result<Claim, Refusal> {
    claim(kind, dir, name, library, None)
}

/// [`claim_new`] for the record of a library GDTF, the archive at `archive`
/// (project-relative, as [`LibraryType::archive`]): pinning or recording it
/// under `name`. The archive's own library name is allowed; another
/// archive's is refused, as is anything [`claim_new`] refuses.
pub fn claim_record_for(
    dir: &Path,
    archive: &str,
    name: &str,
    library: &[LibraryType],
) -> Result<Claim, Refusal> {
    claim(Kind::FixtureType, dir, name, library, Some(archive))
}

/// `names` without repeats, in first-seen order.
pub fn unique(names: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in names {
        if !out.contains(name) {
            out.push(name.clone());
        }
    }
    out
}

/// Whether `name` may be given to a `kind` in `dir` at all: the first two
/// of [`claim_new`]'s refusals (declared anywhere; for a fixture type, an
/// unrecorded library type's name). A rename that keeps its file asks only
/// this; a new file is [`claim_new`]ed.
pub fn name_free(
    kind: Kind,
    dir: &Path,
    name: &str,
    library: &[LibraryType],
) -> Result<(), Refusal> {
    name_free_in(&read_record(kind, dir), kind, dir, name, library, None)
}

fn read_record(kind: Kind, dir: &Path) -> Record {
    match kind {
        Kind::Venue => super::project_files::venues(dir).record,
        Kind::FixtureType => super::project_files::type_files(dir).record,
    }
}

fn name_free_in(
    record: &Record,
    kind: Kind,
    dir: &Path,
    name: &str,
    library: &[LibraryType],
    own_archive: Option<&str>,
) -> Result<(), Refusal> {
    if let Some(file) = record.file_declaring_same(name) {
        return Err(Refusal::Declared {
            kind,
            name: name.to_string(),
            file: display_in(dir, file),
        });
    }
    if kind == Kind::FixtureType {
        if let Some(other) = library
            .iter()
            .find(|t| same_name(&t.name, name) && Some(t.archive.as_str()) != own_archive)
        {
            return Err(Refusal::Library {
                name: name.to_string(),
                archive: other.archive.clone(),
            });
        }
    }
    Ok(())
}

fn claim(
    kind: Kind,
    dir: &Path,
    name: &str,
    library: &[LibraryType],
    own_archive: Option<&str>,
) -> Result<Claim, Refusal> {
    let record = read_record(kind, dir);
    name_free_in(&record, kind, dir, name, library, own_archive)?;
    let path = canonical_path(kind, dir, name);
    file_free(kind, &record, dir, &path, None)?;
    Ok(Claim { path })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::parser::ItemKind;

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
    fn stem_is_the_same_for_composed_and_decomposed_names() {
        let composed = "Caf\u{e9} Par";
        let decomposed = "Cafe\u{301} Par";
        assert_ne!(composed, decomposed);
        assert_eq!(stem(composed), "caf\u{e9}_par");
        assert_eq!(stem(decomposed), "caf\u{e9}_par");
    }

    #[test]
    fn stem_keeps_the_marks_of_a_kept_character() {
        // "İ" lowercases to "i" and a combining dot (U+0307), which NFC
        // cannot compose: the dot stays with its "i".
        assert_eq!(stem("İstanbul"), "i\u{307}stanbul");
        assert_eq!(stem("İSTANBUL Club"), "i\u{307}stanbul_club");
        // A mark with no composed form stays with its letter.
        assert_eq!(stem("Spın\u{308}al Tap"), "spın\u{308}al_tap");
        // A mark after a separator is not a character of the name.
        assert_eq!(stem("a \u{308}b"), "a_b");
    }

    #[test]
    fn stem_parts_an_alphanumeric_mark_from_a_separator() {
        // The vowel sign U+093F is a combining mark that is also
        // alphanumeric: after a separator it is a separator all the same.
        assert!('\u{93F}'.is_alphanumeric());
        assert_eq!(stem("a \u{93F}b"), "a_b");
        assert_eq!(stem("\u{93F}a"), "a");
        // After a kept letter it stays with it.
        assert_eq!(stem("\u{939}\u{93F}"), "\u{939}\u{93F}");
    }

    #[test]
    fn fold_lowercases_each_character_on_its_own() {
        // `str::to_lowercase` would end "ΟΔΟΣ" in a final sigma.
        assert_eq!("ΟΔΟΣ".to_lowercase(), "οδος");
        assert_eq!(fold("ΟΔΟΣ"), "οδοσ");
        assert_eq!(stem("ΟΔΟΣ"), "οδοσ");
        assert_eq!(fold("ΟΔΟΣ"), fold("οδος".to_uppercase().as_str()));
        assert_eq!(fold("İ"), "i\u{307}");
        assert_eq!(fold("Cafe\u{301}"), "caf\u{e9}");
    }

    #[test]
    fn the_stem_of_a_folded_name_is_the_stem_of_the_name() {
        for name in [
            "ΟΔΟΣ",
            "İstanbul",
            "i\u{307}stanbul",
            "Cafe\u{301} Par",
            "Spın\u{308}al Tap",
            "Club (Main)",
            "Astera-PixelBrick",
            "STRASSE ß",
            "///",
        ] {
            assert_eq!(stem(name), stem(&fold(name)), "{name:?}");
            assert_eq!(fold(&stem(name)), stem(name), "{name:?}");
        }
    }

    #[test]
    fn stem_fits_a_file_name_with_its_extension() {
        let long = stem(&"a".repeat(300));
        assert_eq!(long, "a".repeat(MAX_STEM_BYTES));
        assert!(format!("{long}.fixture").len() <= 255);

        // Two bytes each: cut on a character boundary.
        let umlauts = stem(&"ü".repeat(128));
        assert_eq!(umlauts, "ü".repeat(MAX_STEM_BYTES / 2));
        assert!(format!("{umlauts}.fixture").len() <= 255);

        // A cut that ends on a separator is trimmed.
        let name = format!("{} b", "a".repeat(MAX_STEM_BYTES - 1));
        assert_eq!(stem(&name), "a".repeat(MAX_STEM_BYTES - 1));
    }

    #[test]
    fn a_cut_never_parts_a_character_from_its_marks() {
        // "हि" is a letter and a vowel sign (a combining mark), 3 bytes
        // each. Placed so the cut falls between them, both go.
        let name = format!("{}हि", "a".repeat(MAX_STEM_BYTES - 3));
        assert_eq!(stem(&name), "a".repeat(MAX_STEM_BYTES - 3));
        // A whole cluster that fits is kept.
        let name = format!("{}हि", "a".repeat(MAX_STEM_BYTES - 6));
        assert_eq!(stem(&name), format!("{}हि", "a".repeat(MAX_STEM_BYTES - 6)));
        // Two marks on one base: the base goes with both.
        let name = format!("{}n\u{308}\u{301}", "a".repeat(MAX_STEM_BYTES - 3));
        assert_eq!(stem(&name), "a".repeat(MAX_STEM_BYTES - 3));
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

    fn names(names: &[&str]) -> Result<Contents, String> {
        Ok(Contents {
            names: names.iter().map(|n| n.to_string()).collect(),
            others: Vec::new(),
        })
    }

    fn item(kind: ItemKind, name: Option<&str>) -> FileItem {
        FileItem {
            kind,
            name: name.map(str::to_string),
        }
    }

    fn record() -> Record {
        let mut record = Record::default();
        record.push("a.venue".into(), names(&["solo"]));
        record.push("b.venue".into(), names(&["club", "bar", "twice"]));
        record.push("c.venue".into(), Err("expected venue_name".into()));
        record.push("d/twice.venue".into(), names(&["twice"]));
        record.push("e.venue".into(), names(&[]));
        record.push("f.venue".into(), names(&["tw", "tw"]));
        record.push(
            "g.venue".into(),
            Ok(Contents {
                names: vec!["mixed".into()],
                others: vec![
                    item(ItemKind::FixtureType, Some("T")),
                    item(ItemKind::Show, Some("S")),
                ],
            }),
        );
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
    fn a_name_one_file_declares_twice_is_a_duplicate_in_that_file() {
        assert_eq!(
            record().owner("tw"),
            Some(Owner {
                path: "f.venue".into(),
                editable: Editable::ReadOnly(ReadOnly::Duplicate {
                    files: vec!["f.venue".into(), "f.venue".into()],
                }),
            })
        );
        assert_eq!(record().repeats(), [(Path::new("f.venue"), "tw")]);
    }

    #[test]
    fn a_file_holding_other_kinds_too_is_read_only_naming_them() {
        let owner = record().owner("mixed").unwrap();
        let Editable::ReadOnly(ReadOnly::Mixed { others }) = owner.editable else {
            panic!("{owner:?}");
        };
        let said: Vec<String> = others.iter().map(ToString::to_string).collect();
        assert_eq!(said, ["fixture_type \"T\"", "show \"S\""]);
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
                (Some("tw"), Path::new("f.venue")),
                (Some("mixed"), Path::new("g.venue")),
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
    fn each_read_only_owner_says_its_file_and_why() {
        let dir = Path::new("/v");
        let owner = |editable| Owner {
            path: PathBuf::from("/v/tour/all.light"),
            editable,
        };
        let reason = |editable| owner(editable).reason(Kind::Venue, dir);
        assert_eq!(reason(Editable::Editable), None);
        assert_eq!(
            reason(Editable::ReadOnly(ReadOnly::Shared {
                others: vec!["Arena".into()]
            }))
            .unwrap(),
            "Defined in tour/all.light alongside venue \"Arena\"; edit that file by hand."
        );
        assert_eq!(
            reason(Editable::ReadOnly(ReadOnly::Shared {
                others: vec!["A".into(), "B".into()]
            }))
            .unwrap(),
            "Defined in tour/all.light alongside venues \"A\", \"B\"; edit that file by hand."
        );
        assert_eq!(
            reason(Editable::ReadOnly(ReadOnly::Mixed {
                others: vec![FileItem {
                    kind: ItemKind::Show,
                    name: Some("S".into())
                }]
            }))
            .unwrap(),
            "Defined in tour/all.light alongside show \"S\"; edit that file by hand."
        );
        assert_eq!(
            reason(Editable::ReadOnly(ReadOnly::Duplicate {
                files: vec!["/v/tour/all.light".into(), "/v/b.venue".into()]
            }))
            .unwrap(),
            "Defined more than once, in tour/all.light, b.venue; edit those files by hand."
        );
        assert_eq!(
            reason(Editable::ReadOnly(ReadOnly::Duplicate {
                files: vec!["/v/tour/all.light".into(), "/v/tour/all.light".into()]
            }))
            .unwrap(),
            "Defined 2 times in tour/all.light; edit that file by hand."
        );
    }

    #[test]
    fn only_a_file_in_the_directory_with_the_names_stem_is_at_its_canonical_place() {
        let dir = Path::new("/v");
        let at = |path: &str, name: &str| {
            Owner {
                path: PathBuf::from(path),
                editable: Editable::Editable,
            }
            .at_canonical_place(Kind::Venue, dir, name)
        };
        assert!(at("/v/club_main.venue", "Club (Main)"));
        assert!(at("/v/club_main.light", "Club (Main)"));
        assert!(at("/v/Club_Main.venue", "Club (Main)"));
        assert!(!at("/v/tour/club_main.venue", "Club (Main)"));
        assert!(!at("/v/club.venue", "Club (Main)"));
        assert!(!at("/v/club_main.fixture", "Club (Main)"));
        // The harness's record is not at "Astera-PixelBrick"'s.
        let astera = Owner {
            path: PathBuf::from("/t/astera_pixelbrick.fixture"),
            editable: Editable::Editable,
        };
        assert!(!astera.at_canonical_place(
            Kind::FixtureType,
            Path::new("/t"),
            "Astera-PixelBrick"
        ));
    }

    #[test]
    fn a_text_is_one_names_alone_by_the_editable_rule() {
        let alone =
            |text: &str, name: &str| Contents::parse(Kind::Venue, text).unwrap().alone(name);
        assert_eq!(alone(VENUE, "house"), Some(Editable::Editable));
        assert_eq!(alone(VENUE, "other"), None);
        assert_eq!(
            alone(&format!("{VENUE}venue \"annex\" {{\n}}\n"), "house"),
            Some(Editable::ReadOnly(ReadOnly::Shared {
                others: vec!["annex".into()]
            }))
        );
        assert!(matches!(
            alone(&format!("{VENUE}{VENUE}"), "house"),
            Some(Editable::ReadOnly(ReadOnly::Duplicate { .. }))
        ));
        assert!(Contents::parse(Kind::Venue, "venue {").is_err());
    }

    #[test]
    fn name_free_refuses_a_declared_name_but_not_a_taken_file() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "house.venue", VENUE);
        write(dir.path(), "annex.venue", "venue \"other\" {\n}\n");
        assert!(matches!(
            name_free(Kind::Venue, dir.path(), "house", &[]),
            Err(Refusal::Declared { .. })
        ));
        // `annex.venue` is taken for a new file, but the name is free.
        assert!(name_free(Kind::Venue, dir.path(), "annex", &[]).is_ok());
        assert!(matches!(
            claim_new(Kind::Venue, dir.path(), "annex", &[]),
            Err(Refusal::Taken { .. })
        ));
    }

    /// The owner of the one venue in `text`, read as the loader reads it.
    fn venue_owner(text: &str, name: &str) -> Editable {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "v.venue", text);
        crate::lighting::project_files::venues(dir.path())
            .record
            .owner(name)
            .unwrap()
            .editable
    }

    const VENUE: &str = "venue \"house\" {\n}\n";

    #[test]
    fn a_venue_file_is_editable_only_with_nothing_else_in_it() {
        assert_eq!(venue_owner(VENUE, "house"), Editable::Editable);
        let mixed = |extra: &str, said: &[&str]| {
            let Editable::ReadOnly(ReadOnly::Mixed { others }) =
                venue_owner(&format!("{VENUE}{extra}"), "house")
            else {
                panic!("{extra}");
            };
            let got: Vec<String> = others.iter().map(ToString::to_string).collect();
            assert_eq!(got, said, "{extra}");
        };
        mixed(
            "fixture_type \"T\" {\n  channels: 1\n  channel_map: { \"dimmer\": 1 }\n}\n",
            &["fixture_type \"T\""],
        );
        mixed(
            "show \"S\" {\n    @00:00.000\n    front_wash: static color: \"blue\"\n}\n",
            &["show \"S\""],
        );
        mixed(
            "tempo {\n  start: 0.0s\n  bpm: 120\n  time_signature: 4/4\n}\n",
            &["tempo"],
        );
        mixed(
            "sequence \"hit\" {\n    @1/1\n    front_wash: static, color: \"red\"\n}\n",
            &["sequence \"hit\""],
        );
        mixed("include \"common.light\"\n", &["include \"common.light\""]);
    }

    #[test]
    fn a_venue_declared_twice_in_one_file_is_a_duplicate() {
        let dir = tempfile::tempdir().unwrap();
        let second = "venue \"house\" {\n  fixture \"Dim\" Par @ 1:1\n}\n";
        write(dir.path(), "twice.venue", &format!("{VENUE}{second}"));
        let read = crate::lighting::project_files::venues(dir.path());
        assert_eq!(
            read.record.owner("house").unwrap().editable,
            Editable::ReadOnly(ReadOnly::Duplicate {
                files: vec![
                    dir.path().join("twice.venue"),
                    dir.path().join("twice.venue")
                ],
            })
        );
        // It still loads, as before: the last block.
        assert_eq!(read.items.len(), 1);
        assert_eq!(read.items[0].item.fixtures().len(), 1);
    }

    #[test]
    fn claim_new_gives_the_canonical_path_and_writes_it_once() {
        let dir = tempfile::tempdir().unwrap();
        let claim = claim_new(Kind::Venue, dir.path(), "Club (Main)", &[]).unwrap();
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
        let claim = claim_new(Kind::FixtureType, &dir.path().join("none"), "Par", &[]).unwrap();
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
        let error = claim_new(Kind::FixtureType, dir.path(), "Par", &[]).unwrap_err();
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
        let error = claim_new(Kind::Venue, dir.path(), "Club (Main)", &[]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "club_main.venue is taken by \"Club Main\"; choose another name"
        );
    }

    #[test]
    fn claim_new_refuses_a_same_stem_file_in_any_case_or_extension_of_the_kind() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "Club_Main.LIGHT", "");
        let error = claim_new(Kind::Venue, dir.path(), "club main", &[]).unwrap_err();
        assert!(
            matches!(&error, Refusal::Taken { file, by } if file == "Club_Main.LIGHT" && by.is_empty()),
            "{error:?}"
        );
        // A file that does not parse is not searched for names, but its
        // file name is still taken.
        write(dir.path(), "wash.venue", "venue {");
        let error = claim_new(Kind::Venue, dir.path(), "Wash", &[]).unwrap_err();
        assert_eq!(
            error.to_string(),
            "wash.venue already exists; choose another name"
        );
        // Another kind's extension is not this kind's file.
        write(dir.path(), "spot.fixture", "");
        assert!(claim_new(Kind::Venue, dir.path(), "Spot", &[]).is_ok());
    }

    #[test]
    fn claim_new_refuses_a_same_stem_file_in_another_normalisation() {
        let dir = tempfile::tempdir().unwrap();
        // Written on a Mac: decomposed.
        write(dir.path(), "Cafe\u{301}_Par.venue", "");
        let error = claim_new(Kind::Venue, dir.path(), "Caf\u{e9} Par", &[]).unwrap_err();
        assert!(matches!(error, Refusal::Taken { .. }), "{error:?}");
    }

    fn library_type(name: &str, archive: &str) -> LibraryType {
        LibraryType {
            name: name.to_string(),
            archive: archive.to_string(),
            file_name: archive.rsplit('/').next().unwrap().to_string(),
            fixture: name.to_string(),
            manufacturer: "mtrack".to_string(),
            renamed_from: None,
        }
    }

    #[test]
    fn claim_new_refuses_a_library_type_s_name() {
        let dir = tempfile::tempdir().unwrap();
        let library = [library_type("Synth Brick", "lighting/library/synth.gdtf")];
        let error = claim_new(Kind::FixtureType, dir.path(), "Synth Brick", &library).unwrap_err();
        assert_eq!(
            error.to_string(),
            "\"Synth Brick\" is the GDTF library type in lighting/library/synth.gdtf; \
             choose another name"
        );
        assert!(claim_new(Kind::FixtureType, dir.path(), "Brick", &library).is_ok());
        // The library holds fixture types, not venues.
        assert!(claim_new(Kind::Venue, dir.path(), "Synth Brick", &library).is_ok());
    }

    #[test]
    fn a_library_type_s_record_may_take_its_own_name_only() {
        let dir = tempfile::tempdir().unwrap();
        let library = [
            library_type("Synth Brick", "lighting/library/synth.gdtf"),
            library_type("Other", "lighting/library/other.gdtf"),
        ];
        let synth = "lighting/library/synth.gdtf";
        let claim = claim_record_for(dir.path(), synth, "Synth Brick", &library).unwrap();
        assert_eq!(claim.path(), dir.path().join("synth_brick.fixture"));
        assert!(claim_record_for(dir.path(), synth, "Renamed", &library).is_ok());
        let error = claim_record_for(dir.path(), synth, "Other", &library).unwrap_err();
        assert!(
            matches!(&error, Refusal::Library { archive, .. } if archive == "lighting/library/other.gdtf"),
            "{error:?}"
        );
        // A name a file declares is refused for a record too.
        write(
            dir.path(),
            "brick.fixture",
            "fixture_type \"Synth Brick\" from gdtf(\"lighting/library/synth.gdtf\") {\n}\n",
        );
        let error = claim_record_for(dir.path(), synth, "Synth Brick", &library).unwrap_err();
        assert!(
            matches!(&error, Refusal::Declared { file, .. } if file == "brick.fixture"),
            "{error:?}"
        );
    }

    #[test]
    fn a_name_is_taken_whatever_its_normalisation() {
        let dir = tempfile::tempdir().unwrap();
        // Declared decomposed, in a file whose stem is not the name's.
        write(dir.path(), "old.venue", "venue \"Cafe\u{301}\" {\n}\n");
        let error = claim_new(Kind::Venue, dir.path(), "Caf\u{e9}", &[]).unwrap_err();
        assert!(
            matches!(&error, Refusal::Declared { file, .. } if file == "old.venue"),
            "{error:?}"
        );
        // The name stays as the file writes it.
        let record = crate::lighting::project_files::venues(dir.path()).record;
        assert!(record.owner("Cafe\u{301}").is_some());
        assert!(record.owner("Caf\u{e9}").is_none());

        let library = [library_type(
            "Cafe\u{301} Par",
            "lighting/library/cafe.gdtf",
        )];
        let error =
            claim_new(Kind::FixtureType, dir.path(), "Caf\u{e9} Par", &library).unwrap_err();
        assert!(matches!(error, Refusal::Library { .. }), "{error:?}");
    }

    #[test]
    fn a_final_sigma_or_a_dotted_i_file_blocks_its_name() {
        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "ΟΔΟΣ.venue", "");
        let error = claim_new(Kind::Venue, dir.path(), "ΟΔΟΣ", &[]).unwrap_err();
        assert!(matches!(error, Refusal::Taken { .. }), "{error:?}");

        // İ both ways: a file named with the capital blocks the folded
        // name, and a file named folded blocks the capital.
        write(dir.path(), "İstanbul.venue", "");
        let error = claim_new(Kind::Venue, dir.path(), "i\u{307}stanbul", &[]).unwrap_err();
        assert!(matches!(error, Refusal::Taken { .. }), "{error:?}");
        let other = tempfile::tempdir().unwrap();
        write(other.path(), "i\u{307}stanbul.venue", "");
        let error = claim_new(Kind::Venue, other.path(), "İstanbul", &[]).unwrap_err();
        assert!(matches!(error, Refusal::Taken { .. }), "{error:?}");
    }
}
