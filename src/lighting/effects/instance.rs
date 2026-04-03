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

/// An instance of an effect with timing and targeting information.
/// All effects have a finite duration — there are no perpetual or permanent effects.
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
    pub fn new(
        id: String,
        effect_type: EffectType,
        target_fixtures: Vec<String>,
        up_time: Option<Duration>,
        hold_time: Option<Duration>,
        down_time: Option<Duration>,
    ) -> Self {
        // Extract duration from effect_type (None for Dimmer, which uses its own timing model)
        let duration = if matches!(&effect_type, EffectType::Dimmer { .. }) {
            None
        } else {
            Some(effect_type.duration())
        };

        // Use the effect type's duration as hold_time if hold_time wasn't explicitly provided.
        // Dimmer is special: it doesn't use the up/hold/down timing model.
        let default_hold_time = if matches!(&effect_type, EffectType::Dimmer { .. }) {
            None
        } else {
            duration
        };

        let final_hold_time = hold_time.or(default_hold_time);

        Self {
            id,
            effect_type,
            target_fixtures,
            priority: 0,
            layer: EffectLayer::Background,
            blend_mode: BlendMode::Replace,
            start_time: None,
            cue_time: None,
            up_time,
            hold_time: final_hold_time,
            down_time,
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

    /// Calculate the crossfade multiplier for this effect at the given elapsed time.
    /// All effects have a finite up/hold/down lifecycle.
    pub fn calculate_crossfade_multiplier(&self, elapsed: Duration) -> f64 {
        let up_time = self.up_time.unwrap_or(Duration::ZERO);
        let hold_time = self.hold_time.unwrap_or(Duration::ZERO);
        let down_time = self.down_time.unwrap_or(Duration::ZERO);

        let up_end = up_time;
        let hold_end = up_time + hold_time;
        let total_end = up_time + hold_time + down_time;

        // Small epsilon to make boundary checks inclusive and avoid flapping
        let eps = Duration::from_micros(1);

        if up_time.is_zero() {
            // No fade in phase - go directly to hold or fade out
            if elapsed <= hold_end + eps {
                // Hold phase (100%)
                1.0
            } else if !down_time.is_zero() && elapsed < total_end + eps {
                // Fade out phase (100% to 0%)
                let fade_out_elapsed = elapsed.saturating_sub(hold_end);
                let t = (fade_out_elapsed.as_secs_f64() / down_time.as_secs_f64()).clamp(0.0, 1.0);
                1.0 - t
            } else if elapsed > total_end + eps {
                // Effect has ended
                0.0
            } else {
                1.0
            }
        } else if elapsed < up_end + eps {
            // Fade in phase (0% to 100%)
            (elapsed.as_secs_f64() / up_time.as_secs_f64()).clamp(0.0, 1.0)
        } else if elapsed <= hold_end + eps {
            // Hold phase (100%)
            1.0
        } else if !down_time.is_zero() && elapsed < total_end + eps {
            // Fade out phase (100% to 0%)
            let fade_out_elapsed = elapsed.saturating_sub(hold_end);
            let t = (fade_out_elapsed.as_secs_f64() / down_time.as_secs_f64()).clamp(0.0, 1.0);
            1.0 - t
        } else if elapsed > total_end + eps {
            // Effect has ended
            0.0
        } else {
            1.0
        }
    }

    /// Get the total duration of this effect (up_time + hold_time + down_time).
    /// All effects have a finite duration.
    pub fn total_duration(&self) -> Duration {
        // For dimmers, use the effect's own duration field (timing params not used)
        if let EffectType::Dimmer { duration, .. } = &self.effect_type {
            return *duration;
        }

        self.up_time.unwrap_or(Duration::ZERO)
            + self.hold_time.unwrap_or(Duration::ZERO)
            + self.down_time.unwrap_or(Duration::ZERO)
    }

    /// Determine if the effect has reached its intended terminal state for the given elapsed time.
    pub fn has_reached_terminal_state(&self, elapsed: Duration) -> bool {
        let eps = Duration::from_micros(1);
        match &self.effect_type {
            EffectType::Dimmer { duration, .. } => elapsed + eps >= *duration,
            _ => {
                let d = self.total_duration();
                elapsed + eps >= d
            }
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

    fn static_effect(duration: Duration) -> EffectType {
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

    fn strobe_effect(duration: Duration) -> EffectType {
        EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(10.0),
            duration,
        }
    }

    fn color_cycle_effect(duration: Duration) -> EffectType {
        EffectType::ColorCycle {
            colors: vec![Color::new(255, 0, 0), Color::new(0, 0, 255)],
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Fade,
            duration,
        }
    }

    fn chase_effect(duration: Duration) -> EffectType {
        EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration,
        }
    }

    fn rainbow_effect(duration: Duration) -> EffectType {
        EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(0.5),
            saturation: 1.0,
            brightness: 1.0,
            duration,
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

    // ── new — timing defaults ──────────────────────────────────────

    #[test]
    fn new_static_sets_hold_from_duration() {
        let inst = make_instance(static_effect(Duration::from_secs(5)));
        assert_eq!(inst.hold_time, Some(Duration::from_secs(5)));
    }

    #[test]
    fn new_strobe_sets_hold_from_duration() {
        let inst = make_instance(strobe_effect(Duration::from_secs(3)));
        assert_eq!(inst.hold_time, Some(Duration::from_secs(3)));
    }

    #[test]
    fn new_color_cycle_sets_hold_from_duration() {
        let inst = make_instance(color_cycle_effect(Duration::from_secs(10)));
        assert_eq!(inst.hold_time, Some(Duration::from_secs(10)));
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
        let inst = make_instance(static_effect(Duration::from_secs(5)));
        assert_eq!(inst.layer, EffectLayer::Background);
        assert_eq!(inst.blend_mode, BlendMode::Replace);
        assert_eq!(inst.priority, 0);
        assert!(inst.enabled);
    }

    #[test]
    fn new_with_user_timing_overrides() {
        let inst = make_instance_timed(
            static_effect(Duration::from_secs(5)),
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
    fn crossfade_hold_only() {
        let inst = make_instance(static_effect(Duration::from_secs(2)));
        // hold=2s → at t=0 → 1.0, at t=1s → 1.0, at t=3s → 0.0
        assert!((inst.calculate_crossfade_multiplier(Duration::ZERO) - 1.0).abs() < 1e-9);
        assert!((inst.calculate_crossfade_multiplier(Duration::from_secs(1)) - 1.0).abs() < 1e-9);
        assert!((inst.calculate_crossfade_multiplier(Duration::from_secs(3)) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn crossfade_with_hold_and_down() {
        let inst = make_instance_timed(
            static_effect(Duration::from_secs(5)),
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
    fn crossfade_fade_in_phase() {
        let inst = make_instance_timed(
            static_effect(Duration::from_secs(5)),
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
            static_effect(Duration::from_secs(5)),
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
            static_effect(Duration::from_secs(5)),
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
            static_effect(Duration::from_secs(5)),
            Some(Duration::from_secs(1)),
            Some(Duration::from_secs(1)),
            Some(Duration::from_secs(1)),
        );
        // total = 3s, at t=4s → 0.0
        let mult = inst.calculate_crossfade_multiplier(Duration::from_secs(4));
        assert!((mult - 0.0).abs() < 1e-9);
    }

    // ── total_duration ─────────────────────────────────────────────

    #[test]
    fn total_duration_static() {
        let inst = make_instance(static_effect(Duration::from_secs(5)));
        assert_eq!(inst.total_duration(), Duration::from_secs(5));
    }

    #[test]
    fn total_duration_static_with_up_down() {
        let inst = make_instance_timed(
            static_effect(Duration::from_secs(5)),
            Some(Duration::from_secs(1)),
            None, // hold defaults to effect duration
            Some(Duration::from_secs(1)),
        );
        // up(1) + hold(5) + down(1) = 7
        assert_eq!(inst.total_duration(), Duration::from_secs(7));
    }

    #[test]
    fn total_duration_dimmer() {
        let inst = make_instance(dimmer_effect(0.0, 1.0, Duration::from_secs(3)));
        assert_eq!(inst.total_duration(), Duration::from_secs(3));
    }

    #[test]
    fn total_duration_strobe() {
        let inst = make_instance(strobe_effect(Duration::from_secs(5)));
        assert_eq!(inst.total_duration(), Duration::from_secs(5));
    }

    #[test]
    fn total_duration_color_cycle() {
        let inst = make_instance(color_cycle_effect(Duration::from_secs(10)));
        assert_eq!(inst.total_duration(), Duration::from_secs(10));
    }

    #[test]
    fn total_duration_chase() {
        let inst = make_instance(chase_effect(Duration::from_secs(8)));
        assert_eq!(inst.total_duration(), Duration::from_secs(8));
    }

    #[test]
    fn total_duration_rainbow() {
        let inst = make_instance(rainbow_effect(Duration::from_secs(6)));
        assert_eq!(inst.total_duration(), Duration::from_secs(6));
    }

    #[test]
    fn total_duration_with_timing_override() {
        let inst = make_instance_timed(
            static_effect(Duration::from_secs(5)),
            Some(Duration::from_secs(1)),
            Some(Duration::from_secs(2)),
            Some(Duration::from_secs(1)),
        );
        // up(1) + hold(2, overridden) + down(1) = 4
        assert_eq!(inst.total_duration(), Duration::from_secs(4));
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
    fn terminal_static_at_end() {
        let inst = make_instance(static_effect(Duration::from_secs(3)));
        assert!(!inst.has_reached_terminal_state(Duration::from_secs(2)));
        assert!(inst.has_reached_terminal_state(Duration::from_secs(3)));
    }

    #[test]
    fn terminal_strobe_at_end() {
        let inst = make_instance(strobe_effect(Duration::from_secs(2)));
        assert!(!inst.has_reached_terminal_state(Duration::from_secs(1)));
        assert!(inst.has_reached_terminal_state(Duration::from_secs(2)));
    }

    #[test]
    fn terminal_color_cycle_at_end() {
        let inst = make_instance(color_cycle_effect(Duration::from_secs(5)));
        assert!(!inst.has_reached_terminal_state(Duration::from_secs(3)));
        assert!(inst.has_reached_terminal_state(Duration::from_secs(5)));
    }

    // ── builder helpers ────────────────────────────────────────────

    #[test]
    fn with_priority_sets_priority() {
        let inst = make_instance(static_effect(Duration::from_secs(5))).with_priority(5);
        assert_eq!(inst.priority, 5);
    }

    #[test]
    fn with_timing_sets_fields() {
        let now = Instant::now();
        let inst = make_instance(static_effect(Duration::from_secs(5)))
            .with_timing(Some(now), Some(Duration::from_secs(10)));
        assert_eq!(inst.start_time, Some(now));
        assert_eq!(inst.hold_time, Some(Duration::from_secs(10)));
    }
}
