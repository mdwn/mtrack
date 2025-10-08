use std::any::Any;
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
use hound::SampleFormat;
use std::{error::Error, fmt, sync::Arc};

use crate::config;
use crate::playsync::CancelHandle;
use crate::songs::Song;
use std::collections::HashMap;
use std::sync::Barrier;

mod cpal;
pub mod mock;
pub mod sample_source;

/// Target audio format for transcoding
#[derive(Debug, Clone, PartialEq)]
pub struct TargetFormat {
    /// Sample rate in Hz
    pub sample_rate: u32,
    /// Sample format (integer or float)
    pub sample_format: SampleFormat,
    /// Bits per sample
    pub bits_per_sample: u16,
}

impl TargetFormat {
    /// Creates a new TargetFormat
    pub fn new(
        sample_rate: u32,
        sample_format: SampleFormat,
        bits_per_sample: u16,
    ) -> Result<Self, Box<dyn Error>> {
        // Basic sanity check - let the audio interface decide what's actually supported
        if sample_rate == 0 {
            return Err("Sample rate must be greater than 0".into());
        }

        Ok(TargetFormat {
            sample_rate,
            sample_format,
            bits_per_sample,
        })
    }
}

impl Default for TargetFormat {
    /// Creates a default target format (44.1kHz, 16-bit integer)
    fn default() -> Self {
        TargetFormat {
            sample_rate: 44100,
            sample_format: SampleFormat::Int,
            bits_per_sample: 16,
        }
    }
}

#[cfg(test)]
impl TargetFormat {
    /// Returns the bytes per sample for this format
    pub fn bytes_per_sample(&self) -> usize {
        self.bits_per_sample as usize / 8
    }

    /// Returns the bytes per second for this format (for a single channel)
    pub fn bytes_per_second(&self) -> usize {
        self.sample_rate as usize * self.bytes_per_sample()
    }

    /// Returns a human-readable description of this format
    pub fn description(&self) -> String {
        let format_str = match self.sample_format {
            SampleFormat::Int => "Integer",
            SampleFormat::Float => "Float",
        };
        format!(
            "{}Hz, {}-bit {}",
            self.sample_rate, self.bits_per_sample, format_str
        )
    }
}

/// An audio device that can play songs back.
pub trait Device: Any + fmt::Display + std::marker::Send + std::marker::Sync {
    /// Plays the given song through the audio interface.
    fn play(
        &self,
        song: Arc<Song>,
        mappings: &HashMap<String, Vec<u16>>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
    ) -> Result<(), Box<dyn Error>>;

    #[cfg(test)]
    fn to_mock(&self) -> Result<Arc<mock::Device>, Box<dyn Error>>;
}

/// Lists devices known to cpal.
pub fn list_devices() -> Result<Vec<Box<dyn Device>>, Box<dyn Error>> {
    cpal::Device::list()
}

/// Gets a device with the given name.
pub fn get_device(config: Option<config::Audio>) -> Result<Arc<dyn Device>, Box<dyn Error>> {
    let config = match config {
        Some(config) => config,
        None => return Err("there must be an audio device specified".into()),
    };

    let device = config.device();
    if device.starts_with("mock") {
        return Ok(Arc::new(mock::Device::get(device)));
    };

    Ok(Arc::new(cpal::Device::get(config)?))
}
