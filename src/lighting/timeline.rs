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

use crate::lighting::effects::Color;
use crate::lighting::engine::Replay;
use crate::lighting::{
    parser::{Cue, Effect, LayerCommand, LightShow},
    EffectEngine, EffectInstance,
};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// Global atomic counter for generating unique, deterministic effect IDs.
static EFFECT_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Moves the effect id counter on to `short_by` short of the next power of
/// ten at least that far above it, and returns that power: a test can put
/// two cues' ids either side of it (#669). Never back: other tests take ids
/// from the counter at the same time, and must not be given ones already
/// out.
#[cfg(test)]
pub(crate) fn advance_effect_id_counter_short_of_power_of_ten(short_by: u64) -> u64 {
    let mut boundary = 0;
    EFFECT_ID_COUNTER
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |current| {
            boundary = 10;
            while boundary - short_by < current {
                boundary = boundary.checked_mul(10)?;
            }
            Some(boundary - short_by)
        })
        .expect("the effect id counter has passed every power of ten");
    boundary
}

/// One thing a cue does, as the engine applies it.
// Most steps are effects started: boxing them would only add an allocation
// per effect.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum Step {
    /// A layer command: a clear, a freeze or unfreeze, a master.
    Command(LayerCommand),
    /// A `stop sequence`: the effects cued from that sequence go.
    Stop(String),
    /// An effect cued.
    Start(EffectInstance),
}

/// What the timeline fired: each step with its cue's song time, in the
/// order the engine applies them (see `fire`).
#[derive(Debug, Clone, Default)]
pub struct TimelineUpdate {
    pub steps: Vec<(Duration, Step)>,
}

impl TimelineUpdate {
    /// Passes every effect through `f`: resolving their groups, before the
    /// update goes to an engine.
    pub fn map_effects(&mut self, mut f: impl FnMut(EffectInstance) -> EffectInstance) {
        self.steps = std::mem::take(&mut self.steps)
            .into_iter()
            .map(|(time, step)| match step {
                Step::Start(effect) => (time, Step::Start(f(effect))),
                step => (time, step),
            })
            .collect();
    }

    /// Whether the update fires nothing.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// The effects it starts, in order.
    pub fn effects(&self) -> impl Iterator<Item = &EffectInstance> {
        self.steps.iter().filter_map(|(_, step)| match step {
            Step::Start(effect) => Some(effect),
            _ => None,
        })
    }

    /// Its layer commands, each with its cue's time, in order.
    pub fn layer_commands(&self) -> impl Iterator<Item = (Duration, &LayerCommand)> {
        self.steps.iter().filter_map(|(time, step)| match step {
            Step::Command(cmd) => Some((*time, cmd)),
            _ => None,
        })
    }

    /// Its sequence stops, each with its cue's time, in order.
    pub fn stop_sequences(&self) -> impl Iterator<Item = (Duration, &str)> {
        self.steps.iter().filter_map(|(time, step)| match step {
            Step::Stop(name) => Some((*time, name.as_str())),
            _ => None,
        })
    }
}

/// A seek's history: every step the cues before `at` take, for
/// [`EffectEngine::start_from`] to replay up to `at`. Empty for a start from
/// the top.
#[derive(Debug, Clone, Default)]
pub struct SeekHistory {
    /// The song time the timeline starts from.
    pub at: Duration,
    pub update: TimelineUpdate,
}

impl SeekHistory {
    /// See [`TimelineUpdate::map_effects`].
    pub fn map_effects(&mut self, f: impl FnMut(EffectInstance) -> EffectInstance) {
        self.update.map_effects(f);
    }
}

