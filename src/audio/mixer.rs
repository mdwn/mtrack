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
// Core audio mixing logic that can be used by both CPAL and test implementations
use crate::audio::sample_source::ChannelMappedSampleSource;
use crate::audio::simd;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

/// Core audio mixing logic that's independent of any audio backend
#[derive(Clone)]
pub struct AudioMixer {
    /// Active audio sources currently playing
    active_sources: Arc<RwLock<Vec<ActiveSource>>>,
    /// Number of output channels
    num_channels: u16,
    /// Sample rate
    sample_rate: u32,
}

/// Represents an active audio source in the mixer
pub struct ActiveSource {
    /// Unique ID for this source
    pub id: u64,
    /// The channel mapped sample source
    pub source: Box<dyn ChannelMappedSampleSource + Send + Sync>,
    /// Track mappings for this source (needed for precomputation)
    pub track_mappings: HashMap<String, Vec<u16>>,
    /// Precomputed channel mappings: source_channel_index -> Vec<output_channel_index>
    /// This eliminates HashMap lookups during mixing for better performance
    pub channel_mappings: Vec<Vec<usize>>,
    /// Whether this source has finished playing
    pub is_finished: Arc<AtomicBool>,
    /// Cancel handle for this source
    pub cancel_handle: crate::playsync::CancelHandle,
}

impl AudioMixer {
    /// Creates a new audio mixer
    pub fn new(num_channels: u16, sample_rate: u32) -> Self {
        Self {
            active_sources: Arc::new(RwLock::new(Vec::new())),
            num_channels,
            sample_rate,
        }
    }

    /// Precomputes channel mappings for optimal performance during mixing
    fn precompute_channel_mappings(
        source: &dyn ChannelMappedSampleSource,
        track_mappings: &HashMap<String, Vec<u16>>,
    ) -> Vec<Vec<usize>> {
        let source_channel_count = source.source_channel_count() as usize;
        let mut channel_mappings = Vec::with_capacity(source_channel_count);

        for source_channel in 0..source_channel_count {
            let mut output_channels = Vec::new();

            // Get the labels for this source channel
            if let Some(labels) = source.channel_mappings().get(source_channel) {
                // For each label, find the corresponding output channels
                for label in labels {
                    if let Some(track_channels) = track_mappings.get(label) {
                        // Convert 1-indexed track channels to 0-indexed output indices
                        for &track_channel in track_channels {
                            let output_index = (track_channel - 1) as usize;
                            output_channels.push(output_index);
                        }
                    }
                }
            }

            channel_mappings.push(output_channels);
        }

        channel_mappings
    }

    /// Adds a new audio source to the mixer
    pub fn add_source(&self, mut source: ActiveSource) {
        // Precompute channel mappings for optimal performance
        let channel_mappings =
            Self::precompute_channel_mappings(source.source.as_ref(), &source.track_mappings);
        source.channel_mappings = channel_mappings;

        let mut sources = self.active_sources.write().unwrap();
        sources.push(source);
    }

    /// Removes sources by ID
    pub fn remove_sources(&self, source_ids: Vec<u64>) {
        let mut sources = self.active_sources.write().unwrap();
        sources.retain(|source| !source_ids.contains(&source.id));
    }

    /// Processes one frame of audio mixing
    /// This is the core mixing logic extracted from the CPAL callback
    pub fn process_frame(&self) -> Vec<f32> {
        let mut frame = vec![0.0f32; self.num_channels as usize];
        
        // Use SIMD to initialize the frame buffer with zeros
        simd::clear_buffer_simd(&mut frame);

        let mut sources = self.active_sources.write().unwrap();
        let mut finished_source_ids = Vec::new();

        sources.retain_mut(|active_source| {
            if active_source.is_finished.load(Ordering::Relaxed)
                || active_source.cancel_handle.is_cancelled()
            {
                finished_source_ids.push(active_source.id);
                return false;
            }

            // Get next frame from this source
            match active_source.source.next_frame() {
                Ok(Some(source_frame)) => {
                    // Use SIMD-optimized mixing with runtime feature detection
                    simd::mix_samples_simd(&mut frame, &source_frame, &active_source.channel_mappings);
                    true
                }
                Ok(None) => {
                    active_source.is_finished.store(true, Ordering::Relaxed);
                    false
                }
                Err(_) => {
                    active_source.is_finished.store(true, Ordering::Relaxed);
                    false
                }
            }
        });

        frame
    }

    /// Processes multiple frames of audio mixing
    pub fn process_frames(&self, num_frames: usize) -> Vec<f32> {
        let mut frames = Vec::with_capacity(num_frames * self.num_channels as usize);

        for _ in 0..num_frames {
            let frame = self.process_frame();
            // Use SIMD-optimized buffer copying for frame extension
            let start_idx = frames.len();
            frames.resize(start_idx + frame.len(), 0.0);
            simd::copy_buffer_simd(&mut frames[start_idx..], &frame);
        }

        frames
    }

    /// Gets the number of output channels
    pub fn num_channels(&self) -> u16 {
        self.num_channels
    }

