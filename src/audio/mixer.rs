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
use rayon::prelude::*;
use std::collections::{HashMap, HashSet};
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
#[cfg(test)]
use std::time::Instant;

/// Result of reading from a single audio source (used for parallel processing)
enum SourceReadResult {
    /// Source finished (EOF, cancelled, or error)
    Finished { id: u64 },
    /// Successfully read frames
    Success {
        id: u64,
        frames_read: usize,
        is_finished: bool,
        planar_data: Vec<Vec<f32>>,
        channel_mappings: Vec<Vec<usize>>,
    },
}

impl SourceReadResult {
    fn finished(id: u64) -> Self {
        Self::Finished { id }
    }

    fn success(
        id: u64,
        frames_read: usize,
        is_finished: bool,
        planar_data: Vec<Vec<f32>>,
        channel_mappings: Vec<Vec<usize>>,
    ) -> Self {
        Self::Success {
            id,
            frames_read,
            is_finished,
            planar_data,
            channel_mappings,
        }
    }
}

/// Pre-allocated buffers for mixing operations (avoids allocation in hot path)
struct MixerBuffers {
    /// Planar output buffer for mixing (one Vec per output channel)
    planar_output: Vec<Vec<f32>>,
    /// IDs of sources that have finished (reused to avoid HashSet allocation)
    finished_ids: Vec<u64>,
    /// Capacity these buffers were sized for
    num_frames_capacity: usize,
    num_channels_capacity: usize,
}

impl MixerBuffers {
    fn new(num_channels: usize, initial_frames: usize) -> Self {
        Self {
            planar_output: (0..num_channels)
                .map(|_| vec![0.0f32; initial_frames])
                .collect(),
            finished_ids: Vec::with_capacity(8),
            num_frames_capacity: initial_frames,
            num_channels_capacity: num_channels,
        }
    }

    /// Ensure buffers are sized for the given frame count, resizing only if needed
    fn ensure_capacity(&mut self, num_channels: usize, num_frames: usize) {
        // Resize channel count if needed
        if self.planar_output.len() != num_channels {
            self.planar_output.resize_with(num_channels, Vec::new);
            self.num_channels_capacity = num_channels;
        }

        // Resize frame capacity if needed (only grow, never shrink)
        if num_frames > self.num_frames_capacity {
            for ch in &mut self.planar_output {
                ch.resize(num_frames, 0.0);
            }
            self.num_frames_capacity = num_frames;
        }
    }

    /// Clear and prepare buffers for a new mixing pass
    fn prepare(&mut self, num_channels: usize, num_frames: usize) {
        self.ensure_capacity(num_channels, num_frames);

        // Zero out the output buffers (only the portion we'll use)
        for ch in &mut self.planar_output {
            for sample in ch.iter_mut().take(num_frames) {
                *sample = 0.0;
            }
        }

        self.finished_ids.clear();
    }
}

/// Core audio mixing logic that's independent of any audio backend.
/// Uses planar format internally for efficient mixing, interleaves only at output.
#[derive(Clone)]
pub struct AudioMixer {
    /// Active audio sources currently playing
    active_sources: Arc<RwLock<Vec<Arc<Mutex<ActiveSource>>>>>,
    /// Number of output channels
    num_channels: u16,
    /// Sample rate
    sample_rate: u32,
    /// Pre-allocated buffers for mixing (shared via Arc<Mutex> for Clone support)
    buffers: Arc<Mutex<MixerBuffers>>,
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
    /// Pre-allocated planar buffer for reading from this source
    pub planar_read_buffer: Vec<Vec<f32>>,
}

