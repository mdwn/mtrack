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
struct CaptureBuffer {
    channels: Vec<Mutex<Vec<f32>>>,
    active: AtomicBool,
}

/// Noise floor statistics for a single channel.
struct NoiseFloorStats {
    peak: f32,
    rms: f32,
    low_freq_energy: f32,
}

/// A detected hit envelope on a single channel.
struct HitEnvelope {
    peak_amplitude: f32,
    onset_sample: usize,
    peak_sample: usize,
    decay_sample: usize,
    ring_end_sample: Option<usize>,
}

/// Calibrated parameters for a single channel.
struct ChannelCalibration {
    channel: u16,
    threshold: f32,
    gain: f32,
    scan_time_ms: u32,
    retrigger_time_ms: u32,
    highpass_freq: Option<f32>,
    dynamic_threshold_decay_ms: Option<u32>,
    num_hits_detected: usize,
    noise_floor_peak: f32,
    max_hit_amplitude: f32,
}

/// Calibrated crosstalk parameters.
struct CrosstalkCalibration {
    crosstalk_window_ms: Option<u32>,
    crosstalk_threshold: Option<f32>,
}

/// Resolves stream parameters (channels, sample rate, format) from device and config.
fn resolve_stream_params(
    device: &cpal::Device,
    config: &CalibrationConfig,
) -> Result<(u16, u32, cpal::SampleFormat), Box<dyn Error>> {
    let max_device_channels = device
        .supported_input_configs()
        .map(|configs| configs.map(|c| c.channels()).max().unwrap_or(0))
        .unwrap_or(0);

    let channels = if max_device_channels > 0 {
        max_device_channels
    } else {
        device.default_input_config()?.channels()
    };

    let native_format = device
        .supported_input_configs()
        .ok()
        .and_then(|configs| {
            configs
                .filter(|c| c.channels() == channels)
                .map(|c| c.sample_format())
                .next()
        })
        .unwrap_or(cpal::SampleFormat::F32);

    let stream_format = match (config.sample_format, config.bits_per_sample) {
        (Some(SampleFormat::Float), _) => cpal::SampleFormat::F32,
        (Some(SampleFormat::Int), Some(16)) => cpal::SampleFormat::I16,
        (Some(SampleFormat::Int), _) => cpal::SampleFormat::I32,
        (None, Some(16)) => cpal::SampleFormat::I16,
        (None, Some(32)) => native_format,
        _ => native_format,
    };

    let default_config = device.default_input_config()?;
    let sample_rate = config.sample_rate.unwrap_or(default_config.sample_rate());

    Ok((channels, sample_rate, stream_format))
}

/// Builds a cpal input stream that captures samples into the buffer.
fn build_capture_stream(
    device: &cpal::Device,
    stream_config: &cpal::StreamConfig,
    buffer: Arc<CaptureBuffer>,
    num_channels: u16,
    sample_format: cpal::SampleFormat,
) -> Result<cpal::Stream, Box<dyn Error>> {
    match sample_format {
        cpal::SampleFormat::I16 => {
            build_capture_stream_typed::<i16>(device, stream_config, buffer, num_channels)
        }
        cpal::SampleFormat::I32 => {
            build_capture_stream_typed::<i32>(device, stream_config, buffer, num_channels)
        }
        _ => build_capture_stream_typed::<f32>(device, stream_config, buffer, num_channels),
    }
}

/// Typed capture stream builder — converts samples to f32 and pushes to CaptureBuffer.
fn build_capture_stream_typed<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: Arc<CaptureBuffer>,
    num_channels: u16,
) -> Result<cpal::Stream, Box<dyn Error>>
where
    T: cpal::SizedSample + 'static,
    f32: cpal::FromSample<T>,
{
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            if !buffer.active.load(Ordering::Relaxed) {
                return;
            }
            let nc = num_channels as usize;
            for frame in data.chunks_exact(nc) {
                for (ch_idx, raw_sample) in frame.iter().enumerate() {
                    let sample: f32 = <f32 as cpal::FromSample<T>>::from_sample_(*raw_sample);
                    buffer.channels[ch_idx].lock().push(sample);
                }
            }
        },
        move |err| {
            eprintln!("Capture stream error: {}", err);
        },
        None,
    )?;
    Ok(stream)
}

/// Analyzes the noise floor from captured samples.
fn analyze_noise_floor(samples: &[f32], sample_rate: u32) -> NoiseFloorStats {
    if samples.is_empty() {
        return NoiseFloorStats {
            peak: 0.0,
            rms: 0.0,
            low_freq_energy: 0.0,
        };
    }

    let peak = samples.iter().map(|s| s.abs()).fold(0.0f32, f32::max);

    let sum_sq: f64 = samples.iter().map(|s| (*s as f64) * (*s as f64)).sum();
    let rms = (sum_sq / samples.len() as f64).sqrt() as f32;

    // Estimate low-frequency energy using a simple box-filter lowpass.
    // Window size targets ~100 Hz cutoff.
    let window_size = (sample_rate as usize / 100).max(1);
    let low_freq_energy = if samples.len() >= window_size {
        let mut running_sum: f64 = samples[..window_size].iter().map(|s| *s as f64).sum();
        let mut lf_sum_sq: f64 = 0.0;
        let count = samples.len() - window_size + 1;
        for i in 0..count {
            let avg = (running_sum / window_size as f64) as f32;
            lf_sum_sq += (avg as f64) * (avg as f64);
            if i + window_size < samples.len() {
                running_sum += samples[i + window_size] as f64;
                running_sum -= samples[i] as f64;
            }
        }
        (lf_sum_sq / count as f64).sqrt() as f32
    } else {
        rms
    };

    NoiseFloorStats {
        peak,
        rms,
        low_freq_energy,
    }
}

/// Computes the detection threshold from a noise floor measurement.
/// Set at 5x noise peak with a minimum of 0.005 to avoid triggering on silence.
fn detection_threshold(noise_floor: &NoiseFloorStats) -> f32 {
    (noise_floor.peak * 5.0).max(0.005)
}

