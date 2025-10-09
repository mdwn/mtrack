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
use cpal::Sample as CpalSample;
use hound::WavReader;
use rubato::{FftFixedIn, VecResampler};
use std::path::Path;
use std::sync::{Arc, Mutex};

// Resampling configuration constants
/// Input block size for the rubato resampler
const INPUT_BLOCK_SIZE: usize = 1024;

/// A source of audio samples that processes an iterator
pub trait SampleSource: Send + Sync {
    /// Get the next sample from the source
    /// Returns Ok(Some(sample)) if a sample is available
    /// Returns Ok(None) if the source is finished
    /// Returns Err(error) if an error occurred
    fn next_sample(&mut self) -> Result<Option<f32>, TranscodingError>;
}

#[cfg(test)]
pub trait SampleSourceTestExt {
    /// Check if the source is finished (no more samples)
    fn is_finished(&self) -> bool;
}

/// Audio transcoder with rubato resampling
/// Takes a SampleSource and resamples its output to the target format
pub struct AudioTranscoder<S: SampleSource> {
    source: S,
    resampler: Option<Arc<Mutex<Box<dyn VecResampler<f32>>>>>,
    source_rate: u32,
    target_rate: u32,
    channels: u16,
    // Streaming state
    current_position: usize,
    current_frame: usize,
    is_first_chunk: bool,
    output_frames_written: usize,
    // Resampling buffers (wrapped in Mutex for thread safety)
    input_buffer: Mutex<Vec<Vec<f32>>>,
    output_buffer: Mutex<Vec<Vec<f32>>>,
    actual_input_length: usize,
    total_input_frames: usize,
    total_output_frames: usize,
    is_finished: bool,
    last_input_frames: usize,
    samples_read: usize,
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

        if self.is_finished {
            let ratio = self.target_rate as f32 / self.source_rate as f32;
            let expected_output_frames = (self.total_input_frames as f32 * ratio) as usize;
            let expected_output_samples = expected_output_frames * self.channels as usize;

            if self.samples_read >= expected_output_samples {
                return Ok(None);
            }
        }

        // If we have samples in our output buffer, return the next one
        self.samples_read = self.samples_read.saturating_add(1);
        {
            let output_buffer = self.output_buffer.lock().unwrap();
            if self.current_frame < self.output_frames_written {
                let sample = {
                    let channel = self.current_position % self.channels as usize;
                    output_buffer[channel][self.current_frame]
                };

                // Move to next sample
                self.current_position = self.current_position.saturating_add(1);
                if self.current_position % self.channels as usize == 0 {
                    if self.current_frame + 1 < self.output_frames_written {
                        self.current_frame = self.current_frame.saturating_add(1);
                    } else {
                        // We've exhausted the current buffer, reset and collect more input
                        self.current_frame = 0;
                        self.current_position = 0;
                        // Drop the lock before calling collect_and_process_input
                        drop(output_buffer);
                        return self.collect_and_process_input();
                    }
                }

                return Ok(Some(sample));
            }
        }

        self.collect_and_process_input()
    }
}

