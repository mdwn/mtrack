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
//! The requested times are read off one replay of the cue history through a
//! fresh engine, by the same rules the seek path's start uses: at each time,
//! in order, the cues up to and at it are in, one frame renders and is
//! snapshot as a live engine is. A later time is the earlier one replayed
//! on, which is what a start there replays from the top, so each reads as if
//! evaluated alone: `[10.0, 200.0]` reads as `[200.0, 10.0]` does, and
//! neither perturbs the other.
//! The replay sends the rig the frames playing through would have (see
//! `EffectEngine::trace_wire`), so a channel an effect over before T left set
//! -- a gobo, a zoom -- reads as the rig holds it, as it does live.

use std::collections::HashMap;
use std::time::Duration;

use crate::lighting::effects::{EffectInstance, EffectLayer, FixtureInfo};
use crate::lighting::parser::LightShow;
use crate::lighting::tempo::TempoMap;
use crate::lighting::timeline::LightingTimeline;
use crate::lighting::EffectEngine;
use crate::state::FixtureSnapshot;

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
    /// Where every placed fixture points at this instant — the same
    /// snapshots the live state message carries, so a preview feeds the 3D
    /// scene exactly what the engine would.
    pub poses: Vec<crate::state::PoseSnapshot>,
}

impl Evaluation {
    /// `Some(true)` when no fixture emits light (`channels::is_lit`: its
    /// dimmer times its brightest emitter) — the rig is dark. `None` when
    /// there were no fixtures to evaluate, which is not darkness but an
    /// absence of evidence: no venue is loaded, so nothing can be said
    /// either way. `all()` over an empty set would answer `true`.
    ///
    /// A mover parked at centre reads 128 on pan and tilt, a zoom or focus
    /// holds its setting, and a colour under a closed dimmer emits nothing:
    /// none of that says the rig is lit.
    pub fn is_dark(&self) -> Option<bool> {
        if self.fixtures.is_empty() {
            return None;
        }
        Some(!self.fixtures.iter().any(|f| {
            crate::lighting::channels::is_lit(
                f.channels
                    .iter()
                    .map(|(name, &v)| (name.as_str(), f64::from(v))),
            )
        }))
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
/// `focus_points` are the venue's named stage points, the same map the DMX
/// engine hands its effect engine: without them a `move focus:` cue has
/// nothing to aim at and evaluates as if the point were unbound.
///
/// Returned evaluations are in the order requested, including duplicates.
pub fn evaluate_show<F>(
    shows: Vec<LightShow>,
    fixtures: &[FixtureInfo],
    focus_points: &HashMap<String, [f64; 3]>,
    fallback_tempo_map: Option<&TempoMap>,
    times: &[Duration],
    mut resolve_groups: F,
) -> Vec<Evaluation>
where
    F: FnMut(EffectInstance) -> EffectInstance,
{
    let Some(&last) = times.iter().max() else {
        return Vec::new();
    };
    let mut timeline = LightingTimeline::new(shows);
    let tempo_map = timeline.tempo_map_or(fallback_tempo_map);
    let mut engine = EffectEngine::new();
    engine.set_tempo_map(tempo_map);
    engine.set_focus_points(focus_points.clone());
    for fixture in fixtures {
        engine.register_fixture(fixture.clone());
    }
    // The rig as playing the show through from its start leaves it: what
    // each frame up to each time sent, a gobo an effect over before then
    // left set still set.
    engine.trace_wire();

    // Every cue up to the last time asked for and at it: `start_at` stops
    // short of its time by design, the update fires the cues landing on it.
    let mut history = timeline.start_at(last);
    history.update.steps.extend(timeline.update(last).steps);
    history.map_effects(&mut resolve_groups);

    // One replay, read at each time in order: a later time is the earlier
    // one played on, as a start there would replay it from the top.
    let mut ascending = times.to_vec();
    ascending.sort();
    ascending.dedup();
    let mut sampled: HashMap<Duration, Evaluation> = HashMap::with_capacity(ascending.len());
    // As with a live start, an effect that will not start (a cue targeting
    // a group with no fixtures in this venue) is no reason to abandon it.
    let _ = engine.replay_sampled(history, &ascending, |engine, time| {
        sampled.insert(time, snapshot(engine, time));
    });
    times.iter().map(|time| sampled[time].clone()).collect()
}

/// The shows a song registers: every show its `.light` files hold, in file
/// order and then their order in the file, as the player has them -- the
/// order same-time cues from different shows fire in. Empty when the song
/// has no DSL lighting loaded.
pub fn registered_shows(song: &crate::songs::Song) -> Vec<LightShow> {
    shows_in(song.dsl_lighting_shows())
}

/// [`registered_shows`] of a song's `.light` files, for a song still being
/// built from them.
pub fn shows_in(files: &[crate::songs::DslLightingShow]) -> Vec<LightShow> {
    files
        .iter()
        .flat_map(|dsl| dsl.shows().values().cloned().collect::<Vec<_>>())
        .collect()
}

/// An offline evaluation against the loaded lighting system, plus what the
/// system says about how the shows reach the rig.
pub struct SystemEvaluation {
    /// One per requested time, in the order requested.
    pub evaluations: Vec<Evaluation>,
    /// Fixtures some cue in the shows targets, once groups are resolved.
    pub targeted: std::collections::BTreeSet<String>,
    /// Every fixture the venue places (cell sub-fixtures belong to their
    /// fixture), sorted.
    pub venue_fixtures: Vec<String>,
    /// The groups each effect's cue named, by effect id. An
    /// [`EvaluatedEffect`] only carries what those groups resolved to.
    pub effect_groups: HashMap<String, Vec<String>>,
    /// Why the current venue does not register, when it does not: the whole
    /// venue fails on one fixture that cannot be driven, and the evaluation
    /// then has no rig to drive — which must not read as an empty one.
    pub venue_error: Option<String>,
}

/// The most instants one evaluation may ask for. A scrubber asks for one;
/// anything near this is a script.
pub const MAX_EVALUATE_TIMES: usize = 64;

/// The latest instant one evaluation may ask for: a week. No song is longer,
/// and the evaluator steps through every second it is given.
pub const MAX_EVALUATE_TIME: Duration = Duration::from_secs(7 * 24 * 3600);

/// Why [`evaluate_with_system`] turned a request down.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvaluateError {
    /// More than [`MAX_EVALUATE_TIMES`] instants.
    TooManyTimes,
    /// An instant past [`MAX_EVALUATE_TIME`].
    TimeOutOfRange,
}

impl std::fmt::Display for EvaluateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EvaluateError::TooManyTimes => {
                write!(f, "at most {MAX_EVALUATE_TIMES} times per request")
            }
            EvaluateError::TimeOutOfRange => write!(f, "time out of range"),
        }
    }
}

