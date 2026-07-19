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
// on a shared Rayon thread pool into a lock‑free SPSC ring (rtrb) so the
// real‑time audio callback does no decoding/resampling work, never allocates,
// and never acquires a mutex on the buffer read path.
//

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Condvar, Mutex,
};

use rayon::ThreadPoolBuilder;
use rtrb::{Consumer, Producer, RingBuffer};

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

/// Single‑shot signal used at construction to block the caller until either
/// enough samples are buffered for warmup or the source finishes before
/// reaching that threshold. Never touched on the realtime read path.
struct WarmupSignal {
    state: Mutex<bool>,
    condvar: Condvar,
}

impl WarmupSignal {
    fn new() -> Self {
        Self {
            state: Mutex::new(false),
            condvar: Condvar::new(),
        }
    }

    fn signal(&self) {
        let mut done = self.state.lock().unwrap();
        if !*done {
            *done = true;
            self.condvar.notify_all();
        }
    }

    fn wait(&self) {
        let mut done = self.state.lock().unwrap();
        while !*done {
            done = self.condvar.wait(done).unwrap();
        }
    }
}

/// Buffered wrapper for any ChannelMappedSampleSource used for song playback.
/// The audio callback only reads from a lock‑free SPSC ring buffer; all heavy
/// work runs on the BufferFillPool workers.
pub struct BufferedSampleSource {
    /// Decoder mutex. Only contended by the fill task (at most one in flight
    /// per source, enforced by `refill_in_progress`).
    inner: Arc<Mutex<Box<dyn ChannelMappedSampleSource + Send + Sync>>>,
    /// Consumer end of the lock‑free ring. Owned by this struct; the audio
    /// callback reads from it via `read_frames`/`next_frame`.
    consumer: Consumer<f32>,
    /// Producer end of the ring, parked here when no fill task is running.
    /// A fill task `take()`s it on entry and puts it back on exit. The slot
    /// is only locked by non‑realtime threads.
    producer_slot: Arc<Mutex<Option<Producer<f32>>>>,
    pool: Arc<BufferFillPool>,
    channels: u16,
    refill_threshold_samples: usize,
    warmup_min_samples: usize,
    channel_mappings: Vec<Vec<String>>,
    refill_in_progress: Arc<AtomicBool>,
    finished_flag: Arc<AtomicBool>,
    /// Pre‑allocated frame buffer reused by `next_sample` to avoid per-call
    /// allocation.
    frame_buffer: Vec<f32>,
}

// SAFETY: rtrb's `Consumer<f32>` is `Send` but not `Sync` because the ring
// is a single‑consumer structure (its read index is non‑atomic by design).
// `BufferedSampleSource` only touches `self.consumer` through `&mut self`
// methods, never through `&self`, so concurrent access to the consumer is
// impossible: the borrow checker forbids it at the call site, and the mixer
// further wraps the source in a per‑source `Mutex<ActiveSource>` that
// serializes access across threads. All other fields are independently
// `Sync` (Arc<Mutex<_>>, atomics, plain data).
unsafe impl Sync for BufferedSampleSource {}

impl BufferedSampleSource {
    /// Creates a new buffered wrapper around an existing ChannelMappedSampleSource.
    ///
    /// - `device_buffer_frames`: current audio device buffer size in frames.
    /// - Ring capacity is `device_buffer_frames * 4` frames.
    /// - Warmup blocks until at least `device_buffer_frames` frames are ready
    ///   (or the source ends before that).
    pub fn new(
        inner: Box<dyn ChannelMappedSampleSource + Send + Sync>,
        pool: Arc<BufferFillPool>,
        device_buffer_frames: usize,
    ) -> Self {
        let channels = inner.source_channel_count() as usize;
        let capacity_frames = (device_buffer_frames * 4).max(1);
        let warmup_min_frames = device_buffer_frames.max(1);
        let refill_threshold_frames = capacity_frames / 2;

        let capacity_samples = capacity_frames * channels;
        let warmup_min_samples = warmup_min_frames * channels;
        let refill_threshold_samples = refill_threshold_frames * channels;

        let channel_mappings = inner.channel_mappings().to_vec();

        let (producer, consumer) = RingBuffer::<f32>::new(capacity_samples);
        let producer_slot = Arc::new(Mutex::new(Some(producer)));

        let inner = Arc::new(Mutex::new(inner));
        // Start in the "refill in progress" state so the initial fill task is
        // the sole producer; cleared once it exits.
        let refill_in_progress = Arc::new(AtomicBool::new(true));
        let finished_flag = Arc::new(AtomicBool::new(false));

        let warmup = Arc::new(WarmupSignal::new());

        Self::spawn_fill_task(
            pool.clone(),
            inner.clone(),
            producer_slot.clone(),
            refill_in_progress.clone(),
            finished_flag.clone(),
            channels,
            warmup_min_samples,
            Some(warmup.clone()),
        );

        // Block until warmup completes (sufficient samples or source done).
        // Runs on a non‑realtime thread (song setup).
        warmup.wait();

        Self {
            inner,
            consumer,
            producer_slot,
            pool,
            channels: channels as u16,
            refill_threshold_samples,
            warmup_min_samples,
            channel_mappings,
            refill_in_progress,
            finished_flag,
            frame_buffer: vec![0.0f32; channels],
        }
    }

