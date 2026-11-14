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

//! Audio and MIDI devices, the audio probe and trigger calibration, as plain
//! functions: what `ConfigService`'s device RPCs call. No transport types
//! here, so a caller maps a [`DeviceError`] to its own refusal.

use std::fmt;

use crate::device_info::{AudioDeviceInfo, ChannelCalibration, MidiDeviceInfo};
use crate::player::Player;
use crate::webui::wire::devices::{AudioProbe, NoiseFloor, ProbeAudioRequest, ProbeKind};
use crate::{audio, calibrate, midi};

/// Why a device call was refused. The message is the caller's to show.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DeviceError {
    /// The request is not valid.
    Invalid(String),
    /// The device the request names is not there.
    NotFound(String),
    /// The player or the session is in a state the call does not fit:
    /// playing, or no calibration session.
    WrongState(String),
    /// The device could not be opened or started.
    Unavailable(String),
    /// A task failed.
    Internal(String),
}

impl fmt::Display for DeviceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceError::Invalid(m)
            | DeviceError::NotFound(m)
            | DeviceError::WrongState(m)
            | DeviceError::Unavailable(m)
            | DeviceError::Internal(m) => f.write_str(m),
        }
    }
}

/// Lists the available audio devices.
pub(crate) async fn list_audio_devices() -> Result<Vec<AudioDeviceInfo>, DeviceError> {
    match tokio::task::spawn_blocking(|| audio::list_device_info().map_err(|e| e.to_string())).await
    {
        Ok(Ok(devices)) => Ok(devices),
        Ok(Err(e)) => Err(DeviceError::Internal(format!(
            "failed to list audio devices: {e}"
        ))),
        Err(e) => Err(DeviceError::Internal(format!("task failed: {e}"))),
    }
}

/// Lists the available MIDI devices.
///
/// Lists none if the MIDI subsystem is unavailable (e.g. no ALSA on WSL2),
/// rather than failing.
pub(crate) async fn list_midi_devices() -> Result<Vec<MidiDeviceInfo>, DeviceError> {
    match tokio::task::spawn_blocking(|| midi::list_device_info().map_err(|e| e.to_string())).await
    {
        Ok(Ok(devices)) => Ok(devices),
        Ok(Err(e)) => {
            tracing::warn!("MIDI subsystem unavailable: {}", e);
            Ok(Vec::new())
        }
        Err(e) => Err(DeviceError::Internal(format!("task failed: {e}"))),
    }
}

/// Serializes device tests across every client.
///
/// Process-wide rather than per-connection because the thing being contended
/// for is the hardware, not the session.
static PROBE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Opens a device, confirms its callback runs, and closes it again.
///
/// The check the device picker cannot make on its own: listing a device proves
/// its name exists, not that the format will be accepted or that a callback
/// will ever fire. Silent by construction, so it is runnable with the PA hot.
///
/// A device already held by this player is reported from its live health
/// instead of being probed. It would refuse the second open, and answering
/// "could not be opened" for the device currently playing is worse than not
/// answering at all.
pub(crate) async fn probe_audio_device(
    player: &Player,
    body: ProbeAudioRequest,
) -> Result<AudioProbe, DeviceError> {
    let name = body.device.trim().to_string();
    if name.is_empty() {
        return Err(DeviceError::Invalid("device is required".to_string()));
    }

    if player.holds_audio_device(&name) {
        let health = player.hardware_status().audio_output;
        // No health means a device that cannot report one — a mock. It is open
        // and in use, which is all that can be said, and all a probe of it
        // would have said either.
        let ok = health.as_ref().is_none_or(|h| {
            matches!(
                h.status,
                crate::audio::health::OutputStatus::Healthy
                    | crate::audio::health::OutputStatus::Recovering
            )
        });
        return Ok(AudioProbe {
            outcome: ProbeKind::InUse,
            ok,
            reason: None,
            callbacks: health.as_ref().map(|h| h.callbacks),
            health: Some(health),
        });
    }

    // Opening a second interface mid-song risks an xrun on the one that is
    // playing. Deliberately after the in-use branch: asking about the device
    // currently playing is answered from health, opens nothing, and is exactly
    // the question worth asking when a rig goes quiet during a set.
    if player.is_playing().await {
        return Err(DeviceError::WrongState(
            "Cannot test another audio device during playback".to_string(),
        ));
    }

    // One test at a time. A probe opens a real interface, and two overlapping
    // opens of the same one make the second fail as busy — a wrong answer about
    // a working device, which is the failure this endpoint exists to avoid. The
    // button being disabled while a test runs only guards one tab.
    // Waits rather than refusing: a probe takes about two seconds at worst, and
    // a caller that waited and got a real answer is better served than one told
    // to try again.
    let _in_flight = PROBE_LOCK.lock().await;

    let mut audio = crate::config::Audio::new(&name);
    if let Some(rate) = body.sample_rate {
        audio = audio.with_sample_rate(rate);
    }
    if let Some(format) = body.sample_format.as_deref() {
        audio = audio.with_sample_format(format);
    }
    if let Some(bits) = body.bits_per_sample {
        audio = audio.with_bits_per_sample(bits);
    }
    if let Some(size) = body.buffer_size {
        audio = audio.with_buffer_size(size);
    }
    if let Some(sbs) = body.stream_buffer_size {
        audio = audio.with_stream_buffer_size(sbs);
    }

    // Blocking: the probe waits up to two seconds for a callback, and opens a
    // real device while it does.
    match tokio::task::spawn_blocking(move || audio::probe_device(audio)).await {
        Ok(outcome) => {
            let (kind, reason, callbacks) = match &outcome {
                audio::ProbeOutcome::Streaming { callbacks } => {
                    (ProbeKind::Streaming, None, Some(*callbacks))
                }
                audio::ProbeOutcome::OpenedButSilent => (ProbeKind::OpenedButSilent, None, None),
                audio::ProbeOutcome::CouldNotOpen(why) => {
                    (ProbeKind::CouldNotOpen, Some(why.clone()), None)
                }
                audio::ProbeOutcome::OpenedCannotReport => {
                    (ProbeKind::OpenedCannotReport, None, None)
                }
            };
            Ok(AudioProbe {
                outcome: kind,
                ok: outcome.is_ok(),
                reason: Some(reason),
                callbacks,
                health: None,
            })
        }
        Err(e) => Err(DeviceError::Internal(format!("task failed: {e}"))),
    }
}

