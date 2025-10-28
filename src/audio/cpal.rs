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
use std::{
    collections::HashMap,
    error::Error,
    fmt,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Barrier,
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

/// Global atomic counter for generating unique source IDs
static SOURCE_ID_COUNTER: AtomicU64 = AtomicU64::new(1);

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

/// Single-thread callback function that handles mixing directly
fn create_single_thread_callback<T: cpal::Sample + cpal::FromSample<f32> + std::fmt::Debug>(
    mixer: AudioMixer,
    source_rx: crossbeam_channel::Receiver<MixerActiveSource>,
    num_channels: u16,
) -> impl FnMut(&mut [T], &cpal::OutputCallbackInfo) + Send + 'static
where
    f32: cpal::FromSample<T>,
{
    move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
        // Clear the output buffer
        for sample in data.iter_mut() {
            *sample = T::from_sample(0.0f32);
        }

        // Process new sources from the channel
        while let Ok(new_source) = source_rx.try_recv() {
            mixer.add_source(new_source);
        }

        // Use the core mixing logic
        let frames = data.len() / num_channels as usize;
        let mixed_frames = mixer.process_frames(frames);

        // Copy the mixed frames to the CPAL buffer
        for (i, &sample) in mixed_frames.iter().enumerate() {
            if i < data.len() {
                data[i] = T::from_sample(sample);
            }
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

        // Wait for the output thread to finish
        if let Some(thread) = self.output_thread.take() {
            let _ = thread.join();
        }
    }
}

impl OutputManager {
    /// Creates a new output manager.
    fn new(num_channels: u16, sample_rate: u32) -> Result<Self, Box<dyn Error>> {
        let (source_tx, source_rx) = crossbeam_channel::unbounded();

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
    fn start_output_thread(
        &mut self,
        device: cpal::Device,
        target_format: TargetFormat,
    ) -> Result<(), Box<dyn Error>> {
        let mixer = self.mixer.clone();
        let source_rx = self.source_rx.clone();
        let num_channels = mixer.num_channels();
        let sample_rate = mixer.sample_rate();

        // Start the output thread - create the stream inside the thread
        let output_thread = thread::spawn(move || {
            // Create the CPAL stream
            // Create the CPAL stream configuration - let CPAL choose appropriate buffer size
            let config = cpal::StreamConfig {
                channels: num_channels,
                sample_rate: cpal::SampleRate(sample_rate),
                buffer_size: cpal::BufferSize::Default, // Let CPAL choose the buffer size
            };

            // Create the output stream based on the target format
            // Map hound::SampleFormat to the appropriate CPAL stream type

            let stream_result = if target_format.sample_format == crate::audio::SampleFormat::Float
            {
                let mut callback = create_single_thread_callback::<f32>(
                    mixer.clone(),
                    source_rx.clone(),
                    num_channels,
                );
                device.build_output_stream(
                    &config,
                    move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
                        callback(data, info);
                    },
                    |err| error!("CPAL output stream error: {}", err),
                    None,
                )
            } else {
                // For integer formats, we need to convert from f32 to the target integer type
                match target_format.bits_per_sample {
                    16 => {
                        let mut callback = create_single_thread_callback::<i16>(
                            mixer.clone(),
                            source_rx.clone(),
                            num_channels,
                        );
                        device.build_output_stream(
                            &config,
                            move |data: &mut [i16], info: &cpal::OutputCallbackInfo| {
                                callback(data, info);
                            },
                            |err| error!("CPAL output stream error: {}", err),
                            None,
                        )
                    }
                    32 => {
                        let mut callback = create_single_thread_callback::<i32>(
                            mixer.clone(),
                            source_rx.clone(),
                            num_channels,
                        );
                        device.build_output_stream(
                            &config,
                            move |data: &mut [i32], info: &cpal::OutputCallbackInfo| {
                                callback(data, info);
                            },
                            |err| error!("CPAL output stream error: {}", err),
                            None,
                        )
                    }
                    _ => {
                        error!("Unsupported bit depth for integer format");
                        return;
                    }
                }
            };

            // Start the stream
            match stream_result {
                Ok(stream) => {
                    if let Err(e) = stream.play() {
                        error!("Failed to start CPAL stream: {}", e);
                        return;
                    }
                    info!("CPAL output stream started successfully");

                    // Keep the stream alive by waiting
                    loop {
                        thread::sleep(Duration::from_millis(100));
                    }
                }
                Err(e) => {
                    error!("Failed to create CPAL stream: {}", e);
                }
            }
        });

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
                        name: device.name()?,
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

                // Start the output thread
                output_manager
                    .start_output_thread(device.device.clone(), device.target_format.clone())?;

                device.output_manager = Arc::new(output_manager);
                device.audio_config = config;

                Ok(device)
            }
            None => Err(format!("no device found with name {}", name).into()),
        }
    }
}

impl AudioDevice for Device {
    /// Play the given song through the audio device.
    fn play(
        &self,
        song: Arc<Song>,
        mappings: &HashMap<String, Vec<u16>>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
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
        spin_sleep::sleep(self.playback_delay);

        // Create channel mapped sources for each track in the song
        let channel_mapped_sources = song.create_channel_mapped_sources(
            mappings,
            self.target_format.clone(),
            self.audio_config.buffer_size(),
            self.audio_config.buffer_threshold(),
        )?;

        // Add all sources to the output manager
        if channel_mapped_sources.is_empty() {
            return Err("No sources found in song".into());
        }

        // Create unique IDs for each source and track them
        let mut source_ids = Vec::new();

        for source in channel_mapped_sources {
            let current_source_id = SOURCE_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
            let active_source = MixerActiveSource {
                id: current_source_id,
                source,
                track_mappings: mappings.clone(), // Clone for each source
                channel_mappings: Vec::new(),     // Will be precomputed in add_source
                cancel_handle: cancel_handle.clone(), // Clone for each source
                is_finished: Arc::new(AtomicBool::new(false)),
            };

            source_ids.push(current_source_id);
            self.output_manager.add_source(active_source)?;
        }

        // Give the mixer a moment to process all the sources before starting monitoring
        thread::sleep(Duration::from_millis(10));

        // Wait for either cancellation or natural completion
        let finished = Arc::new(AtomicBool::new(false));

        // Start a background thread to monitor if all sources have finished
        let finished_monitor = finished.clone();
        let mixer = self.output_manager.mixer.clone();
        thread::spawn(move || {
            // Poll the mixer to see if all sources for this play operation have finished
            loop {
                let active_sources = mixer.get_active_sources();
                let sources = active_sources.read().unwrap();
                let has_active_sources = sources.iter().any(|source| {
                    let source_guard = source.lock().unwrap();
                    source_ids.contains(&source_guard.id)
                });

                if !has_active_sources {
                    // All sources for this play operation have finished
                    finished_monitor.store(true, Ordering::Relaxed);
                    break;
                }

                thread::sleep(Duration::from_millis(10));
            }
        });

        cancel_handle.wait(finished);

        Ok(())
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
