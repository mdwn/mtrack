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
use crate::audio::TargetFormat;
use rubato::{
    SincFixedIn, SincInterpolationParameters, SincInterpolationType, VecResampler, WindowFunction,
};
use std::sync::Mutex;

use super::error::SampleSourceError;
use super::traits::SampleSource;

// Import VecResampler trait to bring methods into scope for method resolution
#[allow(unused_imports)]
use rubato::VecResampler as _;

// Resampling configuration constants
/// Input block size for the sinc resampler.
const INPUT_BLOCK_SIZE: usize = 1024;

/// Sliding-window input buffer for streaming resampling (planar format)
struct PlanarInputBuffer {
    /// Per-channel input samples (sliding window)
    channels: Vec<Vec<f32>>,
    /// Whether source has reached EOF
    source_finished: bool,
}

impl PlanarInputBuffer {
    fn new(num_channels: usize) -> Self {
        Self {
            channels: vec![Vec::new(); num_channels],
            source_finished: false,
        }
    }

    /// Number of frames currently in the buffer
    fn len(&self) -> usize {
        self.channels.first().map(|c| c.len()).unwrap_or(0)
    }

    /// Append planar frames from source
    fn push_planar(&mut self, planar_input: &[Vec<f32>], num_frames: usize) {
        for (ch_idx, ch) in self.channels.iter_mut().enumerate() {
            if ch_idx < planar_input.len() {
                let frames_to_copy = num_frames.min(planar_input[ch_idx].len());
                ch.extend_from_slice(&planar_input[ch_idx][..frames_to_copy]);
            }
        }
    }

    /// Drain the first `n` frames from all channels
    fn drain_frames(&mut self, n: usize) {
        for ch in &mut self.channels {
            ch.drain(0..n.min(ch.len()));
        }
    }
}

/// Planar FIFO output buffer for streaming sample delivery
struct PlanarOutputFifo {
    /// Per-channel output samples ready for consumption
    channels: Vec<Vec<f32>>,
    /// Current read position (in frames)
    read_pos: usize,
}

impl PlanarOutputFifo {
    fn new(num_channels: usize) -> Self {
        Self {
            channels: vec![Vec::new(); num_channels],
            read_pos: 0,
        }
    }

    /// Number of frames available to read
    fn available_frames(&self) -> usize {
        self.channels
            .first()
            .map(|c| c.len().saturating_sub(self.read_pos))
            .unwrap_or(0)
    }

    /// Drain frames into planar output buffers, returns number of frames written
    fn drain_to_planar(&mut self, output: &mut [Vec<f32>], max_frames: usize) -> usize {
        let available = self.available_frames();
        let to_copy = available.min(max_frames);

        if to_copy > 0 {
            for (ch_idx, out_ch) in output.iter_mut().enumerate() {
                if ch_idx < self.channels.len() {
                    out_ch.extend_from_slice(
                        &self.channels[ch_idx][self.read_pos..self.read_pos + to_copy],
                    );
                }
            }
            self.read_pos += to_copy;

            // Compact buffers if we've consumed a lot
            if self.read_pos > 4096 {
                for ch in self.channels.iter_mut() {
                    ch.drain(..self.read_pos);
                }
                self.read_pos = 0;
            }
        }
        to_copy
    }

    /// Append frames from resampler output (already planar)
    fn push_planar(&mut self, per_channel: &[Vec<f32>], num_frames: usize) {
        for (ch_idx, ch) in self.channels.iter_mut().enumerate() {
            if ch_idx < per_channel.len() {
                let frames_to_copy = num_frames.min(per_channel[ch_idx].len());
                ch.extend_from_slice(&per_channel[ch_idx][..frames_to_copy]);
            }
        }
    }
}

/// Audio transcoder with rubato resampling (planar format throughout)
pub struct AudioTranscoder<S: SampleSource> {
    source: S,
    /// Sinc resampler wrapped in Mutex for Sync
    pub resampler: Option<Mutex<SincFixedIn<f32>>>,
    pub source_rate: u32,
    pub target_rate: u32,
    target_bits_per_sample: u16,
    channels: u16,

    /// Sliding window of input samples (planar)
    input_buffer: PlanarInputBuffer,
    /// FIFO of output samples ready for consumption (planar)
    output_fifo: PlanarOutputFifo,
    /// Temporary buffer for resampler output (reused to avoid allocation)
    output_scratch: Vec<Vec<f32>>,
    /// Temporary buffer for reading from source (planar, reused)
    source_planar_buffer: Vec<Vec<f32>>,
}

impl<S> SampleSource for AudioTranscoder<S>
where
    S: SampleSource,
{
    fn next_chunk(
        &mut self,
        output: &mut [Vec<f32>],
        max_frames: usize,
    ) -> Result<usize, SampleSourceError> {
        // If no resampler, just pass through directly
        if self.resampler.is_none() {
            return self.source.next_chunk(output, max_frames);
        }

        let num_channels = self.channels as usize;
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

        let mut total_frames = 0;

        while total_frames < max_frames {
            // First, drain any available frames from the output FIFO
            let drained = self
                .output_fifo
                .drain_to_planar(output, max_frames - total_frames);
            total_frames += drained;

            if total_frames >= max_frames {
                break;
            }

            // Output FIFO depleted - need to process more input
            let had_output = self.fill_output_fifo()?;

            // If fill_output_fifo didn't produce any output and source is done, we're finished
            if !had_output
                && self.input_buffer.source_finished
                && self.output_fifo.available_frames() == 0
            {
                break;
            }
        }

        Ok(total_frames)
    }

    fn channel_count(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.target_rate
    }

    fn bits_per_sample(&self) -> u16 {
        self.target_bits_per_sample
    }

    fn sample_format(&self) -> crate::audio::SampleFormat {
        crate::audio::SampleFormat::Float
    }

    fn duration(&self) -> Option<std::time::Duration> {
        self.source.duration()
    }
}

