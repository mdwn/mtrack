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

//! Stream construction for audio capture during calibration.

use std::error::Error;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use cpal::traits::DeviceTrait;

use super::{CalibrationConfig, CaptureBuffer};
use crate::audio::format::SampleFormat;

/// Resolves stream parameters (channels, sample rate, format) from device and config.
pub fn resolve_stream_params(
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
pub fn build_capture_stream(
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

/// Typed capture stream builder -- converts samples to f32 and pushes to CaptureBuffer.
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
