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
//! Captures the interface's real output and measures what is on it.
//!
//! This is what turns "mtrack reported it played" into "the signal was on the
//! physical channel it was supposed to be on". Everything here works on tones
//! at known frequencies, so a measurement can distinguish *which* track landed
//! on a channel rather than merely detecting that the channel was not silent.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use mtrack::calibrate::CaptureBuffer;
use parking_lot::Mutex;

use crate::capabilities::{AudioInput, Capabilities};

/// A tone is considered present on a channel when it is within this many dB of
/// that channel's strongest tone.
///
/// The measured patch on the harness rig shows roughly 66 dB between signal
/// and crosstalk, so 40 dB sits comfortably between "real signal" and "bleed
/// from the neighbouring channel" without being sensitive to the exact rig.
const RELATIVE_FLOOR_DB: f32 = -40.0;

/// Absolute magnitude below which a measurement is treated as silence,
/// regardless of what else is on the channel. Prevents a channel carrying only
/// noise from reporting its loudest noise bin as a detected tone.
const ABSOLUTE_FLOOR: f32 = 1e-4;

/// Magnitude of a single frequency within `samples`.
///
/// A Goertzel filter rather than an FFT: only a handful of known frequencies
/// matter, and this avoids both a dependency and the cost of computing every
/// bin. The result is normalized by length so windows of different durations
/// are comparable.
pub fn goertzel(samples: &[f32], sample_rate: u32, freq: f32) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }

    let n = samples.len();
    // Snap to the nearest exact bin so the tone sits at the filter's centre;
    // an off-bin frequency leaks into neighbours and reads low.
    let k = (0.5 + (n as f32 * freq) / sample_rate as f32) as usize;
    let omega = 2.0 * std::f32::consts::PI * k as f32 / n as f32;
    let coeff = 2.0 * omega.cos();

    let (mut s_prev, mut s_prev2) = (0.0f32, 0.0f32);
    for &sample in samples {
        let s = sample + coeff * s_prev - s_prev2;
        s_prev2 = s_prev;
        s_prev = s;
    }

    let power = s_prev2 * s_prev2 + s_prev * s_prev - coeff * s_prev * s_prev2;
    power.max(0.0).sqrt() / n as f32
}

/// Which of `tones` are present on `samples`, as `(index, magnitude)`.
///
/// Presence is judged relative to the strongest tone on this channel rather
/// than against a global threshold, because input channels differ in gain and
/// a fixed threshold would either miss quiet channels or accept crosstalk on
/// loud ones.
pub fn tones_present(samples: &[f32], sample_rate: u32, tones: &[f32]) -> Vec<(usize, f32)> {
    let mags: Vec<f32> = tones
        .iter()
        .map(|&f| goertzel(samples, sample_rate, f))
        .collect();

    let peak = mags.iter().copied().fold(0.0f32, f32::max);
    if peak < ABSOLUTE_FLOOR {
        return Vec::new();
    }

    let floor = peak * 10f32.powf(RELATIVE_FLOOR_DB / 20.0);
    mags.iter()
        .enumerate()
        .filter(|(_, &m)| m >= floor && m >= ABSOLUTE_FLOOR)
        .map(|(i, &m)| (i, m))
        .collect()
}

/// De-interleaves a capture into per-channel sample vectors.
pub fn deinterleave(samples: &[f32], channels: u16) -> Vec<Vec<f32>> {
    let channels = channels.max(1) as usize;
    let mut out = vec![Vec::with_capacity(samples.len() / channels); channels];
    for frame in samples.chunks_exact(channels) {
        for (ch, &sample) in frame.iter().enumerate() {
            out[ch].push(sample);
        }
    }
    out
}

/// Records from the configured input device for a fixed duration.
pub struct Capture {
    /// Per-channel samples.
    channels: Vec<Vec<f32>>,
    sample_rate: u32,
}

