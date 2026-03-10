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
use crate::config::ResamplerType;
use parking_lot::Mutex;
use rubato::audioadapter::{Adapter, AdapterMut};
use rubato::{
    Async, Fft, FixedAsync, FixedSync, Resampler, SincInterpolationParameters,
    SincInterpolationType, WindowFunction,
};
use tracing::info;

use super::error::SampleSourceError;
use super::traits::SampleSource;

/// Adapter for `&[Vec<f32>]` (channel-major layout) used by rubato's `Resampler` trait.
struct ChannelBufRef<'a> {
    buf: &'a [Vec<f32>],
    frames: usize,
}

impl<'a> ChannelBufRef<'a> {
    fn new(buf: &'a [Vec<f32>], frames: usize) -> Self {
        Self { buf, frames }
    }
}

impl<'a> Adapter<'a, f32> for ChannelBufRef<'a> {
    unsafe fn read_sample_unchecked(&self, channel: usize, frame: usize) -> f32 {
        *self.buf.get_unchecked(channel).get_unchecked(frame)
    }
    fn channels(&self) -> usize {
        self.buf.len()
    }
    fn frames(&self) -> usize {
        self.frames
    }
}

/// Mutable adapter for `&mut [Vec<f32>]` (channel-major layout).
struct ChannelBufMut<'a> {
    buf: &'a mut [Vec<f32>],
    frames: usize,
}

impl<'a> ChannelBufMut<'a> {
    fn new(buf: &'a mut [Vec<f32>], frames: usize) -> Self {
        Self { buf, frames }
    }
}

impl<'a> Adapter<'a, f32> for ChannelBufMut<'a> {
    unsafe fn read_sample_unchecked(&self, channel: usize, frame: usize) -> f32 {
        *self.buf.get_unchecked(channel).get_unchecked(frame)
    }
    fn channels(&self) -> usize {
        self.buf.len()
    }
    fn frames(&self) -> usize {
        self.frames
    }
}

