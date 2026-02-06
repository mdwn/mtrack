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

//! Sample loading and caching for triggered samples.
//!
//! Samples are loaded entirely into memory at startup for zero-latency playback.

use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use tracing::{debug, info, warn};

use crate::audio::sample_source::{create_sample_source_from_file, MemorySampleSource};
use crate::config::samples::SampleDefinition;

/// Default buffer size for reading samples (in samples, not bytes).
const DEFAULT_BUFFER_SIZE: usize = 4096;

/// A loaded sample that can be played back.
/// The sample data is stored in an Arc for efficient sharing between voices.
#[derive(Clone)]
pub struct LoadedSample {
    /// The sample data as f32 samples (interleaved if multi-channel).
    data: Arc<Vec<f32>>,
    /// Number of channels in the sample.
    channel_count: u16,
    /// Sample rate of the audio data.
    sample_rate: u32,
}

impl LoadedSample {
    /// Creates a new MemorySampleSource for playback with the given volume.
    pub fn create_source(&self, volume: f32) -> MemorySampleSource {
        MemorySampleSource::from_shared(
            self.data.clone(),
            self.channel_count,
            self.sample_rate,
            volume,
        )
    }

    /// Returns the number of channels.
    pub fn channel_count(&self) -> u16 {
        self.channel_count
    }

    /// Returns the memory size in bytes.
    pub fn memory_size(&self) -> usize {
        self.data.len() * std::mem::size_of::<f32>()
    }
}

/// Manages loading and caching of sample data.
pub struct SampleLoader {
    /// Cache of loaded samples by file path.
    cache: HashMap<PathBuf, LoadedSample>,
    /// Target sample rate for transcoding (matches audio output).
    target_sample_rate: u32,
}

impl SampleLoader {
    /// Creates a new sample loader.
    pub fn new(target_sample_rate: u32) -> Self {
        Self {
            cache: HashMap::new(),
            target_sample_rate,
        }
    }

    /// Loads a sample from a file into memory.
    /// Returns a cached version if already loaded.
    pub fn load(&mut self, path: &Path) -> Result<LoadedSample, Box<dyn Error>> {
        // Check cache first
        if let Some(sample) = self.cache.get(path) {
            debug!(path = ?path, "Using cached sample");
            return Ok(sample.clone());
        }

        info!(path = ?path, "Loading sample into memory");

        // Create a sample source from the file (include path in error)
        let mut source = create_sample_source_from_file(path, None, DEFAULT_BUFFER_SIZE).map_err(
            |e| -> Box<dyn std::error::Error> {
                format!("Failed to load sample {}: {}", path.display(), e).into()
            },
        )?;
        let source_sample_rate = source.sample_rate();
        let channel_count = source.channel_count();

        // Read all samples into memory
        let mut samples = Vec::new();
        while let Some(sample) = source.next_sample()? {
            samples.push(sample);
        }

        // Transcode if sample rate doesn't match
        let (final_samples, final_sample_rate) = if source_sample_rate != self.target_sample_rate {
            info!(
                source_rate = source_sample_rate,
                target_rate = self.target_sample_rate,
                "Transcoding sample"
            );
            let transcoded = self.transcode_samples(
                &samples,
                channel_count,
                source_sample_rate,
                self.target_sample_rate,
            )?;
            (transcoded, self.target_sample_rate)
        } else {
            (samples, source_sample_rate)
        };

        // Calculate duration
        let total_samples = final_samples.len();
        let samples_per_channel = total_samples as f64 / channel_count as f64;
        let duration_secs = samples_per_channel / final_sample_rate as f64;
        let duration = Duration::from_secs_f64(duration_secs);

        let loaded = LoadedSample {
            data: Arc::new(final_samples),
            channel_count,
            sample_rate: final_sample_rate,
        };

        info!(
            path = ?path,
            channels = channel_count,
            sample_rate = final_sample_rate,
            duration_ms = duration.as_millis(),
            memory_kb = loaded.memory_size() / 1024,
            "Sample loaded"
        );

        // Cache it
        self.cache.insert(path.to_path_buf(), loaded.clone());

        Ok(loaded)
    }

