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

use cpal::traits::{DeviceTrait, HostTrait};
use tracing::{debug, error, info, span, Level};

use crate::audio::format::{SampleFormat, TargetFormat};
use crate::audio::mixer::ActiveSource as MixerActiveSource;
use crate::audio::Device as AudioDevice;
use crate::config;
use crate::config::StreamBufferSize;
use crate::playsync::CancelHandle;
use crate::songs::Song;

use super::manager::OutputManager;
use super::stream::CpalOutputStreamFactory;

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

/// A supported sample format for an audio device.
#[derive(serde::Serialize, Clone, PartialEq, Eq, Hash)]
pub struct SupportedFormat {
    pub sample_format: String,
    pub bits_per_sample: u32,
}

/// Standard sample rates to check against device-reported ranges.
const STANDARD_SAMPLE_RATES: &[u32] = &[
    8000, 11025, 16000, 22050, 44100, 48000, 88200, 96000, 176400, 192000,
];

/// Serializable info about an audio device for the web UI.
#[derive(serde::Serialize)]
pub struct AudioDeviceInfo {
    pub name: String,
    pub max_channels: u16,
    pub host_name: String,
    pub supported_sample_rates: Vec<u32>,
    pub supported_formats: Vec<SupportedFormat>,
}

/// Maps a cpal SampleFormat to mtrack's (sample_format, bits_per_sample) representation.
fn map_cpal_format(fmt: cpal::SampleFormat) -> SupportedFormat {
    let (sample_format, bits_per_sample) = if fmt.is_float() {
        ("float", fmt.bits_per_sample())
    } else {
        ("int", fmt.bits_per_sample())
    };
    SupportedFormat {
        sample_format: sample_format.to_string(),
        bits_per_sample,
    }
}

