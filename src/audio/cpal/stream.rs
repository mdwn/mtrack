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
use std::error::Error;
use std::time::Instant;

use cpal::traits::{DeviceTrait, StreamTrait};
use tracing::error;

use crate::audio::format::{SampleFormat, TargetFormat};
use crate::audio::mixer::{ActiveSource as MixerActiveSource, AudioMixer};
use crate::thread_priority::{
    callback_thread_priority, env_flag, promote_to_realtime, rt_audio_enabled,
};

use super::profiler::CallbackProfiler;
use super::CondvarNotify;

// ── Output stream abstraction ────────────────────────────────────────

/// A playing audio output stream. Dropping it stops playback.
/// Wraps the backend-specific stream handle so the lifecycle code in
/// `start_output_thread` is backend-agnostic.
pub(crate) trait OutputStream: Send {}

/// Factory that builds output streams for a specific device + format.
/// Implementations own the device handle and format details; the thread
/// only asks "give me a new stream" each time recovery is needed.
pub(crate) trait OutputStreamFactory: Send + 'static {
    /// Build a new output stream that mixes audio from `mixer`, draining
    /// new sources from `source_rx`.  The implementation must wire the
    /// `error_notify` condvar so the lifecycle thread can detect backend
    /// errors and recreate the stream.
    fn build_stream(
        &self,
        mixer: AudioMixer,
        source_rx: crossbeam_channel::Receiver<MixerActiveSource>,
        num_channels: u16,
        error_notify: CondvarNotify,
    ) -> Result<Box<dyn OutputStream>, Box<dyn Error>>;
}

/// Wraps a `cpal::Stream` so it satisfies `OutputStream`.
struct CpalOutputStream {
    _stream: cpal::Stream,
}

impl OutputStream for CpalOutputStream {}

/// Builds CPAL output streams for a given device, format, and buffer config.
pub(super) struct CpalOutputStreamFactory {
    device: cpal::Device,
    target_format: TargetFormat,
    config: cpal::StreamConfig,
    max_samples: usize,
}

impl CpalOutputStreamFactory {
    pub(super) fn new(
        device: cpal::Device,
        target_format: TargetFormat,
        output_buffer_size: Option<u32>,
    ) -> Self {
        let buffer_size = match output_buffer_size {
            Some(size) => cpal::BufferSize::Fixed(size),
            None => cpal::BufferSize::Default,
        };
        // Template config — num_channels is filled in at build_stream time.
        let config = cpal::StreamConfig {
            channels: 0,
            sample_rate: target_format.sample_rate,
            buffer_size,
        };
        let max_samples = output_buffer_size
            .map(|f| f as usize * 64)
            .unwrap_or(4096 * 64);

        Self {
            device,
            target_format,
            config,
            max_samples,
        }
    }
}