impl AudioMixer {
    /// Creates a new audio mixer
    pub fn new(num_channels: u16, sample_rate: u32) -> Self {
        // Pre-allocate buffers for typical audio callback size (1024 frames is common)
        let initial_frames = 1024;
        Self {
            active_sources: Arc::new(RwLock::new(Vec::new())),
            num_channels,
            sample_rate,
            buffers: Arc::new(Mutex::new(MixerBuffers::new(
                num_channels as usize,
                initial_frames,
            ))),
            #[cfg(test)]
            frame_count: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            total_frame_time: Arc::new(AtomicUsize::new(0)),
            #[cfg(test)]
            max_frame_time: Arc::new(AtomicUsize::new(0)),
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
        // Cache source channel count (avoids repeated trait calls)
        if source.cached_source_channel_count == 0 {
            source.cached_source_channel_count = source.source.source_channel_count();
        }

        // Pre-allocate planar read buffer for this source
        let source_channels = source.cached_source_channel_count as usize;
        source.planar_read_buffer = vec![Vec::new(); source_channels];

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
    /// Returns interleaved output for compatibility with existing tests.
    #[cfg(test)]
    pub fn process_frame(&self) -> Vec<f32> {
        #[cfg(test)]
        let start_time = Instant::now();

        let channels = self.num_channels as usize;
        let mut frame = vec![0.0f32; channels];

        // Get a snapshot of source references to process (minimize lock duration)
        let sources_to_process = {
            let sources = self.active_sources.read().unwrap();
            sources.clone()
        };

        let mut finished_source_ids = HashSet::new();

        // Process each source without holding the lock
        for active_source_arc in sources_to_process {
            let mut active_source = active_source_arc.lock().unwrap();

            if active_source.is_finished.load(Ordering::Relaxed)
                || active_source.cancel_handle.is_cancelled()
            {
                finished_source_ids.insert(active_source.id);
                continue;
            }

            let source_channel_count = active_source.cached_source_channel_count as usize;

            // Take buffer out of struct to avoid borrow conflict
            let mut read_buffer = std::mem::take(&mut active_source.planar_read_buffer);

            // Ensure planar buffer is properly sized
            if read_buffer.len() != source_channel_count {
                read_buffer = vec![Vec::new(); source_channel_count];
            }

            // Read one frame in planar format
            let result = active_source.source.next_frames(&mut read_buffer, 1);

            match result {
                Ok(frames_read) if frames_read > 0 => {
                    // Mix planar data into output frame
                    for (source_channel, channel_samples) in read_buffer.iter().enumerate() {
                        if let Some(&sample) = channel_samples.first() {
                            if let Some(output_channels) =
                                active_source.channel_mappings.get(source_channel)
                            {
                                for &output_index in output_channels {
                                    if output_index < channels {
                                        frame[output_index] += sample;
                                    }
                                }
                            }
                        }
                    }
                }
                Ok(_) => {
                    // 0 frames read means EOF
                    active_source.is_finished.store(true, Ordering::Relaxed);
                    finished_source_ids.insert(active_source.id);
                }
                Err(_) => {
                    active_source.is_finished.store(true, Ordering::Relaxed);
                    finished_source_ids.insert(active_source.id);
                }
            }

            // Put buffer back
            active_source.planar_read_buffer = read_buffer;
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
    /// Returns interleaved output.
    #[cfg(test)]
    pub fn process_frames(&self, num_frames: usize) -> Vec<f32> {
        let mut frames = Vec::with_capacity(num_frames * self.num_channels as usize);

        for _ in 0..num_frames {
            let frame = self.process_frame();
            frames.extend(frame);
        }

        frames
    }

    /// Processes multiple frames directly into the provided interleaved output buffer.
    /// Mixes in planar format internally for efficiency, then interleaves at the end.
    /// The buffer must be sized to num_frames * num_channels.
    /// Uses parallel processing to decode/resample sources concurrently.
    pub fn process_into_output(&self, output: &mut [f32], num_frames: usize) {
        let channels = self.num_channels as usize;
        debug_assert_eq!(output.len(), num_frames * channels);

        // Lock pre-allocated buffers and prepare them for this mixing pass
        let mut buffers = self.buffers.lock().unwrap();
        buffers.prepare(channels, num_frames);

        // Get a snapshot of source references to process (minimize lock duration)
        let sources_to_process: Vec<_> = {
            let sources = self.active_sources.read().unwrap();
            sources.clone()
        };

        // Read from all sources in parallel - rayon handles small collections efficiently
        // Each source decodes/resamples independently, which is the expensive work
        let read_results: Vec<_> = sources_to_process
            .par_iter()
            .map(|active_source_arc| {
                let mut active_source = active_source_arc.lock().unwrap();

                // Check if source should be skipped
                if active_source.is_finished.load(Ordering::Relaxed)
                    || active_source.cancel_handle.is_cancelled()
                {
                    return SourceReadResult::finished(active_source.id);
                }

                let source_channel_count = active_source.cached_source_channel_count as usize;

                // Take buffer out of struct to avoid borrow conflict
                let mut read_buffer = std::mem::take(&mut active_source.planar_read_buffer);

                // Ensure planar buffer is properly sized
                if read_buffer.len() != source_channel_count {
                    read_buffer = vec![Vec::new(); source_channel_count];
                }

                // Read all frames at once in planar format (the expensive part)
                let result = active_source.source.next_frames(&mut read_buffer, num_frames);

                // Build result based on read outcome
                match result {
                    Ok(frames_read) => {
                        if frames_read == 0 {
                            // EOF - put buffer back immediately
                            active_source.is_finished.store(true, Ordering::Relaxed);
                            active_source.planar_read_buffer = read_buffer;
                            SourceReadResult::finished(active_source.id)
                        } else {
                            let is_done = frames_read < num_frames;
                            if is_done {
                                active_source.is_finished.store(true, Ordering::Relaxed);
                            }
                            // Move buffer into result - will be returned after mixing
                            SourceReadResult::success(
                                active_source.id,
                                frames_read,
                                is_done,
                                read_buffer,
                                active_source.channel_mappings.clone(),
                            )
                        }
                    }
                    Err(_) => {
                        // Error - put buffer back immediately
                        active_source.is_finished.store(true, Ordering::Relaxed);
                        active_source.planar_read_buffer = read_buffer;
                        SourceReadResult::finished(active_source.id)
                    }
                }
            })
            .collect();

        // Mix all results into output buffer (sequential - fast and avoids contention)
        for (idx, result) in read_results.into_iter().enumerate() {
            match result {
                SourceReadResult::Finished { id } => {
                    buffers.finished_ids.push(id);
                }
                SourceReadResult::Success {
                    id,
                    frames_read,
                    is_finished,
                    planar_data,
                    channel_mappings,
                } => {
                    // Mix planar data into output
                    for (source_channel, channel_samples) in planar_data.iter().enumerate() {
                        if let Some(output_channels) = channel_mappings.get(source_channel) {
                            for &output_index in output_channels {
                                if output_index < channels {
                                    for (frame_idx, &sample) in
                                        channel_samples.iter().take(frames_read).enumerate()
                                    {
                                        buffers.planar_output[output_index][frame_idx] += sample;
                                    }
                                }
                            }
                        }
                    }

                    // Return buffer to source
                    if let Some(source_arc) = sources_to_process.get(idx) {
                        let mut source = source_arc.lock().unwrap();
                        source.planar_read_buffer = planar_data;
                    }

                    if is_finished {
                        buffers.finished_ids.push(id);
                    }
                }
            }
        }

        // Remove finished sources in a separate, quick write lock
        if !buffers.finished_ids.is_empty() {
            let mut sources = self.active_sources.write().unwrap();
            sources.retain(|source| {
                let source_guard = source.lock().unwrap();
                !buffers.finished_ids.contains(&source_guard.id)
            });
        }

        // Interleave planar output into the final interleaved buffer
        // This is the ONLY place we interleave in the entire pipeline
        for frame_idx in 0..num_frames {
            let output_base = frame_idx * channels;
            for (ch_idx, channel_data) in buffers.planar_output.iter().enumerate() {
                output[output_base + ch_idx] = channel_data[frame_idx];
            }
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
            channel_mappings: Vec::new(),
            cached_source_channel_count: 1,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            planar_read_buffer: Vec::new(),
        };

        mixer.add_source(active_source);

        // Process frames
        let frames = mixer.process_frames(2);

        assert_eq!(frames.len(), 4); // 2 frames * 2 channels
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
            channel_mappings: Vec::new(),
            cached_source_channel_count: 2,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            planar_read_buffer: Vec::new(),
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
            channel_mappings: Vec::new(),
            cached_source_channel_count: 2,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            planar_read_buffer: Vec::new(),
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
            channel_mappings: Vec::new(),
            cached_source_channel_count: 32,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            planar_read_buffer: Vec::new(),
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