impl<S> AudioTranscoder<S>
where
    S: SampleSource,
{
    /// Creates a new AudioTranscoder with a SampleSource
    pub fn new(
        source: S,
        source_format: &TargetFormat,
        target_format: &TargetFormat,
        channels: u16,
    ) -> Result<Self, SampleSourceError> {
        let needs_resampling = source_format.sample_rate != target_format.sample_rate;

        let (resampler, output_scratch) = if needs_resampling {
            let sinc_params = SincInterpolationParameters {
                sinc_len: 256,
                f_cutoff: 0.95,
                oversampling_factor: 128,
                interpolation: SincInterpolationType::Linear,
                window: WindowFunction::BlackmanHarris2,
            };
            let resample_ratio =
                target_format.sample_rate as f64 / source_format.sample_rate as f64;

            let r = SincFixedIn::<f32>::new(
                resample_ratio,
                1.0,
                sinc_params,
                INPUT_BLOCK_SIZE,
                channels as usize,
            )
            .map_err(|_e| {
                SampleSourceError::ResamplingFailed(
                    source_format.sample_rate,
                    target_format.sample_rate,
                )
            })?;

            let scratch = r.output_buffer_allocate(true);
            (Some(Mutex::new(r)), scratch)
        } else {
            (None, Vec::new())
        };

        // Pre-allocate planar buffer for reading from source
        let source_planar_buffer = vec![Vec::with_capacity(INPUT_BLOCK_SIZE); channels as usize];

        Ok(AudioTranscoder {
            source,
            resampler,
            source_rate: source_format.sample_rate,
            target_rate: target_format.sample_rate,
            target_bits_per_sample: target_format.bits_per_sample,
            channels,
            input_buffer: PlanarInputBuffer::new(channels as usize),
            output_fifo: PlanarOutputFifo::new(channels as usize),
            output_scratch,
            source_planar_buffer,
        })
    }

    /// Fill the output FIFO by reading from source and processing through resampler.
    /// Returns true if any output was produced.
    fn fill_output_fifo(&mut self) -> Result<bool, SampleSourceError> {
        let resampler_mutex = match self.resampler.as_ref() {
            Some(r) => r,
            None => return Ok(false),
        };

        // 1. Try to fill input buffer from source
        if !self.input_buffer.source_finished {
            let input_frames_needed = {
                let r = resampler_mutex.lock().unwrap();
                r.input_frames_next()
            };

            // Read planar chunks from source until we have enough frames
            while self.input_buffer.len() < input_frames_needed {
                let frames_needed = input_frames_needed - self.input_buffer.len();

                // Read planar data from source
                let frames_read = self
                    .source
                    .next_chunk(&mut self.source_planar_buffer, frames_needed)?;

                if frames_read == 0 {
                    self.input_buffer.source_finished = true;
                    break;
                }

                // Add to input buffer (already planar, no conversion needed!)
                self.input_buffer
                    .push_planar(&self.source_planar_buffer, frames_read);
            }
        }

        // 2. Process if we have enough input
        let mut resampler = resampler_mutex.lock().unwrap();
        let input_frames_needed = resampler.input_frames_next();

        if self.input_buffer.len() >= input_frames_needed {
            let (nbr_in, nbr_out) = resampler
                .process_into_buffer(
                    &self.input_buffer.channels,
                    &mut self.output_scratch,
                    None,
                )
                .map_err(|_e| {
                    SampleSourceError::ResamplingFailed(self.source_rate, self.target_rate)
                })?;

            drop(resampler);

            self.input_buffer.drain_frames(nbr_in);

            if nbr_out > 0 {
                // Output is already planar from rubato, just copy to FIFO
                self.output_fifo.push_planar(&self.output_scratch, nbr_out);
                return Ok(true);
            }

            if nbr_in == 0 {
                return Ok(false);
            }
            return Ok(false);
        } else if self.input_buffer.source_finished {
            // 3. Source finished - process any remaining input
            if self.input_buffer.len() == 0 {
                return Ok(false);
            }

            let (_nbr_in, nbr_out) = resampler
                .process_partial_into_buffer(
                    Some(&self.input_buffer.channels as &[Vec<f32>]),
                    &mut self.output_scratch,
                    None,
                )
                .map_err(|_e| {
                    SampleSourceError::ResamplingFailed(self.source_rate, self.target_rate)
                })?;

            drop(resampler);

            self.input_buffer.drain_frames(self.input_buffer.len());

            if nbr_out > 0 {
                self.output_fifo.push_planar(&self.output_scratch, nbr_out);
                return Ok(true);
            }

            return Ok(false);
        }

        Ok(false)
    }
}
