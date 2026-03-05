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

use std::time::{Duration, Instant};

use super::types::{BlendMode, EffectLayer, EffectType};

/// An instance of an effect with timing and targeting information
#[derive(Debug, Clone)]
pub struct EffectInstance {
    pub id: String,
    pub effect_type: EffectType,
    pub target_fixtures: Vec<String>, // Fixture names or group names
    pub priority: u8,                 // Higher priority overrides lower
    pub layer: EffectLayer,           // Layer for layering system
    pub blend_mode: BlendMode,        // How to blend with other effects
    pub start_time: Option<Instant>,  // Real-time instant when effect started
    pub cue_time: Option<Duration>, // Song time when effect was supposed to start (for deterministic randomness)
    pub up_time: Option<Duration>,  // Fade in duration (0% to 100%)
    pub hold_time: Option<Duration>, // Time at full intensity (100%)
    pub down_time: Option<Duration>, // Fade out duration (100% to 0%)
    pub enabled: bool,
}

impl EffectInstance {
    /// Determine if this effect is permanent (state-changing) or temporary (show effect)
    pub fn is_permanent(&self) -> bool {
        match &self.effect_type {
            EffectType::Static { duration, .. } => {
                // Static effects are permanent only if they have no duration AND no timing parameters
                duration.is_none()
                    && self.up_time.is_none()
                    && self.hold_time.is_none()
                    && self.down_time.is_none()
            }
            // Dimmer effects are always permanent so their resulting brightness persists
            EffectType::Dimmer { .. } => true,
            EffectType::ColorCycle { .. } => false, // Cycles complete and end
            EffectType::Strobe { .. } => false,     // Strobe completes and end
            EffectType::Chase { .. } => false,      // Chases complete and end
            EffectType::Rainbow { .. } => false,    // Rainbow cycles complete and end
            EffectType::Pulse { .. } => false,      // Pulse cycles complete and end
        }
    }

    pub fn new(
        id: String,
        effect_type: EffectType,
        target_fixtures: Vec<String>,
        up_time: Option<Duration>,
        hold_time: Option<Duration>,
        down_time: Option<Duration>,
    ) -> Self {
        // Extract duration from effect_type if available
        // Effects without an explicit duration are perpetual until replaced
        let duration = match &effect_type {
            EffectType::Static { duration, .. } => *duration,
            EffectType::Dimmer { duration, .. } => Some(*duration), // Dimmer duration becomes up_time
            EffectType::ColorCycle { .. } => None,                  // Perpetual until replaced
            EffectType::Strobe { duration, .. } => *duration,
            EffectType::Chase { .. } => None, // Perpetual until replaced
            EffectType::Rainbow { .. } => None, // Perpetual until replaced
            EffectType::Pulse { duration, .. } => *duration,
        };

        // Determine timing based on effect type, but allow override from parameters
        let (default_up_time, default_hold_time, default_down_time) = match &effect_type {
            EffectType::Dimmer { .. } => (None, None, None), // Dimmer uses its duration field
            EffectType::Static { duration: None, .. } => {
                // If timing parameters are provided, treat as timed effect
                if up_time.is_some() || hold_time.is_some() || down_time.is_some() {
                    // Use provided timing parameters for timed static effect
                    (up_time, hold_time, down_time)
                } else {
                    (None, None, None) // Truly indefinite static effect
                }
            }
            EffectType::Static {
                duration: Some(_), ..
            } => (None, duration, None), // Static effects with duration just hold for that duration
            _ => (None, duration, None), // Effects without duration are perpetual until replaced
        };

        // Use provided timing or fall back to defaults
        let final_up_time = up_time.or(default_up_time);
        let final_hold_time = hold_time.or(default_hold_time);
        let final_down_time = down_time.or(default_down_time);

        Self {
            id,
            effect_type,
            target_fixtures,
            priority: 0,
            layer: EffectLayer::Background,
            blend_mode: BlendMode::Replace,
            start_time: None,
            cue_time: None,
            up_time: final_up_time,
            hold_time: final_hold_time,
            down_time: final_down_time,
            enabled: true,
        }
    }

