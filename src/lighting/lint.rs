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
//! Non-fatal checks over a parsed light show.
//!
//! "Does it parse" is a low bar. Several classes of mistake are perfectly legal
//! DSL and silently do nothing, or silently do the wrong thing: a group that
//! matches no fixture, an effect running past the end of the song, two
//! `replace` effects fighting over the same layer and group, a `tempo` block
//! that drifts from the click track it is supposed to describe.
//!
//! These are warnings, never errors. A show that parses is still valid; these
//! only say what looks suspicious.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Duration;

use crate::audio::click_analysis::BeatGrid;
use crate::lighting::effects::{BlendMode, EffectLayer, EffectType};
use crate::lighting::parser::LayerCommandType;
use crate::lighting::parser::LightShow;

/// A non-fatal finding.
#[derive(Clone, Debug, PartialEq)]
pub struct Warning {
    /// Stable identifier, so callers can filter without matching prose.
    pub kind: &'static str,
    pub message: String,
}

impl Warning {
    fn new(kind: &'static str, message: String) -> Self {
        Warning { kind, message }
    }
}

/// What the checks can be run against. Every field is optional; a check whose
/// input is missing is skipped rather than guessed at, because a warning that
/// fires on absent information is worse than no warning.
#[derive(Default)]
pub struct LintContext<'a> {
    /// How many fixtures each targeted group resolves to in the current venue.
    /// Leave empty when no venue is loaded: every group would resolve to
    /// nothing, and reporting them all as empty is noise, not a finding.
    pub group_fixture_counts: HashMap<String, usize>,
    /// Length of the song the show accompanies.
    pub song_duration: Option<Duration>,
    /// The song's click-derived beat grid, for checking a `tempo` block against
    /// what the audio actually does.
    pub beat_grid: Option<&'a BeatGrid>,
}

/// Runs every applicable check over `shows`.
pub fn lint_shows(shows: &[LightShow], ctx: &LintContext) -> Vec<Warning> {
    let mut warnings = Vec::new();
    for show in shows {
        empty_groups(show, ctx, &mut warnings);
        tempo_disagrees_with_grid(show, ctx, &mut warnings);
        cues_beyond_the_tempo_map(show, ctx, &mut warnings);
    }
    // Across all shows at once, for the same reason the stomp check is: a
    // `clear` in one show of a file ends effects in its siblings.
    effects_past_end_of_song(shows, ctx, &mut warnings);
    parameters_the_effect_ignores(shows, &mut warnings);
    // Across all shows at once, not per show: `LightingTimeline` merges every
    // show in a file into one cue list and plays them together, so two shows
    // both driving `wash` on the background layer stomp each other exactly as
    // two cues in one show would. Checking them separately misses that.
    stomping_replace_effects(shows, &mut warnings);
    warnings
}

/// The keyword an author writes for an effect type.
fn dsl_keyword(effect_type: &EffectType) -> &'static str {
    match effect_type {
        EffectType::Static { .. } => "static",
        EffectType::ColorCycle { .. } => "cycle",
        EffectType::Strobe { .. } => "strobe",
        EffectType::Pulse { .. } => "pulse",
        EffectType::Chase { .. } => "chase",
        EffectType::Dimmer { .. } => "dimmer",
        EffectType::Rainbow { .. } => "rainbow",
    }
}

/// A parameter the effect type does not use.
///
/// The DSL accepts any parameter and each effect type takes what it recognises,
/// so `rainbow ... direction: left_to_right` parses and does nothing — the
/// author has written a setting that will never have an effect, and nothing
/// says so. Non-fatal, because shows already carry these and rejecting them
/// would break on upgrade.
fn parameters_the_effect_ignores(shows: &[LightShow], out: &mut Vec<Warning>) {
    let mut reported = HashSet::new();
    for cue in shows.iter().flat_map(|show| show.cues.iter()) {
        for effect in &cue.effects {
            for name in &effect.ignored_parameters {
                // The DSL keyword, not the Rust variant: `ColorCycle` would be
                // reported as "colorcycle", which is not a word the author can
                // have written.
                let kind = dsl_keyword(&effect.effect_type);
                // Once per (group, effect type, parameter): a show repeating
                // the same mistake on forty cues has one problem, not forty —
                // but the message names a group and a time, so two groups
                // making the same mistake must not collapse into one warning
                // pointing at whichever came first.
                if !reported.insert((effect.groups.join(", "), kind, name.clone())) {
                    continue;
                }
                out.push(Warning::new(
                    "unused-parameter",
                    format!(
                        "`{}` on `{}` at {:.3}s is not a parameter {} uses, so it does \
                         nothing — check the spelling, or the effect type",
                        name,
                        effect.groups.join(", "),
                        cue.time.as_secs_f64(),
                        kind,
                    ),
                ));
            }
        }
    }
}

