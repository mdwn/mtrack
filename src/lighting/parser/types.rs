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

/// The cue an effect is parsed in: the tempo map in force, the cue's time
/// and the show `offset` it includes.
#[derive(Clone)]
pub(crate) struct ParseContext {
    pub tempo_map: Option<TempoMap>,
    pub cue_time: Duration,
    pub offset_secs: f64,
}

impl ParseContext {
    /// A duration parameter's value -- `duration`, `up_time`, `hold_time`,
    /// `down_time`, on any effect -- read at the cue's place in the score:
    /// its time less the offset, on the unshifted map, which is the map
    /// shifted by the offset as the cue was (#525). One in beats or
    /// measures so integrates through the tempo changes and ramps from
    /// there, as the cue times do. Two other rules apply on purpose: a
    /// sequence's effects are resolved again where the sequence is
    /// expanded (`Effect::resolve_musical_durations`, #529), and its
    /// `length:` is read from its first cue (`declared_length`), there
    /// being no cue to read it at.
    pub fn duration(&self, value: &str) -> Result<Duration, Box<dyn std::error::Error>> {
        let score_time = self
            .cue_time
            .saturating_sub(Duration::from_secs_f64(self.offset_secs));
        super::utils::parse_duration_string(value, &self.tempo_map, Some(score_time), 0.0)
    }
}

/// A file's shows by name, in the order the file has them: the timeline
/// fires same-time cues from different shows in that order, so it must not
/// change from one parse to the next as a `HashMap`'s does (#597).
pub type LightShows = indexmap::IndexMap<String, LightShow>;

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
    /// The tempo map its cue times were resolved on: where it is used, a
    /// cue is placed the same number of beats in, counted on this map.
    pub tempo_map: Option<TempoMap>,
    /// How long one pass lasts, from the first cue, when the sequence declares
    /// a `length:`. Otherwise a pass lasts until its last effect ends.
    pub length: Option<Duration>,
}

impl Sequence {
    /// How long one pass lasts: its declared `length:`, else as
    /// [`pass_length`] has it.
    pub fn pass_length(&self) -> Duration {
        self.length.unwrap_or_else(|| pass_length(&self.cues))
    }
}

/// When a pass of `cues` starts: at the earliest of them.
pub(crate) fn pass_start(cues: &[Cue]) -> Duration {
    cues.iter().map(|c| c.time).min().unwrap_or_default()
}

/// How long one pass of `cues` -- a sequence's, or an inline loop body's --
/// lasts when nothing says: from its earliest cue until its last effect
/// ends, or, when no cue has an effect, until its last cue.
pub(crate) fn pass_length(cues: &[Cue]) -> Duration {
    let start = pass_start(cues);
    let effects_end = cues
        .iter()
        .flat_map(|c| c.effects.iter().map(move |e| c.time + e.total_duration()))
        .max();
    let last_cue = cues.iter().map(|c| c.time).max();
    effects_end
        .or(last_cue)
        .unwrap_or_default()
        .saturating_sub(start)
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

/// A `sequence "name"(…), loop: …` reference, as written in a cue.
#[derive(Debug, Clone)]
pub(crate) struct SequenceCall {
    pub name: String,
    /// The parameters bound here, in source order, with their values as written.
    pub args: Vec<(String, String)>,
    pub loop_param: Option<SequenceLoop>,
    /// The line the reference is on, for errors raised expanding it.
    pub line: usize,
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
    /// The sequences it was started through, innermost first; empty for a
    /// show's own effect. `stop sequence X` stops it when X is any of them.
    pub sequences: Vec<String>,
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
        match super::utils::parse_time_value(value).ok()? {
            super::utils::TimeValue::Measures(n) => Some(MusicalLength::Measures(n)),
            super::utils::TimeValue::Beats(n) => Some(MusicalLength::Beats(n)),
            super::utils::TimeValue::Seconds(_) => None,
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
    /// The sequence it is written in, if any: the innermost it was started
    /// through.
    pub fn sequence_name(&self) -> Option<&str> {
        self.sequences.first().map(String::as_str)
    }

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

    /// How long the effect lasts (see [`EffectType::total_duration`]).
    pub fn total_duration(&self) -> Duration {
        self.effect_type
            .total_duration(self.up_time, self.hold_time, self.down_time)
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
            sequences: Vec::new(),
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
            sequences: Vec::new(),
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

    // ── pass_length ────────────────────────────────────────────────

    fn cue(time: u64, effects: Vec<Effect>) -> Cue {
        Cue {
            time: Duration::from_secs(time),
            effects,
            layer_commands: vec![],
            stop_sequences: vec![],
            start_sequences: vec![],
        }
    }

    #[test]
    fn pass_length_empty() {
        assert_eq!(pass_length(&[]), Duration::ZERO);
    }

    #[test]
    fn pass_length_runs_from_the_earliest_cue_to_the_last_effect_end() {
        let cues = [
            cue(5, vec![static_effect(Duration::from_secs(10))]),
            cue(2, vec![static_effect(Duration::from_secs(3))]),
            cue(20, vec![]),
        ];
        // From 2 to max(5+10, 2+3) = 15; the effectless cue at 20 does not count.
        assert_eq!(pass_length(&cues), Duration::from_secs(13));
        let seq = Sequence {
            cues: cues.to_vec(),
            tempo_map: None,
            length: None,
        };
        assert_eq!(seq.pass_length(), Duration::from_secs(13));
    }

    #[test]
    fn pass_length_without_effects_runs_to_the_last_cue() {
        assert_eq!(
            pass_length(&[cue(2, vec![]), cue(7, vec![])]),
            Duration::from_secs(5)
        );
        assert_eq!(pass_length(&[cue(2, vec![])]), Duration::ZERO);
    }

    #[test]
    fn pass_length_mixed_short_and_long() {
        let cues = [cue(
            1,
            vec![
                static_effect(Duration::from_secs(1)), // short: 1+1=2
                static_effect(Duration::from_secs(4)), // long: 1+4=5
            ],
        )];
        assert_eq!(pass_length(&cues), Duration::from_secs(4));
    }

    // ── LayerCommandType ───────────────────────────────────────────

    #[test]
    fn layer_command_type_equality() {
        assert_eq!(LayerCommandType::Clear, LayerCommandType::Clear);
        assert_ne!(LayerCommandType::Freeze, LayerCommandType::Unfreeze);
    }
}
