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

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde_json::json;

use super::super::server::WebUiState;
use crate::{audio, calibrate, midi};

/// GET /api/devices/audio — lists available audio devices.
pub(super) async fn get_audio_devices() -> impl IntoResponse {
    match tokio::task::spawn_blocking(|| audio::list_device_info().map_err(|e| e.to_string())).await
    {
        Ok(Ok(devices)) => (StatusCode::OK, Json(json!(devices))).into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("failed to list audio devices: {}", e)})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("task failed: {}", e)})),
        )
            .into_response(),
    }
}

/// GET /api/devices/midi — lists available MIDI devices.
pub(super) async fn get_midi_devices() -> impl IntoResponse {
    match tokio::task::spawn_blocking(|| midi::list_device_info().map_err(|e| e.to_string())).await
    {
        Ok(Ok(devices)) => (StatusCode::OK, Json(json!(devices))).into_response(),
        Ok(Err(e)) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("failed to list MIDI devices: {}", e)})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("task failed: {}", e)})),
        )
            .into_response(),
    }
}

// ---------------------------------------------------------------------------
// Calibration endpoints
// ---------------------------------------------------------------------------

/// Server-side state for an in-progress calibration session.
pub(crate) struct CalibrationSession {
    device: cpal::Device,
    stream_config: cpal::StreamConfig,
    stream_format: cpal::SampleFormat,
    num_device_channels: u16,
    target_channel: u16,
    sample_rate: u32,
    #[allow(dead_code)]
    device_name: String,
    noise_floor: calibrate::NoiseFloorStats,
    // Phase 2 state
    hit_buffer: Option<std::sync::Arc<calibrate::CaptureBuffer>>,
    hit_stream: Option<cpal::Stream>,
}

#[derive(serde::Deserialize)]
pub(super) struct CalibrateStartRequest {
    device: String,
    channel: u16,
    #[serde(default = "default_noise_duration")]
    duration: f32,
    sample_rate: Option<u32>,
    /// "int" or "float"
    sample_format: Option<String>,
    bits_per_sample: Option<u16>,
}

fn default_noise_duration() -> f32 {
    3.0
}

/// POST /api/calibrate/start — begins noise floor measurement.
///
/// Blocks for `duration` seconds while capturing audio, then returns
/// the noise floor stats for the target channel.
pub(super) async fn post_calibrate_start(
    State(state): State<WebUiState>,
    Json(body): Json<CalibrateStartRequest>,
) -> impl IntoResponse {
    use cpal::traits::StreamTrait;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    if body.channel < 1 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error": "channel must be >= 1"})),
        )
            .into_response();
    }

    let duration = body.duration.clamp(0.5, 30.0);
    let device_name = body.device.clone();
    let target_channel = body.channel;
    let sample_rate_opt = body.sample_rate;
    let sample_format_opt: Option<crate::audio::format::SampleFormat> =
        body.sample_format.as_deref().and_then(|s| s.parse().ok());
    let bits_per_sample_opt = body.bits_per_sample;

    let result = tokio::task::spawn_blocking(move || -> Result<_, String> {
        let device =
            audio::find_input_device(&device_name).map_err(|e| format!("Device not found: {e}"))?;

        let cal_config = calibrate::CalibrationConfig {
            device_name: device_name.clone(),
            sample_rate: sample_rate_opt,
            noise_floor_duration_secs: duration,
            sample_format: sample_format_opt,
            bits_per_sample: bits_per_sample_opt,
        };

        let (channels, sample_rate, stream_format) =
            calibrate::resolve_stream_params(&device, &cal_config)
                .map_err(|e| format!("Failed to resolve stream params: {e}"))?;

        if target_channel > channels {
            return Err(format!(
                "Channel {target_channel} exceeds device's {channels} channels"
            ));
        }

        let stream_config = cpal::StreamConfig {
            channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let expected_samples = (duration * sample_rate as f32) as usize + 1024;
        let buffer = Arc::new(calibrate::CaptureBuffer {
            channels: (0..channels)
                .map(|_| parking_lot::Mutex::new(Vec::with_capacity(expected_samples)))
                .collect(),
            active: AtomicBool::new(true),
        });

        let stream = calibrate::build_capture_stream(
            &device,
            &stream_config,
            buffer.clone(),
            channels,
            stream_format,
        )
        .map_err(|e| format!("Failed to build capture stream: {e}"))?;

        stream
            .play()
            .map_err(|e| format!("Failed to start stream: {e}"))?;
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
            device_name,
            noise_floor,
        ))
    })
    .await;

    match result {
        Ok(Ok((
            device,
            stream_config,
            stream_format,
            channels,
            sample_rate,
            device_name,
            noise_floor,
        ))) => {
            let response = json!({
                "peak": noise_floor.peak,
                "rms": noise_floor.rms,
                "low_freq_energy": noise_floor.low_freq_energy,
                "channel": target_channel,
                "sample_rate": sample_rate,
                "device_channels": channels,
            });

            let session = CalibrationSession {
                device,
                stream_config,
                stream_format,
                num_device_channels: channels,
                target_channel,
                sample_rate,
                device_name,
                noise_floor,
                hit_buffer: None,
                hit_stream: None,
            };

            // Replace any existing session
            *state.calibration.lock() = Some(session);

            (StatusCode::OK, Json(response)).into_response()
        }
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, Json(json!({"error": e}))).into_response(),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Task failed: {e}")})),
        )
            .into_response(),
    }
}

