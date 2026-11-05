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

//! A profile's `audio:` section as written: light, so the web UI's WebAssembly module compiles it as it is
//! and the UI edits it by the generated types. What it means is `audio`'s.

use serde::{Deserialize, Serialize};

/// Which resampling algorithm to use when source and output sample rates differ.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResamplerType {
    /// High-quality sinc interpolation (lower latency, higher CPU). This is the default.
    #[default]
    Sinc,
    /// FFT-based resampling (considerably faster for fixed-ratio resampling).
    Fft,
}

/// How to choose the CPAL stream buffer size (period size). Affects latency vs underrun tolerance.
///
/// Accepts `min`, `default`, or a frame count:
///
/// ```yaml
/// stream_buffer_size: min       # smallest period the device supports
/// stream_buffer_size: default   # let the backend choose
/// stream_buffer_size: 1024      # a fixed number of frames
/// ```
///
/// Deserialized by hand rather than with `#[serde(untagged)]`. Untagged matches
/// by shape and ignores variant renames, so `#[serde(rename = "min")]` on a unit
/// variant did nothing — a unit variant matches only `null` — and both keywords
/// failed to parse at all, while the web UI's own tooltip told people to type
/// them. It also round-tripped `Min` through `null` back to `Default`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StreamBufferSize {
    /// Use the backend's default (may be high latency on some systems).
    Default,
    /// Use the device's minimum supported period size (lowest latency, most jitter-sensitive).
    Min,
    /// Use a fixed size in frames (same as buffer_size when not set).
    Fixed(usize),
}

impl StreamBufferSize {
    /// The keyword accepted for this variant, if it has one.
    fn keyword(&self) -> Option<&'static str> {
        match self {
            StreamBufferSize::Default => Some("default"),
            StreamBufferSize::Min => Some("min"),
            StreamBufferSize::Fixed(_) => None,
        }
    }
}

impl Serialize for StreamBufferSize {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            StreamBufferSize::Fixed(frames) => serializer.serialize_u64(*frames as u64),
            other => serializer.serialize_str(other.keyword().expect("non-Fixed has a keyword")),
        }
    }
}

impl<'de> Deserialize<'de> for StreamBufferSize {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// What a config file is allowed to contain here.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Repr {
            Keyword(String),
            Frames(usize),
        }

        match Repr::deserialize(deserializer)? {
            Repr::Frames(frames) => Ok(StreamBufferSize::Fixed(frames)),
            Repr::Keyword(word) if word.eq_ignore_ascii_case("min") => Ok(StreamBufferSize::Min),
            Repr::Keyword(word) if word.eq_ignore_ascii_case("default") => {
                Ok(StreamBufferSize::Default)
            }
            Repr::Keyword(word) => Err(serde::de::Error::custom(format!(
                "unknown stream_buffer_size {word:?}: expected \"min\", \"default\", \
                 or a number of frames"
            ))),
        }
    }
}

/// A YAML representation of the audio configuration.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(missing_as_null, hashmap_as_object)
)]
#[derive(Deserialize, Serialize, Clone)]
pub struct Audio {
    /// The audio device. Defaults to empty rather than failing
    /// deserialization so validate() can report a readable error.
    #[serde(default)]
    pub(super) device: String,

    /// Controls how long to wait before playback of an audio file starts.
    #[serde(default)]
    pub(super) playback_delay: Option<String>,

    /// Target sample rate in Hz (default: 44100)
    #[serde(default)]
    pub(super) sample_rate: Option<u32>,

    /// Target sample format (default: "int")
    #[serde(default)]
    pub(super) sample_format: Option<String>,

    /// Target bits per sample (default: 32)
    #[serde(default)]
    pub(super) bits_per_sample: Option<u16>,

    /// Buffer size for decoded audio samples (default: 1024 samples per channel)
    #[serde(default)]
    pub(super) buffer_size: Option<usize>,

    /// CPAL stream buffer: "default" (backend default), "min" (lowest latency), or a number (frames).
    /// When unset, uses buffer_size. Lower values = lower latency but more sensitive to callback jitter.
    #[serde(default)]
    #[cfg_attr(feature = "tsify", tsify(type = "\"default\" | \"min\" | number"))]
    pub(super) stream_buffer_size: Option<StreamBufferSize>,

    /// Number of worker threads for buffered song sources.
    /// Defaults to a small fixed value; must be >= 1.
    #[serde(default)]
    pub(super) buffer_threads: Option<usize>,

    /// Resampling algorithm: "sinc" (default, high quality) or "fft" (faster on low-power hardware).
    #[serde(default)]
    pub(super) resampler: Option<ResamplerType>,
}
