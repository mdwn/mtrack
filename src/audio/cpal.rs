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
use std::{
    collections::HashMap,
    error::Error,
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
    time::Duration,
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use tracing::{error, info, span, Level};

use crate::audio::mixer::{ActiveSource as MixerActiveSource, AudioMixer};
use crate::{
    audio::{Device as AudioDevice, SampleFormat, TargetFormat},
    config,
    playsync::CancelHandle,
    songs::Song,
};
use std::sync::Barrier;

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

/// f32 callback: read directly into CPAL buffer (true zero-copy)
/// Direct mixer callback for f32 output - no intermediate ring buffer
fn create_direct_f32_callback(
    mixer: AudioMixer,
    source_rx: crossbeam_channel::Receiver<MixerActiveSource>,
    num_channels: u16,
) -> impl FnMut(&mut [f32], &cpal::OutputCallbackInfo) + Send + 'static {
    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        // Process any pending new sources (non-blocking)
        while let Ok(new_source) = source_rx.try_recv() {
            mixer.add_source(new_source);
        }

        // Mix directly into the output buffer (cleanup happens inline)
        let num_frames = data.len() / num_channels as usize;
        mixer.process_into_output(data, num_frames);
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

    move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
        // Process any pending new sources (non-blocking)
        while let Ok(new_source) = source_rx.try_recv() {
            mixer.add_source(new_source);
        }

        // Pre-allocated for typical period size; resize only if backend gives a larger buffer (rare)
        if temp_buffer.len() < data.len() {
            temp_buffer.resize(data.len(), 0.0);
        }
        let temp_slice = &mut temp_buffer[..data.len()];
        let num_frames = data.len() / num_channels as usize;
        mixer.process_into_output(temp_slice, num_frames);

        // Convert to output format
        for (out, &sample) in data.iter_mut().zip(temp_slice.iter()) {
            *out = T::from_sample(sample);
        }
    }
}

impl Drop for OutputManager {
    fn drop(&mut self) {
        // Stop all active sources when the output manager is dropped
        if let Ok(active_sources) = self.mixer.get_active_sources().read() {
            let source_ids: Vec<u64> = active_sources
                .iter()
                .map(|source| {
                    let source_guard = source.lock().unwrap();
                    source_guard.id
                })
                .collect();
            if !source_ids.is_empty() {
                self.mixer.remove_sources(source_ids);
            }
        }

        // Close the channels to signal the audio callback to stop
        // Note: The channels will be automatically dropped when the struct is dropped

        // Wait for threads to finish
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
        };

