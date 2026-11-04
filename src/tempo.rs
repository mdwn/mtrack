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

/// Default BPM used when no tempo map is available.
pub const DEFAULT_BPM: f64 = 120.0;

/// Time signature (numerator/denominator)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimeSignature {
    pub numerator: u32,
    pub denominator: u32,
}

impl TimeSignature {
    pub fn new(numerator: u32, denominator: u32) -> Self {
        TimeSignature {
            numerator,
            denominator,
        }
    }

    /// Get beats per measure, normalized to quarter-note beats.
    /// For example, 4/4 returns 4.0, 6/8 returns 3.0 (six eighth-notes = three quarter-notes).
    pub fn beats_per_measure(&self) -> f64 {
        self.numerator as f64 * 4.0 / self.denominator as f64
    }
}

/// Tempo transition curve type
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransitionCurve {
    /// Linear interpolation: bpm(t) = old_bpm + (new_bpm - old_bpm) * t
    Linear,
    // Future: EaseIn, EaseOut, EaseInOut, etc.
}

impl TransitionCurve {
    /// Get BPM at normalized time t (0.0 to 1.0) during transition
    pub fn bpm_at(&self, t: f64, old_bpm: f64, new_bpm: f64) -> f64 {
        match self {
            TransitionCurve::Linear => {
                let t = t.clamp(0.0, 1.0);
                old_bpm + (new_bpm - old_bpm) * t
            }
        }
    }

    /// Calculate how many beats occur in the remaining portion of a transition
    /// from elapsed to total_duration
    pub fn beats_in_remaining_transition(
        &self,
        old_bpm: f64,
        new_bpm: f64,
        total_duration: f64,
        elapsed: f64,
    ) -> f64 {
        match self {
            TransitionCurve::Linear => {
                // beats = integral(bpm(t)/60 dt) from elapsed to total_duration
                // = (1/60) * [old_bpm * (total - elapsed) + (new_bpm - old_bpm) * (total^2 - elapsed^2) / (2*T)]
                (old_bpm * (total_duration - elapsed)
                    + (new_bpm - old_bpm) * (total_duration * total_duration - elapsed * elapsed)
                        / (2.0 * total_duration))
                    / 60.0
            }
        }
    }

    /// Solve for duration dt given a number of beats, starting from elapsed time into the transition
    /// Returns the time duration needed for the given number of beats, or
    /// `None` where the curve has no answer: a tempo of zero, or more beats
    /// than the curve reaches (a ramp down ends before them; the caller
    /// settles at the new tempo from there).
    pub fn solve_duration_for_beats(
        &self,
        old_bpm: f64,
        new_bpm: f64,
        total_duration: f64,
        elapsed: f64,
        beats: f64,
    ) -> Option<f64> {
        match self {
            TransitionCurve::Linear => {
                // beats = integral(bpm(t)/60 dt) from elapsed to (elapsed + dt)
                // = (1/60) * [old_bpm * dt + (new_bpm - old_bpm) * (dt^2 + 2*elapsed*dt) / (2*T)]
                // Rearranging to quadratic: a*dt^2 + b*dt + c = 0
                let a = (new_bpm - old_bpm) / (2.0 * total_duration * 60.0);
                let b = (old_bpm + (new_bpm - old_bpm) * elapsed / total_duration) / 60.0;
                let c = -beats;

                if a.abs() < f64::EPSILON {
                    // Linear case (constant BPM): b * dt = beats
                    if b.abs() < f64::EPSILON {
                        None // Zero BPM — cannot solve
                    } else {
                        Some(-c / b)
                    }
                } else {
                    let discriminant = b * b - 4.0 * a * c;
                    (discriminant >= 0.0).then(|| (-b + discriminant.sqrt()) / (2.0 * a))
                }
            }
        }
    }

    /// How long a whole transition from `old_bpm` to `new_bpm` lasts when it
    /// is written to span `beats` beats: the time in which the curve covers
    /// exactly that many, as a DAW renders "over N beats". The inverse of
    /// [`TransitionCurve::beats_in_duration`] over the whole transition.
    pub fn duration_for_beats(&self, old_bpm: f64, new_bpm: f64, beats: f64) -> f64 {
        match self {
            TransitionCurve::Linear => {
                // A tempo linear in time averages (old + new) / 2 over the
                // transition, so N beats take N * 60 / that.
                let mean_bpm = (old_bpm + new_bpm) / 2.0;
                if mean_bpm > 0.0 {
                    steady_secs(mean_bpm, beats)
                } else {
                    steady_secs(old_bpm, beats)
                }
            }
        }
    }
}

/// Seconds that `beats` beats take at a steady `bpm`.
pub(crate) fn steady_secs(bpm: f64, beats: f64) -> f64 {
    beats * 60.0 / bpm
}

/// Beats covered in `secs` at a steady `bpm`. The inverse of [`steady_secs`].
pub(crate) fn steady_beats(bpm: f64, secs: f64) -> f64 {
    secs * bpm / 60.0
}

/// State of an in-progress gradual tempo transition while walking a tempo map
/// beat-by-beat. Tracks how far into the ramp the current position is so that
/// beat→time conversion integrates through the curve instead of snapping.
#[derive(Debug, Clone, Copy)]
struct SegmentRamp {
    old_bpm: f64,
    new_bpm: f64,
    total_secs: f64,
    elapsed_secs: f64,
    curve: TransitionCurve,
}

impl SegmentRamp {
    /// The ramp a tempo change to `new_bpm` starts, given the BPM actually in
    /// effect when it is reached; `None` for a snap. A transition in measures
    /// counts its length in the metre `metre` gives, asked for only then.
    fn started_by(
        transition: TempoTransition,
        bpm_at_change: f64,
        new_bpm: f64,
        metre: impl FnOnce() -> TimeSignature,
    ) -> Option<SegmentRamp> {
        let (beats, curve) = match transition {
            TempoTransition::Snap => return None,
            TempoTransition::Beats(beats, curve) => (beats, curve),
            TempoTransition::Measures(measures, curve) => {
                (measures * metre().beats_per_measure(), curve)
            }
        };
        Some(SegmentRamp {
            old_bpm: bpm_at_change,
            new_bpm,
            total_secs: curve.duration_for_beats(bpm_at_change, new_bpm, beats),
            elapsed_secs: 0.0,
            curve,
        })
    }

    /// Beats left between the current position and the end of the ramp.
    fn beats_remaining(&self) -> f64 {
        self.curve.beats_in_remaining_transition(
            self.old_bpm,
            self.new_bpm,
            self.total_secs,
            self.elapsed_secs,
        )
    }

    /// Seconds needed to advance `beats` beats from the current position,
    /// integrating through the remainder of the ramp and then continuing at
    /// `settled_bpm` once the ramp completes.
    fn secs_for_beats(&self, beats: f64, settled_bpm: f64) -> f64 {
        let remaining = self.beats_remaining();
        if beats <= remaining {
            self.curve
                .solve_duration_for_beats(
                    self.old_bpm,
                    self.new_bpm,
                    self.total_secs,
                    self.elapsed_secs,
                    beats,
                )
                .unwrap_or_else(|| steady_secs((self.old_bpm + self.new_bpm) / 2.0, beats))
        } else {
            (self.total_secs - self.elapsed_secs) + steady_secs(settled_bpm, beats - remaining)
        }
    }

    /// Beats covered in `secs` from the current position, integrating through
    /// the remainder of the ramp and then continuing at `settled_bpm`. The
    /// inverse of [`SegmentRamp::secs_for_beats`].
    fn beats_in_secs(&self, secs: f64, settled_bpm: f64) -> f64 {
        let secs_left = self.total_secs - self.elapsed_secs;
        let remaining = self.beats_remaining();
        if secs >= secs_left {
            remaining + steady_beats(settled_bpm, secs - secs_left)
        } else {
            remaining
                - self.curve.beats_in_remaining_transition(
                    self.old_bpm,
                    self.new_bpm,
                    self.total_secs,
                    self.elapsed_secs + secs,
                )
        }
    }

    /// BPM after advancing `secs` from the current position.
    fn bpm_after_secs(&self, secs: f64) -> f64 {
        let t = ((self.elapsed_secs + secs) / self.total_secs).clamp(0.0, 1.0);
        self.curve.bpm_at(t, self.old_bpm, self.new_bpm)
    }

    /// The ramp state after advancing `secs`, or `None` once it has completed.
    fn advanced_by(self, secs: f64) -> Option<SegmentRamp> {
        let elapsed = self.elapsed_secs + secs;
        if elapsed >= self.total_secs {
            None
        } else {
            Some(SegmentRamp {
                elapsed_secs: elapsed,
                ..self
            })
        }
    }
}

/// Seconds needed to advance `beats` beats given an optional in-progress ramp,
/// settling at `bpm` when no ramp is active (or after it completes).
fn segment_secs_for_beats(ramp: Option<&SegmentRamp>, bpm: f64, beats: f64) -> f64 {
    match ramp {
        Some(ramp) => ramp.secs_for_beats(beats, bpm),
        None => steady_secs(bpm, beats),
    }
}

/// Beats covered in `secs` given an optional in-progress ramp, settling at
/// `bpm` when no ramp is active (or after it completes). The inverse of
/// [`segment_secs_for_beats`].
fn segment_beats_in_secs(ramp: Option<&SegmentRamp>, bpm: f64, secs: f64) -> f64 {
    match ramp {
        Some(ramp) => ramp.beats_in_secs(secs, bpm),
        None => steady_beats(bpm, secs),
    }
}

/// A span in seconds as a `Duration`: none for a negative (or NaN) span, the
/// longest there is for one too long to hold.
fn duration_from_secs(secs: f64) -> Duration {
    Duration::try_from_secs_f64(secs.max(0.0)).unwrap_or(Duration::MAX)
}

/// A beat position rounded to a millionth of a beat.
///
/// Times are whole nanoseconds, so a position read back from a time carries
/// a few nanoseconds' worth of noise (about 3e-9 of a beat at 180 BPM). Adding
/// a length to that and converting back lands a nanosecond off the position a
/// cue computes from the exact beat, and an effect ending "on the next
/// downbeat" leaves a nanosecond gap before it (#534). A millionth of a beat
/// is far below that noise's reach yet far finer than anything musical --
/// 0.2 µs at 300 BPM -- so snapping removes the noise and nothing else.
pub(crate) fn snap_beats(beats: f64) -> f64 {
    (beats * 1e6).round() / 1e6
}

/// Tempo transition type
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TempoTransition {
    /// Instant change (snap)
    Snap,
    /// Gradual change over N beats with a curve
    Beats(f64, TransitionCurve),
    /// Gradual change over N measures with a curve
    Measures(f64, TransitionCurve),
}

/// Position where a tempo change occurs
#[derive(Debug, Clone, PartialEq)]
pub enum TempoChangePosition {
    /// Absolute time position
    Time(Duration),
    /// Measure/beat position
    MeasureBeat(u32, f64),
}

/// A tempo change at a specific position (can be measure/beat or absolute time)
#[derive(Debug, Clone, PartialEq)]
pub struct TempoChange {
    /// Position where this change occurs
    pub position: TempoChangePosition,
    /// Original measure/beat position (if this was originally specified as measure/beat)
    /// This is used to preserve measure/beat information after conversion to Time
    pub original_measure_beat: Option<(u32, f64)>,
    /// New BPM (if changed)
    pub bpm: Option<f64>,
    /// New time signature (if changed)
    pub time_signature: Option<TimeSignature>,
    /// Transition type and duration
    pub transition: TempoTransition,
}

impl TempoChangePosition {
    /// Get absolute time if this is a Time position
    pub fn absolute_time(&self) -> Option<Duration> {
        match self {
            TempoChangePosition::Time(t) => Some(*t),
            TempoChangePosition::MeasureBeat(_, _) => None,
        }
    }
}

/// Where a change is reached from the state before it: at the beat
/// position its bar gives, or at its time.
#[derive(Debug, Clone, Copy)]
enum ChangeAt {
    Beats(f64),
    Time(Duration),
}

/// Where the beat walk stands just past a change: the beat position and
/// time of the change, and the tempo from there.
#[derive(Debug, Clone, Copy)]
struct SegmentState {
    beats: f64,
    time: Duration,
    bpm: f64,
    ramp: Option<SegmentRamp>,
}

impl SegmentState {
    /// Bar 1 of `map`, at its starting tempo.
    fn at_bar_one(map: &TempoMap) -> Self {
        SegmentState {
            beats: 0.0,
            time: map.start_offset,
            bpm: map.initial_bpm,
            ramp: None,
        }
    }

