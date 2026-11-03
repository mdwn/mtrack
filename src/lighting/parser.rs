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

mod effect_parse;
mod error;
pub(crate) mod fixture_venue; // Make accessible for tests
pub(crate) mod grammar;
mod include;
mod sequences;
mod show;
mod tempo_parse;
mod types;
pub(crate) mod utils; // Make utils accessible for tests

#[cfg(all(test, not(mtrack_wasm)))]
mod tests;

// Re-export public items
pub use fixture_venue::{parse_fixture_types, parse_venues};
pub use include::included_files;
pub use show::{
    layout_sequence_calls, parse_light_file, parse_light_file_within, parse_light_shows,
    parse_light_shows_with_tempo, CallLayout, FileLayout, ShowLayout, MAX_LAID_OUT_CUES,
};
pub use types::{Cue, DroppedCue, Effect, LayerCommand, LayerCommandType, LightShow, LightShows};
