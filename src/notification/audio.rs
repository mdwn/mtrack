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

//! Audio generation and loading for notifications.
//!
//! Provides default tone generation for each notification event type and
//! loading of user-provided WAV override files via [`SampleLoader`].

use std::collections::HashMap;
use std::error::Error;
use std::path::Path;
use std::sync::Arc;

use tracing::{info, warn};

use crate::samples::loader::SampleLoader;

/// Generates the default tone PCM for each notification event type.
///
/// Returns a map of event key → mono f32 PCM samples at the given sample rate.
pub fn generate_default_tones(sample_rate: u32) -> HashMap<String, Arc<Vec<f32>>> {
    let mut tones = HashMap::new();

    // SectionEntering: two short ascending tones (800Hz → 1200Hz).
    tones.insert(
        "section_entering".to_string(),
        Arc::new(generate_two_tone(sample_rate, 800.0, 1200.0, 30, 0.25)),
    );

    // LoopArmed: single confirmation tone (1kHz, 50ms) — matches existing behavior.
    tones.insert(
        "loop_armed".to_string(),
        Arc::new(generate_sine_tone(sample_rate, 1000.0, 50, 0.25)),
    );

    // BreakRequested: two short descending tones (1200Hz → 800Hz).
    tones.insert(
        "break_requested".to_string(),
        Arc::new(generate_two_tone(sample_rate, 1200.0, 800.0, 30, 0.25)),
    );

    // LoopExited: two descending tones (800Hz → 600Hz).
    tones.insert(
        "loop_exited".to_string(),
        Arc::new(generate_two_tone(sample_rate, 800.0, 600.0, 30, 0.25)),
    );

    tones
}

/// Loads an audio file from disk into mono f32 PCM at the target sample rate.
///
/// Uses [`SampleLoader`] for file decoding and sample rate transcoding,
/// then mixes down to mono if the source is multi-channel.
pub fn load_audio_file(
    loader: &mut SampleLoader,
    path: &Path,
) -> Result<Arc<Vec<f32>>, Box<dyn Error>> {
    info!(path = ?path, "Loading notification audio override");

    let loaded = loader.load(path)?;
    let channel_count = loaded.channel_count() as usize;

    // Create a temporary source to read out the samples.
    let mut source = loaded.create_source(1.0);

    use crate::audio::sample_source::traits::SampleSource;
    let mut raw_samples = Vec::new();
    while let Some(sample) = source.next_sample()? {
        raw_samples.push(sample);
    }

    // Mix down to mono if multi-channel.
    let mono_samples = if channel_count > 1 {
        let frame_count = raw_samples.len() / channel_count;
        let mut mono = Vec::with_capacity(frame_count);
        for frame in 0..frame_count {
            let mut sum = 0.0f32;
            for ch in 0..channel_count {
                sum += raw_samples[frame * channel_count + ch];
            }
            mono.push(sum / channel_count as f32);
        }
        mono
    } else {
        raw_samples
    };

    Ok(Arc::new(mono_samples))
}

/// Loads override audio files from a map of key → path string.
///
/// Returns successfully loaded overrides; logs warnings for failures.
pub fn load_overrides(
    overrides: &HashMap<String, String>,
    base_path: &Path,
    loader: &mut SampleLoader,
) -> HashMap<String, Arc<Vec<f32>>> {
    let mut loaded = HashMap::new();

    for (key, path_str) in overrides {
        let path = if Path::new(path_str).is_absolute() {
            path_str.into()
        } else {
            base_path.join(path_str)
        };
        match load_audio_file(loader, &path) {
            Ok(samples) => {
                loaded.insert(key.clone(), samples);
            }
            Err(e) => {
                warn!(
                    key = key.as_str(),
                    path = ?path,
                    err = %e,
                    "Failed to load notification audio override"
                );
            }
        }
    }

    loaded
}

