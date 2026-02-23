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

//! Audio trigger engine: cpal input stream + event forwarding.
//!
//! Opens a cpal input device, routes per-channel samples to `TriggerDetector`
//! instances, and produces `TriggerAction` events via a crossbeam channel.

use std::error::Error;

use cpal::traits::{DeviceTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use tracing::{debug, error, info, warn};

use super::detector::TriggerDetector;
use super::ms_to_samples;
use crate::audio::format::SampleFormat;
use crate::config::trigger::{TriggerConfig, TriggerInput, TriggerInputAction};
use crate::samples::TriggerAction;

/// Manages a cpal input stream and per-channel trigger detectors.
///
/// The engine captures audio from the configured input device, feeds samples
/// to the appropriate detectors, and forwards resulting `TriggerAction` events
/// through a crossbeam channel.
pub struct TriggerEngine {
    /// The cpal input stream (kept alive by ownership).
    _stream: cpal::Stream,
    /// Receiver for trigger actions produced by detectors.
    receiver: Receiver<TriggerAction>,
}

impl TriggerEngine {
    /// Creates a new trigger engine from configuration.
    ///
    /// Opens the named cpal input device, creates detectors for each configured
    /// input channel, and starts the input stream.
    pub fn new(config: &TriggerConfig) -> Result<Self, Box<dyn Error>> {
        let device_name = config
            .device()
            .ok_or("Trigger config has audio inputs but no device specified")?;
        let device = crate::audio::find_input_device(device_name)?;

        let device_id = device
            .id()
            .map(|id| id.to_string())
            .unwrap_or_else(|_| "unknown".to_string());
        info!(device = %device_id, "Found trigger input device");

        // Query device capabilities once and reuse.
        let default_config = device.default_input_config()?;
        let supported_configs: Vec<_> = device
            .supported_input_configs()
            .map(|configs| configs.collect())
            .unwrap_or_default();

        // Open with the device's max supported input channel count, matching how the
        // output stream works. ALSA plughw will remap/downmix if you request fewer
        // channels than the hardware provides, so we need the full width to get the
        // correct physical channels.
        let channels = supported_configs
            .iter()
            .map(|c: &cpal::SupportedStreamConfigRange| c.channels())
            .max()
            .filter(|&n| n > 0)
            .unwrap_or_else(|| default_config.channels());

        // Determine sample format: if the config specifies format/bits, use that;
        // otherwise auto-detect the device's native format.
        let native_format = supported_configs
            .iter()
            .find(|c| c.channels() == channels)
            .map(|c| c.sample_format())
            .unwrap_or(cpal::SampleFormat::F32);

        let stream_format = match (config.sample_format(), config.bits_per_sample()) {
            (Some(SampleFormat::Float), _) => cpal::SampleFormat::F32,
            (Some(SampleFormat::Int), Some(16)) => cpal::SampleFormat::I16,
            (Some(SampleFormat::Int), _) => cpal::SampleFormat::I32,
            (None, Some(16)) => cpal::SampleFormat::I16,
            _ => native_format,
        };

        let sample_rate = config.sample_rate().unwrap_or(default_config.sample_rate());

        let buffer_size = match config.buffer_size() {
            Some(n) => cpal::BufferSize::Fixed(n as u32),
            None => cpal::BufferSize::Default,
        };

        let stream_config = cpal::StreamConfig {
            channels,
            sample_rate,
            buffer_size,
        };

        info!(
            sample_rate = sample_rate,
            channels,
            stream_format = ?stream_format,
            buffer_size = ?config.buffer_size(),
            "Trigger input stream config"
        );

        // Pre-compute crosstalk parameters (ms → samples) if both fields are set.
        let crosstalk: Option<(u32, f32)> =
            match (config.crosstalk_window_ms(), config.crosstalk_threshold()) {
                (Some(ms), Some(mult)) => {
                    let window_samples = ms_to_samples(ms, sample_rate);
                    info!(
                        crosstalk_window_ms = ms,
                        crosstalk_threshold = mult,
                        crosstalk_window_samples = window_samples,
                        "Crosstalk suppression enabled"
                    );
                    Some((window_samples, mult))
                }
                _ => None,
            };

        // Build detectors for each configured input, keyed by 0-indexed channel
        let mut detector_map: Vec<Option<TriggerDetector>> = (0..channels).map(|_| None).collect();

        for input in config.inputs().iter().filter_map(|i| match i {
            TriggerInput::Audio(audio) => Some(audio),
            _ => None,
        }) {
            // Validate that trigger actions have a sample name and release actions have a group
            match input.action() {
                TriggerInputAction::Trigger => {
                    if input.sample().is_none_or(|s| s.is_empty()) {
                        return Err(format!(
                            "Trigger input on channel {} has action 'trigger' but no sample name configured",
                            input.channel()
                        ).into());
                    }
                }
                TriggerInputAction::Release => {
                    if input.release_group().is_none_or(|s| s.is_empty()) {
                        return Err(format!(
                            "Trigger input on channel {} has action 'release' but no release_group configured",
                            input.channel()
                        ).into());
                    }
                }
            }

            let ch_idx = input.channel().checked_sub(1).ok_or_else(|| {
                format!(
                    "Trigger input channel must be >= 1, got {}",
                    input.channel()
                )
            })? as usize;

            if ch_idx >= channels as usize {
                warn!(
                    channel = input.channel(),
                    device_channels = channels,
                    "Trigger input channel exceeds device channel count, skipping"
                );
                continue;
            }

            let detector = TriggerDetector::from_input(input, sample_rate);

            detector_map[ch_idx] = Some(detector);
            debug!(
                channel = input.channel(),
                sample = input.sample().unwrap_or("(release)"),
                "Trigger detector created"
            );
        }

        // Create the event channel (bounded to prevent unbounded growth under load)
        let (tx, rx) = crossbeam_channel::bounded(256);

        // Build the input stream using the resolved sample format
        let stream = Self::build_input_stream(
            &device,
            &stream_config,
            detector_map,
            channels,
            tx,
            stream_format,
            crosstalk,
        )?;

        stream.play()?;
        info!("Trigger input stream started");

        Ok(TriggerEngine {
            _stream: stream,
            receiver: rx,
        })
    }

    /// Returns a clone of the trigger action receiver.
    pub fn subscribe(&self) -> Receiver<TriggerAction> {
        self.receiver.clone()
    }

    /// Builds the cpal input stream, matching the device's native sample format.
    fn build_input_stream(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        detectors: Vec<Option<TriggerDetector>>,
        channels: u16,
        tx: Sender<TriggerAction>,
        sample_format: cpal::SampleFormat,
        crosstalk: Option<(u32, f32)>,
    ) -> Result<cpal::Stream, Box<dyn Error>> {
        match sample_format {
            cpal::SampleFormat::I16 => Self::build_input_stream_typed::<i16>(
                device, config, detectors, channels, tx, crosstalk,
            ),
            cpal::SampleFormat::I32 => Self::build_input_stream_typed::<i32>(
                device, config, detectors, channels, tx, crosstalk,
            ),
            _ => Self::build_input_stream_typed::<f32>(
                device, config, detectors, channels, tx, crosstalk,
            ),
        }
    }

    /// Builds a typed cpal input stream, converting samples to f32 for detection.
    fn build_input_stream_typed<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        mut detectors: Vec<Option<TriggerDetector>>,
        channels: u16,
        tx: Sender<TriggerAction>,
        crosstalk: Option<(u32, f32)>,
    ) -> Result<cpal::Stream, Box<dyn Error>>
    where
        T: cpal::SizedSample + 'static,
        f32: cpal::FromSample<T>,
    {
        let stream = device.build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                // Data is interleaved: [ch0, ch1, ch2, ..., ch0, ch1, ...]
                for frame in data.chunks_exact(channels as usize) {
                    // Track which channels fired in this frame for crosstalk.
                    let mut fired_channels: u64 = 0;

                    for (ch_idx, raw_sample) in frame.iter().enumerate() {
                        if let Some(ref mut detector) = detectors[ch_idx] {
                            let sample: f32 =
                                <f32 as cpal::FromSample<T>>::from_sample_(*raw_sample);
                            if let Some(action) = detector.process_sample(sample) {
                                if ch_idx < 64 {
                                    fired_channels |= 1u64 << ch_idx;
                                }
                                // Non-blocking send — drop events if channel is full
                                if tx.try_send(action).is_err() {
                                    error!("Trigger event dropped (channel full)");
                                }
                            }
                        }
                    }

                    // Apply crosstalk suppression to all OTHER detectors when any fired.
                    // Crosstalk tracking is limited to the first 64 channels.
                    if let Some((window_samples, multiplier)) = crosstalk {
                        if fired_channels != 0 {
                            for (ch_idx, slot) in detectors.iter_mut().enumerate() {
                                if ch_idx >= 64 || fired_channels & (1u64 << ch_idx) == 0 {
                                    if let Some(ref mut detector) = slot {
                                        detector.apply_crosstalk_suppression(
                                            window_samples,
                                            multiplier,
                                        );
                                    }
                                }
                            }
                        }
                    }
                }
            },
            move |err| {
                error!(error = %err, "Trigger input stream error");
            },
            None,
        )?;

        Ok(stream)
    }
}
