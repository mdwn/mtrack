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
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct StoreSnapshot {
    pub yaml: String,
    pub checksum: String,
}

/// A config save's answer: the config after it, and what became of its
/// hardware reload.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ConfigSaved {
    pub yaml: String,
    pub checksum: String,
    #[serde(flatten)]
    pub reload: ConfigReload,
}

/// What a config or profile save made of the hardware reload: it ran
/// (`applied`), waits until playback stops (`deferred`), or could not run
/// (`failed`), with why when it was not applied. A save while a song plays
/// is kept and its reload deferred, so "Saved" alone would read as a change
/// already in effect.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
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