/// Fires one time's `cues` as the timeline does live, into `out` in the
/// order the engine applies them: all their layer commands, then all their
/// sequence stops, then their effects -- a sequence's before the show's own,
/// so a show's own effect at the same time wins a Replace conflict and a
/// show's own move takes the heads (#599) -- so a clear in one cue does not
/// take out an effect another cue starts at the same time. Cue by cue, a
/// cue's sequence starts clear their stopped status, its effects are taken
/// unless a sequence they were started through is stopped by then, and its
/// stops then count for the cues after it: an iteration-boundary stop
/// (a pass restart, or where the cue also starts effects from the same
/// sequence) only goes to the engine, while an explicit stop also suppresses future cues from that
/// sequence. `fired`
/// sees each effect started, with its cue's and its own place.
fn fire<'a>(
    stopped_sequences: &mut HashSet<String>,
    cues: impl IntoIterator<Item = (usize, &'a Cue)>,
    out: &mut Vec<(Duration, Step)>,
    mut fired: impl FnMut(usize, usize, &EffectInstance),
) {
    let mut stops = Vec::new();
    let mut effects = Vec::new();
    for (index, cue) in cues {
        for seq_name in &cue.start_sequences {
            stopped_sequences.remove(seq_name);
        }
        for (place, effect) in cue.effects.iter().enumerate() {
            if effect
                .sequences
                .iter()
                .any(|name| stopped_sequences.contains(name))
            {
                continue;
            }
            let instance = LightingTimeline::create_effect_instance(effect, cue.time);
            fired(index, place, &instance);
            effects.push((cue.time, Step::Start(instance)));
        }
        let cue_sequence_names: HashSet<&String> =
            cue.effects.iter().flat_map(|e| &e.sequences).collect();
        for seq_name in &cue.stop_sequences {
            if !cue_sequence_names.contains(seq_name) && !cue.pass_restarts.contains(seq_name) {
                stopped_sequences.insert(seq_name.clone());
            }
            stops.push((cue.time, Step::Stop(seq_name.clone())));
        }
        out.extend(
            cue.layer_commands
                .iter()
                .map(|cmd| (cue.time, Step::Command(cmd.clone()))),
        );
    }
    out.extend(stops);
    effects.sort_by_key(|(_, step)| !matches!(step, Step::Start(e) if !e.sequences.is_empty()));
    out.extend(effects);
}

/// How an effect a show fires really ends (see [`effect_ends`]).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EffectEnd {
    /// The cue that fired it, by its place among the cues given.
    pub cue: usize,
    /// Its place among that cue's effects.
    pub effect: usize,
    /// When it ends: of itself (a freeze holds it past its written end, the
    /// speed master moves it), or cut short by a clear, a stop of its
    /// sequence or a move taking every head it has. `None` for one a freeze
    /// the show leaves on holds for good.
    pub end: Option<Duration>,
    /// Its written end: its cue's time and its duration.
    pub written: Duration,
}

impl EffectEnd {
    /// When it is over: its end, or for one held for good the end it was
    /// written with.
    pub fn until(&self) -> Duration {
        self.end.unwrap_or(self.written)
    }
}

