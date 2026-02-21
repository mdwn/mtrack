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

//! 2nd-order Butterworth high-pass biquad filter.
//!
//! Rejects low-frequency content (stage rumble, bass cab vibration) from
//! piezo trigger signals. Uses Direct Form I processing.

use std::f64::consts::{FRAC_1_SQRT_2, PI};

/// A 2nd-order Butterworth high-pass IIR filter (biquad).
pub(super) struct BiquadHighPass {
    // Numerator coefficients (pre-divided by a0).
    b0: f32,
    b1: f32,
    b2: f32,
    // Denominator coefficients (pre-divided by a0).
    a1: f32,
    a2: f32,
    // Input delay line.
    x1: f32,
    x2: f32,
    // Output delay line.
    y1: f32,
    y2: f32,
}

impl BiquadHighPass {
    /// Creates a new Butterworth high-pass filter.
    ///
    /// - `cutoff_hz`: the -3dB cutoff frequency in Hz.
    /// - `sample_rate`: the audio sample rate in Hz.
    pub fn new(cutoff_hz: f32, sample_rate: u32) -> Self {
        // Bilinear transform pre-warped frequency.
        let omega = 2.0 * PI * (cutoff_hz as f64) / (sample_rate as f64);
        let cos_omega = omega.cos();
        let sin_omega = omega.sin();
        // Q = 1/sqrt(2) for Butterworth (maximally flat).
        let alpha = sin_omega / (2.0 * FRAC_1_SQRT_2);

        let a0 = 1.0 + alpha;
        let b0 = ((1.0 + cos_omega) / 2.0) / a0;
        let b1 = (-(1.0 + cos_omega)) / a0;
        let b2 = ((1.0 + cos_omega) / 2.0) / a0;
        let a1 = (-2.0 * cos_omega) / a0;
        let a2 = (1.0 - alpha) / a0;

        Self {
            b0: b0 as f32,
            b1: b1 as f32,
            b2: b2 as f32,
            a1: a1 as f32,
            a2: a2 as f32,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    /// Processes a single sample through the filter (Direct Form I).
    pub fn process(&mut self, sample: f32) -> f32 {
        let output = self.b0 * sample + self.b1 * self.x1 + self.b2 * self.x2
            - self.a1 * self.y1
            - self.a2 * self.y2;

        self.x2 = self.x1;
        self.x1 = sample;
        self.y2 = self.y1;
        self.y1 = output;

        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dc_rejection() {
        let mut filter = BiquadHighPass::new(80.0, 44100);
        // Feed DC (constant 1.0) for enough samples to settle.
        for _ in 0..10000 {
            filter.process(1.0);
        }
        // After settling, output should be near zero (DC rejected).
        let out = filter.process(1.0);
        assert!(out.abs() < 0.001, "DC should be rejected, got {}", out);
    }

    #[test]
    fn test_high_frequency_passthrough() {
        let mut filter = BiquadHighPass::new(80.0, 44100);
        let freq = 4000.0_f64; // Well above cutoff
        let sample_rate = 44100.0_f64;

        // Let filter settle with a few cycles.
        for i in 0..4410 {
            let sample = (2.0 * PI * freq * (i as f64) / sample_rate).sin() as f32;
            filter.process(sample);
        }

        // Measure amplitude over one full cycle.
        let cycle_samples = (sample_rate / freq).ceil() as usize;
        let mut max_out: f32 = 0.0;
        for i in 4410..(4410 + cycle_samples) {
            let sample = (2.0 * PI * freq * (i as f64) / sample_rate).sin() as f32;
            let out = filter.process(sample);
            max_out = max_out.max(out.abs());
        }

        // High frequency should pass through with minimal attenuation.
        assert!(
            max_out > 0.9,
            "4kHz should pass through, got amplitude {}",
            max_out
        );
    }

    #[test]
    fn test_below_cutoff_attenuated() {
        let mut filter = BiquadHighPass::new(200.0, 44100);
        let freq = 20.0_f64; // Well below cutoff
        let sample_rate = 44100.0_f64;

        // Let filter settle.
        for i in 0..44100 {
            let sample = (2.0 * PI * freq * (i as f64) / sample_rate).sin() as f32;
            filter.process(sample);
        }

        // Measure amplitude over one cycle.
        let cycle_samples = (sample_rate / freq).ceil() as usize;
        let mut max_out: f32 = 0.0;
        for i in 44100..(44100 + cycle_samples) {
            let sample = (2.0 * PI * freq * (i as f64) / sample_rate).sin() as f32;
            let out = filter.process(sample);
            max_out = max_out.max(out.abs());
        }

        // 20Hz with 200Hz cutoff should be heavily attenuated (2nd order = -12dB/octave,
        // ~3.3 octaves below = ~-40dB).
        assert!(
            max_out < 0.05,
            "20Hz should be attenuated below 200Hz cutoff, got amplitude {}",
            max_out
        );
    }

    #[test]
    fn test_gain_at_cutoff_is_minus_3db() {
        let cutoff = 200.0_f64;
        let sample_rate = 44100.0_f64;
        let mut filter = BiquadHighPass::new(cutoff as f32, sample_rate as u32);

        // Let filter settle with several seconds of signal at the cutoff frequency.
        for i in 0..44100 {
            let sample = (2.0 * PI * cutoff * (i as f64) / sample_rate).sin() as f32;
            filter.process(sample);
        }

        // Measure amplitude over one full cycle.
        let cycle_samples = (sample_rate / cutoff).ceil() as usize;
        let mut max_out: f32 = 0.0;
        for i in 44100..(44100 + cycle_samples) {
            let sample = (2.0 * PI * cutoff * (i as f64) / sample_rate).sin() as f32;
            let out = filter.process(sample);
            max_out = max_out.max(out.abs());
        }

        // Butterworth -3dB at cutoff: gain ≈ 1/sqrt(2) ≈ 0.707.
        assert!(
            max_out > 0.65 && max_out < 0.75,
            "Expected ~0.707 gain at cutoff, got {}",
            max_out
        );
    }

    #[test]
    fn test_zero_input() {
        let mut filter = BiquadHighPass::new(80.0, 44100);
        for _ in 0..1000 {
            let out = filter.process(0.0);
            assert!(
                out.abs() < f32::EPSILON,
                "Zero input should produce zero output, got {}",
                out
            );
        }
    }
}