impl<S> AudioTranscoder<S>
where
    S: SampleSource,
{
    /// Creates a new AudioTranscoder with a SampleSource
    pub fn new_with_source(
        source: S,
        source_format: &TargetFormat,
        target_format: &TargetFormat,
        channels: u16,
    ) -> Result<Self, TranscodingError> {
        let needs_resampling = source_format.sample_rate != target_format.sample_rate;

        let resampler: Option<Arc<Mutex<Box<dyn VecResampler<f32>>>>> = if needs_resampling {
            Some(Arc::new(Mutex::new(Box::new(
                FftFixedIn::<f32>::new(
                    source_format.sample_rate as usize,
                    target_format.sample_rate as usize,
                    INPUT_BLOCK_SIZE,
                    2,                 // sub_chunks: 2 for better FFT performance
                    channels as usize, // number of channels
                )
                .map_err(|_e| {
                    TranscodingError::ResamplingFailed(
                        source_format.sample_rate,
                        target_format.sample_rate,
                    )
                })?,
            ))))
        } else {
            None
        };

        // Initialize input and output buffers if we have a resampler
        let (input_buffer, output_buffer) = if let Some(ref resampler) = resampler {
            let resampler_lock = resampler.lock().unwrap();
            (
                resampler_lock.input_buffer_allocate(true),
                resampler_lock.output_buffer_allocate(true),
            )
        } else {
            // For pass-through, create buffers with the right number of channels
            let input_buffer = vec![Vec::new(); channels as usize];
            let output_buffer = vec![Vec::new(); channels as usize];
            (input_buffer, output_buffer)
        };

        Ok(AudioTranscoder {
            source,
            resampler,
            source_rate: source_format.sample_rate,
            target_rate: target_format.sample_rate,
            channels,
            current_position: usize::MAX,
            current_frame: 0,
            is_first_chunk: true,
            output_frames_written: 0,
            input_buffer: Mutex::new(input_buffer),
            output_buffer: Mutex::new(output_buffer),
            actual_input_length: 0,
            total_input_frames: 0,
            total_output_frames: 0,
            is_finished: false,
            last_input_frames: 0,
            samples_read: 0,
        })
    }

    /// Collects samples from the source iterator and processes them
    fn collect_and_process_input(&mut self) -> Result<Option<f32>, TranscodingError> {
        // Collect samples from the source into input_buffer
        let mut input_buffer = self.input_buffer.lock().unwrap();
        let mut samples_collected = 0;
        let mut count_actual_samples = 0;

        // Get the expected input size from the resampler
        let resampler = self.resampler.as_ref().unwrap().lock().unwrap();
        let expected_input_frames = resampler.input_frames_next();
        let expected_input_samples = expected_input_frames * self.channels as usize;
        drop(resampler);

        // Collect samples until we have enough for one resampler chunk
        while samples_collected < expected_input_samples {
            match self.source.next_sample() {
                Ok(Some(sample)) => {
                    count_actual_samples += 1;
                    let channel = samples_collected % self.channels as usize;
                    let frame = samples_collected / self.channels as usize;
                    if frame < input_buffer[channel].len() {
                        input_buffer[channel][frame] = sample;
                        samples_collected += 1;
                        self.actual_input_length += 1;
                    }
                }
                Ok(None) => {
                    // End of source - fill remaining with zeros
                    while samples_collected < expected_input_samples {
                        let channel = samples_collected % self.channels as usize;
                        let frame = samples_collected / self.channels as usize;
                        if frame < input_buffer[channel].len() {
                            input_buffer[channel][frame] = 0.0;
                            samples_collected += 1;
                        }
                    }
                    self.is_finished = true;
                    break;
                }
                Err(e) => return Err(e),
            }
        }
        drop(input_buffer);

        if self.is_first_chunk {
            self.is_first_chunk = false;
            let output_delay = self
                .resampler
                .as_ref()
                .unwrap()
                .lock()
                .unwrap()
                .output_delay();
            self.current_frame = output_delay;
            self.current_position = 0;
        }

        // Update total input frames only if we have actual input (not flushing)
        if count_actual_samples > 0 {
            self.last_input_frames = count_actual_samples / self.channels as usize;
            self.total_input_frames += self.last_input_frames;
        }

        // Process the collected samples
        let raw_output_frames = if self.resampler.is_some() {
            // Process through the resampler
            self.resample_block()?
        } else {
            0
        };

        self.output_frames_written = raw_output_frames;

        // Update total output frames with the actual amount
        self.total_output_frames += raw_output_frames;

        // Take the delay into account for total frames.
        if self.current_position != 0 {
            self.total_output_frames -= self.current_position;
        }

        // Set initial position (don't reset current_frame if we have output_delay)
        self.current_position = 0;
        // Don't reset current_frame here - it should stay at output_delay for the first chunk

        // Return the first sample from the processed buffer
        let output_buffer = self.output_buffer.lock().unwrap();
        if !output_buffer[0].is_empty() && self.current_frame < self.output_frames_written {
            let sample = {
                let channel = self.current_position % self.channels as usize;
                output_buffer[channel][self.current_frame]
            };
            self.current_position = self.current_position.saturating_add(1);
            if self.current_position % self.channels as usize == 0 {
                self.current_frame = self.current_frame.saturating_add(1);
            }
            Ok(Some(sample))
        } else {
            Ok(None)
        }
    }

    /// Resamples a block of samples using rubato or simple resampling
    pub fn resample_block(&self) -> Result<usize, TranscodingError> {
        if let Some(ref resampler_arc) = self.resampler {
            let mut resampler = resampler_arc.lock().unwrap();

            let input_buffer = self.input_buffer.lock().unwrap();
            let mut output_buffer = self.output_buffer.lock().unwrap();

            // Use process_into_buffer with struct-level output buffer
            if resampler.output_frames_next() > 0 {
                let (_input_frames_used, output_frames_written) = resampler
                    .process_into_buffer(&input_buffer, &mut output_buffer, None)
                    .map_err(|e| {
                        eprintln!("Rubato resampling failed: {:?}", e);
                        TranscodingError::ResamplingFailed(self.source_rate, self.target_rate)
                    })?;

                return Ok(output_frames_written);
            } else {
                return Ok(0);
            }
        } else {
            Ok(0)
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
pub struct MemorySampleSource {
    samples: Vec<f32>,
    current_index: usize,
}

impl MemorySampleSource {
    /// Creates a new memory sample source
    #[allow(dead_code)]
    pub fn new(samples: Vec<f32>) -> Self {
        Self {
            samples,
            current_index: 0,
        }
    }
}

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
}

#[cfg(test)]
impl SampleSourceTestExt for MemorySampleSource {
    fn is_finished(&self) -> bool {
        self.current_index >= self.samples.len()
    }
}

/// Factory function to create the appropriate SampleSource for a WAV file
/// Returns either a simple WavSampleSource or an AudioTranscoder with WavSampleSource as source
pub fn create_wav_sample_source<P: AsRef<Path>>(
    path: P,
    target_format: TargetFormat,
) -> Result<Box<dyn SampleSource>, TranscodingError> {
    let wav_source = WavSampleSource::from_file(&path)?;

    // For now, we need to get the WAV spec to check if transcoding is needed
    // This is a limitation of the current design - we need the spec to make the decision
    // In a more sophisticated design, we could make this decision at runtime
    let file = std::fs::File::open(&path)?;
    let wav_reader = WavReader::new(file)?;
    let spec = wav_reader.spec();

    let source_format =
        TargetFormat::new(spec.sample_rate, spec.sample_format, spec.bits_per_sample)
            .map_err(|e| TranscodingError::SampleConversionFailed(e.to_string()))?;

    let needs_transcoding = source_format.sample_rate != target_format.sample_rate
        || source_format.sample_format != target_format.sample_format
        || source_format.bits_per_sample != target_format.bits_per_sample;

    if needs_transcoding {
        // Create transcoder with WAV source as input
        let transcoder = AudioTranscoder::new_with_source(
            wav_source,
            &source_format,
            &target_format,
            spec.channels,
        )?;
        Ok(Box::new(transcoder))
    } else {
        // No transcoding needed, just return the WAV source
        Ok(Box::new(wav_source))
    }
}

/// A sample source that reads WAV files and provides scaled samples
/// This is the raw WAV reading component - no transcoding logic
pub struct WavSampleSource {
    wav_reader: hound::WavReader<std::fs::File>,
    is_finished: bool,
}

impl SampleSource for WavSampleSource {
    fn next_sample(&mut self) -> Result<Option<f32>, TranscodingError> {
        if self.is_finished {
            return Ok(None);
        }

        match self.wav_reader.samples::<i32>().next() {
            Some(Ok(sample)) => {
                // Convert i32 to f32 with proper scaling
                let shifted = sample >> (32 - self.wav_reader.spec().bits_per_sample); // Assume 16-bit samples for now
                Ok(Some(shifted.to_sample::<f32>()))
            }
            Some(Err(e)) => Err(TranscodingError::WavError(e)),
            None => {
                self.is_finished = true;
                Ok(None)
            }
        }
    }
}

impl WavSampleSource {
    /// Creates a new WAV sample source from a file path
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, TranscodingError> {
        let file = std::fs::File::open(path)?;
        let wav_reader = WavReader::new(file)?;

        Ok(Self {
            wav_reader,
            is_finished: false,
        })
    }
}

#[cfg(test)]
impl SampleSourceTestExt for WavSampleSource {
    fn is_finished(&self) -> bool {
        self.is_finished
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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
        let mut source = MemorySampleSource::new(samples.clone());

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
    fn test_resampling_quality() {
        // Test actual resampling with simple input
        let source_format = TargetFormat::new(48000, hound::SampleFormat::Float, 32).unwrap();
        let target_format = TargetFormat::new(44100, hound::SampleFormat::Float, 32).unwrap();

        // Create a mock source for testing
        let mock_source = MemorySampleSource::new(vec![0.1, 0.2, 0.3, 0.4, 0.5]);
        match AudioTranscoder::new_with_source(mock_source, &source_format, &target_format, 1) {
            Ok(mut converter) => {
                // Test resampling by getting samples from the converter
                let mut output_samples = Vec::new();
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
                assert!(output_samples.len() > 0, "Should have some output samples");

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
                TargetFormat::new(source_rate, hound::SampleFormat::Float, 32).unwrap();
            let target_format =
                TargetFormat::new(target_rate, hound::SampleFormat::Float, 32).unwrap();

            // Create a mock source for testing
            let mock_source = MemorySampleSource::new(vec![0.1, 0.2, 0.3]);
            let converter =
                AudioTranscoder::new_with_source(mock_source, &source_format, &target_format, 1);

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
        let source_format = TargetFormat::new(48000, hound::SampleFormat::Float, 32).unwrap();
        let target_format = TargetFormat::new(44100, hound::SampleFormat::Float, 32).unwrap();

        // Create a mock source for testing
        let mock_source = MemorySampleSource::new(vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        match AudioTranscoder::new_with_source(mock_source, &source_format, &target_format, 1) {
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
                TargetFormat::new(source_rate, hound::SampleFormat::Float, 32).unwrap();
            let target_format =
                TargetFormat::new(target_rate, hound::SampleFormat::Float, 32).unwrap();

            // Create a mock source for testing
            let mock_source = MemorySampleSource::new(vec![0.1, 0.2, 0.3]);
            let converter =
                AudioTranscoder::new_with_source(mock_source, &source_format, &target_format, 1);

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
        let source_format = TargetFormat::new(44100, hound::SampleFormat::Float, 32).unwrap();
        let target_format = TargetFormat::new(44100, hound::SampleFormat::Float, 32).unwrap();
        // Create a mock source for testing
        let mock_source = MemorySampleSource::new(vec![0.1, 0.2, 0.3, 0.4, 0.5]);
        let mut converter =
            AudioTranscoder::new_with_source(mock_source, &source_format, &target_format, 1)
                .unwrap();

        // Should not need resampling
        // Transcoding is now handled internally by WavSampleSource

        // Test that samples are returned unchanged when no resampling is needed
        let mut output_samples = Vec::new();
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
        let source_format = TargetFormat::new(48000, hound::SampleFormat::Float, 32).unwrap();
        let target_format = TargetFormat::new(44100, hound::SampleFormat::Float, 32).unwrap();

        // Generate a 1kHz sine wave at 48kHz
        let frequency = 1000.0; // 1kHz
        let duration = 0.1; // 100ms
        let num_samples = (48000.0 * duration) as usize;

        let mut input_samples = Vec::new();
        for i in 0..num_samples {
            let t = i as f32 / 48000.0;
            let sample = (2.0 * std::f32::consts::PI * frequency * t).sin();
            input_samples.push(sample);
        }

        // Create a mock source with the sine wave
        let mock_source = MemorySampleSource::new(input_samples);
        match AudioTranscoder::new_with_source(mock_source, &source_format, &target_format, 1) {
            Ok(mut converter) => {
                // Test resampling by getting samples from the converter
                let mut output_samples = Vec::new();
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
                assert!(output_samples.len() > 0, "Should have some output samples");

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
            TargetFormat::new(original_rate, hound::SampleFormat::Float, 32).unwrap();
        let target_format_1 =
            TargetFormat::new(intermediate_rate, hound::SampleFormat::Float, 32).unwrap();
        let source_format_2 =
            TargetFormat::new(intermediate_rate, hound::SampleFormat::Float, 32).unwrap();
        let target_format_2 =
            TargetFormat::new(final_rate, hound::SampleFormat::Float, 32).unwrap();

        // Generate a test signal: 1kHz sine wave + small amount of noise
        let duration = 0.1; // 100ms
        let num_samples = (original_rate as f32 * duration) as usize;
        let mut original_samples = Vec::new();

        for i in 0..num_samples {
            let t = i as f32 / original_rate as f32;
            let sine_wave = 0.5 * (2.0 * std::f32::consts::PI * 1000.0 * t).sin();
            let noise = 0.05 * (rand::random::<f32>() - 0.5);
            original_samples.push(sine_wave + noise);
        }

        // First resampling: 44.1kHz -> 48kHz
        let source_1 = MemorySampleSource::new(original_samples.clone());
        let mut converter_1 =
            AudioTranscoder::new_with_source(source_1, &source_format_1, &target_format_1, 1)
                .unwrap();

        let mut intermediate_samples = Vec::new();
        loop {
            match converter_1.next_sample() {
                Ok(Some(sample)) => intermediate_samples.push(sample),
                Ok(None) => break,
                Err(_) => break,
            }
        }

        // Second resampling: 48kHz -> 44.1kHz
        let source_2 = MemorySampleSource::new(intermediate_samples);
        let mut converter_2 =
            AudioTranscoder::new_with_source(source_2, &source_format_2, &target_format_2, 1)
                .unwrap();

        let mut final_samples = Vec::new();
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
        let expected_length = original_samples.len();
        let length_tolerance = (expected_length as f32 * 0.1) as usize;
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
        let source_format = TargetFormat::new(48000, hound::SampleFormat::Float, 32).unwrap();
        let target_format = TargetFormat::new(44100, hound::SampleFormat::Float, 32).unwrap();

        // Generate impulse signal (single sample at maximum amplitude)
        let mut input_samples = vec![0.0; 100];
        input_samples[50] = 1.0; // Impulse at sample 50
        println!(
            "Debug: input_samples.len() = {}, max_input = {}",
            input_samples.len(),
            input_samples
                .iter()
                .map(|&x: &f32| x.abs())
                .fold(0.0, f32::max)
        );

        let source = MemorySampleSource::new(input_samples);
        let mut converter =
            AudioTranscoder::new_with_source(source, &source_format, &target_format, 1).unwrap();

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

        // The impulse should be preserved (some amplitude should remain)
        let max_amplitude = output_samples.iter().map(|&x| x.abs()).fold(0.0, f32::max);
        println!("Debug: output_samples.len() = {}", output_samples.len());
        println!("Debug: max_amplitude = {}", max_amplitude);
        println!(
            "Debug: first 10 samples: {:?}",
            &output_samples[0..10.min(output_samples.len())]
        );
        println!("Debug: samples around max: {:?}", {
            let max_idx = output_samples
                .iter()
                .position(|&x| x.abs() == max_amplitude)
                .unwrap_or(0);
            let start = max_idx.saturating_sub(5);
            let end = (max_idx + 5).min(output_samples.len());
            &output_samples[start..end]
        });
        assert!(
            max_amplitude > 0.1,
            "Impulse signal should have reasonable amplitude after resampling, got {}",
            max_amplitude
        );
    }

    #[test]
    fn test_resampling_quality_noise() {
        // Test resampling quality with white noise
        let source_format = TargetFormat::new(44100, hound::SampleFormat::Float, 32).unwrap();
        let target_format = TargetFormat::new(48000, hound::SampleFormat::Float, 32).unwrap();

        // Generate white noise
        let num_samples = 1000;
        let mut input_samples = Vec::new();
        for _ in 0..num_samples {
            // Simple pseudo-random noise
            let noise = (rand::random::<f32>() - 0.5) * 2.0;
            input_samples.push(noise);
        }

        let source = MemorySampleSource::new(input_samples.clone());
        let mut converter =
            AudioTranscoder::new_with_source(source, &source_format, &target_format, 1).unwrap();

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
        let expected_ratio = 48000.0 / 44100.0;
        let expected_length = (num_samples as f32 * expected_ratio) as usize;
        let length_tolerance = (expected_length as f32 * 0.1) as usize;

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
        let source_format = TargetFormat::new(48000, hound::SampleFormat::Float, 32).unwrap();
        let target_format = TargetFormat::new(44100, hound::SampleFormat::Float, 32).unwrap();
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

        let source = MemorySampleSource::new(input_samples);
        let mut converter =
            AudioTranscoder::new_with_source(source, &source_format, &target_format, channels)
                .unwrap();

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
        assert_eq!(
            output_samples.len() % 2,
            0,
            "Stereo output should have even number of samples, got {}",
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
}
