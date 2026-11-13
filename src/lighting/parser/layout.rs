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
//! Where a `.light` file's cues land in time, for the web UI's timeline:
//! the file parsed by the one parse path (`parse_file_with`, as the player
//! loads it), each written cue with the time the player fires it at, the
//! effects it puts on the timeline (its own, its inline loops' and its
//! sequence references' passes) with their written ends, the passes
//! themselves, and the tempo map's bar and beat lines and tempo stretches.
//! The UI draws these and computes nothing of its own; everything here is
//! read off the parser's cues and the player's `TempoMap`.
//!
//! The types' TypeScript declarations are generated from them (`tsify`).

use std::path::Path;
use std::time::Duration;

use serde::Serialize;

use super::grammar::Rule;
use super::include::IncludeReader;
use super::show::{parse_file_with, ParsedCue, ParsedShow};
use super::types::EffectKind;
use super::utils::unquote;
use crate::lighting::effects::DEFAULT_LAYER;
use crate::position::duration_from_secs_saturating;
use crate::tempo::TempoMap;

/// How far past the last written end the timeline reaches.
const TAIL: Duration = Duration::from_secs(5);

/// Grid lines are drawn for at most this many beats.
const MAX_GRID_LINES: usize = 100_000;

/// A file laid out in time.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(into_wasm_abi, hashmap_as_object)
)]
#[derive(Serialize, Clone, Debug, PartialEq, Default)]
pub struct Layout {
    /// Why the file could not be laid out: it does not parse, or does not
    /// load (a `@bar/beat` with no tempo, a sequence that is not defined),
    /// as the player would report it.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub error: Option<String>,
    /// The map the file's shows play to unless they carry their own: the
    /// file's `tempo` block, else the song's; `None` when there is neither.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub tempo: Option<TempoLayout>,
    /// The shows the player loads, in file order (a show whose name a
    /// later show takes is not among them, as the player drops it).
    pub shows: Vec<ShowLayout>,
    /// The file's own sequences, each on its own timeline.
    pub sequences: Vec<SequenceLayout>,
    /// The latest written end over the shows, in milliseconds: how far the
    /// timeline must reach.
    pub end_ms: f64,
}

/// A tempo map's bar and beat lines and tempo stretches.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TempoLayout {
    /// Where bar 1 is.
    pub start_ms: f64,
    pub spans: Vec<TempoSpan>,
    pub grid: Vec<GridLine>,
}

/// A stretch of the tempo lane with one tempo and metre: from each change,
/// each ramp's end and each barline a mid-bar signature change governs from.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct TempoSpan {
    pub start_ms: f64,
    pub end_ms: f64,
    /// The tempo where the stretch starts.
    pub bpm: f64,
    /// Inside a gradual transition, the tempo it is heading for.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub bpm_to: Option<f64>,
    /// The signature of the bar the stretch starts in.
    pub sig: (u32, u32),
}

/// A bar or beat line.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct GridLine {
    pub ms: f64,
    pub measure: u32,
    /// The beat, 1-based, as the beat grid counts it: one per denominator
    /// note, so a 6/8 bar has six.
    pub beat: u32,
    pub kind: GridLineKind,
}

#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum GridLineKind {
    Measure,
    Beat,
}

/// A show's cues in time.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct ShowLayout {
    pub name: String,
    pub cues: Vec<CueLayout>,
    /// The show's own tempo layout, when it differs from the file's: it
    /// has a `tempo` block of its own, or an `offset` moves its map.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub tempo: Option<TempoLayout>,
    /// The show's latest written end.
    pub end_ms: f64,
}

/// A sequence's cues on its own timeline, with its parameters at their
/// defaults.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct SequenceLayout {
    pub name: String,
    pub cues: Vec<CueLayout>,
    /// Its own tempo layout, when it has a `tempo` block of its own.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub tempo: Option<TempoLayout>,
    /// Why it cannot be laid out alone: a parameter with no default, or an
    /// error in its body.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub error: Option<String>,
    pub end_ms: f64,
}

/// One written cue, where the player fires it.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct CueLayout {
    /// Its place among the show's written cues.
    pub index: u32,
    /// When it fires: its score time plus the offset in force.
    pub ms: f64,
    /// Its place in the tempo map, before any offset: where its durations
    /// are timed from.
    pub score_ms: f64,
    /// The offset in force at it.
    pub offset_ms: f64,
    /// The offset in force for the cues after it: its own `offset` added,
    /// or dropped by its `reset_measures`.
    pub next_offset_ms: f64,
    /// What its `offset N measures` commands add up to.
    pub offset_measures: u32,
    pub reset: bool,
    /// Every effect it puts on the timeline: its own, its inline loops'
    /// passes' and its sequence references' passes'.
    pub effects: Vec<EffectLayout>,
    /// The passes of each sequence it references.
    pub passes: Vec<PassLayout>,
}

