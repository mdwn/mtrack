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

use crate::lighting::effects::EffectLayer;
use crate::lighting::{
    parser::{Cue, Effect, LayerCommand, LayerCommandType, LightShow},
    EffectInstance,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Global atomic counter for generating unique, deterministic effect IDs.
static EFFECT_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Result of processing timeline cues
#[derive(Debug, Clone, Default)]
pub struct TimelineUpdate {
    /// Effects to be triggered
    pub effects: Vec<EffectInstance>,
    /// Effects to be triggered with pre-calculated elapsed time (for seeking)
    /// Maps effect ID to (effect, elapsed_time)
    pub effects_with_elapsed: HashMap<String, (EffectInstance, Duration)>,
    /// Layer commands to be executed
    pub layer_commands: Vec<LayerCommand>,
    /// The song time of each layer command's cue, for cues fired as the
    /// song plays: a freeze holds, and an unfreeze resumes, from its cue's
    /// own time, not the frame it lands in. Empty for a seek's history,
    /// which holds a freeze at the start point.
    pub layer_command_times: Vec<Duration>,
    /// Names of sequences to stop
    pub stop_sequences: Vec<String>,
}

/// A layer's freeze and speed master as a seek replays its history.
#[derive(Clone)]
struct ReplayLayer {
    /// When the layer froze, while it is frozen.
    frozen_since: Option<Duration>,
    /// The speed master, 1.0 by default.
    speed: f64,
    /// When the last freeze, unfreeze or master landed on the layer: its
    /// effects have run at one speed since.
    since: Duration,
}

impl Default for ReplayLayer {
    fn default() -> Self {
        Self {
            frozen_since: None,
            speed: 1.0,
            since: Duration::ZERO,
        }
    }
}

/// An effect's elapsed at `time` before its layer's speed master: its score
/// time less its time frozen, held where its layer froze while frozen.
fn base_elapsed(
    effect: &EffectInstance,
    layers: &HashMap<EffectLayer, ReplayLayer>,
    time: Duration,
) -> Duration {
    let now = layers
        .get(&effect.layer)
        .and_then(|layer| layer.frozen_since)
        .unwrap_or(time);
    now.saturating_sub(effect.cue_time.unwrap_or(Duration::ZERO))
        .saturating_sub(effect.score_frozen)
}

/// An effect's own elapsed at `time`, as the engine renders and expires it:
/// [`base_elapsed`] scaled by its layer's speed master (0 freezes, and is
/// not a scale).
fn own_elapsed(
    effect: &EffectInstance,
    layers: &HashMap<EffectLayer, ReplayLayer>,
    time: Duration,
) -> Duration {
    let speed = layers.get(&effect.layer).map_or(1.0, |layer| layer.speed);
    crate::lighting::engine::scale_by_speed(base_elapsed(effect, layers, time), speed)
}

/// Whether an effect is a move: one that leaves the head where it ends.
fn is_move(effect: &EffectInstance) -> bool {
    matches!(
        effect.effect_type,
        crate::lighting::effects::EffectType::Move { .. }
    )
}

/// Whether a layer command changes how its layer's effects run in time: a
/// freeze, an unfreeze, or a master with a speed (an intensity alone does
/// not).
fn times_layer(cmd: &LayerCommand) -> bool {
    match cmd.command_type {
        LayerCommandType::Freeze | LayerCommandType::Unfreeze => true,
        LayerCommandType::Master => cmd.speed.is_some(),
        LayerCommandType::Clear => false,
    }
}

/// What the replay has taken out of its effects as they ran their course.
#[derive(Default)]
struct Settled {
    /// Moves that have arrived: they stay for their pose.
    arrived: HashSet<String>,
    /// When the latest effect taken out ended.
    ended: Duration,
}

/// Takes the effects on `layer` that have run their course by `time` out
/// of the replay -- but for moves, which stay for their pose, marked as
/// arrived -- and moves `settled.ended` on to when the latest of them ended:
/// its layer has held its freeze and speed since its last command, so that
/// is how far back past its end its elapsed is. With `keep_ending`, one
/// ending exactly at `time` stays: live, a cue fires before the frame it
/// lands in expires anything, so a freeze or speed change on the very
/// instant an effect ends still catches it.
fn settle_layer(
    layer: EffectLayer,
    time: Duration,
    layers: &HashMap<EffectLayer, ReplayLayer>,
    effects: &mut HashMap<String, (EffectInstance, Duration)>,
    settled: &mut Settled,
    keep_ending: bool,
) {
    let state = layers.get(&layer);
    let now = state.and_then(|layer| layer.frozen_since).unwrap_or(time);
    let speed = state.map_or(1.0, |layer| layer.speed);
    let since = state.map_or(Duration::ZERO, |layer| layer.since);
    effects.retain(|id, (effect, _)| {
        // An arrived move ended when it arrived.
        if effect.layer != layer || settled.arrived.contains(id) {
            return true;
        }
        let elapsed = own_elapsed(effect, layers, time);
        let running = if keep_ending {
            elapsed <= effect.total_duration()
        } else {
            elapsed < effect.total_duration()
        };
        if running {
            return true;
        }
        let past = elapsed.saturating_sub(effect.total_duration());
        let past = if crate::lighting::engine::speed_scales(speed) {
            Duration::try_from_secs_f64(past.as_secs_f64() / speed).unwrap_or(now)
        } else {
            past
        };
        // Not before it started, nor before the change it ran under.
        let floor = since.max(effect.cue_time.unwrap_or(Duration::ZERO));
        settled.ended = settled.ended.max(now.saturating_sub(past).max(floor));
        if is_move(effect) {
            settled.arrived.insert(id.clone());
        }
        is_move(effect)
    });
}

/// Moves a layer's freeze and speed on by a freeze, unfreeze or master at
/// `time`, as the engine does live, the effects on it settled first: at the
/// speed or freeze they ran under until then.
fn replay_layer_command(
    cmd: &crate::lighting::parser::LayerCommand,
    layer: EffectLayer,
    time: Duration,
    layers: &mut HashMap<EffectLayer, ReplayLayer>,
    effects: &mut HashMap<String, (EffectInstance, Duration)>,
    settled: &mut Settled,
) {
    settle_layer(layer, time, layers, effects, settled, true);
    layers.entry(layer).or_default().since = time;
    let freeze = |layers: &mut HashMap<EffectLayer, ReplayLayer>| {
        let state = layers.entry(layer).or_default();
        state.frozen_since.get_or_insert(time);
    };
    let unfreeze = |layers: &mut HashMap<EffectLayer, ReplayLayer>,
                    effects: &mut HashMap<String, (EffectInstance, Duration)>| {
        let Some(frozen_at) = layers.get_mut(&layer).and_then(|l| l.frozen_since.take()) else {
            return;
        };
        // From the freeze, or from its own cue for one cued during it.
        for (effect, _) in effects.values_mut() {
            if effect.layer == layer {
                let cue_time = effect.cue_time.unwrap_or(Duration::ZERO);
                effect.score_frozen += time.saturating_sub(cue_time.max(frozen_at));
            }
        }
    };
    match cmd.command_type {
        LayerCommandType::Freeze => freeze(layers),
        LayerCommandType::Unfreeze => unfreeze(layers, effects),
        LayerCommandType::Master => {
            if let Some(speed) = cmd.speed {
                let speed = speed.max(0.0);
                if speed == 0.0 {
                    freeze(layers);
                } else if layers.get(&layer).is_some_and(|l| l.speed == 0.0) {
                    unfreeze(layers, effects);
                }
                layers.entry(layer).or_default().speed = speed;
            }
        }
        LayerCommandType::Clear => {}
    }
}

/// Where a run through some cues leaves things, as the engine would have them
/// live: the effects started and not yet cleared or stopped (their elapsed
/// still to be worked out), the layers' freezes and speeds, the moves that
/// have arrived, and the sequences stopped.
#[derive(Default)]
struct Replay {
    result: TimelineUpdate,
    layers: HashMap<EffectLayer, ReplayLayer>,
    settled: Settled,
    stopped_sequences: HashSet<String>,
}

/// Marks the moves `which` picks that end by `time` on the layers as they
/// stood before the cues there (`before`) as arrived: live the head was at
/// their destination by the frame before, and a clear or stop in those cues
/// leaves it there, even after a speed change in them put the move back on
/// its way.
fn arrive_moves_ending_at(
    time: Duration,
    before: &HashMap<EffectLayer, ReplayLayer>,
    effects: &HashMap<String, (EffectInstance, Duration)>,
    settled: &mut Settled,
    which: impl Fn(&str, &EffectInstance) -> bool,
) {
    for (id, (effect, _)) in effects {
        if is_move(effect)
            && which(id, effect)
            && own_elapsed(effect, before, time) >= effect.total_duration()
        {
            settled.arrived.insert(id.clone());
        }
    }
}

/// Runs `cues`, in order, the way the engine runs them live: layer commands
/// move the layers' freezes and speeds on, clears and stopped sequences take
/// their effects out, and every effect cued goes in. The effects over by the
/// last cue are still in; [`settle_layer`] takes them out.
fn replay(cues: &[Cue]) -> Replay {
    let mut replay = Replay::default();
    let Replay {
        result,
        layers,
        settled,
        stopped_sequences,
    } = &mut replay;

    // Cues at the same time fire in the same update live, which applies
    // all their layer commands, then their sequence stops, then their
    // effects: a clear in one cue does not take out an effect another cue
    // starts at the same time.
    for group in cues.chunk_by(|a, b| a.time == b.time) {
        let time = group[0].time;
        // The layers as they stood when the group landed: a move ending on
        // this instant has arrived by them, whatever a speed change in the
        // group then makes of it (see `arrive_moves_ending_at`).
        let before = layers.clone();
        // Cue by cue, as the timeline fires them live: a cue's sequence
        // starts clear its stopped status, its effects are taken unless their
        // sequence is stopped by then, and its stops then count for the cues
        // after it. The effects start below, after the group's stops.
        let mut fired = Vec::new();
        for cue in group {
            for seq_name in &cue.start_sequences {
                stopped_sequences.remove(seq_name);
            }
            for effect in &cue.effects {
                if let Some(ref seq_name) = effect.sequence_name {
                    if stopped_sequences.contains(seq_name) {
                        continue;
                    }
                }
                fired.push(LightingTimeline::create_effect_instance(effect, cue.time));
            }
            process_stop_sequences(stopped_sequences, cue, result);
        }

        for cue in group {
            // Apply all layer commands from historical cues
            result
                .layer_commands
                .extend(cue.layer_commands.iter().cloned());

            // Simulate clear commands: purge effects that would have been
            // stopped so we only start effects that are actually active at the
            // seek point. A finished move stays through a layer clear: live,
            // only a full `clear` releases pose memory (design §15.4), and the
            // replayed move is what carries the pose to the seek point. The
            // other layer commands move the layers' freezes and speeds on.
            for cmd in &cue.layer_commands {
                match cmd.command_type {
                    LayerCommandType::Clear => {
                        if let Some(layer) = cmd.layer {
                            settle_layer(
                                layer,
                                time,
                                layers,
                                &mut result.effects_with_elapsed,
                                settled,
                                false,
                            );
                            arrive_moves_ending_at(
                                time,
                                &before,
                                &result.effects_with_elapsed,
                                settled,
                                |_, effect| effect.layer == layer,
                            );
                            let mut cut = false;
                            result.effects_with_elapsed.retain(|id, (effect, _)| {
                                let keep = effect.layer != layer || settled.arrived.contains(id);
                                cut |= !keep;
                                keep
                            });
                            if cut {
                                settled.ended = settled.ended.max(time);
                            }
                            layers.remove(&layer);
                        } else {
                            // What is over by now ended on its own, not here.
                            let all: HashSet<EffectLayer> = result
                                .effects_with_elapsed
                                .values()
                                .map(|(effect, _)| effect.layer)
                                .collect();
                            for layer in all {
                                settle_layer(
                                    layer,
                                    time,
                                    layers,
                                    &mut result.effects_with_elapsed,
                                    settled,
                                    false,
                                );
                            }
                            if result
                                .effects_with_elapsed
                                .keys()
                                .any(|id| !settled.arrived.contains(id))
                            {
                                settled.ended = settled.ended.max(time);
                            }
                            result.effects_with_elapsed.clear();
                            layers.clear();
                            settled.arrived.clear();
                        }
                    }
                    _ => {
                        if let Some(layer) = cmd.layer {
                            replay_layer_command(
                                cmd,
                                layer,
                                time,
                                layers,
                                &mut result.effects_with_elapsed,
                                settled,
                            );
                        }
                    }
                }
            }
        }
        // The cue lands, then the frame renders and expires anything at
        // or past its end: an effect ending on this instant was kept for
        // the commands to catch, and one they left over goes now. A look
        // frozen on its last instant is over, and a later change of speed
        // must not bring it back.
        let timed: HashSet<EffectLayer> = group
            .iter()
            .flat_map(|cue| &cue.layer_commands)
            .filter(|cmd| times_layer(cmd))
            .filter_map(|cmd| cmd.layer)
            .collect();
        for layer in timed {
            settle_layer(
                layer,
                time,
                layers,
                &mut result.effects_with_elapsed,
                settled,
                false,
            );
        }

        for cue in group {
            // Purge already-accumulated effects from stopped sequences,
            // mirroring what EffectEngine::stop_sequence() does at runtime.
            // Stopping a sequence does not release pose memory either.
            for seq_name in &cue.stop_sequences {
                let prefix = format!("seq_{}_", seq_name);
                let stopped: Vec<EffectLayer> = result
                    .effects_with_elapsed
                    .iter()
                    .filter(|(id, _)| id.starts_with(&prefix))
                    .map(|(_, (effect, _))| effect.layer)
                    .collect();
                for layer in stopped.into_iter().collect::<HashSet<_>>() {
                    settle_layer(
                        layer,
                        time,
                        layers,
                        &mut result.effects_with_elapsed,
                        settled,
                        false,
                    );
                }
                arrive_moves_ending_at(
                    time,
                    &before,
                    &result.effects_with_elapsed,
                    settled,
                    |id, _| id.starts_with(&prefix),
                );
                let mut cut = false;
                result.effects_with_elapsed.retain(|id, _| {
                    let keep = !id.starts_with(&prefix) || settled.arrived.contains(id);
                    cut |= !keep;
                    keep
                });
                if cut {
                    settled.ended = settled.ended.max(time);
                }
            }
        }

        // Every effect cued goes in; the ones over by a given time come
        // out once their layers' history up to it is known. The frame these
        // cues land in then renders, and expires one over already: a look
        // with no length, on a frozen layer too.
        let started: HashSet<EffectLayer> = fired.iter().map(|effect| effect.layer).collect();
        for effect_instance in fired {
            result.effects_with_elapsed.insert(
                effect_instance.id.clone(),
                (effect_instance, Duration::ZERO),
            );
        }
        for layer in started {
            settle_layer(
                layer,
                time,
                layers,
                &mut result.effects_with_elapsed,
                settled,
                false,
            );
        }
    }
    replay
}

/// When the show's last effect ends, by its own elapsed: those over by the
/// last cue when the replay took them out, and those still running then
/// on from there: a slowed layer stretches an effect past its authored end, and a
/// freeze holds it there for as long as it lasts. With whether one is on a
/// layer the show leaves frozen, which holds it for good: that one has no
/// end, and the others' still count.
fn running_end(cues: &[Cue]) -> (Duration, bool) {
    let Some(last) = cues.last().map(|cue| cue.time) else {
        return (Duration::ZERO, false);
    };
    let Replay {
        mut result,
        layers,
        mut settled,
        ..
    } = replay(cues);
    let all: HashSet<EffectLayer> = result
        .effects_with_elapsed
        .values()
        .map(|(effect, _)| effect.layer)
        .collect();
    for layer in all {
        settle_layer(
            layer,
            last,
            &layers,
            &mut result.effects_with_elapsed,
            &mut settled,
            false,
        );
    }
    let mut end = settled.ended;
    let mut open_ended = false;
    for (id, (effect, _)) in &result.effects_with_elapsed {
        if settled.arrived.contains(id) {
            continue;
        }
        let state = layers.get(&effect.layer);
        if state.is_some_and(|layer| layer.frozen_since.is_some()) {
            open_ended = true;
            continue;
        }
        let remaining = effect
            .total_duration()
            .saturating_sub(own_elapsed(effect, &layers, last));
        // Back into song time, as the engine scales it (a 0 left from a
        // freeze by speed since undone does not scale).
        let speed = state.map_or(1.0, |layer| layer.speed);
        let remaining = if crate::lighting::engine::speed_scales(speed) {
            Duration::try_from_secs_f64(remaining.as_secs_f64() / speed).ok()
        } else {
            Some(remaining)
        };
        // So slow it never ends in any time there is: as good as frozen.
        match remaining.and_then(|remaining| last.checked_add(remaining)) {
            Some(at) => end = end.max(at),
            None => open_ended = true,
        }
    }
    (end, open_ended)
}

/// A timeline arming's [`LightingTimeline::epoch`]: unique across every
/// timeline.
fn next_epoch() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// Timeline processor for lighting cues during song playback
pub struct LightingTimeline {
    /// Cues sorted by time (now using DSL Cue directly)
    cues: Vec<Cue>,
    /// Current position in the timeline
    current_time: Duration,
    /// Index of the next cue to process
    next_cue_index: usize,
    /// Whether the timeline is currently playing
    is_playing: bool,
    /// Armed, but firing nothing yet: while the state it starts from goes
    /// in, a cue at the start point would fire into the engine about to be
    /// cleared, and be lost.
    held: bool,
    /// Tempo map for tempo-aware effects (from the first show that has one)
    tempo_map: Option<crate::lighting::tempo::TempoMap>,
    /// Sequences that have been stopped — future cues from these sequences are suppressed
    stopped_sequences: HashSet<String>,
    /// Which arming of which timeline this is: new for every timeline, and
    /// at every start, seek and stop. What it fired is applied only while it
    /// is still the same, so a cue taken from it just before a seek, a
    /// section loop's restart or a hand-over to another engine does not land
    /// on what came after.
    epoch: u64,
    /// When the last effect of the last cue has run its course: the show's
    /// end, as opposed to its last cue's time.
    show_end: Duration,
    /// A bound the show's end is held to — the song's audio length — so a
    /// tail past the audio never holds a song open.
    end_cap: Option<Duration>,
    /// The show leaves a layer frozen with an effect on it: held for good,
    /// so with audio the show runs to the cap.
    open_ended: bool,
}

impl LightingTimeline {
    /// Creates a new lighting timeline from DSL light shows
    pub fn new(shows: Vec<LightShow>) -> Self {
        let mut all_cues = Vec::new();
        let mut tempo_map: Option<crate::lighting::tempo::TempoMap> = None;

        // Extract cues and tempo map from shows
        // Use the first show's tempo map if available
        for show in shows {
            all_cues.extend(show.cues);
            // Use the first show's tempo map
            if tempo_map.is_none() {
                tempo_map = show.tempo_map;
            }
        }

        let mut timeline = Self::new_with_cues(all_cues);
        timeline.tempo_map = tempo_map;
        timeline
    }

    /// Creates a new lighting timeline from DSL cues (for testing)
    pub(crate) fn new_with_cues(cues: Vec<Cue>) -> Self {
        // The last effect's real end: a `clear` or a `stop sequence` cuts
        // an effect short, and a show that ends on one is over then, not
        // when the cleared bed would have run out. Same reckoning as the
        // lint's; a freeze or the speed master then carries it on (below),
        // which the lint does not know.
        let clears: Vec<(Option<crate::lighting::effects::EffectLayer>, Duration)> = cues
            .iter()
            .flat_map(|cue| {
                cue.layer_commands
                    .iter()
                    .filter(|cmd| cmd.command_type == LayerCommandType::Clear)
                    .map(move |cmd| (cmd.layer, cue.time))
            })
            .collect();
        let stops: Vec<(String, Duration)> = cues
            .iter()
            .flat_map(|cue| {
                cue.stop_sequences
                    .iter()
                    .map(move |name| (name.clone(), cue.time))
            })
            .collect();
        let (clears, stops) = (&clears, &stops);
        let show_end = cues
            .iter()
            .flat_map(|cue| {
                cue.effects.iter().map(move |e| {
                    crate::lighting::lint::effective_end(
                        cue.time + e.total_duration(),
                        cue.time,
                        e.layer
                            .unwrap_or(crate::lighting::effects::EffectLayer::Background),
                        e.sequence_name.as_deref(),
                        clears,
                        stops,
                    )
                })
            })
            .max()
            .unwrap_or(Duration::ZERO);
        let mut timeline = Self {
            cues,
            current_time: Duration::ZERO,
            next_cue_index: 0,
            is_playing: false,
            held: false,
            tempo_map: None,
            stopped_sequences: HashSet::new(),
            epoch: next_epoch(),
            show_end,
            end_cap: None,
            open_ended: false,
        };
        timeline.sort_cues();
        // A freeze or the speed master holds an effect past its authored
        // end, and the song's time stops once the show is over: an effect
        // still running then never ends (#590). So the show runs on until
        // they do. The effects over before the last cue are over by it.
        let holds = timeline
            .cues
            .iter()
            .any(|cue| cue.layer_commands.iter().any(times_layer));
        if holds {
            let (end, open_ended) = running_end(&timeline.cues);
            // A look held for good has no end to wait for. Without audio
            // to run to, the show keeps at least the end it was written
            // with: an ambient show loops then, the frozen look up until it
            // does.
            timeline.show_end = if open_ended {
                timeline.show_end.max(end)
            } else {
                end
            };
            timeline.open_ended = open_ended;
        }
        timeline
    }

    /// Bounds the show's end to the song's length, when the song has one:
    /// with audio, the audio decides when the song ends, as it always has;
    /// without, the last effect does.
    pub fn set_end_cap(&mut self, cap: Option<Duration>) {
        self.end_cap = cap;
    }

    /// The bound set by [`Self::set_end_cap`], carried over on a hot reload.
    pub fn end_cap(&self) -> Option<Duration> {
        self.end_cap
    }

    /// When the show is over: its last effect's end, or the cap. A show
    /// that leaves a look frozen for good runs to the cap; with none, it
    /// keeps at least the end it was written with, the look held.
    pub fn show_end(&self) -> Duration {
        match self.end_cap {
            Some(cap) if self.open_ended => cap,
            Some(cap) => self.show_end.min(cap),
            None => self.show_end,
        }
    }

    /// Get the tempo map for this timeline
    pub fn tempo_map(&self) -> Option<&crate::lighting::tempo::TempoMap> {
        self.tempo_map.as_ref()
    }

    /// Sorts cues by time
    fn sort_cues(&mut self) {
        self.cues.sort_by(|a, b| a.time.cmp(&b.time));
    }

    /// Starts the timeline
    pub fn start(&mut self) {
        self.epoch = next_epoch();
        self.is_playing = true;
        self.current_time = Duration::ZERO;
        self.next_cue_index = 0;
        self.stopped_sequences.clear();
    }

    /// Starts the timeline at a specific time (for seeking)
    /// This processes all cues before start_time to ensure deterministic state
    pub fn start_at(&mut self, start_time: Duration) -> TimelineUpdate {
        self.epoch = next_epoch();
        self.is_playing = true;
        self.current_time = start_time;
        self.stopped_sequences.clear();
        self.next_cue_index = self.find_cue_index_at(start_time);

        // Process all cues before start_time to ensure deterministic state.
        // The layers' freezes and speed masters as the history leaves them
        // are kept as the engine keeps them live: an effect's own elapsed is
        // its score time less its time frozen, scaled by its layer's speed,
        // and it is that, not score time, which says whether it is still
        // running and where it picks up (#589). Moves that have arrived did
        // so at the speed they ran at, which a later change of speed or a
        // clear must not take back.
        let Replay {
            mut result,
            layers,
            mut settled,
            stopped_sequences,
        } = replay(&self.cues[..self.next_cue_index]);
        self.stopped_sequences = stopped_sequences;

        // Only include effects that would still be running at start_time,
        // started at their own elapsed there; a finished move stays for its
        // pose (see `settle_layer`). A freeze, unfreeze or speed change cued
        // at the start point fires in the first frame, before it expires
        // anything: an effect ending exactly there stays for it to catch, as
        // it does played through.
        let timed_at_start: HashSet<EffectLayer> = self.cues[self.next_cue_index..]
            .iter()
            .take_while(|cue| cue.time == start_time)
            .flat_map(|cue| &cue.layer_commands)
            .filter(|cmd| times_layer(cmd))
            .filter_map(|cmd| cmd.layer)
            .collect();
        let all: HashSet<EffectLayer> = result
            .effects_with_elapsed
            .values()
            .map(|(effect, _)| effect.layer)
            .collect();
        for layer in all {
            settle_layer(
                layer,
                start_time,
                &layers,
                &mut result.effects_with_elapsed,
                &mut settled,
                timed_at_start.contains(&layer),
            );
        }
        let arrived = settled.arrived;
        for (id, (effect, elapsed)) in result.effects_with_elapsed.iter_mut() {
            *elapsed = base_elapsed(effect, &layers, start_time);
            // Kept on its last instant for a cue at the start point: started
            // just short of it, or the engine would judge a move arrived at
            // the speed before the cue fires, and commit it.
            if timed_at_start.contains(&effect.layer)
                && !arrived.contains(id)
                && own_elapsed(effect, &layers, start_time) == effect.total_duration()
            {
                *elapsed = elapsed.saturating_sub(Duration::from_nanos(1));
            }
            if arrived.contains(id)
                && own_elapsed(effect, &layers, start_time) < effect.total_duration()
            {
                // There at the speed it ran at, which has slowed (or been
                // cleared) since: the engine commits it to its destination.
                *elapsed = Duration::MAX;
                continue;
            }
            // On a layer still frozen, the engine holds it at the start
            // point (where it is told the layer froze) less its time frozen:
            // counted up to the start point, that is where it froze.
            if let Some(frozen_at) = layers.get(&effect.layer).and_then(|l| l.frozen_since) {
                let cue_time = effect.cue_time.unwrap_or(Duration::ZERO);
                effect.score_frozen += start_time.saturating_sub(cue_time.max(frozen_at));
            }
        }

        let total_effects = result.effects.len() + result.effects_with_elapsed.len();
        if total_effects > 20 {
            tracing::warn!(
                effects = result.effects.len(),
                effects_with_elapsed = result.effects_with_elapsed.len(),
                start_time_ms = start_time.as_millis(),
                "start_at() yielded unusually large effect batch"
            );
        }

        result
    }

    /// Finds the index of the first cue that should trigger at or after the given time.
    /// Uses `partition_point` to handle duplicate cue times correctly —
    /// `binary_search` may land on any duplicate, but `partition_point`
    /// always returns the first cue with `time >= target`.
    fn find_cue_index_at(&self, time: Duration) -> usize {
        self.cues.partition_point(|cue| cue.time < time)
    }

    /// Which arming of which timeline this is (see the field).
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    /// Holds an armed timeline from firing cues (`true`), or lets it go on
    /// from where it was armed (`false`).
    pub fn hold(&mut self, held: bool) {
        self.held = held;
    }

    /// Stops the timeline
    pub fn stop(&mut self) {
        self.epoch = next_epoch();
        self.is_playing = false;
        self.current_time = Duration::ZERO;
        self.next_cue_index = 0;
        self.stopped_sequences.clear();
    }

    /// Whether the timeline has been started and is dispatching cues.
    ///
    /// A timeline that is installed but never armed looks identical from the
    /// outside to one that is armed and simply between cues — which is the
    /// ambiguity that made #332 diagnosable only by inference.
    pub fn is_armed(&self) -> bool {
        self.is_playing
    }

    /// How many cues are still ahead of the pointer.
    pub fn cues_remaining(&self) -> usize {
        self.cues.len().saturating_sub(self.next_cue_index)
    }

    /// Total cues in the timeline.
    pub fn cue_count(&self) -> usize {
        self.cues.len()
    }

    /// When the next cue fires, if any remain.
    pub fn next_cue_time(&self) -> Option<Duration> {
        self.cues.get(self.next_cue_index).map(|cue| cue.time)
    }

    /// Whether the show is over: every cue dispatched and, while playing,
    /// the last effect run its course (or the song's audio ended). An
    /// empty timeline is over at once, so nothing waits on it forever. A
    /// song with no audio used to end the moment its last cue fired,
    /// cutting that cue's fade or hold; now it ends when the cue does.
    pub fn is_finished(&self) -> bool {
        self.next_cue_index >= self.cues.len()
            && (!self.is_playing || self.current_time >= self.show_end())
    }

    /// Updates the timeline with the current song time
    /// Returns both effects and layer commands to be processed
    pub fn update(&mut self, song_time: Duration) -> TimelineUpdate {
        if !self.is_playing || self.held {
            return TimelineUpdate::default();
        }
        self.current_time = song_time;
        let mut result = TimelineUpdate::default();

        // Process all cues that should trigger at or before the current time
        while self.next_cue_index < self.cues.len() {
            let cue = &self.cues[self.next_cue_index];

            if cue.time <= song_time {
                // Process start_sequences first: clear stopped status for new invocations
                for seq_name in &cue.start_sequences {
                    self.stopped_sequences.remove(seq_name);
                }

                // Process effects, skipping those from stopped sequences
                for effect in &cue.effects {
                    if let Some(ref seq_name) = effect.sequence_name {
                        if self.stopped_sequences.contains(seq_name) {
                            continue;
                        }
                    }
                    let effect_instance = Self::create_effect_instance(effect, cue.time);
                    result.effects.push(effect_instance);
                }
                // Process layer commands
                result
                    .layer_commands
                    .extend(cue.layer_commands.iter().cloned());
                result
                    .layer_command_times
                    .extend(cue.layer_commands.iter().map(|_| cue.time));

                process_stop_sequences(&mut self.stopped_sequences, cue, &mut result);

                self.next_cue_index += 1;
            } else {
                // No more cues to process at this time
                break;
            }
        }

        if result.effects.len() > 20 {
            tracing::warn!(
                effects = result.effects.len(),
                cue_index = self.next_cue_index,
                song_time_ms = song_time.as_millis(),
                "Timeline yielded unusually large effect batch in single update"
            );
        }

        result
    }

    /// Each cue's time paired with the effects it starts.
    ///
    /// Coverage analysis needs both to know where the running set can change:
    /// a cue time, and that time plus each effect's authored duration.
    pub fn cue_effects(&self) -> impl Iterator<Item = (Duration, &[Effect])> {
        self.cues
            .iter()
            .map(|cue| (cue.time, cue.effects.as_slice()))
    }

    /// Get all cues with their times and indices
    pub fn cues(&self) -> Vec<(Duration, usize)> {
        self.cues
            .iter()
            .enumerate()
            .map(|(index, cue)| (cue.time, index))
            .collect()
    }

    /// Creates an EffectInstance from a DSL Effect.
    /// cue_time is the song time when this effect was supposed to start (for deterministic randomness).
    pub fn create_effect_instance(effect: &Effect, cue_time: Duration) -> EffectInstance {
        let id = EFFECT_ID_COUNTER.fetch_add(1, Ordering::Relaxed);
        let effect_id = if let Some(ref seq_name) = effect.sequence_name {
            format!("seq_{}_effect_{}", seq_name, id)
        } else {
            format!("song_effect_{}", id)
        };

        let mut effect_instance = EffectInstance::new(
            effect_id,
            effect.effect_type.clone(),
            effect.groups.clone(),
            effect.up_time,
            effect.hold_time,
            effect.down_time,
        );

        // Store the cue time for deterministic randomness
        effect_instance.cue_time = Some(cue_time);

        // Apply layering information if specified in DSL
        if let Some(layer) = effect.layer {
            effect_instance.layer = layer;
        }
        if let Some(blend_mode) = effect.blend_mode {
            effect_instance.blend_mode = blend_mode;
        }
        effect_instance.per_cell = effect.per_cell;
        effect_instance.spread = effect.spread;
        effect_instance.tempo_offset_secs = effect.tempo_offset_secs;

        effect_instance
    }
}

/// Process stop_sequences for a cue: iteration-boundary stops (where the cue
/// also starts effects from the same sequence) only go to the engine, while
/// explicit stops also suppress future cues from that sequence.
fn process_stop_sequences(
    stopped_sequences: &mut HashSet<String>,
    cue: &crate::lighting::parser::Cue,
    result: &mut TimelineUpdate,
) {
    let cue_sequence_names: HashSet<&String> = cue
        .effects
        .iter()
        .filter_map(|e| e.sequence_name.as_ref())
        .collect();
    for seq_name in &cue.stop_sequences {
        if !cue_sequence_names.contains(seq_name) {
            stopped_sequences.insert(seq_name.clone());
        }
    }
    result
        .stop_sequences
        .extend(cue.stop_sequences.iter().cloned());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::effects::EffectType;
    use crate::lighting::parser::Effect;
    use std::collections::HashMap;

    #[test]
    fn test_timeline_creation() {
        let cues = vec![];
        let timeline = LightingTimeline::new_with_cues(cues);
        assert_eq!(timeline.cues.len(), 0);
        assert!(!timeline.is_playing);
    }

    #[test]
    fn test_timeline_start_stop() {
        let cues = vec![];
        let mut timeline = LightingTimeline::new_with_cues(cues);

        assert!(!timeline.is_playing);
        timeline.start();
        assert!(timeline.is_playing);
        timeline.stop();
        assert!(!timeline.is_playing);
    }

    #[test]
    fn test_timeline_is_finished() {
        use crate::lighting::parser::Cue;

        // Empty timeline should be considered finished (prevents blocking forever)
        let empty_timeline = LightingTimeline::new_with_cues(vec![]);
        assert!(
            empty_timeline.is_finished(),
            "Empty timeline should be finished"
        );

        // Timeline with cues should not be finished initially
        let effect = Effect {
            sequence_name: None,
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(5),
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };
        let cues = vec![Cue {
            stop_sequences: vec![],
            start_sequences: vec![],
            time: Duration::from_millis(0),
            effects: vec![effect],
            layer_commands: vec![],
        }];
        let mut timeline = LightingTimeline::new_with_cues(cues);
        assert!(
            !timeline.is_finished(),
            "Timeline with unprocessed cues should not be finished"
        );

        // The last cue has fired but its 5 s effect is still running: the
        // show is not over until it is.
        timeline.start();
        let _ = timeline.update(Duration::from_secs(1));
        assert!(
            !timeline.is_finished(),
            "the last cue's effect is still running"
        );
        let _ = timeline.update(Duration::from_secs(5));
        assert!(timeline.is_finished(), "the last effect has run its course");
    }

    /// A show that ends on a `clear` is over at the clear, not when the
    /// bed it cleared would have run out; a layer clear ends only that
    /// layer's effects, and a stopped sequence ends at the stop.
    #[test]
    fn a_clear_or_a_stop_ends_the_show_early() {
        let parse = |src: &str| {
            crate::lighting::parser::parse_light_shows(src)
                .unwrap()
                .into_values()
                .collect::<Vec<_>>()
        };
        let full = LightingTimeline::new(parse(
            "show \"s\" {\n    @00:00.000\n    spots: static red: 100%, duration: 30s\n    \
             @00:05.000\n    clear()\n}\n",
        ));
        assert_eq!(full.show_end(), Duration::from_secs(5));

        let layer = LightingTimeline::new(parse(
            "show \"s\" {\n    @00:00.000\n    spots: static red: 100%, duration: 30s\n    \
             spots: static blue: 100%, duration: 12s, layer: midground\n    \
             @00:05.000\n    clear(layer: background)\n}\n",
        ));
        assert_eq!(
            layer.show_end(),
            Duration::from_secs(12),
            "the midground bed runs on"
        );

        let stopped = LightingTimeline::new(parse(
            "sequence \"bed\" {\n    @00:00.000\n    spots: static red: 100%, duration: 30s\n}\n\n\
             show \"s\" {\n    @00:00.000\n    sequence \"bed\"\n    @00:07.000\n    stop sequence \"bed\"\n}\n",
        ));
        assert_eq!(stopped.show_end(), Duration::from_secs(7));
    }

    /// A song with audio ends with the audio: an effect tail past the
    /// audio's end is cut, as it always was, not waited for.
    #[test]
    fn the_end_cap_bounds_the_show_to_the_audio() {
        let shows = crate::lighting::parser::parse_light_shows(
            "show \"s\" {\n    @00:10.000\n    spots: static red: 100%, duration: 30s\n}\n",
        )
        .unwrap()
        .into_values()
        .collect();
        let mut timeline = LightingTimeline::new(shows);
        assert_eq!(timeline.show_end(), Duration::from_secs(40));
        timeline.set_end_cap(Some(Duration::from_secs(20)));
        assert_eq!(timeline.show_end(), Duration::from_secs(20));
        timeline.start();
        let _ = timeline.update(Duration::from_secs(12));
        assert!(!timeline.is_finished());
        let _ = timeline.update(Duration::from_secs(20));
        assert!(timeline.is_finished(), "the audio has ended");
    }

    /// A freeze or the speed master holds an effect past its authored end,
    /// and song time stops once the show is over, so the show runs on to
    /// the effect's real end: one still running when the timeline finished
    /// never ended (#590).
    #[test]
    fn a_freeze_or_the_speed_master_carries_the_show_end() {
        let end = |body: &str| {
            let shows = crate::lighting::parser::parse_light_shows(&format!(
                "show \"s\" {{\n    @00:00.000\n    \
                 spots: static red: 100%, duration: 4s, layer: midground\n{body}}}\n"
            ))
            .unwrap()
            .into_values()
            .collect();
            LightingTimeline::new(shows)
        };
        let secs = |s: f64| Duration::from_secs_f64(s);

        let frozen = end("    @00:02.000\n    freeze(layer: midground)\n    \
             @00:07.000\n    unfreeze(layer: midground)\n");
        assert_eq!(frozen.show_end(), secs(9.0), "frozen 2-7 s, a 4 s look");

        let stopped = end(
            "    @00:02.000\n    master(layer: midground, speed: 0)\n    \
             @00:07.000\n    master(layer: midground, speed: 1)\n",
        );
        assert_eq!(stopped.show_end(), secs(9.0), "speed 0 then 1, as a freeze");

        let slowed = end("    @00:01.000\n    master(layer: midground, speed: 0.5)\n");
        // At half speed from 1 s, a 4 s look's elapsed is half its score
        // time, as the engine scales it: 0.5 s there, so 7 s more to go.
        assert_eq!(slowed.show_end(), secs(8.0), "slowed to half at 1 s");

        let quick = end("    @00:01.000\n    master(layer: midground, speed: 2)\n");
        assert_eq!(quick.show_end(), secs(2.0), "double speed from 1 s");

        let early = end("    @00:01.000\n    freeze(layer: midground)\n    \
             @00:02.000\n    unfreeze(layer: midground)\n    \
             @00:08.000\n    freeze(layer: background)\n");
        assert_eq!(
            early.show_end(),
            secs(5.0),
            "frozen 1-2 s, over at 5 s: before the last cue, which the show ends on all the same"
        );

        // Frozen for good: with audio the show runs to its end; with none,
        // it keeps the end it was written with, the look held.
        let mut held = end("    @00:02.000\n    freeze(layer: midground)\n");
        assert_eq!(held.show_end(), secs(4.0));
        held.set_end_cap(Some(secs(60.0)));
        assert_eq!(held.show_end(), secs(60.0));
        held.start();
        let _ = held.update(secs(30.0));
        assert!(!held.is_finished(), "the frozen look is still up");
    }

    /// The speed master's edges: an unfreeze from speed 0 runs the rest at
    /// normal speed, as the engine does; a speed too fast to measure ends
    /// the look at once; one too slow ever to end holds it, like a freeze.
    /// None of them may panic, which they did as the show loaded.
    #[test]
    fn the_show_end_survives_extreme_speeds() {
        let end = |body: &str| {
            let shows = crate::lighting::parser::parse_light_shows(&format!(
                "show \"s\" {{\n    @00:00.000\n    \
                 spots: static red: 100%, duration: 4s, layer: midground\n    \
                 @00:01.000\n{body}}}\n"
            ))
            .unwrap()
            .into_values()
            .collect();
            LightingTimeline::new(shows)
        };
        let secs = |s: f64| Duration::from_secs_f64(s);

        let undone = end("    master(layer: midground, speed: 0)\n    \
             @00:02.000\n    unfreeze(layer: midground)\n");
        assert_eq!(undone.show_end(), secs(5.0), "frozen 1-2 s, then the rest");

        let fast = end("    master(layer: midground, speed: 100000000000000000000)\n");
        assert_eq!(fast.show_end(), secs(1.0), "over at once");

        let mut slow = end("    master(layer: midground, speed: 0.00000000000000000001)\n");
        assert_eq!(slow.show_end(), secs(4.0), "no audio: its written end");
        slow.set_end_cap(Some(secs(60.0)));
        assert_eq!(slow.show_end(), secs(60.0), "with audio: held to its end");
    }

    /// A layer left frozen holds its look for good, and the others' looks
    /// still count: with no audio (a song's length, an ambient loop's) the
    /// show is not cut to the last cue.
    #[test]
    fn a_layer_left_frozen_keeps_the_other_layers_ends() {
        let shows = crate::lighting::parser::parse_light_shows(
            "show \"s\" {\n    @00:00.000\n    \
             spots: static red: 100%, duration: 4s, layer: midground\n    \
             spots: static blue: 100%, duration: 10s, layer: foreground\n    \
             @00:01.000\n    freeze(layer: midground)\n}\n",
        )
        .unwrap()
        .into_values()
        .collect();
        let mut timeline = LightingTimeline::new(shows);
        assert_eq!(timeline.show_end(), Duration::from_secs(10));
        timeline.set_end_cap(Some(Duration::from_secs(60)));
        assert_eq!(timeline.show_end(), Duration::from_secs(60));
    }

    /// Cues at the same time fire together live, layer commands before
    /// effects: a layer clear in one cue leaves an effect another cue
    /// starts then, from another show or file, running.
    #[test]
    fn a_clear_at_the_same_time_leaves_another_cues_effect() {
        let cues_of = |src: &str| {
            crate::lighting::parser::parse_light_shows(src)
                .unwrap()
                .into_values()
                .next()
                .unwrap()
                .cues
        };
        let mut cues = cues_of(
            "show \"a\" {\n    @00:00.000\n    \
             spots: static blue: 100%, duration: 10s, layer: foreground\n}\n",
        );
        cues.extend(cues_of(
            "show \"b\" {\n    @00:00.000\n    clear(layer: foreground)\n    \
             @00:01.000\n    master(layer: background, speed: 0.5)\n}\n",
        ));
        let mut timeline = LightingTimeline::new_with_cues(cues);
        assert_eq!(timeline.show_end(), Duration::from_secs(10));
        let update = timeline.start_at(Duration::from_secs(5));
        assert_eq!(update.effects_with_elapsed.len(), 1, "a seek keeps it too");
    }

    /// A show without freezes or speed masters keeps its authored end.
    #[test]
    fn a_show_without_holds_keeps_its_end() {
        let shows = crate::lighting::parser::parse_light_shows(
            "show \"s\" {\n    @00:00.000\n    spots: static red: 100%, duration: 4s\n    \
             @00:01.000\n    master(layer: background, intensity: 50%)\n}\n",
        )
        .unwrap()
        .into_values()
        .collect();
        assert_eq!(
            LightingTimeline::new(shows).show_end(),
            Duration::from_secs(4)
        );
    }

    #[test]
    fn test_timeline_with_dsl_cues() {
        use crate::lighting::parser::Cue;

        // Create DSL cues directly
        let mut parameters = HashMap::new();
        parameters.insert("color".to_string(), "blue".to_string());
        parameters.insert("dimmer".to_string(), "60%".to_string());

        let effect = Effect {
            sequence_name: None,
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
            groups: vec!["front_wash".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(5),
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![Cue {
            stop_sequences: vec![],
            start_sequences: vec![],
            time: Duration::from_millis(0),
            effects: vec![effect],
            layer_commands: vec![],
        }];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        // Test that the first cue triggers at the right time
        let result = timeline.update(Duration::from_millis(0));
        assert_eq!(result.effects.len(), 1);
        assert_eq!(result.effects[0].target_fixtures, vec!["front_wash"]);
    }

    #[test]
    fn test_timeline_cue_ordering() {
        use crate::lighting::parser::Cue;

        // Create cues in non-chronological order
        let mut parameters = HashMap::new();
        parameters.insert("color".to_string(), "blue".to_string());

        let effect = Effect {
            sequence_name: None,
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(5),
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            Cue {
                time: Duration::from_millis(10000),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_millis(5000),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_millis(0),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        // Verify cues are processed in chronological order
        let result = timeline.update(Duration::from_millis(0));
        assert_eq!(result.effects.len(), 1);

        let result = timeline.update(Duration::from_millis(5000));
        assert_eq!(result.effects.len(), 1);

        let result = timeline.update(Duration::from_millis(10000));
        assert_eq!(result.effects.len(), 1);
    }

    #[test]
    fn test_timeline_edge_cases() {
        // Test with empty timeline
        let timeline = LightingTimeline::new_with_cues(vec![]);
        assert_eq!(timeline.cues.len(), 0);

        // Test with cues at the same time
        let mut parameters = HashMap::new();
        parameters.insert("color".to_string(), "blue".to_string());

        let effect = Effect {
            sequence_name: None,
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(5),
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            Cue {
                time: Duration::from_millis(5000),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_millis(5000),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        // Both cues should trigger at the same time
        let result = timeline.update(Duration::from_millis(5000));
        assert_eq!(result.effects.len(), 2);
    }

    #[test]
    fn test_timeline_stop_reset() {
        let mut parameters = HashMap::new();
        parameters.insert("color".to_string(), "red".to_string());

        let effect1 = Effect {
            sequence_name: None,
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
            groups: vec!["fixture1".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(5),
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let effect2 = Effect {
            sequence_name: None,
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
            groups: vec!["fixture2".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(5),
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            Cue {
                time: Duration::from_secs(0),
                effects: vec![effect1],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(2),
                effects: vec![effect2],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        // Start the timeline and process some cues
        timeline.start();
        let _effects_at_0s = timeline.update(Duration::from_secs(0));
        let _effects_at_2s = timeline.update(Duration::from_secs(2));

        // Stop the timeline - this should reset the cue index
        timeline.stop();

        // Start again - should trigger the first cue again
        timeline.start();
        let result_at_0s_restart = timeline.update(Duration::from_secs(0));
        assert_eq!(result_at_0s_restart.effects.len(), 1);

        // Should also trigger the second cue again
        let result_at_2s_restart = timeline.update(Duration::from_secs(2));
        assert_eq!(result_at_2s_restart.effects.len(), 1);
    }

    #[test]
    fn test_timeline_layer_commands() {
        use crate::lighting::effects::EffectLayer;
        use crate::lighting::parser::{Cue, LayerCommand, LayerCommandType};

        // Create cue with a layer command
        let cues = vec![
            Cue {
                time: Duration::from_secs(0),
                effects: vec![],
                layer_commands: vec![LayerCommand {
                    command_type: LayerCommandType::Clear,
                    layer: Some(EffectLayer::Foreground),
                    intensity: None,
                    speed: None,
                }],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(1),
                effects: vec![],
                layer_commands: vec![LayerCommand {
                    command_type: LayerCommandType::Freeze,
                    layer: Some(EffectLayer::Background),
                    intensity: None,
                    speed: None,
                }],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(2),
                effects: vec![],
                layer_commands: vec![LayerCommand {
                    command_type: LayerCommandType::Master,
                    layer: Some(EffectLayer::Midground),
                    intensity: Some(0.5),
                    speed: Some(2.0),
                }],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        // First cue: clear command
        let result0 = timeline.update(Duration::from_secs(0));
        assert_eq!(result0.effects.len(), 0);
        assert_eq!(result0.layer_commands.len(), 1);
        assert_eq!(
            result0.layer_commands[0].command_type,
            LayerCommandType::Clear
        );
        assert_eq!(
            result0.layer_commands[0].layer,
            Some(EffectLayer::Foreground)
        );

        // Second cue: freeze command
        let result1 = timeline.update(Duration::from_secs(1));
        assert_eq!(result1.layer_commands.len(), 1);
        assert_eq!(
            result1.layer_commands[0].command_type,
            LayerCommandType::Freeze
        );
        assert_eq!(
            result1.layer_commands[0].layer,
            Some(EffectLayer::Background)
        );

        // Third cue: master command with intensity and speed
        let result2 = timeline.update(Duration::from_secs(2));
        assert_eq!(result2.layer_commands.len(), 1);
        assert_eq!(
            result2.layer_commands[0].command_type,
            LayerCommandType::Master
        );
        assert!((result2.layer_commands[0].intensity.unwrap() - 0.5).abs() < 0.01);
        assert!((result2.layer_commands[0].speed.unwrap() - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_timeline_mixed_effects_and_layer_commands() {
        use crate::lighting::effects::EffectLayer;
        use crate::lighting::parser::{Cue, LayerCommand, LayerCommandType};

        // Create cue with both an effect and a layer command
        let effect = Effect {
            sequence_name: None,
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(5),
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![Cue {
            stop_sequences: vec![],
            start_sequences: vec![],
            time: Duration::from_secs(0),
            effects: vec![effect],
            layer_commands: vec![LayerCommand {
                command_type: LayerCommandType::Master,
                layer: Some(EffectLayer::Background),
                intensity: Some(0.75),
                speed: None,
            }],
        }];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        let result = timeline.update(Duration::from_secs(0));

        // Should have both an effect and a layer command
        assert_eq!(result.effects.len(), 1);
        assert_eq!(result.layer_commands.len(), 1);
        assert_eq!(
            result.layer_commands[0].command_type,
            LayerCommandType::Master
        );
    }

    #[test]
    fn test_timeline_start_at() {
        use crate::lighting::parser::Cue;

        let effect = Effect {
            sequence_name: None,
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(5),
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            Cue {
                time: Duration::from_secs(0),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(5),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(10),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        // Start at 5 seconds - should skip the first cue
        let _historical_update = timeline.start_at(Duration::from_secs(5));
        assert!(timeline.is_playing);
        assert_eq!(timeline.current_time, Duration::from_secs(5));

        // Update at 5 seconds - should trigger the cue at 5s
        let result = timeline.update(Duration::from_secs(5));
        assert_eq!(result.effects.len(), 1);

        // Update at 10 seconds - should trigger the cue at 10s
        let result = timeline.update(Duration::from_secs(10));
        assert_eq!(result.effects.len(), 1);
    }

    #[test]
    fn test_timeline_find_cue_index_at() {
        use crate::lighting::parser::Cue;

        let effect = Effect {
            sequence_name: None,
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(5),
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            Cue {
                time: Duration::from_secs(0),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(5),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(10),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        // Test finding cue index at different times
        timeline.start_at(Duration::from_secs(0));
        assert_eq!(timeline.next_cue_index, 0);

        timeline.start_at(Duration::from_secs(3));
        // Should point to cue at 5s (first cue >= 3s)
        assert_eq!(timeline.next_cue_index, 1);

        timeline.start_at(Duration::from_secs(5));
        assert_eq!(timeline.next_cue_index, 1);

        timeline.start_at(Duration::from_secs(7));
        // Should point to cue at 10s (first cue >= 7s)
        assert_eq!(timeline.next_cue_index, 2);

        timeline.start_at(Duration::from_secs(15));
        // Should point past the end
        assert_eq!(timeline.next_cue_index, 3);
    }

    #[test]
    fn test_timeline_cue_listing() {
        use crate::lighting::parser::Cue;

        let effect = Effect {
            sequence_name: None,
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(5),
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            Cue {
                time: Duration::from_secs(0),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(5),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            Cue {
                time: Duration::from_secs(10),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let timeline = LightingTimeline::new_with_cues(cues);

        // Test cue listing
        let cue_list = timeline.cues();
        assert_eq!(cue_list.len(), 3);
        assert_eq!(cue_list[0], (Duration::from_secs(0), 0));
        assert_eq!(cue_list[1], (Duration::from_secs(5), 1));
        assert_eq!(cue_list[2], (Duration::from_secs(10), 2));
    }

    /// Seeking past a cue drops its finished effects — except a move,
    /// which is replayed with its full elapsed time so the engine can
    /// commit the head to where the move ended.
    #[test]
    fn start_at_replays_a_finished_move_but_not_a_finished_static() {
        let shows = crate::lighting::parser::parse_light_shows(
            "show \"s\" {\n    @00:01.000\n    spots: static red: 100%, duration: 2s\n    \
             @00:02.000\n    spots: move pan: 45deg, tilt: -20deg, duration: 500ms\n}\n",
        )
        .unwrap()
        .into_values()
        .collect();
        let mut timeline = LightingTimeline::new(shows);
        let update = timeline.start_at(Duration::from_secs(10));
        let kinds: Vec<(&'static str, Duration)> = update
            .effects_with_elapsed
            .values()
            .map(|(effect, elapsed)| (effect.effect_type.name(), *elapsed))
            .collect();
        assert_eq!(
            kinds,
            vec![("Move", Duration::from_secs(8))],
            "only the move survives the seek, carrying its elapsed time"
        );
    }

    /// The seek's history judges a move arrived at the speed it ran at
    /// (#589): on a slowed layer it is still on its way past its length in
    /// score time, and one that arrived at double speed stays arrived after
    /// a layer clear puts the speed back.
    #[test]
    fn start_at_judges_a_move_at_its_layers_speed() {
        let elapsed = |src: &str, millis: u64| -> Vec<Duration> {
            let shows = crate::lighting::parser::parse_light_shows(src)
                .unwrap()
                .into_values()
                .collect();
            let mut timeline = LightingTimeline::new(shows);
            timeline
                .start_at(Duration::from_millis(millis))
                .effects_with_elapsed
                .values()
                .map(|(_, elapsed)| *elapsed)
                .collect()
        };
        // Half speed: 4 s of move, 6 s in, is 3 s along.
        assert_eq!(
            elapsed(
                "show \"s\" {\n    @00:00.000\n    master(layer: background, speed: 0.5)\n    \
                 spots: move pan: 45deg, tilt: -20deg, duration: 4s\n}\n",
                6_000
            ),
            vec![Duration::from_secs(6)],
            "the engine scales it to 3 s of 4"
        );
        // Double speed: arrived at 2 s; cleared at 3 s, which puts the
        // speed back, and still there at 3.5 s.
        assert_eq!(
            elapsed(
                "show \"s\" {\n    @00:00.000\n    master(layer: background, speed: 2)\n    \
                 spots: move pan: 45deg, tilt: -20deg, duration: 4s\n    @00:03.000\n    \
                 clear(layer: background)\n}\n",
                3_500
            ),
            vec![Duration::MAX],
            "the move ran again after it had arrived"
        );
    }

    /// Live, a layer clear or a stopped sequence leaves a head where it
    /// is; only a full `clear` releases pose memory. The seek replay
    /// keeps the same distinction.
    #[test]
    fn start_at_keeps_a_finished_move_through_a_layer_clear_but_not_a_full_clear() {
        let parse = |src: &str| {
            crate::lighting::parser::parse_light_shows(src)
                .unwrap()
                .into_values()
                .collect::<Vec<_>>()
        };
        let layer_clear = parse(
            "show \"s\" {\n    @00:01.000\n    spots: move pan: 45deg, tilt: -20deg, duration: 500ms\n    \
             @00:03.000\n    clear(layer: background)\n}\n",
        );
        let mut timeline = LightingTimeline::new(layer_clear);
        let update = timeline.start_at(Duration::from_secs(10));
        assert_eq!(
            update.effects_with_elapsed.len(),
            1,
            "a layer clear does not release the pose, so the move is replayed"
        );

        let full_clear = parse(
            "show \"s\" {\n    @00:01.000\n    spots: move pan: 45deg, tilt: -20deg, duration: 500ms\n    \
             @00:03.000\n    clear()\n}\n",
        );
        let mut timeline = LightingTimeline::new(full_clear);
        let update = timeline.start_at(Duration::from_secs(10));
        assert!(
            update.effects_with_elapsed.is_empty(),
            "a full clear releases the pose, so nothing is replayed"
        );
    }

    #[test]
    fn test_start_at_clears_purge_old_effects() {
        // Verify that seeking past a clear() command does NOT include
        // effects from before the clear in the historical update.
        use crate::lighting::effects::EffectLayer;
        use crate::lighting::parser::{Cue, LayerCommand, LayerCommandType};

        let bg_effect = Effect {
            sequence_name: None,
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(60),
            },
            layer: Some(EffectLayer::Background),
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let fg_effect = Effect {
            sequence_name: None,
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(60),
            },
            layer: Some(EffectLayer::Foreground),
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            // @0s: start perpetual effects on bg and fg
            Cue {
                time: Duration::from_secs(0),
                effects: vec![bg_effect.clone(), fg_effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            // @5s: clear foreground only
            Cue {
                time: Duration::from_secs(5),
                effects: vec![],
                layer_commands: vec![LayerCommand {
                    command_type: LayerCommandType::Clear,
                    layer: Some(EffectLayer::Foreground),
                    intensity: None,
                    speed: None,
                }],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            // @10s: start a new fg effect
            Cue {
                time: Duration::from_secs(10),
                effects: vec![fg_effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        // Seek to 12s — past the clear at 5s and the new effect at 10s
        let update = timeline.start_at(Duration::from_secs(12));

        // Should have 2 effects: the bg from @0s and the new fg from @10s
        // The old fg from @0s should have been purged by the clear at @5s
        assert_eq!(
            update.effects_with_elapsed.len(),
            2,
            "Should have 2 effects (bg from @0s + new fg from @10s), got {}: {:?}",
            update.effects_with_elapsed.len(),
            update
                .effects_with_elapsed
                .values()
                .map(|(e, _)| &e.id)
                .collect::<Vec<_>>()
        );

        // Verify the bg effect is present with 12s elapsed
        let bg = update
            .effects_with_elapsed
            .values()
            .find(|(e, _)| e.layer == EffectLayer::Background);
        assert!(bg.is_some(), "Background effect should be present");
        assert_eq!(bg.unwrap().1, Duration::from_secs(12));

        // Verify the new fg effect is present with 2s elapsed (started at @10s)
        let fg = update
            .effects_with_elapsed
            .values()
            .find(|(e, _)| e.layer == EffectLayer::Foreground);
        assert!(fg.is_some(), "Foreground effect should be present");
        assert_eq!(fg.unwrap().1, Duration::from_secs(2));
    }

    #[test]
    fn test_start_at_clear_all_purges_all_effects() {
        // Verify that seeking past a clear() (all layers) purges everything
        use crate::lighting::parser::{Cue, LayerCommand, LayerCommandType};

        let effect = Effect {
            sequence_name: None,
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(60),
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            // @0s: start effects
            Cue {
                time: Duration::from_secs(0),
                effects: vec![effect.clone(), effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            // @5s: clear all layers
            Cue {
                time: Duration::from_secs(5),
                effects: vec![],
                layer_commands: vec![LayerCommand {
                    command_type: LayerCommandType::Clear,
                    layer: None,
                    intensity: None,
                    speed: None,
                }],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            // @10s: start one new effect
            Cue {
                time: Duration::from_secs(10),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        let update = timeline.start_at(Duration::from_secs(12));

        // Only the effect from @10s should survive (the 2 from @0s were cleared at @5s)
        assert_eq!(
            update.effects_with_elapsed.len(),
            1,
            "Should have 1 effect after clear-all, got {}",
            update.effects_with_elapsed.len()
        );
    }

    #[test]
    fn test_start_at_stopped_sequences_suppresses_future_effects() {
        // Verify that seeking past a stop_sequence command suppresses
        // effects from that sequence at and after the stop point.
        use crate::lighting::parser::Cue;

        let seq_effect = |seq: &str| Effect {
            sequence_name: Some(seq.to_string()),
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(60),
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            // @0s: start effects from sequence A and B
            Cue {
                time: Duration::from_secs(0),
                effects: vec![seq_effect("seqA"), seq_effect("seqB")],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
            // @5s: explicitly stop sequence A (no new effects from A in this cue)
            Cue {
                time: Duration::from_secs(5),
                effects: vec![],
                layer_commands: vec![],
                stop_sequences: vec!["seqA".to_string()],
                start_sequences: vec![],
            },
            // @8s: new effect from sequence A — should be suppressed
            Cue {
                time: Duration::from_secs(8),
                effects: vec![seq_effect("seqA")],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        // Seek to 10s — past the stop at 5s and the suppressed effect at 8s
        let update = timeline.start_at(Duration::from_secs(10));

        // Should have 1 effect: seqB from @0s (seqA was stopped at @5s, so
        // both the @0s seqA effect and the @8s seqA effect are excluded)
        let seq_names: Vec<_> = update
            .effects_with_elapsed
            .values()
            .filter_map(|(e, _)| e.id.split("_effect_").next())
            .collect();
        assert_eq!(
            update.effects_with_elapsed.len(),
            1,
            "Should have 1 effect (seqB only), got {}: {:?}",
            update.effects_with_elapsed.len(),
            seq_names
        );
        // The surviving effect should be from seqB
        let surviving = update.effects_with_elapsed.values().next().unwrap();
        assert!(
            surviving.0.id.starts_with("seq_seqB_"),
            "Surviving effect should be from seqB, got: {}",
            surviving.0.id
        );
    }

    #[test]
    fn test_start_at_iteration_boundary_does_not_suppress() {
        // Verify that iteration-boundary stops (stop + start in same cue)
        // do NOT suppress the sequence — only explicit stops do.
        use crate::lighting::parser::Cue;

        let seq_effect = |seq: &str| Effect {
            sequence_name: Some(seq.to_string()),
            ignored_parameters: Vec::new(),
            tempo_offset_secs: 0.0,
            musical: Default::default(),
            per_cell: false,
            spread: 0.0,
            groups: vec!["test_group".to_string()],
            effect_type: EffectType::Static {
                parameters: HashMap::new(),
                duration: Duration::from_secs(60),
            },
            layer: None,
            blend_mode: None,
            up_time: None,
            hold_time: None,
            down_time: None,
        };

        let cues = vec![
            // @0s: start effects from seqA (iteration 1)
            Cue {
                time: Duration::from_secs(0),
                effects: vec![seq_effect("seqA")],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec!["seqA".to_string()],
            },
            // @5s: iteration boundary — stop old seqA, start new seqA effects
            Cue {
                time: Duration::from_secs(5),
                effects: vec![seq_effect("seqA")],
                layer_commands: vec![],
                stop_sequences: vec!["seqA".to_string()],
                start_sequences: vec!["seqA".to_string()],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        // Seek to 7s — past the iteration boundary at 5s
        let update = timeline.start_at(Duration::from_secs(7));

        // Should have 1 effect: the seqA from @5s (iteration boundary didn't suppress)
        assert_eq!(
            update.effects_with_elapsed.len(),
            1,
            "Should have 1 effect (seqA iteration 2), got {}",
            update.effects_with_elapsed.len()
        );
        let surviving = update.effects_with_elapsed.values().next().unwrap();
        assert!(surviving.0.id.starts_with("seq_seqA_"));
        assert_eq!(surviving.1, Duration::from_secs(2)); // 7s - 5s = 2s elapsed
    }
}

#[cfg(test)]
#[path = "timeline_replay_tests.rs"]
mod replay_tests;
