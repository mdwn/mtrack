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

//! Per-channel transient detection state machine.
//!
//! States: Idle → Scanning → Lockout → Idle
//!
//! - **Idle:** Waits for `|sample * gain| > threshold`, then transitions to Scanning.
//! - **Scanning:** Tracks peak amplitude for `scan_time` samples, then fires a trigger
//!   and transitions to Lockout.
//! - **Lockout:** Waits `retrigger_time` samples, then returns to Idle.

use super::filter::BiquadHighPass;
use crate::config::trigger::{AudioTriggerInput, TriggerInputAction, VelocityCurve};
use crate::samples::{TriggerAction, TriggerEvent};

/// Internal state of the detector.
enum State {
    /// Waiting for a threshold crossing.
    Idle,
    /// Threshold crossed; tracking peak for scan window.
    Scanning { peak: f32, remaining_samples: u32 },
    /// Trigger fired; lockout to prevent double-triggering.
    Lockout { remaining_samples: u32 },
}

/// Per-channel transient detector.
pub(super) struct TriggerDetector {
    state: State,
    /// Input gain multiplier.
    gain: f32,
    /// Amplitude threshold (0.0-1.0).
    threshold: f32,
    /// Scan window in samples.
    scan_samples: u32,
    /// Lockout period in samples.
    lockout_samples: u32,
    /// Velocity curve type.
    velocity_curve: VelocityCurve,
    /// Fixed velocity value (used when curve is Fixed).
    fixed_velocity: u8,
    /// Sample name to trigger (None for release-only inputs).
    sample_name: Option<String>,
    /// Release group for voice management.
    release_group: Option<String>,
    /// Action type (trigger or release).
    action: TriggerInputAction,
    /// Optional high-pass filter for low-frequency rejection.
    highpass: Option<BiquadHighPass>,
    /// Exponential decay coefficient for dynamic threshold.
    dynamic_decay_coeff: Option<f32>,
    /// Current dynamic threshold offset (decays toward 0).
    dynamic_level: f32,
    /// Remaining samples in crosstalk suppression window.
    crosstalk_remaining: u32,
    /// Threshold multiplier during crosstalk suppression.
    crosstalk_multiplier: f32,
    /// EMA coefficient for noise floor tracking (None = disabled).
    noise_floor_alpha: Option<f32>,
    /// Current noise floor EMA estimate.
    noise_floor_ema: f32,
    /// Sensitivity multiplier: noise_ema * sensitivity = adaptive threshold floor.
    noise_floor_sensitivity: f32,
}

