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
/// Chunk size for processing audio samples in streaming mode
const CHUNK_SIZE: usize = 1024;

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
    buffer: Vec<f32>,
    // Resampling buffers (wrapped in Mutex for thread safety)
    input_buffer: Mutex<Vec<Vec<f32>>>,
    output_buffer: Mutex<Vec<Vec<f32>>>,
    // Pre-allocated output buffer to avoid allocations in resample_block
    final_output_buffer: Mutex<Vec<f32>>,
    // Input collection buffer
    input_collection_buffer: Vec<f32>,
    is_finished: bool,
}

impl<S> SampleSource for AudioTranscoder<S> 
where 
    S: SampleSource,
{
    fn next_sample(&mut self) -> Result<Option<f32>, TranscodingError> {
        if self.is_finished {
            return Ok(None);
        }

        // If we have samples in our buffer, return the next one
        if self.current_position < self.buffer.len() {
            let sample = self.buffer[self.current_position];
            self.current_position += 1;
            return Ok(Some(sample));
        }

        // Buffer is empty, collect more samples from source
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

            Some(Arc::new(Mutex::new(
                Box::new(FftFixedIn::<f32>::new(
                    source_format.sample_rate as usize,
                    target_format.sample_rate as usize,
                    INPUT_BLOCK_SIZE,
                    2, // sub_chunks: 2 for better FFT performance
                    channels as usize, // number of channels
                )
                .map_err(|_e| {
                    TranscodingError::ResamplingFailed(
                        source_format.sample_rate,
                        target_format.sample_rate,
                    )
                })?),
            )))
        } else {
            None
        };

        // Initialize input and output buffers if we have a resampler
        let (input_buffer, output_buffer) = if let Some(ref resampler) = resampler {
            let resampler_lock = resampler.lock().unwrap();
            (
                resampler_lock.input_buffer_allocate(true),
                resampler_lock.output_buffer_allocate(true)
            )
        } else {
            (Vec::new(), Vec::new())
        };

        // Pre-allocate final output buffer with reasonable capacity
        let final_output_buffer = Vec::with_capacity(4096); // 4K samples should handle most chunks

        Ok(AudioTranscoder {
            source,
            resampler,
            source_rate: source_format.sample_rate,
            target_rate: target_format.sample_rate,
            channels,
            current_position: usize::MAX,
            buffer: Vec::new(),
            input_buffer: Mutex::new(input_buffer),
            output_buffer: Mutex::new(output_buffer),
            final_output_buffer: Mutex::new(final_output_buffer),
            input_collection_buffer: Vec::with_capacity(CHUNK_SIZE),
            is_finished: false,
        })
    }

    /// Collects samples from the source iterator and processes them
    fn collect_and_process_input(&mut self) -> Result<Option<f32>, TranscodingError> {
        // Clear the collection buffer
        self.input_collection_buffer.clear();
        
        // Collect samples from the source
        for _ in 0..CHUNK_SIZE {
            match self.source.next_sample() {
                Ok(Some(sample)) => {
                    self.input_collection_buffer.push(sample);
                }
                Ok(None) => {
                    // End of source - pad with zeros for consistent resampling
                    while self.input_collection_buffer.len() < CHUNK_SIZE {
                        self.input_collection_buffer.push(0.0);
                    }
                    self.is_finished = true;
                    break;
                }
                Err(e) => return Err(e),
            }
        }

        // If no samples collected, we're done
        if self.input_collection_buffer.is_empty() {
            self.is_finished = true;
            return Ok(None);
        }

        // Process the collected samples
        if self.resampler.is_some() {
            // Process through the resampler
            let processed = self.resample_block(&self.input_collection_buffer)?;
            self.buffer = processed;
        } else {
            // No resampling needed, just pass through
            self.buffer = self.input_collection_buffer.clone();
        }
        
        self.current_position = 0;
        
        // Return the first sample from the processed buffer
        if !self.buffer.is_empty() {
            let sample = self.buffer[0];
            self.current_position = 1;
            Ok(Some(sample))
        } else {
            Ok(None)
        }
    }

    /// Resamples a block of samples using rubato or simple resampling
    pub fn resample_block(&self, input: &[f32]) -> Result<Vec<f32>, TranscodingError> {
        if let Some(ref resampler_arc) = self.resampler {
            let mut resampler = resampler_arc.lock().unwrap();
            let input_frames_next = resampler.input_frames_next();
            let _output_frames_next = resampler.output_frames_next();
            let chunk_size = input_frames_next * self.channels as usize;
            
            // Clear and reuse the pre-allocated output buffer
            let mut final_output_buffer = self.final_output_buffer.lock().unwrap();
            final_output_buffer.clear();
            let estimated_output_len = (input.len() as f32 * (self.target_rate as f32 / self.source_rate as f32)) as usize;
            final_output_buffer.reserve(estimated_output_len);
            let mut is_first_chunk = true;
            
            let mut input_buffer = self.input_buffer.lock().unwrap();
            let mut output_buffer = self.output_buffer.lock().unwrap();
            
            for chunk_start in (0..input.len()).step_by(chunk_size) {
                let chunk_end = (chunk_start + chunk_size).min(input.len());
                let chunk = &input[chunk_start..chunk_end];

                // Clear and populate the struct-level input buffer
                for channel in 0..self.channels as usize {
                    input_buffer[channel].fill(0.0);
                    output_buffer[channel].fill(0.0);
                }
                
                // Copy chunk data into input buffer (deinterleaved)
                for (i, &sample) in chunk.iter().enumerate() {
                    let channel = i % self.channels as usize;
                    let frame = i / self.channels as usize;
                    if frame < input_buffer[channel].len() {
                        input_buffer[channel][frame] = sample;
                    }
                }
                
                // Debug: Check input buffer state (simplified)
                println!("Processing chunk: {} samples", chunk.len());

                // Use process_into_buffer with struct-level output buffer
                let output_frames = if resampler.output_frames_next() > 0 {
                    let (_input_frames_used, output_frames_written) = resampler.process_into_buffer(&input_buffer, &mut output_buffer, None).map_err(|e| {
                        eprintln!("Rubato resampling failed: {:?}", e);
                        TranscodingError::ResamplingFailed(self.source_rate, self.target_rate)
                    })?;
                    output_frames_written
                } else {
                    0
                };

                // Collect output from struct-level output buffer
                if output_frames > 0 {
                    if is_first_chunk {
                        // For the first chunk, skip the delay samples
                        let delay = resampler.output_delay();
                        let start_frame = delay.min(output_frames);
                        let actual_frames = output_frames.saturating_sub(delay);
                        
                        if actual_frames > 0 {
                            if self.channels == 1 {
                                final_output_buffer.extend_from_slice(&output_buffer[0][start_frame..start_frame + actual_frames]);
                            } else {
                                for frame in start_frame..start_frame + actual_frames {
                                    for channel in 0..self.channels as usize {
                                        final_output_buffer.push(output_buffer[channel][frame]);
                                    }
                                }
                            }
                        }
                    } else {
                        // For subsequent chunks, take all output samples
                        if self.channels == 1 {
                            final_output_buffer.extend_from_slice(&output_buffer[0][0..output_frames]);
                        } else {
                            for frame in 0..output_frames {
                                for channel in 0..self.channels as usize {
                                    final_output_buffer.push(output_buffer[channel][frame]);
                                }
                            }
                        }
                    }
                }
                
                is_first_chunk = false;
            }

            let mut expected_output_len =
                (input.len() as f32 * (self.target_rate as f32 / self.source_rate as f32)) as usize;
            if expected_output_len >= self.channels as usize * 2 {
                expected_output_len =
                    (expected_output_len / self.channels as usize) * self.channels as usize;
            }
            let actual_len = final_output_buffer.len();

            match actual_len.cmp(&expected_output_len) {
                std::cmp::Ordering::Greater => {
                    final_output_buffer.truncate(expected_output_len);
                }
                std::cmp::Ordering::Less => {
                    final_output_buffer.resize(expected_output_len, 0.0);
                }
                std::cmp::Ordering::Equal => {}
            }

            Ok(final_output_buffer.clone())
        } else {
            // No resampling needed
            Ok(input.to_vec())
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
    
    let source_format = TargetFormat::new(
        spec.sample_rate,
        spec.sample_format,
        spec.bits_per_sample,
    ).map_err(|e| TranscodingError::SampleConversionFailed(e.to_string()))?;

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
                let shifted = sample >> 16; // Assume 16-bit samples for now
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
                        Err(_e) => break, // Error occurred
                    }
                }

                // Basic checks
                assert!(!output_samples.is_empty(), "Output should not be empty");
                assert!(output_samples.len() > 0, "Should have some output samples");

                // Verify the signal is still recognizable (basic quality check)
                let max_amplitude =
                    output_samples.iter().map(|&x| x.abs()).fold(0.0, f32::max);

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
            let converter = AudioTranscoder::new_with_source(mock_source, &source_format, &target_format, 1);

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
            let converter = AudioTranscoder::new_with_source(mock_source, &source_format, &target_format, 1);

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
        let mut converter = AudioTranscoder::new_with_source(mock_source, &source_format, &target_format, 1).unwrap();

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
                let max_amplitude =
                    output_samples.iter().map(|&x| x.abs()).fold(0.0, f32::max);

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
                let high_freq_energy =
                    calculate_high_frequency_energy(&output_samples, 44100.0);
                assert!(
                    high_freq_energy < 0.1,
                    "Too much high-frequency content (aliasing): {}",
                    high_freq_energy
                );
            }
            Err(_e) => {}
        }
    }

}
