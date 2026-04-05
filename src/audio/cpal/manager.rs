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
use parking_lot::{Condvar, Mutex};
use std::{
    error::Error,
    fmt,
    sync::{Arc, Barrier},
    thread,
    time::Duration,
};

use tracing::{error, info};

use crate::audio::mixer::{ActiveSource as MixerActiveSource, AudioMixer};

use super::stream::OutputStreamFactory;
use super::CondvarNotify;

/// Manages the continuous output stream and mixing of multiple audio sources.
pub(super) struct OutputManager {
    /// The core audio mixer
    pub(super) mixer: AudioMixer,
    /// Channel for receiving new audio sources to play.
    pub(super) source_tx: crossbeam_channel::Sender<MixerActiveSource>,
    /// Channel receiver for processing new sources.
    source_rx: crossbeam_channel::Receiver<MixerActiveSource>,
    /// Handle to the output thread (keeps it alive).
    output_thread: Option<thread::JoinHandle<()>>,
    /// Shared shutdown signal: set to true and notify condvar to stop the output thread.
    shutdown_notify: CondvarNotify,
}

impl fmt::Display for OutputManager {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "OutputManager(channels={}, rate={})",
            self.mixer.num_channels(),
            self.mixer.sample_rate()
        )
    }
}

impl Drop for OutputManager {
    fn drop(&mut self) {
        // Stop all active sources when the output manager is dropped
        let active_sources_arc = self.mixer.get_active_sources();
        let active_sources = active_sources_arc.read();
        let source_ids: Vec<u64> = active_sources
            .iter()
            .map(|source| {
                let source_guard = source.lock();
                source_guard.id
            })
            .collect();
        drop(active_sources); // Release the read lock
        if !source_ids.is_empty() {
            self.mixer.remove_sources(&source_ids);
        }

        // Signal the output thread to shut down and wake it from the condvar wait.
        let (mutex, condvar) = &*self.shutdown_notify;
        *mutex.lock() = true;
        condvar.notify_all();

        // Wait for the output thread to finish
        if let Some(thread) = self.output_thread.take() {
            let _ = thread.join();
        }
    }
}

impl OutputManager {
    /// Creates a new output manager.
    pub(super) fn new(num_channels: u16, sample_rate: u32) -> Result<Self, Box<dyn Error>> {
        // Bounded channel with capacity for typical use cases:
        // - Songs with many tracks (8-16)
        // - Rapid sample triggering
        // If full, send blocks (back-pressure) rather than unbounded growth
        let (source_tx, source_rx) = crossbeam_channel::bounded(64);

        let mixer = AudioMixer::new(num_channels, sample_rate);

        let manager = OutputManager {
            mixer,
            source_tx,
            source_rx,
            output_thread: None,
            shutdown_notify: Arc::new((Mutex::new(false), Condvar::new())),
        };

        Ok(manager)
    }

    /// Adds a new audio source to be played.
    pub(super) fn add_source(&self, source: MixerActiveSource) -> Result<(), Box<dyn Error>> {
        self.source_tx.send(source)?;
        Ok(())
    }

