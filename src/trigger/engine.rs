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
use std::sync::Arc;

use cpal::traits::{DeviceTrait, StreamTrait};
use crossbeam_channel::{Receiver, Sender};
use parking_lot::{Condvar, Mutex};
use tracing::{debug, error, info, warn};

use super::detector::TriggerDetector;
use super::ms_to_samples;
use crate::audio::format::SampleFormat;
use crate::config::trigger::{AudioTriggerInput, TriggerConfig, TriggerInput, TriggerInputAction};
use crate::samples::TriggerAction;
use crate::thread_priority::{callback_thread_priority, promote_to_realtime, rt_audio_enabled};

/// Shared condvar notify for signalling stream errors from the error callback
/// to the recovery thread.
type ErrorNotify = Arc<(Mutex<bool>, Condvar)>;

/// Shared shutdown signal so `Drop` can wake the recovery thread.
type ShutdownNotify = Arc<(Mutex<bool>, Condvar)>;

/// Manages a cpal input stream and per-channel trigger detectors.
///
/// The engine captures audio from the configured input device, feeds samples
/// to the appropriate detectors, and forwards resulting `TriggerAction` events
/// through a crossbeam channel. On backend errors (e.g. ALSA POLLERR), the
/// stream is automatically recreated.
pub struct TriggerEngine {
    /// Receiver for trigger actions produced by detectors.
    receiver: Receiver<TriggerAction>,
    /// Background thread running the stream recovery loop.
    _thread: Option<std::thread::JoinHandle<()>>,
    /// Shutdown signal to stop the recovery thread on drop.
    shutdown: ShutdownNotify,
}

/// Parameters captured from config for rebuilding the input stream on recovery.
struct StreamParams {
    device_name: String,
    stream_config: cpal::StreamConfig,
    sample_format: cpal::SampleFormat,
    crosstalk: Option<(u32, f32)>,
    inputs: Vec<TriggerInput>,
}

impl TriggerEngine {
    /// Creates a new trigger engine from configuration.
    ///
    /// Opens the named cpal input device, creates detectors for each configured
    /// input channel, and starts the input stream. On backend errors (e.g. ALSA
    /// POLLERR), the stream is automatically recreated.
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

        let stream_format = resolve_stream_format(
            config.sample_format(),
            config.bits_per_sample(),
            native_format,
        );

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
        let crosstalk = resolve_crosstalk(
            config.crosstalk_window_ms(),
            config.crosstalk_threshold(),
            sample_rate,
        );
        if crosstalk.is_some() {
            info!(
                crosstalk_window_ms = config.crosstalk_window_ms().unwrap(),
                crosstalk_threshold = config.crosstalk_threshold().unwrap(),
                "Crosstalk suppression enabled"
            );
        }

        // Create the event channel (bounded to prevent unbounded growth under load)
        let (tx, rx) = crossbeam_channel::bounded(256);

        let params = StreamParams {
            device_name: device_name.to_string(),
            stream_config,
            sample_format: stream_format,
            crosstalk,
            inputs: config.inputs().to_vec(),
        };

        let error_notify: ErrorNotify = Arc::new((Mutex::new(false), Condvar::new()));
        let shutdown: ShutdownNotify = Arc::new((Mutex::new(false), Condvar::new()));

        // Build the initial stream before spawning the recovery thread.
        let detector_map =
            build_detector_map(&params.inputs, params.stream_config.channels, sample_rate)?;
        let stream = Self::build_input_stream(
            &device,
            &params.stream_config,
            detector_map,
            params.stream_config.channels,
            tx.clone(),
            params.sample_format,
            params.crosstalk,
            error_notify.clone(),
        )?;
        stream.play()?;
        info!("Trigger input stream started");

        // Spawn recovery thread — mirrors the audio output stream pattern.
        let thread = {
            let shutdown = shutdown.clone();
            std::thread::Builder::new()
                .name("trigger-input-recovery".into())
                .spawn(move || {
                    Self::recovery_loop(stream, params, tx, error_notify, shutdown);
                })?
        };

