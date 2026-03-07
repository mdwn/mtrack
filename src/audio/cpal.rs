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
    collections::HashMap,
    error::Error,
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tracing::{debug, error, info, span, Level};

use super::thread_priority::{
    callback_thread_priority, configure_audio_thread_priority, env_flag, rt_audio_enabled,
};

struct CallbackProfiler {
    enabled: bool,
    last_log: Instant,
    count: u64,
    sum_mix_us: u128,
    max_mix_us: u64,
    sum_convert_us: u128,
    max_convert_us: u64,
    last_cb: Option<Instant>,
    sum_gap_us: u128,
    gap_count: u64,
    max_gap_us: u64,
}

impl CallbackProfiler {
    fn new(enabled: bool) -> Self {
        Self {
            enabled,
            last_log: Instant::now(),
            count: 0,
            sum_mix_us: 0,
            max_mix_us: 0,
            sum_convert_us: 0,
            max_convert_us: 0,
            last_cb: None,
            sum_gap_us: 0,
            gap_count: 0,
            max_gap_us: 0,
        }
    }

    fn on_cb_start(&mut self) -> Option<Instant> {
        if !self.enabled {
            return None;
        }
        let now = Instant::now();
        if let Some(last) = self.last_cb {
            let gap_us = now.duration_since(last).as_micros() as u64;
            self.sum_gap_us += gap_us as u128;
            self.gap_count += 1;
            if gap_us > self.max_gap_us {
                self.max_gap_us = gap_us;
            }
        }
        self.last_cb = Some(now);
        Some(now)
    }

    fn on_mix_done(&mut self, start: Option<Instant>) {
        if !self.enabled {
            return;
        }
        let start = match start {
            Some(s) => s,
            None => return,
        };
        let mix_us = start.elapsed().as_micros() as u64;
        self.count += 1;
        self.sum_mix_us += mix_us as u128;
        if mix_us > self.max_mix_us {
            self.max_mix_us = mix_us;
        }
    }

    fn on_convert_done(&mut self, start: Option<Instant>) {
        if !self.enabled {
            return;
        }
        let start = match start {
            Some(s) => s,
            None => return,
        };
        let convert_us = start.elapsed().as_micros() as u64;
        self.sum_convert_us += convert_us as u128;
        if convert_us > self.max_convert_us {
            self.max_convert_us = convert_us;
        }
    }

    fn maybe_log_float(&mut self) {
        if !self.should_log() {
            return;
        }
        let mix_avg_us = self.avg(self.sum_mix_us, self.count);
        let cb_avg_gap_us = self.avg(self.sum_gap_us, self.gap_count);
        debug!(
            mix_avg_us,
            mix_max_us = self.max_mix_us,
            cb_avg_gap_us,
            cb_max_gap_us = self.max_gap_us,
            callbacks = self.count,
            "audio profile: mix (float)"
        );
        self.reset();
    }

    fn maybe_log_int(&mut self) {
        if !self.should_log() {
            return;
        }
        let mix_avg_us = self.avg(self.sum_mix_us, self.count);
        let convert_avg_us = self.avg(self.sum_convert_us, self.count);
        let cb_avg_gap_us = self.avg(self.sum_gap_us, self.gap_count);
        debug!(
            mix_avg_us,
            mix_max_us = self.max_mix_us,
            convert_avg_us,
            convert_max_us = self.max_convert_us,
            cb_avg_gap_us,
            cb_max_gap_us = self.max_gap_us,
            callbacks = self.count,
            "audio profile: mix/convert (int)"
        );
        self.reset();
    }

    fn should_log(&self) -> bool {
        self.enabled && self.last_log.elapsed().as_secs_f32() >= 1.0
    }

    fn avg(&self, sum: u128, count: u64) -> u64 {
        if count > 0 {
            (sum / count as u128) as u64
        } else {
            0
        }
    }

    fn reset(&mut self) {
        self.last_log = Instant::now();
        self.count = 0;
        self.sum_mix_us = 0;
        self.max_mix_us = 0;
        self.sum_convert_us = 0;
        self.max_convert_us = 0;
        self.sum_gap_us = 0;
        self.gap_count = 0;
        self.max_gap_us = 0;
    }
}

