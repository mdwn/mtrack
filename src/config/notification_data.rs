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

//! A profile's `notifications:` as written: light, so the web UI's WebAssembly module compiles it as it is
//! and the UI edits it by the generated types. What it means is `notification`'s.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Global notification audio configuration.
///
/// Configured in the hardware profile to override default notification tones
/// with custom audio files. All paths are relative to the config base directory
/// unless absolute.
///
/// ```yaml
/// notifications:
///   loop_armed: /path/to/custom_loop_armed.wav
///   break_requested: /path/to/custom_break.wav
///   loop_exited: /path/to/custom_exited.wav
///   section_entering: /path/to/default_section_enter.wav
///   sections:
///     verse: /path/to/verse_announce.wav
///     chorus: /path/to/chorus_announce.wav
/// ```
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Serialize, Clone, Debug, Default)]
pub struct NotificationConfig {
    /// Override audio for the "loop armed" notification.
    #[serde(default)]
    pub(super) loop_armed: Option<String>,
    /// Override audio for the "break requested" notification.
    #[serde(default)]
    pub(super) break_requested: Option<String>,
    /// Override audio for the "loop exited" notification.
    #[serde(default)]
    pub(super) loop_exited: Option<String>,
    /// Override audio for the generic "section entering" notification.
    #[serde(default)]
    pub(super) section_entering: Option<String>,
    /// Per-section-name audio overrides.
    #[serde(default)]
    pub(super) sections: HashMap<String, String>,
}
