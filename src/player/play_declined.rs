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

//! Why a play started no song: in words for every surface, and by key for
//! a client that must tell the reasons apart without reading the words
//! (the gRPC API names it in its [`PLAY_DECLINED_KEY`] metadata).

use serde::Serialize;

/// The gRPC metadata a declined play names its reason under.
pub const PLAY_DECLINED_KEY: &str = "mtrack-play-declined";

/// Why a play started no song.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Debug, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlayDeclined {
    AlreadyPlaying,
    EmptyPlaylist,
    NotStarted,
}

impl PlayDeclined {
    /// The reason's key, as it is serialized: `already_playing`, ….
    pub fn key(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|v| v.as_str().map(String::from))
            .unwrap_or_default()
    }
}

impl std::fmt::Display for PlayDeclined {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            PlayDeclined::AlreadyPlaying => "song already playing",
            PlayDeclined::EmptyPlaylist => {
                "the playlist is empty — no songs loaded, which usually \
                 means none passed validation at startup (check the log)"
            }
            PlayDeclined::NotStarted => "the player did not start a song",
        })
    }
}