/// Detects hit envelopes from captured samples.
///
/// Uses a holdoff approach: after finding a peak, the signal must stay below
/// threshold for `holdoff_samples` consecutive samples before the hit is
/// considered finished. This prevents oscillating/ringing signals from being
/// split into multiple spurious hits.
fn detect_hits(
    samples: &[f32],
    noise_floor: &NoiseFloorStats,
    sample_rate: u32,
) -> Vec<HitEnvelope> {
    let threshold = detection_threshold(noise_floor);
    // Holdoff: signal must stay below threshold for this many consecutive
    // samples before we consider the transient over. 50ms is long enough to
    // ride out piezo ringing and mic oscillation.
    let holdoff_samples = (sample_rate as f64 * 0.050) as usize; // 50ms

    let mut hits = Vec::new();
    let mut i = 0;

    while i < samples.len() {
        let abs_val = samples[i].abs();
        if abs_val >= threshold {
            // Found onset
            let onset_sample = i;
            let mut peak_amplitude: f32 = abs_val;
            let mut peak_sample = i;

            // Walk forward through the entire transient envelope.
            // Track the peak and find the decay point — the first sample where
            // the signal stays below threshold for `holdoff_samples` in a row.
            let mut consecutive_below: usize = 0;
            let mut decay_sample = i;
            i += 1;
            while i < samples.len() {
                let v = samples[i].abs();
                if v > peak_amplitude {
                    peak_amplitude = v;
                    peak_sample = i;
                }
                if v < threshold {
                    if consecutive_below == 0 {
                        decay_sample = i; // first sample below threshold
                    }
                    consecutive_below += 1;
                    if consecutive_below >= holdoff_samples {
                        break;
                    }
                } else {
                    consecutive_below = 0;
                }
                i += 1;
            }

            // If we never got a sustained dip, decay_sample is the last position
            if consecutive_below < holdoff_samples {
                decay_sample = if i >= samples.len() {
                    samples.len() - 1
                } else {
                    i
                };
            }

            // Find ring end: signal must stay below noise floor peak for
            // holdoff_samples consecutive samples.
            let ring_threshold = noise_floor.peak.max(0.001);
            let mut ring_end_sample = None;
            let mut j = decay_sample;
            let mut ring_consecutive: usize = 0;
            while j < samples.len() {
                if samples[j].abs() < ring_threshold {
                    ring_consecutive += 1;
                    if ring_consecutive >= holdoff_samples {
                        ring_end_sample = Some(j - holdoff_samples + 1);
                        break;
                    }
                } else {
                    ring_consecutive = 0;
                }
                j += 1;
            }

            hits.push(HitEnvelope {
                peak_amplitude,
                onset_sample,
                peak_sample,
                decay_sample,
                ring_end_sample,
            });

            // Resume scanning from whichever is furthest: end of decay holdoff,
            // or end of ring detection. This ensures we skip the entire transient.
            let resume_from = ring_end_sample
                .map(|re| re + holdoff_samples)
                .unwrap_or(decay_sample + holdoff_samples);
            i = resume_from.max(i);
        } else {
            i += 1;
        }
    }

    hits
}