    /// The state just past a change reached at `at` from this one: the
    /// change's position in beats and time, through any gradual transition
    /// still in progress, then the tempo it sets, `change_bpm` by
    /// `transition`. A ramp in measures is sized in the metre `metre` gives
    /// for the state at the change, asked for only then. A change before
    /// this state's time counts as at it, as the beat walks have it.
    fn through(
        &self,
        at: ChangeAt,
        change_bpm: Option<f64>,
        transition: TempoTransition,
        metre: impl FnOnce(&SegmentState) -> TimeSignature,
    ) -> SegmentState {
        let (beats, time, advanced_secs) = match at {
            ChangeAt::Beats(beats) => {
                let secs = segment_secs_for_beats(self.ramp.as_ref(), self.bpm, beats - self.beats);
                (
                    beats,
                    self.time.saturating_add(duration_from_secs(secs)),
                    secs,
                )
            }
            ChangeAt::Time(time) => {
                let secs = time.saturating_sub(self.time).as_secs_f64();
                let beats = self.beats + segment_beats_in_secs(self.ramp.as_ref(), self.bpm, secs);
                (beats, self.time.max(time), secs)
            }
        };
        // The BPM actually in effect at this change may be a mid-ramp value.
        let bpm_at_change = match self.ramp.as_ref() {
            Some(ramp) => ramp.bpm_after_secs(advanced_secs),
            None => self.bpm,
        };
        let mut next = SegmentState {
            beats,
            time,
            bpm: self.bpm,
            ramp: self.ramp.and_then(|ramp| ramp.advanced_by(advanced_secs)),
        };
        if let Some(new_bpm) = change_bpm {
            let ramp = SegmentRamp::started_by(transition, bpm_at_change, new_bpm, || metre(&next));
            next.ramp = ramp;
            next.bpm = new_bpm;
        }
        next
    }

    /// The time of a beat position at or past this state's, before the next.
    fn time_at(&self, beats: f64) -> Duration {
        self.time
            .saturating_add(duration_from_secs(segment_secs_for_beats(
                self.ramp.as_ref(),
                self.bpm,
                beats - self.beats,
            )))
    }

    /// The beat position of a time at or past this state's, before the
    /// next. The inverse of [`SegmentState::time_at`].
    fn beats_at(&self, time: Duration) -> f64 {
        let secs = time.saturating_sub(self.time).as_secs_f64();
        self.beats + segment_beats_in_secs(self.ramp.as_ref(), self.bpm, secs)
    }

    /// The tempo at a time at or past this state's, before the next.
    fn bpm_at(&self, time: Duration) -> f64 {
        match self.ramp.as_ref() {
            Some(ramp) => ramp.bpm_after_secs(time.saturating_sub(self.time).as_secs_f64()),
            None => self.bpm,
        }
    }
}

/// Bar order: by measure, then by beat.
fn measure_beat_order(a: (u32, f64), b: (u32, f64)) -> std::cmp::Ordering {
    a.0.cmp(&b.0)
        .then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
}

/// The time signature changes as the beat grid has them, in bar order: a
/// change anchored mid-bar governs from the next bar.
#[derive(Debug, Clone)]
struct Metres {
    initial: TimeSignature,
    changes: Vec<(u32, f64, TimeSignature)>,
}

impl Metres {
    fn new(initial: TimeSignature) -> Self {
        Metres {
            initial,
            changes: Vec::new(),
        }
    }

    /// Adds a change at a (measure, beat) position, keeping bar order.
    fn add(&mut self, measure: u32, beat: f64, signature: TimeSignature) {
        self.changes.push((measure, beat, signature));
        self.changes
            .sort_by(|a, b| measure_beat_order((a.0, a.1), (b.0, b.1)));
    }

    /// The time signature in effect at a given measure number. Changes
    /// apply at the start of their specified measure.
    fn at_measure(&self, measure: u32) -> TimeSignature {
        let mut ts = self.initial;
        for (change_m, change_b, new_ts) in &self.changes {
            // Apply changes that occur before this measure, or exactly at
            // the start (beat ≈ 1) of this measure.
            if *change_m < measure || (*change_m == measure && (*change_b - 1.0).abs() < 0.001) {
                ts = *new_ts;
            }
        }
        ts
    }

    /// Total quarter-note beats from measure 1, beat 1 to the given
    /// (target_measure, target_beat), accounting for time signature changes.
    fn beats_at(&self, target_measure: u32, target_beat: f64) -> f64 {
        let mut target_beats = 0.0;
        let mut current_measure: u32 = 1;
        let mut current_beat_in_measure = 1.0;

        while current_measure < target_measure
            || (current_measure == target_measure && current_beat_in_measure < target_beat)
        {
            let ts_for_this_measure = self.at_measure(current_measure);

            // If we're at the target measure, calculate partial beats
            if current_measure == target_measure {
                target_beats += target_beat - current_beat_in_measure;
                break;
            }

            // Add remaining beats in current measure
            let beats_per_current_measure = ts_for_this_measure.beats_per_measure();
            let beats_already_counted = current_beat_in_measure - 1.0;
            target_beats += beats_per_current_measure - beats_already_counted;

            current_measure += 1;
            current_beat_in_measure = 1.0;
        }

        target_beats
    }

    /// The bar a quarter-note beat position (0 at bar 1 beat 1, not
    /// negative) falls in: its number, the beats into it, and the beats it
    /// holds. The walk stops at a bar of no length, or when the position is
    /// not finite (it would never be reached).
    fn bar_at(&self, beats: f64) -> (u32, f64, f64) {
        let mut remaining = beats;
        let mut measure: u32 = 1;
        loop {
            let per_measure = self.at_measure(measure).beats_per_measure();
            if per_measure <= 0.0 || remaining < per_measure || !remaining.is_finite() {
                return (measure, remaining, per_measure);
            }
            remaining -= per_measure;
            measure += 1;
        }
    }

    /// The (measure, beat) of a quarter-note beat position, snapped; bar 1
    /// for one before it.
    fn measure_beat_at(&self, beats: f64) -> (u32, f64) {
        let (measure, into, _) = self.bar_at(snap_beats(beats.max(0.0)));
        (measure, 1.0 + into)
    }

    /// Measures elapsed (0 at bar 1 beat 1, fractional within a bar) at a
    /// quarter-note beat position; before bar 1, in bars of the initial
    /// time signature.
    fn measures_at(&self, beats: f64) -> f64 {
        if beats < 0.0 {
            return beats / self.initial.beats_per_measure();
        }
        let (measure, into, per_measure) = self.bar_at(beats);
        if per_measure <= 0.0 {
            return (measure - 1) as f64;
        }
        (measure - 1) as f64 + into / per_measure
    }

    /// The quarter-note beat position at a measure position (the inverse of
    /// [`Metres::measures_at`]).
    fn beats_at_measures(&self, measures: f64) -> f64 {
        if measures < 0.0 {
            return measures * self.initial.beats_per_measure();
        }
        let whole = measures.floor();
        let bar = whole as u32 + 1;
        let per_measure = self.at_measure(bar).beats_per_measure();
        let beat_in_bar = 1.0 + (measures - whole) * per_measure;
        self.beats_at(bar, beat_in_bar)
    }
}

/// The tempo map walked once, so that many positions can be read off
/// without walking the changes again for each: the state at bar 1 and just
/// past each change, in time order, and the metres as the beat grid has
/// them. The arithmetic is `TempoMap::new`'s own, so a change lands
/// exactly where it was resolved to, and a position converted one way and
/// back lands where a cue at that position fires.
struct Segments<'a> {
    map: &'a TempoMap,
    states: Vec<SegmentState>,
    metres: Metres,
}

impl Segments<'_> {
    /// The state in force at a score time: that past the last change at or
    /// before it.
    fn state_at(&self, time: Duration) -> &SegmentState {
        let past = self
            .map
            .changes
            .iter()
            .take_while(|change| change.position.absolute_time().is_some_and(|at| at <= time))
            .count();
        &self.states[past]
    }

    /// The quarter-note beat position (0 at bar 1 beat 1) of a score time,
    /// integrating through gradual transitions; negative before bar 1, at
    /// the starting tempo. The inverse of [`Segments::time_at`].
    fn beats_at(&self, time: Duration) -> f64 {
        let start = self.map.start_offset;
        if time < start {
            return -steady_beats(self.map.initial_bpm, (start - time).as_secs_f64());
        }
        self.state_at(time).beats_at(time)
    }

    /// The score time of a quarter-note beat position (0 at bar 1 beat 1);
    /// before bar 1, at the starting tempo.
    fn time_at(&self, beats: f64) -> Duration {
        let start = self.map.start_offset;
        if beats < 0.0 {
            return start.saturating_sub(duration_from_secs(steady_secs(
                self.map.initial_bpm,
                -beats,
            )));
        }
        let past = self.states[1..]
            .iter()
            .take_while(|state| state.beats <= beats)
            .count();
        self.states[past].time_at(beats)
    }
}

/// Tempo map that tracks tempo and time signature changes over time
#[derive(Debug, Clone)]
pub struct TempoMap {
    /// Starting offset in seconds
    pub start_offset: Duration,
    /// Initial BPM
    pub initial_bpm: f64,
    /// Initial time signature
    pub initial_time_signature: TimeSignature,
    /// Sorted list of tempo changes (by time)
    pub changes: Vec<TempoChange>,
}

impl TempoMap {
    /// Create a new TempoMap, resolving all measure/beat positions to absolute time
    pub fn new(
        start_offset: Duration,
        initial_bpm: f64,
        initial_time_signature: TimeSignature,
        changes: Vec<TempoChange>,
    ) -> Self {
        // Resolve all measure/beat positions to absolute time
        // We need to process changes sequentially, converting each measure/beat using
        // the tempo state accumulated so far
        let mut map = TempoMap {
            start_offset,
            initial_bpm,
            initial_time_signature,
            changes: Vec::new(),
        };
        let mut current_time_sig = initial_time_signature;
        // The metre changes resolved so far, as the beat grid has them.
        let mut metres = Metres::new(initial_time_signature);

        // Time-positioned changes in time order, bar-positioned ones in bar
        // order, merged as they are resolved: each step takes whichever comes
        // first in time from where the last left off, so every change is
        // reached through the tempo (and any ramp) of all those before it.
        let (mut timed, barred): (Vec<_>, Vec<_>) = changes
            .into_iter()
            .partition(|change| matches!(change.position, TempoChangePosition::Time(_)));
        timed.sort_by_key(|change| change.position.absolute_time());
        let mut barred: Vec<(u32, f64, TempoChange)> = barred
            .into_iter()
            .filter_map(|change| match change.position {
                TempoChangePosition::MeasureBeat(m, b) => Some((m, b, change)),
                TempoChangePosition::Time(_) => None,
            })
            .collect();
        barred.sort_by(|a, b| measure_beat_order((a.0, a.1), (b.0, b.1)));
        let mut timed = std::collections::VecDeque::from(timed);
        let mut barred = std::collections::VecDeque::from(barred);

        let mut state = SegmentState::at_bar_one(&map);

        loop {
            // Where the next bar-positioned change falls from here, as the
            // beat grid counts its bar, integrating through any
            // still-in-progress gradual transition.
            let next_barred = barred.front().map(|(m, b, _)| metres.beats_at(*m, *b));
            let timed_first = match (timed.front(), next_barred) {
                (Some(change), Some(beats)) => {
                    change.position.absolute_time().unwrap_or(start_offset) <= state.time_at(beats)
                }
                (next_timed, _) => next_timed.is_some(),
            };

            let (change, at) = if timed_first {
                let Some(change) = timed.pop_front() else {
                    break;
                };
                let t = change.position.absolute_time().unwrap_or(start_offset);
                (change, ChangeAt::Time(t))
            } else {
                let (Some((_, _, change)), Some(beats)) = (barred.pop_front(), next_barred) else {
                    break;
                };
                (change, ChangeAt::Beats(beats))
            };

            // A ramp in measures matches bpm_at_time: its length is counted
            // in the time signature in effect after this change and any
            // other at the same position, whichever order they are listed in
            // (`time_signature_through`, before the pending ones are
            // resolved).
            let next = state.through(at, change.bpm, change.transition, |reached| {
                let absolute_time = match at {
                    ChangeAt::Time(t) => t,
                    ChangeAt::Beats(_) => reached.time,
                };
                let mut sizing_time_sig = current_time_sig;
                for pending in timed.iter() {
                    if pending.position.absolute_time() != Some(absolute_time) {
                        break;
                    }
                    if let Some(new_ts) = pending.time_signature {
                        sizing_time_sig = new_ts;
                    }
                }
                for (m, b, pending) in barred.iter() {
                    if metres.beats_at(*m, *b) > reached.beats + 1e-9 {
                        break;
                    }
                    if let Some(new_ts) = pending.time_signature {
                        sizing_time_sig = new_ts;
                    }
                }
                sizing_time_sig
            });
            let absolute_time = match at {
                ChangeAt::Time(t) => t,
                ChangeAt::Beats(_) => next.time,
            };

            // Create resolved change, preserving original measure/beat if it was one
            // Use the original_measure_beat from the change if it exists, otherwise extract from position
            let original_measure_beat = change.original_measure_beat.or(match &change.position {
                TempoChangePosition::MeasureBeat(m, b) => Some((*m, *b)),
                TempoChangePosition::Time(_) => None,
            });
            map.changes.push(TempoChange {
                position: TempoChangePosition::Time(absolute_time),
                original_measure_beat,
                bpm: change.bpm,
                time_signature: change.time_signature,
                transition: change.transition,
            });

            // A change's own metre does not move it: one at beat 1 governs
            // from its bar on, one mid-bar from the next.
            if let Some(new_ts) = change.time_signature {
                current_time_sig = new_ts;
                let (m, b) = match &change.position {
                    TempoChangePosition::MeasureBeat(m, b) => (*m, *b),
                    TempoChangePosition::Time(_) => metres.measure_beat_at(next.beats),
                };
                metres.add(m, b, new_ts);
            }
            state = next;
        }

        // Sort by absolute time (now all are Time positions)
        map.changes.sort_by(|a, b| {
            a.position
                .absolute_time()
                .unwrap_or(Duration::ZERO)
                .cmp(&b.position.absolute_time().unwrap_or(Duration::ZERO))
        });
        map
    }