    /// Starts the output thread that creates and manages the audio stream.
    /// Uses direct callback mode — no intermediate ring buffer for lowest latency.
    /// On backend errors (e.g. ALSA POLLERR), the stream is recreated automatically.
    pub(super) fn start_output_thread(
        &mut self,
        factory: Box<dyn OutputStreamFactory>,
    ) -> Result<(), Box<dyn Error>> {
        let mixer = self.mixer.clone();
        let source_rx = self.source_rx.clone();
        let num_channels = mixer.num_channels();

        // Notify the output thread when the error callback runs (e.g. ALSA POLLERR).
        // The output thread blocks on the condvar and recreates the stream on notification.
        let stream_error_notify = Arc::new((Mutex::new(false), Condvar::new()));

        // Shared shutdown signal so drop can wake the output thread.
        let shutdown = self.shutdown_notify.clone();

        // Use a barrier to ensure the first stream is created before we return.
        let barrier = Arc::new(Barrier::new(2));
        let barrier_clone = barrier.clone();

        let output_thread = thread::spawn(move || {
            let mut first_run = true;

            loop {
                let stream_result = factory.build_stream(
                    mixer.clone(),
                    source_rx.clone(),
                    num_channels,
                    stream_error_notify.clone(),
                );

                match stream_result {
                    Ok(stream) => {
                        if first_run {
                            info!(
                                "Audio output stream started successfully (direct callback mode)"
                            );
                            barrier_clone.wait();
                            first_run = false;
                        } else {
                            info!("Audio output stream recovered after backend error");
                        }

                        // Keep the stream alive; block until either:
                        // - the error callback notifies us (recreate stream), or
                        // - the shutdown signal is set (exit thread).
                        let (err_mutex, err_condvar) = &*stream_error_notify;
                        let (shut_mutex, _) = &*shutdown;
                        loop {
                            // Check shutdown first
                            if *shut_mutex.lock() {
                                drop(stream);
                                return;
                            }
                            // Check error
                            let mut err_guard = err_mutex.lock();
                            if *err_guard {
                                *err_guard = false;
                                break;
                            }
                            // Wait on error condvar with a timeout so we can
                            // periodically re-check the shutdown flag.
                            err_condvar.wait_for(&mut err_guard, Duration::from_millis(100));
                        }

                        // Drop the stream so we can create a new one.
                        drop(stream);
                    }
                    Err(e) => {
                        error!("Failed to create audio stream: {}", e);
                        if first_run {
                            barrier_clone.wait();
                        }
                        return;
                    }
                }
            }
        });

        // Wait for first stream to be created.
        barrier.wait();

        self.output_thread = Some(output_thread);
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use super::super::stream::test::{
        ErrorCapturingFactory, FailingOutputStreamFactory, MockOutputStreamFactory,
    };
    use super::*;
    use crate::audio::mixer::ActiveSource as MixerActiveSource;
    use crate::playsync::CancelHandle;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};

    use super::super::stream::drain_pending_sources;

    fn make_test_source(
        samples: Vec<f32>,
        channels: u16,
        labels: Vec<Vec<String>>,
    ) -> Box<dyn crate::audio::sample_source::ChannelMappedSampleSource + Send + Sync> {
        let memory_source =
            crate::audio::sample_source::MemorySampleSource::new(samples, channels, 44100);
        Box::new(crate::audio::sample_source::ChannelMappedSource::new(
            Box::new(memory_source),
            labels,
            channels,
        ))
    }

    fn make_silent_source(
        channels: u16,
    ) -> Box<dyn crate::audio::sample_source::ChannelMappedSampleSource + Send + Sync> {
        let labels = (0..channels).map(|i| vec![format!("ch{}", i)]).collect();
        make_test_source(vec![0.0; 64], channels, labels)
    }

    fn make_active_source(
        source: Box<dyn crate::audio::sample_source::ChannelMappedSampleSource + Send + Sync>,
        track_mappings: HashMap<String, Vec<u16>>,
    ) -> MixerActiveSource {
        MixerActiveSource {
            id: crate::audio::next_source_id(),
            cached_source_channel_count: source.source_channel_count(),
            source,
            track_mappings,
            channel_mappings: Vec::new(),
            cancel_handle: CancelHandle::new(),
            is_finished: Arc::new(AtomicBool::new(false)),
            start_at_sample: None,
            cancel_at_sample: None,
            gain_envelope: None,
        }
    }

    mod start_output_thread {
        use super::*;