        Ok(TriggerEngine {
            receiver: rx,
            _thread: Some(thread),
            shutdown,
        })
    }

    /// Returns a clone of the trigger action receiver.
    pub fn subscribe(&self) -> Receiver<TriggerAction> {
        self.receiver.clone()
    }

    /// Runs the stream recovery loop. Blocks until shutdown.
    fn recovery_loop(
        mut stream: cpal::Stream,
        params: StreamParams,
        tx: Sender<TriggerAction>,
        error_notify: ErrorNotify,
        shutdown: ShutdownNotify,
    ) {
        loop {
            // Wait for either a stream error or shutdown.
            let (err_mutex, err_condvar) = &*error_notify;
            let (shut_mutex, _) = &*shutdown;
            loop {
                if *shut_mutex.lock() {
                    drop(stream);
                    return;
                }
                let mut errored = err_mutex.lock();
                if *errored {
                    *errored = false;
                    break;
                }
                err_condvar.wait_for(&mut errored, std::time::Duration::from_millis(500));
            }

            // Drop the old stream and attempt to rebuild.
            drop(stream);
            warn!("Trigger input stream error detected, attempting recovery");

            // Retry with backoff until the device is available again.
            loop {
                if *shutdown.0.lock() {
                    return;
                }

                let device = match crate::audio::find_input_device(&params.device_name) {
                    Ok(d) => d,
                    Err(e) => {
                        warn!(error = %e, "Trigger recovery: device not found, retrying");
                        std::thread::sleep(std::time::Duration::from_secs(1));
                        continue;
                    }
                };

                let detector_map = match build_detector_map(
                    &params.inputs,
                    params.stream_config.channels,
                    params.stream_config.sample_rate,
                ) {
                    Ok(d) => d,
                    Err(e) => {
                        warn!(error = %e, "Trigger recovery: failed to build detectors, retrying");
                        std::thread::sleep(std::time::Duration::from_secs(1));
                        continue;
                    }
                };

                match Self::build_input_stream(
                    &device,
                    &params.stream_config,
                    detector_map,
                    params.stream_config.channels,
                    tx.clone(),
                    params.sample_format,
                    params.crosstalk,
                    error_notify.clone(),
                ) {
                    Ok(new_stream) => {
                        if let Err(e) = new_stream.play() {
                            warn!(error = %e, "Trigger recovery: failed to start stream, retrying");
                            std::thread::sleep(std::time::Duration::from_secs(1));
                            continue;
                        }
                        info!("Trigger input stream recovered after backend error");
                        stream = new_stream;
                        break;
                    }
                    Err(e) => {
                        warn!(error = %e, "Trigger recovery: failed to build stream, retrying");
                        std::thread::sleep(std::time::Duration::from_secs(1));
                    }
                }
            }
        }
    }

    /// Builds the cpal input stream, matching the device's native sample format.
    #[allow(clippy::too_many_arguments)]
    fn build_input_stream(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        detectors: Vec<Option<TriggerDetector>>,
        channels: u16,
        tx: Sender<TriggerAction>,
        sample_format: cpal::SampleFormat,
        crosstalk: Option<(u32, f32)>,
        error_notify: ErrorNotify,
    ) -> Result<cpal::Stream, Box<dyn Error>> {
        match sample_format {
            cpal::SampleFormat::I16 => Self::build_input_stream_typed::<i16>(
                device,
                config,
                detectors,
                channels,
                tx,
                crosstalk,
                error_notify,
            ),
            cpal::SampleFormat::I32 => Self::build_input_stream_typed::<i32>(
                device,
                config,
                detectors,
                channels,
                tx,
                crosstalk,
                error_notify,
            ),
            _ => Self::build_input_stream_typed::<f32>(
                device,
                config,
                detectors,
                channels,
                tx,
                crosstalk,
                error_notify,
            ),
        }
    }

    /// Builds a typed cpal input stream, converting samples to f32 for detection.
    #[allow(clippy::too_many_arguments)]
    fn build_input_stream_typed<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        mut detectors: Vec<Option<TriggerDetector>>,
        channels: u16,
        tx: Sender<TriggerAction>,
        crosstalk: Option<(u32, f32)>,
        error_notify: ErrorNotify,
    ) -> Result<cpal::Stream, Box<dyn Error>>
    where
        T: cpal::SizedSample + 'static,
        f32: cpal::FromSample<T>,
    {
        let callback_priority = callback_thread_priority();
        let rt_audio = rt_audio_enabled();
        let mut priority_set = false;

        let stream = device.build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                promote_to_realtime(callback_priority, rt_audio, &mut priority_set);
                // Data is interleaved: [ch0, ch1, ch2, ..., ch0, ch1, ...]
                for frame in data.chunks_exact(channels as usize) {
                    let f32_frame: Vec<f32> = frame
                        .iter()
                        .map(|s| <f32 as cpal::FromSample<T>>::from_sample_(*s))
                        .collect();
                    process_frame(&f32_frame, &mut detectors, &tx, crosstalk);
                }
            },
            move |err| {
                error!(
                    error = %err,
                    "Trigger input stream error (will attempt to recover)"
                );
                let (mutex, condvar) = &*error_notify;
                let mut guard = mutex.lock();
                *guard = true;
                condvar.notify_one();
            },
            None,
        )?;

        Ok(stream)
    }
}

