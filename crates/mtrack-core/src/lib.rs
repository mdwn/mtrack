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
//! What mtrack does with no audio, MIDI or DMX hardware: it reads a project
//! (the configuration files, songs and their audio, playlists and light
//! shows), checks it, and works out what a show does. Playing it through
//! devices is the player's. The player re-exports each module at the path it
//! had there.

pub mod audio;
pub mod config;
pub mod dmx;
pub mod lighting;
pub mod midi;
pub mod playlist;
pub mod position;
pub mod project;
pub mod section_loop;
pub mod song_cache;
pub mod songs;
pub mod tempo;
#[cfg(any(test, feature = "test-util"))]
pub mod testutil;
pub mod util;
pub mod webui;