/// POST /api/calibrate/capture — starts hit capture phase.
pub(super) async fn post_calibrate_capture(State(state): State<WebUiState>) -> impl IntoResponse {
    use cpal::traits::StreamTrait;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let mut guard = state.calibration.lock();
    let session = match guard.as_mut() {
        Some(s) => s,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "No calibration session — call /calibrate/start first"})),
            )
                .into_response()
        }
    };

    // Stop any existing capture
    session.hit_stream = None;
    session.hit_buffer = None;

    let hit_capacity = (60.0 * session.sample_rate as f32) as usize;
    let buffer = Arc::new(calibrate::CaptureBuffer {
        channels: (0..session.num_device_channels)
            .map(|_| parking_lot::Mutex::new(Vec::with_capacity(hit_capacity)))
            .collect(),
        active: AtomicBool::new(true),
    });

    let stream = match calibrate::build_capture_stream(
        &session.device,
        &session.stream_config,
        buffer.clone(),
        session.num_device_channels,
        session.stream_format,
    ) {
        Ok(s) => s,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({"error": format!("Failed to build capture stream: {e}")})),
            )
                .into_response()
        }
    };

    if let Err(e) = stream.play() {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": format!("Failed to start capture: {e}")})),
        )
            .into_response();
    }

    session.hit_buffer = Some(buffer);
    session.hit_stream = Some(stream);

    (StatusCode::OK, Json(json!({"status": "capturing"}))).into_response()
}

/// POST /api/calibrate/stop — stops capture and returns calibration results.
pub(super) async fn post_calibrate_stop(State(state): State<WebUiState>) -> impl IntoResponse {
    use std::sync::atomic::Ordering;

    let mut guard = state.calibration.lock();
    let session = match guard.take() {
        Some(s) => s,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "No calibration session active"})),
            )
                .into_response()
        }
    };

    let hit_buffer = match session.hit_buffer {
        Some(b) => b,
        None => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "Capture not started — call /calibrate/capture first"})),
            )
                .into_response()
        }
    };

    // Stop the stream
    hit_buffer.active.store(false, Ordering::Relaxed);
    drop(session.hit_stream);

    let ch_idx = (session.target_channel - 1) as usize;
    let samples = std::mem::take(&mut *hit_buffer.channels[ch_idx].lock());

    let hits = calibrate::detect_hits(&samples, &session.noise_floor, session.sample_rate);
    let calibration = calibrate::derive_channel_params(
        session.target_channel,
        &session.noise_floor,
        &hits,
        session.sample_rate,
    );

    (StatusCode::OK, Json(json!(calibration))).into_response()
}

/// DELETE /api/calibrate — cancels any in-progress calibration session.
pub(super) async fn delete_calibrate(State(state): State<WebUiState>) -> impl IntoResponse {
    let mut guard = state.calibration.lock();
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
    (StatusCode::OK, Json(json!({"status": "cancelled"}))).into_response()
}