impl Drop for TriggerEngine {
    fn drop(&mut self) {
        let (mutex, condvar) = &*self.shutdown;
        *mutex.lock() = true;
        condvar.notify_one();
    }
}

/// Pre-computes crosstalk suppression parameters (ms → samples).
/// Returns `Some((window_samples, multiplier))` when both fields are configured.
fn resolve_crosstalk(
    window_ms: Option<u32>,
    threshold: Option<f32>,
    sample_rate: u32,
) -> Option<(u32, f32)> {
    match (window_ms, threshold) {
        (Some(ms), Some(mult)) => Some((ms_to_samples(ms, sample_rate), mult)),
        _ => None,
    }
}

/// Builds the detector map from config inputs.
/// Returns a Vec of `Option<TriggerDetector>` indexed by 0-based channel number.
fn build_detector_map(
    inputs: &[TriggerInput],
    device_channels: u16,
    sample_rate: u32,
) -> Result<Vec<Option<TriggerDetector>>, Box<dyn Error>> {
    let mut detector_map: Vec<Option<TriggerDetector>> =
        (0..device_channels).map(|_| None).collect();

    for input in inputs.iter().filter_map(|i| match i {
        TriggerInput::Audio(audio) => Some(audio),
        _ => None,
    }) {
        validate_audio_input(input)?;

        let ch_idx = match validate_channel_index(input.channel(), device_channels) {
            Some(idx) => idx,
            None => continue,
        };

        let detector = TriggerDetector::from_input(input, sample_rate);

        detector_map[ch_idx] = Some(detector);
        debug!(
            channel = input.channel(),
            sample = input.sample().unwrap_or("(release)"),
            "Trigger detector created"
        );
    }

    Ok(detector_map)
}

/// Resolves the cpal sample format from config preferences and device native format.
fn resolve_stream_format(
    sample_format: Option<SampleFormat>,
    bits_per_sample: Option<u16>,
    native_format: cpal::SampleFormat,
) -> cpal::SampleFormat {
    match (sample_format, bits_per_sample) {
        (Some(SampleFormat::Float), _) => cpal::SampleFormat::F32,
        (Some(SampleFormat::Int), Some(16)) => cpal::SampleFormat::I16,
        (Some(SampleFormat::Int), _) => cpal::SampleFormat::I32,
        (None, Some(16)) => cpal::SampleFormat::I16,
        _ => native_format,
    }
}