    #[cfg(test)]
    pub fn with_priority(mut self, priority: u8) -> Self {
        self.priority = priority;
        self
    }

    #[cfg(test)]
    pub fn with_timing(mut self, start_time: Option<Instant>, hold_time: Option<Duration>) -> Self {
        self.start_time = start_time;
        self.hold_time = hold_time;
        self
    }

    /// Calculate the crossfade multiplier for this effect at the given elapsed time
    pub fn calculate_crossfade_multiplier(&self, elapsed: Duration) -> f64 {
        // elapsed is the time since the effect started

        let up_time = self.up_time.unwrap_or(Duration::from_secs(0));
        let hold_time = self.hold_time.unwrap_or(Duration::from_secs(0));
        let down_time = self.down_time.unwrap_or(Duration::from_secs(0));

        let up_end = up_time;
        let hold_end = up_time + hold_time;
        let total_end = up_time + hold_time + down_time;

        // Small epsilon to make boundary checks inclusive and avoid flapping
        let eps = Duration::from_micros(1);

        // Check if this is an indefinite effect (no hold_time and no down_time)
        let is_indefinite = hold_time.is_zero() && down_time.is_zero();

        if up_time.is_zero() {
            // No fade in phase - go directly to hold or fade out
            if is_indefinite {
                // Indefinite effect (like static effects) - always at full intensity
                1.0
            } else if elapsed <= hold_end + eps {
                // Hold phase (100%)
                1.0
            } else if elapsed < total_end + eps {
                // Fade out phase (100% to 0%)
                let fade_out_elapsed = elapsed.saturating_sub(hold_end);
                let t = (fade_out_elapsed.as_secs_f64() / down_time.as_secs_f64()).clamp(0.0, 1.0);
                1.0 - t
            } else {
                // Effect has ended
                0.0
            }
        } else if elapsed < up_end + eps {
            // Fade in phase (0% to 100%)
            (elapsed.as_secs_f64() / up_time.as_secs_f64()).clamp(0.0, 1.0)
        } else if is_indefinite {
            // Indefinite effect after fade-in - always at full intensity
            1.0
        } else if elapsed <= hold_end + eps {
            // Hold phase (100%)
            1.0
        } else if elapsed < total_end + eps {
            // Fade out phase (100% to 0%)
            if down_time.is_zero() {
                0.0
            } else {
                let fade_out_elapsed = elapsed.saturating_sub(hold_end);
                let t = (fade_out_elapsed.as_secs_f64() / down_time.as_secs_f64()).clamp(0.0, 1.0);
                1.0 - t
            }
        } else {
            // Effect has ended
            0.0
        }
    }

    /// Get the total duration of this effect (up_time + hold_time + down_time)
    /// Returns None for indefinite/perpetual effects (effects without explicit duration or timing)
    pub fn total_duration(&self) -> Option<Duration> {
        // Check if this is an indefinite effect (no hold_time and no down_time)
        // This matches the semantics in calculate_crossfade_multiplier()
        // An effect with only up_time (fade-in) but no hold/down time runs indefinitely
        let hold = self.hold_time.unwrap_or(Duration::from_secs(0));
        let down = self.down_time.unwrap_or(Duration::from_secs(0));
        let is_indefinite = hold.is_zero() && down.is_zero();

        // Effects are perpetual if they are indefinite AND have no explicit duration
        if is_indefinite {
            match &self.effect_type {
                // Static effects with no duration are perpetual
                EffectType::Static { duration: None, .. } => return None,
                // ColorCycle, Chase, Rainbow have no duration field - perpetual by design
                EffectType::ColorCycle { .. } => return None,
                EffectType::Chase { .. } => return None,
                EffectType::Rainbow { .. } => return None,
                // Strobe and Pulse with no duration are perpetual
                EffectType::Strobe { duration: None, .. } => return None,
                EffectType::Pulse { duration: None, .. } => return None,
                _ => {} // Fall through to calculate duration
            }
        }

        // For dimmers, use duration field (timing params not used)
        if let EffectType::Dimmer { duration, .. } = &self.effect_type {
            return Some(*duration);
        }

        let duration = self.up_time.unwrap_or(Duration::from_secs(0))
            + self.hold_time.unwrap_or(Duration::from_secs(0))
            + self.down_time.unwrap_or(Duration::from_secs(0));

        Some(duration)
    }