impl<'a> AdapterMut<'a, f32> for ChannelBufMut<'a> {
    unsafe fn write_sample_unchecked(&mut self, channel: usize, frame: usize, value: &f32) -> bool {
        *self.buf.get_unchecked_mut(channel).get_unchecked_mut(frame) = *value;
        false
    }
}

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
    /// Resampler wrapped in Mutex for Sync (contains non-Sync internals)
    pub resampler: Option<Mutex<Box<dyn Resampler<f32>>>>,
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
        resampler_type: ResamplerType,
    ) -> Result<Self, SampleSourceError> {
        let needs_resampling = source_format.sample_rate != target_format.sample_rate;

        let (resampler, output_scratch) = if needs_resampling {
            let resample_ratio =
                target_format.sample_rate as f64 / source_format.sample_rate as f64;
            let num_channels = channels as usize;

            let r: Box<dyn Resampler<f32>> = match resampler_type {
                ResamplerType::Sinc => {
                    let sinc_params = SincInterpolationParameters {
                        sinc_len: 256,
                        f_cutoff: 0.95,
                        oversampling_factor: 128,
                        interpolation: SincInterpolationType::Linear,
                        window: WindowFunction::BlackmanHarris2,
                    };
                    Box::new(
                        Async::<f32>::new_sinc(
                            resample_ratio,
                            1.0,
                            &sinc_params,
                            INPUT_BLOCK_SIZE,
                            num_channels,
                            FixedAsync::Input,
                        )
                        .map_err(|_e| {
                            SampleSourceError::ResamplingFailed(
                                source_format.sample_rate,
                                target_format.sample_rate,
                            )
                        })?,
                    )
                }
                ResamplerType::Fft => Box::new(
                    Fft::<f32>::new(
                        source_format.sample_rate as usize,
                        target_format.sample_rate as usize,
                        INPUT_BLOCK_SIZE,
                        2,
                        num_channels,
                        FixedSync::Input,
                    )
                    .map_err(|_e| {
                        SampleSourceError::ResamplingFailed(
                            source_format.sample_rate,
                            target_format.sample_rate,
                        )
                    })?,
                ),
            };

            info!(
                resampler = ?resampler_type,
                from = source_format.sample_rate,
                to = target_format.sample_rate,
                "Resampling audio",
            );

            let max_out = r.output_frames_max();
            let scratch: Vec<Vec<f32>> = vec![vec![0.0; max_out]; num_channels];
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
        // Safety: fill_output_fifo is only called from next_sample when
        // self.resampler.is_some(), so unwrap is safe here.
        let resampler_mutex = self
            .resampler
            .as_ref()
            .expect("fill_output_fifo called without resampler");

        let num_channels = self.channels as usize;

        // Keep processing until we have output or source is exhausted
        loop {
            // 1. Try to fill input buffer from source
            if !self.input_buffer.source_finished {
                let mut frame = vec![0.0f32; num_channels];

                // Get input_frames_next while holding the lock briefly
                let input_frames_needed = {
                    let r = resampler_mutex.lock();
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
            let mut resampler = resampler_mutex.lock();
            let input_frames_needed = resampler.input_frames_next();

            if self.input_buffer.len() >= input_frames_needed {
                // Process a full chunk
                let input_frames = self.input_buffer.len();
                let input_adapter = ChannelBufRef::new(&self.input_buffer.channels, input_frames);

                let output_frames = self.output_scratch[0].len();
                let mut output_adapter =
                    ChannelBufMut::new(&mut self.output_scratch, output_frames);

                let (nbr_in, nbr_out) = resampler
                    .process_into_buffer(&input_adapter, &mut output_adapter, None)
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

                let remaining = self.input_buffer.len();
                let input_adapter = ChannelBufRef::new(&self.input_buffer.channels, remaining);

                let output_frames = self.output_scratch[0].len();
                let mut output_adapter =
                    ChannelBufMut::new(&mut self.output_scratch, output_frames);

                let indexing = rubato::Indexing {
                    input_offset: 0,
                    output_offset: 0,
                    partial_len: Some(remaining),
                    active_channels_mask: None,
                };

                let (_nbr_in, nbr_out) = resampler
                    .process_into_buffer(&input_adapter, &mut output_adapter, Some(&indexing))
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── AudioTranscoder ──────────────────────────────────────────────

    use crate::audio::sample_source::memory::MemorySampleSource;
    use crate::audio::TargetFormat;
    use crate::config::ResamplerType;

    #[test]
    fn passthrough_returns_samples_unchanged() {
        let samples = vec![0.1, 0.2, 0.3, 0.4, 0.5];
        let source = MemorySampleSource::new(samples.clone(), 1, 44100);
        let fmt = TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        let mut tc = AudioTranscoder::new(source, &fmt, &fmt, 1, ResamplerType::Sinc).unwrap();

        assert!(tc.resampler.is_none(), "same rate should skip resampler");

        let mut out = Vec::new();
        while let Ok(Some(s)) = tc.next_sample() {
            out.push(s);
        }
        assert_eq!(out, samples);
    }

    #[test]
    fn passthrough_trait_methods() {
        let source = MemorySampleSource::new(vec![0.0; 10], 2, 48000);
        let fmt = TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let tc = AudioTranscoder::new(source, &fmt, &fmt, 2, ResamplerType::Sinc).unwrap();

        assert_eq!(tc.channel_count(), 2);
        assert_eq!(tc.sample_rate(), 48000);
        assert_eq!(tc.bits_per_sample(), 32);
        assert_eq!(tc.sample_format(), crate::audio::SampleFormat::Float);
        // duration delegates to underlying source
        assert!(tc.duration().is_some());
    }

    #[test]
    fn sinc_resampler_produces_output() {
        let num_samples = 4800;
        let mut input = Vec::with_capacity(num_samples);
        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            input.push((2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.5);
        }
        let source = MemorySampleSource::new(input, 1, 48000);
        let src_fmt = TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let tgt_fmt = TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        let mut tc =
            AudioTranscoder::new(source, &src_fmt, &tgt_fmt, 1, ResamplerType::Sinc).unwrap();

        assert!(tc.resampler.is_some());
        assert_eq!(tc.source_rate, 48000);
        assert_eq!(tc.target_rate, 44100);

        let mut out = Vec::new();
        while let Ok(Some(s)) = tc.next_sample() {
            out.push(s);
        }
        assert!(!out.is_empty(), "sinc resampler should produce output");
        // Output length should be roughly input_len * (44100/48000)
        let expected = (num_samples as f64 * 44100.0 / 48000.0) as usize;
        let tolerance = (expected as f64 * 0.15) as usize;
        assert!(
            out.len() >= expected.saturating_sub(tolerance) && out.len() <= expected + tolerance,
            "sinc output length {} not near expected {}",
            out.len(),
            expected
        );
    }

    #[test]
    fn fft_resampler_produces_output() {
        let num_samples = 4800;
        let mut input = Vec::with_capacity(num_samples);
        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            input.push((2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.5);
        }
        let source = MemorySampleSource::new(input, 1, 48000);
        let src_fmt = TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let tgt_fmt = TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        let mut tc =
            AudioTranscoder::new(source, &src_fmt, &tgt_fmt, 1, ResamplerType::Fft).unwrap();

        assert!(tc.resampler.is_some());

        let mut out = Vec::new();
        while let Ok(Some(s)) = tc.next_sample() {
            out.push(s);
        }
        assert!(!out.is_empty(), "fft resampler should produce output");
    }

    #[test]
    fn resampler_stereo_channels() {
        // Generate stereo 48kHz signal, resample to 44.1kHz
        let num_frames = 4800;
        let mut input = Vec::with_capacity(num_frames * 2);
        for i in 0..num_frames {
            let t = i as f32 / 48000.0;
            input.push((2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.3); // left
            input.push((2.0 * std::f32::consts::PI * 880.0 * t).sin() * 0.3); // right
        }
        let source = MemorySampleSource::new(input, 2, 48000);
        let src_fmt = TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let tgt_fmt = TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        let mut tc =
            AudioTranscoder::new(source, &src_fmt, &tgt_fmt, 2, ResamplerType::Sinc).unwrap();

        assert_eq!(tc.channel_count(), 2);
        assert_eq!(tc.sample_rate(), 44100);

        let mut out = Vec::new();
        while let Ok(Some(s)) = tc.next_sample() {
            out.push(s);
        }
        assert!(!out.is_empty());
    }

    #[test]
    fn resampler_target_bits_per_sample() {
        let source = MemorySampleSource::new(vec![0.5; 100], 1, 44100);
        let src_fmt = TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        let tgt_fmt = TargetFormat::new(48000, crate::audio::SampleFormat::Float, 24).unwrap();
        let tc = AudioTranscoder::new(source, &src_fmt, &tgt_fmt, 1, ResamplerType::Sinc).unwrap();
        assert_eq!(tc.bits_per_sample(), 24);
    }

    #[test]
    fn resampler_duration_delegates() {
        let source = MemorySampleSource::new(vec![0.0; 44100], 1, 44100);
        let src_fmt = TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        let tgt_fmt = TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let tc = AudioTranscoder::new(source, &src_fmt, &tgt_fmt, 1, ResamplerType::Sinc).unwrap();
        let dur = tc.duration().expect("duration should be Some");
        // 44100 samples / 1 channel / 44100 Hz = 1 second
        assert!((dur.as_secs_f64() - 1.0).abs() < 0.01);
    }
}
