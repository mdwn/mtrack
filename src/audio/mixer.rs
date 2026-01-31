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
// Core audio mixing logic that can be used by both CPAL and test implementations
use crate::audio::sample_source::ChannelMappedSampleSource;
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
#[cfg(test)]
use std::time::Instant;

/// Core audio mixing logic that's independent of any audio backend
#[derive(Clone)]
pub struct AudioMixer {
    /// Active audio sources currently playing
    active_sources: Arc<RwLock<Vec<Arc<Mutex<ActiveSource>>>>>,
    /// Number of output channels
    num_channels: u16,
    /// Sample rate
    sample_rate: u32,
    /// Global sample counter for scheduling (increments each frame processed)
    sample_counter: Arc<AtomicU64>,
    /// Performance monitoring (test only)
    #[cfg(test)]
    frame_count: Arc<AtomicUsize>,
    #[cfg(test)]
    total_frame_time: Arc<AtomicUsize>, // in microseconds
    #[cfg(test)]
    max_frame_time: Arc<AtomicUsize>, // in microseconds
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
    /// Cached source channel count (avoids repeated trait calls)
    pub cached_source_channel_count: u16,
    /// Whether this source has finished playing
    pub is_finished: Arc<AtomicBool>,
    /// Cancel handle for this source
    pub cancel_handle: crate::playsync::CancelHandle,
    /// Sample count at which this source should start playing (for fixed-latency scheduling)
    /// If None, the source plays immediately
    pub start_at_sample: Option<u64>,
    /// Sample count at which this source should stop playing (for scheduled cuts)
    /// If None, the source plays until finished or cancelled
    pub cancel_at_sample: Option<Arc<std::sync::atomic::AtomicU64>>,
}

