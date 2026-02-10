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
use std::{error::Error, str::FromStr, time::Duration};

use duration_string::DurationString;
use serde::Deserialize;

use crate::audio::SampleFormat;

const DEFAULT_AUDIO_PLAYBACK_DELAY: Duration = Duration::ZERO;
const DEFAULT_BUFFER_SIZE: usize = 1024;
const DEFAULT_BUFFER_THREADS: usize = 2;

/// How to choose the CPAL stream buffer size (period size). Affects latency vs underrun tolerance.
#[derive(Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum StreamBufferSize {
    /// Use the backend's default (may be high latency on some systems).
    #[serde(rename = "default")]
    Default,
    /// Use the device's minimum supported period size (lowest latency, most jitter-sensitive).
    #[serde(rename = "min")]
    Min,
    /// Use a fixed size in frames (same as buffer_size when not set).
    Fixed(usize),
}

/// A YAML representation of the audio configuration.
#[derive(Deserialize, Clone)]
pub struct Audio {
    /// The audio device.
    device: String,

    /// Controls how long to wait before playback of an audio file starts.
    playback_delay: Option<String>,

    /// Target sample rate in Hz (default: 44100)
    sample_rate: Option<u32>,

    /// Target sample format (default: "int")
    sample_format: Option<String>,

    /// Target bits per sample (default: 32)
    bits_per_sample: Option<u16>,

    /// Buffer size for decoded audio samples (default: 1024 samples per channel)
    buffer_size: Option<usize>,

    /// CPAL stream buffer: "default" (backend default), "min" (lowest latency), or a number (frames).
    /// When unset, uses buffer_size. Lower values = lower latency but more sensitive to callback jitter.
    stream_buffer_size: Option<StreamBufferSize>,

    /// Number of worker threads for buffered song sources.
    /// Defaults to a small fixed value; must be >= 1.
    buffer_threads: Option<usize>,
}

impl Audio {
    /// New will create a new Audio configuration.
    pub fn new(device: &str) -> Audio {
        Audio {
            device: device.to_string(),
            playback_delay: None,
            sample_rate: None,
            sample_format: None,
            bits_per_sample: None,
            buffer_size: None,
            stream_buffer_size: None,
            buffer_threads: None,
        }
    }

    /// Returns the device from the configuration.
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Returns the playback delay from the configuration.
    pub fn playback_delay(&self) -> Result<Duration, Box<dyn Error>> {
        match &self.playback_delay {
            Some(playback_delay) => Ok(DurationString::from_string(playback_delay.clone())?.into()),
            None => Ok(DEFAULT_AUDIO_PLAYBACK_DELAY),
        }
    }

    /// Returns the target sample rate (default: 44100)
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate.unwrap_or(44100)
    }

    /// Returns the target sample format (default: Float)
    pub fn sample_format(&self) -> Result<SampleFormat, Box<dyn Error>> {
        match self.sample_format.as_deref() {
            Some(format) => SampleFormat::from_str(format),
            None => Ok(SampleFormat::Int),
        }
    }

    /// Returns the target bits per sample (default: 32)
    pub fn bits_per_sample(&self) -> u16 {
        self.bits_per_sample.unwrap_or(32)
    }

    /// Returns the buffer size for decoded audio samples (default: 1024 samples per channel)
    pub fn buffer_size(&self) -> usize {
        self.buffer_size.unwrap_or(DEFAULT_BUFFER_SIZE)
    }

    /// Returns the number of worker threads used for buffered song sources.
    pub fn buffer_threads(&self) -> usize {
        self.buffer_threads.unwrap_or(DEFAULT_BUFFER_THREADS).max(1)
    }

    /// Returns the stream buffer size choice for CPAL (default/min/fixed).
    /// When None, the stream uses buffer_size() as a fixed frame count.
    pub fn stream_buffer_size(&self) -> Option<StreamBufferSize> {
        self.stream_buffer_size.clone()
    }
}
