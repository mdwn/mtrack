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
use super::super::tempo::TempoMap;

/// Bundles the per-cue context that flows through the parsing pipeline,
/// replacing the 5-7 individual parameters previously threaded through
/// `parse_effect_definition` and friends.
#[derive(Clone)]
pub(crate) struct ParseContext {
    pub tempo_map: Option<TempoMap>,
    pub cue_time: Duration,
    pub offset_secs: f64,
    pub unshifted_score_time: Option<Duration>,
}

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
    /// How long one pass lasts, from the first cue, when the sequence declares
    /// a `length:`. Otherwise a pass lasts until its last effect ends.
    pub length: Option<Duration>,
}

impl Sequence {
    /// Calculate the duration of this sequence based on when all effects complete.
    /// Returns the time from sequence start (0) to when the last effect completes.
    pub fn duration(&self) -> Duration {
        if self.cues.is_empty() {
            return Duration::ZERO;
        }

        let mut max_completion_time = Duration::ZERO;

        for cue in &self.cues {
            for effect in &cue.effects {
                let effect_duration = effect.total_duration();
                let completion_time = cue.time + effect_duration;
                if completion_time > max_completion_time {
                    max_completion_time = completion_time;
                }
            }
        }

        max_completion_time
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
    /// `per: cell` (design §17.3): the effect's targets are the group's
    /// fixtures expanded to their cells, in spatial order.
    pub per_cell: bool,
    /// `spread` in degrees: how much of a cycle the ordered targets are
    /// spread across (`rainbow` and `cycle`); zero keeps every target in
    /// phase.
    pub spread: f64,
    /// Parameters this effect type does not use, in the order they sort.
    ///
    /// A `direction` on a rainbow parses and does nothing — the DSL accepts any
    /// parameter and each effect type takes what it recognises. Keeping the
    /// leftovers lets a lint say so; rejecting them at parse time would break
    /// shows that already carry one.
    pub ignored_parameters: Vec<String>,
    /// The show's `offset` in force at this effect's cue, in seconds. Cue
    /// times are shifted by it, and so is the tempo map its `beats` /
    /// `measures` speeds and frequencies are read against at runtime (#525).
    pub tempo_offset_secs: f64,
    /// The durations written in beats or measures, as written. A sequence's
    /// effects are parsed at the sequence's own tempo; when it is expanded
    /// into a show these are resolved again at the expansion point, as the
    /// sequence's cue times are (#529).
    pub musical: MusicalDurations,
}

/// A length written in musical units, kept so it can be resolved against a
/// tempo other than the one it was parsed at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MusicalLength {
    Beats(f64),
    Measures(f64),
}

impl MusicalLength {
    /// The musical length a duration string spells, or `None` for one in
    /// seconds (or that does not parse; `parse_duration_string` reports that).
    pub fn parse(value: &str) -> Option<MusicalLength> {
        let value = value.trim();
        if let Some(n) = value
            .strip_suffix("measures")
            .or_else(|| value.strip_suffix("measure"))
        {
            n.trim().parse().ok().map(MusicalLength::Measures)
        } else if let Some(n) = value
            .strip_suffix("beats")
            .or_else(|| value.strip_suffix("beat"))
        {
            n.trim().parse().ok().map(MusicalLength::Beats)
        } else {
            None
        }
    }

    /// The length in time starting at `at_time` on `tempo_map`, with a show
    /// `offset` of `offset_secs`.
    pub fn resolve(
        self,
        tempo_map: &crate::lighting::tempo::TempoMap,
        at_time: Duration,
        offset_secs: f64,
    ) -> Duration {
        match self {
            MusicalLength::Beats(beats) => tempo_map.beats_to_duration(beats, at_time, offset_secs),
            MusicalLength::Measures(measures) => {
                tempo_map.measures_to_duration(measures, at_time, offset_secs)
            }
        }
    }
}