/// Derives calibration parameters for a channel from its noise floor and detected hits.
fn derive_channel_params(
    channel: u16,
    noise_floor: &NoiseFloorStats,
    hits: &[HitEnvelope],
    sample_rate: u32,
) -> ChannelCalibration {
    let threshold = detection_threshold(noise_floor);

    let max_hit_amplitude = hits.iter().map(|h| h.peak_amplitude).fold(0.0f32, f32::max);

    let gain = if max_hit_amplitude > 0.0 {
        (0.95 / max_hit_amplitude).clamp(0.1, 50.0)
    } else {
        1.0
    };

    // Scan time: median attack time (onset→peak), converted to ms
    let scan_time_ms = if !hits.is_empty() {
        let mut attack_times: Vec<f32> = hits
            .iter()
            .map(|h| {
                let samples = (h.peak_sample - h.onset_sample) as f32;
                samples / sample_rate as f32 * 1000.0
            })
            .collect();
        attack_times.sort_unstable_by(f32::total_cmp);
        let median = attack_times[attack_times.len() / 2];
        (median.ceil() as u32).max(1)
    } else {
        5
    };

    // Retrigger time: median decay time (peak→below threshold) × 1.2 safety margin
    let retrigger_time_ms = if !hits.is_empty() {
        let mut decay_times: Vec<f32> = hits
            .iter()
            .map(|h| {
                let samples = h.decay_sample.saturating_sub(h.peak_sample) as f32;
                samples / sample_rate as f32 * 1000.0
            })
            .collect();
        decay_times.sort_unstable_by(f32::total_cmp);
        let median = decay_times[decay_times.len() / 2];
        ((median * 1.2).ceil() as u32).max(5)
    } else {
        30
    };

    // High-pass filter: enable if low-freq energy is significant relative to RMS
    let highpass_freq =
        if noise_floor.rms > 0.0 && noise_floor.low_freq_energy / noise_floor.rms > 0.5 {
            Some(80.0)
        } else {
            None
        };

    // Dynamic threshold decay: median ringing duration if > 5ms
    let dynamic_threshold_decay_ms = if !hits.is_empty() {
        let ring_durations: Vec<f32> = hits
            .iter()
            .filter_map(|h| {
                h.ring_end_sample.map(|re| {
                    let samples = (re - h.decay_sample) as f32;
                    samples / sample_rate as f32 * 1000.0
                })
            })
            .collect();
        if !ring_durations.is_empty() {
            let mut sorted = ring_durations;
            sorted.sort_unstable_by(f32::total_cmp);
            let median = sorted[sorted.len() / 2];
            if median > 5.0 {
                Some(median.ceil() as u32)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    ChannelCalibration {
        channel,
        threshold,
        gain,
        scan_time_ms,
        retrigger_time_ms,
        highpass_freq,
        dynamic_threshold_decay_ms,
        num_hits_detected: hits.len(),
        noise_floor_peak: noise_floor.peak,
        max_hit_amplitude,
    }
}

/// Analyzes crosstalk between channels.
fn analyze_crosstalk(
    all_samples: &[Vec<f32>],
    all_hits: &[Vec<HitEnvelope>],
    noise_floors: &[NoiseFloorStats],
    sample_rate: u32,
) -> CrosstalkCalibration {
    let window_samples = ((5.0 / 1000.0) * sample_rate as f64).ceil() as usize;
    let mut max_offset: usize = 0;
    let mut max_ratio: f32 = 0.0;
    let mut found_crosstalk = false;

    for (ch, hits) in all_hits.iter().enumerate() {
        for hit in hits {
            let center = hit.peak_sample;
            // Check other channels for spikes in ±window_samples around this hit
            for (other_ch, other_samples) in all_samples.iter().enumerate() {
                if other_ch == ch {
                    continue;
                }
                let other_noise = noise_floors[other_ch].peak.max(0.001);
                let crosstalk_detect_threshold = other_noise * 3.0;

                let start = center.saturating_sub(window_samples);
                let end = (center + window_samples).min(other_samples.len());

                for (idx, sample) in other_samples.iter().enumerate().take(end).skip(start) {
                    let v = sample.abs();
                    if v > crosstalk_detect_threshold {
                        found_crosstalk = true;
                        max_offset = max_offset.max(idx.abs_diff(center));
                        let ratio = v / other_noise;
                        max_ratio = max_ratio.max(ratio);
                    }
                }
            }
        }
    }

    if found_crosstalk {
        let window_ms =
            ((max_offset as f64 / sample_rate as f64 * 1000.0).ceil() as u32 + 1).max(2);
        let threshold = (max_ratio * 1.5).max(2.0);
        CrosstalkCalibration {
            crosstalk_window_ms: Some(window_ms),
            crosstalk_threshold: Some(threshold),
        }
    } else {
        CrosstalkCalibration {
            crosstalk_window_ms: None,
            crosstalk_threshold: None,
        }
    }
}

/// Writes the calibration results as YAML to stdout.
fn write_yaml(
    device_name: &str,
    sample_rate: u32,
    calibrations: &[ChannelCalibration],
    crosstalk: &CrosstalkCalibration,
) {
    println!("# Auto-calibrated trigger configuration");
    println!("# Generated by: mtrack calibrate-triggers");
    println!("trigger:");
    println!("  device: \"{}\"", device_name);
    println!("  sample_rate: {}", sample_rate);

    if let (Some(window), Some(thresh)) =
        (crosstalk.crosstalk_window_ms, crosstalk.crosstalk_threshold)
    {
        println!("  crosstalk_window_ms: {}", window);
        println!("  crosstalk_threshold: {:.1}", thresh);
    }

    println!("  inputs:");
    for cal in calibrations {
        println!("    - channel: {}", cal.channel);
        println!("      # sample: \"TODO\"");
        println!("      threshold: {:.4}", cal.threshold);
        println!("      gain: {:.2}", cal.gain);
        println!("      scan_time_ms: {}", cal.scan_time_ms);
        println!("      retrigger_time_ms: {}", cal.retrigger_time_ms);
        if let Some(freq) = cal.highpass_freq {
            println!("      highpass_freq: {:.1}", freq);
        }
        if let Some(decay) = cal.dynamic_threshold_decay_ms {
            println!("      dynamic_threshold_decay_ms: {}", decay);
        }
        println!(
            "      # Detected {} hits, noise floor peak: {:.6}, max hit: {:.4}",
            cal.num_hits_detected, cal.noise_floor_peak, cal.max_hit_amplitude
        );
    }
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

    let stream = build_capture_stream(
        &device,
        &stream_config,
        buffer.clone(),
        channels,
        stream_format,
    )?;
    stream.play()?;

    std::thread::sleep(std::time::Duration::from_secs_f32(
        config.noise_floor_duration_secs,
    ));

    buffer.active.store(false, Ordering::Relaxed);
    drop(stream);

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

    let crosstalk = analyze_crosstalk(&hit_samples, &all_hits, &noise_floors, sample_rate);

    eprintln!();
    write_yaml(&config.device_name, sample_rate, &calibrations, &crosstalk);

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Generates a synthetic transient signal for testing.
    fn make_transient(
        noise_level: f32,
        peak: f32,
        attack_samples: usize,
        decay_samples: usize,
        total_samples: usize,
        onset: usize,
    ) -> Vec<f32> {
        let mut samples = vec![0.0; total_samples];

        // Fill with noise
        for (i, s) in samples.iter_mut().enumerate() {
            // Simple deterministic "noise" pattern
            *s = noise_level * ((i as f32 * 0.7).sin() * 0.5 + (i as f32 * 1.3).cos() * 0.5);
        }

        // Add transient
        if onset < total_samples {
            // Attack ramp
            for j in 0..attack_samples.min(total_samples - onset) {
                let t = j as f32 / attack_samples as f32;
                let idx = onset + j;
                if idx < total_samples {
                    samples[idx] = peak * t;
                }
            }

            // Decay ramp
            let peak_pos = onset + attack_samples;
            for j in 0..decay_samples {
                let t = 1.0 - (j as f32 / decay_samples as f32);
                let idx = peak_pos + j;
                if idx < total_samples {
                    samples[idx] = peak * t;
                }
            }
        }

        samples
    }

    #[test]
    fn test_analyze_noise_floor_silence() {
        let samples = vec![0.0; 44100];
        let stats = analyze_noise_floor(&samples, 44100);
        assert_eq!(stats.peak, 0.0);
        assert_eq!(stats.rms, 0.0);
        assert_eq!(stats.low_freq_energy, 0.0);
    }

    #[test]
    fn test_analyze_noise_floor_with_noise() {
        let samples: Vec<f32> = (0..44100).map(|i| 0.002 * (i as f32 * 0.1).sin()).collect();
        let stats = analyze_noise_floor(&samples, 44100);
        assert!(stats.peak > 0.0 && stats.peak <= 0.002);
        assert!(stats.rms > 0.0 && stats.rms < stats.peak);
    }

    #[test]
    fn test_detect_hits_single_hit() {
        let samples = make_transient(0.001, 0.8, 50, 500, 44100, 5000);
        let nf = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0003,
        };
        let hits = detect_hits(&samples, &nf, 44100);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].peak_amplitude > 0.7);
        assert!(hits[0].onset_sample >= 5000);
        assert!(hits[0].peak_sample > hits[0].onset_sample);
    }

    #[test]
    fn test_detect_hits_multiple_hits() {
        let sample_rate = 44100;
        let total = sample_rate * 3; // 3 seconds
        let mut samples = vec![0.0f32; total];

        // Add noise
        for (i, s) in samples.iter_mut().enumerate() {
            *s = 0.001 * (i as f32 * 0.7).sin();
        }

        // Add 3 transients well-separated
        for &onset in &[10000, 50000, 90000] {
            for j in 0..50 {
                let idx = onset + j;
                if idx < total {
                    samples[idx] = 0.7 * (j as f32 / 50.0);
                }
            }
            for j in 0..2000 {
                let idx = onset + 50 + j;
                if idx < total {
                    samples[idx] = 0.7 * (1.0 - j as f32 / 2000.0);
                }
            }
        }

        let nf = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0003,
        };
        let hits = detect_hits(&samples, &nf, sample_rate as u32);
        assert_eq!(hits.len(), 3);
    }

    #[test]
    fn test_detect_hits_no_hits() {
        let samples: Vec<f32> = (0..44100).map(|i| 0.001 * (i as f32 * 0.3).sin()).collect();
        let nf = NoiseFloorStats {
            peak: 0.003,
            rms: 0.002,
            low_freq_energy: 0.001,
        };
        let hits = detect_hits(&samples, &nf, 44100);
        assert!(hits.is_empty());
    }

    #[test]
    fn test_detect_hits_with_ringing() {
        let sample_rate: u32 = 44100;
        let total = sample_rate as usize * 2;
        let noise = 0.001;

        // Build a signal: noise + transient + slow decay (ringing)
        let mut samples = vec![0.0f32; total];
        for (i, s) in samples.iter_mut().enumerate() {
            *s = noise * (i as f32 * 0.7).sin();
        }

        // Transient at sample 5000
        let onset = 5000;
        let peak = 0.8;
        let attack = 30;
        let decay = 200;
        let ring_duration = 4000; // ~90ms of ringing

        for j in 0..attack {
            let idx = onset + j;
            samples[idx] = peak * (j as f32 / attack as f32);
        }
        // Quick decay to threshold-ish level
        let peak_pos = onset + attack;
        for j in 0..decay {
            let idx = peak_pos + j;
            if idx < total {
                samples[idx] = peak * (1.0 - j as f32 / decay as f32);
            }
        }
        // Ringing: slowly decaying above noise floor
        let ring_start = peak_pos + decay;
        for j in 0..ring_duration {
            let idx = ring_start + j;
            if idx < total {
                // Decay from ~threshold level down to noise
                let ring_level =
                    noise * 5.0 * (1.0 - j as f32 / ring_duration as f32) + noise * 0.5;
                samples[idx] = ring_level * (j as f32 * 2.0).sin();
            }
        }

        let nf = NoiseFloorStats {
            peak: noise,
            rms: noise * 0.7,
            low_freq_energy: noise * 0.3,
        };
        let hits = detect_hits(&samples, &nf, sample_rate);
        assert_eq!(hits.len(), 1);
        // Should have detected ringing
        assert!(hits[0].ring_end_sample.is_some());
    }

    #[test]
    fn test_derive_params_basic() {
        let nf = NoiseFloorStats {
            peak: 0.003,
            rms: 0.002,
            low_freq_energy: 0.0005,
        };
        let hits = vec![HitEnvelope {
            peak_amplitude: 0.72,
            onset_sample: 1000,
            peak_sample: 1100,
            decay_sample: 2200,
            ring_end_sample: None,
        }];
        let cal = derive_channel_params(1, &nf, &hits, 44100);
        assert_eq!(cal.channel, 1);
        // threshold = max(0.003 * 5, 0.005) = 0.015
        assert!((cal.threshold - 0.015).abs() < 0.001);
        // gain = 0.95 / 0.72 ≈ 1.319
        assert!((cal.gain - 1.319).abs() < 0.1);
        assert!(cal.scan_time_ms >= 1);
        assert!(cal.retrigger_time_ms >= 5);
        assert_eq!(cal.highpass_freq, None); // low_freq / rms = 0.25 < 0.5
    }

    #[test]
    fn test_derive_params_highpass() {
        let nf = NoiseFloorStats {
            peak: 0.003,
            rms: 0.002,
            low_freq_energy: 0.0015, // ratio = 0.75 > 0.5
        };
        let hits = vec![HitEnvelope {
            peak_amplitude: 0.5,
            onset_sample: 1000,
            peak_sample: 1050,
            decay_sample: 2000,
            ring_end_sample: None,
        }];
        let cal = derive_channel_params(1, &nf, &hits, 44100);
        assert_eq!(cal.highpass_freq, Some(80.0));
    }

    #[test]
    fn test_derive_params_dynamic_threshold() {
        let nf = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0002,
        };
        let hits = vec![HitEnvelope {
            peak_amplitude: 0.6,
            onset_sample: 1000,
            peak_sample: 1050,
            decay_sample: 1500,
            // ~23ms of ringing at 44100Hz
            ring_end_sample: Some(1500 + 1000),
        }];
        let cal = derive_channel_params(1, &nf, &hits, 44100);
        assert!(cal.dynamic_threshold_decay_ms.is_some());
        let decay = cal.dynamic_threshold_decay_ms.unwrap();
        assert!(decay > 5);
    }

    #[test]
    fn test_crosstalk_detected() {
        let sample_rate: u32 = 44100;
        let total = 44100;

        // Channel 0: has a hit at sample 5000
        let mut ch0 = vec![0.0f32; total];
        for j in 0..100 {
            let idx = 5000 + j;
            ch0[idx] = 0.8 * (1.0 - j as f32 / 100.0);
        }

        // Channel 1: has a correlated spike near sample 5000 (crosstalk)
        let mut ch1 = vec![0.0f32; total];
        for j in 0..50 {
            let idx = 5010 + j;
            if idx < total {
                ch1[idx] = 0.05 * (1.0 - j as f32 / 50.0);
            }
        }

        let nf0 = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0003,
        };
        let nf1 = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0003,
        };

        let hits0 = vec![HitEnvelope {
            peak_amplitude: 0.8,
            onset_sample: 5000,
            peak_sample: 5000,
            decay_sample: 5100,
            ring_end_sample: None,
        }];
        let hits1 = vec![];

        let all_samples = vec![ch0, ch1];
        let all_hits = vec![hits0, hits1];
        let noise_floors = vec![nf0, nf1];

        let ct = analyze_crosstalk(&all_samples, &all_hits, &noise_floors, sample_rate);
        assert!(ct.crosstalk_window_ms.is_some());
        assert!(ct.crosstalk_threshold.is_some());
        assert!(ct.crosstalk_threshold.unwrap() >= 2.0);
    }

    #[test]
    fn test_crosstalk_absent() {
        let total = 44100;

        // Channel 0: hit at sample 5000
        let mut ch0 = vec![0.0f32; total];
        for j in 0..100 {
            ch0[5000 + j] = 0.8 * (1.0 - j as f32 / 100.0);
        }

        // Channel 1: hit far away at sample 30000 (independent)
        let mut ch1 = vec![0.0f32; total];
        for j in 0..100 {
            ch1[30000 + j] = 0.6 * (1.0 - j as f32 / 100.0);
        }

        let nf = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0003,
        };

        let hits0 = vec![HitEnvelope {
            peak_amplitude: 0.8,
            onset_sample: 5000,
            peak_sample: 5000,
            decay_sample: 5100,
            ring_end_sample: None,
        }];
        let hits1 = vec![HitEnvelope {
            peak_amplitude: 0.6,
            onset_sample: 30000,
            peak_sample: 30000,
            decay_sample: 30100,
            ring_end_sample: None,
        }];

        let all_samples = vec![ch0, ch1];
        let all_hits = vec![hits0, hits1];
        let noise_floors = vec![
            NoiseFloorStats {
                peak: nf.peak,
                rms: nf.rms,
                low_freq_energy: nf.low_freq_energy,
            },
            NoiseFloorStats {
                peak: 0.001,
                rms: 0.0007,
                low_freq_energy: 0.0003,
            },
        ];

        let ct = analyze_crosstalk(&all_samples, &all_hits, &noise_floors, 44100);
        assert!(ct.crosstalk_window_ms.is_none());
        assert!(ct.crosstalk_threshold.is_none());
    }

    #[test]
    fn test_yaml_output() {
        let calibrations = vec![
            ChannelCalibration {
                channel: 1,
                threshold: 0.0160,
                gain: 1.32,
                scan_time_ms: 3,
                retrigger_time_ms: 25,
                highpass_freq: Some(80.0),
                dynamic_threshold_decay_ms: Some(40),
                num_hits_detected: 12,
                noise_floor_peak: 0.003200,
                max_hit_amplitude: 0.7200,
            },
            ChannelCalibration {
                channel: 3,
                threshold: 0.0080,
                gain: 1.58,
                scan_time_ms: 2,
                retrigger_time_ms: 20,
                highpass_freq: None,
                dynamic_threshold_decay_ms: None,
                num_hits_detected: 8,
                noise_floor_peak: 0.001600,
                max_hit_amplitude: 0.6000,
            },
        ];
        let crosstalk = CrosstalkCalibration {
            crosstalk_window_ms: Some(4),
            crosstalk_threshold: Some(2.8),
        };

        // Just verify it doesn't panic — actual output goes to stdout
        write_yaml("UltraLite-mk5", 44100, &calibrations, &crosstalk);
    }

    #[test]
    fn test_detect_hits_oscillating_signal_counts_as_one() {
        // Simulates what a condenser mic tap looks like: the signal oscillates
        // above and below threshold many times as it rings down.  Before the
        // holdoff fix this would produce dozens of spurious hits.
        let sample_rate: u32 = 44100;
        let total = sample_rate as usize * 2;
        let noise = 0.001;
        let mut samples = vec![0.0f32; total];

        // Background noise
        for (i, s) in samples.iter_mut().enumerate() {
            *s = noise * (i as f32 * 0.7).sin();
        }

        // A single tap at sample 5000: fast attack, then decaying oscillation
        // that repeatedly crosses the threshold (5 * noise = 0.005).
        let onset = 5000;
        let peak = 0.6;
        // Decaying sinusoid: amplitude * e^(-t/tau) * sin(freq * t)
        let decay_tau = 3000.0f32; // ~68ms time constant
        let osc_freq = 800.0; // Hz — fast oscillation
        let ring_len = 15000; // ~340ms of ringing
        for j in 0..ring_len {
            let idx = onset + j;
            if idx >= total {
                break;
            }
            let t = j as f32;
            let envelope = peak * (-t / decay_tau).exp();
            let osc = (2.0 * std::f32::consts::PI * osc_freq * t / sample_rate as f32).sin();
            samples[idx] = envelope * osc;
        }

        let nf = NoiseFloorStats {
            peak: noise,
            rms: noise * 0.7,
            low_freq_energy: noise * 0.3,
        };
        let hits = detect_hits(&samples, &nf, sample_rate);
        assert_eq!(
            hits.len(),
            1,
            "Oscillating decay from a single tap should be detected as 1 hit, got {}",
            hits.len()
        );
        assert!(hits[0].peak_amplitude > 0.4);
    }

    #[test]
    fn test_analyze_noise_floor_empty() {
        let stats = analyze_noise_floor(&[], 44100);
        assert_eq!(stats.peak, 0.0);
        assert_eq!(stats.rms, 0.0);
        assert_eq!(stats.low_freq_energy, 0.0);
    }

    #[test]
    fn test_analyze_noise_floor_short_samples() {
        // Fewer samples than window_size (44100/100 = 441), triggers line 206
        let samples: Vec<f32> = (0..200).map(|i| 0.002 * (i as f32 * 0.1).sin()).collect();
        let stats = analyze_noise_floor(&samples, 44100);
        assert!(stats.peak > 0.0);
        assert!(stats.rms > 0.0);
        // When samples < window_size, low_freq_energy should equal rms
        assert_eq!(stats.low_freq_energy, stats.rms);
    }

    #[test]
    fn test_detect_hits_hit_at_end_of_buffer() {
        // Hit placed near the very end so holdoff can't complete
        let total = 5000;
        let noise = 0.001;
        let mut samples = vec![0.0f32; total];
        for (i, s) in samples.iter_mut().enumerate() {
            *s = noise * (i as f32 * 0.7).sin();
        }
        // Place transient starting 100 samples from end
        let onset = total - 100;
        for j in 0..50 {
            let idx = onset + j;
            if idx < total {
                samples[idx] = 0.8 * (1.0 - j as f32 / 50.0);
            }
        }

        let nf = NoiseFloorStats {
            peak: noise,
            rms: noise * 0.7,
            low_freq_energy: noise * 0.3,
        };
        let hits = detect_hits(&samples, &nf, 44100);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].peak_amplitude > 0.5);
    }

    #[test]
    fn test_derive_channel_params_no_hits() {
        let nf = NoiseFloorStats {
            peak: 0.003,
            rms: 0.002,
            low_freq_energy: 0.0005,
        };
        let cal = derive_channel_params(1, &nf, &[], 44100);
        assert_eq!(cal.gain, 1.0);
        assert_eq!(cal.scan_time_ms, 5);
        assert_eq!(cal.retrigger_time_ms, 30);
        assert_eq!(cal.dynamic_threshold_decay_ms, None);
        assert_eq!(cal.num_hits_detected, 0);
    }

    #[test]
    fn test_derive_channel_params_hits_without_ring() {
        let nf = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0002,
        };
        let hits = vec![
            HitEnvelope {
                peak_amplitude: 0.5,
                onset_sample: 1000,
                peak_sample: 1050,
                decay_sample: 2000,
                ring_end_sample: None,
            },
            HitEnvelope {
                peak_amplitude: 0.6,
                onset_sample: 10000,
                peak_sample: 10040,
                decay_sample: 11000,
                ring_end_sample: None,
            },
        ];
        let cal = derive_channel_params(1, &nf, &hits, 44100);
        assert_eq!(cal.dynamic_threshold_decay_ms, None);
        assert_eq!(cal.num_hits_detected, 2);
    }

    #[test]
    fn test_derive_channel_params_short_ring() {
        let nf = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0002,
        };
        // Ring duration: (1510-1500)/44100*1000 ≈ 0.23ms, well under 5ms
        let hits = vec![HitEnvelope {
            peak_amplitude: 0.6,
            onset_sample: 1000,
            peak_sample: 1050,
            decay_sample: 1500,
            ring_end_sample: Some(1510),
        }];
        let cal = derive_channel_params(1, &nf, &hits, 44100);
        assert_eq!(cal.dynamic_threshold_decay_ms, None);
    }

    #[test]
    fn test_detection_threshold_minimum_floor() {
        // When noise peak is very low, threshold should clamp to 0.005
        let nf = NoiseFloorStats {
            peak: 0.0001,
            rms: 0.00005,
            low_freq_energy: 0.00002,
        };
        let t = detection_threshold(&nf);
        assert_eq!(t, 0.005); // 0.0001 * 5 = 0.0005 < 0.005, so clamp
    }

    #[test]
    fn test_detection_threshold_zero_noise() {
        let nf = NoiseFloorStats {
            peak: 0.0,
            rms: 0.0,
            low_freq_energy: 0.0,
        };
        let t = detection_threshold(&nf);
        assert_eq!(t, 0.005);
    }

    #[test]
    fn test_detection_threshold_high_noise() {
        let nf = NoiseFloorStats {
            peak: 0.01,
            rms: 0.007,
            low_freq_energy: 0.003,
        };
        let t = detection_threshold(&nf);
        assert!((t - 0.05).abs() < 0.001); // 0.01 * 5 = 0.05 > 0.005
    }

    #[test]
    fn test_analyze_noise_floor_constant_signal() {
        // All samples the same value
        let samples = vec![0.01; 1000];
        let stats = analyze_noise_floor(&samples, 44100);
        assert!((stats.peak - 0.01).abs() < 0.0001);
        assert!((stats.rms - 0.01).abs() < 0.0001);
    }

    #[test]
    fn test_analyze_noise_floor_single_sample() {
        let samples = vec![0.05];
        let stats = analyze_noise_floor(&samples, 44100);
        assert!((stats.peak - 0.05).abs() < 0.001);
        assert!((stats.rms - 0.05).abs() < 0.001);
        // 1 sample < window_size (441), so low_freq_energy == rms
        assert_eq!(stats.low_freq_energy, stats.rms);
    }

    #[test]
    fn test_analyze_noise_floor_low_sample_rate() {
        // With sample_rate=100, window_size = 100/100 = 1, so even a small
        // buffer takes the long-window branch.
        let samples: Vec<f32> = (0..50).map(|i| 0.003 * (i as f32 * 0.2).sin()).collect();
        let stats = analyze_noise_floor(&samples, 100);
        assert!(stats.peak > 0.0);
        assert!(stats.rms > 0.0);
        assert!(stats.low_freq_energy >= 0.0);
    }

    #[test]
    fn test_analyze_noise_floor_negative_values() {
        // Samples with negative values -- peak should be absolute max
        let samples: Vec<f32> = (0..1000).map(|i| -0.005 * (i as f32 * 0.3).sin()).collect();
        let stats = analyze_noise_floor(&samples, 44100);
        assert!(stats.peak > 0.0);
        assert!(stats.peak <= 0.005);
        assert!(stats.rms > 0.0);
    }

    #[test]
    fn test_detect_hits_empty_samples() {
        let nf = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0003,
        };
        let hits = detect_hits(&[], &nf, 44100);
        assert!(hits.is_empty());
    }

    #[test]
    fn test_detect_hits_all_below_threshold() {
        // All samples below the minimum threshold of 0.005
        let samples: Vec<f32> = (0..10000).map(|i| 0.004 * (i as f32 * 0.1).sin()).collect();
        let nf = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0003,
        };
        let hits = detect_hits(&samples, &nf, 44100);
        assert!(hits.is_empty());
    }

    #[test]
    fn test_detect_hits_decay_not_completed_mid_buffer() {
        // Hit where the signal stays above threshold most of the time with
        // brief dips, so holdoff never completes. Exercises the
        // consecutive_below < holdoff_samples fallback path.
        let sample_rate: u32 = 44100;
        let noise = 0.001;
        let threshold = (noise * 5.0f32).max(0.005);
        let onset = 100;
        let above_len = 3000;
        let total = onset + above_len + 10;
        let mut samples = vec![0.0f32; total];
        for (i, s) in samples.iter_mut().enumerate() {
            *s = noise * 0.1 * (i as f32 * 0.7).sin();
        }
        for j in 0..above_len {
            let idx = onset + j;
            if idx < total {
                let level = if j % 100 < 5 {
                    threshold * 0.5
                } else {
                    threshold * 2.0
                };
                samples[idx] = level;
            }
        }

        let nf = NoiseFloorStats {
            peak: noise,
            rms: noise * 0.7,
            low_freq_energy: noise * 0.3,
        };
        let hits = detect_hits(&samples, &nf, sample_rate);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn test_detect_hits_ring_end_not_found() {
        // Create a hit where ring never settles below noise floor peak
        let sample_rate: u32 = 44100;
        let holdoff = (sample_rate as f64 * 0.050) as usize;
        let noise = 0.001;
        let total = 10000;
        let mut samples = vec![0.0f32; total];
        for (i, s) in samples.iter_mut().enumerate() {
            *s = noise * 0.1 * (i as f32 * 0.7).sin();
        }
        let onset = 100;
        for j in 0..50 {
            let idx = onset + j;
            if idx < total {
                samples[idx] = 0.5 * (1.0 - j as f32 / 50.0);
            }
        }
        // After decay, keep signal above noise floor peak until end
        let decay_end = onset + 50 + holdoff;
        for idx in decay_end..total {
            samples[idx] = 0.002 * (idx as f32 * 3.0).sin();
        }

        let nf = NoiseFloorStats {
            peak: noise,
            rms: noise * 0.7,
            low_freq_energy: noise * 0.3,
        };
        let hits = detect_hits(&samples, &nf, sample_rate);
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn test_detect_hits_peak_tracking() {
        // Verify that the peak is correctly tracked when it occurs
        // later in the transient (not at onset).
        let sample_rate: u32 = 44100;
        let total = sample_rate as usize;
        let mut samples = vec![0.0f32; total];

        let onset = 1000;
        let ramp_len = 200;
        let peak_val = 0.9;
        for j in 0..ramp_len {
            let idx = onset + j;
            if idx < total {
                samples[idx] = peak_val * (j as f32 / ramp_len as f32);
            }
        }
        let peak_pos = onset + ramp_len;
        for j in 0..100 {
            let idx = peak_pos + j;
            if idx < total {
                samples[idx] = peak_val * (1.0 - j as f32 / 100.0);
            }
        }

        let nf = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0003,
        };
        let hits = detect_hits(&samples, &nf, sample_rate);
        assert_eq!(hits.len(), 1);
        assert!((hits[0].peak_amplitude - peak_val).abs() < 0.01);
        assert!(hits[0].peak_sample >= onset + ramp_len - 5);
        assert!(hits[0].peak_sample <= onset + ramp_len + 5);
    }

    #[test]
    fn test_derive_channel_params_gain_clamp_high() {
        // Very small amplitude should clamp gain to 50.0
        let nf = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0002,
        };
        let hits = vec![HitEnvelope {
            peak_amplitude: 0.01,
            onset_sample: 1000,
            peak_sample: 1050,
            decay_sample: 2000,
            ring_end_sample: None,
        }];
        let cal = derive_channel_params(1, &nf, &hits, 44100);
        assert_eq!(cal.gain, 50.0);
    }

    #[test]
    fn test_derive_channel_params_gain_clamp_low() {
        // Very large amplitude should clamp gain to 0.1
        let nf = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0002,
        };
        let hits = vec![HitEnvelope {
            peak_amplitude: 100.0,
            onset_sample: 1000,
            peak_sample: 1050,
            decay_sample: 2000,
            ring_end_sample: None,
        }];
        let cal = derive_channel_params(1, &nf, &hits, 44100);
        assert_eq!(cal.gain, 0.1);
    }

    #[test]
    fn test_derive_channel_params_multiple_hits_median() {
        let nf = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0002,
        };
        let hits = vec![
            HitEnvelope {
                peak_amplitude: 0.5,
                onset_sample: 1000,
                peak_sample: 1010,
                decay_sample: 2000,
                ring_end_sample: None,
            },
            HitEnvelope {
                peak_amplitude: 0.6,
                onset_sample: 10000,
                peak_sample: 10100,
                decay_sample: 11000,
                ring_end_sample: None,
            },
            HitEnvelope {
                peak_amplitude: 0.4,
                onset_sample: 20000,
                peak_sample: 20050,
                decay_sample: 21000,
                ring_end_sample: None,
            },
        ];
        let cal = derive_channel_params(1, &nf, &hits, 44100);
        assert!(cal.scan_time_ms >= 1);
        assert_eq!(cal.num_hits_detected, 3);
        assert!((cal.max_hit_amplitude - 0.6).abs() < 0.001);
    }

    #[test]
    fn test_derive_channel_params_zero_rms_no_highpass() {
        let nf = NoiseFloorStats {
            peak: 0.0,
            rms: 0.0,
            low_freq_energy: 0.0,
        };
        let cal = derive_channel_params(1, &nf, &[], 44100);
        assert_eq!(cal.highpass_freq, None);
    }

    #[test]
    fn test_derive_channel_params_multiple_hits_with_mixed_ring() {
        let nf = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0002,
        };
        let hits = vec![
            HitEnvelope {
                peak_amplitude: 0.5,
                onset_sample: 1000,
                peak_sample: 1050,
                decay_sample: 1500,
                ring_end_sample: Some(1500 + 1000),
            },
            HitEnvelope {
                peak_amplitude: 0.6,
                onset_sample: 10000,
                peak_sample: 10040,
                decay_sample: 10500,
                ring_end_sample: None,
            },
            HitEnvelope {
                peak_amplitude: 0.4,
                onset_sample: 20000,
                peak_sample: 20060,
                decay_sample: 20500,
                ring_end_sample: Some(20500 + 500),
            },
        ];
        let cal = derive_channel_params(1, &nf, &hits, 44100);
        assert!(cal.dynamic_threshold_decay_ms.is_some());
        assert_eq!(cal.num_hits_detected, 3);
    }

    #[test]
    fn test_derive_channel_params_retrigger_minimum() {
        let nf = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0002,
        };
        let hits = vec![HitEnvelope {
            peak_amplitude: 0.5,
            onset_sample: 1000,
            peak_sample: 1050,
            decay_sample: 1051,
            ring_end_sample: None,
        }];
        let cal = derive_channel_params(1, &nf, &hits, 44100);
        assert_eq!(cal.retrigger_time_ms, 5);
    }

    #[test]
    fn test_derive_channel_params_scan_time_minimum() {
        let nf = NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0002,
        };
        let hits = vec![HitEnvelope {
            peak_amplitude: 0.5,
            onset_sample: 1000,
            peak_sample: 1000,
            decay_sample: 2000,
            ring_end_sample: None,
        }];
        let cal = derive_channel_params(1, &nf, &hits, 44100);
        assert_eq!(cal.scan_time_ms, 1);
    }

    #[test]
    fn test_derive_channel_params_threshold_value() {
        let nf = NoiseFloorStats {
            peak: 0.002,
            rms: 0.001,
            low_freq_energy: 0.0005,
        };
        let cal = derive_channel_params(1, &nf, &[], 44100);
        assert!((cal.threshold - 0.01).abs() < 0.0001);
        assert_eq!(cal.noise_floor_peak, 0.002);
        assert_eq!(cal.max_hit_amplitude, 0.0);
    }

    #[test]
    fn test_crosstalk_single_channel() {
        let samples = vec![vec![0.0f32; 44100]];
        let hits = vec![vec![HitEnvelope {
            peak_amplitude: 0.8,
            onset_sample: 5000,
            peak_sample: 5000,
            decay_sample: 5100,
            ring_end_sample: None,
        }]];
        let noise_floors = vec![NoiseFloorStats {
            peak: 0.001,
            rms: 0.0007,
            low_freq_energy: 0.0003,
        }];
        let ct = analyze_crosstalk(&samples, &hits, &noise_floors, 44100);
        assert!(ct.crosstalk_window_ms.is_none());
        assert!(ct.crosstalk_threshold.is_none());
    }

    #[test]
    fn test_crosstalk_no_hits() {
        let samples = vec![vec![0.0f32; 44100], vec![0.0f32; 44100]];
        let hits: Vec<Vec<HitEnvelope>> = vec![vec![], vec![]];
        let noise_floors = vec![
            NoiseFloorStats {
                peak: 0.001,
                rms: 0.0007,
                low_freq_energy: 0.0003,
            },
            NoiseFloorStats {
                peak: 0.001,
                rms: 0.0007,
                low_freq_energy: 0.0003,
            },
        ];
        let ct = analyze_crosstalk(&samples, &hits, &noise_floors, 44100);
        assert!(ct.crosstalk_window_ms.is_none());
        assert!(ct.crosstalk_threshold.is_none());
    }

    #[test]
    fn test_crosstalk_three_channels() {
        let total = 44100;
        let mut ch0 = vec![0.0f32; total];
        let mut ch1 = vec![0.0f32; total];
        let mut ch2 = vec![0.0f32; total];

        for j in 0..100 {
            ch0[5000 + j] = 0.8 * (1.0 - j as f32 / 100.0);
        }
        for j in 0..30 {
            ch1[5005 + j] = 0.04 * (1.0 - j as f32 / 30.0);
        }
        for j in 0..20 {
            ch2[5008 + j] = 0.03 * (1.0 - j as f32 / 20.0);
        }

        let hits0 = vec![HitEnvelope {
            peak_amplitude: 0.8,
            onset_sample: 5000,
            peak_sample: 5000,
            decay_sample: 5100,
            ring_end_sample: None,
        }];

        let ct = analyze_crosstalk(
            &[ch0, ch1, ch2],
            &[hits0, vec![], vec![]],
            &[
                NoiseFloorStats {
                    peak: 0.001,
                    rms: 0.0007,
                    low_freq_energy: 0.0003,
                },
                NoiseFloorStats {
                    peak: 0.001,
                    rms: 0.0007,
                    low_freq_energy: 0.0003,
                },
                NoiseFloorStats {
                    peak: 0.001,
                    rms: 0.0007,
                    low_freq_energy: 0.0003,
                },
            ],
            44100,
        );
        assert!(ct.crosstalk_window_ms.is_some());
        assert!(ct.crosstalk_threshold.is_some());
    }

    #[test]
    fn test_crosstalk_window_minimum() {
        let total = 44100;
        let mut ch0 = vec![0.0f32; total];
        let mut ch1 = vec![0.0f32; total];

        for j in 0..100 {
            ch0[5000 + j] = 0.8 * (1.0 - j as f32 / 100.0);
        }
        ch1[5000] = 0.05;

        let hits0 = vec![HitEnvelope {
            peak_amplitude: 0.8,
            onset_sample: 5000,
            peak_sample: 5000,
            decay_sample: 5100,
            ring_end_sample: None,
        }];

        let ct = analyze_crosstalk(
            &[ch0, ch1],
            &[hits0, vec![]],
            &[
                NoiseFloorStats {
                    peak: 0.001,
                    rms: 0.0007,
                    low_freq_energy: 0.0003,
                },
                NoiseFloorStats {
                    peak: 0.001,
                    rms: 0.0007,
                    low_freq_energy: 0.0003,
                },
            ],
            44100,
        );
        assert!(ct.crosstalk_window_ms.is_some());
        assert!(ct.crosstalk_window_ms.unwrap() >= 2);
    }

    #[test]
    fn test_write_yaml_without_crosstalk() {
        let calibrations = vec![ChannelCalibration {
            channel: 1,
            threshold: 0.015,
            gain: 1.3,
            scan_time_ms: 3,
            retrigger_time_ms: 25,
            highpass_freq: None,
            dynamic_threshold_decay_ms: None,
            num_hits_detected: 5,
            noise_floor_peak: 0.003,
            max_hit_amplitude: 0.72,
        }];
        let crosstalk = CrosstalkCalibration {
            crosstalk_window_ms: None,
            crosstalk_threshold: None,
        };
        write_yaml("TestDevice", 44100, &calibrations, &crosstalk);
    }

    #[test]
    fn test_write_yaml_empty_calibrations() {
        let crosstalk = CrosstalkCalibration {
            crosstalk_window_ms: None,
            crosstalk_threshold: None,
        };
        write_yaml("EmptyDevice", 48000, &[], &crosstalk);
    }
}
