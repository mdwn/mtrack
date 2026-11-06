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

//! The songs API's payloads (`/api/songs`).

use serde::Serialize;

use crate::audio::beat_grid::BeatGrid;
use crate::audio::guessed_tempo::GuessedTempo;
use crate::config::{Section, TempoConfig};
pub use crate::songs::FileKind;

/// `GET /api/songs`: the songs the player knows, and those that failed to
/// load.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SongList {
    pub songs: Vec<SongInfo>,
    pub failures: Vec<SongFailure>,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SongInfo {
    pub name: String,
    pub duration_ms: u64,
    pub duration_display: String,
    pub num_channels: u16,
    pub sample_format: String,
    pub track_count: usize,
    pub tracks: Vec<String>,
    pub has_midi: bool,
    pub has_lighting: bool,
    /// The song's directory, relative to the songs root.
    pub base_dir: String,
    /// The `.light` files, relative to the songs root.
    pub lighting_files: Vec<String>,
    /// The MIDI DMX files, relative to the songs root.
    pub midi_dmx_files: Vec<String>,
    /// From the explicit tempo map or click track analysis.
    pub beat_grid: Option<BeatGrid>,
    pub has_tempo_map: bool,
    pub loop_playback: bool,
    /// The resolved loop crossfade, when the song has one.
    pub loop_crossfade_ms: Option<u64>,
    pub loop_crossfade_cut: Vec<String>,
    pub sections: Vec<Section>,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SongFailure {
    pub name: String,
    pub error: String,
    pub base_dir: String,
    /// Always `true`: what tells a failure from a song in a mixed list.
    pub failed: bool,
}

/// `GET /api/songs/:name/tempo-guess`: a tempo map suggested for the song,
/// and where it came from.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TempoGuess {
    pub source: TempoGuessSource,
    pub tempo: GuessedTempo,
    /// The guess as a song's `tempo:` block (`GuessedTempo::to_config`).
    pub config: TempoConfig,
}

/// What a tempo guess was read from.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TempoGuessSource {
    /// The song's MIDI file's tempo events.
    Midi,
    /// The click track's beat grid.
    BeatGrid,
}

/// `GET /api/songs/:name/files`: the files in a song's directory, by name.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SongFiles {
    pub files: Vec<SongFile>,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SongFile {
    pub name: String,
    #[serde(rename = "type")]
    pub kind: FileKind,
}

/// `GET /api/browse`: a directory under the config root, directories first.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Browse {
    /// Relative to `root`.
    pub path: String,
    pub root: String,
    pub entries: Vec<BrowseEntry>,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct BrowseEntry {
    pub name: String,
    pub path: String,
    #[serde(rename = "type")]
    pub kind: FileKind,
    pub is_dir: bool,
}

/// `POST /api/browse/bulk-import`: the songs made, by directory.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct BulkImport {
    pub created: Vec<String>,
    /// Directories that already have a `song.yaml`.
    pub skipped: Vec<String>,
    pub failed: Vec<ImportFailure>,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ImportFailure {
    pub name: String,
    pub error: String,
}