/// An effect's duration parameters that were written in beats or measures.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MusicalDurations {
    pub duration: Option<MusicalLength>,
    pub up_time: Option<MusicalLength>,
    pub hold_time: Option<MusicalLength>,
    pub down_time: Option<MusicalLength>,
}

impl MusicalDurations {
    /// Records `value` for the duration parameter `key`, if it is one and is
    /// written in musical units.
    pub fn record(&mut self, key: &str, value: &str) {
        let slot = match key {
            "duration" => &mut self.duration,
            "up_time" => &mut self.up_time,
            "hold_time" => &mut self.hold_time,
            "down_time" => &mut self.down_time,
            _ => return,
        };
        *slot = MusicalLength::parse(value);
    }

    pub fn is_empty(&self) -> bool {
        *self == MusicalDurations::default()
    }
}

impl Effect {
    /// Resolves the durations written in beats or measures at `at_time` on
    /// `tempo_map` (with a show `offset` of `offset_secs`), leaving the ones
    /// written in seconds as they are.
    pub fn resolve_musical_durations(
        &mut self,
        tempo_map: &crate::lighting::tempo::TempoMap,
        at_time: Duration,
        offset_secs: f64,
    ) {
        let resolve = |len: MusicalLength| len.resolve(tempo_map, at_time, offset_secs);
        if let Some(len) = self.musical.duration {
            self.effect_type.set_duration(resolve(len));
        }
        if let Some(len) = self.musical.up_time {
            self.up_time = Some(resolve(len));
        }
        if let Some(len) = self.musical.hold_time {
            self.hold_time = Some(resolve(len));
        }
        if let Some(len) = self.musical.down_time {
            self.down_time = Some(resolve(len));
        }
    }

    /// Calculate the total duration of this effect.
    /// All effects have a finite duration.
    pub fn total_duration(&self) -> Duration {
        // For dimmers, use the effect's own duration field (timing params not used)
        if let EffectType::Dimmer { duration, .. } = &self.effect_type {
            return *duration;
        }

        // Extract the effect type's duration field
        let effect_duration = self.effect_type.duration();

        // If hold_time is explicitly set, use up + hold + down.
        // Otherwise use up + effect_duration + down.
        let hold = self.hold_time.unwrap_or(effect_duration);

        self.up_time.unwrap_or(Duration::ZERO) + hold + self.down_time.unwrap_or(Duration::ZERO)
    }
}