impl Capture {
    /// The samples recorded on a 1-based input channel.
    pub fn channel(&self, input_channel: u16) -> &[f32] {
        self.channels
            .get(input_channel.saturating_sub(1) as usize)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Peak absolute sample on a 1-based input channel.
    pub fn peak(&self, input_channel: u16) -> f32 {
        self.channel(input_channel)
            .iter()
            .fold(0.0f32, |acc, &s| acc.max(s.abs()))
    }

    /// The steady middle of the capture, skipping `skip` at the start.
    ///
    /// Stream startup and any fade-in live at the edges; measuring across them
    /// would spread energy outside the tone's bin.
    pub fn steady(&self, input_channel: u16, skip: Duration, take: Duration) -> &[f32] {
        let samples = self.channel(input_channel);
        let start = (skip.as_secs_f32() * self.sample_rate as f32) as usize;
        let len = (take.as_secs_f32() * self.sample_rate as f32) as usize;
        if start >= samples.len() {
            return &[];
        }
        let end = (start + len).min(samples.len());
        &samples[start..end]
    }

    /// Records for `duration` from the suite's chosen input device.
    ///
    /// The stream is built through mtrack's own capture path so the suite
    /// exercises the same format negotiation the trigger and calibration
    /// features rely on.
    pub fn record(duration: Duration) -> Result<Capture, Box<dyn std::error::Error>> {
        let caps = Capabilities::get();
        let input = caps
            .audio_in
            .as_ref()
            .ok_or("no audio input device available for capture")?;
        record_from(input, duration)
    }
}

/// Records for `duration` from a specific input device.
pub fn record_from(
    input: &AudioInput,
    duration: Duration,
) -> Result<Capture, Box<dyn std::error::Error>> {
    {
        let device = find_input_device(&input.name)?;
        let default_config = device.default_input_config()?;
        let channels = device
            .supported_input_configs()
            .map(|configs| configs.map(|c| c.channels()).max().unwrap_or(0))
            .unwrap_or(0)
            .max(default_config.channels());
        let sample_rate = default_config.sample_rate();

        let buffer = Arc::new(CaptureBuffer {
            channels: (0..channels).map(|_| Mutex::new(Vec::new())).collect(),
            active: AtomicBool::new(true),
        });

        let stream_config = cpal::StreamConfig {
            channels,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        // Deliberately not `mtrack::calibrate::build_capture_stream`: that has
        // no I24 arm and falls through to f32, which 24-bit interfaces refuse.
        // The tone side already handles I24, so delegating here would open the
        // output, fail the capture, and surface as the thoroughly misleading
        // "no output channel is patched back to an input".
        let stream = build_capture_stream_any(
            &device,
            &stream_config,
            buffer.clone(),
            channels,
            default_config.sample_format(),
        )?;
        stream.play()?;

        std::thread::sleep(duration);
        buffer.active.store(false, Ordering::SeqCst);
        drop(stream);

        let channels = buffer
            .channels
            .iter()
            .map(|c| c.lock().clone())
            .collect::<Vec<_>>();

        Ok(Capture {
            channels,
            sample_rate,
        })
    }
}

/// Finds a cpal input device by the name mtrack would use.
///
/// mtrack's own `find_input_device` is `pub(crate)`, so this repeats the
/// lookup rather than reaching into the crate.
fn find_input_device(name: &str) -> Result<cpal::Device, Box<dyn std::error::Error>> {
    let _stdout = shh::stdout();
    let _stderr = shh::stderr();

    for host_id in cpal::available_hosts() {
        let Ok(host) = cpal::host_from_id(host_id) else {
            continue;
        };
        let Ok(devices) = host.input_devices() else {
            continue;
        };
        for device in devices {
            if device
                .id()
                .map(|id| id.to_string().trim() == name.trim())
                .unwrap_or(false)
            {
                return Ok(device);
            }
        }
    }
    Err(format!("input device '{name}' not found").into())
}

/// Plays a distinct tone per output channel on `out` while recording every
/// input channel of `input`, and reports which output arrives on which input.
///
/// Works across devices, so a rig whose output interface differs from its
/// capture interface is discovered the same way as a single-box loopback.
/// Needs exclusive use of both devices, so no server may be running.
pub fn probe_pair(
    out: &crate::capabilities::AudioOutput,
    input: &crate::capabilities::AudioInput,
    tones: &[f32],
) -> Result<Vec<(u16, u16)>, Box<dyn std::error::Error>> {
    let device = find_output_device(&out.name)?;
    let default_config = device.default_output_config()?;
    let sample_rate = default_config.sample_rate();
    let out_channels = out.max_channels.min(tones.len() as u16);
    let tones_used: Vec<f32> = tones[..out_channels as usize].to_vec();

    let stream_config = cpal::StreamConfig {
        channels: out.max_channels,
        sample_rate,
        buffer_size: cpal::BufferSize::Default,
    };

    let stream = build_tone_stream(
        &device,
        &stream_config,
        default_config.sample_format(),
        tones_used.clone(),
        out.max_channels as usize,
        sample_rate,
        crate::songs::amplitude(),
        Arc::new(Mutex::new(0u64)),
    )?;
    stream.play()?;

    // Let the output settle before the measured window begins.
    std::thread::sleep(Duration::from_millis(500));
    let capture = record_from(input, Duration::from_secs(2))?;
    drop(stream);

    let mut pairs = Vec::new();
    for in_channel in 1..=capture.channel_count() as u16 {
        let window = capture.steady(
            in_channel,
            Duration::from_millis(400),
            Duration::from_millis(1200),
        );
        for (tone_index, _) in tones_present(window, capture.sample_rate(), &tones_used) {
            pairs.push((tone_index as u16 + 1, in_channel));
        }
    }
    Ok(pairs)
}

/// Builds a capture stream in the device's native sample format, including the
/// 24-bit formats common on consoles and outboard converters.
fn build_capture_stream_any(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: Arc<CaptureBuffer>,
    channels: u16,
    format: cpal::SampleFormat,
) -> Result<cpal::Stream, Box<dyn std::error::Error>> {
    match format {
        cpal::SampleFormat::I16 => capture_typed::<i16>(device, config, buffer, channels),
        cpal::SampleFormat::I32 => capture_typed::<i32>(device, config, buffer, channels),
        cpal::SampleFormat::I24 => capture_typed::<cpal::I24>(device, config, buffer, channels),
        _ => capture_typed::<f32>(device, config, buffer, channels),
    }
}

/// Typed capture builder. Samples are converted to f32 for analysis.
fn capture_typed<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: Arc<CaptureBuffer>,
    channels: u16,
) -> Result<cpal::Stream, Box<dyn std::error::Error>>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    let channels = channels.max(1) as usize;
    Ok(device.build_input_stream(
        *config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            if !buffer.active.load(Ordering::Relaxed) {
                return;
            }
            // One lock per channel per callback, not per sample. The naive
            // version took a mutex and grew a Vec for every sample of every
            // channel -- around 768k lock/push cycles per second on a 16-channel
            // 48 kHz capture, inside the real-time input callback. That is
            // plausibly what was producing the xruns this capture then reported
            // as "the interface underran".
            for (ch, slot) in buffer.channels.iter().enumerate().take(channels) {
                let mut out = slot.lock();
                out.reserve(data.len() / channels);
                out.extend(
                    data.iter()
                        .skip(ch)
                        .step_by(channels)
                        .map(|s| s.to_sample::<f32>()),
                );
            }
        },
        |e| eprintln!("capture stream error: {e}"),
        None,
    )?)
}

