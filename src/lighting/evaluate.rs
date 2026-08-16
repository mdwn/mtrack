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

//! Offline evaluation of a light show.
//!
//! Answers "what would the fixtures be doing at time T" without audio, DMX
//! output, or wall-clock waiting. Verifying a show otherwise means playing the
//! song in real time through the PA and the rig, once per check.
//!
//! Each requested time is evaluated independently against a fresh engine, by
//! the same reconstruction the seek path uses: replay the cue history up to T,
//! start each surviving effect at its elapsed offset, then render one frame.
//! Nothing accumulates between samples, so evaluating `[10.0, 200.0]` costs the
//! same as `[200.0, 10.0]` and neither perturbs the other.

use std::collections::HashMap;
use std::time::Duration;

use crate::lighting::effects::{is_multiplier_channel, EffectInstance, EffectLayer, FixtureInfo};
use crate::lighting::parser::LightShow;
use crate::lighting::tempo::TempoMap;
use crate::lighting::timeline::LightingTimeline;
use crate::lighting::EffectEngine;
use crate::state::{compute_fixture_snapshots, FixtureSnapshot};

/// An effect running at the evaluated instant.
#[derive(Clone, Debug)]
pub struct EvaluatedEffect {
    pub id: String,
    /// Effect kind, e.g. `Static` or `Strobe`.
    pub effect_type: &'static str,
    pub layer: EffectLayer,
    /// How far into the effect this instant is.
    pub elapsed: Duration,
    /// The effect's total authored duration (up + hold + down).
    pub duration: Duration,
    /// Fixtures the effect resolved to, sorted.
    pub fixtures: Vec<String>,
}

/// Rendered show state at one instant.
#[derive(Clone, Debug)]
pub struct Evaluation {
    pub time: Duration,
    /// Per-fixture DMX channel values (0–255), sorted by fixture name. Every
    /// known fixture appears, including ones no effect is touching, so a dark
    /// fixture is reported as zeros rather than by being absent.
    pub fixtures: Vec<FixtureSnapshot>,
    /// Effects running at this instant, sorted by id.
    pub active_effects: Vec<EvaluatedEffect>,
}

impl Evaluation {
    /// True when every channel of every fixture is at zero — the rig is dark.
    pub fn is_dark(&self) -> bool {
        self.fixtures
            .iter()
            .all(|f| f.channels.values().all(|&v| v == 0))
    }
}

