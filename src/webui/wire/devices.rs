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

//! The devices API's payloads (`/api/devices`, `/api/calibrate`).

use serde::{Deserialize, Serialize};

use crate::config::StreamBufferSize;

use crate::player::AudioOutputHealth;

/// `POST /api/devices/audio/probe`: whether a device can stream. A device
/// this player already holds is reported from its health (`in_use`), not
/// opened again; the fields a case does not decide are absent.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct AudioProbe {
    pub outcome: ProbeKind,
    /// Whether the device proved it can stream.
    pub ok: bool,
    /// Why it could not be opened; `null` but for `could_not_open`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Option<String>>,
    pub callbacks: Option<u64>,
    /// The held device's health, for `in_use`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub health: Option<Option<AudioOutputHealth>>,
}

/// What a probe found.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProbeKind {
    Streaming,
    OpenedButSilent,
    CouldNotOpen,
    OpenedCannotReport,
    /// Already open by this player, so reported rather than probed.
    InUse,
}

/// `POST /api/calibrate/start`: a channel's noise floor, and the stream it
/// was measured on.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct NoiseFloor {
    pub peak: f32,
    pub rms: f32,
    pub low_freq_energy: f32,
    pub channel: u16,
    pub sample_rate: u32,
    pub device_channels: u16,
}

/// What the config editor asks to have probed: a device name, plus whichever
/// format settings are filled in beside it.
///
/// Everything but the name is optional and mirrors the profile's audio block,
/// so the probe opens the device the way saving this profile would. Probing
/// with defaults would answer a question nobody asked — a WING opens fine at
/// some formats and refuses the one that matters.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Clone, Debug, PartialEq)]
pub struct ProbeAudioRequest {
    pub device: String,
    #[serde(default)]
    pub sample_rate: Option<u32>,
    #[serde(default)]
    pub sample_format: Option<String>,
    #[serde(default)]
    pub bits_per_sample: Option<u16>,
    #[serde(default)]
    pub buffer_size: Option<usize>,
    #[serde(default)]
    #[cfg_attr(
        feature = "tsify",
        tsify(type = "\"default\" | \"min\" | number | null")
    )]
    pub stream_buffer_size: Option<StreamBufferSize>,
}
