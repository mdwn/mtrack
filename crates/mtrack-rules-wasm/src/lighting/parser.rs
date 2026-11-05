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
//! The player's light show parser (`src/lighting/parser`), the fixture
//! and venue parser left out (the editor edits shows), and the disk-facing
//! entry points of the player's `parser.rs` with it: here the files an
//! `include` names come from the UI (`Sources`).

#[path = "../../../../src/lighting/parser/document.rs"]
pub mod document;
#[path = "../../../../src/lighting/parser/effect_parse.rs"]
mod effect_parse;
#[path = "../../../../src/lighting/parser/error.rs"]
mod error;
#[path = "../../../../src/lighting/parser/include.rs"]
mod include;
#[path = "../../../../src/lighting/parser/layout.rs"]
pub mod layout;
#[path = "../../../../src/lighting/parser/sequences.rs"]
mod sequences;
#[path = "../../../../src/lighting/parser/show.rs"]
mod show;
#[path = "../../../../src/lighting/parser/tempo_parse.rs"]
mod tempo_parse;
#[path = "../../../../src/lighting/parser/types.rs"]
mod types;
#[path = "../../../../src/lighting/parser/utils.rs"]
pub(crate) mod utils;

/// The player's grammar (`src/lighting/grammar.pest`), derived here
/// because `pest_derive` reads the file relative to the crate being built.
pub(crate) mod grammar {
    use pest_derive::Parser;

    #[derive(Parser)]
    #[grammar = "../../src/lighting/grammar.pest"]
    pub struct LightingParser;
}

pub use include::{IncludeReader, Sources};
pub(crate) use show::file_items;
pub use show::{parse_light_file_with, parse_light_shows, parse_light_shows_with_tempo};
pub use types::{Cue, Effect, LayerCommand, LayerCommandType, LightShow, LightShows};