use crate::audio::mixer::{ActiveSource as MixerActiveSource, AudioMixer};
use crate::{
    audio::{Device as AudioDevice, SampleFormat, TargetFormat},
    config,
    config::StreamBufferSize,
    playsync::CancelHandle,
    songs::Song,
};
use std::sync::Barrier;

/// Maps a TargetFormat to the corresponding cpal::SampleFormat.
fn target_to_cpal_sample_format(format: SampleFormat, bits_per_sample: u16) -> cpal::SampleFormat {
    match (format, bits_per_sample) {
        (SampleFormat::Float, _) => cpal::SampleFormat::F32,
        (SampleFormat::Int, 16) => cpal::SampleFormat::I16,
        (SampleFormat::Int, 32) => cpal::SampleFormat::I32,
        _ => cpal::SampleFormat::I32,
    }
}

/// Returns the minimum supported output buffer size (frames) for the device and format, if known.
fn min_supported_buffer_size(
    device: &cpal::Device,
    target_format: &TargetFormat,
    channels: u16,
) -> Option<u32> {
    use cpal::SupportedBufferSize;
    let rate = target_format.sample_rate;
    let want_cpal_format =
        target_to_cpal_sample_format(target_format.sample_format, target_format.bits_per_sample);
    let configs = device.supported_output_configs().ok()?;
    let mut best_min = None::<u32>;
    for range in configs {
        if range.channels() != channels {
            continue;
        }
        if range.sample_format() != want_cpal_format {
            continue;
        }
        let (min_r, max_r) = (range.min_sample_rate(), range.max_sample_rate());
        if rate < min_r || rate > max_r {
            continue;
        }
        if let SupportedBufferSize::Range { min, max: _ } = range.buffer_size() {
            let m = *min;
            best_min = Some(best_min.map_or(m, |b| b.min(m)));
        }
    }
    best_min
}

/// Validates that the channel mappings don't exceed the device's max channel count.
fn validate_channel_count(
    mappings: &HashMap<String, Vec<u16>>,
    max_channels: u16,
    song_name: &str,
    device_name: &str,
) -> Result<(), Box<dyn Error>> {
    let num_channels = *mappings
        .iter()
        .flat_map(|entry| entry.1)
        .max()
        .ok_or("no max channel found")?;

    if max_channels < num_channels {
        return Err(format!(
            "{} channels requested for song {}, audio device {} only has {}",
            num_channels, song_name, device_name, max_channels
        )
        .into());
    }

    Ok(())
}

/// Resolves the output buffer size for the CPAL stream based on the config setting.
/// Returns `None` for default (let CPAL decide), or `Some(size)` for a fixed frame count.
fn resolve_buffer_size(
    stream_buffer_size: Option<StreamBufferSize>,
    fallback_buffer_size: u32,
    min_supported: Option<u32>,
) -> Option<u32> {
    match stream_buffer_size {
        None => Some(fallback_buffer_size),
        Some(StreamBufferSize::Default) => None,
        Some(StreamBufferSize::Min) => min_supported.or(Some(fallback_buffer_size)),
        Some(StreamBufferSize::Fixed(n)) => Some(n as u32),
    }
}

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
        error_notify: Arc<(Mutex<bool>, Condvar)>,
    ) -> Result<Box<dyn OutputStream>, Box<dyn Error>>;
}

/// Wraps a `cpal::Stream` so it satisfies `OutputStream`.
struct CpalOutputStream {
    _stream: cpal::Stream,
}

impl OutputStream for CpalOutputStream {}

/// Builds CPAL output streams for a given device, format, and buffer config.
struct CpalOutputStreamFactory {
    device: cpal::Device,
    target_format: TargetFormat,
    config: cpal::StreamConfig,
    max_samples: usize,
}

