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

//! A hardware profile as written: light, so the web UI's WebAssembly module compiles it as it is
//! and the UI edits it by the generated types. What it means is `profile`'s.

use super::audio_data::Audio;
use super::controller_data::Controller;
use super::dmx_data::Dmx;
use super::midi_data::Midi;
use super::notification_data::NotificationConfig;
use super::trigger_data::TriggerConfig;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// Audio configuration with track mappings.
///
/// Uses `IndexMap` so insertion order is preserved through serialize/deserialize
/// round-trips — important for the web UI config editor.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Deserialize, Serialize, Clone)]
pub struct AudioConfig {
    #[serde(flatten)]
    pub(super) audio: Audio,
    /// Output channel routing per track. May be empty for a profile that is
    /// still being set up; playback requires at least one mapping and fails
    /// with a descriptive error otherwise.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    #[cfg_attr(feature = "tsify", tsify(type = "Record<string, number[]>"))]
    pub(super) track_mappings: IndexMap<String, Vec<u16>>,
    /// Per-output-track gain in dB. Tracks without an entry play at unity.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    #[cfg_attr(feature = "tsify", tsify(type = "Record<string, number>"))]
    pub(super) track_gains: IndexMap<String, f32>,
}

/// A unified hardware profile representing one complete host configuration.
/// Profiles are tried in list order; the first one whose hostname matches (or has
/// no constraint) is used.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone)]
pub struct Profile {
    /// Identifies this config file as a hardware profile.
    #[serde(default = "default_hardware_profile_kind")]
    pub(super) kind: super::kind_data::ConfigKind,

    /// Optional hostname restriction.
    #[serde(default)]
    pub(super) hostname: Option<String>,

    /// Audio configuration (optional if absent from profile).
    #[serde(default)]
    pub(super) audio: Option<AudioConfig>,

    /// MIDI configuration (optional if absent from profile).
    #[serde(default)]
    pub(super) midi: Option<Midi>,

    /// DMX configuration (optional if absent from profile).
    #[serde(default)]
    pub(super) dmx: Option<Dmx>,

    /// Audio trigger configuration (optional).
    #[serde(default)]
    pub(super) trigger: Option<TriggerConfig>,

    /// Controllers associated with this profile. `null` reads as none, so a
    /// whole-profile update can clear the list the way it clears any other
    /// section (see [`Profile::from_update`]).
    #[serde(default, deserialize_with = "null_as_empty")]
    pub(super) controllers: Vec<Controller>,

    /// Notification audio configuration (global overrides).
    #[serde(default)]
    pub(super) notifications: Option<NotificationConfig>,

    /// Status events — MIDI events emitted on player state changes.
    #[serde(default)]
    pub(super) status_events: Option<super::statusevents_data::StatusEvents>,
}

/// A list that may be written as `null`, read as empty.
pub(super) fn null_as_empty<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

pub(super) fn default_hardware_profile_kind() -> super::kind_data::ConfigKind {
    super::kind_data::ConfigKind::HardwareProfile
}