/// An effect on the timeline, with its written end.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct EffectLayout {
    pub kind: EffectKind,
    /// The layer it runs on, the default when the line names none.
    pub layer: String,
    pub groups: Vec<String>,
    pub start_ms: f64,
    /// Its written end: its cue's time plus its fade in, hold and fade out.
    pub end_ms: f64,
    /// The sequence it is written in, for one a reference placed.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[cfg_attr(feature = "tsify", tsify(optional))]
    pub sequence: Option<String>,
    /// Whether it is one of the written cue's own effect lines, in their
    /// order (the others are a pass's or a loop's).
    pub own: bool,
}

/// Where a sequence reference's passes run.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct PassLayout {
    pub sequence: String,
    pub start_ms: f64,
    /// Where the last pass ends; for `loop`, the end of the timeline.
    pub end_ms: f64,
    /// How long the first pass lasts.
    pub pass_ms: f64,
    /// How many passes: 10,000 for `loop`.
    pub count: u32,
    pub infinite: bool,
    /// When each pass starts, those within the timeline: where the
    /// passes are, under whatever tempo they run through.
    pub pass_starts: Vec<f64>,
    /// Where pass after pass would end, laid on through the tempo map to
    /// the end of the timeline however many the reference asks for: what
    /// a resize that grows or shrinks the count reaches (see
    /// [`passes_by`]).
    pub pass_ends: Vec<f64>,
}

/// How many whole passes of `pass` fit before `until_ms`: those that end
/// by then (see [`PassLayout::pass_ends`]), never fewer than one -- the
/// count a reference's block dragged to end there stands for.
pub fn passes_by(pass_ends: &[f64], until_ms: f64) -> u32 {
    let whole = pass_ends.iter().take_while(|&&end| end <= until_ms).count();
    u32::try_from(whole).unwrap_or(u32::MAX).max(1)
}