impl TriggerDetector {
    /// Creates a detector from an `AudioTriggerInput` configuration.
    pub(super) fn from_input(input: &AudioTriggerInput, sample_rate: u32) -> Self {
        Self::new(
            sample_rate,
            input.threshold(),
            input.retrigger_time_ms(),
            input.scan_time_ms(),
            input.gain(),
            input.velocity_curve(),
            input.fixed_velocity(),
            input.sample().map(|s| s.to_string()),
            input.release_group().map(|s| s.to_string()),
            input.action(),
            input.highpass_freq(),
            input.dynamic_threshold_decay_ms(),
            input.noise_floor_sensitivity(),
            input.noise_floor_decay_ms(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        sample_rate: u32,
        threshold: f32,
        retrigger_time_ms: u32,
        scan_time_ms: u32,
        gain: f32,
        velocity_curve: VelocityCurve,
        fixed_velocity: u8,
        sample_name: Option<String>,
        release_group: Option<String>,
        action: TriggerInputAction,
        highpass_freq: Option<f32>,
        dynamic_threshold_decay_ms: Option<u32>,
        noise_floor_sensitivity: Option<f32>,
        noise_floor_decay_ms: u32,
    ) -> Self {
        let highpass = highpass_freq.map(|freq| BiquadHighPass::new(freq, sample_rate));

        let dynamic_decay_coeff = dynamic_threshold_decay_ms.map(|ms| {
            let decay_samples = (ms as f64) * (sample_rate as f64) / 1000.0;
            (-1.0 / decay_samples).exp() as f32
        });

        let noise_floor_alpha = noise_floor_sensitivity.map(|_| {
            let decay_samples = (noise_floor_decay_ms as f64) * (sample_rate as f64) / 1000.0;
            (-1.0 / decay_samples).exp() as f32
        });

        Self {
            state: State::Idle,
            gain,
            threshold,
            scan_samples: super::ms_to_samples(scan_time_ms, sample_rate),
            lockout_samples: super::ms_to_samples(retrigger_time_ms, sample_rate),
            velocity_curve,
            fixed_velocity,
            sample_name,
            release_group,
            action,
            highpass,
            dynamic_decay_coeff,
            dynamic_level: 0.0,
            crosstalk_remaining: 0,
            crosstalk_multiplier: 1.0,
            noise_floor_alpha,
            noise_floor_ema: 0.0,
            noise_floor_sensitivity: noise_floor_sensitivity.unwrap_or(0.0),
        }
    }

    /// Processes a single audio sample. Returns a `TriggerAction` when a trigger fires.
    ///
    /// Applies the high-pass filter (if configured), then delegates to `detect()`.
    /// The split prevents Lockout→Idle recursion from re-running the filter,
    /// which would corrupt biquad state.
    pub(super) fn process_sample(&mut self, sample: f32) -> Option<TriggerAction> {
        let filtered = match &mut self.highpass {
            Some(hpf) => hpf.process(sample),
            None => sample,
        };
        self.detect(filtered)
    }

    /// Core detection state machine operating on (optionally filtered) samples.
    fn detect(&mut self, sample: f32) -> Option<TriggerAction> {
        // Decay the dynamic threshold level each sample.
        if let Some(coeff) = self.dynamic_decay_coeff {
            self.dynamic_level *= coeff;
        }

        // Decrement crosstalk suppression counter.
        if self.crosstalk_remaining > 0 {
            self.crosstalk_remaining -= 1;
        }

        let amplitude = (sample * self.gain).abs();

        // Update noise floor EMA during Idle state only.
        if matches!(self.state, State::Idle) {
            if let Some(alpha) = self.noise_floor_alpha {
                self.noise_floor_ema = alpha * self.noise_floor_ema + (1.0 - alpha) * amplitude;
            }
        }

        let effective_threshold = self.effective_threshold();

        match &mut self.state {
            State::Idle => {
                if amplitude > effective_threshold {
                    if self.scan_samples == 0 {
                        // No scan window — fire immediately
                        let action = self.fire(amplitude);
                        self.state = State::Lockout {
                            remaining_samples: self.lockout_samples,
                        };
                        return Some(action);
                    }
                    // The threshold-crossing sample counts as the first scan sample
                    // (its amplitude is already recorded as the initial peak), so we
                    // start the counter at scan_samples - 1.
                    self.state = State::Scanning {
                        peak: amplitude,
                        remaining_samples: self.scan_samples - 1,
                    };
                }
                None
            }
            State::Scanning {
                peak,
                remaining_samples,
            } => {
                if amplitude > *peak {
                    *peak = amplitude;
                }
                if *remaining_samples == 0 {
                    let final_peak = *peak;
                    let action = self.fire(final_peak);
                    self.state = State::Lockout {
                        remaining_samples: self.lockout_samples,
                    };
                    Some(action)
                } else {
                    *remaining_samples -= 1;
                    None
                }
            }
            State::Lockout { remaining_samples } => {
                if *remaining_samples == 0 {
                    // Transition to Idle and evaluate this sample immediately.
                    // Calls detect() (not process_sample()) to avoid re-filtering.
                    self.state = State::Idle;
                    return self.detect(sample);
                }
                *remaining_samples -= 1;
                None
            }
        }
    }

    /// Computes the effective threshold, incorporating dynamic level and crosstalk.
    fn effective_threshold(&self) -> f32 {
        let adaptive_floor = self.noise_floor_ema * self.noise_floor_sensitivity;
        let base = self.threshold.max(adaptive_floor) + self.dynamic_level;
        if self.crosstalk_remaining > 0 {
            base * self.crosstalk_multiplier
        } else {
            base
        }
    }

    /// Converts a peak amplitude to a velocity value.
    fn amplitude_to_velocity(&self, peak: f32) -> u8 {
        match self.velocity_curve {
            VelocityCurve::Linear => {
                let clamped = peak.min(1.0);
                (clamped * 127.0) as u8
            }
            VelocityCurve::Logarithmic => {
                if peak <= self.threshold {
                    return 1;
                }
                let clamped = peak.min(1.0);
                let range = 1.0 - self.threshold;
                if range < f32::EPSILON {
                    return 127;
                }
                // Map threshold→1.0 logarithmically to 1→127
                let normalized = (clamped - self.threshold) / range;
                let log_val = (normalized + 1.0).ln() / 2.0_f32.ln();
                let velocity = 1.0 + log_val * 126.0;
                (velocity.clamp(1.0, 127.0)) as u8
            }
            VelocityCurve::Fixed => self.fixed_velocity,
        }
    }

    /// Creates a TriggerAction from the detected peak amplitude.
    ///
    /// When dynamic threshold is enabled, sets the dynamic level based on
    /// how far above threshold the peak was.
    fn fire(&mut self, peak: f32) -> TriggerAction {
        // Set dynamic threshold level if enabled.
        if self.dynamic_decay_coeff.is_some() {
            self.dynamic_level = (peak - self.threshold).max(0.0);
        }

        match self.action {
            TriggerInputAction::Trigger => {
                let velocity = self.amplitude_to_velocity(peak);
                TriggerAction::Trigger(TriggerEvent {
                    sample_name: self.sample_name.clone().unwrap_or_default(),
                    velocity,
                    release_group: self.release_group.clone(),
                })
            }
            TriggerInputAction::Release => TriggerAction::Release {
                group: self.release_group.clone().unwrap_or_default(),
            },
        }
    }

    /// Applies crosstalk suppression from another channel firing.
    ///
    /// Sets or extends the suppression window. Uses `max()` to avoid
    /// shortening an existing window.
    pub(super) fn apply_crosstalk_suppression(&mut self, window_samples: u32, multiplier: f32) {
        self.crosstalk_remaining = self.crosstalk_remaining.max(window_samples);
        self.crosstalk_multiplier = self.crosstalk_multiplier.max(multiplier);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expect_trigger(action: Option<TriggerAction>) -> TriggerEvent {
        match action {
            Some(TriggerAction::Trigger(e)) => e,
            other => panic!("Expected TriggerAction::Trigger, got {:?}", other),
        }
    }

    fn expect_release(action: Option<TriggerAction>) -> String {
        match action {
            Some(TriggerAction::Release { group }) => group,
            other => panic!("Expected TriggerAction::Release, got {:?}", other),
        }
    }

    fn make_trigger_detector(threshold: f32, scan_ms: u32, lockout_ms: u32) -> TriggerDetector {
        TriggerDetector::new(
            44100,
            threshold,
            lockout_ms,
            scan_ms,
            1.0,
            VelocityCurve::Linear,
            127,
            Some("kick".to_string()),
            Some("kick".to_string()),
            TriggerInputAction::Trigger,
            None,
            None,
            None,
            200,
        )
    }

    #[test]
    fn test_below_threshold_no_trigger() {
        let mut det = make_trigger_detector(0.1, 5, 30);
        // Feed samples below threshold
        for _ in 0..1000 {
            assert!(det.process_sample(0.05).is_none());
        }
    }

    #[test]
    fn test_above_threshold_fires() {
        let mut det = make_trigger_detector(0.1, 0, 30);
        // With scan_time=0, should fire immediately on threshold crossing
        let event = expect_trigger(det.process_sample(0.5));
        assert_eq!(event.sample_name, "kick");
        assert_eq!(event.release_group, Some("kick".to_string()));
        // Linear velocity: 0.5 * 127 = 63
        assert_eq!(event.velocity, 63);
    }

    #[test]
    fn test_peak_detection_during_scan() {
        // scan_time_ms = 5ms at 44100 = 221 samples
        let mut det = make_trigger_detector(0.1, 5, 30);

        // Cross threshold
        assert!(det.process_sample(0.2).is_none());

        // Peak during scan window
        let scan_samples = ((5.0_f64 * 44100.0) / 1000.0).ceil() as u32;
        for i in 0..scan_samples {
            let sample = if i == scan_samples / 2 { 0.8 } else { 0.3 };
            let result = det.process_sample(sample);
            if i == scan_samples - 1 {
                // Should fire on last scan sample — peak was 0.8 → velocity = 101
                let event = expect_trigger(result);
                assert_eq!(event.velocity, 101);
                return;
            }
        }
        panic!("Scan window ended without trigger");
    }

    #[test]
    fn test_retrigger_lockout() {
        let mut det = make_trigger_detector(0.1, 0, 30);

        // First trigger
        assert!(det.process_sample(0.5).is_some());

        // During lockout, even above-threshold signals should be ignored
        let lockout_samples = ((30.0_f64 * 44100.0) / 1000.0).ceil() as u32;
        for _ in 0..lockout_samples {
            assert!(det.process_sample(0.9).is_none());
        }

        // After lockout, should trigger again
        assert!(det.process_sample(0.5).is_some());
    }

    #[test]
    fn test_linear_velocity() {
        let mut det = make_trigger_detector(0.1, 0, 0);

        assert_eq!(expect_trigger(det.process_sample(1.0)).velocity, 127);
        // Reset by going through idle
        assert_eq!(expect_trigger(det.process_sample(0.5)).velocity, 63);
    }

    #[test]
    fn test_logarithmic_velocity() {
        let mut det = TriggerDetector::new(
            44100,
            0.1,
            0,
            0,
            1.0,
            VelocityCurve::Logarithmic,
            127,
            Some("kick".to_string()),
            None,
            TriggerInputAction::Trigger,
            None,
            None,
            None,
            200,
        );

        // At threshold → velocity 1
        assert!(expect_trigger(det.process_sample(0.1001)).velocity <= 2);
        // At max → velocity 127
        assert_eq!(expect_trigger(det.process_sample(1.0)).velocity, 127);
        // Mid-range should be between 1 and 127
        let vel = expect_trigger(det.process_sample(0.55)).velocity;
        assert!(vel > 1 && vel < 127);
    }

    #[test]
    fn test_fixed_velocity() {
        let mut det = TriggerDetector::new(
            44100,
            0.1,
            0,
            0,
            1.0,
            VelocityCurve::Fixed,
            100,
            Some("kick".to_string()),
            None,
            TriggerInputAction::Trigger,
            None,
            None,
            None,
            200,
        );

        assert_eq!(expect_trigger(det.process_sample(0.5)).velocity, 100);
        // Different amplitude, same fixed velocity
        assert_eq!(expect_trigger(det.process_sample(0.9)).velocity, 100);
    }

    #[test]
    fn test_release_action() {
        let mut det = TriggerDetector::new(
            44100,
            0.05,
            0,
            0,
            1.0,
            VelocityCurve::Linear,
            127,
            None,
            Some("cymbal".to_string()),
            TriggerInputAction::Release,
            None,
            None,
            None,
            200,
        );

        assert_eq!(expect_release(det.process_sample(0.3)), "cymbal");
    }

    #[test]
    fn test_gain_multiplier() {
        // With gain=2.0, a 0.06 sample becomes 0.12 which is above 0.1 threshold
        let mut det = TriggerDetector::new(
            44100,
            0.1,
            0,
            0,
            2.0,
            VelocityCurve::Linear,
            127,
            Some("kick".to_string()),
            None,
            TriggerInputAction::Trigger,
            None,
            None,
            None,
            200,
        );

        let result = det.process_sample(0.06);
        assert!(result.is_some(), "Gain should push signal above threshold");
    }

    #[test]
    fn test_negative_samples() {
        let mut det = make_trigger_detector(0.1, 0, 0);

        // Negative sample should still trigger (uses abs)
        assert_eq!(expect_trigger(det.process_sample(-0.5)).velocity, 63);
    }

    #[test]
    fn test_dynamic_threshold_prevents_ringing_retrigger() {
        // With dynamic threshold enabled, piezo ringing after a hit should not retrigger.
        // threshold=0.1, lockout=0ms (disabled), scan=0ms, dynamic_decay=50ms
        let mut det = TriggerDetector::new(
            44100,
            0.1,
            0, // no lockout
            0, // no scan
            1.0,
            VelocityCurve::Linear,
            127,
            Some("kick".to_string()),
            None,
            TriggerInputAction::Trigger,
            None,
            Some(50), // 50ms dynamic decay
            None,
            200,
        );

        // Strong hit triggers
        let result = det.process_sample(0.8);
        assert!(result.is_some(), "Initial hit should trigger");

        // Immediately after, ringing at 0.3 should NOT retrigger because
        // dynamic_level was set to 0.8 - 0.1 = 0.7, so effective threshold = 0.1 + 0.7 = 0.8
        let result = det.process_sample(0.3);
        assert!(result.is_none(), "Ringing should not retrigger");

        // Even 0.5 should not retrigger right after
        let result = det.process_sample(0.5);
        assert!(result.is_none(), "Ringing at 0.5 should not retrigger");
    }

    #[test]
    fn test_dynamic_threshold_decays_allows_retrigger() {
        // After enough time, dynamic threshold decays and allows a real retrigger.
        let mut det = TriggerDetector::new(
            44100,
            0.1,
            0, // no lockout
            0, // no scan
            1.0,
            VelocityCurve::Linear,
            127,
            Some("kick".to_string()),
            None,
            TriggerInputAction::Trigger,
            None,
            Some(10), // 10ms dynamic decay (fast decay for test)
            None,
            200,
        );

        // Strong hit
        assert!(det.process_sample(0.8).is_some());

        // Feed silence for 200ms (well past 10ms decay) to let dynamic level decay
        let silence_samples = ((200.0_f64 * 44100.0) / 1000.0).ceil() as u32;
        for _ in 0..silence_samples {
            assert!(det.process_sample(0.0).is_none());
        }

        // Now a real hit at 0.3 should trigger (dynamic level has decayed to ~0)
        let result = det.process_sample(0.3);
        assert!(result.is_some(), "Real hit after decay should trigger");
    }

    #[test]
    fn test_crosstalk_suppression_elevates_threshold() {
        let mut det = make_trigger_detector(0.1, 0, 0);

        // Apply crosstalk suppression: 441 samples (10ms), 3x threshold multiplier
        det.apply_crosstalk_suppression(441, 3.0);

        // Signal at 0.2 is above normal threshold (0.1) but below suppressed (0.3)
        let result = det.process_sample(0.2);
        assert!(
            result.is_none(),
            "Should be suppressed during crosstalk window"
        );

        // Signal at 0.4 is above even the suppressed threshold
        let result = det.process_sample(0.4);
        assert!(
            result.is_some(),
            "Strong signal should overcome crosstalk suppression"
        );
    }

    #[test]
    fn test_crosstalk_suppression_expires() {
        let mut det = make_trigger_detector(0.1, 0, 0);

        // Apply crosstalk suppression for 100 samples, 5x multiplier
        det.apply_crosstalk_suppression(100, 5.0);

        // Feed silence to burn through suppression window
        for _ in 0..100 {
            det.process_sample(0.0);
        }

        // After suppression expires, normal threshold applies
        let result = det.process_sample(0.2);
        assert!(
            result.is_some(),
            "Should trigger normally after suppression expires"
        );
    }

    #[test]
    fn test_crosstalk_suppression_extends_window() {
        let mut det = make_trigger_detector(0.1, 0, 0);

        // Apply 50 samples of suppression
        det.apply_crosstalk_suppression(50, 3.0);

        // Burn 30 samples
        for _ in 0..30 {
            det.process_sample(0.0);
        }

        // Extend to 100 samples (should be 100 from now, not 20 remaining)
        det.apply_crosstalk_suppression(100, 3.0);

        // After 50 more samples (80 total from start), should still be suppressed
        for _ in 0..50 {
            det.process_sample(0.0);
        }

        let result = det.process_sample(0.2);
        assert!(
            result.is_none(),
            "Should still be suppressed after window extension"
        );
    }

    #[test]
    fn test_adaptive_noise_floor_raises_threshold() {
        // With sensitivity=5.0 and noise at 0.05, adaptive floor = ~0.25.
        // A signal at 0.3 should barely trigger (just above 0.25).
        let mut det = TriggerDetector::new(
            44100,
            0.1, // base threshold
            0,   // no lockout
            0,   // no scan
            1.0,
            VelocityCurve::Linear,
            127,
            Some("kick".to_string()),
            None,
            TriggerInputAction::Trigger,
            None,
            None,
            Some(5.0), // noise floor sensitivity
            200,       // 200ms decay
        );

        // Feed noise at 0.05 for 1 second to let EMA converge.
        let noise_samples = 44100;
        for _ in 0..noise_samples {
            assert!(det.process_sample(0.05).is_none());
        }

        // EMA should have converged near 0.05, adaptive floor ≈ 0.25.
        // Signal at 0.2 should NOT trigger (below adaptive floor).
        let result = det.process_sample(0.2);
        assert!(
            result.is_none(),
            "Signal below adaptive floor should not trigger"
        );

        // Signal at 0.3 should trigger (above adaptive floor ≈ 0.25).
        let result = det.process_sample(0.3);
        assert!(
            result.is_some(),
            "Signal above adaptive floor should trigger"
        );
    }

    #[test]
    fn test_adaptive_noise_floor_disabled_by_default() {
        // With sensitivity=None, behavior is identical to before.
        let mut det = make_trigger_detector(0.1, 0, 0);

        // Feed noise at 0.05 for a while.
        for _ in 0..4410 {
            assert!(det.process_sample(0.05).is_none());
        }

        // Signal at 0.2 should trigger immediately (no adaptive floor).
        let result = det.process_sample(0.2);
        assert!(
            result.is_some(),
            "Without adaptive noise floor, 0.2 should trigger above 0.1 threshold"
        );
    }

    #[test]
    fn test_adaptive_noise_floor_frozen_during_scanning() {
        // Verify that noise EMA doesn't update during Scanning state.
        let mut det = TriggerDetector::new(
            44100,
            0.1, // base threshold
            100, // 100ms lockout
            5,   // 5ms scan
            1.0,
            VelocityCurve::Linear,
            127,
            Some("kick".to_string()),
            None,
            TriggerInputAction::Trigger,
            None,
            None,
            Some(5.0),
            200,
        );

        // Let EMA settle at low noise.
        for _ in 0..4410 {
            det.process_sample(0.01);
        }

        // Cross threshold to enter Scanning. The threshold-crossing sample
        // is the last one processed while still in Idle, so capture EMA after it.
        det.process_sample(0.5);
        let ema_after_crossing = det.noise_floor_ema;

        // Feed high amplitude during the scan window.
        let scan_samples = ((5.0_f64 * 44100.0) / 1000.0).ceil() as u32;
        for _ in 0..scan_samples {
            det.process_sample(0.8);
        }

        // EMA should not have changed during Scanning.
        assert!(
            (det.noise_floor_ema - ema_after_crossing).abs() < 1e-6,
            "Noise floor EMA should be frozen during Scanning state"
        );
    }

    #[test]
    fn test_adaptive_noise_floor_frozen_during_lockout() {
        let mut det = TriggerDetector::new(
            44100,
            0.1, // base threshold
            100, // 100ms lockout
            0,   // no scan
            1.0,
            VelocityCurve::Linear,
            127,
            Some("kick".to_string()),
            None,
            TriggerInputAction::Trigger,
            None,
            None,
            Some(5.0),
            200,
        );

        // Let EMA settle at low noise.
        for _ in 0..4410 {
            det.process_sample(0.01);
        }

        // Fire a trigger to enter Lockout.
        assert!(det.process_sample(0.5).is_some());
        let ema_after_fire = det.noise_floor_ema;

        // Feed high amplitude during lockout.
        let lockout_samples = ((100.0_f64 * 44100.0) / 1000.0).ceil() as u32;
        for _ in 0..lockout_samples {
            det.process_sample(0.9);
        }

        // EMA should not have changed during Lockout.
        assert!(
            (det.noise_floor_ema - ema_after_fire).abs() < 1e-6,
            "Noise floor EMA should be frozen during Lockout state"
        );
    }

    #[test]
    fn test_highpass_filter_path() {
        // Create detector with highpass enabled to cover the Some(hpf) branch
        let mut det = TriggerDetector::new(
            44100,
            0.1,
            0,
            0,
            1.0,
            VelocityCurve::Linear,
            127,
            Some("kick".to_string()),
            None,
            TriggerInputAction::Trigger,
            Some(80.0), // Enable highpass at 80Hz
            None,
            None,
            200,
        );

        // A strong transient should still trigger through the highpass
        let result = det.process_sample(0.9);
        assert!(result.is_some());
    }

    #[test]
    fn test_logarithmic_velocity_at_threshold() {
        // Tests the `peak <= threshold` branch returning velocity 1
        let mut det = TriggerDetector::new(
            44100,
            0.1,
            0,
            0,
            1.0,
            VelocityCurve::Logarithmic,
            127,
            Some("kick".to_string()),
            None,
            TriggerInputAction::Trigger,
            None,
            None,
            None,
            200,
        );

        // Trigger with a sample barely above threshold — the peak recorded
        // will be just above threshold. The amplitude_to_velocity sees the peak.
        // With scan=0 and lockout=0, fire() is called with the raw amplitude.
        let event = expect_trigger(det.process_sample(0.1001));
        // At threshold, logarithmic returns 1
        assert!(event.velocity <= 2);
    }

    #[test]
    fn test_logarithmic_velocity_threshold_equals_one() {
        // Tests the `range < EPSILON` branch returning velocity 127
        let mut det = TriggerDetector::new(
            44100,
            1.0, // threshold = 1.0, so range = 1.0 - 1.0 = 0.0
            0,
            0,
            2.0, // gain=2 so amplitude of 0.6 * 2 = 1.2 > threshold 1.0
            VelocityCurve::Logarithmic,
            127,
            Some("kick".to_string()),
            None,
            TriggerInputAction::Trigger,
            None,
            None,
            None,
            200,
        );

        let event = expect_trigger(det.process_sample(0.6));
        assert_eq!(event.velocity, 127);
    }

    #[test]
    fn test_adaptive_noise_floor_recovers_after_noise_drops() {
        let mut det = TriggerDetector::new(
            44100,
            0.1,
            0,
            0,
            1.0,
            VelocityCurve::Linear,
            127,
            Some("kick".to_string()),
            None,
            TriggerInputAction::Trigger,
            None,
            None,
            Some(5.0),
            50, // fast 50ms decay for test
        );

        // Feed high noise to raise adaptive floor.
        for _ in 0..44100 {
            det.process_sample(0.08);
        }

        // Adaptive floor should be ~0.4 (0.08 * 5.0), so 0.3 won't trigger.
        let result = det.process_sample(0.3);
        assert!(result.is_none(), "Should not trigger during high noise");

        // Feed silence for 2 seconds to let EMA decay.
        for _ in 0..(44100 * 2) {
            det.process_sample(0.0);
        }

        // Adaptive floor should have decayed back near 0, so 0.2 triggers.
        let result = det.process_sample(0.2);
        assert!(
            result.is_some(),
            "Should trigger after noise floor decays back down"
        );
    }
}
