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
use parking_lot::{Mutex, RwLock};
use std::cell::RefCell;
use std::collections::HashMap;
#[cfg(test)]
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
#[cfg(test)]
use std::time::Instant;
use tracing::debug;

// Thread-local scratch buffers for process_into_output.
// BATCH_READ_SCRATCH: 8192 samples covers 512 frames * 16 channels; resized if needed.
// SOURCES_SCRATCH: reuses the Vec across callbacks to avoid per-callback heap allocation.
thread_local! {
    static BATCH_READ_SCRATCH: RefCell<Vec<f32>> = RefCell::new(vec![0.0; 8192]);
    static SOURCES_SCRATCH: RefCell<Vec<Arc<Mutex<ActiveSource>>>> = const { RefCell::new(Vec::new()) };
}

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

        let mut sources = self.active_sources.write();
        sources.push(Arc::new(Mutex::new(source)));
    }

    /// Removes sources by ID.
    /// Note: `source_ids.contains()` is O(M) per retained source, making this
    /// O(N * M) overall. This is fine for typical counts (< 32 sources) but would
    /// need a HashSet if source counts ever grow significantly.
    pub fn remove_sources(&self, source_ids: &[u64]) {
        let mut sources = self.active_sources.write();
        sources.retain(|source| {
            let source_guard = source.lock();
            !source_ids.contains(&source_guard.id)
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
            let sources = self.active_sources.read();
            sources.clone()
        };

        let mut finished_source_ids = Vec::new();
        // Reusable scratch buffer for source frames (max 64 channels should cover most cases)
        let mut source_frame_buffer = vec![0.0f32; 64];

        // Process each source without holding the lock
        for active_source_arc in sources_to_process {
            let mut active_source = active_source_arc.lock();

            if active_source.is_finished.load(Ordering::Relaxed)
                || active_source.cancel_handle.is_cancelled()
            {
                finished_source_ids.push(active_source.id);
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
                    if active_source.source.is_exhausted().unwrap_or(true) {
                        active_source.is_finished.store(true, Ordering::Relaxed);
                        finished_source_ids.push(active_source.id);
                    }
                }
                Err(_) => {
                    active_source.is_finished.store(true, Ordering::Relaxed);
                    finished_source_ids.push(active_source.id);
                }
            }
        }

        // Remove finished sources in a separate, quick write lock
        if !finished_source_ids.is_empty() {
            self.remove_sources(&finished_source_ids);
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

        // Take the scratch Vec (reuses capacity across callbacks, avoids heap alloc)
        let mut sources_to_process =
            SOURCES_SCRATCH.with(|cell| std::mem::take(&mut *cell.borrow_mut()));
        sources_to_process.clear();

        // Fill from active_sources (quick read lock)
        {
            let sources = self.active_sources.read();
            sources_to_process.extend(sources.iter().cloned());
        }

        let mut finished_source_ids: Vec<u64> = Vec::new();

        // Process each active source across all frames
        for active_source_arc in sources_to_process.iter() {
            let mut active_source = active_source_arc.lock();

            if active_source.is_finished.load(Ordering::Relaxed)
                || active_source.cancel_handle.is_cancelled()
            {
                debug!(
                    source_id = active_source.id,
                    reason = if active_source.is_finished.load(Ordering::Relaxed) {
                        "already_finished"
                    } else {
                        "cancel_handle_cancelled"
                    },
                    "mixer: source marked finished (skip)"
                );
                finished_source_ids.push(active_source.id);
                continue;
            }

            // Check if this source has a scheduled cancellation time
            if let Some(ref cancel_at) = active_source.cancel_at_sample {
                let cancel_sample = cancel_at.load(Ordering::Relaxed);
                if cancel_sample > 0 && current_sample >= cancel_sample {
                    debug!(
                        source_id = active_source.id,
                        cancel_sample,
                        current_sample,
                        "mixer: source marked finished (cancel_at_sample reached)"
                    );
                    active_source.is_finished.store(true, Ordering::Relaxed);
                    finished_source_ids.push(active_source.id);
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

            // cancel_at_sample can be set by another thread to a value before
            // start_at_sample, making end_frame < start_frame. Skip gracefully.
            // The source stays alive for one extra callback; on the next callback
            // the early cancel_at_sample check (above) will fire and remove it.
            if end_frame <= start_frame {
                continue;
            }

            let source_channel_count = active_source.cached_source_channel_count as usize;
            let frames_needed = end_frame - start_frame;

            // Safety invariant: BATCH_READ_SCRATCH is reused across source iterations
            // without clearing. This is safe because we only read back
            // `frames_got * source_channel_count` samples from the buffer, and
            // `read_frames` is required to write exactly that many samples.
            // Stale data beyond that range is never accessed.
            BATCH_READ_SCRATCH.with(|cell| {
                let mut buf = cell.borrow_mut();
                let batch_samples = frames_needed * source_channel_count;
                if buf.len() < batch_samples {
                    buf.resize(batch_samples, 0.0);
                }
                let batch_buf = &mut buf[..batch_samples];

                match active_source.source.read_frames(batch_buf, frames_needed) {
                    Ok(frames_got) => {
                        for frame_idx in 0..frames_got {
                            let src_offset = frame_idx * source_channel_count;
                            let dst_base = (start_frame + frame_idx) * channels;
                            for (source_channel, &sample) in batch_buf
                                [src_offset..src_offset + source_channel_count]
                                .iter()
                                .enumerate()
                            {
                                if let Some(output_channels) =
                                    active_source.channel_mappings.get(source_channel)
                                {
                                    for &output_index in output_channels {
                                        if output_index < channels {
                                            output[dst_base + output_index] += sample;
                                        }
                                    }
                                }
                            }
                        }
                        if frames_got < frames_needed {
                            // Buffered sources report false during transient underruns;
                            // unbuffered sources return None → treated as EOF.
                            if active_source.source.is_exhausted().unwrap_or(true) {
                                debug!(
                                    source_id = active_source.id,
                                    "mixer: source marked finished (read_frames returned fewer frames)"
                                );
                                active_source.is_finished.store(true, Ordering::Relaxed);
                                finished_source_ids.push(active_source.id);
                            }
                        }
                    }
                    Err(_) => {
                        debug!(
                            source_id = active_source.id,
                            "mixer: source marked finished (read_frames error)"
                        );
                        active_source.is_finished.store(true, Ordering::Relaxed);
                        finished_source_ids.push(active_source.id);
                    }
                }
            });
        }

        // Increment the sample counter
        self.sample_counter
            .fetch_add(num_frames as u64, Ordering::Relaxed);

        // Clean up finished sources inline - we're the only accessor in direct callback mode
        if !finished_source_ids.is_empty() {
            debug!(
                source_ids = ?finished_source_ids,
                remaining_before = self.active_sources.read().len(),
                "mixer: removing finished sources"
            );
            self.remove_sources(&finished_source_ids);
        }

        // Put back the scratch Vec for reuse (drop Arc refs first)
        sources_to_process.clear();
        SOURCES_SCRATCH.with(|cell| {
            *cell.borrow_mut() = sources_to_process;
        });
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

    /// Helper to build an ActiveSource with common defaults.
    fn make_active_source(
        id: u64,
        source: Box<dyn ChannelMappedSampleSource>,
        track_mappings: HashMap<String, Vec<u16>>,
    ) -> ActiveSource {
        ActiveSource {
            id,
            source,
            track_mappings,
            channel_mappings: Vec::new(),
            cached_source_channel_count: 0,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            start_at_sample: None,
            cancel_at_sample: None,
        }
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

    #[test]
    fn test_cancel_before_start_does_not_panic() {
        // Regression: when cancel_at_sample < start_at_sample, the subtraction
        // end_frame - start_frame would underflow. The mixer must handle this
        // gracefully (produce silence, no panic).
        let mixer = AudioMixer::new(2, 44100);

        let samples = vec![0.5; 1024]; // plenty of audio
        let source = create_test_source(samples, 1, vec![vec!["test".to_string()]]);

        // start_at_sample=400, cancel_at_sample=200 — cancel comes before start
        let cancel_at = Arc::new(AtomicU64::new(200));
        let active_source = ActiveSource {
            id: 1,
            source,
            track_mappings: {
                let mut map = HashMap::new();
                map.insert("test".to_string(), vec![1]);
                map
            },
            channel_mappings: Vec::new(),
            cached_source_channel_count: 1,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            start_at_sample: Some(400),
            cancel_at_sample: Some(cancel_at),
        };

        mixer.add_source(active_source);

        // Process a buffer that spans both cancel and start points.
        // current_sample starts at 0, buffer covers samples 0..512.
        // end_frame = (200 - 0) = 200, start_frame = (400 - 0) = 400.
        // Without the fix this would panic on 200 - 400.
        let mut output = vec![0.0f32; 512 * 2];
        mixer.process_into_output(&mut output, 512);

        // Source should have produced silence (skipped entirely)
        for &sample in &output {
            assert_eq!(sample, 0.0);
        }

        // The source lingers for one callback (skipped via `continue`), then on
        // the next callback the early cancel_at_sample check removes it.
        let mut output2 = vec![0.0f32; 512 * 2];
        mixer.process_into_output(&mut output2, 512);
        for &sample in &output2 {
            assert_eq!(sample, 0.0);
        }
        // After two callbacks the source should be cleaned up.
        assert_eq!(mixer.active_sources.read().len(), 0);
    }

    #[test]
    fn test_process_into_output_basic() {
        // Verify that process_into_output produces the same audio as process_frames
        // for a start-aligned source (no scheduling).
        let mixer = AudioMixer::new(2, 44100);

        let samples = vec![0.5, 0.8, 0.3, 0.6]; // 4 frames of 1 channel
        let source = create_test_source(samples, 1, vec![vec!["test".to_string()]]);

        let active_source = ActiveSource {
            id: 1,
            source,
            track_mappings: {
                let mut map = HashMap::new();
                map.insert("test".to_string(), vec![1]);
                map
            },
            channel_mappings: Vec::new(),
            cached_source_channel_count: 1,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            start_at_sample: None,
            cancel_at_sample: None,
        };

        mixer.add_source(active_source);

        let mut output = vec![0.0f32; 4 * 2]; // 4 frames * 2 channels
        mixer.process_into_output(&mut output, 4);

        assert_eq!(output[0], 0.5); // Frame 0, Ch 0
        assert_eq!(output[1], 0.0); // Frame 0, Ch 1 (unmapped)
        assert_eq!(output[2], 0.8); // Frame 1, Ch 0
        assert_eq!(output[3], 0.0); // Frame 1, Ch 1
        assert_eq!(output[4], 0.3); // Frame 2, Ch 0
        assert_eq!(output[5], 0.0); // Frame 2, Ch 1
        assert_eq!(output[6], 0.6); // Frame 3, Ch 0
        assert_eq!(output[7], 0.0); // Frame 3, Ch 1
    }

    #[test]
    fn test_process_into_output_start_at_sample_mid_buffer() {
        // Source starts partway through the buffer (start_at_sample > current_sample).
        let mixer = AudioMixer::new(2, 44100);

        // 4 frames of 1 channel; source should start at frame index 2 in the buffer.
        let samples = vec![0.1, 0.2, 0.3, 0.4];
        let source = create_test_source(samples, 1, vec![vec!["test".to_string()]]);

        let active_source = ActiveSource {
            id: 1,
            source,
            track_mappings: {
                let mut map = HashMap::new();
                map.insert("test".to_string(), vec![1]);
                map
            },
            channel_mappings: Vec::new(),
            cached_source_channel_count: 1,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            start_at_sample: Some(2), // Start at sample 2
            cancel_at_sample: None,
        };

        mixer.add_source(active_source);

        // Buffer covers samples 0..4 (4 frames). Source starts at sample 2.
        let mut output = vec![0.0f32; 4 * 2];
        mixer.process_into_output(&mut output, 4);

        // Frames 0-1 should be silence, frames 2-3 should have source audio.
        assert_eq!(output[0], 0.0); // Frame 0, Ch 0 (before start)
        assert_eq!(output[1], 0.0); // Frame 0, Ch 1
        assert_eq!(output[2], 0.0); // Frame 1, Ch 0 (before start)
        assert_eq!(output[3], 0.0); // Frame 1, Ch 1
        assert_eq!(output[4], 0.1); // Frame 2, Ch 0 (source starts)
        assert_eq!(output[5], 0.0); // Frame 2, Ch 1
        assert_eq!(output[6], 0.2); // Frame 3, Ch 0
        assert_eq!(output[7], 0.0); // Frame 3, Ch 1
    }

    #[test]
    fn test_process_into_output_cancel_at_sample_mid_buffer() {
        // Source is cancelled partway through the buffer.
        let mixer = AudioMixer::new(2, 44100);

        let samples = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
        let source = create_test_source(samples, 1, vec![vec!["test".to_string()]]);

        let cancel_at = Arc::new(AtomicU64::new(3)); // Cancel at sample 3
        let active_source = ActiveSource {
            id: 1,
            source,
            track_mappings: {
                let mut map = HashMap::new();
                map.insert("test".to_string(), vec![1]);
                map
            },
            channel_mappings: Vec::new(),
            cached_source_channel_count: 1,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            start_at_sample: None,
            cancel_at_sample: Some(cancel_at),
        };

        mixer.add_source(active_source);

        // Buffer covers samples 0..8. Source plays frames 0..3 then stops.
        let mut output = vec![0.0f32; 8 * 2];
        mixer.process_into_output(&mut output, 8);

        // Frames 0-2 should have audio, frames 3-7 should be silence.
        assert_eq!(output[0], 0.1); // Frame 0
        assert_eq!(output[2], 0.2); // Frame 1
        assert_eq!(output[4], 0.3); // Frame 2
        assert_eq!(output[6], 0.0); // Frame 3 (after cancel)
        assert_eq!(output[8], 0.0); // Frame 4
        assert_eq!(output[10], 0.0); // Frame 5
    }

    #[test]
    fn test_process_into_output_source_finishes_before_buffer_end() {
        // Source has fewer frames than the buffer size.
        let mixer = AudioMixer::new(2, 44100);

        let samples = vec![0.7, 0.9]; // Only 2 frames of 1 channel
        let source = create_test_source(samples, 1, vec![vec!["test".to_string()]]);

        let active_source = ActiveSource {
            id: 1,
            source,
            track_mappings: {
                let mut map = HashMap::new();
                map.insert("test".to_string(), vec![1]);
                map
            },
            channel_mappings: Vec::new(),
            cached_source_channel_count: 1,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            start_at_sample: None,
            cancel_at_sample: None,
        };

        mixer.add_source(active_source);

        // Request 8 frames but source only has 2.
        let mut output = vec![0.0f32; 8 * 2];
        mixer.process_into_output(&mut output, 8);

        // Frames 0-1 should have audio, the rest should be silence.
        assert_eq!(output[0], 0.7); // Frame 0, Ch 0
        assert_eq!(output[1], 0.0); // Frame 0, Ch 1
        assert_eq!(output[2], 0.9); // Frame 1, Ch 0
        assert_eq!(output[3], 0.0); // Frame 1, Ch 1
        for i in 4..16 {
            assert_eq!(output[i], 0.0, "output[{i}] should be silence");
        }

        // Source should have been marked finished and cleaned up.
        assert_eq!(mixer.active_sources.read().len(), 0);
    }

    #[test]
    fn test_process_into_output_multiple_sources() {
        // Two sources mixed together through process_into_output.
        let mixer = AudioMixer::new(2, 44100);

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
            channel_mappings: Vec::new(),
            cached_source_channel_count: 2,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            start_at_sample: None,
            cancel_at_sample: None,
        };

        mixer.add_source(active_source1);
        mixer.add_source(active_source2);

        let mut output = vec![0.0f32; 2]; // 1 frame * 2 channels
        mixer.process_into_output(&mut output, 1);

        assert!((output[0] - 0.7).abs() < 1e-6); // 0.5 + 0.2
        assert!((output[1] - 0.4).abs() < 1e-6); // 0.3 + 0.1
    }

    #[test]
    fn test_process_into_output_start_and_cancel_mid_buffer() {
        // Source starts and stops within the same buffer.
        let mixer = AudioMixer::new(2, 44100);

        let samples = vec![0.1, 0.2, 0.3, 0.4, 0.5, 0.6, 0.7, 0.8];
        let source = create_test_source(samples, 1, vec![vec!["test".to_string()]]);

        let cancel_at = Arc::new(AtomicU64::new(6)); // Cancel at sample 6
        let active_source = ActiveSource {
            id: 1,
            source,
            track_mappings: {
                let mut map = HashMap::new();
                map.insert("test".to_string(), vec![1]);
                map
            },
            channel_mappings: Vec::new(),
            cached_source_channel_count: 1,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            start_at_sample: Some(2), // Start at sample 2
            cancel_at_sample: Some(cancel_at),
        };

        mixer.add_source(active_source);

        // Buffer covers samples 0..8. Source plays frames 2..6.
        let mut output = vec![0.0f32; 8 * 2];
        mixer.process_into_output(&mut output, 8);

        // Frames 0-1: silence (before start)
        assert_eq!(output[0], 0.0);
        assert_eq!(output[2], 0.0);
        // Frames 2-5: audio (4 frames from the source)
        assert_eq!(output[4], 0.1); // Frame 2: first source frame
        assert_eq!(output[6], 0.2); // Frame 3
        assert_eq!(output[8], 0.3); // Frame 4
        assert_eq!(output[10], 0.4); // Frame 5
                                     // Frames 6-7: silence (after cancel)
        assert_eq!(output[12], 0.0);
        assert_eq!(output[14], 0.0);
    }

    /// A source that returns 0 frames on the first N calls to read_frames
    /// (simulating buffer underruns), then yields real data. is_exhausted()
    /// returns Some(false) until truly done — exactly like BufferedSampleSource.
    struct UnderrunSimSource {
        inner: Box<dyn ChannelMappedSampleSource>,
        underrun_calls_remaining: usize,
        exhausted: AtomicBool,
    }

    impl UnderrunSimSource {
        fn new(inner: Box<dyn ChannelMappedSampleSource>, underrun_calls: usize) -> Self {
            Self {
                inner,
                underrun_calls_remaining: underrun_calls,
                exhausted: AtomicBool::new(false),
            }
        }
    }

    impl ChannelMappedSampleSource for UnderrunSimSource {
        fn next_sample(
            &mut self,
        ) -> Result<Option<f32>, crate::audio::sample_source::error::SampleSourceError> {
            self.inner.next_sample()
        }

        fn next_frame(
            &mut self,
            output: &mut [f32],
        ) -> Result<Option<usize>, crate::audio::sample_source::error::SampleSourceError> {
            self.inner.next_frame(output)
        }

        fn read_frames(
            &mut self,
            output: &mut [f32],
            max_frames: usize,
        ) -> Result<usize, crate::audio::sample_source::error::SampleSourceError> {
            if self.underrun_calls_remaining > 0 {
                self.underrun_calls_remaining -= 1;
                return Ok(0); // Simulate empty buffer
            }
            let got = self.inner.read_frames(output, max_frames)?;
            if got < max_frames {
                self.exhausted.store(true, Ordering::Relaxed);
            }
            Ok(got)
        }

        fn channel_mappings(&self) -> &Vec<Vec<String>> {
            self.inner.channel_mappings()
        }

        fn source_channel_count(&self) -> u16 {
            self.inner.source_channel_count()
        }

        fn is_exhausted(&self) -> Option<bool> {
            Some(self.exhausted.load(Ordering::Relaxed))
        }
    }

    #[test]
    fn test_mixer_keeps_source_alive_during_transient_underrun() {
        // Regression: before the livelock fix, a short read from a buffered
        // source was unconditionally treated as EOF, causing the mixer to
        // discard the source even though it still had data.
        let mixer = AudioMixer::new(2, 44100);

        let samples = vec![0.5, 0.8, 0.3, 0.6]; // 4 frames of 1 channel
        let inner = create_test_source(samples, 1, vec![vec!["test".to_string()]]);

        // First call to read_frames returns 0 (underrun), subsequent calls work.
        let source: Box<dyn ChannelMappedSampleSource + Send + Sync> =
            Box::new(UnderrunSimSource::new(inner, 1));

        let is_finished = Arc::new(AtomicBool::new(false));
        let active_source = ActiveSource {
            id: 1,
            source,
            track_mappings: {
                let mut map = HashMap::new();
                map.insert("test".to_string(), vec![1]);
                map
            },
            channel_mappings: Vec::new(),
            cached_source_channel_count: 1,
            is_finished: is_finished.clone(),
            cancel_handle: CancelHandle::new(),
            start_at_sample: None,
            cancel_at_sample: None,
        };

        mixer.add_source(active_source);

        // First callback: underrun returns 0 frames → silence.
        // Source must NOT be removed.
        let mut output = vec![0.0f32; 4 * 2];
        mixer.process_into_output(&mut output, 4);

        for &sample in &output {
            assert_eq!(sample, 0.0, "underrun callback should produce silence");
        }
        assert!(
            !is_finished.load(Ordering::Relaxed),
            "source must not be marked finished on transient underrun"
        );
        assert_eq!(
            mixer.active_sources.read().len(),
            1,
            "source must remain active"
        );

        // Second callback: data flows normally.
        let mut output2 = vec![0.0f32; 4 * 2];
        mixer.process_into_output(&mut output2, 4);

        assert_eq!(output2[0], 0.5); // Frame 0, Ch 0
        assert_eq!(output2[2], 0.8); // Frame 1, Ch 0
        assert_eq!(output2[4], 0.3); // Frame 2, Ch 0
        assert_eq!(output2[6], 0.6); // Frame 3, Ch 0
    }

    #[test]
    fn test_mixer_finishes_source_when_truly_exhausted() {
        // Complement to the underrun test: once is_exhausted() returns
        // Some(true), the mixer must still clean up the source.
        let mixer = AudioMixer::new(2, 44100);

        let samples = vec![0.5, 0.8]; // Only 2 frames
        let inner = create_test_source(samples, 1, vec![vec!["test".to_string()]]);

        // No underruns — source runs out immediately.
        let source: Box<dyn ChannelMappedSampleSource + Send + Sync> =
            Box::new(UnderrunSimSource::new(inner, 0));

        let active_source = ActiveSource {
            id: 1,
            source,
            track_mappings: {
                let mut map = HashMap::new();
                map.insert("test".to_string(), vec![1]);
                map
            },
            channel_mappings: Vec::new(),
            cached_source_channel_count: 1,
            is_finished: Arc::new(AtomicBool::new(false)),
            cancel_handle: CancelHandle::new(),
            start_at_sample: None,
            cancel_at_sample: None,
        };

        mixer.add_source(active_source);

        // Request 8 frames but source only has 2 and is_exhausted() → Some(true).
        let mut output = vec![0.0f32; 8 * 2];
        mixer.process_into_output(&mut output, 8);

        assert_eq!(output[0], 0.5);
        assert_eq!(output[2], 0.8);
        for i in 4..16 {
            assert_eq!(output[i], 0.0, "output[{i}] should be silence");
        }
        assert_eq!(
            mixer.active_sources.read().len(),
            0,
            "exhausted source must be removed"
        );
    }

    mod precompute_channel_mappings_tests {
        use super::*;

        #[test]
        fn single_channel_single_output() {
            let source = create_test_source(vec![0.0], 1, vec![vec!["vocals".to_string()]]);
            let mut track_mappings = HashMap::new();
            track_mappings.insert("vocals".to_string(), vec![1]);

            let mappings =
                AudioMixer::precompute_channel_mappings(source.as_ref(), &track_mappings);
            assert_eq!(mappings.len(), 1);
            assert_eq!(mappings[0], vec![0]); // channel 1 → index 0
        }

        #[test]
        fn one_source_to_multiple_outputs() {
            let source = create_test_source(vec![0.0], 1, vec![vec!["mono".to_string()]]);
            let mut track_mappings = HashMap::new();
            // Map "mono" to output channels 1 and 2
            track_mappings.insert("mono".to_string(), vec![1, 2]);

            let mappings =
                AudioMixer::precompute_channel_mappings(source.as_ref(), &track_mappings);
            assert_eq!(mappings.len(), 1);
            assert_eq!(mappings[0], vec![0, 1]); // channels 1,2 → indices 0,1
        }

        #[test]
        fn unmapped_label_produces_empty_outputs() {
            let source = create_test_source(vec![0.0], 1, vec![vec!["not_in_config".to_string()]]);
            let track_mappings = HashMap::new(); // empty — no labels match

            let mappings =
                AudioMixer::precompute_channel_mappings(source.as_ref(), &track_mappings);
            assert_eq!(mappings.len(), 1);
            assert!(mappings[0].is_empty());
        }

        #[test]
        fn multi_channel_source() {
            let source = create_test_source(
                vec![0.0; 2],
                2,
                vec![vec!["left".to_string()], vec!["right".to_string()]],
            );
            let mut track_mappings = HashMap::new();
            track_mappings.insert("left".to_string(), vec![1]);
            track_mappings.insert("right".to_string(), vec![2]);

            let mappings =
                AudioMixer::precompute_channel_mappings(source.as_ref(), &track_mappings);
            assert_eq!(mappings.len(), 2);
            assert_eq!(mappings[0], vec![0]); // left → index 0
            assert_eq!(mappings[1], vec![1]); // right → index 1
        }

        #[test]
        fn source_channel_with_multiple_labels() {
            // A source channel labelled with two labels, both mapped
            let source = create_test_source(
                vec![0.0],
                1,
                vec![vec!["main".to_string(), "monitor".to_string()]],
            );
            let mut track_mappings = HashMap::new();
            track_mappings.insert("main".to_string(), vec![1]);
            track_mappings.insert("monitor".to_string(), vec![3]);

            let mappings =
                AudioMixer::precompute_channel_mappings(source.as_ref(), &track_mappings);
            assert_eq!(mappings.len(), 1);
            assert_eq!(mappings[0], vec![0, 2]); // ch1→0, ch3→2
        }

        #[test]
        fn no_labels_on_source_channel() {
            // Source has an empty label vector for a channel
            let source = create_test_source(vec![0.0], 1, vec![vec![]]);
            let mut track_mappings = HashMap::new();
            track_mappings.insert("anything".to_string(), vec![1]);

            let mappings =
                AudioMixer::precompute_channel_mappings(source.as_ref(), &track_mappings);
            assert_eq!(mappings.len(), 1);
            assert!(mappings[0].is_empty());
        }
    }

    mod remove_sources_tests {
        use super::*;

        #[test]
        fn remove_single_source() {
            let mixer = AudioMixer::new(2, 44100);
            let source = create_test_source(vec![0.0; 100], 1, vec![vec!["t".to_string()]]);
            let mut mappings = HashMap::new();
            mappings.insert("t".to_string(), vec![1]);
            mixer.add_source(make_active_source(42, source, mappings));

            assert_eq!(mixer.active_sources.read().len(), 1);
            mixer.remove_sources(&[42]);
            assert_eq!(mixer.active_sources.read().len(), 0);
        }

        #[test]
        fn remove_subset_of_sources() {
            let mixer = AudioMixer::new(2, 44100);
            for id in 1..=3 {
                let source = create_test_source(vec![0.0; 100], 1, vec![vec!["t".to_string()]]);
                let mut mappings = HashMap::new();
                mappings.insert("t".to_string(), vec![1]);
                mixer.add_source(make_active_source(id, source, mappings));
            }

            assert_eq!(mixer.active_sources.read().len(), 3);
            mixer.remove_sources(&[1, 3]);
            let sources = mixer.active_sources.read();
            assert_eq!(sources.len(), 1);
            assert_eq!(sources[0].lock().id, 2);
        }

        #[test]
        fn remove_nonexistent_id_is_noop() {
            let mixer = AudioMixer::new(2, 44100);
            let source = create_test_source(vec![0.0; 100], 1, vec![vec!["t".to_string()]]);
            let mut mappings = HashMap::new();
            mappings.insert("t".to_string(), vec![1]);
            mixer.add_source(make_active_source(1, source, mappings));

            mixer.remove_sources(&[99]);
            assert_eq!(mixer.active_sources.read().len(), 1);
        }

        #[test]
        fn remove_from_empty_mixer() {
            let mixer = AudioMixer::new(2, 44100);
            mixer.remove_sources(&[1, 2, 3]); // should not panic
            assert_eq!(mixer.active_sources.read().len(), 0);
        }
    }

    mod sample_counter_tests {
        use super::*;

        #[test]
        fn starts_at_zero() {
            let mixer = AudioMixer::new(2, 44100);
            assert_eq!(mixer.current_sample(), 0);
        }

        #[test]
        fn advances_by_frame_count() {
            let mixer = AudioMixer::new(2, 44100);
            let mut output = vec![0.0f32; 256 * 2];
            mixer.process_into_output(&mut output, 256);
            assert_eq!(mixer.current_sample(), 256);
        }

        #[test]
        fn accumulates_across_calls() {
            let mixer = AudioMixer::new(2, 44100);
            let mut output = vec![0.0f32; 128 * 2];
            mixer.process_into_output(&mut output, 128);
            mixer.process_into_output(&mut output, 128);
            mixer.process_into_output(&mut output, 128);
            assert_eq!(mixer.current_sample(), 384);
        }
    }

    mod cancel_handle_tests {
        use super::*;

        #[test]
        fn cancel_handle_removes_source() {
            let mixer = AudioMixer::new(2, 44100);
            let samples = vec![0.5; 1000];
            let source = create_test_source(samples, 1, vec![vec!["t".to_string()]]);

            let mut mappings = HashMap::new();
            mappings.insert("t".to_string(), vec![1]);
            let active = make_active_source(1, source, mappings);
            let cancel = active.cancel_handle.clone();
            mixer.add_source(active);

            // First callback: source is alive
            let mut output = vec![0.0f32; 64 * 2];
            mixer.process_into_output(&mut output, 64);
            assert_eq!(mixer.active_sources.read().len(), 1);
            assert!(output[0] > 0.0); // source is producing audio

            // Cancel the source
            cancel.cancel();

            // Next callback: source should be cleaned up
            let mut output2 = vec![0.0f32; 64 * 2];
            mixer.process_into_output(&mut output2, 64);
            assert_eq!(mixer.active_sources.read().len(), 0);
            for &sample in &output2 {
                assert_eq!(sample, 0.0);
            }
        }

        #[test]
        fn already_finished_source_skipped() {
            let mixer = AudioMixer::new(2, 44100);
            let samples = vec![0.5; 1000];
            let source = create_test_source(samples, 1, vec![vec!["t".to_string()]]);

            let mut mappings = HashMap::new();
            mappings.insert("t".to_string(), vec![1]);
            let active = make_active_source(1, source, mappings);
            active.is_finished.store(true, Ordering::Relaxed);
            mixer.add_source(active);

            let mut output = vec![0.0f32; 64 * 2];
            mixer.process_into_output(&mut output, 64);

            // Should be silence and source cleaned up
            for &sample in &output {
                assert_eq!(sample, 0.0);
            }
            assert_eq!(mixer.active_sources.read().len(), 0);
        }
    }

    mod channel_mapping_mixing_tests {
        use super::*;

        #[test]
        fn one_to_many_channel_mapping() {
            // A mono source mapped to both output channels
            let mixer = AudioMixer::new(2, 44100);
            let samples = vec![0.5, 0.8]; // 2 frames, 1 channel
            let source = create_test_source(samples, 1, vec![vec!["mono".to_string()]]);

            let mut mappings = HashMap::new();
            mappings.insert("mono".to_string(), vec![1, 2]); // Map to both outputs
            mixer.add_source(make_active_source(1, source, mappings));

            let mut output = vec![0.0f32; 2 * 2];
            mixer.process_into_output(&mut output, 2);

            assert_eq!(output[0], 0.5); // Frame 0, Ch 0
            assert_eq!(output[1], 0.5); // Frame 0, Ch 1 (same source)
            assert_eq!(output[2], 0.8); // Frame 1, Ch 0
            assert_eq!(output[3], 0.8); // Frame 1, Ch 1
        }

        #[test]
        fn unmapped_source_produces_silence() {
            let mixer = AudioMixer::new(2, 44100);
            let samples = vec![0.5, 0.8];
            let source = create_test_source(samples, 1, vec![vec!["not_configured".to_string()]]);

            let mappings = HashMap::new(); // No mappings at all
            mixer.add_source(make_active_source(1, source, mappings));

            let mut output = vec![0.0f32; 2 * 2];
            mixer.process_into_output(&mut output, 2);

            for &sample in &output {
                assert_eq!(sample, 0.0);
            }
        }

        #[test]
        fn source_with_out_of_range_output_channel() {
            // Track mapping points to channel 5 but mixer only has 2 channels
            let mixer = AudioMixer::new(2, 44100);
            let samples = vec![0.5];
            let source = create_test_source(samples, 1, vec![vec!["test".to_string()]]);

            let mut mappings = HashMap::new();
            mappings.insert("test".to_string(), vec![5]); // Out of range
            mixer.add_source(make_active_source(1, source, mappings));

            let mut output = vec![0.0f32; 1 * 2];
            mixer.process_into_output(&mut output, 1);

            // Should not panic, should produce silence
            for &sample in &output {
                assert_eq!(sample, 0.0);
            }
        }
    }

    mod mixer_properties_tests {
        use super::*;

        #[test]
        fn num_channels_returns_configured_value() {
            assert_eq!(AudioMixer::new(2, 44100).num_channels(), 2);
            assert_eq!(AudioMixer::new(32, 48000).num_channels(), 32);
        }

        #[test]
        fn sample_rate_returns_configured_value() {
            assert_eq!(AudioMixer::new(2, 44100).sample_rate(), 44100);
            assert_eq!(AudioMixer::new(2, 48000).sample_rate(), 48000);
        }

        #[test]
        fn add_source_precomputes_channel_count() {
            let mixer = AudioMixer::new(2, 44100);
            let source = create_test_source(
                vec![0.0; 4],
                2,
                vec![vec!["a".to_string()], vec!["b".to_string()]],
            );
            let active = make_active_source(1, source, HashMap::new());
            assert_eq!(active.cached_source_channel_count, 0); // not yet set
            mixer.add_source(active);

            let sources = mixer.active_sources.read();
            let source = sources[0].lock();
            assert_eq!(source.cached_source_channel_count, 2);
        }

        #[test]
        fn empty_mixer_produces_silence() {
            let mixer = AudioMixer::new(4, 44100);
            let mut output = vec![1.0f32; 256 * 4]; // fill with non-zero
            mixer.process_into_output(&mut output, 256);
            for &sample in &output {
                assert_eq!(sample, 0.0);
            }
        }
    }

    mod process_frame_tests {
        use super::*;

        #[test]
        fn process_frame_source_exhausts() {
            // Source has 2 mono samples. Processing 3 frames should exhaust it
            // and trigger the Ok(None) path + finished source cleanup in process_frame.
            let mixer = AudioMixer::new(2, 44100);
            let source = create_test_source(vec![0.5, 0.8], 1, vec![vec!["t".to_string()]]);
            let mut mappings = HashMap::new();
            mappings.insert("t".to_string(), vec![1]);
            mixer.add_source(make_active_source(1, source, mappings));

            // First 2 frames produce audio
            let f1 = mixer.process_frame();
            assert_eq!(f1[0], 0.5);
            let f2 = mixer.process_frame();
            assert_eq!(f2[0], 0.8);

            // Third frame: source returns None → marked finished
            let f3 = mixer.process_frame();
            assert_eq!(f3[0], 0.0);
            assert_eq!(mixer.active_sources.read().len(), 0);
        }

        #[test]
        fn process_frame_cancelled_source() {
            // A source whose cancel_handle is already cancelled before processing.
            let mixer = AudioMixer::new(2, 44100);
            let source = create_test_source(vec![0.5; 100], 1, vec![vec!["t".to_string()]]);
            let mut mappings = HashMap::new();
            mappings.insert("t".to_string(), vec![1]);
            let active = make_active_source(1, source, mappings);
            let cancel = active.cancel_handle.clone();
            mixer.add_source(active);

            // Cancel before processing
            cancel.cancel();

            let frame = mixer.process_frame();
            assert_eq!(frame[0], 0.0); // Cancelled → silence
            assert_eq!(mixer.active_sources.read().len(), 0);
        }

        #[test]
        fn process_frame_erroring_source() {
            // A source that returns Err on next_frame → triggers the Err path.
            let mixer = AudioMixer::new(2, 44100);

            // ErrorAfterN with 0 remaining errors immediately on next_sample.
            // Wrap it in a ChannelMappedSource.
            let error_source = ErroringSource;
            let source: Box<dyn ChannelMappedSampleSource> =
                Box::new(crate::audio::sample_source::ChannelMappedSource::new(
                    Box::new(error_source),
                    vec![vec!["t".to_string()]],
                    1,
                ));

            let mut mappings = HashMap::new();
            mappings.insert("t".to_string(), vec![1]);
            mixer.add_source(make_active_source(1, source, mappings));

            let frame = mixer.process_frame();
            assert_eq!(frame[0], 0.0); // Error → silence
            assert_eq!(mixer.active_sources.read().len(), 0);
        }

        #[test]
        fn process_into_output_start_at_already_passed() {
            // Source with start_at_sample in the past (already passed).
            let mixer = AudioMixer::new(2, 44100);
            let source = create_test_source(vec![0.5; 20], 1, vec![vec!["t".to_string()]]);
            let mut mappings = HashMap::new();
            mappings.insert("t".to_string(), vec![1]);
            let mut active = make_active_source(1, source, mappings);
            active.start_at_sample = Some(0); // Start at sample 0

            mixer.add_source(active);

            // Advance the counter past the start time
            let mut warmup = vec![0.0f32; 10 * 2];
            mixer.process_into_output(&mut warmup, 10);
            // sample_counter is now 10, start_at is 0 → already passed

            // Next buffer: start_at < current_sample → start_frame = 0
            let mut output = vec![0.0f32; 4 * 2];
            mixer.process_into_output(&mut output, 4);
            // Source should play from the beginning of the buffer
            assert!(
                output[0] > 0.0,
                "source should produce audio when start_at already passed"
            );
        }

        #[test]
        fn process_into_output_erroring_source() {
            // Tests the read_frames error path in process_into_output.
            let mixer = AudioMixer::new(2, 44100);

            let error_source = ErroringSource;
            let source: Box<dyn ChannelMappedSampleSource> =
                Box::new(crate::audio::sample_source::ChannelMappedSource::new(
                    Box::new(error_source),
                    vec![vec!["t".to_string()]],
                    1,
                ));

            let mut mappings = HashMap::new();
            mappings.insert("t".to_string(), vec![1]);
            mixer.add_source(make_active_source(1, source, mappings));

            let mut output = vec![0.0f32; 4 * 2];
            mixer.process_into_output(&mut output, 4);
            // Error → source removed, output is silence
            assert_eq!(mixer.active_sources.read().len(), 0);
        }

        #[test]
        fn process_into_output_cancel_at_beyond_buffer() {
            // cancel_at_sample set beyond the buffer range → source plays full buffer.
            let mixer = AudioMixer::new(2, 44100);
            let source = create_test_source(vec![0.5; 100], 1, vec![vec!["t".to_string()]]);
            let mut mappings = HashMap::new();
            mappings.insert("t".to_string(), vec![1]);
            let mut active = make_active_source(1, source, mappings);
            active.cancel_at_sample = Some(Arc::new(AtomicU64::new(99999)));
            mixer.add_source(active);

            let mut output = vec![0.0f32; 8 * 2];
            mixer.process_into_output(&mut output, 8);
            // All frames should have audio
            for i in 0..8 {
                assert_eq!(output[i * 2], 0.5, "frame {i} should have audio");
            }
        }
    }

    /// A source that always errors on next_sample.
    struct ErroringSource;

    impl crate::audio::sample_source::traits::SampleSource for ErroringSource {
        fn next_sample(
            &mut self,
        ) -> Result<Option<f32>, crate::audio::sample_source::error::SampleSourceError> {
            Err(
                crate::audio::sample_source::error::SampleSourceError::SampleConversionFailed(
                    "test error".into(),
                ),
            )
        }

        fn channel_count(&self) -> u16 {
            1
        }

        fn sample_rate(&self) -> u32 {
            44100
        }

        fn bits_per_sample(&self) -> u16 {
            32
        }

        fn sample_format(&self) -> crate::audio::SampleFormat {
            crate::audio::SampleFormat::Float
        }

        fn duration(&self) -> Option<std::time::Duration> {
            None
        }
    }

    mod process_frame_resize_buffer {
        use super::*;

        #[test]
        fn process_frame_handles_large_source_channel_count() {
            // Source with > 64 channels to exercise the resize path in process_frame
            let mixer = AudioMixer::new(2, 44100);
            let channel_count = 70;
            let mut samples = vec![0.0f32; channel_count];
            samples[0] = 0.5;
            let mut mappings_vec: Vec<Vec<String>> = Vec::new();
            for i in 0..channel_count {
                mappings_vec.push(vec![format!("ch{}", i)]);
            }
            let source = create_test_source(samples, channel_count as u16, mappings_vec);
            let mut track_mappings = HashMap::new();
            track_mappings.insert("ch0".to_string(), vec![1]);
            mixer.add_source(make_active_source(1, source, track_mappings));

            let frame = mixer.process_frame();
            assert_eq!(frame[0], 0.5);
        }
    }

    mod cancel_at_sample_zero_tests {
        use super::*;

        #[test]
        fn cancel_at_sample_zero_means_no_cancel() {
            // cancel_at_sample with value 0 should be treated as "no cancel"
            let mixer = AudioMixer::new(2, 44100);
            let samples = vec![0.5; 100];
            let source = create_test_source(samples, 1, vec![vec!["t".to_string()]]);
            let mut mappings = HashMap::new();
            mappings.insert("t".to_string(), vec![1]);
            let mut active = make_active_source(1, source, mappings);
            active.cancel_at_sample = Some(Arc::new(AtomicU64::new(0)));
            mixer.add_source(active);

            let mut output = vec![0.0f32; 8 * 2];
            mixer.process_into_output(&mut output, 8);
            // cancel_at = 0 means "don't cancel", so source should still play
            assert!(output[0] > 0.0, "source should play when cancel_at is 0");
        }
    }
}