    /// Determine if the effect has reached its intended terminal state for the given elapsed time
    /// This prefers value-based completion when applicable (e.g., dimmer hitting end level).
    pub fn has_reached_terminal_state(&self, elapsed: Duration) -> bool {
        let eps = Duration::from_micros(1);
        let value_eps = 1e-9; // small epsilon for floating-point comparison
        match &self.effect_type {
            EffectType::Dimmer {
                duration,
                start_level,
                end_level,
                ..
            } => {
                // Dimmer effect completes when end_level is reached
                if duration.is_zero() {
                    return true; // Instant transition
                }

                // Terminal when we've reached end_level
                let progress = (elapsed.as_secs_f64() / duration.as_secs_f64()).clamp(0.0, 1.0);
                let value = start_level + (end_level - start_level) * progress;
                (value - *end_level).abs() <= value_eps
            }
            EffectType::Static { .. } => {
                // Use total_duration() to include hold_time, up_time, and down_time
                // This ensures static effects with hold_time expire correctly
                self.total_duration()
                    .map(|d| elapsed + eps >= d)
                    .unwrap_or(false)
            }
            EffectType::Strobe { duration, .. } => {
                duration.map(|d| elapsed + eps >= d).unwrap_or(false)
            }
            EffectType::Pulse { duration, .. } => {
                duration.map(|d| elapsed + eps >= d).unwrap_or(false)
            }
            // Cycle-like effects terminate at configured duration
            EffectType::ColorCycle { .. } => self
                .total_duration()
                .map(|d| elapsed + eps >= d)
                .unwrap_or(false),
            EffectType::Chase { .. } => self
                .total_duration()
                .map(|d| elapsed + eps >= d)
                .unwrap_or(false),
            EffectType::Rainbow { .. } => self
                .total_duration()
                .map(|d| elapsed + eps >= d)
                .unwrap_or(false),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::lighting::effects::color::Color;
    use crate::lighting::effects::tempo_aware::{TempoAwareFrequency, TempoAwareSpeed};
    use crate::lighting::effects::types::{
        ChaseDirection, ChasePattern, CycleDirection, CycleTransition, DimmerCurve,
    };

    fn static_effect(duration: Option<Duration>) -> EffectType {
        EffectType::Static {
            parameters: HashMap::new(),
            duration,
        }
    }

    fn dimmer_effect(start: f64, end: f64, dur: Duration) -> EffectType {
        EffectType::Dimmer {
            start_level: start,
            end_level: end,
            duration: dur,
            curve: DimmerCurve::Linear,
        }
    }

    fn strobe_effect(duration: Option<Duration>) -> EffectType {
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(10.0),
            duration,
        }
    }

    fn pulse_effect(duration: Option<Duration>) -> EffectType {
        EffectType::Pulse {
            base_level: 0.0,
            pulse_amplitude: 1.0,
            frequency: TempoAwareFrequency::Fixed(2.0),
            duration,
        }
    }

    fn color_cycle_effect() -> EffectType {
        EffectType::ColorCycle {
            colors: vec![Color::new(255, 0, 0), Color::new(0, 0, 255)],
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Fade,
        }
    }

    fn chase_effect() -> EffectType {
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
        }
    }