/// A group that resolves to no fixtures. Its cues parse and silently no-op, so
/// a whole design layer can vanish with no signal.
fn empty_groups(show: &LightShow, ctx: &LintContext, out: &mut Vec<Warning>) {
    if ctx.group_fixture_counts.is_empty() {
        return;
    }
    let mut reported = HashSet::new();
    for cue in &show.cues {
        for effect in &cue.effects {
            for group in &effect.groups {
                if ctx.group_fixture_counts.get(group) != Some(&0) {
                    continue;
                }
                if reported.insert(group.clone()) {
                    out.push(Warning::new(
                        "empty-group",
                        format!(
                            "group `{group}` resolves to 0 fixtures — its cues will do nothing"
                        ),
                    ));
                }
            }
        }
    }
}

/// An effect whose duration runs past the end of the song. Playback truncates
/// it, which is usually not what the author meant.
fn effects_past_end_of_song(shows: &[LightShow], ctx: &LintContext, out: &mut Vec<Warning>) {
    let Some(song_end) = ctx.song_duration else {
        return;
    };
    // A song with no audio tracks has zero duration, which would put every
    // effect "past the end".
    if song_end.is_zero() {
        return;
    }
    let clears: Vec<(Option<EffectLayer>, Duration)> = shows.iter().flat_map(clear_times).collect();
    let stops: Vec<(String, Duration)> = shows.iter().flat_map(stop_times).collect();
    for cue in shows.iter().flat_map(|show| show.cues.iter()) {
        for effect in &cue.effects {
            // The authored end is not the real one. A long bed cut by a
            // `clear` or a `stop sequence` does not overrun the song, and
            // warning that it does is a false positive.
            let layer = effect.layer.unwrap_or(EffectLayer::Background);
            let end = effective_end(
                cue.time + effect.total_duration(),
                cue.time,
                layer,
                effect.sequence_name.as_deref(),
                &clears,
                &stops,
            );
            if end <= song_end {
                continue;
            }
            out.push(Warning::new(
                "past-end-of-song",
                format!(
                    "an effect on `{}` at {:.3}s runs to {:.3}s, past the song's {:.3}s — \
                     it will be cut short",
                    effect.groups.join(", "),
                    cue.time.as_secs_f64(),
                    end.as_secs_f64(),
                    song_end.as_secs_f64(),
                ),
            ));
        }
    }
}

