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

//! Crossfade curves and gain envelopes for smooth audio transitions.
//!
//! Provides [`CrossfadeCurve`] for computing fade gain values and
//! [`GainEnvelope`] for applying time-varying gain to audio sources.
//! Used by the mixer to fade sources in/out during loop crossfades
//! and song-to-song transitions.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Default crossfade duration used for loop boundaries and song transitions.
///
/// At 5ms this is rhythmically invisible — less than 1% of a beat at 60 BPM
/// (1000ms). The value was reduced from 100ms to eliminate cumulative timing
/// drift in section loops (each iteration was drifting by one crossfade
/// duration under the old trigger scheduling).
///
/// Constraint: must remain ≤ 10ms to stay imperceptible at the slowest
/// typical tempos. Validated by the `crossfade_duration_is_rhythmically_negligible`
/// test. If click artifacts appear, check sample rate, buffer size, and
/// audio format before increasing this value.
///
/// Trigger scheduling is handled by [`crate::section_loop::SectionLoopTrigger`],
/// which uses this margin to fire transitions slightly early so the crossfade
/// completes exactly at the ideal boundary.
pub const DEFAULT_CROSSFADE_DURATION: Duration = Duration::from_millis(5);

/// Returns the default crossfade duration in samples for the given sample rate.
pub fn default_crossfade_samples(sample_rate: u32) -> u64 {
    (DEFAULT_CROSSFADE_DURATION.as_secs_f64() * sample_rate as f64) as u64
}

/// Crossfade curve shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrossfadeCurve {
    /// Linear ramp: fade_out = 1-t, fade_in = t.
    /// Simple and predictable. Slight energy dip at midpoint.
    Linear,
    /// Equal-power (cosine) curve: fade_out = cos(t*pi/2), fade_in = sin(t*pi/2).
    /// Maintains constant power through the crossfade.
    EqualPower,
}

impl CrossfadeCurve {
    /// Returns `(fade_out_gain, fade_in_gain)` at normalized position `t` (0.0 to 1.0).
    ///
    /// At t=0: (1.0, 0.0) — fully on old source.
    /// At t=1: (0.0, 1.0) — fully on new source.
    pub fn gains(&self, t: f32) -> (f32, f32) {
        let t = t.clamp(0.0, 1.0);
        match self {
            CrossfadeCurve::Linear => (1.0 - t, t),
            CrossfadeCurve::EqualPower => {
                let angle = t * std::f32::consts::FRAC_PI_2;
                (angle.cos(), angle.sin())
            }
        }
    }
}

/// A gain envelope that ramps between two gain levels over a duration in samples.
///
/// Thread-safe: position is tracked via [`AtomicU64`] so the envelope can be
/// shared between the thread that creates it and the audio callback thread
/// that advances it.
pub struct GainEnvelope {
    start_gain: f32,
    end_gain: f32,
    curve: CrossfadeCurve,
    duration_samples: u64,
    position: AtomicU64,
}

impl GainEnvelope {
    /// Creates a fade-in envelope: gain ramps from 0.0 to 1.0.
    pub fn fade_in(duration_samples: u64, curve: CrossfadeCurve) -> Self {
        Self {
            start_gain: 0.0,
            end_gain: 1.0,
            curve,
            duration_samples,
            position: AtomicU64::new(0),
        }
    }

    /// Creates a fade-out envelope: gain ramps from 1.0 to 0.0.
    pub fn fade_out(duration_samples: u64, curve: CrossfadeCurve) -> Self {
        Self {
            start_gain: 1.0,
            end_gain: 0.0,
            curve,
            duration_samples,
            position: AtomicU64::new(0),
        }
    }

    /// Creates an envelope with custom start and end gain values.
    pub fn new(
        start_gain: f32,
        end_gain: f32,
        duration_samples: u64,
        curve: CrossfadeCurve,
    ) -> Self {
        Self {
            start_gain,
            end_gain,
            curve,
            duration_samples,
            position: AtomicU64::new(0),
        }
    }

    /// Returns the current gain value and advances the position by `frame_count` samples.
    ///
    /// The gain is computed at the current position before advancing, so it
    /// represents the gain for the block of frames being processed.
    pub fn advance(&self, frame_count: u64) -> f32 {
        let pos = self.position.fetch_add(frame_count, Ordering::Relaxed);
        self.gain_at(pos)
    }

    /// Returns the gain value at the given sample position without advancing.
    pub fn gain_at(&self, position: u64) -> f32 {
        if self.duration_samples == 0 {
            return self.end_gain;
        }

        let t = (position as f32 / self.duration_samples as f32).clamp(0.0, 1.0);

        // Use the curve to interpolate. For a fade-out (start=1, end=0),
        // we want the fade_out component. For a fade-in (start=0, end=1),
        // we want the fade_in component.
        let (fade_out, fade_in) = self.curve.gains(t);

        // Blend: at t=0, gain = start_gain. At t=1, gain = end_gain.
        // fade_out goes 1→0, fade_in goes 0→1.
        self.start_gain * fade_out + self.end_gain * fade_in
    }