impl OutputStreamFactory for CpalOutputStreamFactory {
    fn build_stream(
        &self,
        mixer: AudioMixer,
        source_rx: crossbeam_channel::Receiver<MixerActiveSource>,
        num_channels: u16,
        error_notify: CondvarNotify,
    ) -> Result<Box<dyn OutputStream>, Box<dyn Error>> {
        // Finalize config with actual channel count / sample rate from mixer.
        let config = cpal::StreamConfig {
            channels: num_channels,
            sample_rate: self.target_format.sample_rate,
            buffer_size: self.config.buffer_size,
        };
        let max_samples = self.max_samples.max(num_channels as usize * 4096);

        let stream = if self.target_format.sample_format == SampleFormat::Float {
            let mut callback = create_direct_f32_callback(mixer, source_rx, num_channels);
            let notify = error_notify;
            self.device.build_output_stream(
                &config,
                move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                    callback(data, info);
                },
                move |err: cpal::StreamError| {
                    error!(
                        "CPAL output stream error: {} (will attempt to recover)",
                        err
                    );
                    let (mutex, condvar) = &*notify;
                    let mut guard = mutex.lock();
                    *guard = true;
                    condvar.notify_one();
                },
                None,
            )?
        } else {
            match self.target_format.bits_per_sample {
                16 => {
                    let mut callback = create_direct_int_callback::<i16>(
                        mixer,
                        source_rx,
                        num_channels,
                        max_samples,
                    );
                    let notify = error_notify;
                    self.device.build_output_stream(
                        &config,
                        move |data: &mut [i16], info: &cpal::OutputCallbackInfo| {
                            callback(data, info);
                        },
                        move |err: cpal::StreamError| {
                            error!(
                                "CPAL output stream error: {} (will attempt to recover)",
                                err
                            );
                            let (mutex, condvar) = &*notify;
                            let mut guard = mutex.lock();
                            *guard = true;
                            condvar.notify_one();
                        },
                        None,
                    )?
                }
                32 => {
                    let mut callback = create_direct_int_callback::<i32>(
                        mixer,
                        source_rx,
                        num_channels,
                        max_samples,
                    );
                    let notify = error_notify;
                    self.device.build_output_stream(
                        &config,
                        move |data: &mut [i32], info: &cpal::OutputCallbackInfo| {
                            callback(data, info);
                        },
                        move |err: cpal::StreamError| {
                            error!(
                                "CPAL output stream error: {} (will attempt to recover)",
                                err
                            );
                            let (mutex, condvar) = &*notify;
                            let mut guard = mutex.lock();
                            *guard = true;
                            condvar.notify_one();
                        },
                        None,
                    )?
                }
                bits => {
                    return Err(format!("Unsupported bit depth for integer format: {bits}").into());
                }
            }
        };

        stream.play()?;
        Ok(Box::new(CpalOutputStream { _stream: stream }))
    }
}

/// Drains pending sources from the channel and adds them to the mixer.
pub(super) fn drain_pending_sources(
    mixer: &AudioMixer,
    source_rx: &crossbeam_channel::Receiver<MixerActiveSource>,
) {
    while let Ok(new_source) = source_rx.try_recv() {
        mixer.add_source(new_source);
    }
}

/// Core f32 mixing logic: drains pending sources, mixes into the output buffer, and profiles.
pub(super) fn process_f32_callback(
    data: &mut [f32],
    mixer: &AudioMixer,
    source_rx: &crossbeam_channel::Receiver<MixerActiveSource>,
    num_channels: u16,
    profiler: &mut CallbackProfiler,
) {
    drain_pending_sources(mixer, source_rx);
    let num_frames = data.len() / num_channels as usize;
    let start = profiler.on_cb_start();
    mixer.process_into_output(data, num_frames);
    profiler.on_mix_done(start);
    profiler.maybe_log_float();
}

/// Core integer mixing logic: drains pending sources, mixes into a temp f32 buffer,
/// converts to the target integer type, and profiles. `temp_buffer` must be pre-allocated
/// to the max expected sample count to avoid allocations in the callback.
pub(super) fn process_int_callback<T: cpal::Sample + cpal::FromSample<f32>>(
    data: &mut [T],
    mixer: &AudioMixer,
    source_rx: &crossbeam_channel::Receiver<MixerActiveSource>,
    num_channels: u16,
    temp_buffer: &mut [f32],
    profiler: &mut CallbackProfiler,
) {
    drain_pending_sources(mixer, source_rx);
    // Never allocate in the callback: clamp to pre-allocated size. If the backend
    // ever sends a larger buffer, we mix only the first max_samples and zero the rest.
    let n = std::cmp::min(data.len(), temp_buffer.len());
    let temp_slice = &mut temp_buffer[..n];
    let num_frames = n / num_channels as usize;
    let start = profiler.on_cb_start();
    mixer.process_into_output(temp_slice, num_frames);
    profiler.on_mix_done(start);
    let start_convert = start.map(|_| Instant::now());
    let zero = T::from_sample(0.0);
    for (out, &sample) in data[..n].iter_mut().zip(temp_slice.iter()) {
        *out = T::from_sample(sample);
    }
    if n < data.len() {
        data[n..].fill(zero);
    }
    profiler.on_convert_done(start_convert);
    profiler.maybe_log_int();
}