/// Evaluates `shows` at each requested time.
///
/// `resolve_groups` maps an effect's group targets onto concrete fixture names
/// — the same step the live path performs via the lighting system. Passing a
/// pass-through closure evaluates against literal fixture names.
///
/// `fallback_tempo_map` is used only when no show carries its own `tempo` block,
/// mirroring the live precedence in `dmx::engine::playback` (show tempo > song
/// tempo). Note that today it can never fire: the parser rejects every musical
/// construct — `@bar/beat` cues, `Nbeats` durations, `Nbeat` frequencies — when
/// the `.light` file has no `tempo` block of its own, so a show that would need
/// the fallback fails before it can be evaluated (see #337). The parameter is
/// here so this evaluator keeps matching the live path when that changes.
///
/// Returned evaluations are in the order requested, including duplicates.
pub fn evaluate_show<F>(
    shows: Vec<LightShow>,
    fixtures: &[FixtureInfo],
    fallback_tempo_map: Option<&TempoMap>,
    times: &[Duration],
    mut resolve_groups: F,
) -> Vec<Evaluation>
where
    F: FnMut(EffectInstance) -> EffectInstance,
{
    let mut timeline = LightingTimeline::new(shows);
    let tempo_map = timeline
        .tempo_map()
        .cloned()
        .or_else(|| fallback_tempo_map.cloned());

    // Which fixtures own a dedicated dimmer channel decides whether their RGB
    // values get scaled by the virtual dimmer. Same input the live sampler uses.
    let has_dimmer: HashMap<String, bool> = fixtures
        .iter()
        .map(|f| (f.name.clone(), f.channels.contains_key("dimmer")))
        .collect();

    // Held under a second name because `fixtures` is shadowed inside the closure.
    let venue_fixtures = fixtures;
    times
        .iter()
        .map(|&time| {
            let mut engine = EffectEngine::new();
            engine.set_tempo_map(tempo_map.clone());
            for fixture in fixtures {
                engine.register_fixture(fixture.clone());
            }

            // Replay everything before `time`, then fire the cues landing exactly
            // on it. `start_at` stops short of `time` by design, so both halves
            // are needed for a cue at the requested instant to count.
            let history = timeline.start_at(time);
            apply_timeline_update(&mut engine, history, &mut resolve_groups);
            let now = timeline.update(time);
            apply_timeline_update(&mut engine, now, &mut resolve_groups);

            // Render a single frame at `time`. A zero delta keeps the engine's
            // clock exactly where the elapsed offsets were computed against.
            let _ = engine.update(Duration::ZERO, Some(time));

            let mut fixtures = compute_fixture_snapshots(&engine.get_fixture_states(), &has_dimmer);
            fill_dark_fixtures(&mut fixtures, venue_fixtures.iter());
            let mut active_effects: Vec<EvaluatedEffect> = engine
                .get_active_effects()
                .values()
                .map(|effect| {
                    let mut fixtures = effect.target_fixtures.clone();
                    fixtures.sort();
                    EvaluatedEffect {
                        id: effect.id.clone(),
                        effect_type: effect.effect_type.name(),
                        layer: effect.layer,
                        elapsed: effect
                            .cue_time
                            .map(|cue| time.saturating_sub(cue))
                            .unwrap_or(Duration::ZERO),
                        duration: effect.total_duration(),
                        fixtures,
                    }
                })
                .collect();
            active_effects.sort_by(|a, b| a.id.cmp(&b.id));

            Evaluation {
                time,
                fixtures,
                active_effects,
            }
        })
        .collect()
}

/// Adds every known fixture that no effect is driving, with all its channels at
/// zero.
///
/// The engine only holds state for fixtures something is currently touching, so
/// without this a dark fixture is simply missing — indistinguishable from one
/// that is not in the venue at all. "Par2 is dark" and "Par2 does not exist"
/// are very different answers when you are checking a show's coverage.
fn fill_dark_fixtures<'a>(
    snapshots: &mut Vec<FixtureSnapshot>,
    known: impl Iterator<Item = &'a FixtureInfo>,
) {
    for info in known {
        if snapshots.iter().any(|s| s.name == info.name) {
            continue;
        }
        snapshots.push(FixtureSnapshot {
            name: info.name.clone(),
            channels: info
                .channels
                .keys()
                .filter(|name| !is_multiplier_channel(name))
                .map(|name| (name.clone(), 0u8))
                .collect(),
        });
    }
    snapshots.sort_by(|a, b| a.name.cmp(&b.name));
}

/// Snapshots a running engine into the same shape [`evaluate_show`] produces.
///
/// This is the live twin of offline evaluation: predicted and actual state come
/// out in one format, so they can be compared directly rather than by eye.
///
/// `at` is the time to label the snapshot with — normally the song's elapsed
/// time. It does not drive the reading; effect elapsed values come from the
/// engine's own clock, which is what makes this a measurement rather than a
/// prediction.
pub fn snapshot(engine: &EffectEngine, at: Duration) -> Evaluation {
    let has_dimmer: HashMap<String, bool> = engine
        .get_fixture_registry()
        .iter()
        .map(|(name, info)| (name.clone(), info.channels.contains_key("dimmer")))
        .collect();

    let mut active_effects: Vec<EvaluatedEffect> = engine
        .get_active_effects()
        .values()
        .map(|effect| {
            let mut fixtures = effect.target_fixtures.clone();
            fixtures.sort();
            EvaluatedEffect {
                id: effect.id.clone(),
                effect_type: effect.effect_type.name(),
                layer: effect.layer,
                elapsed: engine.effect_elapsed(effect),
                duration: effect.total_duration(),
                fixtures,
            }
        })
        .collect();
    active_effects.sort_by(|a, b| a.id.cmp(&b.id));

    let mut fixtures = compute_fixture_snapshots(&engine.get_fixture_states(), &has_dimmer);
    fill_dark_fixtures(&mut fixtures, engine.get_fixture_registry().values());

    Evaluation {
        time: at,
        fixtures,
        active_effects,
    }
}