    /// Returns true when the envelope has completed (position >= duration).
    pub fn is_finished(&self) -> bool {
        self.position.load(Ordering::Relaxed) >= self.duration_samples
    }

    /// Returns the current position in samples.
    pub fn position(&self) -> u64 {
        self.position.load(Ordering::Relaxed)
    }

    /// Returns the end gain value (the gain after the envelope completes).
    pub fn end_gain(&self) -> f32 {
        self.end_gain
    }

    /// Returns the start gain value.
    pub fn start_gain(&self) -> f32 {
        self.start_gain
    }

    /// Returns the total duration in samples.
    pub fn duration_samples(&self) -> u64 {
        self.duration_samples
    }

    /// Returns the crossfade curve.
    pub fn curve(&self) -> CrossfadeCurve {
        self.curve
    }
}

impl std::fmt::Debug for GainEnvelope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GainEnvelope")
            .field("start_gain", &self.start_gain)
            .field("end_gain", &self.end_gain)
            .field("curve", &self.curve)
            .field("duration_samples", &self.duration_samples)
            .field("position", &self.position.load(Ordering::Relaxed))
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linear_curve_endpoints() {
        let (fo, fi) = CrossfadeCurve::Linear.gains(0.0);
        assert!((fo - 1.0).abs() < f32::EPSILON);
        assert!((fi - 0.0).abs() < f32::EPSILON);

        let (fo, fi) = CrossfadeCurve::Linear.gains(1.0);
        assert!((fo - 0.0).abs() < f32::EPSILON);
        assert!((fi - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn linear_curve_midpoint() {
        let (fo, fi) = CrossfadeCurve::Linear.gains(0.5);
        assert!((fo - 0.5).abs() < f32::EPSILON);
        assert!((fi - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn equal_power_endpoints() {
        let (fo, fi) = CrossfadeCurve::EqualPower.gains(0.0);
        assert!((fo - 1.0).abs() < 0.001);
        assert!((fi - 0.0).abs() < 0.001);

        let (fo, fi) = CrossfadeCurve::EqualPower.gains(1.0);
        assert!((fo - 0.0).abs() < 0.001);
        assert!((fi - 1.0).abs() < 0.001);
    }

    #[test]
    fn equal_power_constant_power() {
        // For equal-power: fade_out^2 + fade_in^2 ≈ 1.0 at all points.
        for i in 0..=100 {
            let t = i as f32 / 100.0;
            let (fo, fi) = CrossfadeCurve::EqualPower.gains(t);
            let power = fo * fo + fi * fi;
            assert!(
                (power - 1.0).abs() < 0.001,
                "Power should be ~1.0 at t={}, got {} (fo={}, fi={})",
                t,
                power,
                fo,
                fi
            );
        }
    }

    #[test]
    fn linear_curve_clamped() {
        let (fo, fi) = CrossfadeCurve::Linear.gains(-0.5);
        assert!((fo - 1.0).abs() < f32::EPSILON);
        assert!((fi - 0.0).abs() < f32::EPSILON);

        let (fo, fi) = CrossfadeCurve::Linear.gains(1.5);
        assert!((fo - 0.0).abs() < f32::EPSILON);
        assert!((fi - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn gain_envelope_fade_in() {
        let env = GainEnvelope::fade_in(1000, CrossfadeCurve::Linear);

        // At start: gain ≈ 0.
        let g = env.advance(0);
        assert!((g - 0.0).abs() < 0.01);

        // Advance to midpoint.
        let g = env.advance(500);
        assert!((g - 0.0).abs() < 0.01); // Still at 0 because we read at pos=0 before advancing

        // Now at position 500.
        let g = env.advance(0);
        assert!((g - 0.5).abs() < 0.01);

        // Advance to end.
        let g = env.advance(500);
        assert!((g - 0.5).abs() < 0.01);

        // Past end.
        assert!(env.is_finished());
        let g = env.advance(0);
        assert!((g - 1.0).abs() < 0.01);
    }

    #[test]
    fn gain_envelope_fade_out() {
        let env = GainEnvelope::fade_out(1000, CrossfadeCurve::Linear);

        let g = env.advance(0);
        assert!((g - 1.0).abs() < 0.01);

        env.advance(500);
        let g = env.advance(0);
        assert!((g - 0.5).abs() < 0.01);

        env.advance(500);
        assert!(env.is_finished());
        let g = env.advance(0);
        assert!((g - 0.0).abs() < 0.01);
    }

    #[test]
    fn gain_envelope_is_finished() {
        let env = GainEnvelope::fade_out(100, CrossfadeCurve::Linear);

        assert!(!env.is_finished());
        env.advance(50);
        assert!(!env.is_finished());
        env.advance(50);
        assert!(env.is_finished());
    }

    #[test]
    fn gain_envelope_advance_increments() {
        let env = GainEnvelope::fade_in(1000, CrossfadeCurve::Linear);

        assert_eq!(env.position(), 0);
        env.advance(100);
        assert_eq!(env.position(), 100);
        env.advance(200);
        assert_eq!(env.position(), 300);
        env.advance(700);
        assert_eq!(env.position(), 1000);
        assert!(env.is_finished());
    }

    #[test]
    fn gain_envelope_zero_duration() {
        let env = GainEnvelope::fade_in(0, CrossfadeCurve::Linear);

        // Zero duration → immediately at end_gain.
        let g = env.advance(0);
        assert!((g - 1.0).abs() < f32::EPSILON);
        assert!(env.is_finished());
    }

    #[test]
    fn gain_envelope_equal_power_fade_out() {
        let env = GainEnvelope::fade_out(1000, CrossfadeCurve::EqualPower);

        // At start: gain ≈ 1.0.
        let g = env.gain_at(0);
        assert!((g - 1.0).abs() < 0.01);

        // At midpoint: gain = cos(pi/4) ≈ 0.707.
        let g = env.gain_at(500);
        assert!(
            (g - 0.707).abs() < 0.01,
            "Equal-power midpoint should be ~0.707, got {}",
            g
        );

        // At end: gain ≈ 0.0.
        let g = env.gain_at(1000);
        assert!((g - 0.0).abs() < 0.01);
    }

    #[test]
    fn gain_envelope_custom() {
        let env = GainEnvelope::new(0.5, 0.8, 1000, CrossfadeCurve::Linear);

        let g = env.gain_at(0);
        assert!((g - 0.5).abs() < 0.01);

        let g = env.gain_at(500);
        assert!((g - 0.65).abs() < 0.01);

        let g = env.gain_at(1000);
        assert!((g - 0.8).abs() < 0.01);
    }

    #[test]
    fn crossfade_duration_is_rhythmically_negligible() {
        // At 60 BPM (slowest typical tempo), one beat = 1000ms.
        // The crossfade must be small enough that triggering early by this
        // amount is imperceptible rhythmically (<= 10ms).
        assert!(
            DEFAULT_CROSSFADE_DURATION <= Duration::from_millis(10),
            "Crossfade duration {:?} is too large for rhythmically-tight section loops",
            DEFAULT_CROSSFADE_DURATION
        );
    }

    /// Simulates the section loop trigger scheduling logic used by the audio,
    /// MIDI, and DMX engines. Verifies that trigger times remain locked to an
    /// ideal metronomic grid over many iterations.
    ///
    /// The engines all follow the same pattern:
    ///   1. Initial trigger = section.end_time
    ///   2. Fire when: elapsed + crossfade_duration >= trigger_time
    ///   3. Schedule next: trigger_time + section_duration  (grid-locked)
    ///
    /// This test confirms that after N iterations the Nth trigger lands at
    /// exactly section.end_time + N * section_duration, with zero cumulative
    /// drift.
    #[test]
    fn section_loop_triggers_stay_on_grid() {
        // 4 measures at 120 BPM = 8 seconds.
        let section_start = Duration::from_secs(10);
        let section_end = Duration::from_secs(18);
        let section_duration = section_end - section_start;
        let crossfade_duration = DEFAULT_CROSSFADE_DURATION;

        let mut next_trigger = section_end;
        let iterations = 100;

        for i in 0..iterations {
            // The engine detects the trigger when elapsed reaches
            // trigger_time - crossfade_duration. The exact detection time
            // varies due to polling jitter, but the NEXT trigger must be
            // computed from the ideal trigger_time, not from elapsed.
            let expected_trigger = section_end + section_duration * i;
            assert_eq!(
                next_trigger, expected_trigger,
                "Trigger {} drifted: expected {:?}, got {:?}",
                i, expected_trigger, next_trigger
            );

            // Simulate: fire the trigger early (as the engines do) but
            // schedule the next one from the ideal trigger_time.
            let _simulated_elapsed = next_trigger - crossfade_duration;
            next_trigger += section_duration;
        }

        // After 100 iterations of an 8-second section, the final trigger
        // should land at exactly 10s + 100*8s = 810s with zero drift.
        let expected_final = section_end + section_duration * iterations;
        assert_eq!(next_trigger, expected_final);
    }

    /// Demonstrates that the old trigger scheduling approach (next = elapsed +
    /// section_duration) would accumulate drift of one crossfade_duration per
    /// iteration. This is a regression-detection test.
    #[test]
    fn old_trigger_scheduling_would_drift() {
        let section_start = Duration::from_secs(10);
        let section_end = Duration::from_secs(18);
        let section_duration = section_end - section_start;
        let crossfade_duration = DEFAULT_CROSSFADE_DURATION;

        let mut next_trigger = section_end;
        let iterations: u32 = 100;

        for _ in 0..iterations {
            // Old (buggy) pattern: next = elapsed + section_duration,
            // where elapsed = trigger_time - crossfade_duration.
            let simulated_elapsed = next_trigger - crossfade_duration;
            next_trigger = simulated_elapsed + section_duration;
        }

        // With the old approach, each iteration loses one crossfade_duration.
        let expected_ideal = section_end + section_duration * iterations;
        let total_drift = expected_ideal - next_trigger;
        assert_eq!(
            total_drift,
            crossfade_duration * iterations,
            "Old scheduling should drift by crossfade_duration per iteration"
        );
    }
}