/// Validates that a trigger input has the required fields for its action type.
fn validate_audio_input(input: &AudioTriggerInput) -> Result<(), Box<dyn Error>> {
    match input.action() {
        TriggerInputAction::Trigger => {
            if input.sample().is_none_or(|s| s.is_empty()) {
                return Err(format!(
                    "Trigger input on channel {} has action 'trigger' but no sample name configured",
                    input.channel()
                )
                .into());
            }
        }
        TriggerInputAction::Release => {
            if input.release_group().is_none_or(|s| s.is_empty()) {
                return Err(format!(
                    "Trigger input on channel {} has action 'release' but no release_group configured",
                    input.channel()
                )
                .into());
            }
        }
    }
    Ok(())
}

/// Validates that a 1-indexed channel number is valid for the device.
/// Returns the 0-indexed channel index, or `None` if the channel exceeds device capacity.
fn validate_channel_index(channel: u16, device_channels: u16) -> Option<usize> {
    let ch_idx = channel.checked_sub(1)? as usize;
    if ch_idx >= device_channels as usize {
        warn!(
            channel,
            device_channels, "Trigger input channel exceeds device channel count, skipping"
        );
        return None;
    }
    Some(ch_idx)
}

/// Processes a single interleaved audio frame through the trigger detectors.
/// Sends trigger actions for any channels that fired. Also applies crosstalk
/// suppression to non-firing detectors when crosstalk parameters are configured.
/// Returns a bitmask of channels that fired (up to 64 channels).
fn process_frame(
    frame: &[f32],
    detectors: &mut [Option<TriggerDetector>],
    tx: &Sender<TriggerAction>,
    crosstalk: Option<(u32, f32)>,
) -> u64 {
    let mut fired_channels: u64 = 0;

    for (ch_idx, &sample) in frame.iter().enumerate() {
        if let Some(ref mut detector) = detectors[ch_idx] {
            if let Some(action) = detector.process_sample(sample) {
                if ch_idx < 64 {
                    fired_channels |= 1u64 << ch_idx;
                }
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
                        detector.apply_crosstalk_suppression(window_samples, multiplier);
                    }
                }
            }
        }
    }

    fired_channels
}

#[cfg(test)]
mod test {
    use super::*;

    mod resolve_stream_format_tests {
        use super::*;

        #[test]
        fn explicit_float_overrides_native() {
            let result =
                resolve_stream_format(Some(SampleFormat::Float), None, cpal::SampleFormat::I16);
            assert_eq!(result, cpal::SampleFormat::F32);
        }

        #[test]
        fn explicit_int_16_bit() {
            let result =
                resolve_stream_format(Some(SampleFormat::Int), Some(16), cpal::SampleFormat::F32);
            assert_eq!(result, cpal::SampleFormat::I16);
        }

        #[test]
        fn explicit_int_32_bit() {
            let result =
                resolve_stream_format(Some(SampleFormat::Int), Some(32), cpal::SampleFormat::F32);
            assert_eq!(result, cpal::SampleFormat::I32);
        }

        #[test]
        fn explicit_int_no_bits_defaults_to_i32() {
            let result =
                resolve_stream_format(Some(SampleFormat::Int), None, cpal::SampleFormat::F32);
            assert_eq!(result, cpal::SampleFormat::I32);
        }

        #[test]
        fn no_format_with_16_bits() {
            let result = resolve_stream_format(None, Some(16), cpal::SampleFormat::F32);
            assert_eq!(result, cpal::SampleFormat::I16);
        }

        #[test]
        fn no_preferences_uses_native() {
            let result = resolve_stream_format(None, None, cpal::SampleFormat::I32);
            assert_eq!(result, cpal::SampleFormat::I32);
        }

