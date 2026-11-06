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
/// - Hidden directories and files are left out, as the song loader leaves
///   them out (`songs::is_hidden`), and as a save refuses to write one.
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
            if crate::songs::is_hidden(&path) {
                continue;
            }
            if !inside(&path) {
                warn!(link = %path.display(), "A link among the lighting files leads out of their directory, or to nothing; passing over it");
                continue;
            }
            if path.is_dir() {
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
/// named (by the web UI and the MVR and GDTF imports) and an existing one is
/// found at it: lowercase, with each run of spaces and anything but letters,
/// digits and `-` as one `_`, none at either end (`Club-2` is `club-2`,
/// `Synth Brick (twin)` is `synth_brick_twin`). A name with none of those
/// (`!!!`, `★`) is `untitled`, never an empty stem, which would make a
/// hidden `.light`.
pub fn name_stem(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_alphanumeric() || c == '-' {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('_') {
            out.push('_');
        }
    }
    let out = out.trim_end_matches('_');
    if out.is_empty() {
        "untitled".to_string()
    } else {
        out.to_string()
    }
}

/// The stem a NEW file for `name` takes in `dir`: [`name_stem`], unless a
/// file is already at it (in any of `extensions`, in any case) or `taken`
/// claims it, when it is the first free `{stem}_{n}` from 2. Names that
/// share a stem ("Club (Main)" and "Club Main") get files of their own: the
/// one rule for naming a new file, for the web UI and the MVR and GDTF
/// imports.
pub fn new_file_stem(
    dir: &Path,
    name: &str,
    extensions: &[&str],
    taken: &dyn Fn(&str) -> bool,
) -> String {
    let base = name_stem(name);
    let free =
        |stem: &str| !taken(stem) && crate::util::files_at_stem(dir, stem, extensions).is_empty();
    if free(&base) {
        return base;
    }
    (2..)
        .map(|n| format!("{base}_{n}"))
        .find(|stem| free(stem))
        .expect("some suffix is free")
}

/// Why a lighting file does not read, and the named blocks it seems to
/// open all the same (`venue "Club"`, `fixture_type "Par"`): information to
/// show, and to find a half-edited file by the name it opens.
#[derive(Clone, Debug, PartialEq)]
pub struct Unread {
    pub error: String,
    /// Each named block header found: its keyword and the name.
    pub blocks: Vec<(String, String)>,
}

impl std::fmt::Display for Unread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.error)
    }
}

/// What a lighting file declares, or why it does not read.
pub type Declares = Result<Vec<String>, Unread>;

/// The keywords of the blocks a file can name (`venue "Club"`): what a
/// broken file is said to "maybe hold".
const NAMED_BLOCKS: &[&str] = &["venue", "fixture_type", "show", "sequence"];

/// The named blocks `text` seems to open. Never a decision on whether a file
/// may be written or removed (a file that does not parse cannot be read
/// exactly): it is what the 409 says a broken file may hold, and how
/// [`locate`] finds a broken file that seems to open the name's block
/// elsewhere. It errs toward finding one: an extra mention only offers one
/// more file for replacing (kept aside if replaced), where a missed one
/// leaves a broken file behind to declare the name again. Each of
/// [`NAMED_BLOCKS`] as a whole word (letters, digits, `_` and `-`, as the
/// grammar's names run) outside strings and comments, followed past any
/// whitespace or comments by a quoted name, which as in the grammar has no
/// escapes, wherever it stands (`fixture_type "Par"` with `from gdtf(…)` and
/// its `{` on lines of their own, as the GDTF import writes a record; a
/// show's `sequence "x"` reference counts too); a keyword with no quoted
/// name is passed over.
fn block_headers(text: &str) -> Vec<(String, String)> {
    let chars: Vec<char> = text.chars().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_' || c == '-';
    let is_comment = |i: usize| {
        chars.get(i) == Some(&'#') || (chars.get(i) == Some(&'/') && chars.get(i + 1) == Some(&'/'))
    };
    // Past whitespace and comments from `i`.
    let skip = |mut i: usize| loop {
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        if !is_comment(i) {
            return i;
        }
        while i < chars.len() && chars[i] != '\n' {
            i += 1;
        }
    };
    // The string opening at `i` (a quote), and the index past it.
    let string_at = |i: usize| {
        let end = (i + 1..chars.len())
            .find(|&j| chars[j] == '"')
            .unwrap_or(chars.len());
        (chars[i + 1..end].iter().collect::<String>(), end + 1)
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '"' {
            i = string_at(i).1;
        } else if is_comment(i) {
            i = skip(i);
        } else if is_word(c) && (i == 0 || !is_word(chars[i - 1])) {
            let end = (i..chars.len())
                .find(|&j| !is_word(chars[j]))
                .unwrap_or(chars.len());
            let word: String = chars[i..end].iter().collect();
            let next = skip(end);
            if NAMED_BLOCKS.contains(&word.as_str()) && chars.get(next) == Some(&'"') {
                let (name, past) = string_at(next);
                if !name.is_empty() {
                    out.push((word, name));
                }
                i = past;
            } else {
                i = end;
            }
        } else {
            i += 1;
        }
    }
    out
}

