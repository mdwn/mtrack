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
// The `as _` prevents the "unused import" warning while still bringing trait methods into scope
#[allow(unused_imports)]
use rubato::VecResampler as _;

// Resampling configuration constants
/// Input block size for the sinc resampler.
/// Smaller blocks = lower latency. Sinc resampling has much lower latency than FFT.
/// 1024 provides a good balance (~21ms latency at 48kHz).
const INPUT_BLOCK_SIZE: usize = 1024;

/// Sliding-window input buffer for streaming resampling
/// Matches the clean rubato usage pattern: accumulate input, process when ready, drain consumed
struct SlidingInputBuffer {
    /// Per-channel input samples (sliding window)
    channels: Vec<Vec<f32>>,
    /// Whether source has reached EOF
    source_finished: bool,
}

impl SlidingInputBuffer {
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

    /// Append a frame (one sample per channel)
    fn push_frame(&mut self, frame: &[f32]) {
        for (ch, &sample) in self.channels.iter_mut().zip(frame.iter()) {
            ch.push(sample);
        }
    }

    /// Drain the first `n` frames from all channels
    fn drain_frames(&mut self, n: usize) {
        for ch in &mut self.channels {
            ch.drain(0..n.min(ch.len()));
        }
    }
}

/// FIFO output buffer for streaming sample delivery
struct OutputFifo {
    /// Interleaved output samples ready for consumption
    samples: std::collections::VecDeque<f32>,
}

impl OutputFifo {
    fn new() -> Self {
        Self {
            samples: std::collections::VecDeque::new(),
        }
    }

    /// Pop the next sample
    fn pop(&mut self) -> Option<f32> {
        self.samples.pop_front()
    }

    /// Append frames from per-channel buffers (interleaved)
    fn push_frames(&mut self, per_channel: &[Vec<f32>], num_frames: usize) {
        for frame_idx in 0..num_frames {
            for ch in per_channel {
                if let Some(&sample) = ch.get(frame_idx) {
                    self.samples.push_back(sample);
                }
            }
        }
    }
}

/// Audio transcoder with rubato resampling
/// Takes a SampleSource and resamples its output to the target format
///
/// Uses a streaming sliding-window approach that matches rubato's expected usage:
/// - Accumulate input samples until we have enough for a processing block
/// - Process, drain consumed input, append output to FIFO
/// - Return samples from output FIFO one at a time
pub struct AudioTranscoder<S: SampleSource> {
    source: S,
    /// Sinc resampler wrapped in Mutex for Sync (contains non-Sync internals)
    pub resampler: Option<Mutex<SincFixedIn<f32>>>,
    pub source_rate: u32,
    pub target_rate: u32,
    target_bits_per_sample: u16,
    channels: u16,

    /// Sliding window of input samples (per-channel)
    input_buffer: SlidingInputBuffer,
    /// FIFO of output samples ready for consumption
    output_fifo: OutputFifo,
    /// Temporary buffer for resampler output (reused to avoid allocation)
    output_scratch: Vec<Vec<f32>>,
}

impl<S> SampleSource for AudioTranscoder<S>
where
    S: SampleSource,
{
    fn next_sample(&mut self) -> Result<Option<f32>, SampleSourceError> {
        // If no resampler, just pass through directly
        if self.resampler.is_none() {
            return self.source.next_sample();
        }

        // Try to return from output FIFO first
        if let Some(sample) = self.output_fifo.pop() {
            return Ok(Some(sample));
        }

        // Output FIFO empty - need to process more input
        self.fill_output_fifo()?;

        // Try again after processing
        Ok(self.output_fifo.pop())
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
        crate::audio::SampleFormat::Float // AudioTranscoder outputs float samples
    }

    fn duration(&self) -> Option<std::time::Duration> {
        // Delegate to the underlying source - transcoding doesn't change duration
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
            // Use sinc resampling for lower latency and high quality.
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
                1.0, // max_resample_ratio_relative: no dynamic changes
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

        Ok(AudioTranscoder {
            source,
            resampler,
            source_rate: source_format.sample_rate,
            target_rate: target_format.sample_rate,
            target_bits_per_sample: target_format.bits_per_sample,
            channels,
            input_buffer: SlidingInputBuffer::new(channels as usize),
            output_fifo: OutputFifo::new(),
            output_scratch,
        })
    }

    /// Fill the output FIFO by reading from source and processing through resampler.
    /// This uses rubato's standard process_into_buffer pattern for streaming resampling.
    fn fill_output_fifo(&mut self) -> Result<(), SampleSourceError> {
        let resampler_mutex = match self.resampler.as_ref() {
            Some(r) => r,
            None => return Ok(()), // No resampling needed
        };

        let num_channels = self.channels as usize;

        // Keep processing until we have output or source is exhausted
        loop {
            // 1. Try to fill input buffer from source
            if !self.input_buffer.source_finished {
                let mut frame = vec![0.0f32; num_channels];

                // Get input_frames_next while holding the lock briefly
                let input_frames_needed = {
                    let r = resampler_mutex.lock().unwrap();
                    r.input_frames_next()
                };

                loop {
                    // Read one frame at a time from source
                    let mut got_frame = true;
                    for sample in frame.iter_mut().take(num_channels) {
                        match self.source.next_sample()? {
                            Some(s) => *sample = s,
                            None => {
                                self.input_buffer.source_finished = true;
                                got_frame = false;
                                break;
                            }
                        }
                    }

                    if got_frame {
                        self.input_buffer.push_frame(&frame);
                    }

                    // Stop filling when we have enough for processing or source finished
                    if self.input_buffer.source_finished
                        || self.input_buffer.len() >= input_frames_needed
                    {
                        break;
                    }
                }
            }

            // 2. Process if we have enough input
            let mut resampler = resampler_mutex.lock().unwrap();
            let input_frames_needed = resampler.input_frames_next();

            if self.input_buffer.len() >= input_frames_needed {
                // Process a full chunk
                let (nbr_in, nbr_out) = resampler
                    .process_into_buffer(
                        &self.input_buffer.channels,
                        &mut self.output_scratch,
                        None,
                    )
                    .map_err(|_e| {
                        SampleSourceError::ResamplingFailed(self.source_rate, self.target_rate)
                    })?;

                drop(resampler); // Release lock before drain

                // Drain consumed input (this is the key difference from old code!)
                self.input_buffer.drain_frames(nbr_in);

                // Append output to FIFO
                if nbr_out > 0 {
                    self.output_fifo.push_frames(&self.output_scratch, nbr_out);
                    return Ok(()); // We have output, caller can consume it
                }

                // Safety: if resampler consumed nothing, we can't make progress
                if nbr_in == 0 {
                    return Ok(());
                }
                // No output yet, continue processing
            } else if self.input_buffer.source_finished {
                // 3. Source finished - process any remaining input

                // If no remaining input, we're done
                if self.input_buffer.len() == 0 {
                    return Ok(());
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

                drop(resampler); // Release lock before drain

                // Clear remaining input
                self.input_buffer.drain_frames(self.input_buffer.len());

                if nbr_out > 0 {
                    self.output_fifo.push_frames(&self.output_scratch, nbr_out);
                }

                // Done processing - return regardless of whether we got output
                return Ok(());
            } else {
                // Need more input but source isn't finished yet - shouldn't happen in normal flow
                return Ok(());
            }
        }
    }
}
