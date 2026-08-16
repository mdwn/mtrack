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

//! Coverage analysis for a light show.
//!
//! Answers the questions that are invisible in the source and error-prone to
//! compute by hand: where does the rig go dark, how long is each group actually
//! doing something, and how is the work spread across layers.
//!
//! Getting this right needs every duration resolved through the tempo map —
//! `1measure` is 1.5s inside a 2/4 bar and 3.0s inside a 4/4 one — and needs
//! layer commands honoured, since a `clear` truncates effects that the source
//! says run much longer. Both fall out of driving the real timeline and engine
//! rather than reading the text.
//!
//! **Darkness here means "no effect is running", not "every channel is zero".**
//! A strobe is dark half its life by design, and reporting its off-phase as a
//! gap would bury the real ones. An effect that is running but black — a
//! `static` at `dimmer: 0%` — is a deliberate authoring choice and counts as
//! covered.

use std::collections::{BTreeMap, HashSet};
use std::time::Duration;

use crate::lighting::effects::{EffectInstance, EffectLayer, FixtureInfo};
use crate::lighting::evaluate::apply_timeline_update;
use crate::lighting::parser::LightShow;
use crate::lighting::tempo::TempoMap;
use crate::lighting::timeline::LightingTimeline;
use crate::lighting::EffectEngine;

/// A span during which nothing is running.
#[derive(Clone, Debug, PartialEq)]
pub struct DarkWindow {
    pub from: Duration,
    pub to: Duration,
    /// Musical position of `from`, when the show has a tempo map.
    pub position: Option<String>,
}

impl DarkWindow {
    pub fn seconds(&self) -> f64 {
        self.to.saturating_sub(self.from).as_secs_f64()
    }
}

/// What a show does over its own span.
#[derive(Clone, Debug, Default)]
pub struct Analysis {
    pub cue_count: usize,
    /// First moment anything runs to the last moment anything runs.
    ///
    /// This is where activity actually stops, not the furthest authored
    /// duration: an effect written as 60s but cut by a `clear` at 5s ends the
    /// span at 5s, matching the gap analysis rather than contradicting it.
    pub span: Duration,
    pub dark_windows: Vec<DarkWindow>,
    /// Seconds each authored group name is covered by at least one effect.
    /// Union, not sum — two effects on one group at once is still one second.
    pub per_group_seconds: BTreeMap<String, f64>,
    /// Seconds each layer is covered by at least one effect, unioned the same way.
    pub per_layer_seconds: BTreeMap<EffectLayer, f64>,
}

impl Analysis {
    /// Total dark time across the span.
    pub fn dark_seconds(&self) -> f64 {
        self.dark_windows.iter().map(|w| w.seconds()).sum()
    }
}