/// An [`Unread`] for `error`, with the blocks `text` opens.
fn unread(error: String, text: &str) -> Unread {
    Unread {
        error,
        blocks: block_headers(text),
    }
}

/// The one read rule for a venue or fixture type file: its bytes, the
/// names `parse` finds in them, or why not, with the blocks the text opens
/// read from it as far as it reads (`from_utf8_lossy`, so a file with a
/// stray Latin-1 byte still names its blocks).
fn declared_by(path: &Path, parse: fn(&str) -> Result<Vec<String>, String>) -> Declares {
    let bytes = crate::util::read_file(path).map_err(|e| unread(e.to_string(), ""))?;
    let lossy = String::from_utf8_lossy(&bytes);
    let text =
        std::str::from_utf8(&bytes).map_err(|_| unread("not valid UTF-8".to_string(), &lossy))?;
    parse(text).map_err(|e| unread(e, text))
}

/// The fixture types the file at `path` declares ([`Declares`]).
pub fn fixture_types_declared(path: &Path) -> Declares {
    declared_by(path, |text| {
        super::parser::parse_fixture_types(text)
            .map(|types| types.into_keys().collect())
            .map_err(|e| e.to_string())
    })
}

/// The venues the file at `path` declares ([`Declares`]).
pub fn venues_declared(path: &Path) -> Declares {
    declared_by(path, |text| {
        super::parser::parse_venues(text)
            .map(|venues| venues.into_keys().collect())
            .map_err(|e| e.to_string())
    })
}

/// Why a save or delete of a name may not act on a file ([`may_act_on`]).
#[derive(Clone, Debug, PartialEq)]
pub struct Refusal {
    /// The file, as the person is shown it and names it back: its path in
    /// the directory it was found in (`a/club.light`).
    pub file: String,
    /// What is wrong with it: `it holds "Other"`, `it does not read: …`.
    pub why: String,
    /// Its parse error, when it does not read: the one case a person may
    /// choose to replace it (the file kept aside either way).
    pub parse_error: Option<String>,
    /// The other named blocks a file that does not read seems to open
    /// (`venue "Arena"`), for the person to weigh: information only.
    pub also_mentions: Vec<String>,
}

/// The one rule for a venue or fixture type file that a save or delete of
/// `name` (a `keyword` block: `venue`, `fixture_type`) would act on (write
/// over, rewrite or remove), wherever it is asked: the web UI's saves and
/// deletes, and the MVR and GDTF imports.
///
/// - A file that declares `name` may be acted on.
/// - One that reads and holds only other names is refused.
/// - One that does not read is refused, unless `replace_broken` names
///   exactly that file: a person who was shown its parse error, and the
///   other blocks it seems to hold, chose to replace it. It is kept aside
///   either way, so nothing it held is lost.
///
/// `shown` is the file as the person sees it: its path in its directory
/// ([`display_in`]), so one of two alike-named files in two subdirectories
/// is never taken for the other.
pub fn may_act_on(
    name: &str,
    keyword: &str,
    shown: &str,
    declares: &Declares,
    replace_broken: &[String],
) -> Result<(), Refusal> {
    let (why, parse_error, also_mentions) = match declares {
        Ok(names) if names.iter().any(|n| n == name) => return Ok(()),
        Ok(names) if names.is_empty() => ("it defines nothing".to_string(), None, Vec::new()),
        Ok(names) => {
            let mut names: Vec<String> = names.iter().map(|n| format!("\"{n}\"")).collect();
            names.sort();
            (format!("it holds {}", names.join(", ")), None, Vec::new())
        }
        Err(_) if replace_broken.iter().any(|f| f == shown) => return Ok(()),
        Err(unread) => {
            let mut also: Vec<String> = unread
                .blocks
                .iter()
                .filter(|(k, n)| !(k == keyword && n == name))
                .map(|(k, n)| format!("{k} {n}"))
                .collect();
            also.sort();
            also.dedup();
            (
                format!("it does not read: {}", unread.error),
                Some(unread.error.clone()),
                also,
            )
        }
    };
    Err(Refusal {
        file: shown.to_string(),
        why,
        parse_error,
        also_mentions,
    })
}