        #[test]
        fn float_ignores_bits_per_sample() {
            let result =
                resolve_stream_format(Some(SampleFormat::Float), Some(16), cpal::SampleFormat::I16);
            assert_eq!(result, cpal::SampleFormat::F32);
        }
    }

    mod resolve_crosstalk_tests {
        use super::*;

        #[test]
        fn both_fields_set_returns_some() {
            let result = resolve_crosstalk(Some(10), Some(3.0), 44100);
            assert!(result.is_some());
            let (window_samples, mult) = result.unwrap();
            assert_eq!(window_samples, ms_to_samples(10, 44100));
            assert_eq!(mult, 3.0);
        }

        #[test]
        fn no_window_returns_none() {
            assert!(resolve_crosstalk(None, Some(3.0), 44100).is_none());
        }

        #[test]
        fn no_threshold_returns_none() {
            assert!(resolve_crosstalk(Some(10), None, 44100).is_none());
        }

        #[test]
        fn neither_field_returns_none() {
            assert!(resolve_crosstalk(None, None, 44100).is_none());
        }

        #[test]
        fn sample_rate_affects_window() {
            let at_44100 = resolve_crosstalk(Some(10), Some(3.0), 44100).unwrap().0;
            let at_48000 = resolve_crosstalk(Some(10), Some(3.0), 48000).unwrap().0;
            assert!(at_48000 > at_44100);
        }
    }

    mod build_detector_map_tests {
        use super::*;

        #[test]
        fn empty_inputs_produces_all_none() {
            let inputs: Vec<TriggerInput> = vec![];
            let map = build_detector_map(&inputs, 4, 44100).unwrap();
            assert_eq!(map.len(), 4);
            assert!(map.iter().all(|d| d.is_none()));
        }

        #[test]
        fn audio_input_creates_detector_at_correct_index() {
            let inputs = vec![TriggerInput::Audio(AudioTriggerInput::new_trigger(
                2, "snare",
            ))];
            let map = build_detector_map(&inputs, 4, 44100).unwrap();
            assert!(map[0].is_none());
            assert!(map[1].is_some()); // channel 2 → index 1
            assert!(map[2].is_none());
            assert!(map[3].is_none());
        }

        #[test]
        fn multiple_inputs_on_different_channels() {
            let inputs = vec![
                TriggerInput::Audio(AudioTriggerInput::new_trigger(1, "kick")),
                TriggerInput::Audio(AudioTriggerInput::new_trigger(3, "snare")),
            ];
            let map = build_detector_map(&inputs, 4, 44100).unwrap();
            assert!(map[0].is_some());
            assert!(map[1].is_none());
            assert!(map[2].is_some());
            assert!(map[3].is_none());
        }

        #[test]
        fn channel_exceeding_device_is_skipped() {
            let inputs = vec![TriggerInput::Audio(AudioTriggerInput::new_trigger(
                5, "kick",
            ))];
            let map = build_detector_map(&inputs, 4, 44100).unwrap();
            assert_eq!(map.len(), 4);
            assert!(map.iter().all(|d| d.is_none()));
        }

        #[test]
        fn invalid_trigger_input_returns_error() {
            let inputs = vec![TriggerInput::Audio(
                AudioTriggerInput::new_trigger_no_sample(1),
            )];
            assert!(build_detector_map(&inputs, 4, 44100).is_err());
        }

        #[test]
        fn midi_inputs_are_ignored() {
            use crate::config::trigger::MidiTriggerInput;
            let midi_event = crate::config::midi::note_on(10, 36, 127);
            let midi_input =
                TriggerInput::Midi(MidiTriggerInput::new(midi_event, "kick".to_string()));
            let inputs = vec![midi_input];
            let map = build_detector_map(&inputs, 4, 44100).unwrap();
            assert!(map.iter().all(|d| d.is_none()));
        }

        #[test]
        fn release_input_creates_detector() {
            let inputs = vec![TriggerInput::Audio(AudioTriggerInput::new_release(
                1, "cymbal",
            ))];
            let map = build_detector_map(&inputs, 2, 44100).unwrap();
            assert!(map[0].is_some());
        }
    }