/// Generates a sine wave tone with fade-in/fade-out envelope.
fn generate_sine_tone(
    sample_rate: u32,
    frequency: f32,
    duration_ms: u32,
    amplitude: f32,
) -> Vec<f32> {
    let num_samples = (sample_rate as f64 * duration_ms as f64 / 1000.0) as usize;
    let mut samples = Vec::with_capacity(num_samples);
    let fade_samples = (sample_rate as f64 * 0.002) as usize; // 2ms fade

    for i in 0..num_samples {
        let t = i as f64 / sample_rate as f64;
        let phase = 2.0 * std::f64::consts::PI * frequency as f64 * t;
        let mut sample = (phase.sin() * amplitude as f64) as f32;

        // Fade envelope to avoid clicks.
        if i < fade_samples {
            sample *= i as f32 / fade_samples as f32;
        } else if i >= num_samples - fade_samples {
            sample *= (num_samples - 1 - i) as f32 / fade_samples as f32;
        }

        samples.push(sample);
    }

    samples
}

/// Generates a two-tone pattern (first tone, short gap, second tone).
fn generate_two_tone(
    sample_rate: u32,
    freq1: f32,
    freq2: f32,
    tone_duration_ms: u32,
    amplitude: f32,
) -> Vec<f32> {
    let tone1 = generate_sine_tone(sample_rate, freq1, tone_duration_ms, amplitude);
    let tone2 = generate_sine_tone(sample_rate, freq2, tone_duration_ms, amplitude);
    let gap_samples = (sample_rate as f64 * 0.010) as usize; // 10ms gap

    let mut combined = Vec::with_capacity(tone1.len() + gap_samples + tone2.len());
    combined.extend_from_slice(&tone1);
    combined.extend(std::iter::repeat_n(0.0f32, gap_samples));
    combined.extend_from_slice(&tone2);
    combined
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_tones_all_present() {
        let tones = generate_default_tones(44100);
        assert!(tones.contains_key("section_entering"));
        assert!(tones.contains_key("loop_armed"));
        assert!(tones.contains_key("break_requested"));
        assert!(tones.contains_key("loop_exited"));
    }

    #[test]
    fn default_tones_not_empty() {
        let tones = generate_default_tones(48000);
        for (key, samples) in &tones {
            assert!(!samples.is_empty(), "Tone '{}' should not be empty", key);
        }
    }

    #[test]
    fn sine_tone_correct_length() {
        let samples = generate_sine_tone(44100, 1000.0, 50, 0.25);
        // 50ms at 44100 Hz = 2205 samples.
        assert_eq!(samples.len(), 2205);
    }

    #[test]
    fn sine_tone_bounded_amplitude() {
        let samples = generate_sine_tone(48000, 1000.0, 50, 0.25);
        for &s in &samples {
            assert!(s.abs() <= 0.26, "Sample {} exceeds expected amplitude", s);
        }
    }

    #[test]
    fn sine_tone_starts_and_ends_near_zero() {
        let samples = generate_sine_tone(44100, 1000.0, 50, 0.25);
        assert!(samples[0].abs() < 0.01);
        assert!(samples.last().unwrap().abs() < 0.01);
    }

    #[test]
    fn two_tone_longer_than_single() {
        let single = generate_sine_tone(44100, 800.0, 30, 0.25);
        let double = generate_two_tone(44100, 800.0, 1200.0, 30, 0.25);
        assert!(double.len() > single.len() * 2);
    }

    #[test]
    fn two_tone_has_gap() {
        let tone_samples = (44100.0 * 0.030) as usize; // 30ms tone
        let gap_samples = (44100.0 * 0.010) as usize; // 10ms gap

        let double = generate_two_tone(44100, 800.0, 1200.0, 30, 0.25);

        // Check that the gap region is silent.
        for (i, &sample) in double
            .iter()
            .enumerate()
            .skip(tone_samples)
            .take(gap_samples)
        {
            assert_eq!(sample, 0.0, "Gap sample at index {} should be silent", i);
        }
    }
}
