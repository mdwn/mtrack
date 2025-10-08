// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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

#[cfg(test)]
use std::{
    error::Error,
    fs::File,
    path::PathBuf,
    thread,
    time::{Duration, SystemTime},
};

#[cfg(test)]
use hound::{SampleFormat, WavSpec, WavWriter};

#[cfg(test)]
use crate::songs::Sample;

/// Audio test utilities for generating test signals and validating results
#[cfg(test)]
pub mod audio_test_utils {
    use std::f32::consts::PI;

    /// Generate a multi-frequency signal (sum of sine waves)
    pub fn generate_multi_frequency_signal(
        frequencies: &[f32],
        amplitudes: &[f32],
        sample_rate: u32,
        duration_seconds: f32,
    ) -> Vec<f32> {
        assert_eq!(
            frequencies.len(),
            amplitudes.len(),
            "Frequencies and amplitudes must have same length"
        );

        let sample_count = (sample_rate as f32 * duration_seconds) as usize;
        let mut samples = vec![0.0; sample_count];

        for i in 0..sample_count {
            let t = i as f32 / sample_rate as f32;
            for (freq, amp) in frequencies.iter().zip(amplitudes.iter()) {
                samples[i] += amp * (2.0 * PI * freq * t).sin();
            }
        }

        samples
    }

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

/// Wait for the given predicate to return true or fail.
#[inline]
#[cfg(test)]
pub fn eventually<F>(predicate: F, error_msg: &str)
where
    F: Fn() -> bool,
{
    let start = SystemTime::now();
    let tick = Duration::from_millis(10);
    let timeout = Duration::from_secs(3);

    loop {
        let elapsed = start.elapsed();
        if elapsed.is_err() {
            panic!("System time error");
        }
        let elapsed = elapsed.unwrap();

        if elapsed > timeout {
            panic!("{}", error_msg);
        }
        if predicate() {
            return;
        }
        thread::sleep(tick);
    }
}

/// Wait for the given async predicate to return true or fail.
#[inline]
#[cfg(test)]
pub async fn eventually_async<F, Fut>(mut predicate: F, error_msg: &str)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = SystemTime::now();
    let tick = Duration::from_millis(10);
    let timeout = Duration::from_secs(3);

    loop {
        let elapsed = start.elapsed();
        if elapsed.is_err() {
            panic!("System time error");
        }
        let elapsed = elapsed.unwrap();

        if elapsed > timeout {
            panic!("{}", error_msg);
        }
        if predicate().await {
            return;
        }
        tokio::time::sleep(tick).await;
    }
}

#[cfg(test)]
pub fn write_wav<S: Sample>(
    path: PathBuf,
    samples: Vec<Vec<S>>,
    sample_rate: u32,
) -> Result<(), Box<dyn Error>> {
    let tempwav = File::create(path)?;

    // Determine sample format based on the type
    let sample_format = if std::any::TypeId::of::<S>() == std::any::TypeId::of::<f32>() {
        SampleFormat::Float
    } else if std::any::TypeId::of::<S>() == std::any::TypeId::of::<i32>() {
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
            bits_per_sample: 32,
            sample_format,
        },
    )?;

    // Write a simple set of samples to the wav file.
    for channel in 0..samples.len() {
        for sample in &samples[channel] {
            writer.write_sample(sample.to_owned())?;
        }
    }

    Ok(())
}
