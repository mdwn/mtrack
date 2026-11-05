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

//! The `samples:` a config defines and triggers, as written: light, so the web UI's WebAssembly module compiles it as it is
//! and the UI edits it by the generated types. What it means is `samples`'s.

use super::midi_event::MidiEvent;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Default maximum number of concurrent sample voices globally.
pub const DEFAULT_MAX_SAMPLE_VOICES: u32 = 32;

/// A YAML representation of a sample definition.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Clone, Serialize, Debug)]
pub struct SampleDefinition {
    /// The audio file for this sample (used when velocity mode is not "layers").
    #[serde(default)]
    pub(super) file: Option<String>,

    /// The output channels to route this sample to (1-indexed).
    #[serde(default)]
    pub(super) output_channels: Vec<u16>,

    /// A track mapping name to resolve output channels from the profile's track_mappings.
    /// Mutually exclusive with output_channels; if both are set, output_track takes precedence.
    #[serde(default)]
    pub(super) output_track: Option<String>,

    /// Velocity handling configuration.
    #[serde(default)]
    pub(super) velocity: VelocityConfig,

    /// Behavior when the voice is released (e.g. Note Off, trigger release).
    #[serde(default, alias = "note_off")]
    pub(super) release_behavior: ReleaseBehavior,

    /// Behavior when the sample is retriggered while still playing.
    #[serde(default)]
    pub(super) retrigger: RetriggerBehavior,

    /// Maximum number of concurrent voices for this sample.
    /// If not set, only the global limit applies.
    #[serde(default)]
    pub(super) max_voices: Option<u32>,

    /// Fade time in milliseconds for release_behavior: fade.
    #[serde(default = "default_fade_time_ms")]
    pub(super) fade_time_ms: u32,
}

pub(super) fn default_fade_time_ms() -> u32 {
    50
}

/// Configuration for velocity handling.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Clone, Serialize, Debug, Default)]
pub struct VelocityConfig {
    /// The velocity handling mode.
    #[serde(default)]
    pub(super) mode: VelocityMode,

    /// Default velocity value when mode is "ignore".
    #[serde(default)]
    pub(super) default: Option<u8>,

    /// Whether to also scale volume by velocity when using layers.
    #[serde(default)]
    pub(super) scale: Option<bool>,

    /// Velocity layers (used when mode is "layers").
    #[serde(default)]
    pub(super) layers: Vec<VelocityLayer>,
}

/// Velocity handling mode.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Clone, Copy, Serialize, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VelocityMode {
    /// Ignore velocity, play at default volume.
    #[default]
    Ignore,
    /// Scale volume by velocity (velocity/127).
    Scale,
    /// Select different sample files based on velocity ranges.
    Layers,
}

/// A velocity layer that maps a velocity range to a sample file.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Clone, Serialize, Debug)]
pub struct VelocityLayer {
    /// The velocity range [min, max] inclusive (0-127).
    pub(super) range: [u8; 2],

    /// The audio file for this velocity layer.
    pub(super) file: String,
}

/// Behavior when a voice is released (e.g. Note Off, trigger release).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Clone, Copy, Serialize, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseBehavior {
    /// Let the sample play to completion, ignoring the release.
    #[default]
    PlayToCompletion,
    /// Immediately stop the sample on release.
    Stop,
    /// Fade out the sample over a short duration on release.
    Fade,
}

/// Behavior when a sample is triggered while it's already playing.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Clone, Copy, Serialize, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetriggerBehavior {
    /// Stop the previous voice and start a new one.
    #[default]
    Cut,
    /// Allow multiple voices to play simultaneously.
    Polyphonic,
}

/// A trigger that maps a MIDI event to a sample.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Clone, Serialize, Debug)]
pub struct SampleTrigger {
    /// The MIDI event that triggers the sample.
    pub(super) trigger: MidiEvent,

    /// The name of the sample to trigger (references a SampleDefinition).
    pub(super) sample: String,
}

/// Global samples configuration that can be embedded in player config or loaded from a file.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", tsify(missing_as_null, hashmap_as_object))]
#[derive(Deserialize, Clone, Serialize, Debug, Default)]
pub struct SamplesConfig {
    /// Sample definitions by name.
    #[serde(default)]
    pub(super) samples: HashMap<String, SampleDefinition>,

    /// Sample triggers.
    #[serde(default)]
    pub(super) sample_triggers: Vec<SampleTrigger>,

    /// Maximum number of concurrent sample voices globally.
    #[serde(default = "default_max_sample_voices")]
    pub(super) max_sample_voices: u32,
}

pub(super) fn default_max_sample_voices() -> u32 {
    DEFAULT_MAX_SAMPLE_VOICES
}
