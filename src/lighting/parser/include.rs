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
//! `include "common.light"`: sequences shared between files (#540).
//!
//! An included path is relative to the folder of the file that includes it,
//! and the included file may define only sequences, and include others. Its
//! sequences then play as if the including file defined them, under that
//! file's tempo.

use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::io;
use std::path::{Component, Path, PathBuf};

use pest::iterators::Pair;
use pest::Parser;

use super::error::describe_pest_error;
use super::grammar::{LightingParser, Rule};
use super::sequences::SequenceLibrary;
use super::utils::items;

/// Where the files an `include` names come from. The player reads them
/// off the disk (`DiskReader`, in the parser's root module); the web UI
/// fetches them itself and hands them over as [`Sources`]. The one parse
/// path reads through whichever it is given, so a show lays out in the
/// editor exactly as it loads in the player.
pub trait IncludeReader {
    /// What stands for the file at `path` wherever it is reached from --
    /// its canonical path on disk -- so a file included twice is read
    /// once and an include cycle ends; or why it cannot be reached.
    fn resolve(&self, path: &Path) -> io::Result<PathBuf>;
    /// The file's text.
    fn read(&self, path: &Path) -> io::Result<String>;
}

/// Sources held in memory under the paths they would have on disk:
/// the web UI's, which fetched them. A file it could not fetch is held
/// with the reason, so the include that names it reports as the player
/// would, with that reason where the player has the file system's.
#[derive(Default)]
pub struct Sources {
    files: HashMap<PathBuf, Result<String, String>>,
}

impl Sources {
    /// No sources at all: for content with no path, whose includes cannot
    /// resolve anyway.
    pub fn none() -> Self {
        Sources::default()
    }

    /// Holds `source` as the file at `path`.
    pub fn add(&mut self, path: impl AsRef<Path>, source: String) {
        self.files.insert(normalize(path.as_ref()), Ok(source));
    }

    /// Records that the file at `path` could not be fetched, and why.
    pub fn add_missing(&mut self, path: impl AsRef<Path>, reason: String) {
        self.files.insert(normalize(path.as_ref()), Err(reason));
    }

    fn get(&self, path: &Path) -> io::Result<&str> {
        match self.files.get(&normalize(path)) {
            Some(Ok(source)) => Ok(source),
            Some(Err(reason)) => Err(io::Error::new(io::ErrorKind::NotFound, reason.clone())),
            None => Err(io::Error::new(
                io::ErrorKind::NotFound,
                "No such file or directory",
            )),
        }
    }
}

impl IncludeReader for Sources {
    fn resolve(&self, path: &Path) -> io::Result<PathBuf> {
        self.get(path).map(|_| normalize(path))
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        self.get(path).map(str::to_string)
    }
}

/// A path with its `.` and `..` components folded away, as the file
/// system would reach it: what stands for a file held in memory.
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// An `include` as written: its path and the line it is on.
pub(super) struct Include {
    pub written: String,
    pub line: usize,
}

impl Include {
    pub(super) fn from_pair(pair: Pair<Rule>) -> Include {
        let line = pair.line_col().0;
        let written = pair
            .into_inner()
            .find(|p| p.as_rule() == Rule::string)
            .map(|p| p.as_str().trim_matches('"').to_string())
            .unwrap_or_default();
        Include { written, line }
    }
}

/// A file being read for its includes: where it is, and how to name it in
/// errors.
#[derive(Clone)]
pub(super) struct IncludingFile<'a> {
    /// The file's path on disk, or `None` for source that was never read
    /// from one (a draft being validated).
    pub path: Option<&'a Path>,
    /// How errors name the file: `None` for the file being parsed, else its
    /// path as included, relative to that file.
    pub display: Option<PathBuf>,
    /// The folder includes must stay inside, canonicalized, when the source
    /// comes from somewhere that may not read the whole disk (the web UI, an
    /// MCP client).
    pub root: Option<&'a Path>,
}

/// Adds the sequences of every file `includes` names, and of the files they
/// include, to `library`. `seen` holds each file already read, so one
/// included twice is read once and an include cycle ends.
pub(super) fn add_included(
    library: &mut SequenceLibrary,
    includes: Vec<Include>,
    from: IncludingFile,
    reader: &dyn IncludeReader,
    seen: &mut HashSet<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    let in_file = |e: String| -> Box<dyn Error> {
        match &from.display {
            Some(display) => format!("{}: {e}", display.display()).into(),
            None => e.into(),
        }
    };
    for include in includes {
        let Include { written, line } = include;
        let Some(from_path) = from.path else {
            return Err(in_file(format!(
                "line {line}: include \"{written}\" resolves against the file's folder, and this \
                 source is not a file; save it first, or validate it with its song"
            )));
        };
        if Path::new(&written).extension().and_then(|e| e.to_str()) != Some("light") {
            return Err(in_file(format!(
                "line {line}: include \"{written}\": only .light files can be included"
            )));
        }
        let folder = from_path.parent().unwrap_or(Path::new(""));
        let path = folder.join(&written);
        let canonical = reader.resolve(&path).map_err(|e| {
            in_file(format!(
                "line {line}: include \"{written}\": cannot read {}: {e}",
                path.display()
            ))
        })?;
        if let Some(root) = from.root {
            if !canonical.starts_with(root) {
                return Err(in_file(format!(
                    "line {line}: include \"{written}\" is outside the songs folder"
                )));
            }
        }
        if !seen.insert(canonical.clone()) {
            continue;
        }
        let display = match &from.display {
            Some(parent) => parent.parent().unwrap_or(Path::new("")).join(&written),
            None => PathBuf::from(&written),
        };
        let content = reader.read(&canonical).map_err(|e| {
            in_file(format!(
                "line {line}: include \"{written}\": cannot read {}: {e}",
                path.display()
            ))
        })?;
        // Nested includes resolve from the path as written, not the
        // canonical one, so a symlinked file includes what its link's folder
        // holds, the same however the file was reached.
        add_file(library, &content, &path, display, from.root, reader, seen)?;
    }
    Ok(())
}

/// Adds the sequences one included file defines, and those of the files it
/// includes.
fn add_file(
    library: &mut SequenceLibrary,
    content: &str,
    path: &Path,
    display: PathBuf,
    root: Option<&Path>,
    reader: &dyn IncludeReader,
    seen: &mut HashSet<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    let shown = display.display().to_string();
    let pairs = LightingParser::parse(Rule::file, content)
        .map_err(|e| format!("{shown}: {}", describe_pest_error(&e, content)))?;

    let mut includes = Vec::new();
    let mut sequences = Vec::new();
    for pair in items(pairs) {
        let what = match pair.as_rule() {
            Rule::include => {
                includes.push(Include::from_pair(pair));
                continue;
            }
            Rule::sequence => {
                sequences.push(pair);
                continue;
            }
            Rule::light_show => "a show",
            Rule::tempo => "a tempo",
            Rule::fixture_type => "a fixture type",
            Rule::venue => "a venue",
            _ => continue,
        };
        return Err(format!(
            "{shown}: line {}: an included file defines only sequences, but this is {what}",
            pair.line_col().0
        )
        .into());
    }

    add_included(
        library,
        includes,
        IncludingFile {
            path: Some(path),
            display: Some(display),
            root,
        },
        reader,
        seen,
    )?;
    library.add(sequences, Some(&shown))
}
