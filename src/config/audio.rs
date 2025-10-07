// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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
use std::{error::Error, time::Duration};

use duration_string::DurationString;
use hound::SampleFormat;
use serde::Deserialize;

const DEFAULT_AUDIO_PLAYBACK_DELAY: Duration = Duration::ZERO;

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

    /// Returns the target sample format (default: Int)
    pub fn sample_format(&self) -> Result<SampleFormat, Box<dyn Error>> {
        match self.sample_format.as_deref() {
            Some("float") | Some("Float") => Ok(SampleFormat::Float),
            Some("int") | Some("Int") => Ok(SampleFormat::Int),
            Some(format) => Err(format!("Unsupported sample format: {}", format).into()),
            None => Ok(SampleFormat::Int), // Default to integer
        }
    }

    /// Returns the target bits per sample (default: 32)
    pub fn bits_per_sample(&self) -> u16 {
        self.bits_per_sample.unwrap_or(32)
    }
}
