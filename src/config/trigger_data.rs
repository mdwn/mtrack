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

//! A profile's `trigger:` section as written: light, so the web UI's WebAssembly module compiles it as it is
//! and the UI edits it by the generated types. What it means is `trigger`'s.

use serde::{Deserialize, Serialize};

pub(super) fn default_threshold() -> f32 {
    0.1
}

pub(super) fn default_retrigger_time_ms() -> u32 {
    30
}

pub(super) fn default_scan_time_ms() -> u32 {
    5
}

pub(super) fn default_gain() -> f32 {
    1.0
}

pub(super) fn default_fixed_velocity() -> u8 {
    127
}

pub(super) fn default_noise_floor_decay_ms() -> u32 {
    200
}

/// Configuration for the trigger system.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct TriggerConfig {
    /// The audio input device name (matched against cpal input devices).
    /// Only required when audio inputs are configured.
    #[serde(default)]
    pub(super) device: Option<String>,
    /// Optional sample rate override for the input stream.
    #[serde(default)]
    pub(super) sample_rate: Option<u32>,
    /// Target sample format ("int" or "float"). Default: device native.
    #[serde(default)]
    pub(super) sample_format: Option<String>,
    /// Target bits per sample (16 or 32). Default: device native.
    #[serde(default)]
    pub(super) bits_per_sample: Option<u16>,
    /// Stream buffer size in frames. Default: device default.
    #[serde(default)]
    pub(super) buffer_size: Option<usize>,
    /// Per-channel input configurations.
    #[serde(default)]
    pub(super) inputs: Vec<TriggerInput>,
    /// Crosstalk suppression window in ms after any channel fires.
    /// Both `crosstalk_window_ms` and `crosstalk_threshold` must be set to enable suppression.
    #[serde(default)]
    pub(super) crosstalk_window_ms: Option<u32>,
    /// Threshold multiplier during crosstalk suppression window.
    /// Both `crosstalk_window_ms` and `crosstalk_threshold` must be set to enable suppression.
    #[serde(default)]
    pub(super) crosstalk_threshold: Option<f32>,
}

/// A trigger input, discriminated by `kind`.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerInput {
    /// Audio trigger input (piezo, line-level, etc.).
    Audio(AudioTriggerInput),
    /// MIDI event trigger input.
    Midi(MidiTriggerInput),
}

/// What action an input channel performs when triggered.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerInputAction {
    /// Fire a sample (default).
    #[default]
    Trigger,
    /// Release voices in the named release group.
    Release,
}

/// How amplitude maps to velocity (0-127).
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VelocityCurve {
    /// Linear mapping: velocity = peak * 127.
    #[default]
    Linear,
    /// Logarithmic mapping: threshold→1.0 maps log-scaled to 1→127.
    Logarithmic,
    /// Fixed velocity: always returns the configured fixed_velocity.
    Fixed,
}

/// Configuration for a single audio input channel used as a trigger.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct AudioTriggerInput {
    /// 1-indexed input channel number.
    pub(super) channel: u16,
    /// Sample name to trigger (from samples config). Required when action is "trigger".
    #[serde(default)]
    pub(super) sample: Option<String>,
    /// Minimum amplitude to trigger (0.0-1.0).
    #[serde(default = "default_threshold")]
    pub(super) threshold: f32,
    /// Lockout period after trigger fires, in milliseconds.
    #[serde(default = "default_retrigger_time_ms")]
    pub(super) retrigger_time_ms: u32,
    /// Peak detection window after threshold crossing, in milliseconds.
    #[serde(default = "default_scan_time_ms")]
    pub(super) scan_time_ms: u32,
    /// Input gain multiplier.
    #[serde(default = "default_gain")]
    pub(super) gain: f32,
    /// How amplitude maps to velocity.
    #[serde(default)]
    pub(super) velocity_curve: VelocityCurve,
    /// Velocity value when velocity_curve is "fixed".
    #[serde(default = "default_fixed_velocity")]
    pub(super) fixed_velocity: u8,
    /// Optional release group name for voice management.
    #[serde(default)]
    pub(super) release_group: Option<String>,
    /// What action this input performs (trigger or release).
    #[serde(default)]
    pub(super) action: TriggerInputAction,
    /// High-pass filter cutoff frequency in Hz.
    #[serde(default)]
    pub(super) highpass_freq: Option<f32>,
    /// Decay time for adaptive/dynamic threshold in ms.
    #[serde(default)]
    pub(super) dynamic_threshold_decay_ms: Option<u32>,
    /// Noise floor tracking sensitivity multiplier. The adaptive noise floor
    /// estimate is multiplied by this value to derive a minimum threshold.
    /// Default: None (disabled). Typical value: 5.0.
    #[serde(default)]
    pub(super) noise_floor_sensitivity: Option<f32>,
    /// Time constant for noise floor EMA in ms. Controls how quickly the
    /// estimate tracks changing ambient levels.
    #[serde(default = "default_noise_floor_decay_ms")]
    pub(super) noise_floor_decay_ms: u32,
}

/// Configuration for a MIDI event trigger input.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct MidiTriggerInput {
    /// The MIDI event to listen for.
    pub(super) event: super::midi_event::MidiEvent,
    /// The sample name to trigger.
    pub(super) sample: String,
}
