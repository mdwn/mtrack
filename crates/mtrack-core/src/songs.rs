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
//! What a file in a project is, by its name. Loading a song is the player's.

mod file_kind;
pub use file_kind::{
    extension_of, has_exact_extension, has_extension, is_yaml_config, FileKind, AUDIO_EXTENSIONS,
    LIGHTING_EXTENSIONS, MIDI_EXTENSIONS, YAML_EXTENSIONS,
};
