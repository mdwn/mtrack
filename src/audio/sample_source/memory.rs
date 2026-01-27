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
// These imports are used in the impl blocks below, but the linter may not see them
// in non-test mode due to #[cfg(test)] blocks
#[allow(unused_imports)]
use super::error::SampleSourceError;
#[allow(unused_imports)]
use super::traits::SampleSource;

#[cfg(test)]
use super::traits::SampleSourceTestExt;

/// A sample source that produces samples from memory
/// Useful for testing and future sample trigger functionality
#[cfg(test)]
pub struct MemorySampleSource {
    samples: Vec<f32>,
    current_index: usize,
    channel_count: u16,
    sample_rate: u32,
}

#[cfg(test)]
impl MemorySampleSource {
    /// Creates a new memory sample source
    pub fn new(samples: Vec<f32>, channel_count: u16, sample_rate: u32) -> Self {
        Self {
            samples,
            current_index: 0,
            channel_count,
            sample_rate,
        }
    }
}

#[cfg(test)]
impl SampleSource for MemorySampleSource {
    fn next_sample(&mut self) -> Result<Option<f32>, SampleSourceError> {
        if self.current_index >= self.samples.len() {
            Ok(None)
        } else {
            let sample = self.samples[self.current_index];
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
        32 // Memory samples are typically 32-bit float
    }

    fn sample_format(&self) -> crate::audio::SampleFormat {
        crate::audio::SampleFormat::Float // Memory samples are float
    }

    fn duration(&self) -> Option<std::time::Duration> {
        // Calculate duration from sample count and sample rate
        // For interleaved samples, we need to account for the channel count
        let total_samples = self.samples.len() as f64;
        let samples_per_channel = total_samples / self.channel_count as f64;
        let duration_secs = samples_per_channel / self.sample_rate as f64;
        Some(std::time::Duration::from_secs_f64(duration_secs))
    }
}

#[cfg(test)]
impl SampleSourceTestExt for MemorySampleSource {
    fn is_finished(&self) -> bool {
        self.current_index >= self.samples.len()
    }
}