    fn rainbow_effect() -> EffectType {
        EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(0.5),
            saturation: 1.0,
            brightness: 1.0,
        }
    }

    fn make_instance(effect_type: EffectType) -> EffectInstance {
        EffectInstance::new(
            "test".to_string(),
            effect_type,
            vec!["fixture1".to_string()],
            None,
            None,
            None,
        )
    }

    fn make_instance_timed(
        effect_type: EffectType,
        up: Option<Duration>,
        hold: Option<Duration>,
        down: Option<Duration>,
    ) -> EffectInstance {
        EffectInstance::new(
            "test".to_string(),
            effect_type,
            vec!["fixture1".to_string()],
            up,
            hold,
            down,
        )
    }

    // ── is_permanent ───────────────────────────────────────────────

    #[test]
    fn is_permanent_static_no_duration() {
        let inst = make_instance(static_effect(None));
        assert!(inst.is_permanent());
    }

    #[test]
    fn is_permanent_static_with_duration() {
        let inst = make_instance(static_effect(Some(Duration::from_secs(5))));
        assert!(!inst.is_permanent());
    }

    #[test]
    fn is_permanent_static_with_timing_params() {
        let inst = make_instance_timed(
            static_effect(None),
            Some(Duration::from_secs(1)),
            None,
            None,
        );
        assert!(!inst.is_permanent()); // has up_time
    }

    #[test]
    fn is_permanent_dimmer_always() {
        let inst = make_instance(dimmer_effect(0.0, 1.0, Duration::from_secs(2)));
        assert!(inst.is_permanent());
    }

    #[test]
    fn is_permanent_strobe_false() {
        let inst = make_instance(strobe_effect(None));
        assert!(!inst.is_permanent());
    }

    #[test]
    fn is_permanent_color_cycle_false() {
        let inst = make_instance(color_cycle_effect());
        assert!(!inst.is_permanent());
    }

    #[test]
    fn is_permanent_chase_false() {
        let inst = make_instance(chase_effect());
        assert!(!inst.is_permanent());
    }

    #[test]
    fn is_permanent_rainbow_false() {
        let inst = make_instance(rainbow_effect());
        assert!(!inst.is_permanent());
    }

    #[test]
    fn is_permanent_pulse_false() {
        let inst = make_instance(pulse_effect(None));
        assert!(!inst.is_permanent());
    }

    // ── new — timing defaults ──────────────────────────────────────

    #[test]
    fn new_static_no_duration_indefinite() {
        let inst = make_instance(static_effect(None));
        assert!(inst.up_time.is_none());
        assert!(inst.hold_time.is_none());
        assert!(inst.down_time.is_none());
    }

    #[test]
    fn new_static_with_duration_sets_hold() {
        let inst = make_instance(static_effect(Some(Duration::from_secs(5))));
        assert_eq!(inst.hold_time, Some(Duration::from_secs(5)));
    }

    #[test]
    fn new_dimmer_no_default_timing() {
        let inst = make_instance(dimmer_effect(0.0, 1.0, Duration::from_secs(3)));
        assert!(inst.up_time.is_none());
        assert!(inst.hold_time.is_none());
        assert!(inst.down_time.is_none());
    }

    #[test]
    fn new_defaults_layer_and_blend() {
        let inst = make_instance(static_effect(None));
        assert_eq!(inst.layer, EffectLayer::Background);
        assert_eq!(inst.blend_mode, BlendMode::Replace);
        assert_eq!(inst.priority, 0);
        assert!(inst.enabled);
    }

    #[test]
    fn new_with_user_timing_overrides() {
        let inst = make_instance_timed(
            static_effect(None),
            Some(Duration::from_secs(1)),
            Some(Duration::from_secs(2)),
            Some(Duration::from_secs(1)),
        );
        assert_eq!(inst.up_time, Some(Duration::from_secs(1)));
        assert_eq!(inst.hold_time, Some(Duration::from_secs(2)));
        assert_eq!(inst.down_time, Some(Duration::from_secs(1)));
    }

    // ── calculate_crossfade_multiplier ─────────────────────────────

    #[test]
    fn crossfade_indefinite_always_one() {
        let inst = make_instance(static_effect(None));
        assert!((inst.calculate_crossfade_multiplier(Duration::ZERO) - 1.0).abs() < 1e-9);
        assert!(
            (inst.calculate_crossfade_multiplier(Duration::from_secs(1000)) - 1.0).abs() < 1e-9
        );
    }

    #[test]
    fn crossfade_with_hold_only() {
        let inst = make_instance_timed(
            static_effect(None),
            None,
            Some(Duration::from_secs(2)),
            None,
        );
        // hold_time=2s, no down_time → hold_end=2s, total_end=2s
        // At t=0 → hold phase → 1.0
        assert!((inst.calculate_crossfade_multiplier(Duration::ZERO) - 1.0).abs() < 1e-9);
        // At t=1s → hold phase → 1.0
        assert!((inst.calculate_crossfade_multiplier(Duration::from_secs(1)) - 1.0).abs() < 1e-9);
        // At t=3s → past total_end → 0.0
        assert!((inst.calculate_crossfade_multiplier(Duration::from_secs(3)) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn crossfade_fade_in_phase() {
        let inst = make_instance_timed(
            static_effect(None),
            Some(Duration::from_secs(2)),
            Some(Duration::from_secs(2)),
            None,
        );
        // At t=1s, midway through 2s fade-in → 0.5
        let mult = inst.calculate_crossfade_multiplier(Duration::from_secs(1));
        assert!((mult - 0.5).abs() < 1e-9);
    }

    #[test]
    fn crossfade_hold_phase_after_fade_in() {
        let inst = make_instance_timed(
            static_effect(None),
            Some(Duration::from_secs(1)),
            Some(Duration::from_secs(2)),
            None,
        );
        // After 1s fade-in, at t=2s → in hold phase → 1.0
        let mult = inst.calculate_crossfade_multiplier(Duration::from_secs(2));
        assert!((mult - 1.0).abs() < 1e-9);
    }

    #[test]
    fn crossfade_fade_out_phase() {
        let inst = make_instance_timed(
            static_effect(None),
            None,
            Some(Duration::from_secs(1)),
            Some(Duration::from_secs(2)),
        );
        // hold=1s, down=2s → hold_end=1s, total_end=3s
        // At t=2s → 1s into 2s fade-out → 0.5
        let mult = inst.calculate_crossfade_multiplier(Duration::from_secs(2));
        assert!((mult - 0.5).abs() < 1e-9);
    }

    #[test]
    fn crossfade_after_total_end() {
        let inst = make_instance_timed(
            static_effect(None),
            Some(Duration::from_secs(1)),
            Some(Duration::from_secs(1)),
            Some(Duration::from_secs(1)),
        );
        // total = 3s, at t=4s → 0.0
        let mult = inst.calculate_crossfade_multiplier(Duration::from_secs(4));
        assert!((mult - 0.0).abs() < 1e-9);
    }

    #[test]
    fn crossfade_fade_in_then_indefinite() {
        let inst = make_instance_timed(
            static_effect(None),
            Some(Duration::from_secs(2)),
            None,
            None,
        );
        // Fade-in only, no hold/down → indefinite after fade-in
        // At t=1s → 0.5 (fade-in)
        assert!((inst.calculate_crossfade_multiplier(Duration::from_secs(1)) - 0.5).abs() < 1e-9);
        // At t=3s → past fade-in, indefinite → 1.0
        assert!((inst.calculate_crossfade_multiplier(Duration::from_secs(3)) - 1.0).abs() < 1e-9);
    }

    // ── total_duration ─────────────────────────────────────────────

    #[test]
    fn total_duration_perpetual_static() {
        let inst = make_instance(static_effect(None));
        assert_eq!(inst.total_duration(), None);
    }

    #[test]
    fn total_duration_perpetual_color_cycle() {
        let inst = make_instance(color_cycle_effect());
        assert_eq!(inst.total_duration(), None);
    }

    #[test]
    fn total_duration_perpetual_chase() {
        let inst = make_instance(chase_effect());
        assert_eq!(inst.total_duration(), None);
    }

    #[test]
    fn total_duration_perpetual_rainbow() {
        let inst = make_instance(rainbow_effect());
        assert_eq!(inst.total_duration(), None);
    }

    #[test]
    fn total_duration_perpetual_strobe_no_duration() {
        let inst = make_instance(strobe_effect(None));
        assert_eq!(inst.total_duration(), None);
    }

    #[test]
    fn total_duration_perpetual_pulse_no_duration() {
        let inst = make_instance(pulse_effect(None));
        assert_eq!(inst.total_duration(), None);
    }

    #[test]
    fn total_duration_dimmer_uses_effect_duration() {
        let inst = make_instance(dimmer_effect(0.0, 1.0, Duration::from_secs(3)));
        assert_eq!(inst.total_duration(), Some(Duration::from_secs(3)));
    }

    #[test]
    fn total_duration_timed_static() {
        let inst = make_instance_timed(
            static_effect(None),
            Some(Duration::from_secs(1)),
            Some(Duration::from_secs(2)),
            Some(Duration::from_secs(1)),
        );
        assert_eq!(inst.total_duration(), Some(Duration::from_secs(4)));
    }

    #[test]
    fn total_duration_strobe_with_duration() {
        // Strobe with duration → hold_time gets set via new()
        let inst = make_instance(strobe_effect(Some(Duration::from_secs(5))));
        assert_eq!(inst.total_duration(), Some(Duration::from_secs(5)));
    }

    // ── has_reached_terminal_state ─────────────────────────────────

    #[test]
    fn terminal_dimmer_zero_duration() {
        let inst = make_instance(dimmer_effect(0.0, 1.0, Duration::ZERO));
        assert!(inst.has_reached_terminal_state(Duration::ZERO));
    }

    #[test]
    fn terminal_dimmer_at_end() {
        let inst = make_instance(dimmer_effect(0.0, 1.0, Duration::from_secs(2)));
        assert!(inst.has_reached_terminal_state(Duration::from_secs(2)));
    }

    #[test]
    fn terminal_dimmer_not_yet() {
        let inst = make_instance(dimmer_effect(0.0, 1.0, Duration::from_secs(2)));
        assert!(!inst.has_reached_terminal_state(Duration::from_secs(1)));
    }

    #[test]
    fn terminal_static_perpetual_never() {
        let inst = make_instance(static_effect(None));
        assert!(!inst.has_reached_terminal_state(Duration::from_secs(1000)));
    }

    #[test]
    fn terminal_static_timed() {
        let inst = make_instance(static_effect(Some(Duration::from_secs(3))));
        assert!(!inst.has_reached_terminal_state(Duration::from_secs(2)));
        assert!(inst.has_reached_terminal_state(Duration::from_secs(3)));
    }

    #[test]
    fn terminal_strobe_no_duration_never() {
        let inst = make_instance(strobe_effect(None));
        assert!(!inst.has_reached_terminal_state(Duration::from_secs(1000)));
    }

    #[test]
    fn terminal_strobe_with_duration() {
        let inst = make_instance(strobe_effect(Some(Duration::from_secs(2))));
        assert!(!inst.has_reached_terminal_state(Duration::from_secs(1)));
        assert!(inst.has_reached_terminal_state(Duration::from_secs(2)));
    }

    #[test]
    fn terminal_pulse_no_duration_never() {
        let inst = make_instance(pulse_effect(None));
        assert!(!inst.has_reached_terminal_state(Duration::from_secs(1000)));
    }

    #[test]
    fn terminal_color_cycle_perpetual_never() {
        let inst = make_instance(color_cycle_effect());
        assert!(!inst.has_reached_terminal_state(Duration::from_secs(1000)));
    }

    #[test]
    fn terminal_chase_perpetual_never() {
        let inst = make_instance(chase_effect());
        assert!(!inst.has_reached_terminal_state(Duration::from_secs(1000)));
    }

    #[test]
    fn terminal_rainbow_perpetual_never() {
        let inst = make_instance(rainbow_effect());
        assert!(!inst.has_reached_terminal_state(Duration::from_secs(1000)));
    }

    // ── builder helpers ────────────────────────────────────────────

    #[test]
    fn with_priority_sets_priority() {
        let inst = make_instance(static_effect(None)).with_priority(5);
        assert_eq!(inst.priority, 5);
    }

    #[test]
    fn with_timing_sets_fields() {
        let now = Instant::now();
        let inst = make_instance(static_effect(None))
            .with_timing(Some(now), Some(Duration::from_secs(10)));
        assert_eq!(inst.start_time, Some(now));
        assert_eq!(inst.hold_time, Some(Duration::from_secs(10)));
    }
}