/// Analyses `shows` without audio, DMX, or the wall clock.
///
/// Group names are reported as authored, not resolved to fixtures: the author
/// reasons in group names, and a group that resolves to nothing still shows the
/// time it was *meant* to cover. Whether a group resolves is a separate
/// question, answered where the venue is known.
pub fn analyze_show(shows: Vec<LightShow>, fallback_tempo_map: Option<&TempoMap>) -> Analysis {
    let cue_count: usize = shows.iter().map(|s| s.cues.len()).sum();

    let mut timeline = LightingTimeline::new(shows);
    let tempo_map = timeline
        .tempo_map()
        .cloned()
        .or_else(|| fallback_tempo_map.cloned());

    // The state can only change where a cue fires or an effect ends, so those
    // are the only boundaries worth testing. Sampling on a fixed grid instead
    // would quantise every window to the step size and still cost more.
    let boundaries = boundaries(&timeline);
    if boundaries.is_empty() {
        return Analysis {
            cue_count,
            ..Default::default()
        };
    }

    let mut engine = EffectEngine::new();
    engine.set_tempo_map(tempo_map.clone());
    // Register a stand-in fixture per authored group name. The engine rejects
    // effects targeting names it does not know, and analysis is about the show,
    // not the rig — a group that resolves to nothing in the current venue still
    // occupies the time the author gave it.
    for name in target_names(&timeline) {
        engine.register_fixture(stand_in_fixture(&name));
    }
    timeline.start();

    let mut analysis = Analysis {
        cue_count,
        ..Default::default()
    };
    let mut dark_from: Option<Duration> = None;
    let mut first_active: Option<Duration> = None;
    let mut last_active_end = Duration::ZERO;
    let mut previous = Duration::ZERO;

    for pair in boundaries.windows(2) {
        let (start, end) = (pair[0], pair[1]);
        // Probe the middle of the interval: the endpoints are exactly where
        // effects start and stop, so they are the one place the answer is
        // ambiguous.
        let probe = start + (end - start) / 2;

        for at in [start, probe] {
            let update = timeline.update(at);
            apply_timeline_update(&mut engine, update, &mut |e| e);
            let _ = engine.update(at.saturating_sub(previous), Some(at));
            previous = at;
        }

        let active: Vec<&EffectInstance> = engine.get_active_effects().values().collect();
        let seconds = (end - start).as_secs_f64();

        if active.is_empty() {
            dark_from.get_or_insert(start);
        } else {
            if let Some(from) = dark_from.take() {
                analysis.dark_windows.push(window(from, start, &tempo_map));
            }
            first_active.get_or_insert(start);
            last_active_end = end;
            // A group or layer covered by any running effect earns the whole
            // interval once, however many effects are stacked on it.
            let mut groups: HashSet<&str> = HashSet::new();
            let mut layers: HashSet<EffectLayer> = HashSet::new();
            for effect in active {
                groups.extend(effect.target_fixtures.iter().map(String::as_str));
                layers.insert(effect.layer);
            }
            for group in groups {
                *analysis
                    .per_group_seconds
                    .entry(group.to_string())
                    .or_default() += seconds;
            }
            for layer in layers {
                *analysis.per_layer_seconds.entry(layer).or_default() += seconds;
            }
        }
    }

    // A run of dark intervals reaching the end of the show is not a gap in the
    // show, it is the end of it.
    dark_from.take();

    analysis.span = last_active_end.saturating_sub(first_active.unwrap_or_default());
    analysis
}

/// Every distinct name the show's cues target.
fn target_names(timeline: &LightingTimeline) -> Vec<String> {
    let mut names: Vec<String> = timeline
        .cue_effects()
        .flat_map(|(_, effects)| effects.iter().flat_map(|e| e.groups.iter().cloned()))
        .collect();
    names.sort();
    names.dedup();
    names
}

/// A minimal RGB+dimmer fixture, so effects targeting a group name validate.
/// Never addressed and never rendered — only its presence matters.
fn stand_in_fixture(name: &str) -> FixtureInfo {
    let channels = [("red", 1), ("green", 2), ("blue", 3), ("dimmer", 4)]
        .into_iter()
        .map(|(c, offset)| (c.to_string(), offset))
        .collect();
    FixtureInfo::new(
        name.to_string(),
        1,
        1,
        "analysis-stand-in".to_string(),
        channels,
        None,
    )
}

/// Every instant the running set can change: each cue time, and each effect's
/// end. Sorted and deduplicated.
fn boundaries(timeline: &LightingTimeline) -> Vec<Duration> {
    let mut times: Vec<Duration> = Vec::new();
    for (time, effects) in timeline.cue_effects() {
        times.push(time);
        for effect in effects {
            times.push(time + effect.total_duration());
        }
    }
    times.sort();
    times.dedup();
    times
}

