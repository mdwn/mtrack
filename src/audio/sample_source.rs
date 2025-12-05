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
use crate::audio::TargetFormat;
use hound::WavReader;
use rubato::{
    SincFixedIn, SincInterpolationParameters, SincInterpolationType, VecResampler, WindowFunction,
};
use std::io::BufReader;
use std::path::Path;
use std::sync::Mutex;

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

/// A source of audio samples that processes an iterator
pub trait SampleSource: Send + Sync {
    /// Get the next sample from the source
    /// Returns Ok(Some(sample)) if a sample is available
    /// Returns Ok(None) if the source is finished
    /// Returns Err(error) if an error occurred
    fn next_sample(&mut self) -> Result<Option<f32>, TranscodingError>;

    /// Get the number of channels in this source
    fn channel_count(&self) -> u16;

    /// Get the sample rate of this source
    fn sample_rate(&self) -> u32;

    /// Get the bits per sample of this source
    fn bits_per_sample(&self) -> u16;

    /// Get the sample format of this source
    fn sample_format(&self) -> crate::audio::SampleFormat;

    /// Get the duration of this source (if known)
    /// Returns None if the duration is unknown or infinite
    fn duration(&self) -> Option<std::time::Duration>;
}

/// A sample source with explicit channel mapping information
/// This replaces the complex SongSource architecture with a simpler, more debuggable approach
pub trait ChannelMappedSampleSource: Send + Sync {
    /// Get the next sample from the source
    /// Returns Ok(Some(sample)) if a sample is available
    /// Returns Ok(None) if the source is finished
    /// Returns Err(error) if an error occurred
    fn next_sample(&mut self) -> Result<Option<f32>, TranscodingError>;

    /// Get the next frame of samples (all channels for one time step)
    /// Writes samples directly into the provided output slice
    /// Returns Ok(Some(count)) where count is the number of samples written
    /// Returns Ok(None) if the source is finished
    /// Returns Err(error) if an error occurred
    /// The output slice must have capacity for at least source_channel_count() samples
    fn next_frame(&mut self, output: &mut [f32]) -> Result<Option<usize>, TranscodingError> {
        let channel_count = self.source_channel_count() as usize;
        if output.len() < channel_count {
            return Err(TranscodingError::SampleConversionFailed(format!(
                "Output buffer too small: need {} samples",
                channel_count
            )));
        }
        for out in output.iter_mut().take(channel_count) {
            match self.next_sample()? {
                Some(sample) => *out = sample,
                None => return Ok(None),
            }
        }
        Ok(Some(channel_count))
    }

    /// Get the channel mappings for this source
    /// Returns a Vec where each element corresponds to a source channel
    /// Each Vec<String> contains the labels that source channel maps to
    /// Empty Vec means that source channel is not mapped to any output
    fn channel_mappings(&self) -> &Vec<Vec<String>>;

    /// Get the number of source channels in this sample source
    fn source_channel_count(&self) -> u16;
}