impl AudioMixer {
    /// Creates a new audio mixer
    pub fn new(num_channels: u16, sample_rate: u32) -> Self {
        Self {
            active_sources: Arc::new(RwLock::new(Vec::new())),
            num_channels,
            sample_rate,
            sample_counter: Arc::new(AtomicU64::new(0)),
            #[cfg(test)]
            frame_count: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            total_frame_time: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            max_frame_time: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Returns the current sample count (for scheduling triggered sources)
    pub fn current_sample(&self) -> u64 {
        self.sample_counter.load(Ordering::Relaxed)
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
        // Cache source channel count (avoids repeated trait calls)
        if source.cached_source_channel_count == 0 {
            source.cached_source_channel_count = source.source.source_channel_count();
        }
        // Precompute channel mappings for optimal performance
        let channel_mappings =
            Self::precompute_channel_mappings(source.source.as_ref(), &source.track_mappings);
        source.channel_mappings = channel_mappings;

        let mut sources = self.active_sources.write().unwrap();
        sources.push(Arc::new(Mutex::new(source)));
    }

    /// Removes sources by ID
    pub fn remove_sources(&self, source_ids: Vec<u64>) {
        let source_ids_set: HashSet<u64> = source_ids.into_iter().collect();
        let mut sources = self.active_sources.write().unwrap();
        sources.retain(|source| {
            let source_guard = source.lock().unwrap();
            !source_ids_set.contains(&source_guard.id)
        });
    }

    /// Processes one frame of audio mixing with performance monitoring (test only)
    /// This is the core mixing logic extracted from the CPAL callback
    /// Minimizes lock duration by cloning Arc references and processing without holding the lock
    #[cfg(test)]
    pub fn process_frame(&self) -> Vec<f32> {
        #[cfg(test)]
        let start_time = Instant::now();
        let mut frame = vec![0.0f32; self.num_channels as usize];

        // Get a snapshot of source references to process (minimize lock duration)
        let sources_to_process = {
            let sources = self.active_sources.read().unwrap();
            sources.clone()
        };

        let mut finished_source_ids = HashSet::new();
        // Reusable scratch buffer for source frames (max 64 channels should cover most cases)
        let mut source_frame_buffer = vec![0.0f32; 64];

        // Process each source without holding the lock
        for active_source_arc in sources_to_process {
            let mut active_source = active_source_arc.lock().unwrap();

            if active_source.is_finished.load(Ordering::Relaxed)
                || active_source.cancel_handle.is_cancelled()
            {
                finished_source_ids.insert(active_source.id);
                continue;
            }

            // Get next frame from this source
            let source_channel_count = active_source.cached_source_channel_count as usize;
            // Resize buffer if needed (should be rare)
            if source_frame_buffer.len() < source_channel_count {
                source_frame_buffer.resize(source_channel_count, 0.0);
            }

            match active_source
                .source
                .next_frame(&mut source_frame_buffer[..source_channel_count])
            {
                Ok(Some(_count)) => {
                    // Process each channel in the source frame using precomputed mappings
                    for (source_channel, &sample) in source_frame_buffer[..source_channel_count]
                        .iter()
                        .enumerate()
                    {
                        // Use precomputed channel mappings for optimal performance
                        if let Some(output_channels) =
                            active_source.channel_mappings.get(source_channel)
                        {
                            // Map this sample to all precomputed output channels
                            for &output_index in output_channels {
                                if output_index < frame.len() {
                                    // Mix: add new sample to existing
                                    frame[output_index] += sample;
                                }
                            }
                        }
                    }
                }
                Ok(None) => {
                    active_source.is_finished.store(true, Ordering::Relaxed);
                    finished_source_ids.insert(active_source.id);
                }
                Err(_) => {
                    active_source.is_finished.store(true, Ordering::Relaxed);
                    finished_source_ids.insert(active_source.id);
                }
            }
        }

        // Remove finished sources in a separate, quick write lock
        if !finished_source_ids.is_empty() {
            let mut sources = self.active_sources.write().unwrap();
            sources.retain(|source| {
                let source_guard = source.lock().unwrap();
                !finished_source_ids.contains(&source_guard.id)
            });
        }

        // Update performance statistics (test only)
        #[cfg(test)]
        {
            let frame_time = start_time.elapsed();
            let frame_time_us = frame_time.as_micros() as usize;

            self.frame_count.fetch_add(1, Ordering::Relaxed);
            self.total_frame_time
                .fetch_add(frame_time_us, Ordering::Relaxed);

            // Update max frame time (using compare_and_swap for thread safety)
            let mut current_max = self.max_frame_time.load(Ordering::Relaxed);
            while frame_time_us > current_max {
                match self.max_frame_time.compare_exchange_weak(
                    current_max,
                    frame_time_us,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(x) => current_max = x,
                }
            }
        }

        frame
    }

    /// Processes multiple frames of audio mixing (test only)
    #[cfg(test)]
    pub fn process_frames(&self, num_frames: usize) -> Vec<f32> {
        let mut frames = Vec::with_capacity(num_frames * self.num_channels as usize);

        for _ in 0..num_frames {
            let frame = self.process_frame();
            frames.extend(frame);
        }

        frames
    }

    /// Processes multiple frames directly into the provided output buffer (zero-allocation)
    /// The buffer must be sized to num_frames * num_channels.
    pub fn process_into_output(&self, output: &mut [f32], num_frames: usize) {
        let channels = self.num_channels as usize;
        debug_assert_eq!(output.len(), num_frames * channels);

        // Get current sample position for scheduling
        let current_sample = self.sample_counter.load(Ordering::Relaxed);
        let buffer_end_sample = current_sample + num_frames as u64;

        // Clear the buffer once
        output.fill(0.0);

        // Get a snapshot of source references to process (minimize lock duration)
        let sources_to_process = {
            let sources = self.active_sources.read().unwrap();
            sources.clone()
        };

        let mut finished_source_ids = HashSet::new();
        // Reusable scratch buffer for source frames (max 64 channels should cover most cases)
        let mut source_frame_buffer = vec![0.0f32; 64];

        // Process each active source across all frames
        for active_source_arc in sources_to_process {
            let mut active_source = active_source_arc.lock().unwrap();

            if active_source.is_finished.load(Ordering::Relaxed)
                || active_source.cancel_handle.is_cancelled()
            {
                finished_source_ids.insert(active_source.id);
                continue;
            }

            // Check if this source has a scheduled cancellation time
            if let Some(ref cancel_at) = active_source.cancel_at_sample {
                let cancel_sample = cancel_at.load(Ordering::Relaxed);
                if cancel_sample > 0 && current_sample >= cancel_sample {
                    // Scheduled cancellation time reached
                    active_source.is_finished.store(true, Ordering::Relaxed);
                    finished_source_ids.insert(active_source.id);
                    continue;
                }
            }

            // Check if this source should start playing yet (fixed-latency scheduling)
            let start_frame = if let Some(start_at) = active_source.start_at_sample {
                if start_at >= buffer_end_sample {
                    // Source hasn't reached its start time yet, skip entirely
                    continue;
                }
                // Calculate which frame in this buffer to start at
                if start_at > current_sample {
                    (start_at - current_sample) as usize
                } else {
                    0 // Start time already passed, play from beginning of buffer
                }
            } else {
                0 // No scheduling, play immediately
            };

            // Check if this source has a scheduled end time within this buffer
            let end_frame = if let Some(ref cancel_at) = active_source.cancel_at_sample {
                let cancel_sample = cancel_at.load(Ordering::Relaxed);
                if cancel_sample > 0
                    && cancel_sample > current_sample
                    && cancel_sample < buffer_end_sample
                {
                    // Source should stop partway through this buffer
                    (cancel_sample - current_sample) as usize
                } else {
                    num_frames
                }
            } else {
                num_frames
            };

            let source_channel_count = active_source.cached_source_channel_count as usize;
            // Resize buffer if needed (should be rare)
            if source_frame_buffer.len() < source_channel_count {
                source_frame_buffer.resize(source_channel_count, 0.0);
            }

            for frame_index in start_frame..end_frame {
                match active_source
                    .source
                    .next_frame(&mut source_frame_buffer[..source_channel_count])
                {
                    Ok(Some(_count)) => {
                        // Mix using precomputed mappings
                        for (source_channel, &sample) in source_frame_buffer[..source_channel_count]
                            .iter()
                            .enumerate()
                        {
                            if let Some(output_channels) =
                                active_source.channel_mappings.get(source_channel)
                            {
                                let base = frame_index * channels;
                                for &output_index in output_channels {
                                    if output_index < channels {
                                        output[base + output_index] += sample;
                                    }
                                }
                            }
                        }
                    }
                    Ok(None) => {
                        active_source.is_finished.store(true, Ordering::Relaxed);
                        finished_source_ids.insert(active_source.id);
                        break;
                    }
                    Err(_) => {
                        active_source.is_finished.store(true, Ordering::Relaxed);
                        finished_source_ids.insert(active_source.id);
                        break;
                    }
                }
            }
        }

        // Increment the sample counter
        self.sample_counter
            .fetch_add(num_frames as u64, Ordering::Relaxed);

        // Clean up finished sources inline - we're the only accessor in direct callback mode
        if !finished_source_ids.is_empty() {
            let mut sources = self.active_sources.write().unwrap();
            sources.retain(|source| {
                let source_guard = source.lock().unwrap();
                !finished_source_ids.contains(&source_guard.id)
            });
        }
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
    pub fn get_active_sources(&self) -> Arc<RwLock<Vec<Arc<Mutex<ActiveSource>>>>> {
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
            cached_source_channel_count: 1,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            start_at_sample: None,
            cancel_at_sample: None,
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
            cached_source_channel_count: 2,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            start_at_sample: None,
            cancel_at_sample: None,
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
            cached_source_channel_count: 2,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            start_at_sample: None,
            cancel_at_sample: None,
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
            cached_source_channel_count: 32,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            start_at_sample: None,
            cancel_at_sample: None,
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