    /// The map walked once (see [`Segments`]): each change at its
    /// `original_measure_beat` when it has one, placed in the bars the
    /// signature changes before it make, else at its time, through the
    /// tempo and any ramp before it, as `TempoMap::new` resolved it.
    fn segments(&self) -> Segments<'_> {
        let mut metres = Metres::new(self.initial_time_signature);
        let mut states = Vec::with_capacity(self.changes.len() + 1);
        let mut state = SegmentState::at_bar_one(self);
        for change in &self.changes {
            let at = match change.original_measure_beat {
                Some((m, b)) => ChangeAt::Beats(metres.beats_at(m, b)),
                None => {
                    ChangeAt::Time(change.position.absolute_time().unwrap_or(self.start_offset))
                }
            };
            let next = state.through(at, change.bpm, change.transition, |_| {
                self.time_signature_through(change)
            });
            if let Some(new_ts) = change.time_signature {
                let (m, b) = change
                    .original_measure_beat
                    .unwrap_or_else(|| metres.measure_beat_at(next.beats));
                metres.add(m, b, new_ts);
            }
            states.push(std::mem::replace(&mut state, next));
        }
        states.push(state);
        Segments {
            map: self,
            states,
            metres,
        }
    }

    /// Convert a measure/beat position to absolute time with an offset.
    /// `measure_offset` moves the position itself (score measure to playback
    /// measure); `offset_secs` shifts the tempo map the way it shifts cue
    /// times, so the position's time is shifted by it too.
    pub fn measure_to_time_with_offset(
        &self,
        measure: u32,
        beat: f64,
        measure_offset: u32,
        offset_secs: f64,
    ) -> Option<Duration> {
        // Measures and beats are 1-indexed
        if measure < 1 || beat < 1.0 {
            return None;
        }
        let offset_duration = Duration::from_secs_f64(offset_secs);
        let segments = self.segments();
        let target_beats = segments.metres.beats_at(measure + measure_offset, beat);
        Some(
            segments
                .time_at(target_beats)
                .saturating_add(offset_duration),
        )
    }

    /// Convert an absolute time to a (measure, beat) position, as the beat
    /// grid counts it: through all tempo changes and ramps before it, in the
    /// bars the time signature changes make.
    pub(crate) fn time_to_measure_beat(&self, target_time: Duration) -> (u32, f64) {
        let segments = self.segments();
        segments
            .metres
            .measure_beat_at(segments.beats_at(target_time))
    }

    /// Get BPM at a given time (accounting for tempo changes)
    /// If offset_secs is provided, it's added to tempo change times to account for timeline shifts
    ///
    /// The tempo the beat walk has there: inside a gradual transition, the
    /// curve at that point, started from the tempo in force when the change
    /// was reached (mid-ramp, if a ramp was still running).
    pub fn bpm_at_time(&self, time: Duration, offset_secs: f64) -> f64 {
        let Some(score_time) = time.checked_sub(Duration::from_secs_f64(offset_secs)) else {
            return self.initial_bpm;
        };
        self.segments().state_at(score_time).bpm_at(score_time)
    }

    /// Get time signature at a given time
    /// If offset_secs is provided, it's added to tempo change times to account for timeline shifts
    ///
    /// As the beat grid has it: a change anchored mid-bar governs from the
    /// next bar.
    pub fn time_signature_at_time(&self, time: Duration, offset_secs: f64) -> TimeSignature {
        let offset_duration = Duration::from_secs_f64(offset_secs);
        let mut ts = self.initial_time_signature;

        for change in &self.changes {
            let Some(new_ts) = change.time_signature else {
                continue;
            };
            let Some(at) = change.position.absolute_time() else {
                continue;
            };
            if at.saturating_add(offset_duration) > time {
                break;
            }
            let (m, b) = change
                .original_measure_beat
                .unwrap_or_else(|| self.time_to_measure_beat(at));
            if (b - 1.0).abs() >= 0.001
                && self
                    .measure_to_time_with_offset(m + 1, 1.0, 0, 0.0)
                    .is_some_and(|next_bar| next_bar.saturating_add(offset_duration) > time)
            {
                continue;
            }
            ts = new_ts;
        }

        ts
    }

    /// The time signature set by the changes up to and at `change`'s time,
    /// whichever order those at it are listed in, at once, whether or not a
    /// barline has been reached since: what a transition in measures counts
    /// its length in, as `TempoMap::new` does.
    fn time_signature_through(&self, change: &TempoChange) -> TimeSignature {
        let at = change.position.absolute_time();
        let mut ts = self.initial_time_signature;
        for prev_change in &self.changes {
            if prev_change.position.absolute_time() > at {
                break;
            }
            if let Some(new_ts) = prev_change.time_signature {
                ts = new_ts;
            }
        }
        ts
    }

    /// Convert a duration in beats to absolute Duration at a given time.
    ///
    /// Computed as a position problem, like [`TempoMap::measures_to_duration`]:
    /// the beat position at `at_time`, plus `beats`, converted back to time
    /// through the same walk cue positions use. So an effect lasting up to the
    /// next cue ends exactly where that cue starts -- not a nanosecond either
    /// side, which a separately rounded start and length could miss by (#534) --
    /// and gradual transitions are integrated as cue positions integrate them.
    /// `offset_secs` shifts the tempo map the way it shifts cue times.
    pub fn beats_to_duration(&self, beats: f64, at_time: Duration, offset_secs: f64) -> Duration {
        let segments = self.segments();
        let score_at = at_time.saturating_sub(Duration::from_secs_f64(offset_secs.max(0.0)));
        let start_beats = snap_beats(segments.beats_at(score_at));
        let end_beats = snap_beats(start_beats + beats);
        segments.time_at(end_beats).saturating_sub(score_at)
    }

    /// Convert a duration in measures to absolute Duration at a given time.
    ///
    /// Computed as a position problem: the measure position at `at_time`, plus
    /// `measures`, converted back to time through the same walk cue positions
    /// use. So a duration integrates through gradual transitions and time
    /// signature changes exactly as `@bar/beat` does, and an effect lasting
    /// "one bar" from a downbeat ends on the next downbeat even inside a
    /// ritardando (#526). `offset_secs` shifts the tempo map the way it shifts
    /// cue times: the conversion runs in score time, `at_time - offset_secs`.
    pub fn measures_to_duration(
        &self,
        measures: f64,
        at_time: Duration,
        offset_secs: f64,
    ) -> Duration {
        let segments = self.segments();
        let score_at = at_time.saturating_sub(Duration::from_secs_f64(offset_secs.max(0.0)));
        let start_beats = snap_beats(segments.beats_at(score_at));
        let end_measures = segments.metres.measures_at(start_beats) + measures;
        let end_beats = snap_beats(segments.metres.beats_at_measures(end_measures));
        segments.time_at(end_beats).saturating_sub(score_at)
    }

    /// How long the `beats` before `end` last: the same position walk as
    /// [`Self::beats_to_duration`], run backwards from `end`.
    pub fn beats_before(&self, beats: f64, end: Duration) -> Duration {
        let segments = self.segments();
        let end_beats = snap_beats(segments.beats_at(end));
        let start_beats = snap_beats(end_beats - beats);
        end.saturating_sub(segments.time_at(start_beats))
    }

    /// How long the `measures` before `end` last: the same position walk as
    /// [`Self::measures_to_duration`], run backwards from `end`.
    pub fn measures_before(&self, measures: f64, end: Duration) -> Duration {
        let segments = self.segments();
        let end_beats = snap_beats(segments.beats_at(end));
        let start_measures = segments.metres.measures_at(end_beats) - measures;
        let start_beats = snap_beats(segments.metres.beats_at_measures(start_measures));
        end.saturating_sub(segments.time_at(start_beats))
    }

    /// The times of a MIDI beat clock's pulses, 24 to the quarter note, from
    /// the start of the song up to `until`.
    ///
    /// Bar 1 falls on a pulse, and the pulses follow every tempo change and
    /// ramp from there. Any time before bar 1 (a count-in, a lead-in) pulses
    /// at the starting tempo, in phase with it, so gear following the clock
    /// is already on the beat when bar 1 comes.
    pub(crate) fn beat_clock_ticks(&self, until: Duration) -> Vec<Duration> {
        const PULSES_PER_QUARTER: f64 = 24.0;
        // A bound for a degenerate map: an hour at 300 BPM is ~430k pulses.
        const MAX_PULSES: usize = 2_000_000;

        let mut ticks = Vec::new();
        if !self.initial_bpm.is_finite() || self.initial_bpm <= 0.0 {
            return ticks;
        }
        let states = self.segments().states;

        // The lead-in is whole bars at the starting tempo: gear counts beats,
        // and bars, from the first pulse after Start, so bar 1 must be a whole
        // number of bars of pulses in, and of quarters. What is left over before the first
        // whole bar has no pulses. Snapped first, so a count-in of exactly
        // whole bars that floating point puts a hair short keeps its first.
        let interval = 60.0 / self.initial_bpm / PULSES_PER_QUARTER;
        let bar_pulses = ((self.initial_time_signature.beats_per_measure() * PULSES_PER_QUARTER)
            .round() as usize)
            .max(1);
        let start = self.start_offset.as_secs_f64();
        let lead_bars = (start / (bar_pulses as f64 * interval) + 1e-9).floor() as usize;
        // And whole quarters: gear knows nothing of the song's meter and
        // counts its beats in them, so a bar of 7/8 rounds down to three.
        let quarter = PULSES_PER_QUARTER as usize;
        let lead = (lead_bars.saturating_mul(bar_pulses) / quarter * quarter).min(MAX_PULSES);
        for k in (1..=lead).rev() {
            let at = (start - k as f64 * interval).max(0.0);
            if at < until.as_secs_f64() {
                ticks.push(Duration::from_secs_f64(at));
            }
        }

        // Pulses only move forward, so the segment each falls in does too.
        let mut segment = 0;
        for k in 0..MAX_PULSES {
            let beats = k as f64 / PULSES_PER_QUARTER;
            while states
                .get(segment + 1)
                .is_some_and(|next| next.beats <= beats)
            {
                segment += 1;
            }
            let at = states[segment].time_at(beats);
            if at >= until {
                break;
            }
            ticks.push(at);
        }
        ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    mod beat_clock_ticks_tests {
        use super::*;

        const PULSE_120: f64 = 0.5 / 24.0;

        fn map(start: Duration, bpm: f64, changes: Vec<TempoChange>) -> TempoMap {
            TempoMap::new(start, bpm, TimeSignature::new(4, 4), changes)
        }

        fn secs(ticks: &[Duration]) -> Vec<f64> {
            ticks.iter().map(Duration::as_secs_f64).collect()
        }

        /// 24 pulses to the quarter note at the map's tempo, from the start
        /// of the song and short of `until`.
        #[test]
        fn a_steady_tempo_pulses_evenly_up_to_the_end() {
            let ticks = secs(
                &map(Duration::ZERO, 120.0, vec![]).beat_clock_ticks(Duration::from_millis(990)),
            );
            assert_eq!(ticks.len(), 48, "{ticks:?}");
            for (k, at) in ticks.iter().enumerate() {
                assert!(
                    (at - k as f64 * PULSE_120).abs() < 1e-6,
                    "pulse {k} at {at}"
                );
            }
        }

        /// A count-in before bar 1 pulses at the starting tempo, in whole
        /// bars: gear counting from the first pulse after Start has bar 1 on
        /// a downbeat. What is left over before them has no pulses.
        #[test]
        fn time_before_bar_one_pulses_in_whole_bars() {
            // At 120 BPM in 4/4 a bar is 2 s and 96 pulses.
            for (start_ms, lead_bars) in [(300, 0), (2_000, 1), (2_137, 1), (4_999, 2), (5_000, 2)]
            {
                let start = Duration::from_millis(start_ms);
                let ticks = map(start, 120.0, vec![]).beat_clock_ticks(Duration::from_secs(8));
                let bar_one = ticks
                    .iter()
                    .position(|at| *at == start)
                    .expect("a pulse on bar 1");
                assert_eq!(bar_one, lead_bars * 96, "start {start_ms}ms");
                let first = start.as_secs_f64() - lead_bars as f64 * 2.0;
                assert!(
                    (ticks[0].as_secs_f64() - first).abs() < 1e-6,
                    "start {start_ms}ms"
                );
                for pair in secs(&ticks).windows(2) {
                    assert!((pair[1] - pair[0] - PULSE_120).abs() < 1e-6, "{pair:?}");
                }
            }
        }

        /// A count-in in a meter whose bar is not whole quarters keeps gear's
        /// quarter grid: bar 1 lands on a quarter from the first pulse.
        #[test]
        fn an_odd_meters_count_in_keeps_whole_quarters() {
            // 7/8 at 120 BPM: a bar is 3.5 quarters, 1.75 s, 84 pulses.
            for (start_ms, lead) in [(1_750, 72), (3_500, 168), (5_250, 240)] {
                let start = Duration::from_millis(start_ms);
                let ticks = TempoMap::new(start, 120.0, TimeSignature::new(7, 8), vec![])
                    .beat_clock_ticks(Duration::from_secs(8));
                let bar_one = ticks
                    .iter()
                    .position(|at| *at == start)
                    .expect("a pulse on bar 1");
                assert_eq!(bar_one, lead, "start {start_ms}ms");
                assert_eq!(bar_one % 24, 0);
            }
        }

        /// A count-in of exactly whole bars starts with a pulse at zero, at
        /// any tempo: floating point putting it a hair short must not drop
        /// a bar.
        #[test]
        fn a_count_in_of_whole_bars_pulses_from_zero() {
            for bpm in [60.0, 90.0, 100.0, 120.0, 133.0, 140.0, 150.0, 170.0, 180.0] {
                for bars in 1..=8u32 {
                    let start = Duration::from_secs_f64(bars as f64 * 4.0 * 60.0 / bpm);
                    let ticks = map(start, bpm, vec![]).beat_clock_ticks(Duration::from_secs(40));
                    assert!(
                        ticks[0] < Duration::from_micros(1),
                        "{bpm} BPM, {bars} bars: {:?}",
                        ticks[0]
                    );
                    for pair in ticks.windows(2) {
                        assert!(pair[1] > pair[0], "{bpm} BPM, {bars} bars: {pair:?}");
                    }
                }
            }
        }

        /// A tempo change lands on a pulse and the pulses after it run at
        /// the new tempo.
        #[test]
        fn a_tempo_change_retunes_the_pulses_from_the_change() {
            let ticks = secs(
                &map(
                    Duration::ZERO,
                    120.0,
                    vec![TempoChange {
                        position: TempoChangePosition::Time(Duration::from_secs(5)),
                        original_measure_beat: None,
                        bpm: Some(180.0),
                        time_signature: None,
                        transition: TempoTransition::Snap,
                    }],
                )
                .beat_clock_ticks(Duration::from_secs(7)),
            );
            // Ten beats at 120 BPM: the change is pulse 240.
            assert!((ticks[240] - 5.0).abs() < 1e-6, "{}", ticks[240]);
            assert!((ticks[240] - ticks[239] - PULSE_120).abs() < 1e-6);
            let pulse_180 = 60.0 / 180.0 / 24.0;
            assert!((ticks[241] - ticks[240] - pulse_180).abs() < 1e-6);
            assert!((ticks.last().unwrap() - ticks[ticks.len() - 2] - pulse_180).abs() < 1e-6);
        }

        /// Through a ramp the pulses speed up steadily, and every 24th one
        /// falls on the beat the tempo map (and the lighting) puts there.
        #[test]
        fn a_ramp_is_followed_pulse_by_pulse_and_stays_on_the_beat() {
            let tempo = map(
                Duration::from_millis(250),
                120.0,
                vec![TempoChange {
                    position: TempoChangePosition::Time(Duration::from_secs(2)),
                    original_measure_beat: None,
                    bpm: Some(180.0),
                    time_signature: None,
                    transition: TempoTransition::Beats(8.0, TransitionCurve::Linear),
                }],
            );
            let all = tempo.beat_clock_ticks(Duration::from_secs(10));
            // Pulses from bar 1 on: the lead-in is 12 of them.
            let lead = all
                .iter()
                .take_while(|at| **at < Duration::from_millis(250))
                .count();
            let ticks = secs(&all[lead..]);
            for pair in ticks.windows(2) {
                assert!(pair[1] > pair[0], "{pair:?}");
            }
            let gaps: Vec<f64> = ticks.windows(2).map(|p| p[1] - p[0]).collect();
            let first = gaps.first().unwrap();
            let last = gaps.last().unwrap();
            assert!((first - PULSE_120).abs() < 1e-6);
            assert!((last - 60.0 / 180.0 / 24.0).abs() < 1e-6);
            for pair in gaps.windows(2) {
                assert!(pair[1] <= pair[0] + 5e-9, "a ramp up slowed down: {pair:?}");
            }
            for quarter in (0..ticks.len()).step_by(24) {
                let beat = quarter / 24;
                let on_beat = tempo
                    .measure_to_time_with_offset(
                        beat as u32 / 4 + 1,
                        (beat % 4) as f64 + 1.0,
                        0,
                        0.0,
                    )
                    .unwrap()
                    .as_secs_f64();
                assert!(
                    (ticks[quarter] - on_beat).abs() < 1e-6,
                    "beat {beat}: pulse at {}, beat at {on_beat}",
                    ticks[quarter]
                );
            }
        }

        /// Pulses are in quarter notes whatever the bar: 6/8 does not change
        /// their spacing, only where the bars fall.
        #[test]
        fn the_metre_does_not_change_the_pulse() {
            let ticks = secs(
                &TempoMap::new(Duration::ZERO, 120.0, TimeSignature::new(6, 8), vec![])
                    .beat_clock_ticks(Duration::from_secs(1)),
            );
            for pair in ticks.windows(2) {
                assert!((pair[1] - pair[0] - PULSE_120).abs() < 1e-6, "{pair:?}");
            }
        }

        /// Snaps, ramps in beats and in bars, and metre changes: every pulse
        /// is where the tempo map's own beat-to-time walk puts its position.
        #[test]
        fn every_pulse_is_where_the_tempo_map_puts_it() {
            let changes = (1..40)
                .map(|i| TempoChange {
                    position: TempoChangePosition::Time(Duration::from_secs(i * 10)),
                    original_measure_beat: None,
                    bpm: Some(100.0 + (i % 7) as f64 * 10.0),
                    time_signature: (i % 5 == 0).then(|| TimeSignature::new(7, 8)),
                    transition: match i % 3 {
                        0 => TempoTransition::Beats(4.0, TransitionCurve::Linear),
                        1 => TempoTransition::Measures(2.0, TransitionCurve::Linear),
                        _ => TempoTransition::Snap,
                    },
                })
                .collect();
            let tempo = TempoMap::new(
                Duration::from_secs(2),
                120.0,
                TimeSignature::new(4, 4),
                changes,
            );
            let ticks = tempo.beat_clock_ticks(Duration::from_secs(420));
            let segments = tempo.segments();
            let lead = ticks
                .iter()
                .take_while(|at| **at < Duration::from_secs(2))
                .count();
            assert!(ticks.len() > 20_000, "{}", ticks.len());
            for (k, at) in ticks[lead..].iter().enumerate() {
                assert_eq!(*at, segments.time_at(k as f64 / 24.0), "pulse {k}");
            }
        }

        /// No time, no pulses.
        #[test]
        fn nothing_to_play_has_no_pulses() {
            assert!(map(Duration::ZERO, 120.0, vec![])
                .beat_clock_ticks(Duration::ZERO)
                .is_empty());
        }
    }

    // ── TimeSignature ──────────────────────────────────────────────

    #[test]
    fn time_signature_4_4() {
        let ts = TimeSignature::new(4, 4);
        assert!((ts.beats_per_measure() - 4.0).abs() < f64::EPSILON);
    }

    #[test]
    fn time_signature_3_4() {
        let ts = TimeSignature::new(3, 4);
        assert!((ts.beats_per_measure() - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn time_signature_6_8() {
        // 6/8 = 6 * 4/8 = 3 quarter-note beats
        let ts = TimeSignature::new(6, 8);
        assert!((ts.beats_per_measure() - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn time_signature_7_8() {
        // 7/8 = 7 * 4/8 = 3.5 quarter-note beats
        let ts = TimeSignature::new(7, 8);
        assert!((ts.beats_per_measure() - 3.5).abs() < f64::EPSILON);
    }

    #[test]
    fn time_signature_2_2() {
        // 2/2 = 2 * 4/2 = 4 quarter-note beats
        let ts = TimeSignature::new(2, 2);
        assert!((ts.beats_per_measure() - 4.0).abs() < f64::EPSILON);
    }

    // ── TransitionCurve::bpm_at ────────────────────────────────────

    #[test]
    fn linear_bpm_at_start() {
        assert!((TransitionCurve::Linear.bpm_at(0.0, 120.0, 180.0) - 120.0).abs() < f64::EPSILON);
    }

    #[test]
    fn linear_bpm_at_end() {
        assert!((TransitionCurve::Linear.bpm_at(1.0, 120.0, 180.0) - 180.0).abs() < f64::EPSILON);
    }

    #[test]
    fn linear_bpm_at_midpoint() {
        assert!((TransitionCurve::Linear.bpm_at(0.5, 120.0, 180.0) - 150.0).abs() < f64::EPSILON);
    }

    #[test]
    fn linear_bpm_clamped_below_zero() {
        // t < 0 should clamp to 0
        assert!((TransitionCurve::Linear.bpm_at(-0.5, 100.0, 200.0) - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn linear_bpm_clamped_above_one() {
        // t > 1 should clamp to 1
        assert!((TransitionCurve::Linear.bpm_at(1.5, 100.0, 200.0) - 200.0).abs() < f64::EPSILON);
    }

    #[test]
    fn linear_bpm_deceleration() {
        // BPM decreasing
        assert!((TransitionCurve::Linear.bpm_at(0.5, 200.0, 100.0) - 150.0).abs() < f64::EPSILON);
    }

    // ── SegmentRamp::beats_in_secs ──────────────────────────────────

    /// A ramp from `old` to `new` BPM over `total` seconds, `elapsed` in.
    fn ramp(old: f64, new: f64, total: f64, elapsed: f64) -> SegmentRamp {
        SegmentRamp {
            old_bpm: old,
            new_bpm: new,
            total_secs: total,
            elapsed_secs: elapsed,
            curve: TransitionCurve::Linear,
        }
    }

    #[test]
    fn beats_in_secs_constant_bpm() {
        // When old_bpm == new_bpm, this reduces to bpm * dt / 60
        let beats = ramp(120.0, 120.0, 4.0, 0.0).beats_in_secs(2.0, 120.0);
        // 120 bpm for 2 seconds = 4 beats
        assert!((beats - 4.0).abs() < 1e-9);
    }

    #[test]
    fn beats_in_secs_full_transition() {
        // Full transition from 60 to 120 bpm over 4 seconds
        // integral of (60 + 60*t/4)/60 from 0 to 4 = integral of (1 + t/4) from 0 to 4
        // = [t + t^2/8] from 0 to 4 = 4 + 16/8 = 4 + 2 = 6
        let beats = ramp(60.0, 120.0, 4.0, 0.0).beats_in_secs(4.0, 120.0);
        assert!((beats - 6.0).abs() < 1e-9);
    }

    #[test]
    fn beats_in_secs_half_transition() {
        // First 2 seconds of a 4-second transition from 60 to 120 bpm
        // integral of (60 + 60*t/4)/60 from 0 to 2 = integral of (1 + t/4) from 0 to 2
        // = [t + t^2/8] from 0 to 2 = 2 + 4/8 = 2 + 0.5 = 2.5
        let beats = ramp(60.0, 120.0, 4.0, 0.0).beats_in_secs(2.0, 120.0);
        assert!((beats - 2.5).abs() < 1e-9);
    }

    // ── TransitionCurve::beats_in_remaining_transition ──────────────

    #[test]
    fn beats_in_remaining_from_start() {
        // From start to end should equal beats_in_secs for the full duration
        let full = ramp(60.0, 120.0, 4.0, 0.0).beats_in_secs(4.0, 120.0);
        let remaining =
            TransitionCurve::Linear.beats_in_remaining_transition(60.0, 120.0, 4.0, 0.0);
        assert!((full - remaining).abs() < 1e-9);
    }

    #[test]
    fn beats_in_remaining_from_midpoint() {
        // Second half of transition: from elapsed=2 to total=4
        // integral of (60 + 60*t/4)/60 from 2 to 4
        // = [t + t^2/8] from 2 to 4 = (4+2) - (2+0.5) = 6 - 2.5 = 3.5
        let remaining =
            TransitionCurve::Linear.beats_in_remaining_transition(60.0, 120.0, 4.0, 2.0);
        assert!((remaining - 3.5).abs() < 1e-9);
    }

    // ── TransitionCurve::solve_duration_for_beats ────────────────────

    #[test]
    fn solve_duration_constant_bpm() {
        // Constant 120 bpm: 4 beats should take 2 seconds
        let dt = TransitionCurve::Linear
            .solve_duration_for_beats(120.0, 120.0, 4.0, 0.0, 4.0)
            .unwrap();
        assert!((dt - 2.0).abs() < 1e-6);
    }

    #[test]
    fn solve_duration_round_trip() {
        // beats_in_secs -> secs_for_beats should round-trip, from the start
        // of a ramp and from part way in.
        for elapsed in [0.0, 2.5] {
            let ramp = ramp(80.0, 160.0, 8.0, elapsed);
            let beats = ramp.beats_in_secs(3.0, 160.0);
            let dt_output = ramp.secs_for_beats(beats, 160.0);
            assert!((3.0 - dt_output).abs() < 1e-6, "{elapsed} in: {dt_output}");
        }
    }

    #[test]
    fn solve_duration_zero_bpm_returns_none() {
        // Both BPMs zero should return None
        let result = TransitionCurve::Linear.solve_duration_for_beats(0.0, 0.0, 4.0, 0.0, 1.0);
        assert!(result.is_none());
    }

    /// Beats a ramp down never reaches have no time on its curve: the
    /// curve says so, rather than guessing from an average tempo, and the
    /// ramp settles at the new tempo for the beats past its end.
    #[test]
    fn beats_past_a_ramps_reach_are_timed_at_the_settled_tempo() {
        // 120 to 60 BPM over 4 s covers 6 beats; 100 beats are past it.
        assert!(TransitionCurve::Linear
            .solve_duration_for_beats(120.0, 60.0, 4.0, 0.0, 100.0)
            .is_none());
        let secs = ramp(120.0, 60.0, 4.0, 0.0).secs_for_beats(100.0, 60.0);
        assert!((secs - (4.0 + 94.0)).abs() < 1e-9, "{secs}");
    }

    // ── TempoChangePosition ─────────────────────────────────────────

    #[test]
    fn tempo_change_position_time() {
        let pos = TempoChangePosition::Time(Duration::from_secs(5));
        assert_eq!(pos.absolute_time(), Some(Duration::from_secs(5)));
    }

    #[test]
    fn tempo_change_position_measure_beat() {
        let pos = TempoChangePosition::MeasureBeat(4, 1.0);
        assert_eq!(pos.absolute_time(), None);
    }

    // ── TempoMap basics ─────────────────────────────────────────────

    fn simple_tempo_map(bpm: f64) -> TempoMap {
        TempoMap::new(Duration::ZERO, bpm, TimeSignature::new(4, 4), vec![])
    }

    #[test]
    fn bpm_at_time_no_changes() {
        let map = simple_tempo_map(120.0);
        assert!((map.bpm_at_time(Duration::from_secs(10), 0.0) - 120.0).abs() < f64::EPSILON);
    }

    #[test]
    fn time_signature_at_time_no_changes() {
        let map = simple_tempo_map(120.0);
        assert_eq!(
            map.time_signature_at_time(Duration::from_secs(10), 0.0),
            TimeSignature::new(4, 4)
        );
    }

    #[test]
    fn bpm_at_time_with_snap_change() {
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(5)),
                original_measure_beat: None,
                bpm: Some(180.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        // Before the change
        assert!((map.bpm_at_time(Duration::from_secs(3), 0.0) - 120.0).abs() < f64::EPSILON);
        // After the change
        assert!((map.bpm_at_time(Duration::from_secs(6), 0.0) - 180.0).abs() < f64::EPSILON);
    }

    #[test]
    fn time_signature_change() {
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(8)),
                original_measure_beat: None,
                bpm: None,
                time_signature: Some(TimeSignature::new(3, 4)),
                transition: TempoTransition::Snap,
            }],
        );
        assert_eq!(
            map.time_signature_at_time(Duration::from_secs(3), 0.0),
            TimeSignature::new(4, 4)
        );
        assert_eq!(
            map.time_signature_at_time(Duration::from_secs(10), 0.0),
            TimeSignature::new(3, 4)
        );
    }

    // ── TempoMap::beats_to_duration ──────────────────────────────────

    #[test]
    fn beats_to_duration_constant_tempo() {
        let map = simple_tempo_map(120.0);
        // 4 beats at 120 bpm = 2 seconds
        let dur = map.beats_to_duration(4.0, Duration::ZERO, 0.0);
        assert!((dur.as_secs_f64() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn beats_to_duration_across_tempo_change() {
        let map = TempoMap::new(
            Duration::ZERO,
            60.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(2)),
                original_measure_beat: None,
                bpm: Some(120.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        // 3 beats starting at t=0: 60 bpm = 1 beat/sec
        // First 2 beats take 2 seconds (reaching tempo change at t=2)
        // Then 1 beat at 120 bpm = 0.5 seconds
        // Total = 2.5 seconds
        let dur = map.beats_to_duration(3.0, Duration::ZERO, 0.0);
        assert!((dur.as_secs_f64() - 2.5).abs() < 1e-6);
    }

    #[test]
    fn beats_to_duration_starting_after_change() {
        let map = TempoMap::new(
            Duration::ZERO,
            60.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(2)),
                original_measure_beat: None,
                bpm: Some(120.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        // 4 beats starting at t=5 (after the tempo change to 120 bpm)
        // 4 beats at 120 bpm = 2 seconds
        let dur = map.beats_to_duration(4.0, Duration::from_secs(5), 0.0);
        assert!((dur.as_secs_f64() - 2.0).abs() < 1e-6);
    }

    // ── TempoMap::beats_before / measures_before ──────────────────────

    #[test]
    fn beats_and_measures_before_count_back_through_changes() {
        // 60 bpm to t=2, then 120 bpm.
        let map = TempoMap::new(
            Duration::ZERO,
            60.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(2)),
                original_measure_beat: None,
                bpm: Some(120.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        // From t=3: the 2 beats after the change (1 s), then 1 at 60 bpm.
        let three_beats = map.beats_before(3.0, Duration::from_secs(3));
        assert!(
            (three_beats.as_secs_f64() - 2.0).abs() < 1e-6,
            "{three_beats:?}"
        );
        // Bar 2 starts at t=4 (four beats at 60); one bar back from t=6 is
        // 4 beats at 120 = 2 s.
        let bar = map.measures_before(1.0, Duration::from_secs(6));
        assert!((bar.as_secs_f64() - 2.0).abs() < 1e-6, "{bar:?}");
        // Counting back and forward agree.
        let forward = map.measures_to_duration(1.0, Duration::from_secs(6) - bar, 0.0);
        assert_eq!(forward, bar);
    }

    // ── TempoMap::measures_to_duration ────────────────────────────────

    #[test]
    fn measures_to_duration_constant_tempo_4_4() {
        let map = simple_tempo_map(120.0);
        // 2 measures of 4/4 at 120 bpm = 8 beats = 4 seconds
        let dur = map.measures_to_duration(2.0, Duration::ZERO, 0.0);
        assert!((dur.as_secs_f64() - 4.0).abs() < 1e-9);
    }

    /// The Jotunheim tempo block from #526: 12/8 at 169, a snap to 150 at
    /// bar 3, and a one-measure ramp to `ramp_to` at bar 5.
    fn ramp_map(ramp_to: f64) -> TempoMap {
        TempoMap::new(
            Duration::from_millis(35),
            169.0,
            TimeSignature::new(12, 8),
            vec![
                TempoChange {
                    position: TempoChangePosition::MeasureBeat(3, 1.0),
                    original_measure_beat: None,
                    bpm: Some(150.0),
                    time_signature: None,
                    transition: TempoTransition::Snap,
                },
                TempoChange {
                    position: TempoChangePosition::MeasureBeat(5, 1.0),
                    original_measure_beat: None,
                    bpm: Some(ramp_to),
                    time_signature: None,
                    transition: TempoTransition::Measures(1.0, TransitionCurve::Linear),
                },
            ],
        )
    }

    fn bar(map: &TempoMap, measure: u32, offset_secs: f64) -> Duration {
        map.measure_to_time_with_offset(measure, 1.0, 0, 0.0)
            .unwrap()
            + Duration::from_secs_f64(offset_secs)
    }

    fn assert_close(got: Duration, want: Duration, what: &str) {
        let diff = (got.as_secs_f64() - want.as_secs_f64()).abs();
        assert!(diff < 1e-6, "{what}: got {got:?}, want {want:?}");
    }

    /// #526: one measure started on the downbeat of a ramp bar ends on the
    /// next downbeat -- for a ritardando and an accelerando, in measures and
    /// in the equivalent beats, with and without an offset.
    #[test]
    fn a_measure_inside_a_ramp_ends_on_the_next_downbeat() {
        for ramp_to in [100.0, 200.0] {
            let map = ramp_map(ramp_to);
            for offset in [0.0, 2.4] {
                let start = bar(&map, 5, offset);
                let next = bar(&map, 6, offset);
                assert_close(
                    start + map.measures_to_duration(1.0, start, offset),
                    next,
                    &format!("1 measure, ramp to {ramp_to}, offset {offset}"),
                );
                assert_close(
                    start + map.beats_to_duration(6.0, start, offset),
                    next,
                    &format!("6 beats, ramp to {ramp_to}, offset {offset}"),
                );
                // Two measures from mid-bar 4 cross the snap-free bar 4 tail,
                // the ramp bar, and land half-way through bar 6.
                let from = map.measure_to_time_with_offset(4, 4.0, 0, 0.0).unwrap()
                    + Duration::from_secs_f64(offset);
                let to = map.measure_to_time_with_offset(6, 4.0, 0, 0.0).unwrap()
                    + Duration::from_secs_f64(offset);
                assert_close(
                    from + map.measures_to_duration(2.0, from, offset),
                    to,
                    &format!("2 measures across the ramp, ramp to {ramp_to}, offset {offset}"),
                );
            }
        }
    }

    /// A tempo that is no tempo gives a beat position that is not a
    /// number; a length in measures from it answers rather than walking
    /// bars for ever looking for it.
    #[test]
    fn a_length_in_measures_at_no_tempo_does_not_hang() {
        let map = simple_tempo_map(f64::NAN);
        map.measures_to_duration(1.0, Duration::from_secs(1), 0.0);
        map.measures_before(1.0, Duration::from_secs(1));
    }

    #[test]
    fn measures_to_duration_fractional() {
        let map = simple_tempo_map(120.0);
        // 0.5 measures of 4/4 at 120 bpm = 2 beats = 1 second
        let dur = map.measures_to_duration(0.5, Duration::ZERO, 0.0);
        assert!((dur.as_secs_f64() - 1.0).abs() < 1e-9);
    }

    // ── TempoMap::measure_to_time_with_offset ────────────────────────

    #[test]
    fn measure_to_time_measure_1_beat_1() {
        let map = simple_tempo_map(120.0);
        // Measure 1, beat 1, no offset = start_offset = 0
        let t = map.measure_to_time_with_offset(1, 1.0, 0, 0.0).unwrap();
        assert!((t.as_secs_f64() - 0.0).abs() < 1e-9);
    }

    #[test]
    fn measure_to_time_measure_2_beat_1() {
        let map = simple_tempo_map(120.0);
        // Measure 2, beat 1 = 4 beats at 120 bpm = 2 seconds
        let t = map.measure_to_time_with_offset(2, 1.0, 0, 0.0).unwrap();
        assert!((t.as_secs_f64() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn measure_to_time_measure_1_beat_3() {
        let map = simple_tempo_map(120.0);
        // Measure 1, beat 3 = 2 beats at 120 bpm = 1 second
        let t = map.measure_to_time_with_offset(1, 3.0, 0, 0.0).unwrap();
        assert!((t.as_secs_f64() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn measure_to_time_with_start_offset() {
        let map = TempoMap::new(
            Duration::from_secs(1),
            120.0,
            TimeSignature::new(4, 4),
            vec![],
        );
        // Measure 1, beat 1 with start_offset=1s
        let t = map.measure_to_time_with_offset(1, 1.0, 0, 0.0).unwrap();
        assert!((t.as_secs_f64() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn measure_to_time_invalid_measure_zero() {
        let map = simple_tempo_map(120.0);
        assert!(map.measure_to_time_with_offset(0, 1.0, 0, 0.0).is_none());
    }

    #[test]
    fn measure_to_time_invalid_beat_below_one() {
        let map = simple_tempo_map(120.0);
        assert!(map.measure_to_time_with_offset(1, 0.5, 0, 0.0).is_none());
    }

    /// `offset_secs` shifts the whole map: a position after a change, at a
    /// bar or at a time, is that much later. (It used to shift only the
    /// placement of time-positioned changes, by the offset's worth of beats
    /// on the unshifted map, and leave the position itself unshifted.)
    #[test]
    fn measure_to_time_with_offset_secs_shifts_the_position() {
        for position in [
            TempoChangePosition::MeasureBeat(3, 1.0),
            TempoChangePosition::Time(Duration::from_secs(4)),
        ] {
            let map = TempoMap::new(
                Duration::ZERO,
                120.0,
                TimeSignature::new(4, 4),
                vec![snap_change(position.clone(), Some(60.0), None)],
            );
            // Bars 1-2 at 120 are 4 s, bars 3-4 at 60 are 8 s; bar 5 is 2 s
            // later under an offset of 2 s.
            for (offset, want) in [(0.0, 12.0), (2.0, 14.0)] {
                let bar5 = map.measure_to_time_with_offset(5, 1.0, 0, offset).unwrap();
                assert!(
                    (bar5.as_secs_f64() - want).abs() < 1e-9,
                    "{position:?} offset {offset}: {bar5:?}"
                );
            }
        }
    }

    #[test]
    fn measure_to_time_with_measure_offset() {
        let map = simple_tempo_map(120.0);
        // Measure 1 with offset 2 → effective playback measure 3
        // = 8 beats at 120 bpm = 4 seconds
        let t = map.measure_to_time_with_offset(1, 1.0, 2, 0.0).unwrap();
        assert!((t.as_secs_f64() - 4.0).abs() < 1e-9);
    }

    // ── TempoMap construction with MeasureBeat positions ─────────────

    #[test]
    fn tempo_map_resolves_measure_beat_to_time() {
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::MeasureBeat(3, 1.0),
                original_measure_beat: None,
                bpm: Some(60.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        // Measure 3 beat 1 at 120 bpm 4/4 = 8 beats = 4 seconds
        let change_time = map.changes[0].position.absolute_time().unwrap();
        assert!((change_time.as_secs_f64() - 4.0).abs() < 1e-6);
        // After the change
        assert!((map.bpm_at_time(Duration::from_secs(5), 0.0) - 60.0).abs() < f64::EPSILON);
    }

    // ── TempoMap with offset_secs ────────────────────────────────────

    #[test]
    fn bpm_at_time_with_offset() {
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(5)),
                original_measure_beat: None,
                bpm: Some(60.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        // Without offset, change at t=5: at t=4 we should still be at 120
        assert!((map.bpm_at_time(Duration::from_secs(4), 0.0) - 120.0).abs() < f64::EPSILON);
        // With offset=3, change shifts to t=8: at t=6 we should still be at 120
        assert!((map.bpm_at_time(Duration::from_secs(6), 3.0) - 120.0).abs() < f64::EPSILON);
        // With offset=3, at t=9 we should be at 60
        assert!((map.bpm_at_time(Duration::from_secs(9), 3.0) - 60.0).abs() < f64::EPSILON);
    }

    // ── Multiple tempo changes ────────────────────────────────────────

    #[test]
    fn multiple_snap_tempo_changes() {
        let map = TempoMap::new(
            Duration::ZERO,
            60.0,
            TimeSignature::new(4, 4),
            vec![
                TempoChange {
                    position: TempoChangePosition::Time(Duration::from_secs(4)),
                    original_measure_beat: None,
                    bpm: Some(120.0),
                    time_signature: None,
                    transition: TempoTransition::Snap,
                },
                TempoChange {
                    position: TempoChangePosition::Time(Duration::from_secs(8)),
                    original_measure_beat: None,
                    bpm: Some(60.0),
                    time_signature: None,
                    transition: TempoTransition::Snap,
                },
            ],
        );
        assert!((map.bpm_at_time(Duration::from_secs(2), 0.0) - 60.0).abs() < f64::EPSILON);
        assert!((map.bpm_at_time(Duration::from_secs(6), 0.0) - 120.0).abs() < f64::EPSILON);
        assert!((map.bpm_at_time(Duration::from_secs(10), 0.0) - 60.0).abs() < f64::EPSILON);
    }

    #[test]
    fn beats_to_duration_across_multiple_changes() {
        let map = TempoMap::new(
            Duration::ZERO,
            60.0, // 1 beat/sec
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(2)),
                original_measure_beat: None,
                bpm: Some(60.0), // same BPM (no change, effectively)
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        // 4 beats at constant 60 bpm = 4 seconds
        let dur = map.beats_to_duration(4.0, Duration::ZERO, 0.0);
        assert!((dur.as_secs_f64() - 4.0).abs() < 1e-6);
    }

    // ── bpm_at_time: Beats transition ───────────────────────────────

    #[test]
    fn bpm_at_time_during_beats_transition() {
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(2)),
                original_measure_beat: None,
                bpm: Some(60.0),
                time_signature: None,
                transition: TempoTransition::Beats(8.0, TransitionCurve::Linear),
            }],
        );
        // Before change: should be 120
        let bpm_before = map.bpm_at_time(Duration::from_secs(1), 0.0);
        assert!((bpm_before - 120.0).abs() < f64::EPSILON);

        // After transition completes: should be 60
        let bpm_after = map.bpm_at_time(Duration::from_secs(20), 0.0);
        assert!((bpm_after - 60.0).abs() < f64::EPSILON);
    }

    // ── bpm_at_time: with offset_secs shifting change ──────────────

    #[test]
    fn bpm_at_time_with_offset_shifts_change() {
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(4)),
                original_measure_beat: None,
                bpm: Some(60.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        // Without offset: at t=5, should be 60bpm (after change at t=4)
        let bpm_no_offset = map.bpm_at_time(Duration::from_secs(5), 0.0);
        assert!((bpm_no_offset - 60.0).abs() < f64::EPSILON);

        // With offset=2: change shifts to t=6, so at t=5 still 120bpm
        let bpm_with_offset = map.bpm_at_time(Duration::from_secs(5), 2.0);
        assert!((bpm_with_offset - 120.0).abs() < f64::EPSILON);
    }

    // ── time_signature_at_time: with ts change ──────────────────────

    #[test]
    fn time_signature_at_time_with_change() {
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::Time(Duration::from_secs(5)),
                original_measure_beat: None,
                bpm: None,
                time_signature: Some(TimeSignature::new(3, 4)),
                transition: TempoTransition::Snap,
            }],
        );
        let ts_before = map.time_signature_at_time(Duration::from_secs(3), 0.0);
        assert_eq!(ts_before, TimeSignature::new(4, 4));

        let ts_after = map.time_signature_at_time(Duration::from_secs(6), 0.0);
        assert_eq!(ts_after, TimeSignature::new(3, 4));
    }

    // ── TempoMap::new: sorting and resolution ───────────────────────

    #[test]
    fn tempo_map_sorts_changes_by_time() {
        // Changes given out of order should be sorted
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![
                TempoChange {
                    position: TempoChangePosition::Time(Duration::from_secs(8)),
                    original_measure_beat: None,
                    bpm: Some(180.0),
                    time_signature: None,
                    transition: TempoTransition::Snap,
                },
                TempoChange {
                    position: TempoChangePosition::Time(Duration::from_secs(4)),
                    original_measure_beat: None,
                    bpm: Some(60.0),
                    time_signature: None,
                    transition: TempoTransition::Snap,
                },
            ],
        );
        // Should respect ordering: 120 -> 60 at t=4 -> 180 at t=8
        let bpm_5 = map.bpm_at_time(Duration::from_secs(5), 0.0);
        assert!((bpm_5 - 60.0).abs() < f64::EPSILON);
        let bpm_9 = map.bpm_at_time(Duration::from_secs(9), 0.0);
        assert!((bpm_9 - 180.0).abs() < f64::EPSILON);
    }

    // ── Changes before bar 1 / mid-bar metre changes ─────────────────

    fn snap_change(
        position: TempoChangePosition,
        bpm: Option<f64>,
        time_signature: Option<TimeSignature>,
    ) -> TempoChange {
        TempoChange {
            position,
            original_measure_beat: None,
            bpm,
            time_signature,
            transition: TempoTransition::Snap,
        }
    }

    /// A time-positioned change before bar 1 counts as at bar 1, and the
    /// beat walks do not panic on it.
    #[test]
    fn a_time_change_before_bar_one_counts_as_at_bar_one() {
        let map = TempoMap::new(
            Duration::from_secs(5),
            120.0,
            TimeSignature::new(4, 4),
            vec![snap_change(
                TempoChangePosition::Time(Duration::from_secs(2)),
                Some(140.0),
                None,
            )],
        );
        // Bar 2 is a bar of 140 BPM after bar 1 at 5s.
        let bar2 = map.measure_to_time_with_offset(2, 1.0, 0, 0.0).unwrap();
        assert!(
            (bar2.as_secs_f64() - (5.0 + 4.0 * 60.0 / 140.0)).abs() < 1e-9,
            "{bar2:?}"
        );
        assert!(!map.beat_clock_ticks(Duration::from_secs(10)).is_empty());
    }

    /// `bpm_at_time` reads the tempo the beat walk has: a ramp that starts
    /// inside another starts from the tempo the walk reached it at (it used
    /// to be read a nanosecond before the change), and a ramp change before
    /// bar 1 runs from bar 1, where the walk starts it (it used to run from
    /// the change's own time, and be over by bar 1).
    #[test]
    fn bpm_at_time_reads_the_walk() {
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![
                ramp_change(
                    TempoChangePosition::MeasureBeat(2, 1.0),
                    180.0,
                    TempoTransition::Beats(8.0, TransitionCurve::Linear),
                ),
                ramp_change(
                    TempoChangePosition::MeasureBeat(3, 1.0),
                    60.0,
                    TempoTransition::Beats(4.0, TransitionCurve::Linear),
                ),
            ],
        );
        let bar3 = map.changes[1].position.absolute_time().unwrap();
        let second = map.segments().states[2].ramp.unwrap();
        assert_eq!(map.bpm_at_time(bar3, 0.0), second.old_bpm);
        assert!(second.old_bpm > 120.0 && second.old_bpm < 180.0);
        let mid = bar3 + Duration::from_secs_f64(second.total_secs / 2.0);
        assert!((map.bpm_at_time(mid, 0.0) - (second.old_bpm + 60.0) / 2.0).abs() < 1e-6);

        // A 4-beat ramp from 120 to 60 at 2 s, with bar 1 at 5 s: it runs
        // from 5 s, over 8/3 s.
        let early = TempoMap::new(
            Duration::from_secs(5),
            120.0,
            TimeSignature::new(4, 4),
            vec![ramp_change(
                TempoChangePosition::Time(Duration::from_secs(2)),
                60.0,
                TempoTransition::Beats(4.0, TransitionCurve::Linear),
            )],
        );
        assert_eq!(early.bpm_at_time(Duration::from_secs(1), 0.0), 120.0);
        assert_eq!(early.bpm_at_time(Duration::from_secs(5), 0.0), 120.0);
        assert!((early.bpm_at_time(Duration::from_millis(6_333), 0.0) - 90.0).abs() < 0.1);
        assert_eq!(early.bpm_at_time(Duration::from_secs(8), 0.0), 60.0);
    }

    /// A time's beat position is read by the one walk lengths and cue
    /// positions use: a bar-positioned change at its written beats, a
    /// time-positioned one placed through them, exactly where `TempoMap::new`
    /// put it. (Reading it off the changes' resolved times instead placed a
    /// bar-positioned change a few nanoseconds' worth of beats off.) A time
    /// before bar 1 is still bar 1 to the grid, and still a negative beat
    /// position to a length.
    #[test]
    fn a_time_is_placed_by_the_walk_lengths_use() {
        let map = TempoMap::new(
            Duration::from_millis(250),
            169.0,
            TimeSignature::new(4, 4),
            vec![
                snap_change(TempoChangePosition::MeasureBeat(3, 1.0), Some(150.0), None),
                snap_change(
                    TempoChangePosition::Time(Duration::from_secs(7)),
                    Some(100.0),
                    None,
                ),
            ],
        );
        let segments = map.segments();
        let timed = map.changes[1].position.absolute_time().unwrap();
        assert_eq!(segments.beats_at(timed), segments.states[2].beats);
        // Bar 3 at 0.25 + 480/169 s; 5 s is the rest of that at 150.
        let at5 = segments.beats_at(Duration::from_secs(5));
        let want = 8.0 + (5.0 - 0.25 - 480.0 / 169.0) * 2.5;
        assert!((at5 - want).abs() < 1e-9, "{at5}");
        assert_eq!(
            map.time_to_measure_beat(Duration::from_millis(100)),
            (1, 1.0)
        );
        // One beat from 100 ms counts the lead-in at the starting tempo:
        // 60/169 s, not the 150 ms to bar 1 and then a beat.
        let beat = map.beats_to_duration(1.0, Duration::from_millis(100), 0.0);
        assert!((beat.as_secs_f64() - 60.0 / 169.0).abs() < 1e-9, "{beat:?}");
    }

    /// A metre change anchored mid-bar governs from the next bar (as the
    /// beat grid, the clock and `time_signature_at_measure` have it). A
    /// later tempo change then takes effect where the grid puts its bar,
    /// for `bpm_at_time` too.
    #[test]
    fn bpm_at_time_agrees_with_the_grid_after_a_mid_bar_metre_change() {
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![
                snap_change(
                    TempoChangePosition::MeasureBeat(2, 3.0),
                    None,
                    Some(TimeSignature::new(3, 4)),
                ),
                snap_change(TempoChangePosition::MeasureBeat(4, 1.0), Some(60.0), None),
            ],
        );
        // Bar 1 (4) + bar 2 (4, the change is mid-bar) + bar 3 (3) = 11 beats.
        let bar4 = map.measure_to_time_with_offset(4, 1.0, 0, 0.0).unwrap();
        assert!((bar4.as_secs_f64() - 5.5).abs() < 1e-9, "{bar4:?}");
        // Clock pulses still at 120 just before bar 4.
        let ticks = map.beat_clock_ticks(Duration::from_secs(7));
        let gap = ticks
            .windows(2)
            .find(|w| w[1] > Duration::from_millis(5300))
            .unwrap();
        assert!(((gap[1] - gap[0]).as_secs_f64() - 0.5 / 24.0).abs() < 1e-6);
        // ... and bpm_at_time agrees, either side of bar 4.
        assert_eq!(map.bpm_at_time(Duration::from_millis(5250), 0.0), 120.0);
        assert_eq!(map.bpm_at_time(Duration::from_millis(5600), 0.0), 60.0);
    }

    /// Bar-positioned changes after a time-positioned one are reached at the
    /// tempo it set, not at the tempo before it.
    #[test]
    fn a_bar_change_after_a_time_change_counts_through_its_tempo() {
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![
                snap_change(
                    TempoChangePosition::Time(Duration::from_secs(10)),
                    Some(60.0),
                    None,
                ),
                snap_change(TempoChangePosition::MeasureBeat(20, 1.0), Some(100.0), None),
            ],
        );
        // 10s at 120 is bar 6; bars 6 to 19 are 56 beats at 60.
        let resolved = map.changes[1].position.absolute_time().unwrap();
        assert!((resolved.as_secs_f64() - 66.0).abs() < 1e-9, "{resolved:?}");
        let bar20 = map.measure_to_time_with_offset(20, 1.0, 0, 0.0).unwrap();
        assert!((bar20.as_secs_f64() - 66.0).abs() < 1e-9, "{bar20:?}");
        assert_eq!(map.bpm_at_time(Duration::from_secs(65), 0.0), 60.0);
        assert_eq!(map.bpm_at_time(Duration::from_millis(66_500), 0.0), 100.0);
    }

    /// A time-positioned change later than a bar-positioned one is taken in
    /// time order, after it, rather than ahead of every bar-positioned
    /// change (which counted bar 3 back from 30s and panicked).
    #[test]
    fn changes_resolve_in_time_order_whatever_their_positions() {
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![
                snap_change(
                    TempoChangePosition::Time(Duration::from_secs(30)),
                    Some(140.0),
                    None,
                ),
                snap_change(TempoChangePosition::MeasureBeat(3, 1.0), Some(100.0), None),
            ],
        );
        let times: Vec<_> = map
            .changes
            .iter()
            .map(|change| change.position.absolute_time().unwrap())
            .collect();
        assert_eq!(times, vec![Duration::from_secs(4), Duration::from_secs(30)]);
        assert_eq!(map.bpm_at_time(Duration::from_secs(10), 0.0), 100.0);
        assert_eq!(map.bpm_at_time(Duration::from_secs(31), 0.0), 140.0);
        // Bar 3 at 4s; 26s at 100 is 43 1/3 beats on, then 140.
        let bar16 = map.measure_to_time_with_offset(16, 1.0, 0, 0.0).unwrap();
        let expected = 30.0 + (60.0 - 8.0 - 26.0 * 100.0 / 60.0) * 60.0 / 140.0;
        assert!((bar16.as_secs_f64() - expected).abs() < 1e-9, "{bar16:?}");
    }

    /// A time-positioned change inside a ramp is placed through the ramp,
    /// and the grid takes it at its own time, as the tempo map does.
    #[test]
    fn a_time_change_after_a_ramp_counts_through_the_ramp() {
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![
                TempoChange {
                    position: TempoChangePosition::MeasureBeat(1, 1.0),
                    original_measure_beat: None,
                    bpm: Some(60.0),
                    time_signature: None,
                    transition: TempoTransition::Beats(4.0, TransitionCurve::Linear),
                },
                snap_change(
                    TempoChangePosition::Time(Duration::from_secs(10)),
                    Some(100.0),
                    None,
                ),
                snap_change(TempoChangePosition::MeasureBeat(20, 1.0), Some(80.0), None),
            ],
        );
        // The ramp spans its 4 beats in 8/3s (an average of 90 BPM); then
        // 22/3s at 60 is 22/3 beats, so 10s is beat 34/3; bar 20 (beat 76)
        // is 194/3 beats on at 100: 38.8s.
        let resolved = map.changes[2].position.absolute_time().unwrap();
        assert!((resolved.as_secs_f64() - 48.8).abs() < 1e-6, "{resolved:?}");
        let bar20 = map.measure_to_time_with_offset(20, 1.0, 0, 0.0).unwrap();
        assert!((bar20.as_secs_f64() - 48.8).abs() < 1e-6, "{bar20:?}");
        // The clock pulses at 100 from 10s.
        let ticks = map.beat_clock_ticks(Duration::from_secs(12));
        let gap = ticks
            .windows(2)
            .find(|w| w[0] >= Duration::from_millis(10_001))
            .unwrap();
        assert!(((gap[1] - gap[0]).as_secs_f64() - 0.6 / 24.0).abs() < 1e-6);
    }

    /// The time signature a mid-bar change sets holds from the next bar,
    /// where the grid starts counting in it.
    #[test]
    fn time_signature_at_time_follows_a_mid_bar_change_from_the_next_bar() {
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![snap_change(
                TempoChangePosition::MeasureBeat(2, 3.0),
                None,
                Some(TimeSignature::new(3, 4)),
            )],
        );
        let at = |ms| map.time_signature_at_time(Duration::from_millis(ms), 0.0);
        assert_eq!(at(2_500), TimeSignature::new(4, 4));
        assert_eq!(at(3_200), TimeSignature::new(4, 4));
        assert_eq!(at(4_000), TimeSignature::new(3, 4));
        assert_eq!(at(5_000), TimeSignature::new(3, 4));
    }

    /// A time-positioned signature change after a ramp is placed in the bar
    /// its beat position falls in, counted through the ramp.
    #[test]
    fn a_time_signature_change_after_a_ramp_is_placed_through_the_ramp() {
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![
                TempoChange {
                    position: TempoChangePosition::MeasureBeat(1, 1.0),
                    original_measure_beat: None,
                    bpm: Some(60.0),
                    time_signature: None,
                    transition: TempoTransition::Beats(4.0, TransitionCurve::Linear),
                },
                snap_change(
                    TempoChangePosition::Time(Duration::from_millis(7_500)),
                    None,
                    Some(TimeSignature::new(3, 4)),
                ),
            ],
        );
        // The ramp covers its 4 beats in 8/3s, then 29/6s at 60: beat
        // 53/6, which is bar 3 beat 11/6, so 3/4 governs from bar 4 (beat
        // 12, at 32/3s).
        let (m, b) = map.time_to_measure_beat(Duration::from_millis(7_500));
        assert_eq!(m, 3);
        assert!((b - 11.0 / 6.0).abs() < 1e-6, "{b}");
        let bar4 = map.measure_to_time_with_offset(4, 1.0, 0, 0.0).unwrap();
        let bar5 = map.measure_to_time_with_offset(5, 1.0, 0, 0.0).unwrap();
        assert!((bar4.as_secs_f64() - 32.0 / 3.0).abs() < 1e-6, "{bar4:?}");
        assert!((bar5.as_secs_f64() - 41.0 / 3.0).abs() < 1e-6, "{bar5:?}");
        let at = |ms| map.time_signature_at_time(Duration::from_millis(ms), 0.0);
        assert_eq!(at(10_600), TimeSignature::new(4, 4));
        assert_eq!(at(10_700), TimeSignature::new(3, 4));
    }

    /// With the map shifted by an offset, a mid-bar signature change still
    /// waits for the (shifted) next barline.
    #[test]
    fn time_signature_at_time_waits_for_the_next_bar_under_an_offset() {
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![snap_change(
                TempoChangePosition::MeasureBeat(2, 3.0),
                None,
                Some(TimeSignature::new(3, 4)),
            )],
        );
        let at = |ms| map.time_signature_at_time(Duration::from_millis(ms), 10.0);
        assert_eq!(at(12_500), TimeSignature::new(4, 4));
        assert_eq!(at(13_200), TimeSignature::new(4, 4));
        assert_eq!(at(14_000), TimeSignature::new(3, 4));
    }

    /// A ramp in measures sized in the signature set at its position, by
    /// its own change or another at the same position, listed before or
    /// after it.
    #[test]
    fn a_ramp_in_measures_counts_a_signature_set_beside_it_in_either_order() {
        let ramp = TempoChange {
            position: TempoChangePosition::MeasureBeat(5, 1.0),
            original_measure_beat: None,
            bpm: Some(100.0),
            time_signature: None,
            transition: TempoTransition::Measures(2.0, TransitionCurve::Linear),
        };
        let metre = snap_change(
            TempoChangePosition::MeasureBeat(5, 1.0),
            None,
            Some(TimeSignature::new(3, 4)),
        );
        let ramp_ends = |changes| {
            let map = TempoMap::new(Duration::ZERO, 120.0, TimeSignature::new(4, 4), changes);
            // Bar 5 at 8s; a ramp of 2 bars of 3/4 spans 6 beats at an
            // average of 110: 36/11s, ending at about 11.273s.
            let bpm = map.bpm_at_time(Duration::from_millis(11_300), 0.0);
            let mid = map.bpm_at_time(Duration::from_millis(11_250), 0.0);
            let bar7 = map.measure_to_time_with_offset(7, 1.0, 0, 0.0).unwrap();
            (bpm, mid, bar7)
        };
        let ramp_first = ramp_ends(vec![ramp.clone(), metre.clone()]);
        let metre_first = ramp_ends(vec![metre, ramp]);
        assert_eq!(ramp_first.0, 100.0);
        assert!(ramp_first.1 > 100.0, "{ramp_first:?}");
        assert_eq!(ramp_first.0, metre_first.0);
        assert!((ramp_first.1 - metre_first.1).abs() < 1e-9);
        assert!(
            (ramp_first.2.as_secs_f64() - metre_first.2.as_secs_f64()).abs() < 1e-9,
            "{ramp_first:?} {metre_first:?}"
        );
    }

    // ── Ramp length: "over N beats" spans exactly N beats ───────────

    /// Every curve a ramp can take. The match fails to compile when a curve
    /// is added, so the ramp tests below cover it too.
    fn all_curves() -> Vec<TransitionCurve> {
        let all = vec![TransitionCurve::Linear];
        for curve in &all {
            match curve {
                TransitionCurve::Linear => {}
            }
        }
        all
    }

    fn ramp_change(
        position: TempoChangePosition,
        bpm: f64,
        transition: TempoTransition,
    ) -> TempoChange {
        TempoChange {
            position,
            original_measure_beat: None,
            bpm: Some(bpm),
            time_signature: None,
            transition,
        }
    }

    fn secs(time: Duration) -> f64 {
        time.as_secs_f64()
    }

    /// A ramp over N beats ends exactly N beats after it starts, at the
    /// target tempo, for every curve; the grid, the clock and the tempo
    /// lookup all agree on where it ends.
    #[test]
    fn a_ramp_over_n_beats_spans_exactly_n_beats() {
        for curve in all_curves() {
            for (from, to) in [
                (120.0, 180.0),
                (180.0, 120.0),
                (90.0, 140.0),
                (100.0, 100.0),
            ] {
                for beats in [1.0, 4.0, 6.5, 8.0] {
                    let what = format!("{curve:?} {from}->{to} over {beats}");
                    let map = TempoMap::new(
                        Duration::from_millis(500),
                        from,
                        TimeSignature::new(4, 4),
                        vec![ramp_change(
                            TempoChangePosition::MeasureBeat(3, 1.0),
                            to,
                            TempoTransition::Beats(beats, curve),
                        )],
                    );
                    let start = map.changes[0].position.absolute_time().unwrap();
                    assert!(
                        (secs(start) - (0.5 + 8.0 * 60.0 / from)).abs() < 1e-9,
                        "{what}"
                    );
                    let length = curve.duration_for_beats(from, to, beats);
                    // The curve covers exactly N beats in that time.
                    assert!(
                        (curve.beats_in_remaining_transition(from, to, length, 0.0) - beats).abs()
                            < 1e-9,
                        "{what}"
                    );
                    let end = secs(start) + length;

                    // Time to beats: N beats on at the end.
                    let segments = map.segments();
                    let covered =
                        segments.beats_at(Duration::from_secs_f64(end)) - segments.beats_at(start);
                    assert!((covered - beats).abs() < 1e-6, "{what}: {covered}");
                    // Beats to time: beat 8 + N is the end, and the beats
                    // after it are at the target tempo.
                    let at_end = segments.time_at(8.0 + beats);
                    assert!((secs(at_end) - end).abs() < 1e-6, "{what}: {at_end:?}");
                    let later = segments.time_at(8.0 + beats + 4.0);
                    assert!(
                        (secs(later) - (end + 4.0 * 60.0 / to)).abs() < 1e-6,
                        "{what}: {later:?}"
                    );
                    // The tempo reaches the target exactly there.
                    let after = map.bpm_at_time(Duration::from_secs_f64(end + 1e-6), 0.0);
                    assert!((after - to).abs() < 1e-9, "{what}: {after}");
                    if from != to {
                        let before = map.bpm_at_time(Duration::from_secs_f64(end - 1e-3), 0.0);
                        assert!((before - to).abs() > 1e-3, "{what}: {before}");
                    }
                    // The clock's pulse for beat 8 + N falls at the end.
                    let ticks = map.beat_clock_ticks(Duration::from_secs_f64(end + 1.0));
                    let lead = ticks.iter().filter(|t| **t < start).count() - (8 * 24);
                    let pulse = ticks[lead + ((8.0 + beats) * 24.0) as usize];
                    assert!((secs(pulse) - end).abs() < 1e-6, "{what}: {pulse:?}");
                }
            }
        }
    }

    /// A ramp over N measures spans exactly the beats in those measures, in
    /// the metre at its position: it ends on the barline N bars on.
    #[test]
    fn a_ramp_over_n_measures_spans_exactly_those_bars() {
        for curve in all_curves() {
            for ts in [
                TimeSignature::new(4, 4),
                TimeSignature::new(3, 4),
                TimeSignature::new(6, 8),
                TimeSignature::new(7, 8),
            ] {
                for (from, to) in [(120.0, 180.0), (169.0, 100.0)] {
                    let what = format!("{curve:?} {ts:?} {from}->{to}");
                    let map = TempoMap::new(
                        Duration::ZERO,
                        from,
                        ts,
                        vec![ramp_change(
                            TempoChangePosition::MeasureBeat(3, 1.0),
                            to,
                            TempoTransition::Measures(2.0, curve),
                        )],
                    );
                    let per_bar = ts.beats_per_measure();
                    let bar3 = 2.0 * per_bar * 60.0 / from;
                    let bar5 = bar3 + curve.duration_for_beats(from, to, 2.0 * per_bar);
                    let bar6 = bar5 + per_bar * 60.0 / to;
                    assert!((secs(bar(&map, 3, 0.0)) - bar3).abs() < 1e-9, "{what}");
                    assert!((secs(bar(&map, 5, 0.0)) - bar5).abs() < 1e-6, "{what}");
                    assert!((secs(bar(&map, 6, 0.0)) - bar6).abs() < 1e-6, "{what}");
                    let at_bar5 = map.bpm_at_time(Duration::from_secs_f64(bar5 + 1e-6), 0.0);
                    assert!((at_bar5 - to).abs() < 1e-9, "{what}: {at_bar5}");
                    let (m, b) = map.time_to_measure_beat(Duration::from_secs_f64(bar5 + 1e-7));
                    assert_eq!(m, 5, "{what}");
                    assert!((b - 1.0).abs() < 1e-5, "{what}: {b}");
                }
            }
        }
    }

    /// A change written after a ramp lands on its own bar: the bars the
    /// ramp spans are whole bars, so nothing after it drifts.
    #[test]
    fn a_change_after_a_ramp_lands_on_its_written_bar() {
        for curve in all_curves() {
            let map = TempoMap::new(
                Duration::ZERO,
                120.0,
                TimeSignature::new(4, 4),
                vec![
                    ramp_change(
                        TempoChangePosition::MeasureBeat(3, 1.0),
                        180.0,
                        TempoTransition::Beats(8.0, curve),
                    ),
                    snap_change(TempoChangePosition::MeasureBeat(7, 1.0), Some(90.0), None),
                ],
            );
            // Bar 3 at 4s; 8 beats from 120 to 180 average 150: 3.2s, so
            // bar 5 at 7.2s; two bars at 180 put bar 7 at 9.8667s.
            let bar7 = 7.2 + 8.0 * 60.0 / 180.0;
            let change = map.changes[1].position.absolute_time().unwrap();
            assert!((secs(change) - bar7).abs() < 1e-6, "{curve:?} {change:?}");
            assert!((secs(bar(&map, 5, 0.0)) - 7.2).abs() < 1e-6, "{curve:?}");
            assert!((secs(bar(&map, 7, 0.0)) - bar7).abs() < 1e-6, "{curve:?}");
            let bar8 = bar7 + 4.0 * 60.0 / 90.0;
            assert!((secs(bar(&map, 8, 0.0)) - bar8).abs() < 1e-6, "{curve:?}");
            let (m, b) = map.time_to_measure_beat(change);
            assert_eq!(m, 7, "{curve:?}");
            assert!((b - 1.0).abs() < 1e-6, "{curve:?}: {b}");
            assert_eq!(map.bpm_at_time(change, 0.0), 90.0);
        }
    }

    /// Time to bar and beat and back, and spans in beats forward and back,
    /// land where they started all through a ramp.
    #[test]
    fn time_and_beats_round_trip_through_a_ramp() {
        for curve in all_curves() {
            for to in [180.0, 70.0] {
                let map = TempoMap::new(
                    Duration::from_millis(250),
                    120.0,
                    TimeSignature::new(4, 4),
                    vec![ramp_change(
                        TempoChangePosition::MeasureBeat(2, 3.0),
                        to,
                        TempoTransition::Beats(7.0, curve),
                    )],
                );
                for ms in (0..12_000).step_by(137) {
                    let t = Duration::from_millis(250 + ms);
                    let (m, b) = map.time_to_measure_beat(t);
                    let back = map.measure_to_time_with_offset(m, b, 0, 0.0).unwrap();
                    assert!(
                        (secs(back) - secs(t)).abs() < 1e-6,
                        "{curve:?} to {to} at {t:?}: bar {m} beat {b} is {back:?}"
                    );
                    let span = map.beats_to_duration(2.5, t, 0.0);
                    let back = map.beats_before(2.5, t + span);
                    assert!(
                        (secs(back) - secs(span)).abs() < 1e-6,
                        "{curve:?} to {to} at {t:?}: {span:?} vs {back:?}"
                    );
                }
            }
        }
    }

    /// Snaps and steady tempos time exactly as they always have: only a
    /// ramp's own length changed.
    #[test]
    fn snaps_and_steady_tempos_time_as_before() {
        let map = TempoMap::new(
            Duration::from_secs(1),
            120.0,
            TimeSignature::new(4, 4),
            vec![
                snap_change(TempoChangePosition::MeasureBeat(3, 1.0), Some(150.0), None),
                snap_change(
                    TempoChangePosition::MeasureBeat(5, 1.0),
                    Some(100.0),
                    Some(TimeSignature::new(3, 4)),
                ),
                snap_change(
                    TempoChangePosition::Time(Duration::from_secs(20)),
                    Some(60.0),
                    None,
                ),
            ],
        );
        // Bar 3 at 1 + 8 * 0.5 = 5s; bar 5 at 5 + 8 * 0.4 = 8.2s; bar 7 at
        // 8.2 + 6 * 0.6 = 11.8s.
        assert_eq!(secs(bar(&map, 1, 0.0)), 1.0);
        assert!((secs(bar(&map, 3, 0.0)) - 5.0).abs() < 1e-9);
        assert!((secs(bar(&map, 5, 0.0)) - 8.2).abs() < 1e-9);
        assert!((secs(bar(&map, 7, 0.0)) - 11.8).abs() < 1e-9);
        // 20s is 11.8s past bar 5 at 100: 19 2/3 beats, bar 11 beat 2 2/3.
        let (m, b) = map.time_to_measure_beat(Duration::from_secs(20));
        assert_eq!(m, 11);
        assert!((b - (1.0 + 59.0 / 3.0 - 18.0)).abs() < 1e-6, "{b}");
        // Bar 12 is 1 1/3 beats on at 60.
        assert!((secs(bar(&map, 12, 0.0)) - (20.0 + 4.0 / 3.0)).abs() < 1e-6);
        assert_eq!(map.bpm_at_time(Duration::from_secs(6), 0.0), 150.0);
        assert_eq!(map.bpm_at_time(Duration::from_secs(21), 0.0), 60.0);

        // A ramp to the tempo it starts at times as a steady tempo.
        for curve in all_curves() {
            let steady = TempoMap::new(
                Duration::ZERO,
                120.0,
                TimeSignature::new(4, 4),
                vec![ramp_change(
                    TempoChangePosition::MeasureBeat(2, 1.0),
                    120.0,
                    TempoTransition::Measures(2.0, curve),
                )],
            );
            for m in 1..10 {
                assert!(
                    (secs(bar(&steady, m, 0.0)) - (m - 1) as f64 * 2.0).abs() < 1e-9,
                    "{curve:?} bar {m}"
                );
            }
        }
    }
}
