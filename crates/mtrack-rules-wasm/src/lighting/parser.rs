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
//! The player's light show parser (`crates/mtrack-core/src/lighting/parser`), the fixture
//! and venue parser left out (the editor edits shows), and the disk-facing
//! entry points of the player's `parser.rs` with it: here the files an
//! `include` names come from the UI (`Sources`).

#[path = "../../../mtrack-core/src/lighting/parser/document.rs"]
pub mod document;
#[path = "../../../mtrack-core/src/lighting/parser/edit.rs"]
pub mod edit;
#[path = "../../../mtrack-core/src/lighting/parser/effect_parse.rs"]
mod effect_parse;
#[path = "../../../mtrack-core/src/lighting/parser/error.rs"]
mod error;
#[path = "../../../mtrack-core/src/lighting/parser/include.rs"]
mod include;
#[path = "../../../mtrack-core/src/lighting/parser/layout.rs"]
pub mod layout;
#[path = "../../../mtrack-core/src/lighting/parser/palette.rs"]
pub mod palette;
#[path = "../../../mtrack-core/src/lighting/parser/palette_edit.rs"]
pub mod palette_edit;
#[path = "../../../mtrack-core/src/lighting/parser/sequences.rs"]
mod sequences;
#[path = "../../../mtrack-core/src/lighting/parser/show.rs"]
mod show;
#[path = "../../../mtrack-core/src/lighting/parser/tempo_parse.rs"]
mod tempo_parse;
#[path = "../../../mtrack-core/src/lighting/parser/types.rs"]
mod types;
#[path = "../../../mtrack-core/src/lighting/parser/utils.rs"]
pub(crate) mod utils;
#[path = "../../../mtrack-core/src/lighting/parser/writer.rs"]
pub mod writer;

/// The player's grammar (`crates/mtrack-core/src/lighting/grammar.pest`), derived here
/// because `pest_derive` reads the file relative to the crate being built.
pub(crate) mod grammar {
    use pest_derive::Parser;

    #[derive(Parser)]
    #[grammar = "../mtrack-core/src/lighting/grammar.pest"]
    pub struct LightingParser;
}

pub use include::{IncludeReader, Sources};
pub use palette::{declared_slots, resolve_shows, ColorRef, Palette};
pub(crate) use show::file_items;
pub use show::{parse_light_file_with, parse_light_shows, parse_light_shows_with_tempo};
pub use types::{Cue, Effect, LayerCommand, LayerCommandType, LightShow, LightShows};
