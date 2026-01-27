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
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    thread,
    time::Duration,
};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::atomic::AtomicUsize;
use tracing::{error, info, span, Level};

use crate::audio::mixer::{ActiveSource as MixerActiveSource, AudioMixer};
use crate::{
    audio::{Device as AudioDevice, SampleFormat, TargetFormat},
    config,
    playsync::CancelHandle,
    songs::Song,
};
use std::sync::Barrier;

/// Lock-free circular buffer for zero-copy audio streaming
struct CircularBuffer {
    /// Backing buffer
    buffer: Vec<f32>,
    /// Capacity (must be power of 2)
    capacity: usize,
    /// Read position (consumer)
    read_pos: AtomicUsize,
    /// Write position (producer)
    write_pos: AtomicUsize,
}

impl CircularBuffer {
    fn new(capacity: usize) -> Self {
        // Round up to next power of 2 for efficient modulo
        let cap = capacity.next_power_of_two();
        Self {
            buffer: vec![0.0; cap],
            capacity: cap,
            read_pos: AtomicUsize::new(0),
            write_pos: AtomicUsize::new(0),
        }
    }

    /// Get number of samples available to read
    #[inline]
    fn available(&self) -> usize {
        let write = self.write_pos.load(Ordering::Acquire);
        let read = self.read_pos.load(Ordering::Acquire);
        // Both positions are in [0, capacity), so:
        // - If write >= read: available = write - read
        // - If write < read: wrapped, available = capacity - read + write
        if write >= read {
            write - read
        } else {
            self.capacity - read + write
        }
    }

    /// Get space available to write
    #[inline]
    fn space(&self) -> usize {
        self.capacity - self.available() - 1
    }

    /// Write samples directly into buffer (zero-copy)
    /// Returns number of samples actually written
    fn write(&self, samples: &[f32]) -> usize {
        let space = self.space();
        if space == 0 {
            return 0;
        }
        let to_write = space.min(samples.len());
        let write = self.write_pos.load(Ordering::Acquire);
        let mask = self.capacity - 1;

        // Write in one or two chunks (if wrap-around)
        let first_chunk = (self.capacity - write).min(to_write);
        unsafe {
            let ptr = self.buffer.as_ptr().add(write) as *mut f32;
            std::ptr::copy_nonoverlapping(samples.as_ptr(), ptr, first_chunk);
        }

        if to_write > first_chunk {
            let second_chunk = to_write - first_chunk;
            unsafe {
                let ptr = self.buffer.as_ptr() as *mut f32;
                std::ptr::copy_nonoverlapping(samples.as_ptr().add(first_chunk), ptr, second_chunk);
            }
        }

        self.write_pos
            .store((write + to_write) & mask, Ordering::Release);
        to_write
    }

    /// Read samples directly from buffer (zero-copy)
    /// Returns number of samples actually read
    fn read(&self, output: &mut [f32]) -> usize {
        let available = self.available();
        if available == 0 {
            return 0;
        }
        let to_read = available.min(output.len());
        let read = self.read_pos.load(Ordering::Acquire);
        let mask = self.capacity - 1;

        // Read in one or two chunks (if wrap-around)
        let first_chunk = (self.capacity - read).min(to_read);
        unsafe {
            let ptr = self.buffer.as_ptr().add(read);
            std::ptr::copy_nonoverlapping(ptr, output.as_mut_ptr(), first_chunk);
        }

        if to_read > first_chunk {
            let second_chunk = to_read - first_chunk;
            unsafe {
                let ptr = self.buffer.as_ptr();
                std::ptr::copy_nonoverlapping(
                    ptr,
                    output.as_mut_ptr().add(first_chunk),
                    second_chunk,
                );
            }
        }

        self.read_pos
            .store((read + to_read) & mask, Ordering::Release);
        to_read
    }
}

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
    /// Handle to the producer thread (fills ring buffer).
    producer_thread: Option<thread::JoinHandle<()>>,
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
fn create_f32_callback(
    ring: Arc<CircularBuffer>,
) -> impl FnMut(&mut [f32], &cpal::OutputCallbackInfo) + Send + 'static {
    move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
        let read = ring.read(data);
        // Zero-fill any shortfall
        data[read..].fill(0.0);
    }
}

