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
use super::super::tempo::{snap_beats, TempoMap, DEFAULT_BPM};

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
    /// The cues of sequences it plays that fall past a pass's `length:` and
    /// do not play: for the lint to say so.
    pub dropped: Vec<DroppedCue>,
}

/// A reusable sequence of cues that can be referenced in shows
#[derive(Debug, Clone)]
pub struct Sequence {
    pub cues: Vec<Cue>,
    /// The tempo map the sequence's cues were placed through: its own
    /// `tempo` block, or the file's. Its positions are counted in beats
    /// through it, and placed in beats where it is played.
    pub tempo_map: Option<TempoMap>,
    /// How long one pass lasts, from the first cue, when the sequence declares
    /// a `length:`. Otherwise a pass lasts until its content ends where it
    /// plays.
    pub length: Option<Duration>,
    /// Whether the `length:` is written in beats or measures (then a pass
    /// of a body in beats lasts it on its grid); one in seconds lasts its
    /// seconds wherever it plays.
    pub(crate) length_in_beats: bool,
    /// Whether the body is purely in seconds
    /// ([`body_in_seconds`](super::show::body_in_seconds)): laid out in
    /// seconds wherever it plays, its cues and passes keeping their times
    /// whatever the tempo does. Any other body is laid out in beats,
    /// through the tempo map where it plays.
    pub in_seconds: bool,
    /// For each cue (by index; empty for none), how far it is from the
    /// anchor of the inline loop purely in seconds it comes from, when it
    /// does: the loop keeps its seconds, its anchor placed in beats with the
    /// rest (see [`PlacedCue`](super::show::PlacedCue)).
    pub from_anchor: Vec<Option<Duration>>,
    /// For each cue (by index; empty for none), the sequences it plays:
    /// played where the cue lands, each time the sequence is played, so
    /// every sequence is laid out where it plays.
    pub(crate) calls: Vec<Vec<SequenceCall>>,
    /// For each cue (by index; empty for all), whether it keeps a cue of its
    /// own where it is played: not one with nothing but the sequences it
    /// plays.
    pub(crate) keeps: Vec<bool>,
    /// Whether it plays a sequence for good (`loop: loop`, here or in a
    /// sequence it plays): played, it stops at the end any show can have.
    pub endless: bool,
    /// Whether any effect plays in it, its own or a sequence's it plays:
    /// without one, its cues' times are where its passes end.
    pub has_effects: bool,
}

/// A cue of a sequence that lands at or past the end of its pass (a
/// `length:`), where it plays -- one in seconds where the tempo is faster
/// than the sequence was written at -- and so does not play, as a cue
/// written there cannot.
#[derive(Debug, Clone, PartialEq)]
pub struct DroppedCue {
    /// The sequence whose pass it falls outside.
    pub sequence: String,
    /// The sequence the show's cue plays it from (itself, or one it is
    /// played inside).
    pub call: String,
    /// Where the show's cue is.
    pub call_at: Duration,
    /// Where it would have played, in the show.
    pub time: Duration,
    /// Where the pass it belongs to ends, in the show.
    pub pass_end: Duration,
}

/// The quarter-note beats from `from` to `to` through `tempo_map` (at the
/// default tempo without one), as the player snaps them.
pub(crate) fn beats_between(tempo_map: Option<&TempoMap>, from: Duration, to: Duration) -> f64 {
    let at = |t: Duration| match tempo_map {
        Some(tm) => tm.beats_at(t, 0.0),
        None => t.as_secs_f64() * DEFAULT_BPM / 60.0,
    };
    snap_beats(at(to) - at(from))
}

impl Sequence {
    /// When the sequence starts: its earliest cue.
    pub(crate) fn start_time(&self) -> Duration {
        self.cues.iter().map(|c| c.time).min().unwrap_or_default()
    }

    /// The beats from `start` (its start) to `time`, through its tempo map.
    pub(crate) fn beats_between(&self, start: Duration, time: Duration) -> f64 {
        beats_between(self.tempo_map.as_ref(), start, time)
    }

    /// How many beats its declared `length:` is, from its start: what a
    /// pass in beats lasts, when it declares one. (A pass without one lasts
    /// until its content ends where it plays: see `expand_sequence_call`.)
    pub(crate) fn declared_beats(&self) -> Option<f64> {
        let length = self.length.filter(|_| self.length_in_beats)?;
        let start = self.start_time();
        Some(self.beats_between(start, start + length))
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
    /// The offset in force at the cue, in seconds: a sequence played from it
    /// is placed under it.
    pub offset_secs: f64,
    pub effects: Vec<Effect>,
    pub layer_commands: Vec<LayerCommand>,
    pub stop_sequences: Vec<String>,
    pub sequence_references: Vec<SequenceCall>,
    /// For a cue of an inline loop purely in seconds: how far it is from
    /// the loop's anchor (see [`PlacedCue`](super::show::PlacedCue)).
    pub from_anchor: Option<Duration>,
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

#[cfg(all(test, not(mtrack_wasm)))]
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

    // ── LayerCommandType ───────────────────────────────────────────

    #[test]
    fn layer_command_type_equality() {
        assert_eq!(LayerCommandType::Clear, LayerCommandType::Clear);
        assert_ne!(LayerCommandType::Freeze, LayerCommandType::Unfreeze);
    }
}