/// How each effect `cues` fire really ends: the cues fired as the timeline
/// fires them and replayed through a rigless engine (see
/// [`EffectEngine::reckoner`]) by the rules a seek's history replays by --
/// a freeze, the speed master, a clear, a stop and a move taking heads all
/// as the engine has them live. With no venue, groups are fixtures by name:
/// two groups that share a fixture keep both moves on it. The show's end,
/// the lint and the analysis all take an effect's end from here. An effect a
/// stopped sequence never fires has none.
pub(crate) fn effect_ends<'a>(cues: impl IntoIterator<Item = &'a Cue>) -> Vec<EffectEnd> {
    let cues: Vec<&Cue> = cues.into_iter().collect();
    // In time order, same-time cues as given, as the timeline sorts them.
    let mut order: Vec<usize> = (0..cues.len()).collect();
    order.sort_by_key(|&i| cues[i].time);
    let Some(last) = order.last().map(|&i| cues[i].time) else {
        return Vec::new();
    };
    let mut engine = EffectEngine::reckoner();
    let mut replay = Replay::new(&mut engine, last);
    let mut stopped = HashSet::new();
    let mut fired: HashMap<String, (usize, usize, Duration)> = HashMap::new();
    let mut steps = Vec::new();
    for group in order.chunk_by(|&a, &b| cues[a].time == cues[b].time) {
        fire(
            &mut stopped,
            group.iter().map(|&i| (i, cues[i])),
            &mut steps,
            |cue, effect, instance| {
                let written = cues[cue].time.saturating_add(instance.total_duration());
                fired.insert(instance.id.clone(), (cue, effect, written));
            },
        );
        replay.at(cues[group[0]].time, steps.drain(..).map(|(_, step)| step));
    }
    // Rigless, every effect starts.
    let _ = replay.finish();
    let mut ends = engine.take_ends();
    let mut out: Vec<EffectEnd> = fired
        .into_iter()
        .filter_map(|(id, (cue, effect, written))| {
            Some(EffectEnd {
                cue,
                effect,
                end: ends.remove(&id)?,
                written,
            })
        })
        .collect();
    out.sort_by_key(|end| (end.cue, end.effect));
    out
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

/// The tempo map a song's shows all play to: the first show's with a `tempo`
/// block, in the order the song registers them, else `fallback` -- the
/// song's. A later show's own block does not place its cues.
pub fn shows_tempo_map<'a>(
    shows: &'a [LightShow],
    fallback: Option<&'a crate::lighting::tempo::TempoMap>,
) -> Option<&'a crate::lighting::tempo::TempoMap> {
    shows
        .iter()
        .find_map(|show| show.tempo_map.as_ref())
        .or(fallback)
}

impl LightingTimeline {
    /// Creates a new lighting timeline from DSL light shows, their palettes
    /// at the defaults the shows declare.
    pub fn new(shows: Vec<LightShow>) -> Self {
        Self::with_palette(shows, &HashMap::new())
    }

    /// Creates a new lighting timeline from DSL light shows, their palette
    /// slots settled before the first frame: each slot is `overrides`'
    /// colour when it names the slot, else the declaring show's default. An
    /// override for a slot no show declares is ignored. The timeline's
    /// effects hold only literal colours from here on.
    pub fn with_palette(shows: Vec<LightShow>, overrides: &HashMap<String, Color>) -> Self {
        let shows = crate::lighting::parser::resolve_shows(shows, overrides);
        let tempo_map = shows_tempo_map(&shows, None).cloned();
        let all_cues = shows.into_iter().flat_map(|show| show.cues).collect();
        let mut timeline = Self::new_with_cues(all_cues);
        timeline.tempo_map = tempo_map;
        timeline
    }

