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
use std::sync::Arc;

use super::error::SampleSourceError;
use super::traits::SampleSource;

#[cfg(test)]
use super::traits::SampleSourceTestExt;

/// A sample source that produces samples from memory.
/// Used for triggered samples and testing.
pub struct MemorySampleSource {
    /// The samples stored in an Arc for efficient cloning.
    samples: Arc<Vec<f32>>,
    /// Current playback position.
    current_index: usize,
    /// Number of channels.
    channel_count: u16,
    /// Sample rate.
    sample_rate: u32,
    /// Volume scale factor (0.0 to 1.0).
    volume: f32,
}

impl MemorySampleSource {
    /// Creates a new memory sample source from shared sample data.
    /// This allows multiple playback instances to share the same sample data.
    pub fn from_shared(
        samples: Arc<Vec<f32>>,
        channel_count: u16,
        sample_rate: u32,
        volume: f32,
    ) -> Self {
        Self {
            samples,
            current_index: 0,
            channel_count,
            sample_rate,
            volume,
        }
    }
}

#[cfg(test)]
impl MemorySampleSource {
    /// Creates a new memory sample source (test only).
    pub fn new(samples: Vec<f32>, channel_count: u16, sample_rate: u32) -> Self {
        Self {
            samples: Arc::new(samples),
            current_index: 0,
            channel_count,
            sample_rate,
            volume: 1.0,
        }
    }
}

impl SampleSource for MemorySampleSource {
    fn next_sample(&mut self) -> Result<Option<f32>, SampleSourceError> {
        if self.current_index >= self.samples.len() {
            Ok(None)
        } else {
            let sample = self.samples[self.current_index] * self.volume;
            self.current_index += 1;
            Ok(Some(sample))
        }
    }

    fn channel_count(&self) -> u16 {
        self.channel_count
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn bits_per_sample(&self) -> u16 {
        32 // Memory samples are 32-bit float
    }

    fn sample_format(&self) -> crate::audio::SampleFormat {
        crate::audio::SampleFormat::Float
    }

    fn duration(&self) -> Option<std::time::Duration> {
        let total_samples = self.samples.len() as f64;
        let samples_per_channel = total_samples / self.channel_count as f64;
        let duration_secs = samples_per_channel / self.sample_rate as f64;
        Some(std::time::Duration::from_secs_f64(duration_secs))
    }
}

impl Clone for MemorySampleSource {
    fn clone(&self) -> Self {
        Self {
            samples: self.samples.clone(),
            current_index: 0, // Start from the beginning
            channel_count: self.channel_count,
            sample_rate: self.sample_rate,
            volume: self.volume,
        }
    }
}

#[cfg(test)]
impl SampleSourceTestExt for MemorySampleSource {
    fn is_finished(&self) -> bool {
        self.current_index >= self.samples.len()
    }
}