    /// Gets the sample rate
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Gets a reference to the active sources (for CPAL integration)
    pub fn get_active_sources(&self) -> Arc<RwLock<Vec<ActiveSource>>> {
        self.active_sources.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::sample_source::ChannelMappedSampleSource;
    use crate::playsync::CancelHandle;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    /// Helper function to create a test source using MemorySampleSource wrapped in ChannelMappedSource
    fn create_test_source(
        samples: Vec<f32>,
        channel_count: u16,
        mappings: Vec<Vec<String>>,
    ) -> Box<dyn ChannelMappedSampleSource> {
        let memory_source =
            crate::audio::sample_source::MemorySampleSource::new(samples, channel_count, 44100);
        Box::new(crate::audio::sample_source::ChannelMappedSource::new(
            Box::new(memory_source),
            mappings,
            channel_count,
        ))
    }

    #[test]
    fn test_basic_mixing() {
        let mixer = AudioMixer::new(2, 44100);

        // Create a test source with known samples - single channel
        let samples = vec![0.5, 0.8]; // 2 frames of 1 channel
        let source = create_test_source(samples, 1, vec![vec!["test".to_string()]]);

        let active_source = ActiveSource {
            id: 1,
            source,
            track_mappings: {
                let mut map = HashMap::new();
                map.insert("test".to_string(), vec![1]); // Map to channel 1 only
                map
            },
            channel_mappings: Vec::new(), // Will be precomputed in add_source
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
        };

        mixer.add_source(active_source);

        // Process frames
        let frames = mixer.process_frames(2);

        assert_eq!(frames.len(), 4); // 2 frames * 2 channels
                                     // The output should be: [frame1_ch1, frame1_ch2, frame2_ch1, frame2_ch2]
                                     // Which is: [0.5, 0.0, 0.8, 0.0] based on the input samples
        assert_eq!(frames[0], 0.5); // Frame 1, Channel 1
        assert_eq!(frames[1], 0.0); // Frame 1, Channel 2 (unused)
        assert_eq!(frames[2], 0.8); // Frame 2, Channel 1
        assert_eq!(frames[3], 0.0); // Frame 2, Channel 2 (unused)
    }

    #[test]
    fn test_multiple_source_mixing() {
        let mixer = AudioMixer::new(2, 44100);

        // Add two sources
        let source1 = create_test_source(
            vec![0.5, 0.3],
            2,
            vec![vec!["ch0".to_string()], vec!["ch1".to_string()]],
        );
        let source2 = create_test_source(
            vec![0.2, 0.1],
            2,
            vec![vec!["ch0".to_string()], vec!["ch1".to_string()]],
        );

        let active_source1 = ActiveSource {
            id: 1,
            source: source1,
            track_mappings: {
                let mut map = HashMap::new();
                map.insert("ch0".to_string(), vec![1]);
                map.insert("ch1".to_string(), vec![2]);
                map
            },
            channel_mappings: Vec::new(), // Will be precomputed in add_source
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
        };

        let active_source2 = ActiveSource {
            id: 2,
            source: source2,
            track_mappings: {
                let mut map = HashMap::new();
                map.insert("ch0".to_string(), vec![1]);
                map.insert("ch1".to_string(), vec![2]);
                map
            },
            channel_mappings: Vec::new(), // Will be precomputed in add_source
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
        };

        mixer.add_source(active_source1);
        mixer.add_source(active_source2);

        // Process one frame - should mix both sources
        let frame = mixer.process_frame();

        assert_eq!(frame.len(), 2);
        assert_eq!(frame[0], 0.7); // 0.5 + 0.2
        assert_eq!(frame[1], 0.4); // 0.3 + 0.1
    }

    #[test]
    fn test_32_channel_mixing() {
        let mixer = AudioMixer::new(32, 44100);

        // Create a source that maps to specific channels
        let mut samples = vec![0.0; 64]; // 2 frames * 32 channels
        samples[0] = 0.5; // Channel 0, frame 0
        samples[1] = 0.3; // Channel 1, frame 0
        samples[32] = 0.8; // Channel 0, frame 1
        samples[33] = 0.2; // Channel 1, frame 1

        let source = create_test_source(samples, 32, {
            let mut mappings = vec![vec![]; 32];
            mappings[0] = vec!["ch0".to_string()];
            mappings[1] = vec!["ch1".to_string()];
            mappings
        });
        let active_source = ActiveSource {
            id: 1,
            source,
            track_mappings: {
                let mut map = HashMap::new();
                map.insert("ch0".to_string(), vec![1]); // Map to channel 1
                map.insert("ch1".to_string(), vec![2]); // Map to channel 2
                map
            },
            channel_mappings: Vec::new(), // Will be precomputed in add_source
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
        };

        mixer.add_source(active_source);

        // Process frames
        let frames = mixer.process_frames(2);

        assert_eq!(frames.len(), 64); // 2 frames * 32 channels
        assert_eq!(frames[0], 0.5); // Channel 1, frame 1
        assert_eq!(frames[1], 0.3); // Channel 2, frame 1
        assert_eq!(frames[32], 0.8); // Channel 1, frame 2
        assert_eq!(frames[33], 0.2); // Channel 2, frame 2

        // All other channels should be 0.0
        for frame in frames.iter().take(32).skip(2) {
            assert_eq!(*frame, 0.0);
        }
        for frame in frames.iter().take(64).skip(34) {
            assert_eq!(*frame, 0.0);
        }
    }
}
