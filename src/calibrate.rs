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

//! Auto-calibration for audio trigger detection parameters.
//!
//! Measures the user's hardware (noise floor, hit characteristics) and
//! generates a ready-to-paste YAML trigger configuration.

mod analysis;
mod output;
mod stream;

pub use analysis::{analyze_noise_floor, derive_channel_params, detect_hits};
pub use stream::{build_capture_stream, resolve_stream_params};

use std::error::Error;
use std::io::{self, BufRead};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::audio::format::SampleFormat;
use cpal::traits::{DeviceTrait, StreamTrait};
use parking_lot::Mutex;

/// Configuration for a calibration run.
pub struct CalibrationConfig {
    pub device_name: String,
    pub sample_rate: Option<u32>,
    pub noise_floor_duration_secs: f32,
    pub sample_format: Option<SampleFormat>,
    pub bits_per_sample: Option<u16>,
}

/// Shared capture buffer for the cpal input callback.
pub struct CaptureBuffer {
    pub channels: Vec<Mutex<Vec<f32>>>,
    pub active: AtomicBool,
}

/// Noise floor statistics for a single channel.
#[derive(serde::Serialize)]
pub struct NoiseFloorStats {
    pub peak: f32,
    pub rms: f32,
    pub low_freq_energy: f32,
}

/// A detected hit envelope on a single channel.
pub struct HitEnvelope {
    pub peak_amplitude: f32,
    pub onset_sample: usize,
    pub peak_sample: usize,
    pub decay_sample: usize,
    pub ring_end_sample: Option<usize>,
}

/// Calibrated parameters for a single channel.
#[derive(serde::Serialize)]
pub struct ChannelCalibration {
    pub channel: u16,
    pub threshold: f32,
    pub gain: f32,
    pub scan_time_ms: u32,
    pub retrigger_time_ms: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub highpass_freq: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dynamic_threshold_decay_ms: Option<u32>,
    pub num_hits_detected: usize,
    pub noise_floor_peak: f32,
    pub max_hit_amplitude: f32,
}

/// Calibrated crosstalk parameters.
pub struct CrosstalkCalibration {
    pub(crate) crosstalk_window_ms: Option<u32>,
    pub(crate) crosstalk_threshold: Option<f32>,
}

/// Runs the calibration process.
pub fn run(config: CalibrationConfig) -> Result<(), Box<dyn Error>> {
    let device = crate::audio::find_input_device(&config.device_name)?;

    let device_id = device
        .id()
        .map(|id| id.to_string())
        .unwrap_or_else(|_| "unknown".to_string());
    eprintln!("Found input device: {}", device_id);

    let (channels, sample_rate, stream_format) = resolve_stream_params(&device, &config)?;
    eprintln!(
        "Stream config: {} channels, {}Hz, {:?}",
        channels, sample_rate, stream_format
    );

    let stream_config = cpal::StreamConfig {
        channels,
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };

    // Pre-allocate capacity for expected duration
    let expected_samples = (config.noise_floor_duration_secs * sample_rate as f32) as usize + 1024;

    // --- Phase 1: Noise floor ---
    eprintln!(
        "\nPhase 1: Measuring noise floor ({:.0}s) -- keep all pads silent...",
        config.noise_floor_duration_secs
    );

    let buffer = Arc::new(CaptureBuffer {
        channels: (0..channels)
            .map(|_| Mutex::new(Vec::with_capacity(expected_samples)))
            .collect(),
        active: AtomicBool::new(true),
    });

    let capture_stream = build_capture_stream(
        &device,
        &stream_config,
        buffer.clone(),
        channels,
        stream_format,
    )?;
    capture_stream.play()?;

    std::thread::sleep(std::time::Duration::from_secs_f32(
        config.noise_floor_duration_secs,
    ));

    buffer.active.store(false, Ordering::Relaxed);
    drop(capture_stream);

    let noise_samples: Vec<Vec<f32>> = buffer
        .channels
        .iter()
        .map(|ch| std::mem::take(&mut *ch.lock()))
        .collect();

    let noise_floors: Vec<NoiseFloorStats> = noise_samples
        .iter()
        .map(|s| analyze_noise_floor(s, sample_rate))
        .collect();

    eprintln!("\nNoise floor results:");
    for (i, nf) in noise_floors.iter().enumerate() {
        eprintln!(
            "  Channel {}: peak={:.6}, rms={:.6}",
            i + 1,
            nf.peak,
            nf.rms
        );
    }

    // --- Phase 2: Capture hits ---
    eprintln!("\nPhase 2: Hit each pad several times at varying velocities.");
    eprintln!("         Press Enter when done.");

    // Allocate generous buffer for hit capture (up to ~60 seconds)
    let hit_capacity = (60.0 * sample_rate as f32) as usize;
    let hit_buffer = Arc::new(CaptureBuffer {
        channels: (0..channels)
            .map(|_| Mutex::new(Vec::with_capacity(hit_capacity)))
            .collect(),
        active: AtomicBool::new(true),
    });

    let hit_stream = build_capture_stream(
        &device,
        &stream_config,
        hit_buffer.clone(),
        channels,
        stream_format,
    )?;
    hit_stream.play()?;

    // Wait for user to press Enter
    let stdin = io::stdin();
    let _ = stdin.lock().lines().next();

    hit_buffer.active.store(false, Ordering::Relaxed);
    drop(hit_stream);

    let hit_samples: Vec<Vec<f32>> = hit_buffer
        .channels
        .iter()
        .map(|ch| std::mem::take(&mut *ch.lock()))
        .collect();

    // --- Phase 3: Analysis ---
    eprintln!("\nPhase 3: Analyzing captured data...");

    let all_hits: Vec<Vec<HitEnvelope>> = hit_samples
        .iter()
        .zip(noise_floors.iter())
        .map(|(samples, nf)| detect_hits(samples, nf, sample_rate))
        .collect();

    let mut calibrations = Vec::new();
    for (i, (hits, nf)) in all_hits.iter().zip(noise_floors.iter()).enumerate() {
        if hits.is_empty() {
            continue;
        }
        let channel = (i + 1) as u16;
        eprintln!("  Channel {}: {} hits detected", channel, hits.len());
        calibrations.push(derive_channel_params(channel, nf, hits, sample_rate));
    }

    if calibrations.is_empty() {
        eprintln!("\nNo hits detected on any channel. Make sure your pads are connected and producing signal.");
        return Ok(());
    }

    let crosstalk =
        analysis::analyze_crosstalk(&hit_samples, &all_hits, &noise_floors, sample_rate);

    eprintln!();
    output::write_yaml(&config.device_name, sample_rate, &calibrations, &crosstalk);

    Ok(())
}