/// Sets a file aside before a save replaces it or a delete removes it (a
/// broken file a person chose to replace, a venue file the MVR import must
/// regenerate): renamed beside itself to `<file>.replaced-<YYYYMMDD-HHMMSS>` (UTC),
/// an extension no loader reads, so whatever it held is kept however the
/// block scan read it. The one way every "Replace it" and "Remove it", and
/// every regenerated file, is kept. Returns where it went.
pub fn set_aside_broken(path: &Path) -> std::io::Result<PathBuf> {
    let file = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stamp = crate::util::utc_stamp(std::time::SystemTime::now());
    let mut backup = path.with_file_name(format!("{file}.replaced-{stamp}"));
    let mut n = 2;
    while backup.symlink_metadata().is_ok() {
        backup = path.with_file_name(format!("{file}.replaced-{stamp}-{n}"));
        n += 1;
    }
    std::fs::rename(path, &backup)?;
    Ok(backup)
}

/// Sets each of `paths` aside ([`set_aside_broken`]), returning each file
/// and where it went. If one cannot be, those already set aside are put
/// back ([`put_back`]) and nothing is left half done.
pub fn set_aside_all(paths: &[PathBuf]) -> Result<Vec<(PathBuf, PathBuf)>, String> {
    let mut done = Vec::new();
    for path in paths {
        match set_aside_broken(path) {
            Ok(backup) => done.push((path.clone(), backup)),
            Err(e) => return Err(format!("{e}{}", put_back(&done))),
        }
    }
    Ok(done)
}

/// Puts files [`set_aside_all`] set aside back where they were, after the
/// write they made way for failed. Any that cannot be is named, with where
/// it is kept, for the error the caller reports.
pub fn put_back(set_aside: &[(PathBuf, PathBuf)]) -> String {
    let stuck: Vec<String> = set_aside
        .iter()
        .filter(|(original, backup)| std::fs::rename(backup, original).is_err())
        .map(|(original, backup)| format!("{} is kept as {}", original.display(), backup.display()))
        .collect();
    if stuck.is_empty() {
        String::new()
    } else {
        format!(" ({})", stuck.join("; "))
    }
}

/// The one rule for whether a file's text may hold a name or an archive's
/// record, read as far as it reads: the header scan ([`block_headers`])
/// finds the `keyword "name"` block, or the name appears after a quote
/// (`"Club"`, `"Club` with its quote left open), or the archive's path
/// appears (`lighting/library/par.gdtf`, for a fixture type's record). It
/// errs toward yes: a yes only offers a file for replacing (kept aside if
/// replaced) or keeps a name or an archive from being taken, where a wrong
/// no can leave a broken file to declare the name again, or delete an
/// archive its record still names. `locate`, the GDTF library's fallback
/// and its deletes, and the imports' choice of a free name all ask it.
pub fn may_hold(text: &str, name: Option<(&str, &str)>, archive: Option<&str>) -> bool {
    let named = name.is_some_and(|(keyword, name)| {
        text.contains(&format!("\"{name}"))
            || block_headers(text)
                .iter()
                .any(|(k, n)| k == keyword && n == name)
    });
    named || archive.is_some_and(|archive| text.contains(archive))
}