/// Applies one timeline update to the engine in the same order the live path
/// uses: layer commands, then stopped sequences, then seek-started effects in
/// cue order, then freshly-fired effects with sequence effects first.
pub(crate) fn apply_timeline_update<F>(
    engine: &mut EffectEngine,
    update: crate::lighting::timeline::TimelineUpdate,
    resolve_groups: &mut F,
) where
    F: FnMut(EffectInstance) -> EffectInstance,
{
    for cmd in &update.layer_commands {
        engine.apply_layer_command(cmd);
    }

    for sequence_name in &update.stop_sequences {
        engine.stop_sequence(sequence_name);
    }

    let mut with_elapsed: Vec<_> = update.effects_with_elapsed.into_values().collect();
    with_elapsed.sort_by_key(|(effect, _)| effect.cue_time.unwrap_or(Duration::ZERO));
    for (effect, elapsed) in with_elapsed {
        let resolved = resolve_groups(effect);
        // A cue targeting a group with no fixtures in this venue fails validation.
        // That is a fact about the show worth surfacing elsewhere (#340), not a
        // reason to abandon the sample.
        let _ = engine.start_effect_with_elapsed(resolved, elapsed);
    }

    let mut effects = update.effects;
    effects.sort_by_key(|e| if e.id.starts_with("seq_") { 0 } else { 1 });
    for effect in effects {
        let resolved = resolve_groups(effect);
        let _ = engine.start_effect(resolved);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::parser::parse_light_shows;

    fn rgb_fixture(name: &str, address: u16) -> FixtureInfo {
        let mut channels = HashMap::new();
        channels.insert("red".to_string(), 1);
        channels.insert("green".to_string(), 2);
        channels.insert("blue".to_string(), 3);
        FixtureInfo::new(
            name.to_string(),
            1,
            address,
            "RGB".to_string(),
            channels,
            None,
        )
    }

    /// Parses a source into shows, ordered so evaluation is deterministic.
    fn shows(source: &str) -> Vec<LightShow> {
        let mut parsed: Vec<_> = parse_light_shows(source)
            .expect("test show should parse")
            .into_values()
            .collect();
        parsed.sort_by(|a, b| a.name.cmp(&b.name));
        parsed
    }

    fn eval(source: &str, fixtures: &[FixtureInfo], times: &[f64]) -> Vec<Evaluation> {
        let times: Vec<Duration> = times.iter().map(|&t| Duration::from_secs_f64(t)).collect();
        evaluate_show(shows(source), fixtures, None, &times, |e| e)
    }

    fn channel(evaluation: &Evaluation, fixture: &str, channel: &str) -> u8 {
        evaluation
            .fixtures
            .iter()
            .find(|f| f.name == fixture)
            .and_then(|f| f.channels.get(channel).copied())
            .unwrap_or(0)
    }

    const BLUE_5S: &str = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 5s
}
"#;

    #[test]
    fn effect_is_live_inside_its_duration_and_gone_after() {
        let fixtures = vec![rgb_fixture("wash", 1)];
        let results = eval(BLUE_5S, &fixtures, &[1.0, 4.9, 5.1, 30.0]);

        assert_eq!(channel(&results[0], "wash", "blue"), 255);
        assert_eq!(channel(&results[1], "wash", "blue"), 255);
        assert!(results[2].is_dark(), "past its duration the effect is gone");
        assert!(results[3].is_dark());

        assert_eq!(results[0].active_effects.len(), 1);
        assert_eq!(results[0].active_effects[0].effect_type, "Static");
        assert_eq!(
            results[0].active_effects[0].duration,
            Duration::from_secs(5)
        );
        assert!(results[2].active_effects.is_empty());
    }

    #[test]
    fn elapsed_is_measured_from_the_cue() {
        let fixtures = vec![rgb_fixture("wash", 1)];
        let results = eval(BLUE_5S, &fixtures, &[0.0, 2.0, 4.0]);
        assert_eq!(results[0].active_effects[0].elapsed, Duration::ZERO);
        assert_eq!(results[1].active_effects[0].elapsed, Duration::from_secs(2));
        assert_eq!(results[2].active_effects[0].elapsed, Duration::from_secs(4));
    }

    #[test]
    fn a_cue_landing_exactly_on_the_requested_time_counts() {
        // `start_at` deliberately stops short of the requested instant, so a cue
        // at exactly T only appears if the evaluator also pumps the timeline.
        let fixtures = vec![rgb_fixture("wash", 1)];
        let source = r#"
show "T" {
    @00:10.000
    wash: static color: "red", duration: 5s
}
"#;
        let results = eval(source, &fixtures, &[9.999, 10.0]);
        assert!(results[0].is_dark(), "not yet fired just before the cue");
        assert_eq!(channel(&results[1], "wash", "red"), 255);
    }

    #[test]
    fn times_are_independent_of_request_order() {
        let fixtures = vec![rgb_fixture("wash", 1)];
        let source = r#"
show "T" {
    @00:00.000
    wash: static color: "red", duration: 4s

    @00:10.000
    wash: static color: "blue", duration: 4s
}
"#;
        let ascending = eval(source, &fixtures, &[1.0, 11.0]);
        let descending = eval(source, &fixtures, &[11.0, 1.0]);

        assert_eq!(channel(&ascending[0], "wash", "red"), 255);
        assert_eq!(channel(&ascending[1], "wash", "blue"), 255);
        // Same instants, reversed request order, same answers.
        assert_eq!(channel(&descending[1], "wash", "red"), 255);
        assert_eq!(channel(&descending[0], "wash", "blue"), 255);
    }

    #[test]
    fn repeated_times_evaluate_identically() {
        let fixtures = vec![rgb_fixture("wash", 1)];
        let results = eval(BLUE_5S, &fixtures, &[2.0, 2.0, 2.0]);
        assert_eq!(results.len(), 3);
        assert_eq!(channel(&results[0], "wash", "blue"), 255);
        assert_eq!(channel(&results[1], "wash", "blue"), 255);
        assert_eq!(channel(&results[2], "wash", "blue"), 255);
    }

    #[test]
    fn a_clear_in_the_history_kills_the_effect() {
        let fixtures = vec![rgb_fixture("wash", 1)];
        let source = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 60s

    @00:05.000
    clear(layer: background)
}
"#;
        let results = eval(source, &fixtures, &[4.0, 6.0]);
        assert_eq!(channel(&results[0], "wash", "blue"), 255);
        assert!(
            results[1].is_dark(),
            "the clear at 5s should have killed a 60s effect"
        );
    }

    #[test]
    fn group_resolution_closure_maps_targets_to_fixtures() {
        // The show targets `all_wash`, which is not a fixture name. Without
        // resolution the effect fails validation and nothing lights.
        let fixtures = vec![rgb_fixture("brick1", 1), rgb_fixture("brick2", 4)];
        let source = r#"
show "T" {
    @00:00.000
    all_wash: static color: "green", duration: 5s
}
"#;
        let times = [Duration::from_secs(1)];

        let unresolved = evaluate_show(shows(source), &fixtures, None, &times, |e| e);
        assert!(
            unresolved[0].is_dark(),
            "an unresolved group name matches no fixture"
        );

        let resolved = evaluate_show(shows(source), &fixtures, None, &times, |mut e| {
            if e.target_fixtures == ["all_wash"] {
                e.target_fixtures = vec!["brick1".to_string(), "brick2".to_string()];
            }
            e
        });
        assert_eq!(channel(&resolved[0], "brick1", "green"), 255);
        assert_eq!(channel(&resolved[0], "brick2", "green"), 255);
        assert_eq!(resolved[0].active_effects[0].fixtures, ["brick1", "brick2"]);
    }

    #[test]
    fn musical_timing_requires_the_shows_own_tempo_block() {
        // Pins today's behaviour, and explains why `fallback_tempo_map` cannot
        // rescue a bar/beat show: the parser rejects musical timing outright
        // when the `.light` file carries no `tempo` block, long before any
        // fallback map is consulted. #337 is the issue that would change this.
        let bar_beat_cue = r#"
show "T" {
    @3/1
    wash: static color: "blue", duration: 5s
}
"#;
        let beat_duration = r#"
show "T" {
    @00:00.000
    wash: static color: "blue", duration: 4beats
}
"#;
        assert!(parse_light_shows(bar_beat_cue).is_err());
        assert!(parse_light_shows(beat_duration).is_err());
    }

    #[test]
    fn the_shows_own_tempo_wins_over_the_fallback() {
        let fixtures = vec![rgb_fixture("wash", 1)];
        let source = r#"
tempo {
    start: 0ms
    bpm: 120
    time_signature: 4/4
}

show "T" {
    @3/1
    wash: static color: "blue", duration: 4beats
}
"#;
        // Half the show's tempo. If the fallback were winning, bar 3 beat 1
        // would land at 8.0s instead of 4.0s and 4 beats would last 4s not 2s.
        let slow = TempoMap::new(
            Duration::ZERO,
            60.0,
            crate::tempo::TimeSignature::new(4, 4),
            vec![],
        );
        let times = [
            Duration::from_secs_f64(4.5),
            Duration::from_secs_f64(6.5),
            Duration::from_secs_f64(8.5),
        ];
        let results = evaluate_show(shows(source), &fixtures, Some(&slow), &times, |e| e);

        assert_eq!(channel(&results[0], "wash", "blue"), 255);
        assert!(results[1].is_dark(), "4 beats at 120bpm ends by 6.0s");
        assert!(results[2].is_dark(), "the 60bpm fallback was not used");
    }

    #[test]
    fn no_fixtures_yields_no_state_but_still_reports_effects() {
        let results = eval(BLUE_5S, &[], &[1.0]);
        assert!(results[0].fixtures.is_empty());
        assert!(results[0].is_dark());
        // With nothing to validate against the effect never starts, so an empty
        // venue reads as a dark rig rather than a crash.
        assert!(results[0].active_effects.is_empty());
    }

    #[test]
    fn an_empty_show_is_dark_everywhere() {
        let fixtures = vec![rgb_fixture("wash", 1)];
        let results = eval("show \"T\" {\n}\n", &fixtures, &[0.0, 5.0, 100.0]);
        assert!(results.iter().all(|r| r.is_dark()));
        assert!(results.iter().all(|r| r.active_effects.is_empty()));
    }

    /// Drives the show forward from zero in small steps, the way a playback loop
    /// does, sampling the fixture state at each requested time. This is the
    /// thing `evaluate_show` claims to be a shortcut for.
    fn play_through(
        source: &str,
        fixtures: &[FixtureInfo],
        times: &[Duration],
    ) -> Vec<Vec<FixtureSnapshot>> {
        let step = Duration::from_millis(5);
        let mut timeline = LightingTimeline::new(shows(source));
        let tempo_map = timeline.tempo_map().cloned();
        let mut engine = EffectEngine::new();
        engine.set_tempo_map(tempo_map);
        for fixture in fixtures {
            engine.register_fixture(fixture.clone());
        }
        let has_dimmer: HashMap<String, bool> = fixtures
            .iter()
            .map(|f| (f.name.clone(), f.channels.contains_key("dimmer")))
            .collect();

        timeline.start();
        let last = times.iter().copied().max().unwrap_or(Duration::ZERO);
        let mut sampled = Vec::new();
        let mut now = Duration::ZERO;

        loop {
            let update = timeline.update(now);
            apply_timeline_update(&mut engine, update, &mut |e| e);
            let _ = engine.update(step, Some(now));

            if times.contains(&now) {
                let mut snap = compute_fixture_snapshots(&engine.get_fixture_states(), &has_dimmer);
                fill_dark_fixtures(&mut snap, fixtures.iter());
                sampled.push((now, snap));
            }
            if now >= last {
                break;
            }
            now += step;
        }

        times
            .iter()
            .map(|t| {
                sampled
                    .iter()
                    .find(|(at, _)| at == t)
                    .map(|(_, snap)| snap.clone())
                    .unwrap_or_default()
            })
            .collect()
    }

    #[test]
    fn reconstruction_agrees_with_playing_the_show_through() {
        // If these ever disagree, offline evaluation is lying about the rig.
        let fixtures = vec![rgb_fixture("brick1", 1), rgb_fixture("brick2", 4)];
        let source = r#"
show "T" {
    @00:00.000
    brick1: static color: "red", duration: 8s
    brick2: static color: "blue", duration: 3s

    @00:03.000
    brick2: static color: "green", duration: 2s

    @00:06.000
    clear(layer: background)

    @00:07.000
    brick1: static color: "white", duration: 4s
    brick2: static color: "white", duration: 4s
}
"#;
        let times: Vec<Duration> = [
            0.0, 0.5, 2.5, 3.0, 3.5, 4.5, 5.0, 5.5, 6.0, 6.5, 7.0, 8.0, 10.0, 11.0, 12.0,
        ]
        .iter()
        .map(|&t| Duration::from_secs_f64(t))
        .collect();

        let stepped = play_through(source, &fixtures, &times);
        let reconstructed = evaluate_show(shows(source), &fixtures, None, &times, |e| e);

        for (i, time) in times.iter().enumerate() {
            let expected = &stepped[i];
            let actual = &reconstructed[i].fixtures;
            assert_eq!(
                expected.len(),
                actual.len(),
                "fixture count differs at {:?}",
                time
            );
            for (want, got) in expected.iter().zip(actual.iter()) {
                assert_eq!(want.name, got.name, "fixture order differs at {:?}", time);
                assert_eq!(
                    want.channels, got.channels,
                    "at {:?}, fixture `{}`: played-through {:?} but evaluated {:?}",
                    time, want.name, want.channels, got.channels
                );
            }
        }
    }

    #[test]
    fn untouched_fixtures_are_reported_dark_not_omitted() {
        // "Par2 is dark" and "Par2 is not in this venue" are different answers.
        // The engine only holds state for fixtures an effect is driving, so
        // without zero-filling the second fixture would simply be missing.
        let fixtures = vec![rgb_fixture("lit", 1), rgb_fixture("unlit", 4)];
        let source = r#"
show "T" {
    @00:00.000
    lit: static color: "blue", duration: 5s
}
"#;
        let results = eval(source, &fixtures, &[1.0]);

        let names: Vec<&str> = results[0]
            .fixtures
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(names, ["lit", "unlit"], "both fixtures should be reported");
        assert_eq!(channel(&results[0], "lit", "blue"), 255);
        assert_eq!(channel(&results[0], "unlit", "blue"), 0);

        let unlit = results[0]
            .fixtures
            .iter()
            .find(|f| f.name == "unlit")
            .expect("unlit reported");
        assert!(
            !unlit.channels.is_empty(),
            "a dark fixture still names its channels"
        );
        assert!(unlit.channels.values().all(|&v| v == 0));
        assert!(!results[0].is_dark(), "one fixture is lit");
    }

    #[test]
    fn no_times_requested_returns_no_evaluations() {
        let fixtures = vec![rgb_fixture("wash", 1)];
        assert!(eval(BLUE_5S, &fixtures, &[]).is_empty());
    }
}