/// Lists audio devices as simple info structs (no trait objects).
pub fn list_device_info() -> Result<Vec<AudioDeviceInfo>, Box<dyn Error>> {
    // Suppress noisy output here.
    let _shh_stdout = shh::stdout()?;
    let _shh_stderr = shh::stderr()?;

    let mut infos: Vec<AudioDeviceInfo> = Vec::new();
    for host_id in cpal::available_hosts() {
        let host_devices = match cpal::host_from_id(host_id)?.devices() {
            Ok(d) => d,
            Err(_) => continue,
        };
        for device in host_devices {
            let mut max_channels = 0u16;
            let output_configs = match device.supported_output_configs() {
                Ok(configs) => configs,
                Err(_) => continue,
            };

            let mut sample_rates = std::collections::BTreeSet::new();
            let mut formats = std::collections::BTreeSet::new();

            for cfg in output_configs {
                if max_channels < cfg.channels() {
                    max_channels = cfg.channels();
                }

                let min_rate = cfg.min_sample_rate();
                let max_rate = cfg.max_sample_rate();
                for &rate in STANDARD_SAMPLE_RATES {
                    if rate >= min_rate && rate <= max_rate {
                        sample_rates.insert(rate);
                    }
                }

                let mapped = map_cpal_format(cfg.sample_format());
                formats.insert((mapped.sample_format.clone(), mapped.bits_per_sample));
            }
            if max_channels > 0 {
                if let Ok(id) = device.id() {
                    infos.push(AudioDeviceInfo {
                        name: id.to_string(),
                        max_channels,
                        host_name: host_id.name().to_string(),
                        supported_sample_rates: sample_rates.into_iter().collect(),
                        supported_formats: formats
                            .into_iter()
                            .map(|(sample_format, bits_per_sample)| SupportedFormat {
                                sample_format,
                                bits_per_sample,
                            })
                            .collect(),
                    });
                }
            }
        }
    }
    infos.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(infos)
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

/// Constructs a `MixerActiveSource` and its finish flag.
///
/// Every call site in `play_from` follows the same pattern: allocate a source ID,
/// create an `is_finished` flag, build the struct, and hand back the flag for
/// monitoring. This helper captures that boilerplate.
fn build_active_source(
    source: Box<dyn crate::audio::sample_source::ChannelMappedSampleSource + Send + Sync>,
    track_mappings: &HashMap<String, Vec<u16>>,
    cancel_handle: &CancelHandle,
    gain_envelope: Option<Arc<crate::audio::crossfade::GainEnvelope>>,
) -> (MixerActiveSource, Arc<AtomicBool>) {
    let id = crate::audio::next_source_id();
    let cached_source_channel_count = source.source_channel_count();
    let is_finished = Arc::new(AtomicBool::new(false));
    let flag = is_finished.clone();
    let active = MixerActiveSource {
        id,
        source,
        track_mappings: track_mappings.clone(),
        channel_mappings: Vec::new(),
        cached_source_channel_count,
        cancel_handle: cancel_handle.clone(),
        is_finished,
        start_at_sample: None,
        cancel_at_sample: None,
        gain_envelope,
    };
    (active, flag)
}

impl AudioDevice for Device {
    /// Play the given song through the audio device, starting from a specific time.
    fn play_from(
        &self,
        song: Arc<Song>,
        mappings: &HashMap<String, Vec<u16>>,
        sync: crate::playsync::PlaybackSync,
    ) -> Result<(), Box<dyn Error>> {
        let crate::playsync::PlaybackSync {
            cancel_handle,
            mut ready_tx,
            clock,
            start_time,
            loop_control,
        } = sync;
        let crate::playsync::LoopControl {
            loop_break,
            active_section,
            section_loop_break,
            loop_time_consumed,
        } = loop_control;
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

        ready_tx.send();

        clock.wait_for_start_or_cancel(&cancel_handle);
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

        // If there are already sources in the mixer (fading out from a previous
        // song), apply a fade-in envelope to the new sources for a smooth crossfade.
        let has_existing_sources = {
            let sources = self.output_manager.mixer.get_active_sources();
            let guard = sources.read();
            !guard.is_empty()
        };
        // Create sources and track their finish flags (no locks needed for monitoring)
        let mut source_finish_flags = Vec::new();

        let song_crossfade_envelope = if has_existing_sources {
            let cs = crate::audio::crossfade::default_crossfade_samples(
                self.output_manager.mixer.sample_rate(),
            );
            // Linear is fine for ≤5ms crossfades — perceptual difference
            // from EqualPower is inaudible at this duration, and Linear
            // is cheaper (no trig).
            Some(Arc::new(crate::audio::crossfade::GainEnvelope::fade_in(
                cs,
                crate::audio::crossfade::CrossfadeCurve::Linear,
            )))
        } else {
            None
        };

        for source in channel_mapped_sources.into_iter() {
            let (active_source, flag) = build_active_source(
                source,
                mappings,
                &cancel_handle,
                song_crossfade_envelope.clone(),
            );
            source_finish_flags.push(flag);
            self.output_manager.add_source(active_source)?;
        }

        // Monitor loop: polls for source completion, cancellation, and section boundaries.
        let crossfade_samples = crate::audio::crossfade::default_crossfade_samples(
            self.output_manager.mixer.sample_rate(),
        );
        let crossfade_duration = crate::audio::crossfade::DEFAULT_CROSSFADE_DURATION;

        let mut section_trigger = crate::section_loop::SectionLoopTrigger::new();

        'monitor: loop {
            if cancel_handle.is_cancelled() || loop_break.load(Ordering::Relaxed) {
                break;
            }

            // Check if all sources have finished (EOF).
            let all_finished = source_finish_flags
                .iter()
                .all(|flag| flag.load(Ordering::Relaxed));
            if all_finished {
                break;
            }

            // Check for section loop boundary.
            if let Some(section) = active_section.read().as_ref() {
                if !section_loop_break.load(Ordering::Relaxed) {
                    let elapsed = clock.elapsed();

                    if section_trigger
                        .check(section, elapsed, crossfade_duration)
                        .is_some()
                    {
                        info!(
                            section = section.name,
                            "Audio section loop: crossfading back to section start"
                        );

                        // Fade out current sources.
                        let current_ids: Vec<u64> = {
                            let sources = self.output_manager.mixer.get_active_sources();
                            let guard = sources.read();
                            guard.iter().map(|s| s.lock().id).collect()
                        };
                        if !current_ids.is_empty() {
                            let fade_out =
                                Arc::new(crate::audio::crossfade::GainEnvelope::fade_out(
                                    crossfade_samples,
                                    crate::audio::crossfade::CrossfadeCurve::Linear,
                                ));
                            self.output_manager
                                .mixer
                                .set_gain_envelope(&current_ids, fade_out);
                        }

                        // Create new sources at section start with fade-in.
                        let section_start = section.start_time;
                        match song.create_channel_mapped_sources_from(
                            &playback_context,
                            section_start,
                            mappings,
                        ) {
                            Ok(new_sources) => {
                                // Replace finish flags with new source flags.
                                source_finish_flags.clear();
                                let fade_in_envelope =
                                    Some(Arc::new(crate::audio::crossfade::GainEnvelope::fade_in(
                                        crossfade_samples,
                                        crate::audio::crossfade::CrossfadeCurve::Linear,
                                    )));
                                for source in new_sources {
                                    let (active_source, flag) = build_active_source(
                                        source,
                                        mappings,
                                        &cancel_handle,
                                        fade_in_envelope.clone(),
                                    );
                                    source_finish_flags.push(flag);
                                    if let Err(e) = self.output_manager.add_source(active_source) {
                                        error!(err = %e, "Failed to add section loop source");
                                    }
                                }
                            }
                            Err(e) => {
                                error!(err = %e, "Failed to create section loop sources");
                                break 'monitor;
                            }
                        }

                        // Accumulate consumed time so elapsed() reports correct song position.
                        let section_duration = section.end_time.saturating_sub(section.start_time);
                        *loop_time_consumed.lock() += section_duration;

                        // Continue monitoring the new sources.
                        continue 'monitor;
                    }
                }
            }

            thread::sleep(Duration::from_millis(10));
        }

        // When section_loop_break is set, clear active_section so the song
        // continues normally past the section end.
        if section_loop_break.load(Ordering::Relaxed) {
            let mut section = active_section.write();
            *section = None;
        }

        // Loop if the song has loop_playback and we haven't been told to stop.
        while song.loop_playback()
            && !cancel_handle.is_cancelled()
            && !loop_break.load(Ordering::Relaxed)
        {
            info!(song = song.name(), "Audio loop: creating crossfade sources");

            let crossfade_samples = crate::audio::crossfade::default_crossfade_samples(
                self.output_manager.mixer.sample_rate(),
            );

            // Fade out any remaining sources from previous iteration.
            let current_ids: Vec<u64> = {
                let sources = self.output_manager.mixer.get_active_sources();
                let guard = sources.read();
                guard.iter().map(|s| s.lock().id).collect()
            };
            if !current_ids.is_empty() {
                let fade_out = Arc::new(crate::audio::crossfade::GainEnvelope::fade_out(
                    crossfade_samples,
                    crate::audio::crossfade::CrossfadeCurve::Linear,
                ));
                self.output_manager
                    .mixer
                    .set_gain_envelope(&current_ids, fade_out);
            }

            // Create new sources at t=0 with fade-in.
            let new_sources = match song.create_channel_mapped_sources_from(
                &playback_context,
                Duration::ZERO,
                mappings,
            ) {
                Ok(s) => s,
                Err(e) => {
                    error!(err = e.as_ref(), "Failed to create loop audio sources");
                    break;
                }
            };

            let mut new_finish_flags = Vec::new();
            let fade_in_envelope = Some(Arc::new(crate::audio::crossfade::GainEnvelope::fade_in(
                crossfade_samples,
                crate::audio::crossfade::CrossfadeCurve::Linear,
            )));
            for source in new_sources {
                let (active_source, flag) =
                    build_active_source(source, mappings, &cancel_handle, fade_in_envelope.clone());
                new_finish_flags.push(flag);
                if let Err(e) = self.output_manager.add_source(active_source) {
                    error!(err = %e, "Failed to add loop source to mixer");
                    break;
                }
            }

            if new_finish_flags.is_empty() {
                break;
            }

            // Wait for new sources to finish or cancel/loop_break.
            let loop_finished = Arc::new(AtomicBool::new(false));
            let loop_finished_monitor = loop_finished.clone();
            let cancel_for_monitor = cancel_handle.clone();
            let loop_break_for_monitor = loop_break.clone();
            let monitor_flags = new_finish_flags;

            thread::spawn(move || loop {
                if cancel_for_monitor.is_cancelled()
                    || loop_break_for_monitor.load(Ordering::Relaxed)
                {
                    break;
                }
                if monitor_flags.iter().all(|f| f.load(Ordering::Relaxed)) {
                    loop_finished_monitor.store(true, Ordering::Relaxed);
                    cancel_for_monitor.notify();
                    break;
                }
                thread::sleep(Duration::from_millis(10));
            });

            cancel_handle.wait(loop_finished);
        }

        Ok(())
    }

    fn mixer(&self) -> Option<Arc<super::super::mixer::AudioMixer>> {
        Some(Arc::new(self.output_manager.mixer.clone()))
    }

    fn source_sender(&self) -> Option<super::super::SourceSender> {
        Some(self.output_manager.source_tx.clone())
    }

    fn sample_counter(&self) -> Option<Arc<AtomicU64>> {
        Some(self.output_manager.mixer.sample_counter())
    }

    fn sample_rate(&self) -> Option<u32> {
        Some(self.output_manager.mixer.sample_rate())
    }

    #[cfg(test)]
    fn to_mock(&self) -> Result<Arc<super::super::mock::Device>, Box<dyn Error>> {
        Err("not a mock".into())
    }
}

#[cfg(test)]
mod test {
    use super::*;

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