impl std::error::Error for EvaluateError {}

/// Checks the instants one evaluation asks for against the bounds every
/// surface shares.
fn check_times(times: &[Duration]) -> Result<(), EvaluateError> {
    if times.len() > MAX_EVALUATE_TIMES {
        return Err(EvaluateError::TooManyTimes);
    }
    if times.iter().any(|t| *t > MAX_EVALUATE_TIME) {
        return Err(EvaluateError::TimeOutOfRange);
    }
    Ok(())
}

/// Evaluates `shows` against the venue the lighting system has loaded: its
/// fixtures, focus points and logical groups. With no system (no DMX device)
/// there is no venue, and every fixture list is empty.
///
/// Every group the shows mention is resolved once up front, so the system's
/// lock is held for a short bounded step rather than across evaluation. Both
/// MCP's `evaluate_show` and the web UI's preview call this, so the two
/// resolve groups identically. Blocking: the lock is shared with the effects
/// loop thread.
///
/// Turns down more than [`MAX_EVALUATE_TIMES`] instants, or one past
/// [`MAX_EVALUATE_TIME`], before doing any work.
pub fn evaluate_with_system(
    shows: Vec<LightShow>,
    fallback_tempo_map: Option<&TempoMap>,
    times: &[Duration],
    system: Option<&parking_lot::Mutex<crate::lighting::system::LightingSystem>>,
) -> Result<SystemEvaluation, EvaluateError> {
    check_times(times)?;
    let mut venue_error = None;
    let (fixtures, focus_points, group_map) = match system {
        Some(system) => {
            let mut guard = system.lock();
            let fixtures = match guard.load_current_venue_fixtures() {
                Ok(fixtures) => fixtures,
                // No current venue at all is not an error here: there is
                // simply no rig, as without a system.
                Err(_) if guard.get_current_venue().is_none() => Vec::new(),
                Err(fault) => {
                    venue_error = Some(fault.reason);
                    Vec::new()
                }
            };
            let focus_points = guard.current_focus_points();
            let mut group_map: HashMap<String, Vec<String>> = HashMap::new();
            for name in crate::lighting::readiness::group_names(&shows) {
                let resolved = guard.resolve_logical_group_graceful(&name);
                group_map.insert(name, resolved);
            }
            (fixtures, focus_points, group_map)
        }
        None => (Vec::new(), HashMap::new(), HashMap::new()),
    };

    let targeted = group_map.values().flatten().cloned().collect();
    let mut venue_fixtures: Vec<String> = fixtures
        .iter()
        .filter(|f| f.parent.is_none())
        .map(|f| f.name.clone())
        .collect();
    venue_fixtures.sort();

    let mut effect_groups: HashMap<String, Vec<String>> = HashMap::new();
    let evaluations = evaluate_show(
        shows,
        &fixtures,
        &focus_points,
        fallback_tempo_map,
        times,
        |mut effect| {
            effect_groups.insert(effect.id.clone(), effect.target_fixtures.clone());
            effect.target_fixtures = effect
                .target_fixtures
                .iter()
                .flat_map(|target| match group_map.get(target) {
                    Some(resolved) => resolved.clone(),
                    // Not a known group — already a fixture name, or a name
                    // that resolves to nothing. Either way, leave it for the
                    // engine to accept or reject.
                    None => vec![target.clone()],
                })
                .collect();
            effect
        },
    );
    Ok(SystemEvaluation {
        evaluations,
        targeted,
        venue_fixtures,
        effect_groups,
        venue_error,
    })
}