// ---------------------------------------------------------------------------
// Calibration
// ---------------------------------------------------------------------------

/// Server-side state for an in-progress calibration session.
struct CalibrationSession {
    device: cpal::Device,
    stream_config: cpal::StreamConfig,
    stream_format: cpal::SampleFormat,
    num_device_channels: u16,
    target_channel: u16,
    sample_rate: u32,
    noise_floor: calibrate::NoiseFloorStats,
    // Phase 2 state
    hit_buffer: Option<std::sync::Arc<calibrate::CaptureBuffer>>,
    hit_stream: Option<cpal::Stream>,
}

/// The calibration session (at most one at a time). Process-wide like
/// [`PROBE_LOCK`], for the same reason: the thing contended for is the
/// hardware, and every transport and port serves the one player.
static CALIBRATION: parking_lot::Mutex<Option<CalibrationSession>> = parking_lot::Mutex::new(None);

/// What a calibration is asked to measure.
pub(crate) struct CalibrateStart {
    pub device: String,
    /// The device's channel, from 1.
    pub channel: u16,
    /// Seconds of noise to record; kept between 0.5 and 30.
    pub duration: f32,
    pub sample_rate: Option<u32>,
    /// "int" or "float".
    pub sample_format: Option<String>,
    pub bits_per_sample: Option<u16>,
}

/// How long a noise floor is recorded when the request names no time.
pub(crate) const DEFAULT_NOISE_DURATION: f32 = 3.0;