fn window(from: Duration, to: Duration, tempo_map: &Option<TempoMap>) -> DarkWindow {
    let position = tempo_map.as_ref().map(|map| {
        let (measure, beat) = map.time_to_measure_beat(from);
        format!("@{measure}/{beat:.2}")
    });
    DarkWindow { from, to, position }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::parser::parse_light_shows;

    fn shows(source: &str) -> Vec<LightShow> {
        let mut parsed: Vec<_> = parse_light_shows(source)
            .expect("test show should parse")
            .into_values()
            .collect();
        parsed.sort_by(|a, b| a.name.cmp(&b.name));
        parsed
    }

    fn analyze(source: &str) -> Analysis {
        analyze_show(shows(source), None)
    }

    fn windows(analysis: &Analysis) -> Vec<(f64, f64)> {
        analysis
            .dark_windows
            .iter()
            .map(|w| (w.from.as_secs_f64(), w.to.as_secs_f64()))
            .collect()
    }

    #[test]
    fn a_gap_between_two_cues_is_a_dark_window() {
        let analysis = analyze(
            r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 4s

    @00:06.000
    wash: static color: "red", duration: 4s
}
"#,
        );
        assert_eq!(analysis.cue_count, 2);
        assert_eq!(windows(&analysis), [(4.0, 6.0)]);
        assert_eq!(analysis.dark_seconds(), 2.0);
        assert_eq!(analysis.span, Duration::from_secs(10));
    }

    #[test]
    fn a_show_with_no_gaps_reports_none() {
        let analysis = analyze(
            r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 5s

    @00:05.000
    wash: static color: "red", duration: 5s
}
"#,
        );
        assert!(analysis.dark_windows.is_empty());
        assert_eq!(analysis.dark_seconds(), 0.0);
    }

    #[test]
    fn overlapping_effects_leave_no_gap() {
        let analysis = analyze(
            r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 10s

    @00:04.000
    spot: static color: "red", duration: 2s
}
"#,
        );
        assert!(
            analysis.dark_windows.is_empty(),
            "the bed covers the whole span: {:?}",
            analysis.dark_windows
        );
        // Union, not sum: the overlap is not counted twice for either group.
        assert_eq!(analysis.per_group_seconds["wash"], 10.0);
        assert_eq!(analysis.per_group_seconds["spot"], 2.0);
    }

    #[test]
    fn trailing_silence_is_not_a_dark_window() {
        // The show simply ends. Reporting the end as a gap would put a false
        // finding on every show ever written.
        let analysis = analyze(
            r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 4s
}
"#,
        );
        assert!(analysis.dark_windows.is_empty());
        assert_eq!(analysis.span, Duration::from_secs(4));
    }

    #[test]
    fn a_clear_opens_a_gap_the_source_does_not_show() {
        // The source says 60s. The clear at 5s means the rig is dark from
        // there until the next cue — the class of gap that is invisible when
        // reading durations off the page.
        let analysis = analyze(
            r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 60s

    @00:05.000
    clear(layer: background)

    @00:08.000
    wash: static color: "red", duration: 2s
}
"#,
        );
        assert_eq!(windows(&analysis), [(5.0, 8.0)]);
        assert_eq!(
            analysis.per_group_seconds["wash"], 7.0,
            "5s before the clear plus the 2s cue, not the authored 60s"
        );
    }

    #[test]
    fn a_strobes_off_phase_is_not_a_gap() {
        // Darkness is "nothing running", not "every channel zero". An 8Hz
        // strobe is dark half its life by design.
        let analysis = analyze(
            r#"
show "T" {
    @00:00.000
    wash: strobe frequency: 8, duration: 5s
}
"#,
        );
        assert!(
            analysis.dark_windows.is_empty(),
            "strobe phase should not read as gaps: {:?}",
            analysis.dark_windows
        );
        assert_eq!(analysis.per_group_seconds["wash"], 5.0);
    }

    #[test]
    fn layer_seconds_are_unioned_per_layer() {
        let analysis = analyze(
            r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 10s, layer: background
    spot: static color: "red", duration: 4s, layer: background
    beam: strobe frequency: 4, duration: 6s, layer: foreground
}
"#,
        );
        // Two background effects overlap; the layer still earns 10s, not 14.
        assert_eq!(analysis.per_layer_seconds[&EffectLayer::Background], 10.0);
        assert_eq!(analysis.per_layer_seconds[&EffectLayer::Foreground], 6.0);
        assert!(!analysis
            .per_layer_seconds
            .contains_key(&EffectLayer::Midground));
    }

    #[test]
    fn several_gaps_are_reported_separately() {
        let analysis = analyze(
            r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 2s

    @00:03.000
    wash: static color: "red", duration: 2s

    @00:07.000
    wash: static color: "green", duration: 2s
}
"#,
        );
        assert_eq!(windows(&analysis), [(2.0, 3.0), (5.0, 7.0)]);
        assert_eq!(analysis.dark_seconds(), 3.0);
    }

    #[test]
    fn measure_durations_resolve_through_the_tempo_map() {
        // The reason this cannot be done by reading the source: `1measure` is
        // 1.5s in the 2/4 bar and 3.0s either side of it.
        let analysis = analyze(
            r#"
tempo {
    start: 0ms
    bpm: 80
    time_signature: 4/4
    changes: [
        @2/1 { time_signature: 2/4 },
        @3/1 { time_signature: 4/4 }
    ]
}

show "T" {
    @1/1
    wash: static color: "blue", duration: 1measures

    @3/1
    wash: static color: "red", duration: 1measures
}
"#,
        );
        // 80bpm: a 4/4 bar is 3.0s, the 2/4 bar is 1.5s. Bar 1 covers 0.0-3.0,
        // bar 2 is the short one (3.0-4.5), so bar 3 starts at 4.5.
        let gaps = windows(&analysis);
        assert_eq!(gaps.len(), 1, "one gap across the short bar: {gaps:?}");
        let (from, to) = gaps[0];
        assert!((from - 3.0).abs() < 0.01, "gap starts at bar 2: {from}");
        assert!((to - 4.5).abs() < 0.01, "2/4 bar is 1.5s, not 3.0s: {to}");
    }

    #[test]
    fn dark_windows_carry_a_musical_position_when_tempo_is_known() {
        let analysis = analyze(
            r#"
tempo {
    start: 0ms
    bpm: 120
    time_signature: 4/4
}

show "T" {
    @1/1
    wash: static color: "blue", duration: 1s

    @2/1
    wash: static color: "red", duration: 2s
}
"#,
        );
        // At 120bpm a 4/4 bar is 2s, so bar 2 starts at 2.0 and the 1s effect
        // leaves a second of nothing behind it.
        assert_eq!(windows(&analysis), [(1.0, 2.0)]);
        let position = analysis.dark_windows[0]
            .position
            .as_deref()
            .expect("a tempo map should yield a position");
        assert!(position.starts_with('@'), "{position}");
    }

    #[test]
    fn a_show_without_tempo_omits_positions() {
        let analysis = analyze(
            r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 2s

    @00:04.000
    wash: static color: "red", duration: 2s
}
"#,
        );
        assert_eq!(analysis.dark_windows.len(), 1);
        assert!(analysis.dark_windows[0].position.is_none());
    }

    #[test]
    fn span_ends_where_activity_ends_not_where_the_source_claims() {
        // The bed is authored for 60s but cut at 5s. Reporting a 60s span
        // would contradict the gap analysis sitting right next to it.
        let analysis = analyze(
            r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 60s

    @00:05.000
    clear(layer: background)

    @00:08.000
    wash: static color: "red", duration: 2s
}
"#,
        );
        assert_eq!(analysis.span, Duration::from_secs(10));
    }

    #[test]
    fn an_empty_show_analyses_to_nothing() {
        let analysis = analyze("show \"T\" {\n}\n");
        assert_eq!(analysis.cue_count, 0);
        assert_eq!(analysis.span, Duration::ZERO);
        assert!(analysis.dark_windows.is_empty());
        assert!(analysis.per_group_seconds.is_empty());
    }
}
