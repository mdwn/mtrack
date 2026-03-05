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

use std::time::Duration;

use super::super::effects::{BlendMode, EffectLayer, EffectType};

// Light show DSL data structures
#[derive(Debug, Clone)]
pub struct LightShow {
    pub name: String,
    pub cues: Vec<Cue>,
    pub tempo_map: Option<crate::lighting::tempo::TempoMap>,
}

/// A reusable sequence of cues that can be referenced in shows
#[derive(Debug, Clone)]
pub struct Sequence {
    pub cues: Vec<Cue>,
    /// The effective BPM used when parsing this sequence's internal cue timing.
    /// Used to rescale cue times when the sequence is expanded at a different tempo.
    pub bpm: f64,
}

impl Sequence {
    /// Calculate the duration of this sequence based on when all effects complete
    /// Returns None if the sequence contains perpetual effects (never finishes)
    /// Otherwise returns the time from sequence start (0) to when the last effect completes
    pub fn duration(&self) -> Option<Duration> {
        if self.cues.is_empty() {
            return Some(Duration::ZERO);
        }

        let mut max_completion_time = Duration::ZERO;
        let mut has_any_duration = false;

        for cue in &self.cues {
            for effect in &cue.effects {
                if let Some(effect_duration) = effect.total_duration() {
                    // Effect completes at: cue_time + effect_duration
                    let completion_time = cue.time + effect_duration;
                    if completion_time > max_completion_time {
                        max_completion_time = completion_time;
                    }
                    has_any_duration = true;
                }
                // Perpetual effects are ignored for duration calculation
            }
        }

        if has_any_duration {
            Some(max_completion_time)
        } else {
            // All effects are perpetual - sequence never finishes
            None
        }
    }
}

/// Loop mode for sequence references
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum SequenceLoop {
    Once,
    Loop, // Infinite loop
    PingPong,
    Random,
    Count(usize), // Loop N times
}

/// Unexpanded sequence cue information for two-pass parsing
#[derive(Debug, Clone)]
pub(crate) struct UnexpandedSequenceCue {
    pub time: Duration,
    pub effects: Vec<Effect>,
    pub layer_commands: Vec<LayerCommand>,
    pub stop_sequences: Vec<String>,
    pub sequence_references: Vec<(String, Option<SequenceLoop>)>, // (sequence_name, loop_param)
}

#[derive(Debug, Clone)]
pub struct Cue {
    pub time: Duration,
    pub effects: Vec<Effect>,
    pub layer_commands: Vec<LayerCommand>,
    pub stop_sequences: Vec<String>, // Names of sequences to stop at this cue time
    pub start_sequences: Vec<String>, // Names of sequences starting fresh at this cue time
}

#[derive(Debug, Clone)]
pub struct Effect {
    pub groups: Vec<String>,
    pub effect_type: EffectType,
    pub layer: Option<EffectLayer>,
    pub blend_mode: Option<BlendMode>,
    pub up_time: Option<Duration>,
    pub hold_time: Option<Duration>,
    pub down_time: Option<Duration>,
    pub sequence_name: Option<String>, // Track which sequence this effect came from (for stopping)
}

impl Effect {
    /// Calculate the total duration of this effect
    /// Returns None for perpetual/indefinite effects
    pub fn total_duration(&self) -> Option<Duration> {
        // Check if this is an indefinite effect (no hold_time and no down_time)
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

        // For static effects, if duration is specified in effect_type, use it as the hold_time
        // and include up_time and down_time in the total calculation
        if let EffectType::Static {
            duration: Some(static_duration),
            ..
        } = &self.effect_type
        {
            let duration = self.up_time.unwrap_or(Duration::from_secs(0))
                + *static_duration
                + self.down_time.unwrap_or(Duration::from_secs(0));
            return Some(duration);
        }

        let duration = self.up_time.unwrap_or(Duration::from_secs(0))
            + self.hold_time.unwrap_or(Duration::from_secs(0))
            + self.down_time.unwrap_or(Duration::from_secs(0));

        Some(duration)
    }
}

/// Layer control command types (grandMA-inspired)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayerCommandType {
    /// Clear - immediately stop all effects on the layer
    Clear,
    /// Release - gracefully fade out all effects on the layer
    Release,
    /// Freeze - pause all effects on the layer at their current state
    Freeze,
    /// Unfreeze - resume paused effects on the layer
    Unfreeze,
    /// Master - set layer intensity and/or speed master
    Master,
}

/// A layer control command
#[derive(Debug, Clone)]
pub struct LayerCommand {
    pub command_type: LayerCommandType,
    pub layer: Option<EffectLayer>, // None means all layers (only valid for clear)
    pub fade_time: Option<Duration>,
    pub intensity: Option<f64>,
    pub speed: Option<f64>,
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::lighting::effects::{
        ChaseDirection, ChasePattern, Color, CycleDirection, CycleTransition, DimmerCurve,
        TempoAwareFrequency, TempoAwareSpeed,
    };

    fn static_effect(duration: Option<Duration>) -> Effect {
        Effect {
            groups: vec![],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration,
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
            sequence_name: None,
        }
    }

    fn timed_effect(
        effect_type: EffectType,
        up: Option<Duration>,
        hold: Option<Duration>,
        down: Option<Duration>,
    ) -> Effect {
        Effect {
            groups: vec![],
            effect_type,
            layer: None,
            blend_mode: None,
            up_time: up,
            hold_time: hold,
            down_time: down,
            sequence_name: None,
        }
    }

    // ── Effect::total_duration ──────────────────────────────────────

    #[test]
    fn effect_perpetual_static_no_duration() {
        let e = static_effect(None);
        assert_eq!(e.total_duration(), None);
    }