/// Snapshots a running engine into the same shape [`evaluate_show`] produces.
///
/// This is the live twin of offline evaluation: predicted and actual state come
/// out in one format, so they can be compared directly rather than by eye.
///
/// `at` is the time to label the snapshot with — normally the song's elapsed
/// time. It does not drive the reading; effect elapsed values come from the
/// engine's own state -- the song time it last rendered at, as it renders
/// them -- which is what makes this a measurement rather than a prediction.
pub fn snapshot(engine: &EffectEngine, at: Duration) -> Evaluation {
    let has_dimmer: HashMap<String, bool> = engine
        .get_fixture_registry()
        .iter()
        .map(|(name, info)| (name.clone(), info.has_dimmer()))
        .collect();

    let mut active_effects: Vec<(u64, EvaluatedEffect)> = engine
        .get_active_effects()
        .values()
        .map(|effect| {
            let mut fixtures = effect.target_fixtures.clone();
            fixtures.sort();
            let evaluated = EvaluatedEffect {
                id: effect.id.clone(),
                effect_type: effect.effect_type.name(),
                layer: effect.layer,
                elapsed: engine.effect_elapsed(effect),
                duration: effect.total_duration(),
                fixtures,
            };
            (effect.start_order, evaluated)
        })
        .collect();
    // Ordered by nothing that depends on the id. Ids come from a
    // process-global counter and were compared as strings, so
    // `song_effect_10` sorted before `song_effect_9` and the order shifted as
    // the counter advanced -- two evaluations of the same instant in one call
    // could disagree. Elapsed descending puts the earliest-started effect
    // first; the rest of the key is stable content, then the order the
    // engine started them in, which is how it stacks them (#669).
    active_effects.sort_by(|(order_a, a), (order_b, b)| {
        b.elapsed
            .cmp(&a.elapsed)
            .then_with(|| a.layer.cmp(&b.layer))
            .then_with(|| a.effect_type.cmp(b.effect_type))
            .then_with(|| a.fixtures.cmp(&b.fixtures))
            .then_with(|| order_a.cmp(order_b))
    });
    let active_effects: Vec<EvaluatedEffect> = active_effects
        .into_iter()
        .map(|(_, effect)| effect)
        .collect();

    let registry = engine.get_fixture_registry();
    // What the rig shows: mid-crossfade, the blend going out (#555).
    let mut fixtures = crate::state::fixture_snapshots_with_cells(
        engine.shown_fixture_states(),
        &has_dimmer,
        registry,
        engine.wire(),
        crate::state::StrobeRates::Instant,
    );
    // The engine only holds state for fixtures something is touching, so
    // without this a dark fixture would be missing -- indistinguishable
    // from one not in the venue. "Par2 is dark" and "Par2 does not exist"
    // are different answers when checking a show's coverage.
    crate::state::fill_dark_fixtures(&mut fixtures, registry);

    let poses =
        crate::state::compute_pose_snapshots(&crate::state::shown_poses_on_wire(engine), registry);
    Evaluation {
        time: at,
        fixtures,
        active_effects,
        poses,
    }
}

/// Applies one timeline update to the engine as the live path does (see
/// [`EffectEngine::apply_timeline_update`]), its groups resolved first.
pub(crate) fn apply_timeline_update<F>(
    engine: &mut EffectEngine,
    mut update: crate::lighting::timeline::TimelineUpdate,
    resolve_groups: &mut F,
) where
    F: FnMut(EffectInstance) -> EffectInstance,
{
    update.map_effects(resolve_groups);
    // A cue targeting a group with no fixtures in this venue fails validation.
    // That is a fact about the show worth surfacing elsewhere (#340), not a
    // reason to abandon the sample.
    let _ = engine.apply_timeline_update(update);
}