/// Builds the probe's tone stream in the device's native sample format.
#[allow(clippy::too_many_arguments)]
fn build_tone_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    format: cpal::SampleFormat,
    tones: Vec<f32>,
    channels: usize,
    sample_rate: u32,
    amplitude: f32,
    phase: Arc<Mutex<u64>>,
) -> Result<cpal::Stream, Box<dyn std::error::Error>> {
    match format {
        cpal::SampleFormat::I16 => build_tone_stream_typed::<i16>(
            device,
            config,
            tones,
            channels,
            sample_rate,
            amplitude,
            phase,
        ),
        cpal::SampleFormat::I32 => build_tone_stream_typed::<i32>(
            device,
            config,
            tones,
            channels,
            sample_rate,
            amplitude,
            phase,
        ),
        // 24-bit interfaces are common on consoles and outboard converters,
        // and they usually refuse f32 outright.
        cpal::SampleFormat::I24 => build_tone_stream_typed::<cpal::I24>(
            device,
            config,
            tones,
            channels,
            sample_rate,
            amplitude,
            phase,
        ),
        _ => build_tone_stream_typed::<f32>(
            device,
            config,
            tones,
            channels,
            sample_rate,
            amplitude,
            phase,
        ),
    }
}

/// Typed tone-stream builder. `cpal::FromSample` handles the conversion from
/// the f32 the tone is computed in to whatever the device wants.
#[allow(clippy::too_many_arguments)]
fn build_tone_stream_typed<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    tones: Vec<f32>,
    channels: usize,
    sample_rate: u32,
    amplitude: f32,
    phase: Arc<Mutex<u64>>,
) -> Result<cpal::Stream, Box<dyn std::error::Error>>
where
    T: cpal::SizedSample + cpal::FromSample<f32>,
{
    Ok(device.build_output_stream(
        *config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let mut n = phase.lock();
            for frame in data.chunks_mut(channels) {
                let t = *n as f32 / sample_rate as f32;
                for (ch, slot) in frame.iter_mut().enumerate() {
                    let value = match tones.get(ch) {
                        Some(&hz) => amplitude * (std::f32::consts::TAU * hz * t).sin(),
                        None => 0.0,
                    };
                    *slot = T::from_sample(value);
                }
                *n += 1;
            }
        },
        |e| eprintln!("loopback probe output error: {e}"),
        None,
    )?)
}

/// Finds a cpal output device by the name mtrack would use.
fn find_output_device(name: &str) -> Result<cpal::Device, Box<dyn std::error::Error>> {
    let _stdout = shh::stdout();
    let _stderr = shh::stderr();

    for host_id in cpal::available_hosts() {
        let Ok(host) = cpal::host_from_id(host_id) else {
            continue;
        };
        let Ok(devices) = host.output_devices() else {
            continue;
        };
        for device in devices {
            if device
                .id()
                .map(|id| id.to_string().trim() == name.trim())
                .unwrap_or(false)
            {
                return Ok(device);
            }
        }
    }
    Err(format!("output device '{name}' not found").into())
}

/// Waits for a condition, polling until a deadline.
pub fn wait_for(timeout: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if predicate() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}