/// f32 callback: read directly into CPAL buffer (true zero-copy)
/// Direct mixer callback for f32 output - no intermediate ring buffer
fn create_direct_f32_callback(
    mixer: AudioMixer,
    source_rx: crossbeam_channel::Receiver<MixerActiveSource>,
    num_channels: u16,
) -> impl FnMut(&mut [f32], &cpal::OutputCallbackInfo) + Send + 'static {
    let callback_priority = callback_thread_priority();
    let rt_audio = rt_audio_enabled();
    let profile_audio = env_flag("MTRACK_PROFILE_AUDIO");
    let mut profiler = CallbackProfiler::new(profile_audio);
    let mut priority_set = false;

    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        promote_to_realtime(callback_priority, rt_audio, &mut priority_set);
        process_f32_callback(data, &mixer, &source_rx, num_channels, &mut profiler);
    }
}

/// Direct mixer callback for integer output - no intermediate ring buffer.
/// `max_samples` should be the stream period size in samples (e.g. buffer_size * num_channels)
/// so the temp buffer is pre-allocated and never resized in the callback.
fn create_direct_int_callback<T: cpal::Sample + cpal::FromSample<f32> + std::fmt::Debug>(
    mixer: AudioMixer,
    source_rx: crossbeam_channel::Receiver<MixerActiveSource>,
    num_channels: u16,
    max_samples: usize,
) -> impl FnMut(&mut [T], &cpal::OutputCallbackInfo) + Send + 'static
where
    f32: cpal::FromSample<T>,
{
    let mut temp_buffer = vec![0.0f32; max_samples];
    let callback_priority = callback_thread_priority();
    let rt_audio = rt_audio_enabled();
    let profile_audio = env_flag("MTRACK_PROFILE_AUDIO");
    let mut profiler = CallbackProfiler::new(profile_audio);
    let mut priority_set = false;

    move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
        promote_to_realtime(callback_priority, rt_audio, &mut priority_set);
        process_int_callback(
            data,
            &mixer,
            &source_rx,
            num_channels,
            &mut temp_buffer,
            &mut profiler,
        );
    }
}

#[cfg(test)]
pub(super) mod test {
    use super::super::CondvarNotify;
    use super::*;
    use crate::audio::mixer::AudioMixer;
    use crate::playsync::CancelHandle;
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

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

    /// A mock output stream that stays alive until dropped.
    struct MockOutputStream {
        _alive: Arc<AtomicBool>,
    }

    impl OutputStream for MockOutputStream {}

    impl Drop for MockOutputStream {
        fn drop(&mut self) {
            self._alive.store(false, Ordering::Relaxed);
        }
    }

    /// A mock factory that succeeds, creating a MockOutputStream.
    pub(in crate::audio::cpal) struct MockOutputStreamFactory {
        alive: Arc<AtomicBool>,
    }

    impl MockOutputStreamFactory {
        pub(in crate::audio::cpal) fn new() -> (Self, Arc<AtomicBool>) {
            let alive = Arc::new(AtomicBool::new(false));
            (
                Self {
                    alive: alive.clone(),
                },
                alive,
            )
        }
    }

    impl OutputStreamFactory for MockOutputStreamFactory {
        fn build_stream(
            &self,
            _mixer: AudioMixer,
            _source_rx: crossbeam_channel::Receiver<MixerActiveSource>,
            _num_channels: u16,
            _error_notify: CondvarNotify,
        ) -> Result<Box<dyn OutputStream>, Box<dyn Error>> {
            self.alive.store(true, Ordering::Relaxed);
            Ok(Box::new(MockOutputStream {
                _alive: self.alive.clone(),
            }))
        }
    }