impl CpalOutputStreamFactory {
    fn new(
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
        error_notify: Arc<(Mutex<bool>, Condvar)>,
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

/// A small wrapper around a cpal::Device. Used for storing some extra
/// data that makes multitrack playing more convenient.
pub struct Device {
    /// The name of the device.
    name: String,
    /// Controls how long to wait before playback of an audio file starts.
    playback_delay: Duration,
    /// The maximum number of channels the device supports.
    max_channels: u16,
    /// The host ID of the device.
    host_id: cpal::HostId,
    /// The underlying cpal device.
    device: cpal::Device,
    /// The target format for this device.
    target_format: TargetFormat,
    /// The output stream manager for continuous playback.
    output_manager: Arc<OutputManager>,
    /// Audio configuration for buffering and performance tuning.
    audio_config: config::Audio,
}

/// Manages the continuous output stream and mixing of multiple audio sources.
struct OutputManager {
    /// The core audio mixer
    mixer: AudioMixer,
    /// Channel for receiving new audio sources to play.
    source_tx: crossbeam_channel::Sender<MixerActiveSource>,
    /// Channel receiver for processing new sources.
    source_rx: crossbeam_channel::Receiver<MixerActiveSource>,
    /// Handle to the output thread (keeps it alive).
    output_thread: Option<thread::JoinHandle<()>>,
    /// Shared shutdown signal: set to true and notify condvar to stop the output thread.
    shutdown_notify: Arc<(Mutex<bool>, Condvar)>,
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} (Channels={}) ({})",
            self.name,
            self.max_channels,
            self.host_id.name()
        )
    }
}

/// Drains pending sources from the channel and adds them to the mixer.
fn drain_pending_sources(
    mixer: &AudioMixer,
    source_rx: &crossbeam_channel::Receiver<MixerActiveSource>,
) {
    while let Ok(new_source) = source_rx.try_recv() {
        mixer.add_source(new_source);
    }
}

