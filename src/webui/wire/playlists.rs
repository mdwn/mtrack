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

//! The shapes the web UI holds of playlists. The player sends them as
//! `ListPlaylists`, `GetPlaylist` and `SavePlaylist` messages, which the UI
//! maps into these (src/lib/grpc/library.ts); the types are generated for
//! it from here.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::config::ambient::PlaylistAmbient;
use crate::config::{AutoPlayConfig, EntryFades};

/// A playlist, as the list of them shows it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PlaylistInfo {
    pub name: String,
    pub song_count: usize,
    pub is_active: bool,
    /// The player's own playlist of every song (`playlist::is_reserved_name`),
    /// which has no file to edit or delete.
    pub reserved: bool,
}

/// A playlist's songs and, entry for entry, their settings, with the songs
/// it could have (`GetPlaylist` with `with_library`).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PlaylistDetail {
    pub name: String,
    pub songs: Vec<String>,
    /// Each entry's auto play: `true` for at once, a gap, or `null` for
    /// none.
    pub auto_play: Vec<Option<AutoPlayConfig>>,
    pub fades: Vec<EntryFades>,
    /// The playlist's ambient light: `null` for the profile's, `false` for
    /// none, or settings of its own.
    #[cfg_attr(feature = "tsify", tsify(type = "false | Ambient | null"))]
    pub ambient: Option<PlaylistAmbient>,
    /// Each entry's ambient light before it: `null` for the playlist's,
    /// `false` for none, or settings of its own.
    #[cfg_attr(feature = "tsify", tsify(type = "(false | Ambient | null)[]"))]
    pub entry_ambient: Vec<Option<PlaylistAmbient>>,
    /// Each entry's lighting palette overrides: a slot to its colour, as the
    /// playlist file writes them. Empty for an entry with none.
    pub entry_palette: Vec<BTreeMap<String, String>>,
    pub available_songs: Vec<String>,
    /// Each song's tracks, for the tracks a fade leaves out.
    pub song_tracks: BTreeMap<String, Vec<String>>,
}

/// What a playlist save sets (`SavePlaylist`): the songs, and what is set
/// beyond them; each setting left out keeps what the playlist has.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct PlaylistBody {
    pub songs: Vec<String>,
    /// Each song's auto play, in the order of `songs`.
    #[serde(default)]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub auto_play: Option<Vec<Option<AutoPlayConfig>>>,
    /// Each song's fade out and fade in, in the order of `songs`.
    #[serde(default)]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub fades: Option<Vec<EntryFades>>,
    /// Each song's ambient light before it, in the order of `songs`: `null`
    /// for the playlist's, `false` for none, a map overriding settings.
    #[serde(default)]
    #[cfg_attr(
        feature = "tsify",
        tsify(optional, type = "(false | Ambient | null)[]")
    )]
    pub entry_ambient: Option<Vec<Option<PlaylistAmbient>>>,
    /// Each song's palette overrides, in the order of `songs`. Sent with the
    /// other settings, which replace the entry's: the editor passes back what
    /// it read.
    #[serde(default)]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub entry_palette: Option<Vec<BTreeMap<String, String>>>,
    /// The playlist's ambient light: absent keeps it, `null` removes it
    /// (the profile's plays), `false` turns it off, a map overrides
    /// settings.
    #[serde(default, deserialize_with = "present")]
    #[cfg_attr(feature = "tsify", tsify(optional, type = "false | Ambient | null"))]
    pub ambient: Option<Option<PlaylistAmbient>>,
}

/// A field that is there, null or not: `Some(None)` for null, against the
/// `None` of a field left out.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Deserialize::deserialize(deserializer).map(Some)
}
