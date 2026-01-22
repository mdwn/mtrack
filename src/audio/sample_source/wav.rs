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
use hound::WavReader;
use std::io::BufReader;
use std::path::Path;

use super::error::TranscodingError;
use super::traits::SampleSource;

#[cfg(test)]
use super::traits::SampleSourceTestExt;

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