    /// A factory that always fails to build a stream.
    pub(in crate::audio::cpal) struct FailingOutputStreamFactory;

    impl OutputStreamFactory for FailingOutputStreamFactory {
        fn build_stream(
            &self,
            _mixer: AudioMixer,
            _source_rx: crossbeam_channel::Receiver<MixerActiveSource>,
            _num_channels: u16,
            _error_notify: CondvarNotify,
        ) -> Result<Box<dyn OutputStream>, Box<dyn Error>> {
            Err("mock build failure".into())
        }
    }

    /// Shared state for ErrorCapturingFactory so tests can trigger error recovery
    /// after the factory has been moved into the output thread.
    struct ErrorCapturingState {
        alive: Arc<AtomicBool>,
        build_count: std::sync::atomic::AtomicU32,
        captured_error_notify: parking_lot::Mutex<Option<CondvarNotify>>,
    }

    /// A factory that captures the error_notify so tests can trigger stream error recovery.
    pub(in crate::audio::cpal) struct ErrorCapturingFactory {
        state: Arc<ErrorCapturingState>,
    }

    /// Handle returned to test code for inspecting and controlling the factory.
    pub(in crate::audio::cpal) struct ErrorCapturingHandle {
        state: Arc<ErrorCapturingState>,
    }

    impl ErrorCapturingHandle {
        pub(in crate::audio::cpal) fn trigger_error(&self) {
            if let Some(notify) = self.state.captured_error_notify.lock().as_ref() {
                let (mutex, condvar) = &**notify;
                let mut guard = mutex.lock();
                *guard = true;
                condvar.notify_one();
            }
        }

        pub(in crate::audio::cpal) fn build_count(&self) -> u32 {
            self.state.build_count.load(Ordering::Relaxed)
        }

        pub(in crate::audio::cpal) fn is_alive(&self) -> bool {
            self.state.alive.load(Ordering::Relaxed)
        }
    }

    impl ErrorCapturingFactory {
        pub(in crate::audio::cpal) fn new() -> (Self, ErrorCapturingHandle) {
            let state = Arc::new(ErrorCapturingState {
                alive: Arc::new(AtomicBool::new(false)),
                build_count: std::sync::atomic::AtomicU32::new(0),
                captured_error_notify: parking_lot::Mutex::new(None),
            });
            let handle = ErrorCapturingHandle {
                state: state.clone(),
            };
            (Self { state }, handle)
        }
    }

    impl OutputStreamFactory for ErrorCapturingFactory {
        fn build_stream(
            &self,
            _mixer: AudioMixer,
            _source_rx: crossbeam_channel::Receiver<MixerActiveSource>,
            _num_channels: u16,
            error_notify: CondvarNotify,
        ) -> Result<Box<dyn OutputStream>, Box<dyn Error>> {
            self.state.build_count.fetch_add(1, Ordering::Relaxed);
            *self.state.captured_error_notify.lock() = Some(error_notify);
            self.state.alive.store(true, Ordering::Relaxed);
            Ok(Box::new(MockOutputStream {
                _alive: self.state.alive.clone(),
            }))
        }
    }

    mod process_callbacks {
        use super::*;

        fn setup(channels: u16) -> (AudioMixer, crossbeam_channel::Receiver<MixerActiveSource>) {
            let (tx, rx) = crossbeam_channel::bounded(64);
            let mixer = AudioMixer::new(channels, 44100);

            // Pre-load a source with known data via the channel.
            let mut track_mappings = HashMap::new();
            track_mappings.insert("ch0".to_string(), vec![1]);
            if channels > 1 {
                track_mappings.insert("ch1".to_string(), vec![2]);
            }

            let labels: Vec<Vec<String>> =
                (0..channels).map(|i| vec![format!("ch{}", i)]).collect();
            // 4 frames of data per channel.
            let samples: Vec<f32> = (0..4 * channels as usize)
                .map(|i| (i + 1) as f32 * 0.1)
                .collect();
            let source =
                make_active_source(make_test_source(samples, channels, labels), track_mappings);
            tx.send(source).unwrap();

            (mixer, rx)
        }