    /// Creates a new lighting timeline from DSL cues (for testing)
    pub(crate) fn new_with_cues(cues: Vec<Cue>) -> Self {
        let mut timeline = Self {
            cues,
            current_time: Duration::ZERO,
            next_cue_index: 0,
            is_playing: false,
            held: false,
            tempo_map: None,
            stopped_sequences: HashSet::new(),
            epoch: next_epoch(),
            show_end: Duration::ZERO,
            end_cap: None,
            open_ended: false,
        };
        timeline.sort_cues();
        // The last effect's real end: a `clear` or a `stop sequence` cuts
        // an effect short, and a show that ends on one is over then, not
        // when the cleared bed would have run out; a freeze or the speed
        // master holds an effect past its written end, and the song's time
        // stops once the show is over, so an effect still running then
        // never ends (#590). A look held for good has no end to wait for:
        // without audio to run to, the show keeps at least the end it was
        // written with, an ambient show looping then with the frozen look
        // up until it does.
        let ends = effect_ends(&timeline.cues);
        timeline.open_ended = ends.iter().any(|end| end.end.is_none());
        timeline.show_end = ends.iter().map(EffectEnd::until).max().unwrap_or_default();
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

    /// The tempo map the show's effects play to: the shows' own
    /// ([`shows_tempo_map`]), or with no `tempo` block among them,
    /// `fallback` -- the song's.
    pub fn tempo_map_or(
        &self,
        fallback: Option<&crate::lighting::tempo::TempoMap>,
    ) -> Option<crate::lighting::tempo::TempoMap> {
        self.tempo_map.as_ref().or(fallback).cloned()
    }

    /// Sorts cues by time, stably: same-time cues keep the order they came
    /// in, their shows' file order and their place in each, and fire in it
    /// the same way at every build of the timeline (#597).
    fn sort_cues(&mut self) {
        self.cues.sort_by(|a, b| a.time.cmp(&b.time));
    }

    /// Starts the timeline from the top: [`Self::start_at`] zero, whose
    /// history is empty.
    pub fn start(&mut self) {
        let _ = self.start_at(Duration::ZERO);
    }

    /// Starts the timeline at `start_time` (for seeking): the cues from
    /// there on fire as the song plays, and the ones before are the history
    /// returned, which [`EffectEngine::start_from`] replays into the engine
    /// so the state there is what playing through would have left.
    pub fn start_at(&mut self, start_time: Duration) -> SeekHistory {
        self.epoch = next_epoch();
        self.is_playing = true;
        self.current_time = start_time;
        self.stopped_sequences.clear();
        self.next_cue_index = self.find_cue_index_at(start_time);
        let mut history = SeekHistory {
            at: start_time,
            update: TimelineUpdate::default(),
        };
        for group in self.cues[..self.next_cue_index].chunk_by(|a, b| a.time == b.time) {
            fire(
                &mut self.stopped_sequences,
                group.iter().enumerate(),
                &mut history.update.steps,
                |_, _, _| {},
            );
        }
        history
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

    /// Updates the timeline with the current song time: fires every cue due
    /// by then, time by time (see `fire`).
    pub fn update(&mut self, song_time: Duration) -> TimelineUpdate {
        if !self.is_playing || self.held {
            return TimelineUpdate::default();
        }
        self.current_time = song_time;
        let mut result = TimelineUpdate::default();

        let due = self.cues[self.next_cue_index..].partition_point(|cue| cue.time <= song_time);
        let due = &self.cues[self.next_cue_index..self.next_cue_index + due];
        for group in due.chunk_by(|a, b| a.time == b.time) {
            fire(
                &mut self.stopped_sequences,
                group.iter().enumerate(),
                &mut result.steps,
                |_, _, _| {},
            );
        }
        self.next_cue_index += due.len();

        let effects = result.effects().count();
        if effects > 20 {
            tracing::warn!(
                effects,
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

    /// When each effect the show cues ends of itself or is cut short, as the
    /// show's end reckons it (see [`effect_ends`]); one left frozen for good
    /// never does, and is not here.
    pub(crate) fn effect_ends(&self) -> Vec<Duration> {
        effect_ends(&self.cues)
            .into_iter()
            .filter_map(|end| end.end)
            .collect()
    }

    /// Each cue's time and how many effects it starts, in firing order.
    pub fn cue_effect_counts(&self) -> Vec<(Duration, usize)> {
        self.cues
            .iter()
            .map(|cue| (cue.time, cue.effects.len()))
            .collect()
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
        let effect_id = if let Some(seq_name) = effect.sequence_name() {
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
        effect_instance.sequences = effect.sequences.clone();

        effect_instance
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::effects::EffectType;
    use crate::lighting::engine::StopScope;
    use crate::lighting::parser::Effect;
    use std::collections::HashMap;

    /// What a seek to `at` leaves running, each effect with its own elapsed
    /// there: the history replayed into a rigless engine, as
    /// `EffectEngine::start_from` replays it.
    fn sought(
        timeline: &mut LightingTimeline,
        at: Duration,
    ) -> HashMap<String, (EffectInstance, Duration)> {
        let mut engine = EffectEngine::reckoner();
        let _ = engine.start_from(timeline.start_at(at), StopScope::Nothing);
        engine
            .get_active_effects()
            .iter()
            .map(|(id, e)| (id.clone(), (e.clone(), engine.elapsed_at(e, Some(at)))))
            .collect()
    }

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
            sequences: Vec::new(),
            ignored_parameters: Vec::new(),
            color_refs: Vec::new(),
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
            pass_restarts: vec![],
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
        // From 1 s into a 4 s look, at half speed: its 3 s left take 6 s.
        assert_eq!(slowed.show_end(), secs(7.0), "slowed to half at 1 s");

        let quick = end("    @00:01.000\n    master(layer: midground, speed: 2)\n");
        // Its 3 s left at double speed take 1.5 s.
        assert_eq!(quick.show_end(), secs(2.5), "double speed from 1 s");

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

    /// With a look held for good, the others count by their real ends, as
    /// without one: a bed at double speed is over at 10 s, not its written
    /// 20 s, and the frozen look keeps the end it was written with.
    #[test]
    fn a_look_held_for_good_keeps_the_others_real_ends() {
        let shows = crate::lighting::parser::parse_light_shows(
            "show \"s\" {\n    @00:00.000\n    master(layer: midground, speed: 2)\n    \
             spots: static red: 100%, duration: 20s, layer: midground\n    \
             spots: static blue: 100%, duration: 5s, layer: foreground\n    \
             @00:01.000\n    freeze(layer: foreground)\n}\n",
        )
        .unwrap()
        .into_values()
        .collect();
        let timeline = LightingTimeline::new(shows);
        assert_eq!(timeline.show_end(), Duration::from_secs(10));
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
        let update = sought(&mut timeline, Duration::from_secs(5));
        assert_eq!(update.len(), 1, "a seek keeps it too");
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
            sequences: Vec::new(),
            ignored_parameters: Vec::new(),
            color_refs: Vec::new(),
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
            pass_restarts: vec![],
            time: Duration::from_millis(0),
            effects: vec![effect],
            layer_commands: vec![],
        }];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        // Test that the first cue triggers at the right time
        let result = timeline.update(Duration::from_millis(0));
        assert_eq!(result.effects().count(), 1);
        assert_eq!(
            result.effects().next().unwrap().target_fixtures,
            vec!["front_wash"]
        );
    }

    #[test]
    fn test_timeline_cue_ordering() {
        use crate::lighting::parser::Cue;

        // Create cues in non-chronological order
        let mut parameters = HashMap::new();
        parameters.insert("color".to_string(), "blue".to_string());

        let effect = Effect {
            sequences: Vec::new(),
            ignored_parameters: Vec::new(),
            color_refs: Vec::new(),
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
                pass_restarts: vec![],
            },
            Cue {
                time: Duration::from_millis(5000),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
                pass_restarts: vec![],
            },
            Cue {
                time: Duration::from_millis(0),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
                pass_restarts: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        // Verify cues are processed in chronological order
        let result = timeline.update(Duration::from_millis(0));
        assert_eq!(result.effects().count(), 1);

        let result = timeline.update(Duration::from_millis(5000));
        assert_eq!(result.effects().count(), 1);

        let result = timeline.update(Duration::from_millis(10000));
        assert_eq!(result.effects().count(), 1);
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
            sequences: Vec::new(),
            ignored_parameters: Vec::new(),
            color_refs: Vec::new(),
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
                pass_restarts: vec![],
            },
            Cue {
                time: Duration::from_millis(5000),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
                pass_restarts: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        // Both cues should trigger at the same time
        let result = timeline.update(Duration::from_millis(5000));
        assert_eq!(result.effects().count(), 2);
    }

    #[test]
    fn test_timeline_stop_reset() {
        let mut parameters = HashMap::new();
        parameters.insert("color".to_string(), "red".to_string());

        let effect1 = Effect {
            sequences: Vec::new(),
            ignored_parameters: Vec::new(),
            color_refs: Vec::new(),
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
            sequences: Vec::new(),
            ignored_parameters: Vec::new(),
            color_refs: Vec::new(),
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
                pass_restarts: vec![],
            },
            Cue {
                time: Duration::from_secs(2),
                effects: vec![effect2],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
                pass_restarts: vec![],
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
        assert_eq!(result_at_0s_restart.effects().count(), 1);

        // Should also trigger the second cue again
        let result_at_2s_restart = timeline.update(Duration::from_secs(2));
        assert_eq!(result_at_2s_restart.effects().count(), 1);
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
                pass_restarts: vec![],
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
                pass_restarts: vec![],
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
                pass_restarts: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);
        timeline.start();

        // First cue: clear command
        let result0 = timeline.update(Duration::from_secs(0));
        assert_eq!(result0.effects().count(), 0);
        assert_eq!(result0.layer_commands().count(), 1);
        assert_eq!(
            result0.layer_commands().next().unwrap().1.command_type,
            LayerCommandType::Clear
        );
        assert_eq!(
            result0.layer_commands().next().unwrap().1.layer,
            Some(EffectLayer::Foreground)
        );

        // Second cue: freeze command
        let result1 = timeline.update(Duration::from_secs(1));
        assert_eq!(result1.layer_commands().count(), 1);
        assert_eq!(
            result1.layer_commands().next().unwrap().1.command_type,
            LayerCommandType::Freeze
        );
        assert_eq!(
            result1.layer_commands().next().unwrap().1.layer,
            Some(EffectLayer::Background)
        );

        // Third cue: master command with intensity and speed
        let result2 = timeline.update(Duration::from_secs(2));
        assert_eq!(result2.layer_commands().count(), 1);
        assert_eq!(
            result2.layer_commands().next().unwrap().1.command_type,
            LayerCommandType::Master
        );
        assert!(
            (result2
                .layer_commands()
                .next()
                .unwrap()
                .1
                .intensity
                .unwrap()
                - 0.5)
                .abs()
                < 0.01
        );
        assert!((result2.layer_commands().next().unwrap().1.speed.unwrap() - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_timeline_mixed_effects_and_layer_commands() {
        use crate::lighting::effects::EffectLayer;
        use crate::lighting::parser::{Cue, LayerCommand, LayerCommandType};

        // Create cue with both an effect and a layer command
        let effect = Effect {
            sequences: Vec::new(),
            ignored_parameters: Vec::new(),
            color_refs: Vec::new(),
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
            pass_restarts: vec![],
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
        assert_eq!(result.effects().count(), 1);
        assert_eq!(result.layer_commands().count(), 1);
        assert_eq!(
            result.layer_commands().next().unwrap().1.command_type,
            LayerCommandType::Master
        );
    }

    #[test]
    fn test_timeline_start_at() {
        use crate::lighting::parser::Cue;

        let effect = Effect {
            sequences: Vec::new(),
            ignored_parameters: Vec::new(),
            color_refs: Vec::new(),
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
                pass_restarts: vec![],
            },
            Cue {
                time: Duration::from_secs(5),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
                pass_restarts: vec![],
            },
            Cue {
                time: Duration::from_secs(10),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
                pass_restarts: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        // Start at 5 seconds - should skip the first cue
        let _historical_update = timeline.start_at(Duration::from_secs(5));
        assert!(timeline.is_playing);
        assert_eq!(timeline.current_time, Duration::from_secs(5));

        // Update at 5 seconds - should trigger the cue at 5s
        let result = timeline.update(Duration::from_secs(5));
        assert_eq!(result.effects().count(), 1);

        // Update at 10 seconds - should trigger the cue at 10s
        let result = timeline.update(Duration::from_secs(10));
        assert_eq!(result.effects().count(), 1);
    }

    #[test]
    fn test_timeline_find_cue_index_at() {
        use crate::lighting::parser::Cue;

        let effect = Effect {
            sequences: Vec::new(),
            ignored_parameters: Vec::new(),
            color_refs: Vec::new(),
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
                pass_restarts: vec![],
            },
            Cue {
                time: Duration::from_secs(5),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
                pass_restarts: vec![],
            },
            Cue {
                time: Duration::from_secs(10),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
                pass_restarts: vec![],
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
            sequences: Vec::new(),
            ignored_parameters: Vec::new(),
            color_refs: Vec::new(),
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
                pass_restarts: vec![],
            },
            Cue {
                time: Duration::from_secs(5),
                effects: vec![effect.clone()],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
                pass_restarts: vec![],
            },
            Cue {
                time: Duration::from_secs(10),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
                pass_restarts: vec![],
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

    /// Seeking past a cue leaves none of its finished effects running: a
    /// finished move's pose is committed instead (see `move_seek_tests`).
    #[test]
    fn a_seek_past_finished_effects_leaves_none_running() {
        let shows = crate::lighting::parser::parse_light_shows(
            "show \"s\" {\n    @00:01.000\n    spots: static red: 100%, duration: 2s\n    \
             @00:02.000\n    spots: move pan: 45deg, tilt: -20deg, duration: 500ms\n}\n",
        )
        .unwrap()
        .into_values()
        .collect();
        let mut timeline = LightingTimeline::new(shows);
        assert!(sought(&mut timeline, Duration::from_secs(10)).is_empty());
    }

    /// The seek's history runs a move at the speed it ran at (#589): on a
    /// slowed layer it is still on its way past its length in score time,
    /// and one that arrived at double speed stays over after a layer clear
    /// puts the speed back.
    #[test]
    fn a_seek_runs_a_move_at_its_layers_speed() {
        let elapsed = |src: &str, millis: u64| -> Vec<Duration> {
            let shows = crate::lighting::parser::parse_light_shows(src)
                .unwrap()
                .into_values()
                .collect();
            let mut timeline = LightingTimeline::new(shows);
            sought(&mut timeline, Duration::from_millis(millis))
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
            vec![Duration::from_secs(3)],
        );
        // Double speed: arrived at 2 s; cleared at 3 s, which puts the
        // speed back, and still over at 3.5 s.
        assert_eq!(
            elapsed(
                "show \"s\" {\n    @00:00.000\n    master(layer: background, speed: 2)\n    \
                 spots: move pan: 45deg, tilt: -20deg, duration: 4s\n    @00:03.000\n    \
                 clear(layer: background)\n}\n",
                3_500
            ),
            Vec::<Duration>::new(),
            "the move ran again after it had arrived"
        );
    }

    #[test]
    fn test_start_at_clears_purge_old_effects() {
        // Verify that seeking past a clear() command does NOT include
        // effects from before the clear in the historical update.
        use crate::lighting::effects::EffectLayer;
        use crate::lighting::parser::{Cue, LayerCommand, LayerCommandType};

        let bg_effect = Effect {
            sequences: Vec::new(),
            ignored_parameters: Vec::new(),
            color_refs: Vec::new(),
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
            sequences: Vec::new(),
            ignored_parameters: Vec::new(),
            color_refs: Vec::new(),
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
                pass_restarts: vec![],
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
                pass_restarts: vec![],
            },
            // @10s: start a new fg effect
            Cue {
                time: Duration::from_secs(10),
                effects: vec![fg_effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
                pass_restarts: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        // Seek to 12s — past the clear at 5s and the new effect at 10s
        let update = sought(&mut timeline, Duration::from_secs(12));

        // Should have 2 effects: the bg from @0s and the new fg from @10s
        // The old fg from @0s should have been purged by the clear at @5s
        assert_eq!(
            update.len(),
            2,
            "Should have 2 effects (bg from @0s + new fg from @10s), got {}: {:?}",
            update.len(),
            update.values().map(|(e, _)| &e.id).collect::<Vec<_>>()
        );

        // Verify the bg effect is present with 12s elapsed
        let bg = update
            .values()
            .find(|(e, _)| e.layer == EffectLayer::Background);
        assert!(bg.is_some(), "Background effect should be present");
        assert_eq!(bg.unwrap().1, Duration::from_secs(12));

        // Verify the new fg effect is present with 2s elapsed (started at @10s)
        let fg = update
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
            sequences: Vec::new(),
            ignored_parameters: Vec::new(),
            color_refs: Vec::new(),
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
                pass_restarts: vec![],
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
                pass_restarts: vec![],
            },
            // @10s: start one new effect
            Cue {
                time: Duration::from_secs(10),
                effects: vec![effect],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
                pass_restarts: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        let update = sought(&mut timeline, Duration::from_secs(12));

        // Only the effect from @10s should survive (the 2 from @0s were cleared at @5s)
        assert_eq!(
            update.len(),
            1,
            "Should have 1 effect after clear-all, got {}",
            update.len()
        );
    }

    #[test]
    fn test_start_at_stopped_sequences_suppresses_future_effects() {
        // Verify that seeking past a stop_sequence command suppresses
        // effects from that sequence at and after the stop point.
        use crate::lighting::parser::Cue;

        let seq_effect = |seq: &str| Effect {
            sequences: vec![seq.to_string()],
            ignored_parameters: Vec::new(),
            color_refs: Vec::new(),
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
                pass_restarts: vec![],
            },
            // @5s: explicitly stop sequence A (no new effects from A in this cue)
            Cue {
                time: Duration::from_secs(5),
                effects: vec![],
                layer_commands: vec![],
                stop_sequences: vec!["seqA".to_string()],
                start_sequences: vec![],
                pass_restarts: vec![],
            },
            // @8s: new effect from sequence A — should be suppressed
            Cue {
                time: Duration::from_secs(8),
                effects: vec![seq_effect("seqA")],
                layer_commands: vec![],
                stop_sequences: vec![],
                start_sequences: vec![],
                pass_restarts: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        // Seek to 10s — past the stop at 5s and the suppressed effect at 8s
        let update = sought(&mut timeline, Duration::from_secs(10));

        // Should have 1 effect: seqB from @0s (seqA was stopped at @5s, so
        // both the @0s seqA effect and the @8s seqA effect are excluded)
        let seq_names: Vec<_> = update
            .values()
            .filter_map(|(e, _)| e.id.split("_effect_").next())
            .collect();
        assert_eq!(
            update.len(),
            1,
            "Should have 1 effect (seqB only), got {}: {:?}",
            update.len(),
            seq_names
        );
        // The surviving effect should be from seqB
        let surviving = update.values().next().unwrap();
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
            sequences: vec![seq.to_string()],
            ignored_parameters: Vec::new(),
            color_refs: Vec::new(),
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
                pass_restarts: vec![],
            },
            // @5s: iteration boundary — stop old seqA, start new seqA effects
            Cue {
                time: Duration::from_secs(5),
                effects: vec![seq_effect("seqA")],
                layer_commands: vec![],
                stop_sequences: vec!["seqA".to_string()],
                start_sequences: vec!["seqA".to_string()],
                pass_restarts: vec![],
            },
        ];

        let mut timeline = LightingTimeline::new_with_cues(cues);

        // Seek to 7s — past the iteration boundary at 5s
        let update = sought(&mut timeline, Duration::from_secs(7));

        // Should have 1 effect: the seqA from @5s (iteration boundary didn't suppress)
        assert_eq!(
            update.len(),
            1,
            "Should have 1 effect (seqA iteration 2), got {}",
            update.len()
        );
        let surviving = update.values().next().unwrap();
        assert!(surviving.0.id.starts_with("seq_seqA_"));
        assert_eq!(surviving.1, Duration::from_secs(2)); // 7s - 5s = 2s elapsed
    }
}

#[cfg(test)]
#[path = "timeline_replay_tests.rs"]
mod replay_tests;