/// Lays out `source` as the player loads it: `path` is where the file is,
/// which its `include`s resolve against, read through `reader`;
/// `external` the song's tempo map for a file with no tempo block. The
/// timeline reaches to `until` at least (the song's length), and past the
/// latest written end by a tail.
pub fn layout(
    source: &str,
    path: Option<&Path>,
    external: Option<&TempoMap>,
    reader: &dyn IncludeReader,
    until: Duration,
) -> Layout {
    let parsed = match parse_file_with(source, path, None, external, reader) {
        Ok(parsed) => parsed,
        Err(error) => {
            return Layout {
                error: Some(error.to_string()),
                ..Layout::default()
            }
        }
    };

    let end = parsed
        .shows
        .values()
        .map(|show| written_end(&show.cues))
        .max()
        .unwrap_or_default();
    let horizon = until.max(end.saturating_add(TAIL));

    let shows = parsed
        .shows
        .values()
        .map(|show| show_layout(show, horizon))
        .collect();

    let sequences = parsed
        .sequence_pairs
        .iter()
        .filter_map(|pair| {
            pair.clone()
                .into_inner()
                .find(|p| p.as_rule() == Rule::sequence_name)
                .map(|p| unquote(&p))
        })
        .map(|name| match parsed.library.structure(&name) {
            Ok(structure) => {
                let (cues, tempo_map, own_tempo) =
                    (structure.cues, structure.tempo_map, structure.own_tempo);
                let end = written_end(&cues);
                let horizon = end.saturating_add(TAIL).max(Duration::from_secs(15));
                let steps = offset_steps(&cues);
                SequenceLayout {
                    name,
                    cues: cue_layouts(&cues, horizon, &tempo_map),
                    tempo: tempo_map
                        .filter(|_| own_tempo)
                        .map(|map| TempoLayout::new(&map, horizon, &steps)),
                    error: None,
                    end_ms: ms(end),
                }
            }
            Err(error) => SequenceLayout {
                name,
                cues: Vec::new(),
                tempo: None,
                error: Some(error.to_string()),
                end_ms: 0.0,
            },
        })
        .collect();

    Layout {
        error: None,
        tempo: parsed
            .tempo_map
            .as_ref()
            .map(|map| TempoLayout::new(map, horizon, &[])),
        shows,
        sequences,
        end_ms: ms(end),
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// Where the offset in force changes along a block's cues: at each cue's
/// score time, the offset for the cues after it. A position exactly at a
/// cue is before it: the cue, and a grid line on it, are drawn before the
/// gap its offset opens.
fn offset_steps(cues: &[ParsedCue]) -> Vec<(Duration, Duration)> {
    let mut steps: Vec<(Duration, Duration)> = cues
        .iter()
        .filter_map(|cue| {
            let new_offset = cue.new_offset?;
            Some((
                cue.time
                    .saturating_sub(duration_from_secs_saturating(cue.offset_secs)),
                duration_from_secs_saturating(new_offset),
            ))
        })
        .collect();
    steps.sort_by_key(|(at, _)| *at);
    steps
}

/// A score time moved by the offset in force there.
fn shifted(steps: &[(Duration, Duration)], at: Duration) -> Duration {
    let shift = steps
        .iter()
        .take_while(|(from, _)| *from < at)
        .last()
        .map(|(_, shift)| *shift)
        .unwrap_or_default();
    at.saturating_add(shift)
}

fn show_layout(show: &ParsedShow, horizon: Duration) -> ShowLayout {
    let steps = offset_steps(&show.cues);
    ShowLayout {
        name: show.name.clone(),
        cues: cue_layouts(&show.cues, horizon, &show.tempo_map),
        tempo: show
            .tempo_map
            .as_ref()
            .filter(|_| show.own_tempo || !steps.is_empty())
            .map(|map| TempoLayout::new(map, horizon, &steps)),
        end_ms: ms(written_end(&show.cues)),
    }
}

/// The latest written end over `cues`: a cue's time, the gap its own
/// offset opens, its effects' ends and its passes' ends -- a `loop`'s
/// first pass only, and nothing its later passes put on the timeline, as
/// it runs until the song ends. Neither the timeline's `show_end` (where
/// playback ends: a loop's every pass, an end cap, a held look) nor
/// `pass_length` (one pass, from its sequence's origin): what the score
/// writes, measured from zero, offsets and all.
fn written_end(cues: &[ParsedCue]) -> Duration {
    let mut end = Duration::ZERO;
    for cue in cues {
        end = end.max(cue.time);
        if let Some(new_offset) = cue.new_offset {
            end = end.max(
                cue.time
                    .saturating_add(duration_from_secs_saturating(new_offset - cue.offset_secs)),
            );
        }
        let mut limit = Duration::MAX;
        for pass in &cue.passes {
            end = end.max(if pass.infinite {
                limit = limit.min(pass.first_end);
                pass.first_end
            } else {
                pass.end
            });
        }
        for placed in &cue.cues {
            if placed.time >= limit {
                continue;
            }
            for effect in &placed.effects {
                end = end.max(placed.time.saturating_add(effect.total_duration()));
            }
        }
    }
    end
}

/// `cues`, a block's, laid out; `tempo_map` is the block's, which placed
/// their passes.
fn cue_layouts(
    cues: &[ParsedCue],
    horizon: Duration,
    tempo_map: &Option<TempoMap>,
) -> Vec<CueLayout> {
    cues.iter()
        .enumerate()
        .map(|(index, cue)| cue_layout(index, cue, horizon, tempo_map))
        .collect()
}

fn cue_layout(
    index: usize,
    cue: &ParsedCue,
    horizon: Duration,
    tempo_map: &Option<TempoMap>,
) -> CueLayout {
    let offset = duration_from_secs_saturating(cue.offset_secs);
    let effects = cue
        .cues
        .iter()
        .enumerate()
        .filter(|(_, placed)| placed.time <= horizon)
        .flat_map(|(index, placed)| {
            placed
                .effects
                .iter()
                .enumerate()
                .map(move |(i, effect)| EffectLayout {
                    kind: EffectKind::of(&effect.effect_type),
                    layer: effect.layer.unwrap_or(DEFAULT_LAYER).keyword().to_string(),
                    groups: effect.groups.clone(),
                    start_ms: ms(placed.time),
                    end_ms: ms(placed.time.saturating_add(effect.total_duration())),
                    sequence: effect.sequence_name().map(str::to_string),
                    own: cue.base == Some(index) && i < cue.own_effects,
                })
        })
        .collect();
    let passes = cue
        .passes
        .iter()
        .map(|pass| {
            let boundaries = pass.boundaries(tempo_map, horizon);
            PassLayout {
                sequence: pass.name.clone(),
                start_ms: ms(pass.start),
                end_ms: ms(if pass.infinite {
                    pass.end.min(horizon)
                } else {
                    pass.end
                }),
                pass_ms: ms(pass.first_end.saturating_sub(pass.start)),
                count: u32::try_from(pass.count).unwrap_or(u32::MAX),
                infinite: pass.infinite,
                pass_starts: boundaries
                    .iter()
                    .take(pass.count)
                    .map(|&start| ms(start))
                    .collect(),
                pass_ends: boundaries.iter().skip(1).map(|&end| ms(end)).collect(),
            }
        })
        .collect();
    CueLayout {
        index: index as u32,
        ms: ms(cue.time),
        score_ms: ms(cue.time.saturating_sub(offset)),
        offset_ms: ms(offset),
        next_offset_ms: ms(duration_from_secs_saturating(
            cue.new_offset.unwrap_or(cue.offset_secs),
        )),
        offset_measures: cue.offset_measures,
        reset: cue.reset,
        effects,
        passes,
    }
}

impl TempoLayout {
    /// `map`'s lines and stretches up to `horizon`, each score time moved
    /// by the offset in force there (`steps`, see [`offset_steps`]).
    pub(crate) fn new(map: &TempoMap, horizon: Duration, steps: &[(Duration, Duration)]) -> Self {
        let at = |time: Duration| ms(shifted(steps, time));
        let mut grid = Vec::new();
        let mut signature_bars = Vec::new();
        for (measure, beat, time) in map.grid_beats() {
            if grid.len() >= MAX_GRID_LINES || time > horizon {
                break;
            }
            let downbeat = beat == 1;
            if downbeat
                && measure > 1
                && map.time_signature_at_measure(measure)
                    != map.time_signature_at_measure(measure - 1)
            {
                signature_bars.push(time);
            }
            grid.push(GridLine {
                ms: at(time),
                measure,
                beat,
                kind: if downbeat {
                    GridLineKind::Measure
                } else {
                    GridLineKind::Beat
                },
            });
        }

        // Where a stretch may begin: bar 1, each change, each ramp's end,
        // each barline a signature change governs from.
        let mut bounds = vec![map.start_offset];
        for change in &map.changes {
            if let Some(time) = change.position.absolute_time() {
                bounds.push(time);
                if let Some((end, _)) = map.transition_at(time) {
                    bounds.push(end);
                }
            }
        }
        bounds.extend(signature_bars);
        bounds.sort();
        bounds.dedup();
        bounds.retain(|&time| time <= horizon);

        let mut spans: Vec<TempoSpan> = Vec::new();
        for (i, &start) in bounds.iter().enumerate() {
            let end = bounds.get(i + 1).copied().unwrap_or(horizon);
            let bpm = map.bpm_at_time(start, 0.0);
            let bpm_to = map.transition_at(start).map(|(_, to)| to);
            let sig = map.time_signature_at_time(start, 0.0);
            let sig = (sig.numerator, sig.denominator);
            // A stretch is one label: the tempo to the hundredth a tempo
            // block is written to, where a ramp is heading, and the metre.
            let key = |bpm: f64, to: Option<f64>, sig: (u32, u32)| {
                (
                    (bpm * 100.0).round(),
                    to.map(|to| (to * 100.0).round()),
                    sig,
                )
            };
            match spans.last_mut() {
                // A change that changes nothing yet (a signature set
                // mid-bar) starts no stretch of its own.
                Some(last) if key(last.bpm, last.bpm_to, last.sig) == key(bpm, bpm_to, sig) => {
                    last.end_ms = at(end);
                }
                _ => spans.push(TempoSpan {
                    start_ms: at(start),
                    end_ms: at(end),
                    bpm,
                    bpm_to,
                    sig,
                }),
            }
        }

        TempoLayout {
            start_ms: at(map.start_offset),
            spans,
            grid,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::parser::Sources;

    fn lay(source: &str) -> Layout {
        layout(source, None, None, &Sources::none(), Duration::ZERO)
    }

    fn close(got: f64, want: f64, what: &str) {
        assert!((got - want).abs() < 1e-6, "{what}: got {got}, want {want}");
    }

    const TEMPO: &str = "tempo {\n    start: 0.0s\n    bpm: 120\n    time_signature: 4/4\n}\n";

    #[test]
    fn cues_land_where_the_player_fires_them() {
        let source = format!(
            "{TEMPO}sequence \"s\" {{\n    @1/1\n    a: static, color: \"red\", duration: 1beat\n    @1/3\n    b: pulse, frequency: 2, duration: 2beats\n}}\nshow \"S\" {{\n    @1/1\n    all: static, color: \"blue\", duration: 1measure\n    @2/1\n    sequence \"s\", loop: 3\n    offset 1 measures\n    @4/1\n    all: dimmer, start_level: 1.0, end_level: 0.0, duration: 2s\n    loop {{\n        @0.0\n        a: strobe, frequency: 4, duration: 0.25s\n    }} repeats: 2\n}}\n"
        );
        let laid = lay(&source);
        let show = &laid.shows[0];
        close(show.cues[0].ms, 0.0, "bar 1");
        close(show.cues[1].ms, 2000.0, "bar 2");
        // Three passes of a 2.5-beat sequence (last effect ends at beat
        // 3 + 2 beats) from bar 2.
        let pass = &show.cues[1].passes[0];
        close(pass.start_ms, 2000.0, "pass start");
        close(pass.pass_ms, 2000.0, "pass length");
        close(pass.end_ms, 8000.0, "passes end");
        assert_eq!(pass.count, 3);
        // Passes start at the sequence's origin (its @1/1, which the
        // sequence's first cue is written at, so this is also where its
        // earliest cue lands), a pass length (2s) apart.
        assert_eq!(pass.pass_starts, vec![2000.0, 4000.0, 6000.0]);
        // Passes laid on past the three, to the end of the timeline: a
        // resize counts the whole ones a dragged end reaches.
        assert_eq!(pass.pass_ends[..4], [4000.0, 6000.0, 8000.0, 10000.0]);
        assert_eq!(passes_by(&pass.pass_ends, 8000.0), 3);
        assert_eq!(passes_by(&pass.pass_ends, 9999.0), 3);
        assert_eq!(passes_by(&pass.pass_ends, 10000.0), 4);
        assert_eq!(passes_by(&pass.pass_ends, 6000.0), 2);
        assert_eq!(passes_by(&pass.pass_ends, 2500.0), 1);
        assert!(show.cues[1]
            .effects
            .iter()
            .all(|e| e.sequence.as_deref() == Some("s")));
        // The offset after bar 2 moves bar 4 by a bar.
        close(show.cues[1].next_offset_ms, 2000.0, "offset");
        close(show.cues[2].ms, 8000.0, "bar 4 under the offset");
        close(show.cues[2].score_ms, 6000.0, "bar 4 in the score");
        let loop_effects: Vec<_> = show.cues[2]
            .effects
            .iter()
            .filter(|e| e.kind == EffectKind::Strobe)
            .collect();
        assert_eq!(loop_effects.len(), 2);
        close(loop_effects[1].start_ms, 8250.0, "second pass of the loop");
        // The show's grid is shifted by the offset from bar 2 on.
        let grid = &show.tempo.as_ref().expect("offset grid").grid;
        let bar = |m: u32| {
            grid.iter()
                .find(|l| l.measure == m && l.beat == 1)
                .map(|l| l.ms)
                .unwrap()
        };
        close(bar(2), 2000.0, "bar 2 on the grid");
        close(bar(3), 6000.0, "bar 3 after the gap");
        assert_eq!(laid.sequences[0].name, "s");
        close(laid.sequences[0].cues[1].ms, 1000.0, "sequence beat 3");
    }

    #[test]
    fn the_tempo_lane_follows_ramps_and_metres() {
        let source = "tempo {\n    bpm: 120\n    time_signature: 4/4\n    changes: [\n        @3/1 { bpm: 180, transition: 8 },\n        @4/1 { time_signature: 3/4 },\n        @6/3 { time_signature: 2/4 }\n    ]\n}\nshow \"S\" {\n    @1/1\n    all: static, color: \"red\", duration: 1s\n}\n";
        let laid = layout(
            source,
            None,
            None,
            &Sources::none(),
            Duration::from_secs(30),
        );
        let tempo = laid.tempo.expect("tempo");
        let spans = &tempo.spans;
        let labels: Vec<(f64, Option<f64>, (u32, u32))> = spans
            .iter()
            .map(|s| ((s.bpm * 100.0).round() / 100.0, s.bpm_to, s.sig))
            .collect();
        // Bar 3 at 4 s; 8 beats averaging 150 take 3.2 s, bar 4 four beats
        // into the ramp; the mid-bar change shows from bar 7.
        assert_eq!(labels[0], (120.0, None, (4, 4)));
        assert_eq!(labels[1], (120.0, Some(180.0), (4, 4)));
        assert_eq!(labels[2].1, Some(180.0));
        assert_eq!(labels[2].2, (3, 4));
        assert_eq!(labels[3], (180.0, None, (3, 4)));
        assert_eq!(labels[4], (180.0, None, (2, 4)));
        close(spans[1].start_ms, 4000.0, "ramp start");
        close(spans[3].start_ms, 7200.0, "ramp end");
        let bar7 = tempo
            .grid
            .iter()
            .find(|l| l.measure == 7 && l.beat == 1)
            .unwrap()
            .ms;
        close(spans[4].start_ms, bar7, "2/4 from bar 7");
        let bar4 = tempo
            .grid
            .iter()
            .find(|l| l.measure == 4 && l.beat == 1)
            .unwrap()
            .ms;
        close(spans[2].start_ms, bar4, "3/4 from bar 4");
        // A 3/4 bar has three beat lines, a 2/4 bar two.
        assert_eq!(tempo.grid.iter().filter(|l| l.measure == 5).count(), 3);
        assert_eq!(tempo.grid.iter().filter(|l| l.measure == 8).count(), 2);
    }

    #[test]
    fn the_beat_lines_are_the_click_grids_beats() {
        // A 6/8 bar is six beats on the click grid, not three quarters.
        let source = "tempo {\n    bpm: 120\n    time_signature: 6/8\n}\nshow \"S\" {\n    @1/1\n    all: static, color: \"red\", duration: 1s\n}\n";
        let laid = layout(source, None, None, &Sources::none(), Duration::from_secs(3));
        let bar: Vec<(u32, f64)> = laid
            .tempo
            .expect("tempo")
            .grid
            .iter()
            .filter(|l| l.measure == 1)
            .map(|l| (l.beat, l.ms))
            .collect();
        let eighths: Vec<(u32, f64)> = (1..=6).map(|b| (b, f64::from(b - 1) * 250.0)).collect();
        assert_eq!(bar, eighths);
    }

    #[test]
    fn an_offset_and_a_reset_as_the_player_places_them() {
        // The values the web UI's old tests held the player to.
        let source = format!(
            "{TEMPO}show \"S\" {{\n    @3/1\n    all: static, color: \"red\", duration: 1beat\n    offset 4 measures\n    @6/1\n    reset_measures\n    @10/1\n    all: static, color: \"red\", duration: 1beat\n}}\n"
        );
        let laid = lay(&source);
        let cues = &laid.shows[0].cues;
        close(cues[1].ms, 10000.0 + 8000.0, "bar 6 under four bars");
        close(cues[2].ms, 18000.0, "bar 10 after the reset");
        let grid = &laid.shows[0].tempo.as_ref().unwrap().grid;
        let bar = |m: u32| {
            grid.iter()
                .find(|l| l.measure == m && l.beat == 1)
                .unwrap()
                .ms
        };
        close(bar(10), 18000.0, "bar 10 on the ruler");
        close(bar(6), 18000.0, "bar 6 on the ruler");
    }

    #[test]
    fn a_file_that_does_not_load_says_why() {
        let laid = lay("show \"S\" {\n    @2/1\n    all: static, color: \"red\"\n}\n");
        assert!(laid
            .error
            .unwrap()
            .contains("Measure-based timing requires a tempo section"));
        let laid = lay(&format!(
            "{TEMPO}show \"S\" {{\n    @1/1\n    sequence \"missing\"\n}}\n"
        ));
        assert!(laid.error.unwrap().contains("not found"));
    }

    #[test]
    fn a_loop_reference_runs_to_the_horizon() {
        let source = format!(
            "{TEMPO}sequence \"s\" {{\n    @1/1\n    a: static, color: \"red\", duration: 1beat\n}}\nshow \"S\" {{\n    @1/1\n    sequence \"s\", loop: loop\n}}\n"
        );
        let laid = layout(
            &source,
            None,
            None,
            &Sources::none(),
            Duration::from_secs(30),
        );
        let cue = &laid.shows[0].cues[0];
        assert!(cue.passes[0].infinite);
        close(cue.passes[0].end_ms, 30000.0, "to the song's end");
        assert!(cue.effects.iter().all(|e| e.start_ms <= 30000.0));
        assert!(cue.effects.len() > 50);
        close(laid.end_ms, 500.0, "the first pass is the written end");
    }
}
