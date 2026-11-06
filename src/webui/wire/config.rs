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

//! The config API's payloads (`/api/config`): the store's YAML, and what a
//! save made of the hardware reload.

use serde::Serialize;

/// `GET /api/config/store`: the config as written, and its checksum.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StoreSnapshot {
    pub yaml: String,
    pub checksum: String,
}

/// A config save's answer: the config after it, and what became of its
/// hardware reload.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ConfigSaved {
    #[serde(flatten)]
    pub snapshot: StoreSnapshot,
    #[serde(flatten)]
    pub reload: ConfigReload,
}

/// What a config or profile save made of the hardware reload: it ran
/// (`applied`), waits until playback stops (`deferred`), or could not run
/// (`failed`), with why when it was not applied. A save while a song plays
/// is kept and its reload deferred, so "Saved" alone would read as a change
/// already in effect.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ConfigReload {
    pub applied: bool,
    /// `applied`, `deferred` or `failed`.
    pub reload: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// A song reload beside it that failed: the player keeps its songs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub songs_error: Option<String>,
    /// For a deferred reload, the generation it waits for: its wait is over
    /// once the playback updates' `hardware_reload_started` reaches it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub awaits_reload: Option<u64>,
    /// The process whose generation that is: a restarted mtrack counts its
    /// reloads from none, and has loaded the saved config.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub awaits_process: Option<String>,
}

/// A config or profile file saved or deleted, with what became of the
/// hardware reload.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct FileSaved {
    /// `saved` or `deleted`.
    pub status: String,
    /// The profile file, by name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    /// A profile file's new version, sent back as `If-Match`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(flatten)]
    pub reload: ConfigReload,
}

/// `GET /api/profiles`: a profile file, and the sections it has.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ProfileFileInfo {
    pub filename: String,
    pub hostname: Option<String>,
    pub has_audio: bool,
    pub has_midi: bool,
    pub has_dmx: bool,
    pub has_trigger: bool,
    pub has_controllers: bool,
}

/// `GET /api/profiles/{name}`: a profile file, read and as written.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ProfileFile {
    /// The profile as a JSON value, so its f32s are widened as the JSON has
    /// always carried them.
    #[cfg_attr(feature = "tsify", tsify(type = "Profile"))]
    pub profile: serde_json::Value,
    pub yaml: String,
    /// Sent back as `If-Match`.
    pub version: String,
}

/// `PUT /api/samples/upload/{name}`: where the sample was stored.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SampleUploaded {
    /// Always `uploaded`.
    pub status: String,
    pub file: String,
    /// Relative to the project, as a sample definition names it.
    pub path: String,
}