        #[test]
        fn starts_successfully_with_mock_factory() {
            let (factory, alive) = MockOutputStreamFactory::new();
            let mut manager = OutputManager::new(2, 44100).unwrap();

            manager
                .start_output_thread(Box::new(factory))
                .expect("should start output thread");

            assert!(
                manager.output_thread.is_some(),
                "output thread should be set"
            );
            // Stream should be alive.
            assert!(alive.load(Ordering::Relaxed), "stream should be alive");

            // Dropping the manager should shut down the thread and drop the stream.
            drop(manager);
            // Give the thread a moment to finish.
            thread::sleep(Duration::from_millis(50));
            assert!(
                !alive.load(Ordering::Relaxed),
                "stream should be dropped after shutdown"
            );
        }

        #[test]
        fn handles_build_failure() {
            let mut manager = OutputManager::new(2, 44100).unwrap();

            // Should not panic even though the factory fails.
            let result = manager.start_output_thread(Box::new(FailingOutputStreamFactory));
            assert!(
                result.is_ok(),
                "start_output_thread should return Ok even if build fails"
            );
            // Thread was spawned but exited after failure.
            assert!(manager.output_thread.is_some());
        }

        #[test]
        fn recovers_from_stream_error() {
            let (factory, handle) = ErrorCapturingFactory::new();
            let mut manager = OutputManager::new(2, 44100).unwrap();

            manager
                .start_output_thread(Box::new(factory))
                .expect("should start");

            assert!(handle.is_alive(), "initial stream alive");
            assert_eq!(handle.build_count(), 1, "should have built one stream");

            // Simulate a backend error — the output thread should recreate the stream.
            handle.trigger_error();

            // Give the thread time to drop old stream and build a new one.
            thread::sleep(Duration::from_millis(250));
            assert_eq!(
                handle.build_count(),
                2,
                "should have rebuilt stream after error"
            );
            assert!(handle.is_alive(), "recovered stream should be alive");

            // Clean shutdown.
            drop(manager);
            thread::sleep(Duration::from_millis(50));
            assert!(
                !handle.is_alive(),
                "stream should be dropped after shutdown"
            );
        }

        #[test]
        fn shutdown_stops_thread() {
            let (factory, alive) = MockOutputStreamFactory::new();
            let mut manager = OutputManager::new(2, 44100).unwrap();

            manager.start_output_thread(Box::new(factory)).unwrap();
            assert!(alive.load(Ordering::Relaxed));

            // Signal shutdown via the notify.
            let (mutex, condvar) = &*manager.shutdown_notify;
            *mutex.lock() = true;
            condvar.notify_all();

            // Give the thread time to see the shutdown signal.
            thread::sleep(Duration::from_millis(250));
            assert!(
                !alive.load(Ordering::Relaxed),
                "stream should be dropped after shutdown signal"
            );
        }
    }

    mod output_manager {
        use super::*;

        #[test]
        fn new_creates_manager() {
            let manager = OutputManager::new(2, 44100).expect("should create output manager");
            assert_eq!(manager.mixer.num_channels(), 2);
            assert_eq!(manager.mixer.sample_rate(), 44100);
            assert!(manager.output_thread.is_none());
        }

        #[test]
        fn add_source_sends_through_channel() {
            let manager = OutputManager::new(2, 44100).expect("should create output manager");
            let source = make_active_source(make_silent_source(2), HashMap::new());
            manager.add_source(source).expect("should add source");
            let received = manager.source_rx.try_recv();
            assert!(received.is_ok());
        }

        #[test]
        fn drop_cleans_up_without_panic() {
            let manager = OutputManager::new(4, 48000).expect("should create output manager");
            drop(manager);
        }

        #[test]
        fn drop_with_active_sources_cleans_up() {
            let manager = OutputManager::new(2, 44100).expect("should create");
            let source = make_active_source(make_silent_source(2), HashMap::new());
            manager.add_source(source).expect("should add");
            // Drain the source into the mixer so it's "active"
            drain_pending_sources(&manager.mixer, &manager.source_rx);
            assert_eq!(manager.mixer.get_active_sources().read().len(), 1);
            drop(manager); // Should clean up active sources without panic
        }
    }
}