/// Two `replace`-blend effects overlapping on the same layer *and* group. Last
/// writer wins with no diagnostic, which is the easiest way to author a cue
/// that appears to do nothing because something else is stomping it.
fn stomping_replace_effects(shows: &[LightShow], out: &mut Vec<Warning>) {
    let clears: Vec<(Option<EffectLayer>, Duration)> = shows.iter().flat_map(clear_times).collect();
    let stops: Vec<(String, Duration)> = shows.iter().flat_map(stop_times).collect();

    // (layer, group) -> the spans replace-blend effects occupy on it.
    let mut spans: BTreeMap<(EffectLayer, String), Vec<(Duration, Duration)>> = BTreeMap::new();

    for cue in shows.iter().flat_map(|show| show.cues.iter()) {
        for effect in &cue.effects {
            // Both default to the same values the engine applies when the DSL
            // omits them, so an unannotated effect is checked as it will run.
            let blend = effect.blend_mode.unwrap_or(BlendMode::Replace);
            if blend != BlendMode::Replace {
                continue;
            }
            let layer = effect.layer.unwrap_or(EffectLayer::Background);
            // The authored end is not the real one: a `clear` on this layer,
            // or a `stop sequence` naming this effect's sequence, ends it
            // there. Comparing authored spans would report a stomp between two
            // effects that never coexist.
            let end = effective_end(
                cue.time + effect.total_duration(),
                cue.time,
                layer,
                effect.sequence_name.as_deref(),
                &clears,
                &stops,
            );
            if end <= cue.time {
                continue;
            }
            for group in &effect.groups {
                spans
                    .entry((layer, group.clone()))
                    .or_default()
                    .push((cue.time, end));
            }
        }
    }

    for ((layer, group), mut occupied) in spans {
        occupied.sort();
        for pair in occupied.windows(2) {
            let ((_, first_end), (second_start, _)) = (pair[0], pair[1]);
            // Touching is fine — one ends exactly where the next begins.
            if second_start >= first_end {
                continue;
            }
            out.push(Warning::new(
                "replace-overlap",
                format!(
                    "two `replace` effects overlap on `{group}` in the {} layer between \
                     {:.3}s and {:.3}s — the later one wins and the earlier appears to do nothing",
                    format!("{layer:?}").to_lowercase(),
                    second_start.as_secs_f64(),
                    first_end.as_secs_f64(),
                ),
            ));
            break; // One report per layer+group is enough to send them looking.
        }
    }
}

/// When each layer is cleared. `None` as the layer key means `clear()` with no
/// argument, which clears every layer.
fn clear_times(show: &LightShow) -> Vec<(Option<EffectLayer>, Duration)> {
    let mut times: Vec<(Option<EffectLayer>, Duration)> = show
        .cues
        .iter()
        .flat_map(|cue| {
            cue.layer_commands
                .iter()
                .filter(|cmd| cmd.command_type == LayerCommandType::Clear)
                .map(move |cmd| (cmd.layer, cue.time))
        })
        .collect();
    times.sort_by_key(|(_, at)| *at);
    times
}

/// When each named sequence is explicitly stopped.
fn stop_times(show: &LightShow) -> Vec<(String, Duration)> {
    let mut times: Vec<(String, Duration)> = show
        .cues
        .iter()
        .flat_map(|cue| {
            cue.stop_sequences
                .iter()
                .map(move |name| (name.clone(), cue.time))
        })
        .collect();
    times.sort_by_key(|(_, at)| *at);
    times
}

/// An effect's real end: the earliest of its authored end, a `clear` affecting
/// its layer, and a `stop sequence` naming the sequence it came from.
///
/// Both callers go through here rather than each capping by the mechanisms it
/// happens to remember. They did not always: the stomp check capped by clears
/// and stops while the past-end check capped by clears alone, so the two
/// disagreed about the same effect — one treating a stopped bed as ending at
/// the stop, the other warning that it overran the song by the difference.
fn effective_end(
    authored_end: Duration,
    start: Duration,
    layer: EffectLayer,
    sequence: Option<&str>,
    clears: &[(Option<EffectLayer>, Duration)],
    stops: &[(String, Duration)],
) -> Duration {
    let cleared_at = clears
        .iter()
        .filter(|(cleared, at)| *at > start && cleared.is_none_or(|l| l == layer))
        .map(|(_, at)| *at);
    // `LightingEngine::stop_sequence` drops every effect carrying the
    // sequence's id prefix, so a stop really is an end.
    let stopped_at = stops
        .iter()
        .filter(|(name, at)| Some(name.as_str()) == sequence && *at > start)
        .map(|(_, at)| *at);

    cleared_at
        .chain(stopped_at)
        .filter(|at| *at < authored_end)
        .min()
        .unwrap_or(authored_end)
}

