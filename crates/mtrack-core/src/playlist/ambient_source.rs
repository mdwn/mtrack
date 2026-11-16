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

//! Whose ambient light settings are up, as every surface reports it:
//! light, so the web UI's WebAssembly module compiles it as it is.

use serde::Serialize;

/// Whose settings an ambient light plays.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AmbientSource {
    /// The profile's, under `dmx.lighting.ambient`.
    Profile,
    /// The playlist's own `ambient`.
    Playlist,
    /// The playlist entry's own `ambient`, before its song.
    Entry,
}

impl AmbientSource {
    pub fn as_str(self) -> &'static str {
        match self {
            AmbientSource::Profile => "profile",
            AmbientSource::Playlist => "playlist",
            AmbientSource::Entry => "entry",
        }
    }
}
