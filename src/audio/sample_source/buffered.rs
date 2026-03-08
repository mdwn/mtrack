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
// Buffered ChannelMappedSampleSource used for song playback. Prefetches audio
// on a shared Rayon thread pool into a ring buffer so the real‑time audio
// callback does no decoding/resampling work and never allocates.
//

use std::cmp;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};

use rayon::ThreadPoolBuilder;

use crate::audio::sample_source::error::SampleSourceError;
use crate::audio::sample_source::traits::ChannelMappedSampleSource;

/// Shared pool used by BufferedSampleSource instances to prefill and refill
/// their internal buffers. Backed by a dedicated Rayon thread pool.
pub struct BufferFillPool {
    pool: rayon::ThreadPool,
}

impl BufferFillPool {
    /// Creates a new pool with the given number of worker threads.
    pub fn new(num_threads: usize) -> Result<Self, String> {
        let threads = num_threads.max(1);
        let pool = ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|i| format!("mtrack-buffer-fill-{i}"))
            .build()
            .map_err(|e| e.to_string())?;
        Ok(Self { pool })
    }

    /// Spawns a one‑shot job on the pool.
    pub fn spawn<F>(&self, job: F)
    where
        F: FnOnce() + Send + 'static,
    {
        self.pool.spawn(job);
    }
}

struct BufferState {
    /// Interleaved frames: [frame0_ch0, frame0_ch1, ..., frameN_chC].
    data: Vec<f32>,
    /// Next frame index to read.
    read_index: usize,
    /// Next frame index to write.
    write_index: usize,
    /// Number of valid frames currently buffered.
    len_frames: usize,
    /// True when the inner source has been fully consumed (EOF or error).
    finished: bool,
    /// True while a refill job is running for this buffer.
    refill_in_progress: bool,
}

struct BufferInner {
    state: Mutex<BufferState>,
    condvar: Condvar,
}

/// Buffered wrapper for any ChannelMappedSampleSource used for song playback.
/// The audio callback only reads from the ring buffer; all heavy work runs on
/// the BufferFillPool workers.
pub struct BufferedSampleSource {
    inner: Arc<Mutex<Box<dyn ChannelMappedSampleSource + Send + Sync>>>,
    buffer: Arc<BufferInner>,
    pool: Arc<BufferFillPool>,
    channels: u16,
    capacity_frames: usize,
    refill_threshold_frames: usize,
    warmup_min_frames: usize,
    channel_mappings: Vec<Vec<String>>,
    finished_flag: Arc<AtomicBool>,
}

impl BufferedSampleSource {
    /// Creates a new buffered wrapper around an existing ChannelMappedSampleSource.
    ///
    /// - `device_buffer_frames`: current audio device buffer size in frames.
    /// - Buffer capacity is 4x `device_buffer_frames`.
    /// - Warmup waits for at least `device_buffer_frames` frames before returning.
    pub fn new(
        inner: Box<dyn ChannelMappedSampleSource + Send + Sync>,
        pool: Arc<BufferFillPool>,
        device_buffer_frames: usize,
    ) -> Self {
        let channels = inner.source_channel_count() as usize;
        let capacity_frames = (device_buffer_frames * 4).max(1);
        let warmup_min_frames = device_buffer_frames.max(1);
        let refill_threshold_frames = capacity_frames / 2;

        let channel_mappings = inner.channel_mappings().clone();

        let buffer_state = BufferState {
            data: vec![0.0; capacity_frames * channels],
            read_index: 0,
            write_index: 0,
            len_frames: 0,
            finished: false,
            refill_in_progress: false,
        };

        let buffer = Arc::new(BufferInner {
            state: Mutex::new(buffer_state),
            condvar: Condvar::new(),
        });

        let inner = Arc::new(Mutex::new(inner));
        let finished_flag = Arc::new(AtomicBool::new(false));

        let this = Self {
            inner: inner.clone(),
            buffer: buffer.clone(),
            pool: pool.clone(),
            channels: channels as u16,
            capacity_frames,
            refill_threshold_frames,
            warmup_min_frames,
            channel_mappings,
            finished_flag: finished_flag.clone(),
        };

        // Kick off initial warmup fill.
        Self::spawn_fill_task(
            pool,
            inner,
            buffer.clone(),
            finished_flag,
            channels,
            capacity_frames,
            capacity_frames,
            warmup_min_frames,
        );

        // Block until at least one device buffer worth of frames is ready or the
        // source finishes/errs. This runs on a non‑realtime thread (song setup).
        {
            let mut state = buffer.state.lock().unwrap();
            while !state.finished && state.len_frames < warmup_min_frames {
                state = buffer.condvar.wait(state).unwrap();
            }
        }

        this
    }

