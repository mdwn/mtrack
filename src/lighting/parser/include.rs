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

use std::collections::HashSet;
use std::error::Error;
use std::path::{Path, PathBuf};

use pest::iterators::Pair;
use pest::Parser;

use super::error::describe_pest_error;
use super::grammar::{LightingParser, Rule};
use super::sequences::SequenceLibrary;

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
        let canonical = path.canonicalize().map_err(|e| {
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
        let content = crate::util::read_to_string(&canonical).map_err(|e| {
            in_file(format!(
                "line {line}: include \"{written}\": cannot read {}: {e}",
                path.display()
            ))
        })?;
        // Nested includes resolve from the path as written, not the
        // canonical one, so a symlinked file includes what its link's folder
        // holds, the same however the file was reached.
        add_file(library, &content, &path, display, from.root, seen)?;
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
    seen: &mut HashSet<PathBuf>,
) -> Result<(), Box<dyn Error>> {
    let shown = display.display().to_string();
    let pairs = LightingParser::parse(Rule::file, content)
        .map_err(|e| format!("{shown}: {}", describe_pest_error(&e, content)))?;

    let mut includes = Vec::new();
    let mut sequences = Vec::new();
    for pair in pairs.flat_map(|p| {
        if p.as_rule() == Rule::file {
            p.into_inner().collect::<Vec<_>>()
        } else {
            vec![p]
        }
    }) {
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
        seen,
    )?;
    library.add(sequences, Some(&shown))
}

/// Every file `path` includes, directly or through another included file,
/// canonicalized for watching alongside it. Best effort: a file that does not read or
/// parse ends the search along that branch, and parsing reports why.
pub fn included_files(path: &Path) -> Vec<PathBuf> {
    let mut seen: HashSet<PathBuf> = path.canonicalize().into_iter().collect();
    let mut found = Vec::new();
    collect(path, &mut seen, &mut found);
    found
}

fn collect(path: &Path, seen: &mut HashSet<PathBuf>, found: &mut Vec<PathBuf>) {
    let Ok(content) = crate::util::read_to_string(path) else {
        return;
    };
    let Ok(pairs) = LightingParser::parse(Rule::file, &content) else {
        return;
    };
    let folder = path.parent().unwrap_or(Path::new(""));
    for pair in pairs.flat_map(|p| p.into_inner()) {
        if pair.as_rule() != Rule::include {
            continue;
        }
        let include = Include::from_pair(pair);
        let written = folder.join(&include.written);
        let Ok(included) = written.canonicalize() else {
            continue;
        };
        if seen.insert(included.clone()) {
            found.push(included);
            collect(&written, seen, found);
        }
    }
}
