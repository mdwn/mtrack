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

//! Audio confirmation sounds for section loop activation.
//!
//! Provides a pluggable [`ConfirmationSound`] trait and a default [`SineTone`]
//! implementation. The confirmation tone is played through the mixer when a
//! section loop engages, giving performers audio feedback.

/// Generates audio samples for a confirmation sound.
///
/// Implementations return mono f32 samples at the requested sample rate.
/// The trait is pluggable — swap the default sine tone for a custom WAV later.
pub trait ConfirmationSound: Send + Sync {
    /// Returns mono f32 samples at the given sample rate.
    fn generate(&self, sample_rate: u32) -> Vec<f32>;
}

/// A short sine wave tone used as the default confirmation sound.
pub struct SineTone {
    /// Frequency in Hz (default: 1000).
    frequency: f32,
    /// Duration in milliseconds (default: 50).
    duration_ms: u32,
    /// Peak amplitude, 0.0–1.0 (default: 0.25, approx -12dB).
    amplitude: f32,
}

impl SineTone {
    /// Creates a new sine tone with the given parameters.
    pub fn new(frequency: f32, duration_ms: u32, amplitude: f32) -> Self {
        Self {
            frequency,
            duration_ms,
            amplitude: amplitude.clamp(0.0, 1.0),
        }
    }
}

impl Default for SineTone {
    fn default() -> Self {
        Self {
            frequency: 1000.0,
            duration_ms: 50,
            amplitude: 0.25,
        }
    }
}

impl ConfirmationSound for SineTone {
    fn generate(&self, sample_rate: u32) -> Vec<f32> {
        let num_samples = (sample_rate as f64 * self.duration_ms as f64 / 1000.0) as usize;
        let mut samples = Vec::with_capacity(num_samples);

        for i in 0..num_samples {
            let t = i as f64 / sample_rate as f64;
            let phase = 2.0 * std::f64::consts::PI * self.frequency as f64 * t;
            let mut sample = (phase.sin() * self.amplitude as f64) as f32;

            // Apply a short fade-in/fade-out envelope (2ms each) to avoid clicks.
            let fade_samples = (sample_rate as f64 * 0.002) as usize;
            if i < fade_samples {
                sample *= i as f32 / fade_samples as f32;
            } else if i >= num_samples - fade_samples {
                sample *= (num_samples - 1 - i) as f32 / fade_samples as f32;
            }

            samples.push(sample);
        }

        samples
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sine_tone_generates_correct_length() {
        let tone = SineTone::default();
        let samples = tone.generate(44100);
        // 50ms at 44100 Hz = 2205 samples.
        assert_eq!(samples.len(), 2205);
    }

    #[test]
    fn sine_tone_custom_params() {
        let tone = SineTone::new(440.0, 100, 0.5);
        let samples = tone.generate(48000);
        // 100ms at 48000 Hz = 4800 samples.
        assert_eq!(samples.len(), 4800);
    }

    #[test]
    fn sine_tone_amplitude_bounded() {
        let tone = SineTone::default();
        let samples = tone.generate(44100);
        for &s in &samples {
            assert!(
                s.abs() <= tone.amplitude + 0.001,
                "Sample {} exceeds amplitude {}",
                s,
                tone.amplitude
            );
        }
    }

    #[test]
    fn sine_tone_starts_and_ends_near_zero() {
        let tone = SineTone::default();
        let samples = tone.generate(44100);
        // Fade envelope means first and last samples should be near zero.
        assert!(
            samples[0].abs() < 0.01,
            "First sample should be near zero: {}",
            samples[0]
        );
        assert!(
            samples.last().unwrap().abs() < 0.01,
            "Last sample should be near zero: {}",
            samples.last().unwrap()
        );
    }

    #[test]
    fn sine_tone_not_silent() {
        let tone = SineTone::default();
        let samples = tone.generate(44100);
        let peak = samples.iter().map(|s| s.abs()).fold(0.0_f32, f32::max);
        assert!(peak > 0.1, "Tone should not be silent, peak: {}", peak);
    }

    #[test]
    fn custom_trait_impl_works() {
        struct ClickSound;
        impl ConfirmationSound for ClickSound {
            fn generate(&self, sample_rate: u32) -> Vec<f32> {
                let len = (sample_rate as f64 * 0.01) as usize; // 10ms
                vec![0.5; len]
            }
        }

        let sound = ClickSound;
        let samples = sound.generate(44100);
        assert_eq!(samples.len(), 441);
        assert_eq!(samples[0], 0.5);
    }
}