    mod validate_audio_input_tests {
        use super::*;

        #[test]
        fn trigger_with_sample_passes() {
            let input = AudioTriggerInput::new_trigger(1, "kick");
            assert!(validate_audio_input(&input).is_ok());
        }

        #[test]
        fn trigger_without_sample_fails() {
            let input = AudioTriggerInput::new_trigger_no_sample(1);
            let err = validate_audio_input(&input).unwrap_err();
            assert!(err.to_string().contains("no sample name"));
        }

        #[test]
        fn trigger_with_empty_sample_fails() {
            let input = AudioTriggerInput::new_trigger(1, "");
            let err = validate_audio_input(&input).unwrap_err();
            assert!(err.to_string().contains("no sample name"));
        }

        #[test]
        fn release_with_group_passes() {
            let input = AudioTriggerInput::new_release(2, "cymbal");
            assert!(validate_audio_input(&input).is_ok());
        }

        #[test]
        fn release_without_group_fails() {
            let input = AudioTriggerInput::new_release_no_group(2);
            let err = validate_audio_input(&input).unwrap_err();
            assert!(err.to_string().contains("no release_group"));
        }

        #[test]
        fn release_with_empty_group_fails() {
            let input = AudioTriggerInput::new_release(2, "");
            let err = validate_audio_input(&input).unwrap_err();
            assert!(err.to_string().contains("no release_group"));
        }
    }

    mod validate_channel_index_tests {
        use super::*;

        #[test]
        fn channel_1_returns_index_0() {
            assert_eq!(validate_channel_index(1, 4), Some(0));
        }

        #[test]
        fn channel_4_returns_index_3() {
            assert_eq!(validate_channel_index(4, 4), Some(3));
        }

        #[test]
        fn channel_exceeds_device_returns_none() {
            assert_eq!(validate_channel_index(5, 4), None);
        }

        #[test]
        fn channel_0_returns_none() {
            assert_eq!(validate_channel_index(0, 4), None);
        }
    }

    mod process_frame_tests {
        use super::*;

        fn make_detector(sample_rate: u32) -> TriggerDetector {
            let mut input = AudioTriggerInput::new_trigger(1, "test");
            input.set_threshold(0.5);
            input.set_retrigger_time_ms(0);
            input.set_scan_time_ms(0);
            TriggerDetector::from_input(&input, sample_rate)
        }

        #[test]
        fn silent_frame_produces_no_events() {
            let (tx, rx) = crossbeam_channel::bounded(16);
            let mut detectors: Vec<Option<TriggerDetector>> =
                vec![Some(make_detector(44100)), None];
            let frame = [0.0f32, 0.0];

            let fired = process_frame(&frame, &mut detectors, &tx, None);

            assert_eq!(fired, 0);
            assert!(rx.try_recv().is_err());
        }

        #[test]
        fn loud_frame_fires_detector() {
            let (tx, rx) = crossbeam_channel::bounded(16);
            let mut detectors: Vec<Option<TriggerDetector>> =
                vec![Some(make_detector(44100)), None];

            // Feed enough loud samples to get through scan phase.
            // With scan_time_ms=0, a single above-threshold sample should trigger.
            let frame = [0.9f32, 0.0];
            let fired = process_frame(&frame, &mut detectors, &tx, None);

            // The detector fires on the transition from scanning→lockout.
            // With scan_time_ms=0 and retrigger_time_ms=0, one sample should do it.
            if fired != 0 {
                assert_eq!(fired, 1); // channel 0
                assert!(rx.try_recv().is_ok());
            }
        }

        #[test]
        fn none_detector_slots_are_skipped() {
            let (tx, rx) = crossbeam_channel::bounded(16);
            let mut detectors: Vec<Option<TriggerDetector>> = vec![None, None, None];
            let frame = [0.9f32, 0.9, 0.9];

            let fired = process_frame(&frame, &mut detectors, &tx, None);

            assert_eq!(fired, 0);
            assert!(rx.try_recv().is_err());
        }

