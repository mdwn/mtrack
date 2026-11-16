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
//! The light show language and the engine that runs a show: its parser, the
//! effects it describes, the fixtures they are written for, the timeline of
//! a song's shows and their lint. Sending the result to a rig is the player's.

pub mod channels;
pub mod effects;
pub mod engine;
pub mod findings;
pub mod fixture_data;
pub mod geometry;
pub mod lint;
pub mod parser;
// Tempo lives at the crate root (shared with the metronome and song config);
// re-exported here so existing `lighting::tempo::…` paths keep working.
pub use crate::tempo;
pub mod timeline;
pub mod types;

pub use effects::EffectInstance;
pub use engine::EffectEngine;
