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

use std::collections::HashSet;
use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};

use pest::Parser;

pub mod document;
pub mod edit;
mod effect_parse;
mod error;
pub(crate) mod fixture_venue; // Make accessible for tests
pub(crate) mod grammar;
mod include;
pub mod layout;
pub mod palette;
mod sequences;
mod show;
mod tempo_parse;
mod types;
pub(crate) mod utils; // Make utils accessible for tests
pub(crate) mod words;
pub mod writer;

#[cfg(test)]
mod tests;

// Re-export public items
pub(crate) use effect_parse::effect_keyword;
pub use fixture_venue::{
    parse_fixture_types, parse_fixture_types_listed, parse_venues, parse_venues_listed, FileItem,
    ItemKind, Listed,
};
pub use include::{IncludeReader, Sources};
pub use palette::{
    declared_slots, layer_overrides, parse_overrides, resolve_shows, unknown_slots, ColorRef,
    Palette,
};
pub use show::{parse_light_file_with, parse_light_shows, parse_light_shows_with_tempo};
pub use types::{Cue, Effect, LayerCommand, LayerCommandType, LightShow, LightShows};

use grammar::{LightingParser, Rule};
use include::Include;
use utils::items;

/// The files an `include` names read off the disk, as the player reads
/// them: by their canonical path, each once. The web UI's rules module
/// compiles the parser without this (it has no disk): it hands over
/// [`Sources`] instead.
pub struct DiskReader;

impl IncludeReader for DiskReader {
    fn resolve(&self, path: &Path) -> io::Result<PathBuf> {
        path.canonicalize()
    }

    fn read(&self, path: &Path) -> io::Result<String> {
        crate::util::read_to_string(path)
    }
}

/// Parses a `.light` file's shows, as `parse_light_shows_with_tempo` does.
/// `path` is where the content lives, which its `include`s resolve against;
/// content with an `include` and no path is an error.
pub fn parse_light_file(
    content: &str,
    path: Option<&Path>,
    external: Option<&crate::tempo::TempoMap>,
) -> Result<LightShows, Box<dyn Error>> {
    parse_light_file_within(content, path, None, external)
}

/// Parses a `.light` file as `parse_light_file` does, its `include`s
/// confined to `root` when one is given: for source from somewhere that may
/// not read the whole disk, such as the web UI or an MCP client.
pub fn parse_light_file_within(
    content: &str,
    path: Option<&Path>,
    root: Option<&Path>,
    external: Option<&crate::tempo::TempoMap>,
) -> Result<LightShows, Box<dyn Error>> {
    parse_light_file_with(content, path, root, external, &DiskReader)
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
    for pair in items(pairs) {
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