        Ok(manager)
    }

    /// Adds a new audio source to be played.
    fn add_source(&self, source: MixerActiveSource) -> Result<(), Box<dyn Error>> {
        self.source_tx.send(source)?;
        Ok(())
    }

    /// Starts the output thread that creates and manages the CPAL stream.
    /// Uses direct callback mode - no intermediate ring buffer for lowest latency.
    /// On ALSA/backend errors (e.g. POLLERR), the stream is recreated automatically.
    fn start_output_thread(
        &mut self,
        device: cpal::Device,
        target_format: TargetFormat,
        output_buffer_size: Option<u32>,
    ) -> Result<(), Box<dyn Error>> {
        let mixer = self.mixer.clone();
        let source_rx = self.source_rx.clone();
        let num_channels = mixer.num_channels();
        let sample_rate = mixer.sample_rate();

        // Notify the output thread when the CPAL error callback runs (e.g. ALSA POLLERR).
        // The output thread blocks on the condvar and recreates the stream on notification.
        let stream_error_notify = Arc::new((Mutex::new(false), Condvar::new()));

        // Use a barrier to ensure the first stream is created before we return
        let barrier = Arc::new(Barrier::new(2));
        let barrier_clone = barrier.clone();

        // Start the output thread - create the stream inside the thread, recreate on error
        let output_thread = thread::spawn(move || {
            let buffer_size = match output_buffer_size {
                Some(size) => cpal::BufferSize::Fixed(size),
                None => cpal::BufferSize::Default,
            };
            let config = cpal::StreamConfig {
                channels: num_channels,
                sample_rate,
                buffer_size,
            };
            let max_samples = output_buffer_size
                .map(|f| f as usize * num_channels as usize)
                .unwrap_or(4096 * num_channels as usize);

            let mut first_run = true;

            loop {
                let notify = stream_error_notify.clone();
                let on_error = move |err: cpal::StreamError| {
                    error!(
                        "CPAL output stream error: {} (will attempt to recover)",
                        err
                    );
                    let (mutex, condvar) = &*notify;
                    let mut guard = mutex.lock().unwrap();
                    *guard = true;
                    condvar.notify_one();
                };

                // Create the output stream with direct mixer callback (no ring buffer)
                let stream_result = if target_format.sample_format
                    == crate::audio::SampleFormat::Float
                {
                    let mut callback =
                        create_direct_f32_callback(mixer.clone(), source_rx.clone(), num_channels);
                    device.build_output_stream(
                        &config,
                        move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                            callback(data, info);
                        },
                        on_error,
                        None,
                    )
                } else {
                    match target_format.bits_per_sample {
                        16 => {
                            let mut callback = create_direct_int_callback::<i16>(
                                mixer.clone(),
                                source_rx.clone(),
                                num_channels,
                                max_samples,
                            );
                            let on_err = stream_error_notify.clone();
                            device.build_output_stream(
                                &config,
                                move |data: &mut [i16], info: &cpal::OutputCallbackInfo| {
                                    callback(data, info);
                                },
                                move |err: cpal::StreamError| {
                                    error!(
                                        "CPAL output stream error: {} (will attempt to recover)",
                                        err
                                    );
                                    let (mutex, condvar) = &*on_err;
                                    let mut guard = mutex.lock().unwrap();
                                    *guard = true;
                                    condvar.notify_one();
                                },
                                None,
                            )
                        }
                        32 => {
                            let mut callback = create_direct_int_callback::<i32>(
                                mixer.clone(),
                                source_rx.clone(),
                                num_channels,
                                max_samples,
                            );
                            let on_err = stream_error_notify.clone();
                            device.build_output_stream(
                                &config,
                                move |data: &mut [i32], info: &cpal::OutputCallbackInfo| {
                                    callback(data, info);
                                },
                                move |err: cpal::StreamError| {
                                    error!(
                                        "CPAL output stream error: {} (will attempt to recover)",
                                        err
                                    );
                                    let (mutex, condvar) = &*on_err;
                                    let mut guard = mutex.lock().unwrap();
                                    *guard = true;
                                    condvar.notify_one();
                                },
                                None,
                            )
                        }
                        _ => {
                            error!("Unsupported bit depth for integer format");
                            if first_run {
                                barrier_clone.wait();
                            }
                            return;
                        }
                    }
                };

                match stream_result {
                    Ok(stream) => {
                        if let Err(e) = stream.play() {
                            error!("Failed to start CPAL stream: {}", e);
                            if first_run {
                                barrier_clone.wait();
                            }
                            return;
                        }
                        if first_run {
                            info!("CPAL output stream started successfully (direct callback mode)");
                            barrier_clone.wait();
                            first_run = false;
                        } else {
                            info!("CPAL output stream recovered after backend error");
                        }

                        // Keep the stream alive; block until the error callback notifies us
                        let (mutex, condvar) = &*stream_error_notify;
                        let mut guard = mutex.lock().unwrap();
                        while !*guard {
                            guard = condvar.wait(guard).unwrap();
                        }
                        *guard = false;
                        drop(guard);

                        // Drop the stream so we can create a new one
                        drop(stream);
                    }
                    Err(e) => {
                        error!("Failed to create CPAL stream: {}", e);
                        if first_run {
                            barrier_clone.wait();
                        }
                        return;
                    }
                }
            }
        });

        // Wait for first stream to be created
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
        match Device::list_cpal_devices()?
            .into_iter()
            .find(|device| device.name.trim() == name)
        {
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

                // Start the output thread with configured buffer size
                output_manager.start_output_thread(
                    device.device.clone(),
                    device.target_format.clone(),
                    Some(config.buffer_size() as u32),
                )?;

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

        let num_channels = *mappings
            .iter()
            .flat_map(|entry| entry.1)
            .max()
            .ok_or("no max channel found")?;

        if self.max_channels < num_channels {
            return Err(format!(
                "{} channels requested for song {}, audio device {} only has {}",
                num_channels,
                song.name(),
                self.name,
                self.max_channels
            )
            .into());
        }

        play_barrier.wait();

        if cancel_handle.is_cancelled() {
            return Ok(());
        }

        spin_sleep::sleep(self.playback_delay);

        // Create channel mapped sources for each track in the song, starting from start_time
        let channel_mapped_sources = song.create_channel_mapped_sources_from(
            start_time,
            mappings,
            self.target_format.clone(),
            self.audio_config.buffer_size(),
        )?;

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
    // Note: Old tests removed - they were testing the obsolete SongSource/IntToFloatIterator architecture
    // The new ChannelMappedSampleSource and AudioMixer architecture is tested in src/audio/mixer.rs
}