/// A `tempo` block that disagrees with the click-derived grid. Silent, and
/// produces a show that is right for the first N bars and wrong after the first
/// meter change.
fn tempo_disagrees_with_grid(show: &LightShow, ctx: &LintContext, out: &mut Vec<Warning>) {
    let (Some(block), Some(grid)) = (show.tempo_map.as_ref(), ctx.beat_grid) else {
        return;
    };
    let Some(measure_count) = grid.measure_starts.len().checked_sub(1) else {
        return;
    };
    if measure_count == 0 {
        return;
    }

    // Compare bar by bar: where does the block put each downbeat, and where
    // does the audio? A block that agrees drifts by milliseconds; one that
    // disagrees diverges and stays diverged.
    const TOLERANCE: f64 = 0.050;
    for measure in 0..measure_count {
        let Some(&beat_index) = grid.measure_starts.get(measure) else {
            break;
        };
        let Some(&grid_time) = grid.beats.get(beat_index) else {
            break;
        };
        let Some(block_time) = block.measure_to_time_with_offset(measure as u32 + 1, 1.0, 0, 0.0)
        else {
            break;
        };
        let drift = (block_time.as_secs_f64() - grid_time).abs();
        if drift <= TOLERANCE {
            continue;
        }
        out.push(Warning::new(
            "tempo-grid-mismatch",
            format!(
                "the tempo block puts bar {} at {:.3}s but the click track has it at {:.3}s \
                 ({:.3}s off) — the block and the audio disagree from here on",
                measure + 1,
                block_time.as_secs_f64(),
                grid_time,
                drift,
            ),
        ));
        return; // The first divergence is the useful one; the rest follow from it.
    }
}

/// A cue positioned past the end of the tempo map. `@150/1` on a 102-measure
/// song resolves to something rather than erroring.
fn cues_beyond_the_tempo_map(show: &LightShow, ctx: &LintContext, out: &mut Vec<Warning>) {
    let Some(map) = show.tempo_map.as_ref() else {
        return;
    };
    // A tempo map alone is unbounded — it will happily place bar 10,000. Only
    // the grid says where the music actually stops, so without one this check
    // cannot run.
    let Some(last_measure) = grid_measure_count(ctx) else {
        return;
    };

    for (index, cue) in show.cues.iter().enumerate() {
        let (measure, _) = map.time_to_measure_beat(cue.time);
        if measure as usize <= last_measure {
            continue;
        }
        out.push(Warning::new(
            "cue-past-end",
            format!("cue {index} resolves to bar {measure}, past the song's {last_measure} bars"),
        ));
        return;
    }
}