/// Begins noise floor measurement.
///
/// Takes `duration` seconds while capturing audio, then answers the noise
/// floor stats for the target channel and keeps the session for the capture.
/// Replaces any session already open.
pub(crate) async fn calibrate_start(body: CalibrateStart) -> Result<NoiseFloor, DeviceError> {
    use cpal::traits::StreamTrait;
    use std::sync::atomic::Ordering;
    use std::sync::Arc;

    if body.channel < 1 {
        return Err(DeviceError::Invalid("channel must be >= 1".to_string()));
    }

    // Explicit comparisons rather than `clamp`, which passes NaN through; every
    // comparison with NaN is false, so it lands on the shortest capture. The
    // duration sizes the capture buffers below, so it must be bounded.
    let duration = if body.duration > 30.0 {
        30.0
    } else if body.duration >= 0.5 {
        body.duration
    } else {
        0.5
    };
    let device_name = body.device.clone();
    let target_channel = body.channel;
    let sample_rate_opt = body.sample_rate;
    let sample_format_opt: Option<crate::audio::format::SampleFormat> =
        body.sample_format.as_deref().and_then(|s| s.parse().ok());
    let bits_per_sample_opt = body.bits_per_sample;

    let result = tokio::task::spawn_blocking(move || -> Result<_, DeviceError> {
        let device = audio::find_input_device(&device_name)
            .map_err(|e| DeviceError::NotFound(format!("Device not found: {e}")))?;

        let cal_config = calibrate::CalibrationConfig {
            device_name: device_name.clone(),
            sample_rate: sample_rate_opt,
            noise_floor_duration_secs: duration,
            sample_format: sample_format_opt,
            bits_per_sample: bits_per_sample_opt,
        };

        let (channels, sample_rate, stream_format) =
            calibrate::resolve_stream_params(&device, &cal_config).map_err(|e| {
                DeviceError::Invalid(format!("Failed to resolve stream params: {e}"))
            })?;

        if target_channel > channels {
            return Err(DeviceError::Invalid(format!(
                "Channel {target_channel} exceeds device's {channels} channels"
            )));
        }

        let stream_config = cpal::StreamConfig {
            channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let buffer = Arc::new(calibrate::CaptureBuffer::new(
            channels,
            duration,
            sample_rate,
        ));

        let stream = calibrate::build_capture_stream(
            &device,
            &stream_config,
            buffer.clone(),
            channels,
            stream_format,
        )
        .map_err(|e| DeviceError::Unavailable(format!("Failed to build capture stream: {e}")))?;

        stream
            .play()
            .map_err(|e| DeviceError::Unavailable(format!("Failed to start stream: {e}")))?;
        std::thread::sleep(std::time::Duration::from_secs_f32(duration));
        buffer.active.store(false, Ordering::Relaxed);
        drop(stream);

        // Extract target channel samples and compute noise floor
        let ch_idx = (target_channel - 1) as usize;
        let samples = std::mem::take(&mut *buffer.channels[ch_idx].lock());
        let noise_floor = calibrate::analyze_noise_floor(&samples, sample_rate);

        Ok((
            device,
            stream_config,
            stream_format,
            channels,
            sample_rate,
            noise_floor,
        ))
    })
    .await;

    match result {
        Ok(Ok((device, stream_config, stream_format, channels, sample_rate, noise_floor))) => {
            let response = NoiseFloor {
                peak: noise_floor.peak.into(),
                rms: noise_floor.rms.into(),
                low_freq_energy: noise_floor.low_freq_energy.into(),
                channel: target_channel,
                sample_rate,
                device_channels: channels,
            };

            let session = CalibrationSession {
                device,
                stream_config,
                stream_format,
                num_device_channels: channels,
                target_channel,
                sample_rate,
                noise_floor,
                hit_buffer: None,
                hit_stream: None,
            };

            // Replace any existing session
            *CALIBRATION.lock() = Some(session);

            Ok(response)
        }
        Ok(Err(e)) => Err(e),
        Err(e) => Err(DeviceError::Internal(format!("Task failed: {e}"))),
    }
}

/// Starts the hit capture phase of the open session.
pub(crate) fn calibrate_capture() -> Result<(), DeviceError> {
    use cpal::traits::StreamTrait;
    use std::sync::Arc;

    let mut guard = CALIBRATION.lock();
    let Some(session) = guard.as_mut() else {
        return Err(DeviceError::WrongState(
            "No calibration session — call StartCalibration first".to_string(),
        ));
    };

    // Stop any existing capture
    session.hit_stream = None;
    session.hit_buffer = None;

    let buffer = Arc::new(calibrate::CaptureBuffer::new(
        session.num_device_channels,
        60.0,
        session.sample_rate,
    ));

    let stream = calibrate::build_capture_stream(
        &session.device,
        &session.stream_config,
        buffer.clone(),
        session.num_device_channels,
        session.stream_format,
    )
    .map_err(|e| DeviceError::Unavailable(format!("Failed to build capture stream: {e}")))?;

    stream
        .play()
        .map_err(|e| DeviceError::Unavailable(format!("Failed to start capture: {e}")))?;

    session.hit_buffer = Some(buffer);
    session.hit_stream = Some(stream);
    Ok(())
}

/// Stops the capture, closes the session and answers the calibration derived
/// from it. The session is closed even when no capture had started.
pub(crate) fn calibrate_stop() -> Result<ChannelCalibration, DeviceError> {
    use std::sync::atomic::Ordering;

    let mut guard = CALIBRATION.lock();
    let Some(session) = guard.take() else {
        return Err(DeviceError::WrongState(
            "No calibration session active".to_string(),
        ));
    };

    let Some(hit_buffer) = session.hit_buffer else {
        return Err(DeviceError::WrongState(
            "Capture not started — call CaptureCalibration first".to_string(),
        ));
    };

    // Stop the stream
    hit_buffer.active.store(false, Ordering::Relaxed);
    drop(session.hit_stream);

    let ch_idx = (session.target_channel - 1) as usize;
    let samples = std::mem::take(&mut *hit_buffer.channels[ch_idx].lock());

    let hits = calibrate::detect_hits(&samples, &session.noise_floor, session.sample_rate);
    Ok(calibrate::derive_channel_params(
        session.target_channel,
        &session.noise_floor,
        &hits,
        session.sample_rate,
    ))
}

/// Cancels any in-progress calibration session.
pub(crate) fn calibrate_cancel() {
    let mut guard = CALIBRATION.lock();
    if let Some(mut session) = guard.take() {
        // Stop any active capture
        if let Some(ref buffer) = session.hit_buffer {
            buffer
                .active
                .store(false, std::sync::atomic::Ordering::Relaxed);
        }
        session.hit_stream = None;
        session.hit_buffer = None;
    }
}