/// Core f32 mixing logic: drains pending sources, mixes into the output buffer, and profiles.
fn process_f32_callback(
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
fn process_int_callback<T: cpal::Sample + cpal::FromSample<f32>>(
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
        configure_audio_thread_priority(callback_priority, rt_audio, &mut priority_set);
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
        configure_audio_thread_priority(callback_priority, rt_audio, &mut priority_set);
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
    fn new(num_channels: u16, sample_rate: u32) -> Result<Self, Box<dyn Error>> {
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
    fn add_source(&self, source: MixerActiveSource) -> Result<(), Box<dyn Error>> {
        self.source_tx.send(source)?;
        Ok(())
    }

    /// Starts the output thread that creates and manages the audio stream.
    /// Uses direct callback mode — no intermediate ring buffer for lowest latency.
    /// On backend errors (e.g. ALSA POLLERR), the stream is recreated automatically.
    fn start_output_thread(
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

impl Device {
    /// Lists cpal devices and produces the Device trait.
    pub fn list() -> Result<Vec<Box<dyn AudioDevice>>, Box<dyn Error>> {
        Ok(Device::list_cpal_devices()?
            .into_iter()
            .map(|device| {
                let device: Box<dyn AudioDevice> = Box::new(device);
                device
            })
            .collect())
    }

    /// Lists cpal devices.
    fn list_cpal_devices() -> Result<Vec<Device>, Box<dyn Error>> {
        // Suppress noisy output here.
        let _shh_stdout = shh::stdout()?;
        let _shh_stderr = shh::stderr()?;

        let mut devices: Vec<Device> = Vec::new();
        for host_id in cpal::available_hosts() {
            let host_devices = match cpal::host_from_id(host_id)?.devices() {
                Ok(host_devices) => host_devices,
                Err(e) => {
                    error!(
                        err = e.to_string(),
                        host = host_id.name(),
                        "Unable to list devices for host"
                    );
                    continue;
                }
            };

            for device in host_devices {
                let mut max_channels = 0;

                let output_configs = device.supported_output_configs();
                if let Err(_e) = output_configs {
                    continue;
                }

                for output_config in device.supported_output_configs()? {
                    if max_channels < output_config.channels() {
                        max_channels = output_config.channels();
                    }
                }

                if max_channels > 0 {
                    // Create device with default format - will be overridden in get() method
                    let default_format = TargetFormat::new(44100, SampleFormat::Int, 32)?;

                    // Create a temporary output manager for listing
                    let temp_output_manager = Arc::new(OutputManager::new(
                        max_channels,
                        default_format.sample_rate,
                    )?);

                    devices.push(Device {
                        name: device.id()?.to_string(),
                        playback_delay: Duration::ZERO,
                        max_channels,
                        host_id,
                        device,
                        target_format: default_format,
                        output_manager: temp_output_manager,
                        audio_config: config::Audio::new("default"), // Default config for listing
                    })
                }
            }
        }

        devices.sort_by_key(|device| device.name.to_string());
        Ok(devices)
    }

    /// Gets the given cpal device.
    pub fn get(config: config::Audio) -> Result<Device, Box<dyn Error>> {
        let name = config.device();
        debug!(
            device_name = %name,
            device_name_len = name.len(),
            device_name_trimmed = %name.trim(),
            "Searching for audio device"
        );
        let devices = Device::list_cpal_devices()?;
        debug!(
            available_devices = ?devices.iter().map(|d| &d.name).collect::<Vec<_>>(),
            "Available CPAL devices"
        );
        match devices.into_iter().find(|device| {
            let device_trimmed = device.name.trim();
            let name_trimmed = name.trim();
            let matches = device_trimmed == name_trimmed;
            debug!(
                device_name = %device.name,
                device_trimmed = %device_trimmed,
                looking_for = %name_trimmed,
                matches = matches,
                "Comparing device"
            );
            matches
        }) {
            Some(mut device) => {
                device.playback_delay = config.playback_delay()?;

                device.target_format = TargetFormat::new(
                    config.sample_rate(),
                    config.sample_format()?,
                    config.bits_per_sample(),
                )?;

                // Initialize the output manager
                let mut output_manager =
                    OutputManager::new(device.max_channels, device.target_format.sample_rate)?;

                // Resolve stream buffer size for CPAL (default / min / fixed)
                let min_size = min_supported_buffer_size(
                    &device.device,
                    &device.target_format,
                    device.max_channels,
                );
                let output_buffer_size = resolve_buffer_size(
                    config.stream_buffer_size(),
                    config.buffer_size() as u32,
                    min_size,
                );
                if let (Some(StreamBufferSize::Min), Some(s)) =
                    (config.stream_buffer_size(), output_buffer_size)
                {
                    if min_size.is_some() {
                        info!(
                            stream_buffer_size = s,
                            "Using minimum supported stream buffer size (low latency)"
                        );
                    }
                }

                // Start the output thread with resolved buffer size
                let factory = Box::new(CpalOutputStreamFactory::new(
                    device.device.clone(),
                    device.target_format.clone(),
                    output_buffer_size,
                ));
                output_manager.start_output_thread(factory)?;

                device.output_manager = Arc::new(output_manager);
                device.audio_config = config;

                Ok(device)
            }
            None => Err(format!("no device found with name {}", name).into()),
        }
    }
}

impl AudioDevice for Device {
    /// Play the given song through the audio device, starting from a specific time.
    fn play_from(
        &self,
        song: Arc<Song>,
        mappings: &HashMap<String, Vec<u16>>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
        start_time: Duration,
    ) -> Result<(), Box<dyn Error>> {
        let span = span!(Level::INFO, "play song (cpal)");
        let _enter = span.enter();

        let is_transcoded = song.needs_transcoding(&self.target_format);
        info!(
            format = if song.sample_format() == SampleFormat::Float {
                "float"
            } else {
                "int"
            },
            device = self.name,
            song = song.name(),
            duration = song.duration_string(),
            transcoded = is_transcoded,
            "Playing song."
        );

        validate_channel_count(mappings, self.max_channels, song.name(), &self.name)?;

        play_barrier.wait();

        if cancel_handle.is_cancelled() {
            return Ok(());
        }

        spin_sleep::sleep(self.playback_delay);

        // Build playback context (format, buffer size, shared pool) for source creation.
        let buffer_threads = self.audio_config.buffer_threads();
        let buffer_fill_pool =
            match crate::audio::sample_source::BufferFillPool::new(buffer_threads) {
                Ok(pool) => Some(Arc::new(pool)),
                Err(e) => {
                    error!(
                        error = %e,
                        threads = buffer_threads,
                        "Failed to create BufferFillPool, falling back to unbuffered song sources"
                    );
                    None
                }
            };

        let playback_context = crate::audio::PlaybackContext::new(
            self.target_format.clone(),
            self.audio_config.buffer_size(),
            buffer_fill_pool,
            self.audio_config.resampler(),
        );

        // Create channel mapped sources for each track in the song, starting from start_time.
        let channel_mapped_sources =
            song.create_channel_mapped_sources_from(&playback_context, start_time, mappings)?;

        // Add all sources to the output manager
        if channel_mapped_sources.is_empty() {
            return Err("No sources found in song".into());
        }

        // Create sources and track their finish flags (no locks needed for monitoring)
        let mut source_finish_flags = Vec::new();

        for source in channel_mapped_sources.into_iter() {
            let current_source_id = crate::audio::next_source_id();
            let source_channel_count = source.source_channel_count();
            let is_finished = Arc::new(AtomicBool::new(false));
            source_finish_flags.push(is_finished.clone());

            let active_source = MixerActiveSource {
                id: current_source_id,
                source,
                track_mappings: mappings.clone(), // Clone for each source
                channel_mappings: Vec::new(),     // Will be precomputed in add_source
                cached_source_channel_count: source_channel_count,
                cancel_handle: cancel_handle.clone(), // Clone for each source
                is_finished,
                start_at_sample: None,  // Song sources play immediately
                cancel_at_sample: None, // Song sources don't have scheduled cancellation
            };

            self.output_manager.add_source(active_source)?;
        }

        // Wait for either cancellation or natural completion
        let finished = Arc::new(AtomicBool::new(false));

        // Start a background thread to monitor if all sources have finished
        // This is completely lock-free - just checks atomic flags
        let finished_monitor = finished.clone();
        let cancel_handle_for_notify = cancel_handle.clone();
        let num_song_sources = source_finish_flags.len();
        thread::spawn(move || {
            loop {
                // Check if all sources have finished (lock-free)
                let all_finished = source_finish_flags
                    .iter()
                    .all(|flag| flag.load(Ordering::Relaxed));

                if all_finished {
                    tracing::debug!(
                        num_song_sources,
                        "play_from: all song sources finished, notifying"
                    );
                    finished_monitor.store(true, Ordering::Relaxed);
                    cancel_handle_for_notify.notify();
                    break;
                }

                thread::sleep(Duration::from_millis(10));
            }
        });

        cancel_handle.wait(finished);

        Ok(())
    }

    fn mixer(&self) -> Option<Arc<super::mixer::AudioMixer>> {
        Some(Arc::new(self.output_manager.mixer.clone()))
    }

    fn source_sender(&self) -> Option<super::SourceSender> {
        Some(self.output_manager.source_tx.clone())
    }

    #[cfg(test)]
    fn to_mock(&self) -> Result<Arc<super::mock::Device>, Box<dyn Error>> {
        Err("not a mock".into())
    }
}

#[cfg(test)]
mod test {
    use super::*;

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
    struct MockOutputStreamFactory {
        alive: Arc<AtomicBool>,
    }

    impl MockOutputStreamFactory {
        fn new() -> (Self, Arc<AtomicBool>) {
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
            _error_notify: Arc<(Mutex<bool>, Condvar)>,
        ) -> Result<Box<dyn OutputStream>, Box<dyn Error>> {
            self.alive.store(true, Ordering::Relaxed);
            Ok(Box::new(MockOutputStream {
                _alive: self.alive.clone(),
            }))
        }
    }

    /// A factory that always fails to build a stream.
    struct FailingOutputStreamFactory;

    impl OutputStreamFactory for FailingOutputStreamFactory {
        fn build_stream(
            &self,
            _mixer: AudioMixer,
            _source_rx: crossbeam_channel::Receiver<MixerActiveSource>,
            _num_channels: u16,
            _error_notify: Arc<(Mutex<bool>, Condvar)>,
        ) -> Result<Box<dyn OutputStream>, Box<dyn Error>> {
            Err("mock build failure".into())
        }
    }

    /// Shared state for ErrorCapturingFactory so tests can trigger error recovery
    /// after the factory has been moved into the output thread.
    struct ErrorCapturingState {
        alive: Arc<AtomicBool>,
        build_count: std::sync::atomic::AtomicU32,
        captured_error_notify: std::sync::Mutex<Option<Arc<(Mutex<bool>, Condvar)>>>,
    }

    /// A factory that captures the error_notify so tests can trigger stream error recovery.
    struct ErrorCapturingFactory {
        state: Arc<ErrorCapturingState>,
    }

    /// Handle returned to test code for inspecting and controlling the factory.
    struct ErrorCapturingHandle {
        state: Arc<ErrorCapturingState>,
    }

    impl ErrorCapturingHandle {
        fn trigger_error(&self) {
            if let Some(notify) = self.state.captured_error_notify.lock().unwrap().as_ref() {
                let (mutex, condvar) = &**notify;
                let mut guard = mutex.lock();
                *guard = true;
                condvar.notify_one();
            }
        }

        fn build_count(&self) -> u32 {
            self.state.build_count.load(Ordering::Relaxed)
        }

        fn is_alive(&self) -> bool {
            self.state.alive.load(Ordering::Relaxed)
        }
    }

    impl ErrorCapturingFactory {
        fn new() -> (Self, ErrorCapturingHandle) {
            let state = Arc::new(ErrorCapturingState {
                alive: Arc::new(AtomicBool::new(false)),
                build_count: std::sync::atomic::AtomicU32::new(0),
                captured_error_notify: std::sync::Mutex::new(None),
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
            error_notify: Arc<(Mutex<bool>, Condvar)>,
        ) -> Result<Box<dyn OutputStream>, Box<dyn Error>> {
            self.state.build_count.fetch_add(1, Ordering::Relaxed);
            *self.state.captured_error_notify.lock().unwrap() = Some(error_notify);
            self.state.alive.store(true, Ordering::Relaxed);
            Ok(Box::new(MockOutputStream {
                _alive: self.state.alive.clone(),
            }))
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

    mod callback_profiler {
        use super::*;

        #[test]
        fn disabled_profiler_returns_none_on_cb_start() {
            let mut profiler = CallbackProfiler::new(false);
            assert!(profiler.on_cb_start().is_none());
        }

        #[test]
        fn enabled_profiler_returns_some_on_cb_start() {
            let mut profiler = CallbackProfiler::new(true);
            assert!(profiler.on_cb_start().is_some());
        }

        #[test]
        fn on_mix_done_noop_when_disabled() {
            let mut profiler = CallbackProfiler::new(false);
            profiler.on_mix_done(Some(Instant::now()));
            assert_eq!(profiler.count, 0);
            assert_eq!(profiler.sum_mix_us, 0);
        }

        #[test]
        fn on_mix_done_noop_when_start_is_none() {
            let mut profiler = CallbackProfiler::new(true);
            profiler.on_mix_done(None);
            assert_eq!(profiler.count, 0);
        }

        #[test]
        fn on_mix_done_tracks_stats() {
            let mut profiler = CallbackProfiler::new(true);
            let start = Instant::now();
            std::thread::sleep(Duration::from_micros(100));
            profiler.on_mix_done(Some(start));
            assert_eq!(profiler.count, 1);
            assert!(profiler.sum_mix_us > 0);
            assert!(profiler.max_mix_us > 0);
        }

        #[test]
        fn on_convert_done_noop_when_disabled() {
            let mut profiler = CallbackProfiler::new(false);
            profiler.on_convert_done(Some(Instant::now()));
            assert_eq!(profiler.sum_convert_us, 0);
        }

        #[test]
        fn on_convert_done_noop_when_start_is_none() {
            let mut profiler = CallbackProfiler::new(true);
            profiler.on_convert_done(None);
            assert_eq!(profiler.sum_convert_us, 0);
        }

        #[test]
        fn on_convert_done_tracks_stats() {
            let mut profiler = CallbackProfiler::new(true);
            let start = Instant::now();
            std::thread::sleep(Duration::from_micros(100));
            profiler.on_convert_done(Some(start));
            assert!(profiler.sum_convert_us > 0);
            assert!(profiler.max_convert_us > 0);
        }

        #[test]
        fn cb_start_tracks_gap_between_callbacks() {
            let mut profiler = CallbackProfiler::new(true);
            profiler.on_cb_start();
            std::thread::sleep(Duration::from_micros(100));
            profiler.on_cb_start();
            assert_eq!(profiler.gap_count, 1);
            assert!(profiler.sum_gap_us > 0);
            assert!(profiler.max_gap_us > 0);
        }

        #[test]
        fn avg_returns_zero_when_count_is_zero() {
            let profiler = CallbackProfiler::new(false);
            assert_eq!(profiler.avg(1000, 0), 0);
        }

        #[test]
        fn avg_computes_correctly() {
            let profiler = CallbackProfiler::new(false);
            assert_eq!(profiler.avg(300, 3), 100);
            assert_eq!(profiler.avg(10, 3), 3); // integer division
        }

        #[test]
        fn reset_clears_all_stats() {
            let mut profiler = CallbackProfiler::new(true);
            // Accumulate some stats.
            profiler.on_cb_start();
            std::thread::sleep(Duration::from_micros(50));
            let start = profiler.on_cb_start();
            profiler.on_mix_done(start);
            profiler.on_convert_done(Some(Instant::now()));

            profiler.reset();

            assert_eq!(profiler.count, 0);
            assert_eq!(profiler.sum_mix_us, 0);
            assert_eq!(profiler.max_mix_us, 0);
            assert_eq!(profiler.sum_convert_us, 0);
            assert_eq!(profiler.max_convert_us, 0);
            assert_eq!(profiler.sum_gap_us, 0);
            assert_eq!(profiler.gap_count, 0);
            assert_eq!(profiler.max_gap_us, 0);
        }

        #[test]
        fn should_log_returns_false_when_disabled() {
            let profiler = CallbackProfiler::new(false);
            assert!(!profiler.should_log());
        }

        #[test]
        fn should_log_returns_false_when_under_one_second() {
            let profiler = CallbackProfiler::new(true);
            // Just created, well under 1 second.
            assert!(!profiler.should_log());
        }

        #[test]
        fn max_mix_us_tracks_maximum() {
            let mut profiler = CallbackProfiler::new(true);

            // First callback - short sleep.
            let start1 = Instant::now();
            std::thread::sleep(Duration::from_micros(50));
            profiler.on_mix_done(Some(start1));
            let first_max = profiler.max_mix_us;

            // Second callback - longer sleep.
            let start2 = Instant::now();
            std::thread::sleep(Duration::from_millis(1));
            profiler.on_mix_done(Some(start2));

            assert!(profiler.max_mix_us >= first_max);
            assert_eq!(profiler.count, 2);
        }

        #[test]
        fn max_convert_us_tracks_maximum() {
            let mut profiler = CallbackProfiler::new(true);

            let start1 = Instant::now();
            std::thread::sleep(Duration::from_micros(50));
            profiler.on_convert_done(Some(start1));
            let first_max = profiler.max_convert_us;

            let start2 = Instant::now();
            std::thread::sleep(Duration::from_millis(1));
            profiler.on_convert_done(Some(start2));

            assert!(profiler.max_convert_us >= first_max);
        }

        #[test]
        fn max_gap_us_tracks_maximum() {
            let mut profiler = CallbackProfiler::new(true);

            // Three callbacks with increasing gaps.
            profiler.on_cb_start();
            std::thread::sleep(Duration::from_micros(50));
            profiler.on_cb_start();
            let first_max = profiler.max_gap_us;

            std::thread::sleep(Duration::from_millis(1));
            profiler.on_cb_start();

            assert!(profiler.max_gap_us >= first_max);
            assert_eq!(profiler.gap_count, 2);
        }

        #[test]
        fn maybe_log_float_logs_and_resets_after_one_second() {
            let mut profiler = CallbackProfiler::new(true);
            // Accumulate some stats.
            let start = profiler.on_cb_start();
            profiler.on_mix_done(start);

            // Backdate last_log so should_log() returns true.
            profiler.last_log = Instant::now() - Duration::from_secs(2);

            profiler.maybe_log_float();

            // After logging, reset should have zeroed stats.
            assert_eq!(profiler.count, 0);
            assert_eq!(profiler.sum_mix_us, 0);
            assert_eq!(profiler.max_mix_us, 0);
        }

        #[test]
        fn maybe_log_float_noop_when_disabled() {
            let mut profiler = CallbackProfiler::new(false);
            // Manually set stats since on_mix_done is also a noop when disabled.
            profiler.count = 5;
            profiler.sum_mix_us = 100;
            profiler.last_log = Instant::now() - Duration::from_secs(2);

            profiler.maybe_log_float();

            // Stats should not have been reset since logging is disabled.
            assert_eq!(profiler.count, 5);
        }

        #[test]
        fn maybe_log_int_logs_and_resets_after_one_second() {
            let mut profiler = CallbackProfiler::new(true);
            let start = profiler.on_cb_start();
            profiler.on_mix_done(start);
            profiler.on_convert_done(Some(Instant::now()));

            profiler.last_log = Instant::now() - Duration::from_secs(2);

            profiler.maybe_log_int();

            assert_eq!(profiler.count, 0);
            assert_eq!(profiler.sum_mix_us, 0);
            assert_eq!(profiler.sum_convert_us, 0);
        }

        #[test]
        fn maybe_log_int_noop_when_disabled() {
            let mut profiler = CallbackProfiler::new(false);
            profiler.count = 5;
            profiler.sum_convert_us = 100;
            profiler.last_log = Instant::now() - Duration::from_secs(2);

            profiler.maybe_log_int();

            assert_eq!(profiler.count, 5);
        }
    }

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

    mod target_to_cpal_sample_format_tests {
        use super::*;

        #[test]
        fn float_any_bits() {
            assert_eq!(
                target_to_cpal_sample_format(SampleFormat::Float, 32),
                cpal::SampleFormat::F32
            );
            assert_eq!(
                target_to_cpal_sample_format(SampleFormat::Float, 64),
                cpal::SampleFormat::F32
            );
        }

        #[test]
        fn int_16_bit() {
            assert_eq!(
                target_to_cpal_sample_format(SampleFormat::Int, 16),
                cpal::SampleFormat::I16
            );
        }

        #[test]
        fn int_32_bit() {
            assert_eq!(
                target_to_cpal_sample_format(SampleFormat::Int, 32),
                cpal::SampleFormat::I32
            );
        }

        #[test]
        fn int_other_defaults_to_i32() {
            assert_eq!(
                target_to_cpal_sample_format(SampleFormat::Int, 24),
                cpal::SampleFormat::I32
            );
            assert_eq!(
                target_to_cpal_sample_format(SampleFormat::Int, 8),
                cpal::SampleFormat::I32
            );
        }
    }

    mod process_callbacks {
        use super::*;
        use crate::audio::mixer::AudioMixer;

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

    mod resolve_buffer_size_tests {
        use super::*;

        #[test]
        fn none_returns_fallback() {
            assert_eq!(resolve_buffer_size(None, 256, None), Some(256));
            assert_eq!(resolve_buffer_size(None, 512, Some(64)), Some(512));
        }

        #[test]
        fn default_returns_none() {
            assert_eq!(
                resolve_buffer_size(Some(StreamBufferSize::Default), 256, None),
                None
            );
        }

        #[test]
        fn min_returns_min_supported_when_available() {
            assert_eq!(
                resolve_buffer_size(Some(StreamBufferSize::Min), 256, Some(64)),
                Some(64)
            );
        }

        #[test]
        fn min_falls_back_when_no_min_supported() {
            assert_eq!(
                resolve_buffer_size(Some(StreamBufferSize::Min), 256, None),
                Some(256)
            );
        }

        #[test]
        fn fixed_returns_specified_value() {
            assert_eq!(
                resolve_buffer_size(Some(StreamBufferSize::Fixed(128)), 256, Some(64)),
                Some(128)
            );
        }
    }

    mod validate_channel_count_tests {
        use super::*;

        #[test]
        fn passes_when_channels_within_limit() {
            let mut mappings = HashMap::new();
            mappings.insert("track1".to_string(), vec![1, 2]);
            mappings.insert("track2".to_string(), vec![3, 4]);

            let result = validate_channel_count(&mappings, 4, "test_song", "test_device");
            assert!(result.is_ok());
        }

        #[test]
        fn fails_when_channels_exceed_limit() {
            let mut mappings = HashMap::new();
            mappings.insert("track1".to_string(), vec![1, 2, 3, 4]);

            let result = validate_channel_count(&mappings, 2, "test_song", "test_device");
            assert!(result.is_err());
            let err = result.unwrap_err().to_string();
            assert!(err.contains("4"), "error should mention requested channels");
            assert!(err.contains("2"), "error should mention available channels");
            assert!(err.contains("test_song"), "error should mention song name");
            assert!(
                err.contains("test_device"),
                "error should mention device name"
            );
        }

        #[test]
        fn fails_on_empty_mappings() {
            let mappings: HashMap<String, Vec<u16>> = HashMap::new();
            let result = validate_channel_count(&mappings, 8, "song", "device");
            assert!(result.is_err());
        }

        #[test]
        fn uses_max_channel_across_all_tracks() {
            let mut mappings = HashMap::new();
            mappings.insert("track1".to_string(), vec![1]);
            mappings.insert("track2".to_string(), vec![8]);

            // Max channel is 8, device has 8 — should pass.
            assert!(validate_channel_count(&mappings, 8, "s", "d").is_ok());
            // Max channel is 8, device has 7 — should fail.
            assert!(validate_channel_count(&mappings, 7, "s", "d").is_err());
        }
    }
}