        #[test]
        fn f32_callback_mixes_into_buffer() {
            let (mixer, rx) = setup(2);
            let mut profiler = CallbackProfiler::new(false);
            let mut output = vec![0.0f32; 8]; // 4 frames * 2 channels

            process_f32_callback(&mut output, &mixer, &rx, 2, &mut profiler);

            // Source should have been drained from channel and mixed in.
            assert!(rx.try_recv().is_err(), "channel should be empty");
            // At least some non-zero samples should be present.
            assert!(
                output.iter().any(|&s| s != 0.0),
                "output should contain mixed audio"
            );
        }

        #[test]
        fn f32_callback_produces_silence_with_no_sources() {
            let (_tx, rx) = crossbeam_channel::bounded::<MixerActiveSource>(64);
            let mixer = AudioMixer::new(2, 44100);
            let mut profiler = CallbackProfiler::new(false);
            let mut output = vec![1.0f32; 8];

            process_f32_callback(&mut output, &mixer, &rx, 2, &mut profiler);

            assert!(output.iter().all(|&s| s == 0.0), "output should be silence");
        }

        #[test]
        fn int_callback_converts_to_i16() {
            let (mixer, rx) = setup(1);
            let mut profiler = CallbackProfiler::new(false);
            let mut temp_buffer = vec![0.0f32; 4];
            let mut output = vec![0i16; 4];

            process_int_callback(&mut output, &mixer, &rx, 1, &mut temp_buffer, &mut profiler);

            assert!(rx.try_recv().is_err(), "channel should be empty");
            assert!(
                output.iter().any(|&s| s != 0),
                "output should contain converted audio"
            );
        }

        #[test]
        fn int_callback_converts_to_i32() {
            let (mixer, rx) = setup(1);
            let mut profiler = CallbackProfiler::new(false);
            let mut temp_buffer = vec![0.0f32; 4];
            let mut output = vec![0i32; 4];

            process_int_callback(&mut output, &mixer, &rx, 1, &mut temp_buffer, &mut profiler);

            assert!(rx.try_recv().is_err(), "channel should be empty");
            assert!(
                output.iter().any(|&s| s != 0),
                "output should contain converted audio"
            );
        }

        #[test]
        fn int_callback_clamps_to_temp_buffer_size() {
            let (mixer, rx) = setup(1);
            let mut profiler = CallbackProfiler::new(false);
            // temp_buffer smaller than output — extra samples should be zeroed.
            let mut temp_buffer = vec![0.0f32; 2];
            let mut output = vec![99i16; 4];

            process_int_callback(&mut output, &mixer, &rx, 1, &mut temp_buffer, &mut profiler);

            // The last 2 samples should be zeroed since they exceed the temp buffer.
            assert_eq!(output[2], 0);
            assert_eq!(output[3], 0);
        }

        #[test]
        fn f32_callback_drains_multiple_sources() {
            let (tx, rx) = crossbeam_channel::bounded(64);
            let mixer = AudioMixer::new(1, 44100);

            // Send two sources.
            for _ in 0..2 {
                let mut mappings = HashMap::new();
                mappings.insert("ch0".to_string(), vec![1]);
                let source = make_active_source(
                    make_test_source(vec![0.5; 4], 1, vec![vec!["ch0".to_string()]]),
                    mappings,
                );
                tx.send(source).unwrap();
            }

            let mut profiler = CallbackProfiler::new(false);
            let mut output = vec![0.0f32; 4];

            process_f32_callback(&mut output, &mixer, &rx, 1, &mut profiler);

            assert!(rx.try_recv().is_err(), "both sources should be drained");
            // Two sources each contributing 0.5 should sum to ~1.0.
            assert!(output[0] > 0.5, "output should be sum of both sources");
        }
    }
}
