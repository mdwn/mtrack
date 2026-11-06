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

use std::{error::Error, fs::File, path::PathBuf};

use hound::{SampleFormat, WavSpec, WavWriter};

pub fn write_wav<S: hound::Sample + Copy + 'static>(
    path: PathBuf,
    samples: Vec<Vec<S>>,
    sample_rate: u32,
) -> Result<(), Box<dyn Error>> {
    write_wav_with_bits(path, samples, sample_rate, 32)
}

pub fn write_wav_with_bits<S: hound::Sample + Copy + 'static>(
    path: PathBuf,
    samples: Vec<Vec<S>>,
    sample_rate: u32,
    bits_per_sample: u16,
) -> Result<(), Box<dyn Error>> {
    let tempwav = File::create(path)?;

    // Determine sample format based on the type
    let sample_format = if std::any::TypeId::of::<S>() == std::any::TypeId::of::<f32>() {
        SampleFormat::Float
    } else if std::any::TypeId::of::<S>() == std::any::TypeId::of::<i32>()
        || std::any::TypeId::of::<S>() == std::any::TypeId::of::<i16>()
    {
        SampleFormat::Int
    } else {
        return Err("Unsupported sample format".into());
    };

    let num_channels = samples.len();
    assert!(num_channels <= u16::MAX.into(), "Too many channels!");
    let mut writer = WavWriter::new(
        tempwav,
        WavSpec {
            channels: num_channels as u16,
            sample_rate,
            bits_per_sample,
            sample_format,
        },
    )?;

    // Interleaved, frame by frame, as WAV stores them: `samples` holds one
    // vector per channel, all the same length.
    let frames = samples.first().map_or(0, Vec::len);
    assert!(
        samples.iter().all(|channel| channel.len() == frames),
        "every channel needs the same number of samples"
    );
    for frame in 0..frames {
        for channel_samples in &samples {
            writer.write_sample(channel_samples[frame])?;
        }
    }

    Ok(())
}

/// Audio test utilities for generating test signals and validating results
pub mod audio_test_utils {
    /// Calculate RMS (Root Mean Square) of a signal
    pub fn calculate_rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }

        let sum_squares: f32 = samples.iter().map(|&x| x * x).sum();
        (sum_squares / samples.len() as f32).sqrt()
    }

    /// Calculate Signal-to-Noise Ratio (SNR) in dB
    pub fn calculate_snr(original: &[f32], processed: &[f32]) -> f32 {
        if original.len() != processed.len() {
            return 0.0;
        }

        let signal_power = calculate_rms(original).powi(2);
        let noise_power = original
            .iter()
            .zip(processed.iter())
            .map(|(o, p)| (o - p).powi(2))
            .sum::<f32>()
            / original.len() as f32;

        if noise_power == 0.0 {
            return f32::INFINITY;
        }

        10.0 * (signal_power / noise_power).log10()
    }
}