/// Starts a fresh engine from a seek's history, as the live start paths do
/// (see [`EffectEngine::start_from`]), its groups resolved first: for tests
/// of the live start against an evaluation.
#[cfg(test)]
pub(crate) fn start_from<F>(
    engine: &mut EffectEngine,
    mut history: crate::lighting::timeline::SeekHistory,
    resolve_groups: &mut F,
) where
    F: FnMut(EffectInstance) -> EffectInstance,
{
    history.map_effects(resolve_groups);
    // As above: an effect that will not start is no reason to abandon it.
    let _ = engine.start_from(history, crate::lighting::engine::StopScope::Nothing);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::parser::parse_light_shows;

    use crate::testutil::lighting::rgb as rgb_fixture;

    /// Parses a source into shows, ordered so evaluation is deterministic.
    use crate::testutil::lighting::sorted_shows as shows;

    fn eval(source: &str, fixtures: &[FixtureInfo], times: &[f64]) -> Vec<Evaluation> {
        let times: Vec<Duration> = times.iter().map(|&t| Duration::from_secs_f64(t)).collect();
        evaluate_show(
            shows(source),
            fixtures,
            &HashMap::new(),
            None,
            &times,
            |e| e,
        )
    }

    /// A single-instant evaluation says a software strobe's rate: the bytes
    /// of one instant show only its phase (on or off), and the preview
    /// flashes at the rate. The live stream does not (see `StrobeRates`).
    #[test]
    fn an_evaluation_reports_a_software_strobes_rate() {
        let source = r#"
show "T" {
    @00:00.000
    wash: strobe frequency: 7, duration: 5s
}
"#;
        let results = eval(source, &[rgb_fixture("wash", 1)], &[0.5, 6.0]);
        let rate = |r: &Evaluation| {
            r.fixtures
                .iter()
                .find(|f| f.name == "wash")
                .and_then(|f| f.strobe_hz)
        };
        assert_eq!(rate(&results[0]), Some(7.0));
        assert_eq!(rate(&results[1]), None, "over");
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
        assert!(
            results[2].is_dark() == Some(true),
            "past its duration the effect is gone"
        );
        assert!(results[3].is_dark() == Some(true));

        assert_eq!(results[0].active_effects.len(), 1);
        assert_eq!(results[0].active_effects[0].effect_type, "Static");
        assert_eq!(
            results[0].active_effects[0].duration,
            Duration::from_secs(5)
        );
        assert!(results[2].active_effects.is_empty());
    }

    /// #536: an effect is live right up to its own end. Two back-to-back
    /// 8-bar sequences cued from the show at 180 BPM meet at 10.695666667 s;
    /// sampled a microsecond or less before that, the first is still lit, and
    /// on the downbeat the second is.
    #[test]
    fn back_to_back_sequences_leave_no_dark_instant() {
        let fixtures = vec![rgb_fixture("wash", 1)];
        let source = r#"
tempo {
    start: 29ms
    bpm: 180
    time_signature: 4/4
}
sequence "a" {
    @1/1
    wash: static, color: "red", layer: background, blend_mode: replace, duration: 8measures
}
show "t" {
    @1/1
    sequence "a"
    @9/1
    sequence "a"
}
"#;
        let times = [
            10.695665,
            10.695666,
            10.6956666,
            10.695666666,
            10.695666667,
            10.69566667,
        ];
        let results = eval(source, &fixtures, &times);
        for (t, result) in times.iter().zip(&results) {
            assert_eq!(
                result.is_dark(),
                Some(false),
                "dark at {t}: {:?}",
                result.active_effects
            );
        }
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
    fn evaluate_at_a_slow_down_on_a_looks_last_instant_has_it_over() {
        let fixtures = vec![rgb_fixture("wash", 1)];
        let results = eval(
            "show \"s\" {\n    @00:00.000\n    wash: static, color: \"blue\", duration: 4s\n    \
             @00:04.000\n    master(layer: background, speed: 0.5)\n}\n",
            &fixtures,
            &[4.0],
        );
        assert_eq!(
            results[0].active_effects.len(),
            0,
            "over at its end: the slow-down applies from there"
        );
    }

    #[test]
    fn evaluate_keeps_a_look_frozen_on_its_last_instant_over() {
        let fixtures = vec![rgb_fixture("wash", 1)];
        let results = eval(
            "show \"s\" {\n    @00:00.000\n    wash: static, color: \"blue\", duration: 4s\n    \
             @00:04.000\n    freeze(layer: background)\n    \
             @00:06.000\n    master(layer: background, speed: 0.5)\n}\n",
            &fixtures,
            &[8.0],
        );
        assert_eq!(results[0].active_effects.len(), 0, "live: over at 4 s");
    }

    /// A speed change applies from where a look is: a fade-in slowed half
    /// way carries on from half way, more slowly, with no step back.
    #[test]
    fn a_fade_slowed_part_way_carries_on_from_where_it_was() {
        let fixtures = vec![rgb_fixture("wash", 1)];
        let results = eval(
            "show \"s\" {\n    @00:00.000\n    wash: static, color: \"red\", up_time: 4s, \
             hold_time: 1s\n    @00:02.000\n    master(layer: background, speed: 0.5)\n}\n",
            &fixtures,
            &[1.99, 2.01, 4.0],
        );
        let red: Vec<i32> = results
            .iter()
            .map(|r| i32::from(channel(r, "wash", "red")))
            .collect();
        assert!(
            (red[0] - red[1]).abs() <= 2,
            "no step at the change: {red:?}"
        );
        // 2 s in at 2 s, then 1 s more of fade in the 2 s to 4 s: 3/4 up.
        assert!((red[2] - 191).abs() <= 2, "3/4 of the fade at 4 s: {red:?}");
    }

    #[test]
    fn evaluate_has_a_look_slowed_part_way_play_on_from_where_it_was() {
        // Slowed to half 2 s into a 4 s look: 2 s left at half speed, so
        // it ends at 6 s, 3 s into it at 4 s.
        let fixtures = vec![rgb_fixture("wash", 1)];
        let results = eval(
            "show \"s\" {\n    @00:00.000\n    wash: static, color: \"blue\", duration: 4s\n    \
             @00:02.000\n    master(layer: background, speed: 0.5)\n}\n",
            &fixtures,
            &[4.0, 5.9, 6.1],
        );
        assert_eq!(results[0].active_effects.len(), 1);
        assert_eq!(results[0].active_effects[0].elapsed, Duration::from_secs(3));
        assert_eq!(results[1].active_effects.len(), 1, "still up at 5.9 s");
        assert_eq!(results[2].active_effects.len(), 0, "over at 6 s");
    }

    #[test]
    fn elapsed_counts_a_freeze_and_the_speed_master() {
        // #589: elapsed is how far into itself the effect is, as the engine
        // renders it, not score time since its cue -- a frozen effect is
        // still where it froze, a slowed one half way along.
        let fixtures = vec![rgb_fixture("wash", 1)];
        let frozen = eval(
            "show \"s\" {\n    @00:00.000\n    wash: static, color: \"blue\", duration: 12s\n    \
             @00:08.000\n    freeze(layer: background)\n}\n",
            &fixtures,
            &[20.0],
        );
        assert_eq!(frozen[0].active_effects[0].elapsed, Duration::from_secs(8));
        let slowed = eval(
            "show \"s\" {\n    @00:00.000\n    master(layer: background, speed: 0.5)\n    \
             wash: static, color: \"blue\", duration: 12s\n}\n",
            &fixtures,
            &[6.0],
        );
        assert_eq!(slowed[0].active_effects[0].elapsed, Duration::from_secs(3));
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
        assert!(
            results[0].is_dark() == Some(true),
            "not yet fired just before the cue"
        );
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
            results[1].is_dark() == Some(true),
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

        let unresolved = evaluate_show(
            shows(source),
            &fixtures,
            &HashMap::new(),
            None,
            &times,
            |e| e,
        );
        assert!(
            unresolved[0].is_dark() == Some(true),
            "an unresolved group name matches no fixture"
        );

        let resolved = evaluate_show(
            shows(source),
            &fixtures,
            &HashMap::new(),
            None,
            &times,
            |mut e| {
                if e.target_fixtures == ["all_wash"] {
                    e.target_fixtures = vec!["brick1".to_string(), "brick2".to_string()];
                }
                e
            },
        );
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
        let results = evaluate_show(
            shows(source),
            &fixtures,
            &HashMap::new(),
            Some(&slow),
            &times,
            |e| e,
        );

        assert_eq!(channel(&results[0], "wash", "blue"), 255);
        assert!(
            results[1].is_dark() == Some(true),
            "4 beats at 120bpm ends by 6.0s"
        );
        assert!(
            results[2].is_dark() == Some(true),
            "the 60bpm fallback was not used"
        );
    }

    #[test]
    fn no_fixtures_yields_no_state_but_still_reports_effects() {
        let results = eval(BLUE_5S, &[], &[1.0]);
        assert!(results[0].fixtures.is_empty());
        // Not `Some(true)`: with no venue there is nothing to be dark, and
        // answering "dark" here would be a vacuous truth over an empty set.
        assert_eq!(results[0].is_dark(), None);
        // With nothing to validate against the effect never starts, so an empty
        // venue reports no effects rather than crashing.
        assert!(results[0].active_effects.is_empty());
    }

    #[test]
    fn an_empty_show_is_dark_everywhere() {
        let fixtures = vec![rgb_fixture("wash", 1)];
        let results = eval("show \"T\" {\n}\n", &fixtures, &[0.0, 5.0, 100.0]);
        assert!(results.iter().all(|r| r.is_dark() == Some(true)));
        assert!(results.iter().all(|r| r.active_effects.is_empty()));
    }

    /// Drives the show forward from zero in small steps, the way a playback loop
    /// does, sampling the fixture state at each requested time. This is the
    /// thing `evaluate_show` claims to be a shortcut for.
    /// The live frame period [`play_through`] plays at.
    const FRAME: Duration = Duration::from_millis(5);

    /// The show played through as the live output plays it, a frame every
    /// [`FRAME`], sampled at each of `times` (on that grid). Besides, a
    /// frame lands at each cue's instant before the cue fires: live, the
    /// last frame before a cue is anywhere up to a frame period earlier,
    /// and one landing right on it is the play-through an evaluation reads
    /// (the limit of a fast enough output). A setting or a head moving
    /// when a cue cuts it then holds what both say, exactly.
    fn play_through(
        source: &str,
        fixtures: &[FixtureInfo],
        times: &[Duration],
    ) -> Vec<Vec<FixtureSnapshot>> {
        let mut timeline = LightingTimeline::new(shows(source));
        let tempo_map = timeline.tempo_map().cloned();
        let mut engine = EffectEngine::new();
        engine.set_tempo_map(tempo_map);
        for fixture in fixtures {
            engine.register_fixture(fixture.clone());
        }
        let mut cues: Vec<Duration> = timeline.cue_effects().map(|(time, _)| time).collect();
        cues.sort();
        cues.dedup();
        let mut cues = cues.into_iter().peekable();

        timeline.start();
        let last = times.iter().copied().max().unwrap_or(Duration::ZERO);
        let mut sampled = Vec::new();
        let mut now = Duration::ZERO;
        let mut rendered = Duration::ZERO;

        loop {
            // The frame at each cue up to now, before it fires.
            while let Some(cue) = cues.next_if(|cue| *cue <= now) {
                engine.render_and_send(cue - rendered, cue);
                rendered = cue;
                let update = timeline.update(cue);
                apply_timeline_update(&mut engine, update, &mut |e| e);
            }
            // Out to the rig, as the live output sends it.
            engine.render_and_send(now - rendered, now);
            rendered = now;

            if times.contains(&now) {
                sampled.push((now, snapshot(&engine, now).fixtures));
            }
            if now >= last {
                break;
            }
            now += FRAME;
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
        let reconstructed = evaluate_show(
            shows(source),
            &fixtures,
            &HashMap::new(),
            None,
            &times,
            |e| e,
        );

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

    /// A CMY wash (dimmer, cyan, magenta, yellow, gobo, zoom at 1-6) and
    /// an RGB par (red, green, blue, dimmer, focus, strobe at 7-12).
    fn wire_rig() -> Vec<FixtureInfo> {
        use crate::testutil::lighting::generic;
        vec![
            generic(
                "wash",
                1,
                1,
                &["dimmer", "cyan", "magenta", "yellow", "gobo", "zoom"],
            ),
            generic(
                "par",
                1,
                7,
                &["red", "green", "blue", "dimmer", "focus", "strobe"],
            ),
        ]
    }

    #[test]
    fn a_setting_an_ended_effect_left_reads_as_the_rig_holds_it() {
        // An effect that ends leaves a gobo, a zoom, a focus as it set them:
        // the universe holds a byte until it is sent another. Its levels and
        // colours go to 0 as it ends (#581): the CMY head's magenta and
        // yellow open, so `red: 100%` alone shows white, as it is sent.
        let source = r#"
show "T" {
    @0.000
    wash: static, dimmer: 100%, red: 100%, green: 0%, blue: 0%, gobo: 50%, zoom: 40%, duration: 5s
    par: static, dimmer: 100%, red: 100%, green: 50%, blue: 0%, focus: 30%, strobe: 50%, duration: 5s
    @5.000
    wash: static, dimmer: 100%, red: 100%, duration: 10s
    par: static, dimmer: 100%, red: 100%, duration: 10s
    @8.000
    clear(layer: background)
}
"#;
        let at = |t: f64| -> HashMap<String, HashMap<String, u8>> {
            eval(source, &wire_rig(), &[t])[0]
                .fixtures
                .iter()
                .map(|f| (f.name.clone(), f.channels.clone()))
                .collect()
        };
        let first = at(1.0);
        assert_eq!(
            (
                first["wash"]["magenta"],
                first["wash"]["yellow"],
                first["wash"]["gobo"]
            ),
            (255, 255, 128)
        );
        let later = at(6.0);
        let (wash, par) = (&later["wash"], &later["par"]);
        assert_eq!(
            (
                wash["cyan"],
                wash["magenta"],
                wash["yellow"],
                wash["dimmer"]
            ),
            (0, 0, 0, 255),
            "open white, as the head is sent"
        );
        assert_eq!((wash["gobo"], wash["zoom"]), (128, 102), "held");
        assert!(!wash.contains_key("red"));
        assert_eq!((par["red"], par["green"], par["focus"]), (255, 0, 77));
        assert_eq!(par["strobe"], 0, "a strobe nobody drives is sent off");
        // Cleared: every level dark, the settings still where they were.
        let cleared = at(9.0);
        assert_eq!(cleared["wash"]["dimmer"], 0);
        assert_eq!(
            (cleared["wash"]["gobo"], cleared["par"]["focus"]),
            (128, 77)
        );
    }

    #[test]
    fn a_colour_under_a_dimmer_never_sent_reads_dark() {
        // The head's dimmer channel holds 0 when nothing ever sent it a
        // byte: a colour alone puts no light out, and the state says so
        // rather than taking the dimmer it left out as open.
        let fixtures = wire_rig();
        let source = "show \"T\" {\n    @0.000\n    par: static, red: 100%, duration: 5s\n}\n";
        let evaluation = &eval(source, &fixtures, &[1.0])[0];
        let par = evaluation
            .fixtures
            .iter()
            .find(|f| f.name == "par")
            .unwrap();
        assert_eq!((par.channels["red"], par.channels["dimmer"]), (255, 0));
        assert_eq!(evaluation.is_dark(), Some(true));
    }

    #[test]
    fn made_up_shows_evaluate_as_played_through_on_every_channel() {
        // The evaluation's rig is the one playing the show through leaves:
        // every channel of every fixture, the ones ended effects left set
        // included.
        const MADE_UP: [(&str, &[&str]); 2] = [
            (
                "wash",
                &["dimmer", "red", "green", "blue", "gobo", "zoom", "cyan"],
            ),
            (
                "par",
                &["dimmer", "red", "green", "blue", "focus", "strobe"],
            ),
        ];
        let mut fixtures = wire_rig();
        fixtures.push(crate::testutil::lighting::mover("spot", 13));
        let times: Vec<Duration> = (0..=36).map(|n| Duration::from_millis(n * 250)).collect();
        for seed in 1..=12 {
            let source = crate::testutil::lighting::made_up_show(seed, &MADE_UP, "spot");
            let stepped = play_through(&source, &fixtures, &times);
            let evaluated = evaluate_show(
                shows(&source),
                &fixtures,
                &HashMap::new(),
                None,
                &times,
                |e| e,
            );
            for (i, time) in times.iter().enumerate() {
                let want: Vec<_> = stepped[i].iter().map(|f| (&f.name, &f.channels)).collect();
                let got: Vec<_> = evaluated[i]
                    .fixtures
                    .iter()
                    .map(|f| (&f.name, &f.channels))
                    .collect();
                assert_eq!(want, got, "seed {seed} at {time:?}\n{source}");
            }
            // One time asked alone reads as it does among the others: the
            // replay they share carries nothing over from an earlier time.
            for i in [3, 17, 30] {
                let alone = &evaluate_show(
                    shows(&source),
                    &fixtures,
                    &HashMap::new(),
                    None,
                    &times[i..=i],
                    |e| e,
                )[0];
                let fixtures_of = |e: &Evaluation| -> Vec<_> {
                    e.fixtures
                        .iter()
                        .map(|f| (f.name.clone(), f.channels.clone()))
                        .collect()
                };
                assert_eq!(
                    fixtures_of(alone),
                    fixtures_of(&evaluated[i]),
                    "seed {seed} at {:?} alone\n{source}",
                    times[i]
                );
                let poses = |e: &Evaluation| -> Vec<_> {
                    e.poses
                        .iter()
                        .map(|p| (p.name.clone(), p.pan, p.tilt))
                        .collect()
                };
                assert_eq!(poses(alone), poses(&evaluated[i]));
            }
        }
    }

    #[test]
    fn a_mover_cleared_after_a_move_is_drawn_where_its_bytes_point() {
        // The clear releases pose memory; the head holds the pan and tilt
        // it was last sent, and the pose says so, as live.
        let mut fixtures = wire_rig();
        fixtures.push(crate::testutil::lighting::mover("spot", 13));
        let source = "show \"T\" {\n    @0.000\n    spot: move pan: 90deg, tilt: 45deg, duration: 500ms\n    @1.000\n    clear()\n}\n";
        let evaluation = &eval(source, &fixtures, &[1.5])[0];
        let pose = evaluation.poses.iter().find(|p| p.name == "spot").unwrap();
        assert!(
            (pose.pan - 90.0).abs() < 0.1 && (pose.tilt - 45.0).abs() < 0.1,
            "drawn at {}/{}",
            pose.pan,
            pose.tilt
        );
        // Never moved: at rest.
        let source = "show \"T\" {\n    @0.000\n    par: static, red: 100%, duration: 5s\n}\n";
        let evaluation = &eval(source, &fixtures, &[1.0])[0];
        let pose = evaluation.poses.iter().find(|p| p.name == "spot").unwrap();
        assert_eq!((pose.pan, pose.tilt), (0.0, 0.0));
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
        assert!(results[0].is_dark() == Some(false), "one fixture is lit");
    }

    #[test]
    fn no_times_requested_returns_no_evaluations() {
        let fixtures = vec![rgb_fixture("wash", 1)];
        assert!(eval(BLUE_5S, &fixtures, &[]).is_empty());
    }

    /// Where a head points is not light: a rig whose only non-zero bytes
    /// are pan and tilt is dark.
    #[test]
    fn is_dark_ignores_pan_and_tilt() {
        let mut channels = HashMap::new();
        channels.insert("pan".to_string(), 128u8);
        channels.insert("pan_fine".to_string(), 7u8);
        channels.insert("tilt".to_string(), 90u8);
        channels.insert("dimmer".to_string(), 0u8);
        let evaluation = Evaluation {
            time: Duration::ZERO,
            fixtures: vec![FixtureSnapshot {
                strobe_hz: None,
                name: "m".to_string(),
                channels: channels.clone(),
                cells: Default::default(),
            }],
            active_effects: Vec::new(),
            poses: Vec::new(),
        };
        assert_eq!(evaluation.is_dark(), Some(true));
        channels.insert("dimmer".to_string(), 1u8);
        let lit = Evaluation {
            fixtures: vec![FixtureSnapshot {
                strobe_hz: None,
                name: "m".to_string(),
                channels,
                cells: Default::default(),
            }],
            ..evaluation
        };
        assert_eq!(lit.is_dark(), Some(false));
    }

    /// A colour under a closed dimmer emits no light: the rig is dark.
    #[test]
    fn a_colour_under_a_closed_dimmer_is_dark() {
        let channels: HashMap<String, u8> = [("red", 255u8), ("dimmer", 0)]
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect();
        let evaluation = Evaluation {
            time: Duration::ZERO,
            fixtures: vec![FixtureSnapshot {
                strobe_hz: None,
                name: "m".to_string(),
                channels,
                cells: Default::default(),
            }],
            active_effects: Vec::new(),
            poses: Vec::new(),
        };
        assert_eq!(evaluation.is_dark(), Some(true));
    }

    /// An optic (zoom, focus) or a setting (a gobo) holds its value in the
    /// dark; only a level says the rig is lit, by the rule going dark
    /// fades by.
    #[test]
    fn is_dark_ignores_optics_and_settings() {
        let channels: HashMap<String, u8> = [("zoom", 128u8), ("gobo", 3), ("dimmer", 0)]
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect();
        let evaluation = Evaluation {
            time: Duration::ZERO,
            fixtures: vec![FixtureSnapshot {
                strobe_hz: None,
                name: "m".to_string(),
                channels,
                cells: Default::default(),
            }],
            active_effects: Vec::new(),
            poses: Vec::new(),
        };
        assert_eq!(evaluation.is_dark(), Some(true));
    }

    /// A GDTF profile names a head's pan `pan:1`, a section of the one
    /// attribute: where it points, all the same, and not a lit channel.
    #[test]
    fn is_dark_ignores_a_gdtf_pan_section() {
        let channels: HashMap<String, u8> = [("pan:1", 128u8), ("tilt:1", 90), ("dimmer", 0)]
            .into_iter()
            .map(|(name, value)| (name.to_string(), value))
            .collect();
        let evaluation = Evaluation {
            time: Duration::ZERO,
            fixtures: vec![FixtureSnapshot {
                strobe_hz: None,
                name: "m".to_string(),
                channels,
                cells: Default::default(),
            }],
            active_effects: Vec::new(),
            poses: Vec::new(),
        };
        assert_eq!(evaluation.is_dark(), Some(true));
    }

    /// A fixture at address 1 with `channels`, one byte each, in order.
    fn fixture_of(name: &str, channels: &[&str]) -> FixtureInfo {
        let channels: HashMap<String, u16> = channels
            .iter()
            .zip(1u16..)
            .map(|(n, o)| (n.to_string(), o))
            .collect();
        FixtureInfo::new(name.to_string(), 1, 1, "T".to_string(), channels, None)
    }

    /// Open white on a dimmer and CMY, then each colour param in turn.
    fn open_white_then(colour: &str) -> String {
        format!(
            "show \"T\" {{\n    @00:00.000\n    \
             wash: static dimmer: 100%, cyan: 0%, magenta: 0%, yellow: 0%, duration: 10s\n    \
             @00:01.000\n    wash: static {colour}, duration: 5s\n}}\n"
        )
    }

    /// A CMY fixture driven through red, green and blue is judged by the
    /// flags the head is sent (#661): red at 0 closes cyan, and the open
    /// magenta and yellow still pass green and blue. The snapshot says
    /// what the head is sent: cyan closed, no red of its own.
    #[test]
    fn a_cmy_fixture_driven_through_rgb_is_judged_by_what_it_emits() {
        let fixtures = [fixture_of("wash", &["dimmer", "cyan", "magenta", "yellow"])];
        let results = eval(&open_white_then("red: 0%"), &fixtures, &[0.5, 2.0]);
        assert_eq!(results[0].is_dark(), Some(false), "open white is lit");
        assert_eq!(results[1].is_dark(), Some(false), "blue and green pass");
        assert_eq!(channel(&results[1], "wash", "cyan"), 255);
        assert_eq!(channel(&results[1], "wash", "magenta"), 0);
        assert_eq!(channel(&results[1], "wash", "yellow"), 0);
        let wash = &results[1].fixtures[0];
        assert!(
            ["red", "green", "blue"]
                .iter()
                .all(|c| !wash.channels.contains_key(*c)),
            "a CMY fixture has no red, green or blue to report: {:?}",
            wash.channels
        );

        // All three primaries at 0 close every flag: dark.
        let results = eval(
            &open_white_then("red: 0%, green: 0%, blue: 0%"),
            &fixtures,
            &[2.0],
        );
        assert_eq!(results[0].is_dark(), Some(true));
        // Full red opens cyan and closes nothing.
        let results = eval(&open_white_then("red: 100%"), &fixtures, &[2.0]);
        assert_eq!(results[0].is_dark(), Some(false));
        assert_eq!(channel(&results[0], "wash", "cyan"), 0);
    }

    /// On a CMY fixture with nothing opening its flags first: a flag no
    /// effect drives is open on the wire, so it reads open, and the
    /// fixture is judged subtractively by the flags the head is sent.
    #[test]
    fn a_cmy_fixture_with_flags_left_alone_reads_them_open() {
        let fixtures = [fixture_of("wash", &["dimmer", "cyan", "magenta", "yellow"])];
        let at = |params: &str| {
            let source = format!(
                "show \"T\" {{\n    @00:00.000\n    \
                 wash: static dimmer: 100%, {params}, duration: 10s\n}}\n"
            );
            let result = eval(&source, &fixtures, &[1.0]).remove(0);
            let shown = crate::lighting::channels::shown_color(
                result.fixtures[0]
                    .channels
                    .iter()
                    .map(|(name, &v)| (name.as_str(), f64::from(v))),
            );
            (result, shown)
        };
        let (red, shown) = at("red: 100%");
        assert_eq!(red.is_dark(), Some(false), "red alone opens cyan: white");
        assert_eq!(shown, [1.0, 1.0, 1.0]);
        let (red_green, shown) = at("red: 100%, green: 100%");
        assert_eq!(red_green.is_dark(), Some(false));
        assert_eq!(shown, [1.0, 1.0, 1.0], "white");
        let (half_green, shown) = at("red: 100%, green: 50%");
        assert_eq!(half_green.is_dark(), Some(false));
        assert_eq!(channel(&half_green, "wash", "magenta"), 128);
        assert_eq!(channel(&half_green, "wash", "yellow"), 0, "blue left open");
        assert_eq!(
            shown,
            [1.0, 127.0 / 255.0, 1.0],
            "magenta takes half the green"
        );
        let (cyan, _) = at("cyan: 0%");
        assert_eq!(cyan.is_dark(), Some(false), "cyan open alone is open white");
    }

    /// A CMY fixture driven through its flags directly reads as before.
    #[test]
    fn a_cmy_fixture_driven_through_its_flags_is_unchanged() {
        let fixtures = [fixture_of("wash", &["dimmer", "cyan", "magenta", "yellow"])];
        let closed = eval(
            &open_white_then("cyan: 100%, magenta: 100%, yellow: 100%"),
            &fixtures,
            &[2.0],
        );
        assert_eq!(closed[0].is_dark(), Some(true));
        assert_eq!(channel(&closed[0], "wash", "cyan"), 255);
        let one = eval(&open_white_then("cyan: 100%"), &fixtures, &[2.0]);
        assert_eq!(one[0].is_dark(), Some(false));
        assert_eq!(channel(&one[0], "wash", "cyan"), 255);
        assert_eq!(channel(&one[0], "wash", "magenta"), 0);
    }

    /// An RGB fixture reads its own red, green and blue, as before.
    #[test]
    fn an_rgb_fixture_is_unchanged() {
        let fixtures = [fixture_of("wash", &["dimmer", "red", "green", "blue"])];
        let source = "show \"T\" {\n    @00:00.000\n    \
                      wash: static dimmer: 100%, red: 0%, green: 0%, blue: 100%, duration: 10s\n    \
                      @00:01.000\n    wash: static blue: 0%, duration: 5s\n}\n";
        let results = eval(source, &fixtures, &[0.5, 2.0]);
        assert_eq!(results[0].is_dark(), Some(false));
        assert_eq!(channel(&results[0], "wash", "blue"), 255);
        assert_eq!(results[1].is_dark(), Some(true));
        assert!(results[1].fixtures[0].channels.contains_key("red"));
    }

    /// A fixture with red, green and blue and cyan, magenta and yellow
    /// mixes additively, as the engine drives it: red drives its own red
    /// channel, and cyan, magenta and yellow are emitters of their own
    /// (an RGB+CMY LED wash), not flags. So red at 0 with cyan up is lit,
    /// and the snapshot keeps every channel by its own name.
    #[test]
    fn a_fixture_with_rgb_and_cmy_mixes_additively() {
        let fixtures = [fixture_of(
            "wash",
            &[
                "dimmer", "red", "green", "blue", "cyan", "magenta", "yellow",
            ],
        )];
        let source = "show \"T\" {\n    @00:00.000\n    \
                      wash: static dimmer: 100%, red: 0%, green: 0%, blue: 0%, \
                      cyan: 0%, magenta: 0%, yellow: 0%, duration: 10s\n    \
                      @00:01.000\n    wash: static cyan: 100%, duration: 5s\n}\n";
        let results = eval(source, &fixtures, &[0.5, 2.0]);
        assert_eq!(results[0].is_dark(), Some(true), "every emitter at 0");
        assert_eq!(results[1].is_dark(), Some(false), "cyan is an emitter");
        assert_eq!(channel(&results[1], "wash", "cyan"), 255);
        assert_eq!(channel(&results[1], "wash", "red"), 0);
        assert!(results[1].fixtures[0].channels.contains_key("red"));
    }
}

#[cfg(test)]
#[path = "move_seek_tests.rs"]
mod move_seek_tests;

#[cfg(test)]
#[path = "show_end_tests.rs"]
mod show_end_tests;

#[cfg(test)]
#[path = "sweep_fuzz_tests.rs"]
mod sweep_fuzz_tests;