/// How many bars the grid covers.
///
/// `measure_starts` holds one index per downbeat, so its length is the number
/// of bars the song reaches. (`BeatGrid::measure_count` subtracts one because
/// it counts bars of *known duration*; for "is this cue past the end", a cue in
/// the final bar is inside the song.)
fn grid_measure_count(ctx: &LintContext) -> Option<usize> {
    ctx.beat_grid
        .map(|g| g.measure_starts.len())
        .filter(|n| *n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::parser::{parse_light_shows, parse_light_shows_with_tempo};

    fn shows(source: &str) -> Vec<LightShow> {
        parse_light_shows(source)
            .expect("test show should parse")
            .into_values()
            .collect()
    }

    fn kinds(warnings: &[Warning]) -> Vec<&str> {
        warnings.iter().map(|w| w.kind).collect()
    }

    /// A steady grid of `measures` bars of 4 beats at `bpm`, starting at zero.
    ///
    /// Shaped the way real grids are: one `measure_starts` entry per downbeat
    /// and nothing after the last beat. An earlier version appended a trailing
    /// sentinel that neither `from_tempo_map` nor `analyze_click_track`
    /// produces, which hid an off-by-one in the bar count.
    fn grid(bpm: f64, measures: usize) -> BeatGrid {
        let spacing = 60.0 / bpm;
        let mut beats = Vec::new();
        let mut measure_starts = Vec::new();
        for m in 0..measures {
            measure_starts.push(beats.len());
            for b in 0..4 {
                beats.push((m * 4 + b) as f64 * spacing);
            }
        }
        BeatGrid {
            beats,
            measure_starts,
        }
    }

    // ── empty groups ───────────────────────────────────────────────

    #[test]
    fn a_group_resolving_to_nothing_is_reported_once() {
        let source = r#"
show "T" {
    @00:00.000
    movers: static color: "blue", duration: 2s

    @00:04.000
    movers: static color: "red", duration: 2s
}
"#;
        let ctx = LintContext {
            group_fixture_counts: HashMap::from([("movers".to_string(), 0)]),
            ..Default::default()
        };
        let warnings = lint_shows(&shows(source), &ctx);
        assert_eq!(kinds(&warnings), ["empty-group"]);
        assert!(warnings[0].message.contains("movers"));
    }

    #[test]
    fn a_group_that_resolves_is_not_reported() {
        let source = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 2s
}
"#;
        let ctx = LintContext {
            group_fixture_counts: HashMap::from([("wash".to_string(), 4)]),
            ..Default::default()
        };
        assert!(lint_shows(&shows(source), &ctx).is_empty());
    }

    #[test]
    fn without_a_venue_the_group_check_does_not_guess() {
        let source = r#"
show "T" {
    @00:00.000
    movers: static color: "blue", duration: 2s
}
"#;
        assert!(lint_shows(&shows(source), &LintContext::default()).is_empty());
    }

    // ── past end of song ───────────────────────────────────────────

    #[test]
    fn an_effect_running_past_the_song_is_reported() {
        let source = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 30s
}
"#;
        let ctx = LintContext {
            song_duration: Some(Duration::from_secs(10)),
            ..Default::default()
        };
        let warnings = lint_shows(&shows(source), &ctx);
        assert_eq!(kinds(&warnings), ["past-end-of-song"]);
    }

    #[test]
    fn an_effect_ending_exactly_at_the_song_end_is_fine() {
        let source = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 10s
}
"#;
        let ctx = LintContext {
            song_duration: Some(Duration::from_secs(10)),
            ..Default::default()
        };
        assert!(lint_shows(&shows(source), &ctx).is_empty());
    }

    // ── stomping replace effects ───────────────────────────────────

    #[test]
    fn overlapping_replace_effects_on_one_group_are_reported() {
        let source = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 10s

    @00:05.000
    wash: static color: "red", duration: 10s
}
"#;
        let warnings = lint_shows(&shows(source), &LintContext::default());
        assert_eq!(kinds(&warnings), ["replace-overlap"]);
        assert!(warnings[0].message.contains("wash"), "{warnings:?}");
    }

    #[test]
    fn abutting_effects_are_not_an_overlap() {
        let source = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 5s

    @00:05.000
    wash: static color: "red", duration: 5s
}
"#;
        assert!(lint_shows(&shows(source), &LintContext::default()).is_empty());
    }

    #[test]
    fn overlaps_on_different_layers_do_not_stomp() {
        let source = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 10s, layer: background

    @00:05.000
    wash: static color: "red", duration: 10s, layer: foreground
}
"#;
        assert!(lint_shows(&shows(source), &LintContext::default()).is_empty());
    }

    #[test]
    fn a_non_replace_blend_is_not_stomping() {
        // `add` is how you deliberately stack effects; only `replace` clobbers.
        let source = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 10s, blend_mode: add

    @00:05.000
    wash: static color: "red", duration: 10s, blend_mode: add
}
"#;
        assert!(lint_shows(&shows(source), &LintContext::default()).is_empty());
    }

    #[test]
    fn overlaps_on_different_groups_do_not_stomp() {
        let source = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 10s

    @00:05.000
    spot: static color: "red", duration: 10s
}
"#;
        assert!(lint_shows(&shows(source), &LintContext::default()).is_empty());
    }

    #[test]
    fn two_shows_in_one_file_can_stomp_each_other() {
        // A file's shows are merged into one timeline and played together, so
        // they contend for a layer exactly as two cues in one show would.
        let source = r#"
show "A" {
    @00:00.000
    wash: static color: "blue", duration: 10s
}

show "B" {
    @00:05.000
    wash: static color: "red", duration: 10s
}
"#;
        let warnings = lint_shows(&shows(source), &LintContext::default());
        assert_eq!(kinds(&warnings), ["replace-overlap"], "{warnings:?}");
    }

    #[test]
    fn a_clear_between_two_effects_prevents_the_stomp() {
        // The bed is authored for 30s and would overlap the later cue, but the
        // clear at 5s ends it. Reporting a stomp here would be a false alarm,
        // and a lint that cries wolf gets ignored.
        let source = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 30s

    @00:05.000
    clear(layer: background)

    @00:08.000
    wash: static color: "red", duration: 2s
}
"#;
        assert!(
            lint_shows(&shows(source), &LintContext::default()).is_empty(),
            "the clear ends the bed before the second cue starts"
        );
    }

    #[test]
    fn a_clear_on_another_layer_does_not_rescue_an_overlap() {
        let source = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 30s, layer: background

    @00:05.000
    clear(layer: foreground)

    @00:08.000
    wash: static color: "red", duration: 2s, layer: background
}
"#;
        let warnings = lint_shows(&shows(source), &LintContext::default());
        assert_eq!(kinds(&warnings), ["replace-overlap"], "{warnings:?}");
    }

    #[test]
    fn a_bare_clear_ends_every_layer() {
        let source = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 30s, layer: foreground

    @00:05.000
    clear()

    @00:08.000
    wash: static color: "red", duration: 2s, layer: foreground
}
"#;
        assert!(lint_shows(&shows(source), &LintContext::default()).is_empty());
    }

    // ── tempo block vs beat grid ───────────────────────────────────

    #[test]
    fn a_tempo_block_matching_the_grid_is_quiet() {
        let source = r#"
tempo {
    start: 0ms
    bpm: 120
    time_signature: 4/4
}

show "T" {
    @1/1
    wash: static color: "blue", duration: 1s
}
"#;
        let g = grid(120.0, 8);
        let ctx = LintContext {
            beat_grid: Some(&g),
            ..Default::default()
        };
        assert!(
            lint_shows(&shows(source), &ctx).is_empty(),
            "a correct block should be silent"
        );
    }

    #[test]
    fn a_tempo_block_drifting_from_the_grid_is_reported_at_first_divergence() {
        // The block claims 120bpm; the click track is at 100. Bar 1 agrees, and
        // everything after it does not.
        let source = r#"
tempo {
    start: 0ms
    bpm: 120
    time_signature: 4/4
}

show "T" {
    @1/1
    wash: static color: "blue", duration: 1s
}
"#;
        let g = grid(100.0, 8);
        let ctx = LintContext {
            beat_grid: Some(&g),
            ..Default::default()
        };
        let warnings = lint_shows(&shows(source), &ctx);
        assert_eq!(kinds(&warnings), ["tempo-grid-mismatch"], "{warnings:?}");
        // Reported once, at the first bar that disagrees, not at all of them.
        assert!(warnings[0].message.contains("bar 2"), "{warnings:?}");
    }

    #[test]
    fn without_a_grid_the_tempo_check_does_not_run() {
        let source = r#"
tempo {
    start: 0ms
    bpm: 120
    time_signature: 4/4
}

show "T" {
    @1/1
    wash: static color: "blue", duration: 1s
}
"#;
        assert!(lint_shows(&shows(source), &LintContext::default()).is_empty());
    }

    // ── cues past the end ──────────────────────────────────────────

    #[test]
    fn a_cue_past_the_last_bar_is_reported() {
        let map = crate::tempo::TempoMap::new(
            Duration::ZERO,
            120.0,
            crate::tempo::TimeSignature::new(4, 4),
            vec![],
        );
        let source = r#"
show "T" {
    @1/1
    wash: static color: "blue", duration: 1s

    @50/1
    wash: static color: "red", duration: 1s
}
"#;
        let parsed: Vec<LightShow> = parse_light_shows_with_tempo(source, Some(&map))
            .expect("parses")
            .into_values()
            .collect();
        let g = grid(120.0, 8);
        let ctx = LintContext {
            beat_grid: Some(&g),
            ..Default::default()
        };
        let warnings = lint_shows(&parsed, &ctx);
        assert_eq!(kinds(&warnings), ["cue-past-end"], "{warnings:?}");
        assert!(warnings[0].message.contains("bar 50"), "{warnings:?}");
    }

    #[test]
    fn cues_inside_the_song_are_quiet() {
        let map = crate::tempo::TempoMap::new(
            Duration::ZERO,
            120.0,
            crate::tempo::TimeSignature::new(4, 4),
            vec![],
        );
        let source = r#"
show "T" {
    @1/1
    wash: static color: "blue", duration: 1s

    @4/1
    wash: static color: "red", duration: 1s
}
"#;
        let parsed: Vec<LightShow> = parse_light_shows_with_tempo(source, Some(&map))
            .expect("parses")
            .into_values()
            .collect();
        let g = grid(120.0, 8);
        let ctx = LintContext {
            beat_grid: Some(&g),
            ..Default::default()
        };
        assert!(lint_shows(&parsed, &ctx).is_empty());
    }

    #[test]
    fn a_cue_in_the_songs_last_bar_is_not_past_the_end() {
        // The bar count is the number of downbeats. Counting bars of known
        // duration instead reports the final bar as out of range.
        let map = crate::tempo::TempoMap::new(
            Duration::ZERO,
            120.0,
            crate::tempo::TimeSignature::new(4, 4),
            vec![],
        );
        let source = r#"
show "T" {
    @8/1
    wash: static color: "blue", duration: 1s
}
"#;
        let parsed: Vec<LightShow> = parse_light_shows_with_tempo(source, Some(&map))
            .expect("parses")
            .into_values()
            .collect();
        let g = grid(120.0, 8);
        let ctx = LintContext {
            beat_grid: Some(&g),
            ..Default::default()
        };
        assert!(
            lint_shows(&parsed, &ctx).is_empty(),
            "bar 8 of an 8-bar song is inside it"
        );
    }

    #[test]
    fn an_effect_cut_by_a_clear_does_not_overrun_the_song() {
        // The long-bed-plus-clear idiom, which `analyze`'s own tests use. The
        // bed is authored for 60s but ends at 5s, well inside a 30s song.
        let source = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 60s

    @00:05.000
    clear(layer: background)
}
"#;
        let ctx = LintContext {
            song_duration: Some(Duration::from_secs(30)),
            ..Default::default()
        };
        assert!(
            lint_shows(&shows(source), &ctx).is_empty(),
            "the clear ends the bed long before the song does"
        );
    }

    #[test]
    fn an_effect_cut_by_a_stop_sequence_does_not_overrun_the_song() {
        // The same claim as the clear case, by the other mechanism that ends an
        // effect early. `LightingEngine::stop_sequence` drops every effect whose
        // id carries the sequence's prefix, so the 60s bed really ends at 15s —
        // well inside a 30s song.
        //
        // The stomp check already reasons this way. When only that one did, two
        // lints in the same file disagreed about the same effect.
        let source = r#"
sequence "X" {
    @00:00.000
    wash: static color: "blue", duration: 60s
}

show "T" {
    @00:10.000
    sequence "X"

    @00:15.000
    stop sequence "X"
}
"#;
        let ctx = LintContext {
            song_duration: Some(Duration::from_secs(30)),
            ..Default::default()
        };
        assert!(
            lint_shows(&shows(source), &ctx).is_empty(),
            "the stop ends the bed long before the song does, but got: {:?}",
            lint_shows(&shows(source), &ctx)
        );
    }

    /// The shipped shows must not carry a parameter that does nothing.
    ///
    /// They carried nineteen: `fade`, `loop`, `duty`, `dimmer` on a cycle,
    /// `intensity` on a strobe, `color` on pulse/chase/strobe, and the rainbow
    /// `direction` that led to this check. Anyone copying an example inherited
    /// settings that are silently ignored, which is how the habit spreads.
    #[test]
    fn the_shipped_examples_carry_no_parameter_that_does_nothing() {
        fn shows_in(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    shows_in(&path, out);
                } else if path.extension().and_then(|e| e.to_str()) == Some("light") {
                    out.push(path);
                }
            }
        }

        let mut files = Vec::new();
        shows_in(std::path::Path::new("examples"), &mut files);
        assert!(!files.is_empty(), "no example shows were found to check");

        let mut offenders = Vec::new();
        for path in files {
            let Ok(source) = std::fs::read_to_string(&path) else {
                continue;
            };
            // Fixture-type and venue files live alongside the shows and are
            // not shows, so a parse failure is expected for them — but a show
            // that stops parsing must not drop silently out of this check.
            let is_show = source.contains("show \"");
            let parsed = match crate::lighting::parser::parse_light_shows(&source) {
                Ok(parsed) => parsed,
                Err(e) => {
                    assert!(
                        !is_show,
                        "{} declares a show but does not parse: {e}",
                        path.display()
                    );
                    continue;
                }
            };
            let shows: Vec<_> = parsed.into_values().collect();
            for warning in lint_shows(&shows, &LintContext::default()) {
                if warning.kind == "unused-parameter" {
                    offenders.push(format!("{}: {}", path.display(), warning.message));
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "shipped examples carry parameters that do nothing:\n  {}",
            offenders.join("\n  ")
        );
    }

    #[test]
    fn a_parameter_the_effect_does_not_use_is_reported() {
        // `EffectType::Rainbow` has no direction. This parses, the direction is
        // dropped, and the author has written a setting that will never do
        // anything — which is what the web UI's own rainbow direction control
        // used to produce before it was removed.
        let source = r#"
show "T" {
    @00:00.000
    wash: rainbow speed: 0.5, direction: left_to_right, duration: 4s
}
"#;
        let warnings = lint_shows(&shows(source), &LintContext::default());
        let unused: Vec<&Warning> = warnings
            .iter()
            .filter(|w| w.kind == "unused-parameter")
            .collect();
        assert_eq!(unused.len(), 1, "{warnings:?}");
        assert!(
            unused[0].message.contains("direction") && unused[0].message.contains("rainbow"),
            "the parameter and the effect type both have to be named: {}",
            unused[0].message
        );
    }

    #[test]
    fn a_parameter_the_effect_does_use_is_not_reported() {
        // Guards against the check firing on every parameter, which would make
        // it noise and get it ignored.
        let source = r#"
show "T" {
    @00:00.000
    wash: rainbow speed: 0.5, saturation: 80%, brightness: 90%, duration: 4s
}
"#;
        let warnings = lint_shows(&shows(source), &LintContext::default());
        assert!(
            !warnings.iter().any(|w| w.kind == "unused-parameter"),
            "{warnings:?}"
        );
    }

    #[test]
    fn the_same_mistake_on_many_cues_is_reported_once() {
        let source = r#"
show "T" {
    @00:00.000
    wash: rainbow speed: 0.5, direction: left_to_right, duration: 2s

    @00:04.000
    wash: rainbow speed: 0.5, direction: left_to_right, duration: 2s
}
"#;
        let warnings = lint_shows(&shows(source), &LintContext::default());
        assert_eq!(
            warnings
                .iter()
                .filter(|w| w.kind == "unused-parameter")
                .count(),
            1,
            "a repeated mistake is one problem, not one per cue: {warnings:?}"
        );
    }

    #[test]
    fn a_song_with_no_duration_is_not_warned_about() {
        // A song with no audio tracks has zero duration; warning that every
        // effect overruns it is noise.
        let source = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 4s
}
"#;
        let ctx = LintContext {
            song_duration: Some(Duration::ZERO),
            ..Default::default()
        };
        assert!(lint_shows(&shows(source), &ctx).is_empty());
    }

    // ── nothing to say ─────────────────────────────────────────────

    #[test]
    fn a_clean_show_produces_no_warnings() {
        let source = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 4s

    @00:04.000
    wash: static color: "red", duration: 4s
}
"#;
        let ctx = LintContext {
            group_fixture_counts: HashMap::from([("wash".to_string(), 4)]),
            song_duration: Some(Duration::from_secs(60)),
            ..Default::default()
        };
        assert!(lint_shows(&shows(source), &ctx).is_empty());
    }
}
