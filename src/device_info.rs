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

//! The audio and MIDI devices and the trigger calibration as the web UI is
//! told of them: light, so its WebAssembly module compiles them as they are.

use serde::Serialize;

/// Serializable info about an audio device for the web UI.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub max_channels: u16,
    pub host_name: String,
    pub supported_sample_rates: Vec<u32>,
    pub supported_formats: Vec<SupportedFormat>,
    /// Whether this is an ALSA plug/virtual node. Such a node is a legitimate
    /// and sometimes mandatory choice, so this labels it rather than hiding it.
    pub virtual_node: bool,
    /// Whether `max_channels` is the device's real maximum. False when the
    /// count is cpal's clamp and no hardware node was available to resolve it,
    /// in which case the true figure is `max_channels` or more.
    pub channels_known: bool,
}

/// A supported sample format for an audio device.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, PartialEq, Eq, Hash)]
pub struct SupportedFormat {
    pub sample_format: String,
    pub bits_per_sample: u32,
}

/// Serializable info about a MIDI device for the web UI.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize)]
pub struct MidiDeviceInfo {
    pub name: String,
    pub has_input: bool,
    pub has_output: bool,
}

/// Noise floor statistics for a single channel.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize)]
pub struct NoiseFloorStats {
    pub peak: f32,
    pub rms: f32,
    pub low_freq_energy: f32,
}

/// Calibrated parameters for a single channel.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize)]
pub struct ChannelCalibration {
    pub channel: u16,
    pub threshold: f32,
    pub gain: f32,
    pub scan_time_ms: u32,
    pub retrigger_time_ms: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highpass_freq: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_threshold_decay_ms: Option<u32>,
    pub num_hits_detected: usize,
    pub noise_floor_peak: f32,
    pub max_hit_amplitude: f32,
}