/// Layer control command types (grandMA-inspired)
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LayerCommandType {
    /// Clear - immediately stop all effects on the layer
    Clear,
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

    fn static_effect(duration: Duration) -> Effect {
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
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
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
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
        }
    }

    // ── Effect::total_duration ──────────────────────────────────────

    #[test]
    fn effect_static_with_duration() {
        let e = static_effect(Duration::from_secs(5));
        // Static with duration uses: up + static_duration + down
        assert_eq!(e.total_duration(), Duration::from_secs(5));
    }

    #[test]
    fn effect_static_with_duration_and_up_down() {
        let e = timed_effect(
            EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(5),
            },
            Some(Duration::from_secs(1)),
            None, // hold_time not used; static_duration acts as hold
            Some(Duration::from_secs(2)),
        );
        // up(1) + static_duration(5) + down(2) = 8
        assert_eq!(e.total_duration(), Duration::from_secs(8));
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
        assert_eq!(e.total_duration(), Duration::from_secs(3));
    }

    #[test]
    fn effect_strobe_with_duration() {
        let e = timed_effect(
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(10.0),
                duration: Duration::from_secs(5),
            },
            None,
            None,
            None,
        );
        assert_eq!(e.total_duration(), Duration::from_secs(5));
    }

    #[test]
    fn effect_pulse_with_duration() {
        let e = timed_effect(
            EffectType::Pulse {
                base_level: 0.0,
                pulse_amplitude: 1.0,
                frequency: TempoAwareFrequency::Fixed(2.0),
                duration: Duration::from_secs(5),
            },
            None,
            None,
            None,
        );
        assert_eq!(e.total_duration(), Duration::from_secs(5));
    }

    #[test]
    fn effect_color_cycle_with_duration() {
        let e = timed_effect(
            EffectType::ColorCycle {
                colors: vec![Color::new(255, 0, 0)],
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: CycleDirection::Forward,
                transition: CycleTransition::Fade,
                duration: Duration::from_secs(5),
            },
            None,
            None,
            None,
        );
        assert_eq!(e.total_duration(), Duration::from_secs(5));
    }

    #[test]
    fn effect_chase_with_duration() {
        let e = timed_effect(
            EffectType::Chase {
                pattern: ChasePattern::Linear,
                speed: TempoAwareSpeed::Fixed(1.0),
                direction: ChaseDirection::LeftToRight,
                transition: CycleTransition::Snap,
                duration: Duration::from_secs(5),
            },
            None,
            None,
            None,
        );
        assert_eq!(e.total_duration(), Duration::from_secs(5));
    }

    #[test]
    fn effect_rainbow_with_duration() {
        let e = timed_effect(
            EffectType::Rainbow {
                speed: TempoAwareSpeed::Fixed(0.5),
                saturation: 1.0,
                brightness: 1.0,
                duration: Duration::from_secs(5),
            },
            None,
            None,
            None,
        );
        assert_eq!(e.total_duration(), Duration::from_secs(5));
    }

    #[test]
    fn effect_with_up_hold_down() {
        let e = timed_effect(
            EffectType::Strobe {
                frequency: TempoAwareFrequency::Fixed(10.0),
                duration: Duration::from_secs(5),
            },
            Some(Duration::from_secs(1)),
            Some(Duration::from_secs(3)),
            Some(Duration::from_secs(1)),
        );
        // up(1) + hold(3) + down(1) = 5
        assert_eq!(e.total_duration(), Duration::from_secs(5));
    }

    // ── Sequence::duration ─────────────────────────────────────────

    #[test]
    fn sequence_empty() {
        let seq = Sequence {
            cues: vec![],
            bpm: 120.0,
            length: None,
        };
        assert_eq!(seq.duration(), Duration::ZERO);
    }

    #[test]
    fn sequence_single_timed_cue() {
        let seq = Sequence {
            cues: vec![Cue {
                time: Duration::from_secs(2),
                effects: vec![static_effect(Duration::from_secs(3))],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            }],
            bpm: 120.0,
            length: None,
        };
        // cue_time(2) + effect_duration(3) = 5
        assert_eq!(seq.duration(), Duration::from_secs(5));
    }

    #[test]
    fn sequence_max_completion_across_cues() {
        let seq = Sequence {
            cues: vec![
                Cue {
                    time: Duration::from_secs(0),
                    effects: vec![static_effect(Duration::from_secs(3))],
                    layer_commands: vec![],
                    stop_sequences: vec![],
                    start_sequences: vec![],
                },
                Cue {
                    time: Duration::from_secs(5),
                    effects: vec![static_effect(Duration::from_secs(10))],
                    layer_commands: vec![],
                    stop_sequences: vec![],
                    start_sequences: vec![],
                },
            ],
            bpm: 120.0,
            length: None,
        };
        // max(0+3, 5+10) = 15
        assert_eq!(seq.duration(), Duration::from_secs(15));
    }

    #[test]
    fn sequence_mixed_short_and_long() {
        let seq = Sequence {
            cues: vec![Cue {
                time: Duration::from_secs(1),
                effects: vec![
                    static_effect(Duration::from_secs(1)), // short: 1+1=2
                    static_effect(Duration::from_secs(4)), // long: 1+4=5
                ],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            }],
            bpm: 120.0,
            length: None,
        };
        assert_eq!(seq.duration(), Duration::from_secs(5));
    }

    // ── LayerCommandType ───────────────────────────────────────────

    #[test]
    fn layer_command_type_equality() {
        assert_eq!(LayerCommandType::Clear, LayerCommandType::Clear);
        assert_ne!(LayerCommandType::Freeze, LayerCommandType::Unfreeze);
    }
}