    fn spawn_refill_if_needed(&self) {
        let mut should_spawn = false;
        {
            let mut state = self.buffer.state.lock().unwrap();
            if !state.finished
                && !state.refill_in_progress
                && state.len_frames <= self.refill_threshold_frames
            {
                state.refill_in_progress = true;
                should_spawn = true;
            }
        }

        if should_spawn {
            Self::spawn_fill_task(
                self.pool.clone(),
                self.inner.clone(),
                self.buffer.clone(),
                self.finished_flag.clone(),
                self.channels as usize,
                self.capacity_frames,
                self.capacity_frames,
                self.warmup_min_frames,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_fill_task(
        pool: Arc<BufferFillPool>,
        inner: Arc<Mutex<Box<dyn ChannelMappedSampleSource + Send + Sync>>>,
        buffer: Arc<BufferInner>,
        finished_flag: Arc<AtomicBool>,
        channels: usize,
        capacity_frames: usize,
        max_batch_frames: usize,
        warmup_min_frames: usize,
    ) {
        pool.spawn(move || {
            let mut local_frame = vec![0.0f32; channels];

            loop {
                // Early exit if buffer is full or finished.
                {
                    let state = buffer.state.lock().unwrap();
                    if state.finished || state.len_frames >= capacity_frames {
                        break;
                    }
                }

                // How many frames should we try to fill in this batch?
                let frames_to_fill = {
                    let state = buffer.state.lock().unwrap();
                    let available = capacity_frames.saturating_sub(state.len_frames);
                    if available == 0 {
                        0
                    } else {
                        cmp::min(max_batch_frames, available)
                    }
                };

                if frames_to_fill == 0 {
                    break;
                }

                for _ in 0..frames_to_fill {
                    // Pull next frame from inner source (no locks held on buffer).
                    let done = {
                        let mut inner_guard = inner.lock().unwrap();
                        match inner_guard.next_frame(&mut local_frame[..]) {
                            Ok(Some(_count)) => false,
                            Ok(None) => true,
                            Err(_) => true,
                        }
                    };

                    // Write frame into ring buffer.
                    {
                        let mut state = buffer.state.lock().unwrap();

                        if done {
                            state.finished = true;
                            finished_flag.store(true, Ordering::Relaxed);
                            buffer.condvar.notify_all();
                            break;
                        }

                        if state.len_frames >= capacity_frames {
                            break;
                        }

                        let base = state.write_index * channels;
                        state.data[base..(base + channels)]
                            .copy_from_slice(&local_frame[..channels]);
                        state.write_index = (state.write_index + 1) % capacity_frames;
                        state.len_frames += 1;

                        if state.len_frames >= warmup_min_frames {
                            buffer.condvar.notify_all();
                        }
                    }
                }
            }

            // Clear refill_in_progress flag and notify any waiters.
            let mut state = buffer.state.lock().unwrap();
            state.refill_in_progress = false;
            buffer.condvar.notify_all();
        });
    }
}

impl ChannelMappedSampleSource for BufferedSampleSource {
    fn next_sample(&mut self) -> Result<Option<f32>, SampleSourceError> {
        let channels = self.channels as usize;
        let mut frame = vec![0.0f32; channels];
        match self.next_frame(&mut frame)? {
            Some(count) if count > 0 => Ok(Some(frame[0])),
            _ => Ok(None),
        }
    }

    fn next_frame(&mut self, output: &mut [f32]) -> Result<Option<usize>, SampleSourceError> {
        let channels = self.channels as usize;
        if output.len() < channels {
            return Err(SampleSourceError::SampleConversionFailed(format!(
                "BufferedSampleSource: output buffer too small: need {channels} samples"
            )));
        }

        let mut maybe_spawn_refill = false;

        {
            let mut state = self.buffer.state.lock().unwrap();

            if state.len_frames == 0 {
                if state.finished {
                    return Ok(None);
                }
                // Buffer underrun but source not exhausted — output silence.
                // Never acquire self.inner here; the fill task needs it.
                output[..channels].fill(0.0);
                maybe_spawn_refill = true;
                // Fall through to drop state lock, then spawn refill below.
            } else {
                let base = state.read_index * channels;
                output[..channels].copy_from_slice(&state.data[base..(base + channels)]);

                state.read_index = (state.read_index + 1) % self.capacity_frames;
                state.len_frames -= 1;

                if !state.finished && state.len_frames <= self.refill_threshold_frames {
                    maybe_spawn_refill = true;
                }
            }
        }

        if maybe_spawn_refill {
            self.spawn_refill_if_needed();
        }

        Ok(Some(channels))
    }

    fn read_frames(
        &mut self,
        output: &mut [f32],
        max_frames: usize,
    ) -> Result<usize, SampleSourceError> {
        let channels = self.channels as usize;
        debug_assert!(
            output.len() >= max_frames * channels,
            "read_frames: output buffer too small ({} < {})",
            output.len(),
            max_frames * channels,
        );
        let mut frames_read = 0;
        let mut maybe_spawn_refill = false;

        {
            let mut state = self.buffer.state.lock().unwrap();

            let available = state.len_frames.min(max_frames);
            if available > 0 {
                let read_start = state.read_index;
                let read_end = read_start + available;

                if read_end <= self.capacity_frames {
                    // Contiguous region — single copy
                    let src_start = read_start * channels;
                    let src_end = read_end * channels;
                    output[..available * channels].copy_from_slice(&state.data[src_start..src_end]);
                } else {
                    // Wrap-around — two copies
                    let first_part = self.capacity_frames - read_start;
                    let first_samples = first_part * channels;
                    let src_start = read_start * channels;
                    output[..first_samples]
                        .copy_from_slice(&state.data[src_start..src_start + first_samples]);

                    let second_samples = (available - first_part) * channels;
                    output[first_samples..first_samples + second_samples]
                        .copy_from_slice(&state.data[..second_samples]);
                }

                state.read_index = (read_start + available) % self.capacity_frames;
                state.len_frames -= available;
                frames_read = available;
            }

            if !state.finished && state.len_frames <= self.refill_threshold_frames {
                maybe_spawn_refill = true;
            }
        }

        if maybe_spawn_refill {
            self.spawn_refill_if_needed();
        }

        Ok(frames_read)
    }

    fn channel_mappings(&self) -> &Vec<Vec<String>> {
        &self.channel_mappings
    }

    fn source_channel_count(&self) -> u16 {
        self.channels
    }

    fn is_exhausted(&self) -> Option<bool> {
        Some(self.finished_flag.load(Ordering::Relaxed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::sample_source::traits::ChannelMappedSampleSource;
    use crate::audio::sample_source::ChannelMappedSource;
    use crate::audio::sample_source::MemorySampleSource;

    fn create_test_pool() -> Arc<BufferFillPool> {
        Arc::new(BufferFillPool::new(1).unwrap())
    }

    fn create_test_inner(
        samples: Vec<f32>,
        channels: u16,
        mappings: Vec<Vec<String>>,
    ) -> Box<dyn ChannelMappedSampleSource + Send + Sync> {
        let mem = MemorySampleSource::new(samples, channels, 44100);
        Box::new(ChannelMappedSource::new(Box::new(mem), mappings, channels))
    }

    #[test]
    fn buffer_fill_pool_spawn_runs_job() {
        let pool = BufferFillPool::new(1).unwrap();
        let flag = Arc::new(AtomicBool::new(false));
        let flag_clone = flag.clone();
        pool.spawn(move || {
            flag_clone.store(true, Ordering::Relaxed);
        });
        // Wait for the job to complete
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(flag.load(Ordering::Relaxed));
    }

    #[test]
    fn buffered_source_reads_all_samples() {
        let pool = create_test_pool();
        let inner = create_test_inner(vec![0.5, 0.8, 0.3, 0.6], 1, vec![vec!["test".to_string()]]);

        let mut buffered = BufferedSampleSource::new(inner, pool, 64);
        assert_eq!(buffered.source_channel_count(), 1);

        let mut samples = Vec::new();
        loop {
            match buffered.next_sample() {
                Ok(Some(s)) => samples.push(s),
                Ok(None) => break,
                Err(_) => break,
            }
            if samples.len() > 100 {
                break;
            }
        }
        assert_eq!(samples.len(), 4);
        assert_eq!(samples[0], 0.5);
        assert_eq!(samples[1], 0.8);
        assert_eq!(samples[2], 0.3);
        assert_eq!(samples[3], 0.6);
    }

    #[test]
    fn buffered_source_next_frame() {
        let pool = create_test_pool();
        let inner = create_test_inner(
            vec![0.1, 0.2, 0.3, 0.4],
            2,
            vec![vec!["l".to_string()], vec!["r".to_string()]],
        );

        let mut buffered = BufferedSampleSource::new(inner, pool, 64);
        assert_eq!(buffered.source_channel_count(), 2);

        let mut frame = vec![0.0f32; 2];
        let result = buffered.next_frame(&mut frame);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), Some(2));
        assert_eq!(frame[0], 0.1);
        assert_eq!(frame[1], 0.2);
    }

    #[test]
    fn buffered_source_read_frames() {
        let pool = create_test_pool();
        // 4 frames of mono data
        let inner = create_test_inner(vec![0.1, 0.2, 0.3, 0.4], 1, vec![vec!["test".to_string()]]);

        let mut buffered = BufferedSampleSource::new(inner, pool, 64);
        let mut output = vec![0.0f32; 4];
        let frames_read = buffered.read_frames(&mut output, 4).unwrap();
        assert_eq!(frames_read, 4);
        assert_eq!(output[0], 0.1);
        assert_eq!(output[1], 0.2);
        assert_eq!(output[2], 0.3);
        assert_eq!(output[3], 0.4);
    }

    #[test]
    fn buffered_source_is_exhausted() {
        let pool = create_test_pool();
        let inner = create_test_inner(vec![0.5, 0.8], 1, vec![vec!["test".to_string()]]);

        let mut buffered = BufferedSampleSource::new(inner, pool, 64);

        // Read all samples
        while let Ok(Some(_)) = buffered.next_sample() {}

        // After draining, should report exhausted
        // Need to wait a moment for the fill task to finish
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(buffered.is_exhausted(), Some(true));
    }

    #[test]
    fn buffered_source_empty_inner() {
        let pool = create_test_pool();
        let inner = create_test_inner(vec![], 1, vec![vec!["test".to_string()]]);

        let mut buffered = BufferedSampleSource::new(inner, pool, 64);
        let result = buffered.next_sample();
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn buffered_source_next_frame_too_small_output() {
        let pool = create_test_pool();
        let inner = create_test_inner(
            vec![0.1, 0.2],
            2,
            vec![vec!["l".to_string()], vec!["r".to_string()]],
        );

        let mut buffered = BufferedSampleSource::new(inner, pool, 64);
        let mut frame = vec![0.0f32; 1]; // too small for 2 channels
        let result = buffered.next_frame(&mut frame);
        assert!(result.is_err());
    }

    #[test]
    fn buffered_source_larger_data_read_frames() {
        let pool = create_test_pool();
        let num_samples = 1000;
        let samples: Vec<f32> = (0..num_samples).map(|i| i as f32 / 1000.0).collect();
        let inner = create_test_inner(samples.clone(), 1, vec![vec!["test".to_string()]]);

        let mut buffered = BufferedSampleSource::new(inner, pool, 64);

        // Use read_frames to read data in batches. read_frames returns
        // the actual number of valid frames without producing silence on underrun.
        let mut output = Vec::new();
        let batch_size = 128;
        let mut batch = vec![0.0f32; batch_size];
        loop {
            let frames = buffered.read_frames(&mut batch, batch_size).unwrap();
            if frames == 0 {
                // Wait briefly for the fill task and try once more
                std::thread::sleep(std::time::Duration::from_millis(20));
                let frames = buffered.read_frames(&mut batch, batch_size).unwrap();
                if frames == 0 {
                    break;
                }
                output.extend_from_slice(&batch[..frames]);
            } else {
                output.extend_from_slice(&batch[..frames]);
            }
            if output.len() >= num_samples + 100 {
                break;
            }
        }
        assert_eq!(output.len(), num_samples);
        assert_eq!(output[0], samples[0]);
        assert_eq!(output[num_samples - 1], samples[num_samples - 1]);
    }
}