/// Integer callback: read from ring and convert
fn create_single_thread_callback<T: cpal::Sample + cpal::FromSample<f32> + std::fmt::Debug>(
    ring: Arc<CircularBuffer>,
) -> impl FnMut(&mut [T], &cpal::OutputCallbackInfo) + Send + 'static
where
    f32: cpal::FromSample<T>,
{
    move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
        let len = data.len();
        let mut temp = vec![0.0f32; len];
        let read = ring.read(&mut temp);

        // Zero-fill any shortfall
        temp[read..].fill(0.0);

        // Convert to output format
        for (dst, &src) in data.iter_mut().zip(temp.iter()) {
            *dst = T::from_sample(src);
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
        if let Some(thread) = self.producer_thread.take() {
            let _ = thread.join();
        }
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
            producer_thread: None,
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

        // Create shared circular buffer (~100ms of audio)
        let capacity_samples = (sample_rate as usize * num_channels as usize) / 10;
        let ring = Arc::new(CircularBuffer::new(capacity_samples.max(1024)));

        // Producer thread: mix audio and write to ring buffer (zero-allocation)
        let mixer_for_producer = mixer.clone();
        let source_rx_for_producer = source_rx.clone();
        let ring_for_producer = ring.clone();
        let producer_thread = thread::spawn(move || {
            // Pre-allocate scratch buffer (reused for all blocks)
            let block_frames = 512; // Small block size for low latency
            let block_samples = block_frames * num_channels as usize;
            let mut scratch = vec![0.0f32; block_samples];

            loop {
                // Process new sources
                while let Ok(new_source) = source_rx_for_producer.try_recv() {
                    mixer_for_producer.add_source(new_source);
                }

                // Check if ring has space for a block
                if ring_for_producer.space() >= block_samples {
                    // Mix directly into scratch buffer (zero-allocation)
                    mixer_for_producer.process_into_output(&mut scratch, block_frames);
                    ring_for_producer.write(&scratch);
                } else {
                    // Ring full, yield briefly
                    thread::sleep(Duration::from_micros(500));
                }
            }
        });

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
                let mut callback = create_f32_callback(ring.clone());
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
                        let mut callback = create_single_thread_callback::<i16>(ring.clone());
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
                        let mut callback = create_single_thread_callback::<i32>(ring.clone());
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
        self.producer_thread = Some(producer_thread);
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
            self.audio_config.buffer_threshold(),
        )?;

        // Add all sources to the output manager
        if channel_mapped_sources.is_empty() {
            return Err("No sources found in song".into());
        }

        // Create unique IDs for each source and track them
        let mut source_ids = Vec::new();

        for source in channel_mapped_sources.into_iter() {
            let current_source_id = SOURCE_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
            let source_channel_count = source.source_channel_count();
            let active_source = MixerActiveSource {
                id: current_source_id,
                source,
                track_mappings: mappings.clone(), // Clone for each source
                channel_mappings: Vec::new(),     // Will be precomputed in add_source
                cached_source_channel_count: source_channel_count,
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
        let cancel_handle_for_notify = cancel_handle.clone();
        thread::spawn(move || {
            // Poll the mixer to see if all sources for this play operation have finished
            loop {
                let active_sources = mixer.get_active_sources();
                let sources = active_sources.read().unwrap();
                let has_active_sources = sources.iter().any(|source| {
                    let source_guard = source.lock().unwrap();
                    source_ids.contains(&source_guard.id)
                });
                drop(sources);

                if !has_active_sources {
                    // All sources for this play operation have finished
                    finished_monitor.store(true, Ordering::Relaxed);
                    cancel_handle_for_notify.notify(); // Wake up the waiting thread!
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