    /// Loads all samples referenced by a sample definition.
    /// Returns a map of file path to loaded sample.
    pub fn load_definition(
        &mut self,
        definition: &SampleDefinition,
        base_path: &Path,
    ) -> Result<HashMap<PathBuf, LoadedSample>, Box<dyn Error>> {
        let mut loaded = HashMap::new();

        for file in definition.all_files() {
            let full_path = if Path::new(file).is_absolute() {
                PathBuf::from(file)
            } else {
                base_path.join(file)
            };

            match self.load(&full_path) {
                Ok(sample) => {
                    loaded.insert(full_path, sample);
                }
                Err(e) => {
                    warn!(path = ?full_path, error = ?e, "Failed to load sample");
                    return Err(
                        format!("Failed to load sample {}: {}", full_path.display(), e).into(),
                    );
                }
            }
        }

        Ok(loaded)
    }

    /// Returns the total memory used by cached samples.
    pub fn total_memory_usage(&self) -> usize {
        self.cache.values().map(|s| s.memory_size()).sum()
    }

    /// Transcodes samples from one sample rate to another using linear interpolation.
    /// For higher quality, the existing Rubato transcoder could be used, but linear
    /// interpolation is simpler and often sufficient for drum hits and one-shots.
    fn transcode_samples(
        &self,
        samples: &[f32],
        channel_count: u16,
        source_rate: u32,
        target_rate: u32,
    ) -> Result<Vec<f32>, Box<dyn Error>> {
        let ratio = target_rate as f64 / source_rate as f64;
        let source_frames = samples.len() / channel_count as usize;
        let target_frames = (source_frames as f64 * ratio).ceil() as usize;
        let channels = channel_count as usize;

        let mut output = Vec::with_capacity(target_frames * channels);

        for target_frame in 0..target_frames {
            let source_pos = target_frame as f64 / ratio;
            let source_frame = source_pos.floor() as usize;
            let frac = source_pos.fract() as f32;

            for channel in 0..channels {
                let idx0 = source_frame * channels + channel;
                let idx1 = (source_frame + 1) * channels + channel;

                let s0 = samples.get(idx0).copied().unwrap_or(0.0);
                let s1 = samples.get(idx1).copied().unwrap_or(s0);

                // Linear interpolation
                let interpolated = s0 + (s1 - s0) * frac;
                output.push(interpolated);
            }
        }

        Ok(output)
    }
}

impl std::fmt::Debug for SampleLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SampleLoader")
            .field("cached_samples", &self.cache.len())
            .field("target_sample_rate", &self.target_sample_rate)
            .field("total_memory_kb", &(self.total_memory_usage() / 1024))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transcode_samples() {
        let loader = SampleLoader::new(48000);

        // Simple mono sine wave at 44100Hz
        let source_rate = 44100;
        let target_rate = 48000;
        let source_samples: Vec<f32> = (0..4410)
            .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / source_rate as f32).sin())
            .collect();

        let result = loader
            .transcode_samples(&source_samples, 1, source_rate, target_rate)
            .unwrap();

        // Should have more samples at higher rate
        let expected_len = (4410.0_f64 * 48000.0 / 44100.0).ceil() as usize;
        assert_eq!(result.len(), expected_len);
    }

    #[test]
    fn test_transcode_stereo() {
        let loader = SampleLoader::new(48000);

        // Stereo: L=1.0, R=-1.0 alternating
        let source_samples = vec![1.0f32, -1.0, 1.0, -1.0, 1.0, -1.0, 1.0, -1.0];

        let result = loader
            .transcode_samples(&source_samples, 2, 44100, 48000)
            .unwrap();

        // Check that channels are preserved
        assert!(result.len() >= 8);
        // First frame should be close to original
        assert!((result[0] - 1.0).abs() < 0.1);
        assert!((result[1] - (-1.0)).abs() < 0.1);
    }
}
