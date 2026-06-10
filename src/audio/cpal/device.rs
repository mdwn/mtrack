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
        mpsc, Arc,
    },
    thread,
    time::Duration,
};

use cpal::traits::{DeviceTrait, HostTrait};
use tracing::{debug, error, info, span, warn, Level};

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
    let num_channels = match mappings.iter().flat_map(|entry| entry.1).max() {
        Some(max) => *max,
        None => {
            return Err(format!(
                "no track mappings configured for audio device {} — map at least one track to an output channel before playing",
                device_name
            )
            .into())
        }
    };

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
/// Finds the section the playhead currently sits inside, if any, resolved to
/// absolute time bounds via the song's beat grid.
///
/// This is the loop-back candidate: whichever section the performer is hearing
/// could be armed at any moment, and arming loops back to its `start_time`. The
/// audio monitor uses this to prewarm the loop-back sources speculatively, so
/// they are ready the instant the loop is armed — even on the last beat of the
/// section — rather than incurring a file-open/seek/warmup stall at the first
/// boundary.
fn current_playhead_section(
    song: &Song,
    elapsed: Duration,
) -> Option<crate::player::SectionBounds> {
    song.sections().iter().find_map(|s| {
        let (start_time, end_time) = song.resolve_section(&s.name)?;
        (elapsed >= start_time && elapsed < end_time).then(|| crate::player::SectionBounds {
            name: s.name.clone(),
            start_time,
            end_time,
        })
    })
}

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
    // Pre-allocate the cancel slot so the section-loop monitor can schedule a
    // sample-accurate cut on an already-playing source via
    // `AudioMixer::set_cancel_at_sample`. The mixer treats 0 as "no cancel".
    let active = MixerActiveSource {
        id,
        source,
        track_mappings: track_mappings.clone(),
        channel_mappings: Vec::new(),
        cached_source_channel_count,
        cancel_handle: cancel_handle.clone(),
        is_finished,
        start_at_sample: None,
        cancel_at_sample: Some(Arc::new(std::sync::atomic::AtomicU64::new(0))),
        gain: Default::default(),
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
    ) -> Result<(), crate::audio::AudioError> {
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

        validate_channel_count(mappings, self.max_channels, song.name(), &self.name)
            .map_err(|e| crate::audio::AudioError::Playback(e.to_string()))?;

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
        let channel_mapped_sources = song
            .create_channel_mapped_sources_from(&playback_context, start_time, mappings)
            .map_err(|e| crate::audio::AudioError::Playback(e.to_string()))?;

        // Add all sources to the output manager
        if channel_mapped_sources.is_empty() {
            return Err(crate::audio::AudioError::Playback(
                "No sources found in song".to_string(),
            ));
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
            self.output_manager
                .add_source(active_source)
                .map_err(|e| crate::audio::AudioError::Playback(e.to_string()))?;
        }

        // Startup skew between the clock epoch and the moment the original
        // source actually goes live in the mixer. `clock.start()` is called by
        // the player *before* this function builds and warms its sources, so the
        // free-running mixer counter has already advanced by the time the audio
        // is added. The clock's elapsed reading here is exactly that skew: the
        // original source plays this many samples behind what the clock reports,
        // for the whole song.
        //
        // Section-loop boundaries are computed from the clock
        // (`loop_boundary_sample`), and the original source is the *outgoing*
        // side of the first loop handoff while every loop source is pinned to
        // the sample counter. So without correction the first handoff lands
        // `skew` samples early (the section is cut short) while later
        // counter-pinned `N -> N` handoffs are seamless. Adding the skew to
        // every computed boundary realigns it to the audio content rather than
        // the clock epoch; loop sources started at the corrected boundaries
        // inherit the same alignment, so all handoffs — first included — stay
        // seamless. (The resampler group delay is symmetric between the outgoing
        // and incoming sources and cancels, so it needs no correction.)
        let audio_clock_skew_samples = (clock.elapsed().as_secs_f64()
            * self.output_manager.mixer.sample_rate() as f64)
            .round() as u64;
        debug!(
            audio_clock_skew_samples,
            skew_us = clock.elapsed().as_micros() as u64,
            "Audio section loop: original sources live; loop boundaries corrected for startup skew"
        );

        // Monitor loop: polls for source completion, cancellation, and section boundaries.
        let crossfade_samples = crate::audio::crossfade::default_crossfade_samples(
            self.output_manager.mixer.sample_rate(),
        );
        let mixer_sample_rate = self.output_manager.mixer.sample_rate();

        // Look-ahead for the section-loop trigger. Must exceed the 10ms poll
        // sleep below so the trigger is always detected *before* the ideal
        // loop boundary; we then schedule the actual crossfade at the exact
        // sample via `start_at_sample`/`cancel_at_sample`, eliminating
        // polling jitter from the audible loop point. The cost is a small
        // race window for break-after-trigger, which we accept.
        let loop_trigger_lookahead = Duration::from_millis(30);

        let mut section_trigger = crate::section_loop::SectionLoopTrigger::new();

        type PrebuiltSources = Vec<Box<dyn crate::audio::sample_source::ChannelMappedSampleSource>>;

        // Sources pre-built for the next loop iteration, keyed by section name
        // so a section change invalidates them. Ready to swap in with zero I/O.
        let mut pending_sources: Option<(String, PrebuiltSources)> = None;

        // In-flight background build (section name + result channel). The build
        // runs on a detached worker thread rather than inline because
        // `create_channel_mapped_sources_from` blocks on BufferedSampleSource
        // warmup (file open/seek/decode). Doing that on the monitor thread
        // could delay the *first* loop boundary — the loop can be armed close
        // to the section end and the first build hits a cold file cache — which
        // showed up as a one-off stutter on the first iteration.
        let mut build_rx: Option<(String, mpsc::Receiver<Result<PrebuiltSources, String>>)> = None;

        // Deferred `loop_time_consumed` bumps. Each entry is the sample at
        // which the audio actually loops (so the reported elapsed only jumps
        // when the listener crosses the boundary, not when the monitor
        // detected it).
        let mut pending_consumption: Vec<(u64, Duration)> = Vec::new();

        // DIAGNOSTIC: per-swap counter to compare the first loop against later ones.
        let mut loop_iteration: u64 = 0;

        'monitor: loop {
            if cancel_handle.is_cancelled() || loop_break.load(Ordering::Relaxed) {
                break;
            }

            // Apply any deferred loop_time_consumed bumps whose sample has
            // been reached by the audio callback.
            if !pending_consumption.is_empty() {
                let now_sample = self.output_manager.mixer.current_sample();
                pending_consumption.retain(|&(apply_at, amount)| {
                    if now_sample >= apply_at {
                        *loop_time_consumed.lock() += amount;
                        false
                    } else {
                        true
                    }
                });
            }

            // Check if all sources have finished (EOF).
            let all_finished = source_finish_flags
                .iter()
                .all(|flag| flag.load(Ordering::Relaxed));
            if all_finished {
                break;
            }

            let armed = active_section.read().clone();
            let break_requested = section_loop_break.load(Ordering::Relaxed);

            // Which section we want warm loop-back sources for. Once armed, the
            // active section is authoritative. Before arming, speculatively
            // target whichever section the playhead is currently inside — it is
            // the loop-back candidate, so the sources are warm the instant the
            // performer arms, even on the section's final beat. (During looping
            // the raw clock runs past the section end, so we rely on `armed`
            // rather than the playhead lookup.)
            let prewarm_target = if break_requested {
                None
            } else if armed.is_some() {
                armed.clone()
            } else {
                current_playhead_section(&song, clock.elapsed())
            };

            // Collect the result of a finished background build. Take the
            // channel out and only restore it if the build is still running.
            let wanted = |name: &str| prewarm_target.as_ref().is_some_and(|t| t.name == name);
            if let Some((name, rx)) = build_rx.take() {
                match rx.try_recv() {
                    Ok(Ok(srcs)) => {
                        // Keep only if we still want this section's sources.
                        if wanted(&name) {
                            pending_sources = Some((name, srcs));
                        }
                    }
                    Ok(Err(e)) => {
                        error!(
                            err = %e,
                            section = name,
                            "Failed to pre-build section loop sources; will retry"
                        );
                    }
                    // Still building — put the channel back.
                    Err(mpsc::TryRecvError::Empty) => build_rx = Some((name, rx)),
                    // Worker vanished without sending (e.g. spawn failed);
                    // leave build_rx cleared so we retry next iteration.
                    Err(mpsc::TryRecvError::Disconnected) => {}
                }
            }

            // Drop warmed sources / abandon an in-flight build if the target
            // section changed (playhead moved on, loop armed elsewhere, section
            // cleared, or a break was requested) — otherwise we'd swap stale
            // audio in on the next trigger.
            if pending_sources
                .as_ref()
                .is_some_and(|(name, _)| !wanted(name))
            {
                pending_sources = None;
            }
            if build_rx.as_ref().is_some_and(|(name, _)| !wanted(name)) {
                // Dropping the receiver detaches the worker; its eventual send
                // is a harmless no-op.
                build_rx = None;
            }

            // Kick off a background build for the target if we have neither
            // warmed sources nor a build already in flight. The worker thread
            // absorbs the warmup/I/O block so trigger detection here is never
            // delayed.
            if let Some(target) = &prewarm_target {
                if pending_sources.is_none() && build_rx.is_none() {
                    let (tx, rx) = mpsc::channel();
                    let song = song.clone();
                    let ctx = playback_context.clone();
                    let mappings = mappings.clone();
                    let start_time = target.start_time;
                    let _ = thread::Builder::new()
                        .name("mtrack-section-prebuild".into())
                        .spawn(move || {
                            let result = song
                                .create_channel_mapped_sources_from(&ctx, start_time, &mappings)
                                .map_err(|e| e.to_string());
                            // Receiver may be gone if the target changed.
                            let _ = tx.send(result);
                        });
                    build_rx = Some((target.name.clone(), rx));
                }
            }

            // Only loop when the section is actually armed.
            if let Some(section) = armed {
                if !break_requested {
                    let elapsed = clock.elapsed();
                    if let Some(trigger_time) =
                        section_trigger.check(&section, elapsed, loop_trigger_lookahead)
                    {
                        let pending = match pending_sources.take() {
                            Some((_, srcs)) => srcs,
                            None => {
                                warn!(
                                    section = section.name,
                                    "Audio section loop: trigger fired with no pre-built \
                                     sources; skipping this boundary"
                                );
                                thread::sleep(Duration::from_millis(10));
                                continue 'monitor;
                            }
                        };

                        // Map the ideal trigger time to an exact mixer sample.
                        // We use the live sample counter plus the remaining
                        // delta so the result is robust to any small skew
                        // between the playback clock and the mixer counter.
                        let now_sample = self.output_manager.mixer.current_sample();
                        // Correct for the startup skew so the boundary tracks the
                        // audio content, not the clock epoch (see where
                        // `audio_clock_skew_samples` is captured). The shift is a
                        // constant, so inter-boundary spacing is unchanged.
                        let loop_sample = crate::section_loop::loop_boundary_sample(
                            now_sample,
                            elapsed,
                            trigger_time,
                            mixer_sample_rate,
                        )
                        .saturating_add(audio_clock_skew_samples);
                        // At a section-loop boundary the loop-point content
                        // coincides on both sources: the section's `end_time` on
                        // the outgoing source is the same musical moment as its
                        // `start_time` on the incoming one — for a one-measure
                        // loop, literally the same downbeat. With the boundary
                        // aligned (seek-residual trim + startup-skew correction),
                        // the two copies land on the same sample, so we overlap
                        // them and let them reinforce into a single click rather
                        // than cutting one.
                        //
                        // The crossfade straddles the boundary on the outgoing
                        // side: the outgoing source plays THROUGH loop_sample at
                        // full gain and fades out over the following window, while
                        // the incoming source begins exactly at loop_sample and
                        // fades in. This matters because the outgoing source's
                        // resampler is in steady state, so its copy of the downbeat
                        // is clean and carries the attack; the incoming source's
                        // resampler is freshly created, so its first samples carry
                        // a sinc pre-ring — the fade-in attenuates that pre-ring
                        // while the clean outgoing transient dominates the attack.
                        let fade_end_sample = loop_sample.saturating_add(crossfade_samples);

                        loop_iteration += 1;
                        debug!(
                            section = section.name,
                            loop_iteration,
                            loop_sample,
                            fade_end_sample,
                            "Audio section loop: scheduling crossfade"
                        );

                        // Schedule fade-out + cut on currently-playing sources.
                        // The envelope holds at 1.0 until loop_sample, then ramps
                        // to 0 by fade_end_sample; cancel_at_sample removes them
                        // once the fade completes.
                        let current_ids: Vec<u64> = {
                            let sources = self.output_manager.mixer.get_active_sources();
                            let guard = sources.read();
                            guard.iter().map(|s| s.lock().id).collect()
                        };
                        if !current_ids.is_empty() {
                            let fade_out = Arc::new(
                                crate::audio::crossfade::GainEnvelope::fade_out(
                                    crossfade_samples,
                                    crate::audio::crossfade::CrossfadeCurve::Linear,
                                )
                                .with_start_sample(loop_sample),
                            );
                            self.output_manager
                                .mixer
                                .set_gain_envelope(&current_ids, fade_out);
                            self.output_manager
                                .mixer
                                .set_cancel_at_sample(&current_ids, fade_end_sample);
                        }

                        // Add the pre-built sources. Their own start_at_sample
                        // delays playback until loop_sample; the fade-in envelope
                        // ramps from there so the incoming source's resampler
                        // pre-ring is attenuated while the clean outgoing transient
                        // carries the downbeat attack.
                        source_finish_flags.clear();
                        for source in pending {
                            // Each source gets its own envelope instance — a shared
                            // GainEnvelope.position would advance once per source
                            // per batch and the fade would complete N× too fast.
                            let fade_in =
                                Some(Arc::new(crate::audio::crossfade::GainEnvelope::fade_in(
                                    crossfade_samples,
                                    crate::audio::crossfade::CrossfadeCurve::Linear,
                                )));
                            let (mut active_source, flag) =
                                build_active_source(source, mappings, &cancel_handle, fade_in);
                            active_source.start_at_sample = Some(loop_sample);
                            source_finish_flags.push(flag);
                            if let Err(e) = self.output_manager.add_source(active_source) {
                                error!(err = %e, "Failed to add section loop source");
                            }
                        }

                        // Defer the consumed-time bump until the audio thread
                        // actually crosses the boundary, so the user-visible
                        // elapsed doesn't briefly jump backwards.
                        let section_duration = section.end_time.saturating_sub(section.start_time);
                        pending_consumption.push((loop_sample, section_duration));

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
    fn to_mock(&self) -> Result<Arc<super::super::mock::Device>, crate::audio::AudioError> {
        Err(crate::audio::AudioError::Other("not a mock".into()))
    }
}

#[cfg(test)]
mod test {
    use super::*;

    mod current_playhead_section_tests {
        use super::*;

        // Two 4-beat measures at 120 BPM: beats every 0.5s.
        // verse = measures 1..2 → [0.0, 2.0); chorus = measures 2..3 → [2.0, 3.5).
        fn song_with_sections() -> Song {
            Song::new_for_test_with_sections(
                "test",
                &["click"],
                crate::audio::click_analysis::BeatGrid {
                    beats: vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
                    measure_starts: vec![0, 4],
                },
                vec![
                    crate::config::Section {
                        name: "verse".to_string(),
                        start_measure: 1,
                        end_measure: 2,
                    },
                    crate::config::Section {
                        name: "chorus".to_string(),
                        start_measure: 2,
                        end_measure: 3,
                    },
                ],
            )
        }

        #[test]
        fn finds_section_containing_playhead() {
            let song = song_with_sections();
            assert_eq!(
                current_playhead_section(&song, Duration::from_millis(500)).map(|s| s.name),
                Some("verse".to_string())
            );
            assert_eq!(
                current_playhead_section(&song, Duration::from_millis(2500)).map(|s| s.name),
                Some("chorus".to_string())
            );
        }

        #[test]
        fn boundary_is_half_open() {
            let song = song_with_sections();
            // Exactly at the verse/chorus boundary belongs to chorus: a
            // section owns [start, end), so the measure boundary is the start
            // of the next section, not the tail of the previous one.
            let at_boundary = current_playhead_section(&song, Duration::from_secs(2)).unwrap();
            assert_eq!(at_boundary.name, "chorus");
            assert_eq!(at_boundary.start_time, Duration::from_secs(2));
        }

        #[test]
        fn none_when_past_all_sections() {
            let song = song_with_sections();
            assert!(current_playhead_section(&song, Duration::from_secs(10)).is_none());
        }

        #[test]
        fn none_without_beat_grid() {
            // A plain test song has no beat grid, so nothing resolves.
            let song = Song::new_for_test("test", &["click"]);
            assert!(current_playhead_section(&song, Duration::from_secs(1)).is_none());
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
        fn fails_on_empty_mappings_with_descriptive_error() {
            let mappings: HashMap<String, Vec<u16>> = HashMap::new();
            let result = validate_channel_count(&mappings, 8, "song", "device");
            let err = result.unwrap_err().to_string();
            assert!(
                err.contains("no track mappings configured"),
                "error should explain that mappings are missing, got: {err}"
            );
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
