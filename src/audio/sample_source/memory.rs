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

/// A sample source that produces samples from memory in planar format.
/// Useful for testing and future sample trigger functionality.
///
/// Input samples are provided as interleaved for convenience but stored planar internally.
#[cfg(test)]
pub struct MemorySampleSource {
    /// Planar sample storage (one Vec per channel)
    planar_samples: Vec<Vec<f32>>,
    /// Current position in frames
    current_frame: usize,
    channel_count: u16,
    sample_rate: u32,
}

#[cfg(test)]
impl MemorySampleSource {
    /// Creates a new memory sample source from interleaved samples.
    /// The samples are converted to planar format internally.
    pub fn new(interleaved_samples: Vec<f32>, channel_count: u16, sample_rate: u32) -> Self {
        let num_channels = channel_count as usize;
        let num_frames = if num_channels > 0 {
            interleaved_samples.len() / num_channels
        } else {
            0
        };

        // Convert interleaved to planar
        let mut planar_samples = vec![Vec::with_capacity(num_frames); num_channels];
        for frame in 0..num_frames {
            for ch in 0..num_channels {
                planar_samples[ch].push(interleaved_samples[frame * num_channels + ch]);
            }
        }

        Self {
            planar_samples,
            current_frame: 0,
            channel_count,
            sample_rate,
        }
    }

    /// Returns the total number of frames
    fn total_frames(&self) -> usize {
        self.planar_samples.first().map(|c| c.len()).unwrap_or(0)
    }
}

#[cfg(test)]
impl SampleSource for MemorySampleSource {
    fn next_chunk(
        &mut self,
        output: &mut [Vec<f32>],
        max_frames: usize,
    ) -> Result<usize, SampleSourceError> {
        let num_channels = self.channel_count as usize;

        if output.len() != num_channels {
            return Err(SampleSourceError::SampleConversionFailed(format!(
                "Output has {} channels, expected {}",
                output.len(),
                num_channels
            )));
        }

        // Clear output buffers
        for ch in output.iter_mut() {
            ch.clear();
        }

        let total_frames = self.total_frames();
        let available = total_frames.saturating_sub(self.current_frame);
        let to_copy = available.min(max_frames);

        if to_copy > 0 {
            for (ch_idx, out_ch) in output.iter_mut().enumerate() {
                if ch_idx < self.planar_samples.len() {
                    out_ch.extend_from_slice(
                        &self.planar_samples[ch_idx]
                            [self.current_frame..self.current_frame + to_copy],
                    );
                }
            }
            self.current_frame += to_copy;
        }

        Ok(to_copy)
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
        let total_frames = self.total_frames();
        let duration_secs = total_frames as f64 / self.sample_rate as f64;
        Some(std::time::Duration::from_secs_f64(duration_secs))
    }
}

#[cfg(test)]
impl SampleSourceTestExt for MemorySampleSource {
    fn is_finished(&self) -> bool {
        self.current_frame >= self.total_frames()
    }
}