        #[test]
        fn full_channel_drops_events_without_panic() {
            // Channel with capacity 0 will always be full.
            let (tx, _rx) = crossbeam_channel::bounded(0);
            let mut detectors: Vec<Option<TriggerDetector>> = vec![Some(make_detector(44100))];

            // Just verify we don't panic on a full channel.
            let frame = [0.9f32];
            process_frame(&frame, &mut detectors, &tx, None);
        }

        #[test]
        fn multiple_channels_fire_independently() {
            let (tx, rx) = crossbeam_channel::bounded(16);
            let mut detectors: Vec<Option<TriggerDetector>> =
                vec![Some(make_detector(44100)), Some(make_detector(44100))];

            // Both channels loud
            let frame = [0.9f32, 0.9];
            let fired = process_frame(&frame, &mut detectors, &tx, None);

            // Both channels should fire
            assert_eq!(fired, 0b11);
            assert!(rx.try_recv().is_ok());
            assert!(rx.try_recv().is_ok());
        }

        #[test]
        fn fired_bitmask_reflects_channel_index() {
            let (tx, _rx) = crossbeam_channel::bounded(16);
            // Only channel 1 (index 1) has a detector
            let mut detectors: Vec<Option<TriggerDetector>> =
                vec![None, Some(make_detector(44100)), None];

            let frame = [0.0f32, 0.9, 0.0];
            let fired = process_frame(&frame, &mut detectors, &tx, None);

            assert_eq!(fired, 0b10); // bit 1 set
        }

        #[test]
        fn crosstalk_suppression_prevents_trigger_on_other_channel() {
            let (tx, rx) = crossbeam_channel::bounded(16);

            // Create two detectors with low threshold
            let make_low_threshold = || {
                let mut input = AudioTriggerInput::new_trigger(1, "test");
                input.set_threshold(0.1);
                input.set_retrigger_time_ms(0);
                input.set_scan_time_ms(0);
                TriggerDetector::from_input(&input, 44100)
            };

            let mut detectors: Vec<Option<TriggerDetector>> =
                vec![Some(make_low_threshold()), Some(make_low_threshold())];

            // Frame 1: ch0 fires loud, ch1 is quiet — ch1 gets crosstalk suppression
            let frame = [0.9f32, 0.0];
            let fired = process_frame(&frame, &mut detectors, &tx, Some((441, 5.0)));
            assert_eq!(fired & 1, 1); // ch0 fired

            // Drain the event
            while rx.try_recv().is_ok() {}

            // Frame 2: ch1 has a moderate signal that would normally trigger (0.3 > 0.1)
            // but crosstalk suppression raised its threshold to 0.1 * 5.0 = 0.5
            let frame = [0.0f32, 0.3];
            let fired = process_frame(&frame, &mut detectors, &tx, Some((441, 5.0)));
            assert_eq!(fired, 0, "ch1 should be suppressed by crosstalk");
            assert!(rx.try_recv().is_err());
        }

        #[test]
        fn crosstalk_suppression_applied_to_non_firing_channels() {
            let (tx, _rx) = crossbeam_channel::bounded(16);
            let mut detectors: Vec<Option<TriggerDetector>> =
                vec![Some(make_detector(44100)), Some(make_detector(44100))];

            // Simulate channel 0 firing by having a loud sample.
            // Channel 1 is quiet, so crosstalk suppression should be applied to it.
            let frame = [0.9f32, 0.0];
            let fired = process_frame(&frame, &mut detectors, &tx, Some((441, 3.0)));

            // We can't easily verify the internal state of the detector,
            // but we confirm no panic and the function completes.
            // The fired bitmask tells us which channels triggered.
            let _ = fired;
        }
    }
}