    #[test]
    fn effect_static_with_duration() {
        let e = static_effect(Some(Duration::from_secs(5)));
        // Static with duration uses: up + static_duration + down
        assert_eq!(e.total_duration(), Some(Duration::from_secs(5)));
    }

    #[test]
    fn effect_static_with_duration_and_up_down() {
        let e = timed_effect(
            EffectType::Static {
                parameters: HashMap::new(),
                duration: Some(Duration::from_secs(5)),
            },
            Some(Duration::from_secs(1)),
            None, // hold_time not used; static_duration acts as hold
            Some(Duration::from_secs(2)),
        );
        // up(1) + static_duration(5) + down(2) = 8
        assert_eq!(e.total_duration(), Some(Duration::from_secs(8)));
    }

    #[test]
    fn effect_dimmer_uses_effect_duration() {
        let e = timed_effect(
            EffectType::Dimmer {
                start_level: 0.0,
                end_level: 1.0,
                duration: Duration::from_secs(3),
                curve: DimmerCurve::Linear,
            },
            None,
            None,
            None,
        );
        assert_eq!(e.total_duration(), Some(Duration::from_secs(3)));
    }

    #[test]
    fn effect_strobe_no_duration_perpetual() {
        let e = timed_effect(
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(10.0),
                duration: None,
            },
            None,
            None,
            None,
        );
        assert_eq!(e.total_duration(), None);
    }

    #[test]
    fn effect_pulse_no_duration_perpetual() {
        let e = timed_effect(
            EffectType::Pulse {
                base_level: 0.0,
                pulse_amplitude: 1.0,
                frequency: TempoAwareFrequency::Fixed(2.0),
                duration: None,
            },
            None,
            None,
            None,
        );
        assert_eq!(e.total_duration(), None);
    }

    #[test]
    fn effect_color_cycle_perpetual() {
        let e = timed_effect(
            EffectType::ColorCycle {
                colors: vec![Color::new(255, 0, 0)],
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: CycleDirection::Forward,
                transition: CycleTransition::Fade,
            },
            None,
            None,
            None,
        );
        assert_eq!(e.total_duration(), None);
    }

    #[test]
    fn effect_chase_perpetual() {
        let e = timed_effect(
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
            },
            None,
            None,
            None,
        );
        assert_eq!(e.total_duration(), None);
    }

    #[test]
    fn effect_rainbow_perpetual() {
        let e = timed_effect(
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Fixed(0.5),
                saturation: 1.0,
                brightness: 1.0,
            },
            None,
            None,
            None,
        );
        assert_eq!(e.total_duration(), None);
    }

    #[test]
    fn effect_with_up_hold_down() {
        let e = timed_effect(
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(10.0),
                duration: Some(Duration::from_secs(5)),
            },
            Some(Duration::from_secs(1)),
            Some(Duration::from_secs(3)),
            Some(Duration::from_secs(1)),
        );
        // up(1) + hold(3) + down(1) = 5
        assert_eq!(e.total_duration(), Some(Duration::from_secs(5)));
    }

    // ── Sequence::duration ─────────────────────────────────────────

    #[test]
    fn sequence_empty() {
        let seq = Sequence {
            cues: vec![],
            bpm: 120.0,
        };
        assert_eq!(seq.duration(), Some(Duration::ZERO));
    }

    #[test]
    fn sequence_all_perpetual() {
        let seq = Sequence {
            cues: vec![Cue {
                time: Duration::ZERO,
                effects: vec![static_effect(None)],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            }],
            bpm: 120.0,
        };
        assert_eq!(seq.duration(), None);
    }

    #[test]
    fn sequence_single_timed_cue() {
        let seq = Sequence {
            cues: vec![Cue {
                time: Duration::from_secs(2),
                effects: vec![static_effect(Some(Duration::from_secs(3)))],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            }],
            bpm: 120.0,
        };
        // cue_time(2) + effect_duration(3) = 5
        assert_eq!(seq.duration(), Some(Duration::from_secs(5)));
    }

    #[test]
    fn sequence_max_completion_across_cues() {
        let seq = Sequence {
            cues: vec![
                Cue {
                    time: Duration::from_secs(0),
                    effects: vec![static_effect(Some(Duration::from_secs(3)))],
                    layer_commands: vec![],
                    stop_sequences: vec![],
                    start_sequences: vec![],
                },
                Cue {
                    time: Duration::from_secs(5),
                    effects: vec![static_effect(Some(Duration::from_secs(10)))],
                    layer_commands: vec![],
                    stop_sequences: vec![],
                    start_sequences: vec![],
                },
            ],
            bpm: 120.0,
        };
        // max(0+3, 5+10) = 15
        assert_eq!(seq.duration(), Some(Duration::from_secs(15)));
    }

    #[test]
    fn sequence_mixed_perpetual_and_timed() {
        // Perpetual effects are ignored; only timed effects count
        let seq = Sequence {
            cues: vec![Cue {
                time: Duration::from_secs(1),
                effects: vec![
                    static_effect(None),                         // perpetual, ignored
                    static_effect(Some(Duration::from_secs(4))), // timed: 1+4=5
                ],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            }],
            bpm: 120.0,
        };
        assert_eq!(seq.duration(), Some(Duration::from_secs(5)));
    }

    // ── LayerCommandType ───────────────────────────────────────────

    #[test]
    fn layer_command_type_equality() {
        assert_eq!(LayerCommandType::Clear, LayerCommandType::Clear);
        assert_ne!(LayerCommandType::Clear, LayerCommandType::Release);
        assert_ne!(LayerCommandType::Freeze, LayerCommandType::Unfreeze);
    }
}