/// A file's text as far as it reads (`from_utf8_lossy`); none if it cannot
/// be read at all.
fn lossy_text(path: &Path) -> String {
    let bytes = crate::util::read_file(path).unwrap_or_default();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Whether the file at `path` [`may_hold`] the name or the archive.
pub fn file_may_hold(path: &Path, name: Option<(&str, &str)>, archive: Option<&str>) -> bool {
    may_hold(&lossy_text(path), name, archive)
}

/// The fixture type files under `dir` that do not read and [`may_hold`]
/// the type `name` or the archive `archive`: each may be a record a
/// decision about that name or archive must not pass over.
pub fn broken_type_files_holding(
    dir: &Path,
    name: Option<&str>,
    archive: Option<&str>,
) -> Vec<PathBuf> {
    broken_type_files(dir)
        .into_iter()
        .filter(|path| file_may_hold(path, name.map(|n| ("fixture_type", n)), archive))
        .collect()
}

/// The fixture type files under `dir` that do not read.
pub fn broken_type_files(dir: &Path) -> Vec<PathBuf> {
    files_under(dir, FIXTURE_TYPE_EXTENSIONS)
        .unwrap_or_default()
        .into_iter()
        .filter(|path| fixture_types_declared(path).is_err())
        .collect()
}

/// A file found for a name ([`locate_venue`], [`locate_fixture_type`]),
/// and what it declares.
#[derive(Clone, Debug)]
pub struct Located {
    pub path: PathBuf,
    pub declares: Declares,
    /// When the file found does not read, every other broken candidate for
    /// the name (a file elsewhere that seems to open its block): a save or
    /// delete acts on them all, so none is left to declare the name again
    /// once fixed.
    pub other_broken: Vec<(PathBuf, Declares)>,
}

/// The one rule for the file a name lives in, in a lighting directory: the
/// first file at any depth that declares it (as the lighting system loads
/// it); else the broken candidates for it: a file at the name's stem
/// ([`name_stem`], in any case) that does not read, and each file elsewhere
/// that does not read but seems to open a block for the name (one an older
/// stem named, half-edited), the stem file first and the rest in
/// [`Located::other_broken`]. A stem file that reads and holds other names
/// is not the name's: a new file is named past it ([`new_file_stem`]).
/// `None` is a name no file holds or claims.
fn locate(
    dir: &Path,
    name: &str,
    extensions: &[&str],
    keyword: &str,
    declared: fn(&Path) -> Declares,
) -> std::io::Result<Option<Located>> {
    let mut mentions = Vec::new();
    for path in files_under(dir, extensions)? {
        let declares = declared(&path);
        match &declares {
            Ok(names) if names.iter().any(|n| n == name) => {
                return Ok(Some(Located {
                    path,
                    declares,
                    other_broken: Vec::new(),
                }))
            }
            Err(_) if file_may_hold(&path, Some((keyword, name)), None) => {
                mentions.push((path, declares));
            }
            _ => {}
        }
    }
    let stem = crate::util::files_at_stem(dir, &name_stem(name), extensions)
        .into_iter()
        .map(|path| {
            let declares = declared(&path);
            (path, declares)
        })
        .find(|(_, declares)| declares.is_err());
    let mut candidates: Vec<(PathBuf, Declares)> = stem.into_iter().collect();
    for (path, declares) in mentions {
        if !candidates.iter().any(|(p, _)| *p == path) {
            candidates.push((path, declares));
        }
    }
    if candidates.is_empty() {
        return Ok(None);
    }
    let (path, declares) = candidates.remove(0);
    Ok(Some(Located {
        path,
        declares,
        other_broken: candidates,
    }))
}

/// The venue file `name` lives in, or would be written to ([`locate`]):
/// for the web UI's venue editor and the MVR import alike.
pub fn locate_venue(dir: &Path, name: &str) -> std::io::Result<Option<Located>> {
    locate(dir, name, VENUE_EXTENSIONS, "venue", venues_declared)
}

/// The fixture type file `name` lives in, or would be written to
/// ([`locate`]): for the web UI's fixture type editor and the GDTF import.
pub fn locate_fixture_type(dir: &Path, name: &str) -> std::io::Result<Option<Located>> {
    locate(
        dir,
        name,
        FIXTURE_TYPE_EXTENSIONS,
        "fixture_type",
        fixture_types_declared,
    )
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

    /// The one locate rule: a declaring file at any depth, else a broken
    /// file at the stem, else a broken file opening a block for the name (as
    /// an older stem left it: `club_2.venue` for "Club-2").
    #[test]
    fn locate_finds_declared_then_stem_then_a_broken_mention() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path();
        write(d, "club_2.venue", "venue \"Club-2\" {\n");
        let found = locate_venue(d, "Club-2").unwrap().expect("found");
        assert_eq!(found.path, d.join("club_2.venue"));
        assert!(found.declares.is_err());

        // A stem file that reads and holds another venue is not the name's.
        write(d, "club-2.venue", "venue \"Other\" {\n}\n");
        assert_eq!(
            locate_venue(d, "Club-2").unwrap().unwrap().path,
            d.join("club_2.venue")
        );
        assert_eq!(
            new_file_stem(d, "Club-2", VENUE_EXTENSIONS, &|_| false),
            "club-2_2"
        );
        // A broken one is, whatever it seems to hold: the scan of a file
        // that does not parse decides nothing.
        write(d, "club-2.venue", "venue \"Other\" {\n");
        assert_eq!(
            locate_venue(d, "Club-2").unwrap().unwrap().path,
            d.join("club-2.venue")
        );
        write(d, "club-2.venue", "{{{\n");
        assert_eq!(
            locate_venue(d, "Club-2").unwrap().unwrap().path,
            d.join("club-2.venue")
        );
        write(d, "club-2.venue", "venue \"Other\" {\n}\n");

        write(d, "tour/x.venue", "venue \"Club-2\" {\n}\n");
        assert_eq!(
            locate_venue(d, "Club-2").unwrap().unwrap().path,
            d.join("tour/x.venue")
        );

        write(
            d,
            "robe_esprite.fixture",
            "fixture_type \"Robe-Esprite\" {\n",
        );
        let found = locate_fixture_type(d, "Robe-Esprite")
            .unwrap()
            .expect("found");
        assert_eq!(found.path, d.join("robe_esprite.fixture"));
        assert!(locate_venue(d, "Nowhere").unwrap().is_none());
    }

    /// The block scan, information only, reads names as the grammar does:
    /// a quoted name past any whitespace or comments, no escapes, and only
    /// the blocks a file names (`venue`, `fixture_type`, `show`,
    /// `sequence`); a bare keyword word (`show-par`, a TODO) is no block.
    #[test]
    fn block_headers_read_named_blocks_as_the_grammar_does() {
        let names = |text: &str| block_headers(text);
        let arena = ("venue".to_string(), "Arena".to_string());
        for text in [
            "venue  \"Arena\" {",
            "venue\t\"Arena\" {",
            "venue\n\"Arena\" {",
            "venue\"Arena\" {",
            "venue // the hall\n \"Arena\" {",
        ] {
            assert_eq!(names(text), std::slice::from_ref(&arena), "{text:?}");
        }
        assert_eq!(names("venue \"Club\" {} venue \"Arena\" {").len(), 2);
        let kinds: Vec<String> = names(
            "sequence \"strike\" {}\nshow \"Song\" {}\ntempo {}\ninclude \"x.light\"\n\
             fixture_type \"Par\" {\n  label: \"a venue\" // venue \"Nope\"\n",
        )
        .into_iter()
        .map(|(k, _)| k)
        .collect();
        assert_eq!(kinds, ["sequence", "show", "fixture_type"]);
        // A show's reference to a sequence is listed too: the scan errs
        // toward finding a block.
        let in_show =
            names("show \"Song\" {\n  @1/1\n  sequence \"strike\"(hit: \"#FF0000\"), loop: 2\n");
        assert_eq!(in_show.len(), 2, "{in_show:?}");
        // A header whose `{` is on a line of its own, as the GDTF import
        // writes a record, one with a comment before it, and one with a
        // quote in a comment after the name, are all found.
        let description =
            crate::lighting::gdtf::parse_archive(&crate::lighting::gdtf::build_zip(&[(
                "description.xml",
                crate::lighting::gdtf::SYNTHETIC_DESCRIPTION.as_bytes(),
            )]))
            .unwrap();
        let record = crate::lighting::import::gdtf_definition(
            "Par",
            "lighting/library/par.gdtf",
            "par.gdtf",
            &description,
        );
        let par = ("fixture_type".to_string(), "Par".to_string());
        assert_eq!(names(&record), std::slice::from_ref(&par), "{record}");
        let club = ("venue".to_string(), "Club".to_string());
        for text in [
            "venue \"Club\"\n{\n",
            "venue \"Club\" // the hall\n{\n",
            "venue \"Club\" // \"{\nvenue \"Arena\" {\n",
        ] {
            assert_eq!(names(text).first(), Some(&club), "{text:?}");
        }
        assert_eq!(names("venue \"Club\" // \"{\nvenue \"Arena\" {\n").len(), 2);
        for nothing in [
            "// TODO: move this to the show\n",
            "fixture show-par {",
            "venue Club {",
            "venue \"\" {",
        ] {
            assert!(names(nothing).is_empty(), "{nothing:?}");
        }
    }

    /// The one "may hold" rule over-reports: a quote left open mid-file,
    /// which hides later headers from the scan, or one left open inside a
    /// name, still lets the name be found, and an archive's path names its
    /// record.
    #[test]
    fn may_hold_finds_a_name_the_scan_cannot() {
        let hidden = "venue \"Other\" {\n  color: \"#FF0000\n}\nvenue \"Club\" {\n";
        assert!(!block_headers(hidden).iter().any(|(_, n)| n == "Club"));
        assert!(may_hold(hidden, Some(("venue", "Club")), None));
        assert!(may_hold("venue \"Club {\n", Some(("venue", "Club")), None));
        assert!(may_hold(
            "{{{ lighting/library/par.gdtf\n",
            Some(("fixture_type", "Spot")),
            Some("lighting/library/par.gdtf")
        ));
        assert!(!may_hold(
            "venue \"Arena\" {\n",
            Some(("venue", "Club")),
            None
        ));

        let dir = tempfile::tempdir().unwrap();
        write(dir.path(), "tour/old.light", hidden);
        assert_eq!(
            locate_venue(dir.path(), "Club").unwrap().unwrap().path,
            dir.path().join("tour/old.light")
        );
    }

    /// Venues and fixture types read alike: a file with a Latin-1 byte
    /// still names the blocks it opens.
    #[test]
    fn a_file_that_is_not_utf8_names_its_blocks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("all.light");
        std::fs::write(&path, b"fixture_type \"Par\" {\n}\nvenue \"Caf\xe9\" {\n").unwrap();
        for declares in [fixture_types_declared(&path), venues_declared(&path)] {
            let unread = declares.unwrap_err();
            assert_eq!(unread.error, "not valid UTF-8");
            assert_eq!(unread.blocks.len(), 2, "{unread:?}");
        }
    }

    /// A name's stem, for the web UI's saves and the MVR import alike.
    #[test]
    fn name_stem_keeps_dashes() {
        assert_eq!(name_stem("Club-2"), "club-2");
        assert_eq!(name_stem("Main Hall"), "main_hall");
        assert_eq!(name_stem("a/b.c"), "a_b_c");
        assert_eq!(name_stem("Synth Brick (twin)"), "synth_brick_twin");
        assert_eq!(name_stem("my_fixture__01"), "my_fixture_01");
        for nothing in ["!!!", "()", "★", "🔥", ""] {
            assert_eq!(name_stem(nothing), "untitled", "{nothing}");
        }
    }

    /// The one rule for a file a save or delete acts on: one declaring the
    /// name; a broken one only when the person named exactly it; never one
    /// that reads and holds other names.
    #[test]
    fn may_act_on_a_broken_file_only_when_named() {
        let file = "a/Par.light";
        let unread = |blocks: &[(&str, &str)]| Unread {
            error: "line 2: expected }".to_string(),
            blocks: blocks
                .iter()
                .map(|(k, n)| (k.to_string(), n.to_string()))
                .collect(),
        };
        let broken: Declares = Err(unread(&[("fixture_type", "Par")]));
        let foreign: Declares = Ok(vec!["Spot".to_string()]);
        let own: Declares = Ok(vec!["Par".to_string(), "Spot".to_string()]);
        let named = |files: &[&str]| files.iter().map(|f| f.to_string()).collect::<Vec<_>>();
        let act = |declares: &Declares, flag: &[&str]| {
            may_act_on("Par", "fixture_type", file, declares, &named(flag))
        };

        assert!(act(&own, &[]).is_ok());
        let refused = act(&broken, &[]).unwrap_err();
        assert_eq!(refused.file, "a/Par.light");
        assert_eq!(refused.parse_error.as_deref(), Some("line 2: expected }"));
        assert!(act(&broken, &["b/x.light", "a/Par.light"]).is_ok());
        // Named by its path in the directory: another subdirectory's file
        // of that name is not it.
        assert!(act(&broken, &["Par.light", "b/Par.light"]).is_err());
        let refused = act(&foreign, &["a/Par.light"]).unwrap_err();
        assert_eq!(refused.parse_error, None);
        assert!(refused.why.contains("\"Spot\""), "{refused:?}");
        // A broken file that seems to open another name's block too may be
        // replaced all the same (it is kept aside); the refusal lists what
        // it seems to hold, for the person to weigh.
        let shared = Err(unread(&[("fixture_type", "Par"), ("venue", "Club")]));
        let refused = act(&shared, &[]).unwrap_err();
        assert!(refused.parse_error.is_some());
        assert_eq!(refused.also_mentions, ["venue Club"]);
        assert!(act(&shared, &["a/Par.light"]).is_ok());
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