    /// If the ring has fallen below the refill threshold and no fill task is
    /// already running, spawn one. Uses atomics only — no buffer mutex.
    fn spawn_refill_if_needed(&self) {
        if self.finished_flag.load(Ordering::Relaxed) {
            return;
        }
        if self.consumer.slots() > self.refill_threshold_samples {
            return;
        }
        if self
            .refill_in_progress
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            // A fill task is already in flight.
            return;
        }
        Self::spawn_fill_task(
            self.pool.clone(),
            self.inner.clone(),
            self.producer_slot.clone(),
            self.refill_in_progress.clone(),
            self.finished_flag.clone(),
            self.channels as usize,
            self.warmup_min_samples,
            None,
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn spawn_fill_task(
        pool: Arc<BufferFillPool>,
        inner: Arc<Mutex<Box<dyn ChannelMappedSampleSource + Send + Sync>>>,
        producer_slot: Arc<Mutex<Option<Producer<f32>>>>,
        refill_in_progress: Arc<AtomicBool>,
        finished_flag: Arc<AtomicBool>,
        channels: usize,
        warmup_min_samples: usize,
        warmup_signal: Option<Arc<WarmupSignal>>,
    ) {
        pool.spawn(move || {
            let mut producer = match producer_slot.lock().unwrap().take() {
                Some(p) => p,
                None => {
                    // The debounce on `refill_in_progress` should make this
                    // unreachable. Bail safely if we ever hit it.
                    refill_in_progress.store(false, Ordering::Release);
                    if let Some(sig) = warmup_signal {
                        sig.signal();
                    }
                    return;
                }
            };

            let mut local_frame = vec![0.0f32; channels];
            let mut samples_written: usize = 0;
            let mut warmup_signalled = warmup_signal.is_none();

            loop {
                // Need at least one frame's worth of free slots to write.
                if producer.slots() < channels {
                    break;
                }

                // Decode one frame outside the producer's critical path.
                let done = {
                    let mut g = inner.lock().unwrap();
                    match g.next_frame(&mut local_frame[..]) {
                        Ok(Some(_)) => false,
                        Ok(None) => true,
                        Err(e) => {
                            // A decode error permanently ends just this source
                            // while the rest of the mix continues — make sure
                            // that leaves a trace in the logs.
                            tracing::warn!("Audio source ended early due to decode error: {}", e);
                            true
                        }
                    }
                };

                if done {
                    finished_flag.store(true, Ordering::Release);
                    break;
                }

                // Push one frame into the ring. `push` is the simple safe
                // single‑item API; for typical channel counts (1–8) the
                // per‑push atomic overhead is negligible relative to decode
                // work.
                let mut pushed_full_frame = true;
                for &sample in &local_frame[..channels] {
                    if producer.push(sample).is_err() {
                        // Ring filled mid‑frame; should not happen because we
                        // checked slots() >= channels above, but be defensive.
                        pushed_full_frame = false;
                        break;
                    }
                }
                if !pushed_full_frame {
                    break;
                }
                samples_written += channels;

                if !warmup_signalled && samples_written >= warmup_min_samples {
                    if let Some(sig) = warmup_signal.as_ref() {
                        sig.signal();
                    }
                    warmup_signalled = true;
                }
            }

            *producer_slot.lock().unwrap() = Some(producer);
            refill_in_progress.store(false, Ordering::Release);

            // If the source ended before we hit the warmup threshold, unblock
            // the constructor anyway.
            if !warmup_signalled {
                if let Some(sig) = warmup_signal {
                    sig.signal();
                }
            }
        });
    }
}

impl ChannelMappedSampleSource for BufferedSampleSource {
    fn next_sample(&mut self) -> Result<Option<f32>, SampleSourceError> {
        let mut frame = std::mem::take(&mut self.frame_buffer);
        let result = self.next_frame(&mut frame);
        self.frame_buffer = frame;
        match result? {
            Some(count) if count > 0 => Ok(Some(self.frame_buffer[0])),
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

        let frames = self.read_frames(&mut output[..channels], 1)?;
        if frames == 0 {
            if self.finished_flag.load(Ordering::Relaxed) {
                return Ok(None);
            }
            // Transient underrun: emit silence, matching the previous
            // mutex‑guarded implementation's semantics.
            output[..channels].fill(0.0);
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

        let samples_wanted = max_frames * channels;
        let available = self.consumer.slots();
        // Round down to a whole‑frame boundary; the ring is sample‑typed.
        let to_read = (available.min(samples_wanted) / channels) * channels;

        let frames_read = if to_read > 0 {
            match self.consumer.read_chunk(to_read) {
                Ok(chunk) => {
                    let (first, second) = chunk.as_slices();
                    output[..first.len()].copy_from_slice(first);
                    output[first.len()..first.len() + second.len()].copy_from_slice(second);
                    chunk.commit_all();
                    to_read / channels
                }
                Err(_) => 0,
            }
        } else {
            0
        };

        self.spawn_refill_if_needed();

        Ok(frames_read)
    }

    fn channel_mappings(&self) -> &[Vec<String>] {
        &self.channel_mappings
    }

    fn source_channel_count(&self) -> u16 {
        self.channels
    }

    fn is_exhausted(&self) -> Option<bool> {
        // Decoder has produced all samples it ever will. The mixer only
        // consults this on short reads, at which point an empty ring plus a
        // set finished_flag means truly end‑of‑source.
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