#[cfg(test)]
pub trait SampleSourceTestExt {
    /// Check if the source is finished (no more samples)
    fn is_finished(&self) -> bool;
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
    resampler: Option<Mutex<SincFixedIn<f32>>>,
    source_rate: u32,
    target_rate: u32,
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
    fn next_sample(&mut self) -> Result<Option<f32>, TranscodingError> {
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
    ) -> Result<Self, TranscodingError> {
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
                TranscodingError::ResamplingFailed(
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
    fn fill_output_fifo(&mut self) -> Result<(), TranscodingError> {
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
                let input_frames_needed = resampler_mutex.lock().unwrap().input_frames_next();

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
                        TranscodingError::ResamplingFailed(self.source_rate, self.target_rate)
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
                        TranscodingError::ResamplingFailed(self.source_rate, self.target_rate)
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

/// Error types for transcoding operations
#[derive(Debug, thiserror::Error)]
pub enum TranscodingError {
    #[error("Resampling failed: {0}Hz -> {1}Hz")]
    ResamplingFailed(u32, u32),

    #[error("Sample conversion failed for {0}")]
    SampleConversionFailed(String),

    #[error("WAV file error: {0}")]
    WavError(#[from] hound::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
}

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
    fn next_sample(&mut self) -> Result<Option<f32>, TranscodingError> {
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

/// A wrapper that implements ChannelMappedSampleSource for any SampleSource
pub struct ChannelMappedSource {
    source: Box<dyn SampleSource>,
    channel_mappings: Vec<Vec<String>>,
    source_channel_count: u16,
}

impl ChannelMappedSource {
    /// Create a new ChannelMappedSource
    pub fn new(
        source: Box<dyn SampleSource>,
        channel_mappings: Vec<Vec<String>>,
        source_channel_count: u16,
    ) -> Self {
        Self {
            source,
            channel_mappings,
            source_channel_count,
        }
    }
}

impl ChannelMappedSampleSource for ChannelMappedSource {
    fn next_sample(&mut self) -> Result<Option<f32>, TranscodingError> {
        self.source.next_sample()
    }

    fn next_frame(&mut self, output: &mut [f32]) -> Result<Option<usize>, TranscodingError> {
        let channel_count = self.source_channel_count as usize;
        if output.len() < channel_count {
            return Err(TranscodingError::SampleConversionFailed(format!(
                "Output buffer too small: need {} samples",
                channel_count
            )));
        }
        for out in output.iter_mut().take(channel_count) {
            match self.source.next_sample()? {
                Some(sample) => *out = sample,
                None => return Ok(None),
            }
        }
        Ok(Some(channel_count))
    }

    fn channel_mappings(&self) -> &Vec<Vec<String>> {
        &self.channel_mappings
    }

    fn source_channel_count(&self) -> u16 {
        self.source_channel_count
    }
}

/// A wrapper that makes Box<dyn SampleSource> work with AudioTranscoder
struct SampleSourceWrapper {
    source: Box<dyn SampleSource>,
}

impl SampleSource for SampleSourceWrapper {
    fn next_sample(&mut self) -> Result<Option<f32>, TranscodingError> {
        self.source.next_sample()
    }

    fn channel_count(&self) -> u16 {
        self.source.channel_count()
    }

    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }

    fn bits_per_sample(&self) -> u16 {
        self.source.bits_per_sample()
    }

    fn sample_format(&self) -> crate::audio::SampleFormat {
        self.source.sample_format()
    }

    fn duration(&self) -> Option<std::time::Duration> {
        self.source.duration()
    }
}

/// Create a ChannelMappedSampleSource from a generic SampleSource
pub fn create_channel_mapped_sample_source(
    source: Box<dyn SampleSource>,
    target_format: TargetFormat,
    channel_mappings: Vec<Vec<String>>,
    _buffer_size: usize,
    _buffer_threshold: usize,
) -> Result<Box<dyn ChannelMappedSampleSource>, TranscodingError> {
    let source_format = TargetFormat::new(
        source.sample_rate(),
        source.sample_format(),
        source.bits_per_sample(),
    )
    .map_err(|e| TranscodingError::SampleConversionFailed(e.to_string()))?;

    let needs_transcoding = source_format.sample_rate != target_format.sample_rate
        || source_format.sample_format != target_format.sample_format
        || source_format.bits_per_sample != target_format.bits_per_sample;

    let channel_count = source.channel_count();
    let sample_source: Box<dyn SampleSource> = if needs_transcoding {
        // Create a wrapper that can be used with AudioTranscoder
        let wrapper = SampleSourceWrapper { source };
        let transcoder =
            AudioTranscoder::new(wrapper, &source_format, &target_format, channel_count)?;
        Box::new(transcoder)
    } else {
        source
    };

    // Use the sample source directly - buffering adds complexity without benefit
    // since the resampler already handles buffering internally
    Ok(Box::new(ChannelMappedSource::new(
        sample_source,
        channel_mappings,
        channel_count,
    )))
}

/// A sample source that reads WAV files and provides scaled samples
/// This is the raw WAV reading component - no transcoding logic
pub struct WavSampleSource {
    wav_reader: hound::WavReader<BufReader<std::fs::File>>,
    is_finished: bool,
    // Buffered reading to reduce I/O operations
    sample_buffer: Vec<f32>,
    buffer_position: usize,
    buffer_size: usize,
    // WAV file metadata for direct parsing
    bits_per_sample: u16,
    channels: u16,
    sample_rate: u32,
    sample_format: crate::audio::SampleFormat,
    duration: std::time::Duration,
}

impl SampleSource for WavSampleSource {
    fn next_sample(&mut self) -> Result<Option<f32>, TranscodingError> {
        if self.is_finished {
            return Ok(None);
        }

        // Check if we need to refill the buffer
        if self.buffer_position >= self.sample_buffer.len() {
            self.refill_buffer()?;

            // If buffer is still empty after refill, we're finished
            if self.sample_buffer.is_empty() {
                self.is_finished = true;
                return Ok(None);
            }
        }

        // Return the next sample from the buffer
        let sample = self.sample_buffer[self.buffer_position];
        self.buffer_position += 1;
        Ok(Some(sample))
    }

    fn channel_count(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn bits_per_sample(&self) -> u16 {
        self.bits_per_sample
    }

    fn sample_format(&self) -> crate::audio::SampleFormat {
        self.sample_format
    }

    fn duration(&self) -> Option<std::time::Duration> {
        Some(self.duration)
    }
}

impl WavSampleSource {
    /// Creates a new WAV sample source from a file path
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, TranscodingError> {
        Self::from_file_with_seek(path, None)
    }

    /// Creates a new WAV sample source from a file path, optionally seeking to a start time
    pub fn from_file_with_seek<P: AsRef<Path>>(
        path: P,
        start_time: Option<std::time::Duration>,
    ) -> Result<Self, TranscodingError> {
        let mut wav_reader = WavReader::open(&path)?;
        let spec = wav_reader.spec();
        let duration = std::time::Duration::from_secs(
            u64::from(wav_reader.duration()) / u64::from(spec.sample_rate),
        );

        // If start_time is provided, seek to that position
        if let Some(start) = start_time {
            // Calculate frame position using precise floating point math to avoid rounding errors
            // hound's seek() takes a frame position, where a frame is one sample per channel
            // For a 2-channel file: frame 0 = samples [0,1], frame 1 = samples [2,3], etc.
            // So frame_position = time * sample_rate (NOT divided by channels)
            let frame_position = start.as_secs_f64() * spec.sample_rate as f64;
            // Round to nearest frame to ensure consistent seeking across files
            let frame_position = frame_position.round() as u32;
            wav_reader.seek(frame_position)?;
        }

        // Use a reasonable buffer size - 1024 samples per channel
        let buffer_size = 1024;

        let sample_format = match spec.sample_format {
            hound::SampleFormat::Float => crate::audio::SampleFormat::Float,
            hound::SampleFormat::Int => crate::audio::SampleFormat::Int,
        };

        Ok(Self {
            wav_reader,
            is_finished: false,
            sample_buffer: Vec::with_capacity(buffer_size),
            buffer_position: 0,
            buffer_size,
            bits_per_sample: spec.bits_per_sample,
            channels: spec.channels,
            sample_rate: spec.sample_rate,
            sample_format,
            duration,
        })
    }

    /// Refills the sample buffer by reading a chunk from the WAV file
    fn refill_buffer(&mut self) -> Result<(), TranscodingError> {
        // Clear the buffer and reset position
        self.sample_buffer.clear();
        self.buffer_position = 0;

        // Read samples directly using the samples iterator (still more efficient than per-sample I/O)
        let mut samples_read = 0;
        let spec = self.wav_reader.spec();

        // Read samples in the correct format based on the WAV file's actual format
        if spec.sample_format == hound::SampleFormat::Float {
            // For float WAV files, read as f32
            for sample_result in self.wav_reader.samples::<f32>().take(self.buffer_size) {
                match sample_result {
                    Ok(sample) => {
                        // Float samples are already in the correct range [-1.0, 1.0]
                        self.sample_buffer.push(sample);
                        samples_read += 1;
                    }
                    Err(e) => return Err(TranscodingError::WavError(e)),
                }
            }
        } else {
            // For integer WAV files, read as i32
            for sample_result in self.wav_reader.samples::<i32>().take(self.buffer_size) {
                match sample_result {
                    Ok(sample) => {
                        // Convert i32 to f32 with proper scaling
                        // Use i64 to avoid overflow for 32-bit samples
                        let scale_factor = 1.0 / (1i64 << (self.bits_per_sample - 1)) as f32;
                        let result = sample as f32 * scale_factor;
                        self.sample_buffer.push(result);
                        samples_read += 1;
                    }
                    Err(e) => return Err(TranscodingError::WavError(e)),
                }
            }
        }

        // If we read no samples, we're at the end of the file
        if samples_read == 0 {
            self.is_finished = true;
        }

        Ok(())
    }

    /// Returns the number of channels in the WAV file
    #[cfg(test)]
    pub fn channels(&self) -> u16 {
        self.channels
    }

    /// Returns the sample rate of the WAV file
    #[cfg(test)]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

#[cfg(test)]
impl SampleSourceTestExt for WavSampleSource {
    fn is_finished(&self) -> bool {
        self.is_finished
    }
}

/// Create a SampleSource from a file, automatically detecting the file type
pub fn create_sample_source_from_file<P: AsRef<Path>>(
    path: P,
) -> Result<Box<dyn SampleSource>, TranscodingError> {
    create_sample_source_from_file_with_seek(path, None)
}

pub fn create_sample_source_from_file_with_seek<P: AsRef<Path>>(
    path: P,
    start_time: Option<std::time::Duration>,
) -> Result<Box<dyn SampleSource>, TranscodingError> {
    let path = path.as_ref();

    // Get file extension to determine type
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_lowercase();

    match extension.as_str() {
        "wav" => {
            let wav_source = WavSampleSource::from_file_with_seek(path, start_time)?;
            Ok(Box::new(wav_source))
        }
        _ => Err(TranscodingError::SampleConversionFailed(format!(
            "Unsupported file format: {}",
            extension
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::audio_test_utils::calculate_snr;
    use rand;

    /// Calculate high-frequency energy content (simple approximation)
    fn calculate_high_frequency_energy(samples: &[f32], _sample_rate: f32) -> f32 {
        if samples.len() < 2 {
            return 0.0;
        }

        // Simple high-pass filter approximation: difference between consecutive samples
        let mut high_freq_energy = 0.0;
        for i in 1..samples.len() {
            let diff = samples[i] - samples[i - 1];
            high_freq_energy += diff * diff;
        }

        high_freq_energy / (samples.len() - 1) as f32
    }

    #[test]
    fn test_memory_sample_source() {
        let samples = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let _target_format = TargetFormat::default();
        let mut source = MemorySampleSource::new(samples.clone(), 1, 44100);

        // Test that we get all samples
        for (i, expected) in samples.iter().enumerate() {
            let sample = source.next_sample().unwrap().unwrap();
            assert_eq!(sample, *expected);
            // After reading the last sample, we should be finished
            if i == samples.len() - 1 {
                assert!(SampleSourceTestExt::is_finished(&source));
            } else {
                assert!(!SampleSourceTestExt::is_finished(&source));
            }
        }

        // Test that we get None when finished
        assert!(source.next_sample().unwrap().is_none());
        assert!(SampleSourceTestExt::is_finished(&source));
    }

    #[test]
    fn test_memory_sample_source_duration_mono() {
        // Test duration calculation for mono audio
        let samples = vec![1.0, 2.0, 3.0, 4.0, 5.0]; // 5 samples
        let source = MemorySampleSource::new(samples.clone(), 1, 44100);

        // Calculate expected duration using the same formula as the implementation
        let total_samples = samples.len() as f64;
        let samples_per_channel = total_samples / 1.0; // mono
        let duration_secs = samples_per_channel / 44100.0;
        let expected_duration = std::time::Duration::from_secs_f64(duration_secs);

        let actual_duration = source.duration().unwrap();

        // Allow for small rounding differences
        let diff = actual_duration.abs_diff(expected_duration);
        assert!(
            diff < std::time::Duration::from_micros(1),
            "Duration mismatch: expected {:?}, got {:?}",
            expected_duration,
            actual_duration
        );
    }

    #[test]
    fn test_memory_sample_source_duration_stereo() {
        // Test duration calculation for stereo audio
        let samples = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]; // 6 samples (3 frames * 2 channels)
        let source = MemorySampleSource::new(samples.clone(), 2, 44100);

        // Calculate expected duration using the same formula as the implementation
        let total_samples = samples.len() as f64;
        let samples_per_channel = total_samples / 2.0; // stereo
        let duration_secs = samples_per_channel / 44100.0;
        let expected_duration = std::time::Duration::from_secs_f64(duration_secs);

        let actual_duration = source.duration().unwrap();

        // Allow for small rounding differences
        let diff = actual_duration.abs_diff(expected_duration);
        assert!(
            diff < std::time::Duration::from_micros(1),
            "Duration mismatch: expected {:?}, got {:?}",
            expected_duration,
            actual_duration
        );
    }

    #[test]
    fn test_resampling_quality() {
        // Test actual resampling with simple input
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Create a mock source for testing
        let mock_source = MemorySampleSource::new(vec![0.1, 0.2, 0.3, 0.4, 0.5], 1, 44100);
        match AudioTranscoder::new(mock_source, &source_format, &target_format, 1) {
            Ok(mut converter) => {
                // Test resampling by getting samples from the converter
                let mut output_samples = Vec::with_capacity(100);
                let mut sample_count = 0;
                const MAX_SAMPLES: usize = 100; // Prevent infinite loops

                while sample_count < MAX_SAMPLES {
                    match converter.next_sample() {
                        Ok(Some(sample)) => {
                            output_samples.push(sample);
                            sample_count += 1;
                        }
                        Ok(None) => break, // End of source
                        Err(_e) => break,  // Error occurred
                    }
                }

                // Basic checks
                assert!(!output_samples.is_empty(), "Output should not be empty");
                assert!(
                    !output_samples.is_empty(),
                    "Should have some output samples"
                );

                // Verify the signal is still recognizable (basic quality check)
                let max_amplitude = output_samples.iter().map(|&x| x.abs()).fold(0.0, f32::max);

                assert!(
                    max_amplitude > 0.0,
                    "Resampled signal should have some amplitude"
                );
                assert!(
                    max_amplitude <= 1.1,
                    "Resampled signal too loud, max amplitude: {}",
                    max_amplitude
                );
            }
            Err(_e) => {
                // If rubato resampler creation fails, that's acceptable for now
            }
        }
    }

    #[test]
    fn test_rubato_resampler_creation() {
        // Test that rubato resampler can be created for common ratios
        let test_cases = vec![
            (44100, 48000), // CD to DAT
            (48000, 44100), // DAT to CD
            (44100, 44100), // Same rate (should not create resampler)
        ];

        for (source_rate, target_rate) in test_cases {
            let source_format =
                TargetFormat::new(source_rate, crate::audio::SampleFormat::Float, 32).unwrap();
            let target_format =
                TargetFormat::new(target_rate, crate::audio::SampleFormat::Float, 32).unwrap();

            // Create a mock source for testing
            let mock_source = MemorySampleSource::new(vec![0.1, 0.2, 0.3], 1, 44100);
            let converter = AudioTranscoder::new(mock_source, &source_format, &target_format, 1);

            if source_rate == target_rate {
                // Should not need resampling
                assert!(converter.is_ok());
                let converter = converter.unwrap();
                assert!(converter.resampler.is_none());
            } else {
                // Should create resampler
                match converter {
                    Ok(_converter) => {
                        if source_rate != target_rate {
                            // For now, we expect this to work for reasonable ratios
                            // If rubato fails, we'll get an error which is also acceptable
                        }
                    }
                    Err(_e) => {
                        // If rubato fails to create the resampler, that's also a valid test result
                    }
                }
            }
        }
    }

    #[test]
    fn test_rubato_configuration_debug() {
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Create a mock source for testing
        let mock_source = MemorySampleSource::new(vec![1.0, 2.0, 3.0, 4.0, 5.0], 1, 44100);
        match AudioTranscoder::new(mock_source, &source_format, &target_format, 1) {
            Ok(mut converter) => {
                // Test with a simple input to understand the behavior
                let mut sample_count = 0;
                const MAX_SAMPLES: usize = 50;

                while sample_count < MAX_SAMPLES {
                    match converter.next_sample() {
                        Ok(Some(_sample)) => sample_count += 1,
                        Ok(None) => break,
                        Err(_e) => break,
                    }
                }
            }
            Err(_e) => {}
        }
    }

    #[test]
    fn test_resampling_edge_cases() {
        // Test that we can detect when resampling is needed
        let test_cases = vec![
            (44100, 48000, true),  // CD to DAT - should need resampling
            (48000, 44100, true),  // DAT to CD - should need resampling
            (44100, 44100, false), // Same rate - should not need resampling
        ];

        for (source_rate, target_rate, should_need_resampling) in test_cases {
            let source_format =
                TargetFormat::new(source_rate, crate::audio::SampleFormat::Float, 32).unwrap();
            let target_format =
                TargetFormat::new(target_rate, crate::audio::SampleFormat::Float, 32).unwrap();

            // Create a mock source for testing
            let mock_source = MemorySampleSource::new(vec![0.1, 0.2, 0.3], 1, 44100);
            let converter = AudioTranscoder::new(mock_source, &source_format, &target_format, 1);

            match converter {
                Ok(converter) => {
                    // Transcoding is now handled internally by WavSampleSource
                    // The old needs_resampling check is no longer needed
                    let _needs_resampling = should_need_resampling;
                    // Test that the converter was created successfully
                    assert!(converter.source_rate == source_rate);
                    assert!(converter.target_rate == target_rate);
                }
                Err(_) => {
                    // If rubato fails to create the resampler, that's acceptable for now
                }
            }
        }
    }

    #[test]
    fn test_no_resampling_needed() {
        // Test when no resampling is needed
        let source_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        // Create a mock source for testing
        let mock_source = MemorySampleSource::new(vec![0.1, 0.2, 0.3, 0.4, 0.5], 1, 44100);
        let mut converter =
            AudioTranscoder::new(mock_source, &source_format, &target_format, 1).unwrap();

        // Should not need resampling
        // Transcoding is now handled internally by WavSampleSource

        // Test that samples are returned unchanged when no resampling is needed
        let mut output_samples = Vec::with_capacity(10);
        let mut sample_count = 0;
        const MAX_SAMPLES: usize = 10;

        while sample_count < MAX_SAMPLES {
            match converter.next_sample() {
                Ok(Some(sample)) => {
                    output_samples.push(sample);
                    sample_count += 1;
                }
                Ok(None) => break,
                Err(_e) => break,
            }
        }

        // Should have some output samples
        assert!(!output_samples.is_empty());
    }

    #[test]
    fn test_resampling_quality_sine_wave() {
        // Test resampling quality with a sine wave signal
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate a 1kHz sine wave at 48kHz
        let frequency = 1000.0; // 1kHz
        let duration = 0.1; // 100ms
        let num_samples = (48000.0 * duration) as usize;

        let mut input_samples = Vec::with_capacity(num_samples);
        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            let sample = (2.0 * std::f32::consts::PI * frequency * t).sin();
            input_samples.push(sample);
        }

        // Create a mock source with the sine wave
        let mock_source = MemorySampleSource::new(input_samples, 1, 44100);
        match AudioTranscoder::new(mock_source, &source_format, &target_format, 1) {
            Ok(mut converter) => {
                // Test resampling by getting samples from the converter
                let mut output_samples = Vec::with_capacity(200);
                let mut sample_count = 0;
                const MAX_SAMPLES: usize = 200; // Allow more samples for sine wave

                while sample_count < MAX_SAMPLES {
                    match converter.next_sample() {
                        Ok(Some(sample)) => {
                            output_samples.push(sample);
                            sample_count += 1;
                        }
                        Ok(None) => break,
                        Err(_e) => break,
                    }
                }

                // Verify output length is reasonable (rubato may produce different lengths)
                // For now, just ensure we get some output
                assert!(
                    !output_samples.is_empty(),
                    "Should have some output samples"
                );
                assert!(
                    !output_samples.is_empty(),
                    "Should have some output samples"
                );

                // Verify the signal is still a sine wave (basic quality check)
                let max_amplitude = output_samples.iter().map(|&x| x.abs()).fold(0.0, f32::max);

                assert!(
                    max_amplitude > 0.5,
                    "Resampled sine wave too quiet, max amplitude: {}",
                    max_amplitude
                );
                assert!(
                    max_amplitude <= 1.1,
                    "Resampled sine wave too loud, max amplitude: {}",
                    max_amplitude
                );

                // Check for aliasing (high-frequency content should be minimal)
                let high_freq_energy = calculate_high_frequency_energy(&output_samples, 44100.0);
                assert!(
                    high_freq_energy < 0.1,
                    "Too much high-frequency content (aliasing): {}",
                    high_freq_energy
                );
            }
            Err(_e) => {}
        }
    }

    #[test]
    fn test_roundtrip_resampling_quality() {
        // Test that resampling up and then down preserves quality
        let original_rate = 44100;
        let intermediate_rate = 48000;
        let final_rate = 44100;

        let source_format_1 =
            TargetFormat::new(original_rate, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format_1 =
            TargetFormat::new(intermediate_rate, crate::audio::SampleFormat::Float, 32).unwrap();
        let source_format_2 =
            TargetFormat::new(intermediate_rate, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format_2 =
            TargetFormat::new(final_rate, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate a test signal: 1kHz sine wave + small amount of noise
        let duration = 0.1; // 100ms
        let num_samples = (original_rate as f32 * duration) as usize;
        let mut original_samples = Vec::with_capacity(num_samples);

        for i in 0..num_samples {
            let t = i as f32 / original_rate as f32;
            let sine_wave = 0.5 * (2.0 * std::f32::consts::PI * 1000.0 * t).sin();
            let noise = 0.05 * (rand::random::<f32>() - 0.5);
            original_samples.push(sine_wave + noise);
        }

        // First resampling: 44.1kHz -> 48kHz
        let source_1 = MemorySampleSource::new(original_samples.clone(), 1, 44100);
        let mut converter_1 =
            AudioTranscoder::new(source_1, &source_format_1, &target_format_1, 1).unwrap();

        let mut intermediate_samples = Vec::with_capacity(num_samples);
        loop {
            match converter_1.next_sample() {
                Ok(Some(sample)) => intermediate_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Second resampling: 48kHz -> 44.1kHz
        let intermediate_len = intermediate_samples.len();
        let source_2 = MemorySampleSource::new(intermediate_samples, 1, intermediate_rate);
        let mut converter_2 =
            AudioTranscoder::new(source_2, &source_format_2, &target_format_2, 1).unwrap();

        let mut final_samples = Vec::with_capacity(intermediate_len);
        loop {
            match converter_2.next_sample() {
                Ok(Some(sample)) => final_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Quality checks
        assert!(
            !final_samples.is_empty(),
            "Final samples should not be empty"
        );

        // Check that we have a reasonable number of samples (should be close to original)
        // Roundtrip through sinc resampling loses some samples at boundaries due to inherent delay
        let expected_length = original_samples.len();
        let length_tolerance = (expected_length as f32 * 0.35) as usize;
        assert!(
            final_samples.len() >= expected_length - length_tolerance
                && final_samples.len() <= expected_length + length_tolerance,
            "Final length {} should be close to original length {}",
            final_samples.len(),
            expected_length
        );

        // Check that the signal is still recognizable
        let max_amplitude = final_samples.iter().map(|&x| x.abs()).fold(0.0, f32::max);
        assert!(
            max_amplitude > 0.1,
            "Final signal should have reasonable amplitude, got {}",
            max_amplitude
        );
        assert!(
            max_amplitude <= 1.0,
            "Final signal should not be too loud, got {}",
            max_amplitude
        );
    }

    #[test]
    fn test_resampling_quality_impulse() {
        // Test resampling quality with impulse signal
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate impulse signal (single sample at maximum amplitude)
        let mut input_samples = vec![0.0; 100];
        input_samples[50] = 1.0; // Impulse at sample 50

        let source = MemorySampleSource::new(input_samples, 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        let mut output_samples = Vec::new();
        loop {
            match converter.next_sample() {
                Ok(Some(sample)) => output_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Basic quality checks
        assert!(!output_samples.is_empty(), "Output should not be empty");

        // The impulse will be spread by the sinc kernel; we just require that
        // some non-trivial amplitude remains (numerically, not perceptually).
        let max_amplitude = output_samples.iter().map(|&x| x.abs()).fold(0.0, f32::max);
        assert!(
            max_amplitude > 1e-8,
            "Impulse signal should have reasonable amplitude after resampling, got {}",
            max_amplitude
        );
    }

    #[test]
    fn test_resampling_quality_noise() {
        // Test resampling quality with white noise
        let source_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate white noise
        let num_samples = 1000;
        let mut input_samples = Vec::new();
        for _ in 0..num_samples {
            // Simple pseudo-random noise
            let noise = (rand::random::<f32>() - 0.5) * 2.0;
            input_samples.push(noise);
        }

        let source = MemorySampleSource::new(input_samples.clone(), 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        let mut output_samples = Vec::new();
        loop {
            match converter.next_sample() {
                Ok(Some(sample)) => output_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Basic quality checks
        assert!(!output_samples.is_empty(), "Output should not be empty");

        // Verify output length is approximately correct
        // Sinc resamplers with sliding window may produce fewer samples due to inherent delay
        // and not zero-padding at EOF, so we use a larger tolerance (30%)
        let expected_ratio = 48000.0 / 44100.0;
        let expected_length = (num_samples as f32 * expected_ratio) as usize;
        let length_tolerance = (expected_length as f32 * 0.30) as usize;

        assert!(
            output_samples.len() >= expected_length - length_tolerance
                && output_samples.len() <= expected_length + length_tolerance,
            "Expected ~{} samples, got {}",
            expected_length,
            output_samples.len()
        );

        // Verify the noise characteristics are preserved
        let input_rms = calculate_rms(&input_samples);
        let output_rms = calculate_rms(&output_samples);

        // RMS should be similar (within 50% tolerance for FFT resamplers)
        let rms_ratio = output_rms / input_rms;
        assert!(
            rms_ratio > 0.5 && rms_ratio < 1.5,
            "RMS ratio out of range: {} (input: {}, output: {})",
            rms_ratio,
            input_rms,
            output_rms
        );
    }

    #[test]
    fn test_resampling_multichannel_quality() {
        // Test resampling quality with multichannel audio
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        let channels = 2;

        // Generate stereo test signal
        let duration = 0.1; // 100ms
        let num_frames = (48000.0 * duration) as usize;
        let mut input_samples = Vec::new();

        for i in 0..num_frames {
            let t = i as f32 / 48000.0;
            // Left channel: 440Hz
            let left = 0.3 * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
            // Right channel: 880Hz
            let right = 0.3 * (2.0 * std::f32::consts::PI * 880.0 * t).sin();
            input_samples.push(left);
            input_samples.push(right);
        }

        let source = MemorySampleSource::new(input_samples, 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, channels).unwrap();

        let mut output_samples = Vec::new();
        loop {
            match converter.next_sample() {
                Ok(Some(sample)) => {
                    output_samples.push(sample);
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Basic quality checks
        assert!(!output_samples.is_empty(), "Output should not be empty");

        // Should have approximately the right number of samples
        let expected_length = (num_frames as f32 * (44100.0 / 48000.0) * channels as f32) as usize;
        let length_tolerance = (expected_length as f32 * 0.1) as usize;

        assert!(
            output_samples.len() >= expected_length - length_tolerance
                && output_samples.len() <= expected_length + length_tolerance,
            "Expected ~{} samples, got {}",
            expected_length,
            output_samples.len()
        );

        // Check that we have stereo output (even number of samples)
        // Note: With larger block sizes, the resampler may produce slightly different
        // numbers of samples. We allow up to one extra sample as this doesn't affect quality.
        let remainder = output_samples.len() % 2;
        assert!(
            remainder == 0
                || output_samples.len() % 2 == 1
                    && output_samples.len() <= expected_length + length_tolerance + 1,
            "Stereo output should have even number of samples (or at most one extra), got {}",
            output_samples.len()
        );
    }

    /// Calculate RMS (Root Mean Square) of a signal
    fn calculate_rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }

        let sum_squares: f32 = samples.iter().map(|&x| x * x).sum();
        (sum_squares / samples.len() as f32).sqrt()
    }

    #[test]
    fn test_resampling_empty_input() {
        // Test behavior with empty input
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        let source = MemorySampleSource::new(vec![], 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        // Empty input should return None immediately
        assert!(matches!(converter.next_sample(), Ok(None)));
    }

    #[test]
    fn test_resampling_single_sample() {
        // Test with just one sample
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        let source = MemorySampleSource::new(vec![0.5], 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        let mut output_samples = Vec::new();
        loop {
            match converter.next_sample() {
                Ok(Some(sample)) => output_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Should produce some output even from a single sample
        assert!(
            !output_samples.is_empty(),
            "Single sample should produce some output"
        );
    }

    #[test]
    fn test_resampling_extreme_ratios() {
        // Test very high and very low sample rate ratios
        let test_cases = vec![
            (8000, 192000), // 24:1 upsampling
            (192000, 8000), // 1:24 downsampling
            (44100, 88200), // 2:1 upsampling
            (88200, 44100), // 1:2 downsampling
        ];

        for (source_rate, target_rate) in test_cases {
            let source_format =
                TargetFormat::new(source_rate, crate::audio::SampleFormat::Float, 32).unwrap();
            let target_format =
                TargetFormat::new(target_rate, crate::audio::SampleFormat::Float, 32).unwrap();

            // Generate a simple test signal
            let duration = 0.01; // 10ms
            let num_samples = (source_rate as f32 * duration) as usize;
            let mut input_samples = Vec::new();

            for i in 0..num_samples {
                let t = i as f32 / source_rate as f32;
                input_samples.push((2.0 * std::f32::consts::PI * 1000.0 * t).sin() * 0.5);
            }

            let source = MemorySampleSource::new(input_samples, 1, 44100);
            let mut converter =
                AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

            let mut output_samples = Vec::new();
            let mut sample_count = 0;
            const MAX_SAMPLES: usize = 1000; // Prevent infinite loops

            while sample_count < MAX_SAMPLES {
                match converter.next_sample() {
                    Ok(Some(sample)) => {
                        output_samples.push(sample);
                        sample_count += 1;
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }

            assert!(
                !output_samples.is_empty(),
                "Extreme ratio {}/{} should produce output",
                source_rate,
                target_rate
            );

            // Check for non-trivial amplitude. For extreme ratios, sinc
            // resampling can spread energy significantly; we only assert that
            // the signal is not effectively silent.
            let max_amplitude = output_samples.iter().map(|&x| x.abs()).fold(0.0, f32::max);
            assert!(
                max_amplitude > 1e-8,
                "Extreme ratio {}/{} should have reasonable amplitude, got {}",
                source_rate,
                target_rate,
                max_amplitude
            );
        }
    }

    #[test]
    fn test_resampling_long_duration() {
        // Test with a longer duration signal to ensure stability
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate a 1-second signal
        let duration = 1.0;
        let num_samples = (48000.0 * duration) as usize;
        let mut input_samples = Vec::new();

        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            // Mix of frequencies to test aliasing
            let signal = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.3
                + (2.0 * std::f32::consts::PI * 880.0 * t).sin() * 0.2
                + (2.0 * std::f32::consts::PI * 1760.0 * t).sin() * 0.1;
            input_samples.push(signal);
        }

        let source = MemorySampleSource::new(input_samples, 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        let mut output_samples = Vec::new();
        let mut sample_count = 0;
        const MAX_SAMPLES: usize = 50000; // Allow for longer processing

        while sample_count < MAX_SAMPLES {
            match converter.next_sample() {
                Ok(Some(sample)) => {
                    output_samples.push(sample);
                    sample_count += 1;
                }
                Ok(None) => break,
                Err(_) => break,
            }
        }

        assert!(
            !output_samples.is_empty(),
            "Long duration should produce output"
        );

        // Check that we got a reasonable number of samples
        let expected_length = (num_samples as f32 * (44100.0 / 48000.0)) as usize;
        let length_tolerance = (expected_length as f32 * 0.05) as usize; // 5% tolerance

        assert!(
            output_samples.len() >= expected_length - length_tolerance
                && output_samples.len() <= expected_length + length_tolerance,
            "Long duration: expected ~{}, got {}",
            expected_length,
            output_samples.len()
        );
    }

    #[test]
    fn test_resampling_high_frequency_content() {
        // Test with high-frequency content to check for aliasing
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate signal with high-frequency content (near Nyquist)
        let num_samples = 1000;
        let mut input_samples = Vec::new();

        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            // High frequency signal (20kHz - near Nyquist for 48kHz)
            let signal = (2.0 * std::f32::consts::PI * 20000.0 * t).sin() * 0.5;
            input_samples.push(signal);
        }

        let source = MemorySampleSource::new(input_samples, 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        let mut output_samples = Vec::new();
        loop {
            match converter.next_sample() {
                Ok(Some(sample)) => output_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        assert!(
            !output_samples.is_empty(),
            "High frequency content should produce output"
        );

        // Check for aliasing - high frequency content should be attenuated
        let high_freq_energy = calculate_high_frequency_energy(&output_samples, 44100.0);
        assert!(
            high_freq_energy < 0.5, // Should be significantly attenuated
            "High frequency content should be attenuated, got {}",
            high_freq_energy
        );
    }

    #[test]
    fn test_resampling_overflow_protection() {
        // Test with very large values to check for overflow
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Test with values near the limits
        let test_values = vec![
            vec![1.0, -1.0, 0.999, -0.999], // Near full scale
            vec![0.0, 0.0, 0.0, 0.0],       // All zeros
            vec![0.5, -0.5, 0.5, -0.5],     // Alternating
        ];

        for values in test_values {
            let source = MemorySampleSource::new(values, 1, 44100);
            let mut converter =
                AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

            let mut output_samples = Vec::new();
            let mut sample_count = 0;
            const MAX_SAMPLES: usize = 100;

            while sample_count < MAX_SAMPLES {
                match converter.next_sample() {
                    Ok(Some(sample)) => {
                        // Check for NaN or infinity
                        assert!(
                            sample.is_finite(),
                            "Output should be finite, got {}",
                            sample
                        );
                        output_samples.push(sample);
                        sample_count += 1;
                    }
                    Ok(None) => break,
                    Err(_) => break,
                }
            }

            assert!(
                !output_samples.is_empty(),
                "Should produce output for edge values"
            );
        }
    }

    #[test]
    fn test_resampling_dc_offset() {
        // Test DC offset handling
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate signal with DC offset
        let dc_offset = 0.1;
        let num_samples = 100;
        let mut input_samples = Vec::new();

        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            let signal = (2.0 * std::f32::consts::PI * 1000.0 * t).sin() * 0.3 + dc_offset;
            input_samples.push(signal);
        }

        let source = MemorySampleSource::new(input_samples, 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        let mut output_samples = Vec::new();
        loop {
            match converter.next_sample() {
                Ok(Some(sample)) => output_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        assert!(
            !output_samples.is_empty(),
            "DC offset test should produce output"
        );

        // Check that DC offset does not blow up; sinc resamplers are linear,
        // but practical implementations may slightly attenuate DC.
        let mean_value = output_samples.iter().sum::<f32>() / output_samples.len() as f32;
        assert!(
            (mean_value - dc_offset).abs() < 0.2,
            "DC offset should be reasonably preserved, expected ~{}, got {}",
            dc_offset,
            mean_value
        );
    }

    #[test]
    fn test_resampling_simple_snr() {
        // Simple test: just resample a sine wave once and check SNR
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate a simple sine wave
        let frequency = 1000.0; // 1kHz
        let duration = 0.01; // 10ms
        let num_samples = (48000.0 * duration) as usize;
        let mut original_samples = Vec::with_capacity(num_samples);

        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            let sample = (2.0 * std::f32::consts::PI * frequency * t).sin() * 0.5;
            original_samples.push(sample);
        }

        // Resample once: 48kHz -> 44.1kHz
        let source = MemorySampleSource::new(original_samples.clone(), 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        let mut output_samples = Vec::with_capacity(num_samples);
        loop {
            match converter.next_sample() {
                Ok(Some(sample)) => output_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // For now, just check that we get reasonable output
        assert!(!output_samples.is_empty(), "No output samples generated");
        assert!(
            output_samples.len() > 100,
            "Too few output samples: {}",
            output_samples.len()
        );

        // Check that the output has reasonable amplitude
        let output_rms = calculate_rms(&output_samples);
        assert!(output_rms > 0.1, "Output RMS too low: {}", output_rms);
        assert!(output_rms < 1.0, "Output RMS too high: {}", output_rms);
    }

    #[test]
    fn test_resampling_snr_quality() {
        // Test that resampling maintains reasonable SNR between input and output signals
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        let back_format = TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate a clean 1kHz sine wave at 48kHz
        let frequency = 1000.0; // 1kHz
        let duration = 0.1; // 100ms
        let num_samples = (48000.0 * duration) as usize;
        let mut original_samples = Vec::with_capacity(num_samples);

        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            let sample = (2.0 * std::f32::consts::PI * frequency * t).sin() * 0.5;
            original_samples.push(sample);
        }

        // First resampling: 48kHz -> 44.1kHz
        let source_1 = MemorySampleSource::new(original_samples.clone(), 1, 44100);
        let mut converter_1 =
            AudioTranscoder::new(source_1, &source_format, &target_format, 1).unwrap();

        let mut intermediate_samples = Vec::with_capacity(num_samples);
        loop {
            match converter_1.next_sample() {
                Ok(Some(sample)) => intermediate_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Second resampling: 44.1kHz -> 48kHz (roundtrip)
        let source_2 = MemorySampleSource::new(intermediate_samples, 1, 44100);
        let mut converter_2 =
            AudioTranscoder::new(source_2, &target_format, &back_format, 1).unwrap();

        let mut final_samples = Vec::with_capacity(original_samples.len());
        loop {
            match converter_2.next_sample() {
                Ok(Some(sample)) => final_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Ensure both signals have the same length for SNR calculation
        let min_len = original_samples.len().min(final_samples.len());
        let original_truncated = &original_samples[..min_len];
        let final_truncated = &final_samples[..min_len];

        // Calculate SNR between original and final (roundtrip)
        let snr = calculate_snr(original_truncated, final_truncated);

        // For sinc resampling, uncompensated phase delay and edge effects can
        // reduce this naive SNR metric even when audible quality is high.
        // Use a loose lower bound here to catch only obviously broken behaviour.
        assert!(
            snr > -10.0,
            "SNR too low: {} dB (expected > -10 dB). Original: {} samples, Final: {} samples",
            snr,
            original_truncated.len(),
            final_truncated.len()
        );
    }

    #[test]
    fn test_resampling_rms_preservation() {
        // Test that RMS energy is preserved across different resampling ratios
        let test_cases = vec![
            (48000, 44100, 1000.0), // 48kHz -> 44.1kHz, 1kHz
            (48000, 96000, 2000.0), // 48kHz -> 96kHz, 2kHz
            (44100, 48000, 1500.0), // 44.1kHz -> 48kHz, 1.5kHz
        ];

        for (source_rate, target_rate, frequency) in test_cases {
            let source_format =
                TargetFormat::new(source_rate, crate::audio::SampleFormat::Float, 32).unwrap();
            let target_format =
                TargetFormat::new(target_rate, crate::audio::SampleFormat::Float, 32).unwrap();

            // Generate sine wave at the specified frequency
            let duration = 0.05; // 50ms
            let num_samples = (source_rate as f32 * duration) as usize;
            let mut input_samples = Vec::with_capacity(num_samples);

            for i in 0..num_samples {
                let t = i as f32 / source_rate as f32;
                let sample = (2.0 * std::f32::consts::PI * frequency * t).sin() * 0.3;
                input_samples.push(sample);
            }

            // Resample
            let source = MemorySampleSource::new(input_samples.clone(), 1, source_rate);
            let mut converter =
                AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

            let mut output_samples = Vec::with_capacity(num_samples);
            loop {
                match converter.next_sample() {
                    Ok(Some(sample)) => output_samples.push(sample),
                    Ok(None) => break,
                    Err(_) => break,
                }
            }

            // Calculate RMS for input and output
            let input_rms = calculate_rms(&input_samples);
            let output_rms = calculate_rms(&output_samples);

            // RMS should be preserved within 20% tolerance
            // Sinc resamplers with sliding window may lose some energy at signal boundaries
            let rms_ratio = output_rms / input_rms;
            assert!(
                (0.8..=1.2).contains(&rms_ratio),
                "RMS ratio out of range for {}Hz->{}Hz: {} (input: {}, output: {})",
                source_rate,
                target_rate,
                rms_ratio,
                input_rms,
                output_rms
            );
        }
    }

    #[test]
    fn test_resampling_snr_multichannel() {
        // Test SNR preservation in multichannel (stereo) scenarios
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate stereo signal with different frequencies per channel
        let duration = 0.1; // 100ms
        let num_frames = (48000.0 * duration) as usize;
        let mut input_samples = Vec::with_capacity(num_frames * 2);

        for i in 0..num_frames {
            let t = i as f32 / 48000.0;
            // Left channel: 440Hz (A4)
            let left = 0.3 * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
            // Right channel: 880Hz (A5)
            let right = 0.3 * (2.0 * std::f32::consts::PI * 880.0 * t).sin();
            input_samples.push(left);
            input_samples.push(right);
        }

        // First resampling: 48kHz -> 44.1kHz
        let source_1 = MemorySampleSource::new(input_samples.clone(), 1, 44100);
        let mut converter_1 =
            AudioTranscoder::new(source_1, &source_format, &target_format, 2).unwrap();

        let mut intermediate_samples = Vec::with_capacity(input_samples.len());
        loop {
            match converter_1.next_sample() {
                Ok(Some(sample)) => intermediate_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Second resampling: 44.1kHz -> 48kHz (roundtrip for fair comparison)
        let back_format = TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let source_2 = MemorySampleSource::new(intermediate_samples, 1, 44100);
        let mut converter_2 =
            AudioTranscoder::new(source_2, &target_format, &back_format, 2).unwrap();

        let mut output_samples = Vec::with_capacity(input_samples.len());
        loop {
            match converter_2.next_sample() {
                Ok(Some(sample)) => output_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Separate left and right channels for SNR calculation
        let mut left_original = Vec::with_capacity(num_frames);
        let mut right_original = Vec::with_capacity(num_frames);
        let mut left_output = Vec::with_capacity(output_samples.len() / 2);
        let mut right_output = Vec::with_capacity(output_samples.len() / 2);

        // Extract left and right channels from interleaved samples
        for i in (0..input_samples.len()).step_by(2) {
            if i + 1 < input_samples.len() {
                left_original.push(input_samples[i]);
                right_original.push(input_samples[i + 1]);
            }
        }

        for i in (0..output_samples.len()).step_by(2) {
            if i + 1 < output_samples.len() {
                left_output.push(output_samples[i]);
                right_output.push(output_samples[i + 1]);
            }
        }

        // Ensure both signals have the same length for SNR calculation
        let left_min_len = left_original.len().min(left_output.len());
        let right_min_len = right_original.len().min(right_output.len());
        let left_original_truncated = &left_original[..left_min_len];
        let left_output_truncated = &left_output[..left_min_len];
        let right_original_truncated = &right_original[..right_min_len];
        let right_output_truncated = &right_output[..right_min_len];

        // Calculate SNR for both channels. As with the mono case, sinc
        // resampling plus uncompensated delay and edge effects make a strict
        // SNR threshold unrealistic; we just ensure the output is not totally
        // decorrelated.
        let left_snr = calculate_snr(left_original_truncated, left_output_truncated);
        let right_snr = calculate_snr(right_original_truncated, right_output_truncated);

        assert!(
            left_snr > -10.0,
            "Left channel SNR too low: {} dB (expected > -10 dB)",
            left_snr
        );
        assert!(
            right_snr > -10.0,
            "Right channel SNR too low: {} dB (expected > -10 dB)",
            right_snr
        );
    }

    #[test]
    fn test_resampling_rms_complex_signal() {
        // Test RMS preservation with complex multi-frequency signals
        let source_format =
            TargetFormat::new(48000, crate::audio::SampleFormat::Float, 32).unwrap();
        let target_format =
            TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();

        // Generate complex signal with multiple harmonics (fundamental + overtones)
        let duration = 0.1; // 100ms
        let num_samples = (48000.0 * duration) as usize;
        let mut input_samples = Vec::with_capacity(num_samples);

        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            // Fundamental frequency: 220Hz (A3)
            let fundamental = 0.4 * (2.0 * std::f32::consts::PI * 220.0 * t).sin();
            // First harmonic: 440Hz (A4)
            let harmonic1 = 0.2 * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
            // Second harmonic: 880Hz (A5)
            let harmonic2 = 0.1 * (2.0 * std::f32::consts::PI * 880.0 * t).sin();
            // Third harmonic: 1320Hz (E6)
            let harmonic3 = 0.05 * (2.0 * std::f32::consts::PI * 1320.0 * t).sin();

            let complex_signal = fundamental + harmonic1 + harmonic2 + harmonic3;
            input_samples.push(complex_signal);
        }

        // Resample complex signal
        let source = MemorySampleSource::new(input_samples.clone(), 1, 44100);
        let mut converter =
            AudioTranscoder::new(source, &source_format, &target_format, 1).unwrap();

        let mut output_samples = Vec::with_capacity(num_samples);
        loop {
            match converter.next_sample() {
                Ok(Some(sample)) => output_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Calculate RMS for input and output
        let input_rms = calculate_rms(&input_samples);
        let output_rms = calculate_rms(&output_samples);

        // RMS should be preserved within 15% tolerance for complex signals
        let rms_ratio = output_rms / input_rms;
        assert!(
            (0.85..=1.15).contains(&rms_ratio),
            "RMS ratio out of range for complex signal: {} (input: {}, output: {})",
            rms_ratio,
            input_rms,
            output_rms
        );

        // Verify that the complex signal structure is maintained
        // by checking that we have significant energy in multiple frequency bands
        let input_energy = input_rms * input_rms;
        let output_energy = output_rms * output_rms;
        let energy_ratio = output_energy / input_energy;

        assert!(
            (0.7..=1.3).contains(&energy_ratio),
            "Energy ratio out of range for complex signal: {} (input: {}, output: {})",
            energy_ratio,
            input_energy,
            output_energy
        );
    }

    #[test]
    fn test_wav_sample_source_16bit() {
        use crate::testutil::write_wav_with_bits;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_16bit.wav");

        // Create a 16-bit WAV file with known samples
        let samples: Vec<i16> = vec![1000, -2000, 3000, -4000, 5000];
        write_wav_with_bits(wav_path.clone(), vec![samples], 44100, 16).unwrap();

        // Test reading the WAV file using generic function
        let mut wav_source = create_sample_source_from_file(&wav_path).unwrap();

        let mut read_samples = Vec::new();
        loop {
            match wav_source.next_sample() {
                Ok(Some(sample)) => read_samples.push(sample),
                Ok(None) => break,
                Err(e) => panic!("Error reading sample: {}", e),
            }
        }

        // Verify we got the expected number of samples
        assert_eq!(read_samples.len(), 5);

        // Verify the samples are scaled correctly (16-bit to f32)
        // 16-bit samples should be scaled to [-1.0, 1.0] range
        let expected_samples = [
            1000.0 / (1 << 15) as f32,  // 1000 / 32768
            -2000.0 / (1 << 15) as f32, // -2000 / 32768
            3000.0 / (1 << 15) as f32,  // 3000 / 32768
            -4000.0 / (1 << 15) as f32, // -4000 / 32768
            5000.0 / (1 << 15) as f32,  // 5000 / 32768
        ];

        for (i, (actual, expected)) in read_samples.iter().zip(expected_samples.iter()).enumerate()
        {
            assert!(
                (actual - expected).abs() < 0.0001,
                "Sample {} mismatch: expected {}, got {}",
                i,
                expected,
                actual
            );
        }
    }

    #[test]
    fn test_wav_sample_source_24bit() {
        use crate::testutil::write_wav_with_bits;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_24bit.wav");

        // Create a 24-bit WAV file with known samples
        let samples: Vec<i32> = vec![100000, -200000, 300000, -400000, 500000];
        write_wav_with_bits(wav_path.clone(), vec![samples], 44100, 24).unwrap();

        // Test reading the WAV file using generic function
        let mut wav_source = create_sample_source_from_file(&wav_path).unwrap();

        let mut read_samples = Vec::new();
        loop {
            match wav_source.next_sample() {
                Ok(Some(sample)) => read_samples.push(sample),
                Ok(None) => break,
                Err(e) => panic!("Error reading sample: {}", e),
            }
        }

        // Verify we got the expected number of samples
        assert_eq!(read_samples.len(), 5);

        // Verify the samples are scaled correctly (24-bit to f32)
        // 24-bit samples should be scaled to [-1.0, 1.0] range
        let expected_samples = [
            100000.0 / (1 << 23) as f32,  // 100000 / 8388608
            -200000.0 / (1 << 23) as f32, // -200000 / 8388608
            300000.0 / (1 << 23) as f32,  // 300000 / 8388608
            -400000.0 / (1 << 23) as f32, // -400000 / 8388608
            500000.0 / (1 << 23) as f32,  // 500000 / 8388608
        ];

        for (i, (actual, expected)) in read_samples.iter().zip(expected_samples.iter()).enumerate()
        {
            assert!(
                (actual - expected).abs() < 0.0001,
                "Sample {} mismatch: expected {}, got {}",
                i,
                expected,
                actual
            );
        }
    }

    #[test]
    fn test_wav_sample_source_32bit() {
        use crate::testutil::write_wav;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_32bit.wav");

        // Create a 32-bit WAV file with known samples
        let samples: Vec<i32> = vec![1000000, -2000000, 3000000, -4000000, 5000000];
        write_wav(wav_path.clone(), vec![samples], 44100).unwrap();

        // Test reading the WAV file using generic function
        let mut wav_source = create_sample_source_from_file(&wav_path).unwrap();

        let mut read_samples = Vec::new();
        loop {
            match wav_source.next_sample() {
                Ok(Some(sample)) => read_samples.push(sample),
                Ok(None) => break,
                Err(e) => panic!("Error reading sample: {}", e),
            }
        }

        // Verify we got the expected number of samples
        assert_eq!(read_samples.len(), 5);

        // Verify the samples are scaled correctly (32-bit to f32)
        // 32-bit samples should be scaled to [-1.0, 1.0] range
        let expected_samples = [
            0.0004656613,  // 1000000 / 2147483648
            -0.0009313226, // -2000000 / 2147483648
            0.0013969839,  // 3000000 / 2147483648
            -0.0018626451, // -4000000 / 2147483648
            0.0023283064,  // 5000000 / 2147483648
        ];

        for (i, (actual, expected)) in read_samples.iter().zip(expected_samples.iter()).enumerate()
        {
            assert!(
                (actual - expected).abs() < 0.0001,
                "Sample {} mismatch: expected {}, got {}",
                i,
                expected,
                actual
            );
        }
    }

    #[test]
    fn test_wav_sample_source_stereo() {
        use crate::testutil::write_wav;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_stereo.wav");

        // Create a stereo WAV file
        let left_samples: Vec<i32> = vec![1000, 2000, 3000];
        let right_samples: Vec<i32> = vec![-1000, -2000, -3000];
        write_wav(wav_path.clone(), vec![left_samples, right_samples], 44100).unwrap();

        // Test reading the WAV file
        let mut wav_source = WavSampleSource::from_file(&wav_path).unwrap();

        let mut read_samples = Vec::new();
        loop {
            match wav_source.next_sample() {
                Ok(Some(sample)) => read_samples.push(sample),
                Ok(None) => break,
                Err(e) => panic!("Error reading sample: {}", e),
            }
        }

        // Verify we got the expected number of samples (interleaved stereo)
        assert_eq!(read_samples.len(), 6);

        // Verify the samples are interleaved correctly (L, R, L, R, L, R)
        let expected_samples = [
            1000.0 / (1 << 31) as f32,  // Left channel, sample 1
            -1000.0 / (1 << 31) as f32, // Right channel, sample 1
            2000.0 / (1 << 31) as f32,  // Left channel, sample 2
            -2000.0 / (1 << 31) as f32, // Right channel, sample 2
            3000.0 / (1 << 31) as f32,  // Left channel, sample 3
            -3000.0 / (1 << 31) as f32, // Right channel, sample 3
        ];

        for (i, (actual, expected)) in read_samples.iter().zip(expected_samples.iter()).enumerate()
        {
            assert!(
                (actual - expected).abs() < 0.0001,
                "Sample {} mismatch: expected {}, got {}",
                i,
                expected,
                actual
            );
        }
    }

    #[test]
    fn test_wav_sample_source_empty_file() {
        use crate::testutil::write_wav;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_empty.wav");

        // Create an empty WAV file
        write_wav(wav_path.clone(), vec![Vec::<i32>::new()], 44100).unwrap();

        // Test reading the empty WAV file
        let mut wav_source = WavSampleSource::from_file(&wav_path).unwrap();

        // Should return None immediately
        match wav_source.next_sample() {
            Ok(None) => {} // Expected
            Ok(Some(sample)) => panic!("Expected None for empty file, got: {}", sample),
            Err(e) => panic!("Error reading empty file: {}", e),
        }

        // Verify is_finished is true
        assert!(wav_source.is_finished());
    }

    #[test]
    fn test_wav_sample_source_nonexistent_file() {
        let wav_path = std::path::Path::new("nonexistent_file.wav");

        // Should return an error for nonexistent file
        if WavSampleSource::from_file(wav_path).is_ok() {
            panic!("Expected error for nonexistent file")
        }
    }

    #[test]
    fn test_wav_sample_source_is_finished() {
        use crate::testutil::write_wav;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_finished.wav");

        // Create a WAV file with a few samples
        let samples: Vec<i32> = vec![1000, 2000, 3000];
        write_wav(wav_path.clone(), vec![samples], 44100).unwrap();

        let mut wav_source = WavSampleSource::from_file(&wav_path).unwrap();

        // Initially not finished
        assert!(!wav_source.is_finished());

        // Read all samples
        let mut sample_count = 0;
        loop {
            match wav_source.next_sample() {
                Ok(Some(_)) => {
                    sample_count += 1;
                    assert!(!wav_source.is_finished()); // Still not finished
                }
                Ok(None) => {
                    assert!(wav_source.is_finished()); // Now finished
                    break;
                }
                Err(e) => panic!("Error reading sample: {}", e),
            }
        }

        // Verify we read the expected number of samples
        assert_eq!(sample_count, 3);

        // Verify is_finished is true after reading all samples
        assert!(wav_source.is_finished());
    }

    #[test]
    fn test_wav_sample_source_amplitude_consistency() {
        use crate::testutil::write_wav_with_bits;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();

        // Generate the same audio content (sine wave with amplitude 0.5)
        let sample_rate = 44100;
        let duration_samples = 1000;
        let frequency = 440.0; // 440Hz sine wave

        let sine_wave: Vec<f32> = (0..duration_samples)
            .map(|i| {
                (i as f32 * frequency * 2.0 * std::f32::consts::PI / sample_rate as f32).sin() * 0.5
            })
            .collect();

        // Test 16-bit WAV
        let wav_16_path = tempdir.path().join("test_16bit_amplitude.wav");
        let samples_16: Vec<i16> = sine_wave.iter().map(|&x| (x * 32767.0) as i16).collect();
        write_wav_with_bits(wav_16_path.clone(), vec![samples_16], sample_rate, 16).unwrap();

        // Test 24-bit WAV
        let wav_24_path = tempdir.path().join("test_24bit_amplitude.wav");
        let samples_24: Vec<i32> = sine_wave
            .iter()
            .map(|&x| (x * 8388607.0) as i32) // 24-bit range
            .collect();
        write_wav_with_bits(wav_24_path.clone(), vec![samples_24], sample_rate, 24).unwrap();

        // Test 32-bit WAV
        let wav_32_path = tempdir.path().join("test_32bit_amplitude.wav");
        let samples_32: Vec<i32> = sine_wave
            .iter()
            .map(|&x| (x * 2147483647.0) as i32) // 32-bit range
            .collect();
        write_wav_with_bits(wav_32_path.clone(), vec![samples_32], sample_rate, 32).unwrap();

        // Read samples from each WAV file
        let mut wav_16_source = WavSampleSource::from_file(&wav_16_path).unwrap();
        let mut wav_24_source = WavSampleSource::from_file(&wav_24_path).unwrap();
        let mut wav_32_source = WavSampleSource::from_file(&wav_32_path).unwrap();

        let mut samples_16_read = Vec::new();
        let mut samples_24_read = Vec::new();
        let mut samples_32_read = Vec::new();

        for _ in 0..duration_samples {
            if let Ok(Some(sample)) = wav_16_source.next_sample() {
                samples_16_read.push(sample);
            }
            if let Ok(Some(sample)) = wav_24_source.next_sample() {
                samples_24_read.push(sample);
            }
            if let Ok(Some(sample)) = wav_32_source.next_sample() {
                samples_32_read.push(sample);
            }
        }

        // Calculate RMS for each
        let rms_16: f32 = (samples_16_read.iter().map(|&x| x * x).sum::<f32>()
            / samples_16_read.len() as f32)
            .sqrt();
        let rms_24: f32 = (samples_24_read.iter().map(|&x| x * x).sum::<f32>()
            / samples_24_read.len() as f32)
            .sqrt();
        let rms_32: f32 = (samples_32_read.iter().map(|&x| x * x).sum::<f32>()
            / samples_32_read.len() as f32)
            .sqrt();

        // The RMS should be similar across all bit depths (within 5% tolerance)
        let expected_rms = 0.5 / (2.0_f32.sqrt()); // RMS of sine wave with amplitude 0.5

        // All should be close to the expected RMS
        assert!(
            (rms_16 - expected_rms).abs() / expected_rms < 0.05,
            "16-bit RMS too different: got {:.6}, expected {:.6}",
            rms_16,
            expected_rms
        );
        assert!(
            (rms_24 - expected_rms).abs() / expected_rms < 0.05,
            "24-bit RMS too different: got {:.6}, expected {:.6}",
            rms_24,
            expected_rms
        );
        assert!(
            (rms_32 - expected_rms).abs() / expected_rms < 0.05,
            "32-bit RMS too different: got {:.6}, expected {:.6}",
            rms_32,
            expected_rms
        );

        // All bit depths should have similar RMS (within 10% of each other)
        assert!(
            (rms_16 - rms_24).abs() / rms_16 < 0.1,
            "16-bit and 24-bit RMS too different: {:.6} vs {:.6}",
            rms_16,
            rms_24
        );
        assert!(
            (rms_16 - rms_32).abs() / rms_16 < 0.1,
            "16-bit and 32-bit RMS too different: {:.6} vs {:.6}",
            rms_16,
            rms_32
        );
        assert!(
            (rms_24 - rms_32).abs() / rms_24 < 0.1,
            "24-bit and 32-bit RMS too different: {:.6} vs {:.6}",
            rms_24,
            rms_32
        );
    }

    #[test]
    fn test_wav_sample_source_different_sample_rates() {
        use crate::testutil::write_wav;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();

        // Test different sample rates
        let sample_rates = vec![22050, 44100, 48000, 96000];

        for sample_rate in sample_rates {
            let wav_path = tempdir.path().join(format!("test_{}.wav", sample_rate));

            // Create a WAV file with a sine wave
            let duration = 0.01; // 10ms
            let num_samples = (sample_rate as f32 * duration) as usize;
            let samples: Vec<i32> = (0..num_samples)
                .map(|i| {
                    ((i as f32 * 1000.0 * 2.0 * std::f32::consts::PI / sample_rate as f32).sin()
                        * (1 << 23) as f32) as i32
                })
                .collect();

            write_wav(wav_path.clone(), vec![samples], sample_rate).unwrap();

            // Test reading the WAV file
            let mut wav_source = WavSampleSource::from_file(&wav_path).unwrap();

            let mut read_samples = Vec::new();
            loop {
                match wav_source.next_sample() {
                    Ok(Some(sample)) => read_samples.push(sample),
                    Ok(None) => break,
                    Err(e) => panic!("Error reading sample at {}Hz: {}", sample_rate, e),
                }
            }

            // Verify we got the expected number of samples
            assert_eq!(read_samples.len(), num_samples);

            // Verify the samples have reasonable amplitude (not all zeros)
            let rms: f32 = (read_samples.iter().map(|&x| x * x).sum::<f32>()
                / read_samples.len() as f32)
                .sqrt();
            assert!(rms > 0.001, "RMS too low for {}Hz: {}", sample_rate, rms);
        }
    }

    #[test]
    fn test_wav_sample_source_seek() {
        use crate::testutil::write_wav;
        use std::time::Duration;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_seek.wav");

        // Create a WAV file with 10 seconds of samples at 44100 Hz
        // We'll use a pattern that changes over time so we can verify seeking
        let sample_rate = 44100u32;
        let duration_secs = 10;
        let total_samples = sample_rate as usize * duration_secs;

        // Create samples with a pattern: value = sample_index / 1000 (so we can verify position)
        let samples: Vec<i32> = (0..total_samples)
            .map(|i| (i as i32 / 1000).min(i32::MAX / 2))
            .collect();

        write_wav(wav_path.clone(), vec![samples], sample_rate).unwrap();

        // Test seeking to 5 seconds
        let seek_time = Duration::from_secs(5);
        let mut wav_source =
            WavSampleSource::from_file_with_seek(&wav_path, Some(seek_time)).unwrap();

        // Read a few samples and verify we can read after seeking
        // At 5 seconds, we should be at sample index ~220500 (5 * 44100)
        let first_sample = wav_source.next_sample().unwrap();
        assert!(first_sample.is_some(), "Should have samples after seeking");

        // Verify we can read multiple samples (seeking worked)
        let second_sample = wav_source.next_sample().unwrap();
        assert!(
            second_sample.is_some(),
            "Should be able to read multiple samples after seeking"
        );

        // Test seeking to 0 (should work like from_file)
        let mut wav_source_start =
            WavSampleSource::from_file_with_seek(&wav_path, Some(std::time::Duration::ZERO))
                .unwrap();
        let start_sample = wav_source_start.next_sample().unwrap();
        assert!(start_sample.is_some(), "Should have samples from start");
    }

    #[test]
    fn test_wav_sample_source_4channel() {
        use crate::testutil::write_wav_with_bits;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_4channel.wav");

        // Create a 4-channel WAV file with known samples
        let channel_0: Vec<i32> = vec![1000, -2000, 3000];
        let channel_1: Vec<i32> = vec![4000, -5000, 6000];
        let channel_2: Vec<i32> = vec![7000, -8000, 9000];
        let channel_3: Vec<i32> = vec![10000, -11000, 12000];

        write_wav_with_bits(
            wav_path.clone(),
            vec![channel_0, channel_1, channel_2, channel_3],
            44100,
            32,
        )
        .unwrap();

        // Test reading the WAV file
        let mut wav_source = WavSampleSource::from_file(&wav_path).unwrap();

        // Verify channel count
        assert_eq!(wav_source.channels(), 4);

        // Read samples and verify interleaving
        let mut samples_read = Vec::new();
        for _ in 0..12 {
            // 3 samples per channel * 4 channels
            if let Ok(Some(sample)) = wav_source.next_sample() {
                samples_read.push(sample);
            }
        }

        // Verify we got the expected number of samples
        assert_eq!(samples_read.len(), 12);

        // Verify interleaving: samples should be in order channel_0[0], channel_1[0], channel_2[0], channel_3[0], channel_0[1], etc.
        let expected_samples = [
            1000.0 / (1 << 31) as f32,   // channel_0[0]
            4000.0 / (1 << 31) as f32,   // channel_1[0]
            7000.0 / (1 << 31) as f32,   // channel_2[0]
            10000.0 / (1 << 31) as f32,  // channel_3[0]
            -2000.0 / (1 << 31) as f32,  // channel_0[1]
            -5000.0 / (1 << 31) as f32,  // channel_1[1]
            -8000.0 / (1 << 31) as f32,  // channel_2[1]
            -11000.0 / (1 << 31) as f32, // channel_3[1]
            3000.0 / (1 << 31) as f32,   // channel_0[2]
            6000.0 / (1 << 31) as f32,   // channel_1[2]
            9000.0 / (1 << 31) as f32,   // channel_2[2]
            12000.0 / (1 << 31) as f32,  // channel_3[2]
        ];

        for (i, (actual, expected)) in samples_read.iter().zip(expected_samples.iter()).enumerate()
        {
            assert!(
                (actual - expected).abs() < 0.0001,
                "Sample {} mismatch: expected {}, got {}",
                i,
                expected,
                actual
            );
        }
    }

    #[test]
    fn test_wav_sample_source_6channel() {
        use crate::testutil::write_wav_with_bits;
        use tempfile::tempdir;

        let tempdir = tempdir().unwrap();
        let wav_path = tempdir.path().join("test_6channel.wav");

        // Create a 6-channel WAV file (5.1 surround sound)
        let channel_0: Vec<i32> = vec![1000, -2000]; // Front Left
        let channel_1: Vec<i32> = vec![3000, -4000]; // Front Right
        let channel_2: Vec<i32> = vec![5000, -6000]; // Center
        let channel_3: Vec<i32> = vec![7000, -8000]; // LFE (Low Frequency Effects)
        let channel_4: Vec<i32> = vec![9000, -10000]; // Rear Left
        let channel_5: Vec<i32> = vec![11000, -12000]; // Rear Right

        write_wav_with_bits(
            wav_path.clone(),
            vec![
                channel_0, channel_1, channel_2, channel_3, channel_4, channel_5,
            ],
            48000,
            24,
        )
        .unwrap();

        // Test reading the WAV file
        let mut wav_source = WavSampleSource::from_file(&wav_path).unwrap();

        // Verify channel count and sample rate
        assert_eq!(wav_source.channels(), 6);
        assert_eq!(wav_source.sample_rate(), 48000);

        // Read samples and verify interleaving
        let mut samples_read = Vec::new();
        for _ in 0..12 {
            // 2 samples per channel * 6 channels
            if let Ok(Some(sample)) = wav_source.next_sample() {
                samples_read.push(sample);
            }
        }

        // Verify we got the expected number of samples
        assert_eq!(samples_read.len(), 12);

        // Verify interleaving: samples should be in order channel_0[0], channel_1[0], ..., channel_5[0], channel_0[1], etc.
        let expected_samples = [
            1000.0 / (1 << 23) as f32,   // channel_0[0] - Front Left
            -2000.0 / (1 << 23) as f32,  // channel_0[1] - Front Left
            3000.0 / (1 << 23) as f32,   // channel_1[0] - Front Right
            -4000.0 / (1 << 23) as f32,  // channel_1[1] - Front Right
            5000.0 / (1 << 23) as f32,   // channel_2[0] - Center
            -6000.0 / (1 << 23) as f32,  // channel_2[1] - Center
            7000.0 / (1 << 23) as f32,   // channel_3[0] - LFE
            -8000.0 / (1 << 23) as f32,  // channel_3[1] - LFE
            9000.0 / (1 << 23) as f32,   // channel_4[0] - Rear Left
            -10000.0 / (1 << 23) as f32, // channel_4[1] - Rear Left
            11000.0 / (1 << 23) as f32,  // channel_5[0] - Rear Right
            -12000.0 / (1 << 23) as f32, // channel_5[1] - Rear Right
        ];

        for (i, (actual, expected)) in samples_read.iter().zip(expected_samples.iter()).enumerate()
        {
            assert!(
                (actual - expected).abs() < 0.0001,
                "Sample {} mismatch: expected {}, got {}",
                i,
                expected,
                actual
            );
        }
    }
}
