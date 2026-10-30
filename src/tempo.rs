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

    /// Calculate how many beats occur during a transition from elapsed=0 to elapsed=dt
    /// Returns the integral of bpm(t)/60 dt from 0 to dt
    pub fn beats_in_duration(
        &self,
        old_bpm: f64,
        new_bpm: f64,
        total_duration: f64,
        dt: f64,
    ) -> f64 {
        match self {
            TransitionCurve::Linear => {
                // beats = integral(bpm(t)/60 dt) from 0 to dt
                // = (1/60) * [old_bpm * dt + (new_bpm - old_bpm) * dt^2 / (2*T)]
                (old_bpm * dt + (new_bpm - old_bpm) * dt * dt / (2.0 * total_duration)) / 60.0
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
    /// Returns the time duration needed for the given number of beats
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
                    if discriminant >= 0.0 {
                        Some((-b + discriminant.sqrt()) / (2.0 * a))
                    } else {
                        // Fallback to average BPM
                        let current_bpm = old_bpm + (new_bpm - old_bpm) * elapsed / total_duration;
                        Some(beats * 60.0 / ((current_bpm + new_bpm) / 2.0))
                    }
                }
            }
        }
    }

    /// Solve for duration dt given a number of beats, starting from the beginning of the transition
    /// This is a convenience method for the common case
    pub fn solve_duration_for_beats_from_start(
        &self,
        old_bpm: f64,
        new_bpm: f64,
        total_duration: f64,
        beats: f64,
    ) -> Option<f64> {
        self.solve_duration_for_beats(old_bpm, new_bpm, total_duration, 0.0, beats)
    }
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
                .unwrap_or_else(|| beats * 60.0 / ((self.old_bpm + self.new_bpm) / 2.0))
        } else {
            (self.total_secs - self.elapsed_secs) + (beats - remaining) * 60.0 / settled_bpm
        }
    }

    /// Beats covered in `secs` from the current position, integrating through
    /// the remainder of the ramp and then continuing at `settled_bpm`. The
    /// inverse of [`SegmentRamp::secs_for_beats`].
    fn beats_in_secs(&self, secs: f64, settled_bpm: f64) -> f64 {
        let secs_left = self.total_secs - self.elapsed_secs;
        let remaining = self.beats_remaining();
        if secs >= secs_left {
            remaining + (secs - secs_left) * settled_bpm / 60.0
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
        None => beats * 60.0 / bpm,
    }
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

/// Where [`TempoMap::integrate_through_segments`] stands just past a change:
/// the beat position and time of the change, and the tempo from there.
struct SegmentState {
    beats: f64,
    time: Duration,
    bpm: f64,
    ramp: Option<SegmentRamp>,
}

impl SegmentState {
    /// The time of a beat position at or past this state's, before the next.
    fn time_at(&self, beats: f64) -> Duration {
        self.time
            + Duration::from_secs_f64(segment_secs_for_beats(
                self.ramp.as_ref(),
                self.bpm,
                beats - self.beats,
            ))
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
        let mut resolved_changes = Vec::new();
        let mut current_bpm = initial_bpm;
        let mut current_time_sig = initial_time_signature;
        let mut accumulated_time = start_offset;
        let mut accumulated_beats = 0.0;

        // Sort changes by their position (approximate - measure/beat vs time)
        let mut sorted_changes = changes;
        sorted_changes.sort_by(|a, b| match (&a.position, &b.position) {
            (TempoChangePosition::Time(ta), TempoChangePosition::Time(tb)) => ta.cmp(tb),
            (
                TempoChangePosition::MeasureBeat(ma, ba),
                TempoChangePosition::MeasureBeat(mb, bb),
            ) => ma
                .cmp(mb)
                .then_with(|| ba.partial_cmp(bb).unwrap_or(std::cmp::Ordering::Equal)),
            (TempoChangePosition::Time(_), TempoChangePosition::MeasureBeat(_, _)) => {
                std::cmp::Ordering::Less
            }
            (TempoChangePosition::MeasureBeat(_, _), TempoChangePosition::Time(_)) => {
                std::cmp::Ordering::Greater
            }
        });

        let mut current_ramp: Option<SegmentRamp> = None;

        for change in sorted_changes {
            // Resolve the change's absolute time, integrating through any
            // still-in-progress gradual transition from the previous change.
            let (absolute_time, advanced_secs) = match &change.position {
                TempoChangePosition::Time(t) => {
                    (*t, t.saturating_sub(accumulated_time).as_secs_f64())
                }
                TempoChangePosition::MeasureBeat(m, b) => {
                    let total_beats =
                        (*m - 1) as f64 * current_time_sig.beats_per_measure() + (*b - 1.0);
                    let beats_from_last_change = total_beats - accumulated_beats;
                    let secs = segment_secs_for_beats(
                        current_ramp.as_ref(),
                        current_bpm,
                        beats_from_last_change,
                    );
                    (accumulated_time + Duration::from_secs_f64(secs), secs)
                }
            };

            // Create resolved change, preserving original measure/beat if it was one
            // Use the original_measure_beat from the change if it exists, otherwise extract from position
            let original_measure_beat = change.original_measure_beat.or(match &change.position {
                TempoChangePosition::MeasureBeat(m, b) => Some((*m, *b)),
                TempoChangePosition::Time(_) => None,
            });
            let resolved_change = TempoChange {
                position: TempoChangePosition::Time(absolute_time),
                original_measure_beat,
                bpm: change.bpm,
                time_signature: change.time_signature,
                transition: change.transition,
            };

            resolved_changes.push(resolved_change);

            // Update tempo state for next iteration. The BPM actually in
            // effect at this change may be a mid-ramp value.
            let bpm_at_change = match current_ramp.as_ref() {
                Some(ramp) => ramp.bpm_after_secs(advanced_secs),
                None => current_bpm,
            };
            current_ramp = current_ramp.and_then(|ramp| ramp.advanced_by(advanced_secs));
            if let Some(new_ts) = change.time_signature {
                current_time_sig = new_ts;
            }
            if let Some(new_bpm) = change.bpm {
                match change.transition {
                    TempoTransition::Snap => current_ramp = None,
                    TempoTransition::Beats(beats, curve) => {
                        current_ramp = Some(SegmentRamp {
                            old_bpm: bpm_at_change,
                            new_bpm,
                            total_secs: beats * 60.0 / bpm_at_change,
                            elapsed_secs: 0.0,
                            curve,
                        });
                    }
                    TempoTransition::Measures(measures, curve) => {
                        // Matches bpm_at_time: the duration is computed with
                        // the time signature in effect after this change.
                        let beats = measures * current_time_sig.beats_per_measure();
                        current_ramp = Some(SegmentRamp {
                            old_bpm: bpm_at_change,
                            new_bpm,
                            total_secs: beats * 60.0 / bpm_at_change,
                            elapsed_secs: 0.0,
                            curve,
                        });
                    }
                }
                current_bpm = new_bpm;
            }

            // Update accumulated position
            match &change.position {
                TempoChangePosition::MeasureBeat(m, b) => {
                    accumulated_beats =
                        (m - 1) as f64 * current_time_sig.beats_per_measure() + (b - 1.0);
                    accumulated_time = absolute_time;
                }
                TempoChangePosition::Time(t) => {
                    // Convert time back to beats for tracking
                    accumulated_beats =
                        (t.as_secs_f64() - start_offset.as_secs_f64()) * current_bpm / 60.0;
                    accumulated_time = *t;
                }
            }
        }

        // Sort by absolute time (now all are Time positions)
        resolved_changes.sort_by(|a, b| {
            a.position
                .absolute_time()
                .unwrap_or(Duration::ZERO)
                .cmp(&b.position.absolute_time().unwrap_or(Duration::ZERO))
        });

        TempoMap {
            start_offset,
            initial_bpm,
            initial_time_signature,
            changes: resolved_changes,
        }
    }

    /// Convert a measure/beat position to absolute time with an offset.
    /// The offset is applied to both the target position and tempo change positions.
    pub fn measure_to_time_with_offset(
        &self,
        measure: u32,
        beat: f64,
        measure_offset: u32,
        offset_secs: f64,
    ) -> Option<Duration> {
        // Measures are 1-indexed
        if measure < 1 {
            return None;
        }

        // Beat must be >= 1.0 (beats are 1-indexed)
        if beat < 1.0 {
            return None;
        }

        let offset_duration = Duration::from_secs_f64(offset_secs);

        // Build sorted list of time signature changes in (measure, beat, TimeSignature) form
        let ts_changes = self.build_time_signature_changes();

        // Apply offset to target measure (score measure -> playback measure)
        let playback_measure = measure + measure_offset;

        // Compute the total number of quarter-note beats from measure 1 to the target position,
        // accounting for time signature changes along the way
        let target_beats = Self::compute_target_beats(
            self.initial_time_signature,
            &ts_changes,
            playback_measure,
            beat,
        );

        // Walk through tempo changes, converting each to a beat position and accumulating time
        self.integrate_through_segments(target_beats, &ts_changes, offset_duration)
    }

    /// Build a sorted list of time signature changes as (measure, beat, TimeSignature).
    ///
    /// Uses `original_measure_beat` when available; otherwise converts absolute time
    /// back to measure/beat by integrating through prior changes.
    fn build_time_signature_changes(&self) -> Vec<(u32, f64, TimeSignature)> {
        let mut ts_changes: Vec<(u32, f64, TimeSignature)> = Vec::new();

        for change in &self.changes {
            if let Some(new_ts) = change.time_signature {
                if let Some((m, b)) = change.original_measure_beat {
                    ts_changes.push((m, b, new_ts));
                } else if let Some(change_time) = change.position.absolute_time() {
                    let (m, b) = self.time_to_measure_beat(change_time);
                    ts_changes.push((m, b, new_ts));
                }
            }
        }

        // Sort by measure then beat (ascending order)
        ts_changes.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        });

        ts_changes
    }

    /// Convert an absolute time to a (measure, beat) position by integrating through
    /// all tempo/time-signature changes that precede it.
    pub(crate) fn time_to_measure_beat(&self, target_time: Duration) -> (u32, f64) {
        let mut m: u32 = 1;
        let mut b: f64 = 1.0;
        let mut temp_time = self.start_offset;
        let mut temp_bpm = self.initial_bpm;
        let mut temp_ts = self.initial_time_signature;

        // Integrate through all changes before target_time
        for prev_change in &self.changes {
            if let Some(prev_time) = prev_change.position.absolute_time() {
                if prev_time >= target_time {
                    break;
                }
                Self::integrate_time_to_measure(
                    &mut m,
                    &mut b,
                    &mut temp_time,
                    prev_time,
                    temp_bpm,
                    temp_ts,
                );
                if let Some(new_bpm) = prev_change.bpm {
                    temp_bpm = new_bpm;
                }
                if let Some(new_ts) = prev_change.time_signature {
                    temp_ts = new_ts;
                }
            }
        }

        // Integrate from last change to target_time
        Self::integrate_time_to_measure(
            &mut m,
            &mut b,
            &mut temp_time,
            target_time,
            temp_bpm,
            temp_ts,
        );

        (m, b)
    }

    /// Advance (measure, beat, time) state forward from `current_time` to `end_time`
    /// at the given BPM and time signature, counting whole measures and fractional beats.
    fn integrate_time_to_measure(
        m: &mut u32,
        b: &mut f64,
        current_time: &mut Duration,
        end_time: Duration,
        bpm: f64,
        ts: TimeSignature,
    ) {
        while *current_time < end_time {
            let beats_per_measure = ts.beats_per_measure();
            let time_per_measure = Duration::from_secs_f64(beats_per_measure * 60.0 / bpm);
            if *current_time + time_per_measure <= end_time {
                *current_time += time_per_measure;
                *m += 1;
                *b = 1.0;
            } else {
                let remaining = end_time - *current_time;
                let remaining_beats = remaining.as_secs_f64() * bpm / 60.0;
                *b += remaining_beats;
                *current_time = end_time;
                break;
            }
        }
    }

    /// Compute total quarter-note beats from measure 1, beat 1 to the given
    /// (target_measure, target_beat), accounting for time signature changes.
    fn compute_target_beats(
        initial_time_signature: TimeSignature,
        ts_changes: &[(u32, f64, TimeSignature)],
        target_measure: u32,
        target_beat: f64,
    ) -> f64 {
        let mut target_beats = 0.0;
        let mut current_measure: u32 = 1;
        let mut current_beat_in_measure = 1.0;

        while current_measure < target_measure
            || (current_measure == target_measure && current_beat_in_measure < target_beat)
        {
            let ts_for_this_measure = Self::time_signature_at_measure(
                initial_time_signature,
                ts_changes,
                current_measure,
            );

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

    /// Look up the time signature in effect at a given measure number, using the sorted
    /// list of time signature changes. Changes apply at the start of their specified measure.
    fn time_signature_at_measure(
        initial_time_signature: TimeSignature,
        ts_changes: &[(u32, f64, TimeSignature)],
        measure: u32,
    ) -> TimeSignature {
        let mut ts = initial_time_signature;
        for (change_m, change_b, new_ts) in ts_changes {
            // Apply changes that occur before this measure, or exactly at
            // the start (beat ≈ 1) of this measure.
            if *change_m < measure || (*change_m == measure && (*change_b - 1.0).abs() < 0.001) {
                ts = *new_ts;
            }
        }
        ts
    }

    /// Compute the beat position of a tempo change, either from its original
    /// measure/beat or by converting its absolute time through prior tempo changes.
    fn compute_change_beats(
        &self,
        change: &TempoChange,
        change_time: Duration,
        ts_changes: &[(u32, f64, TimeSignature)],
    ) -> Option<f64> {
        if let Some((change_m, change_b)) = change.original_measure_beat {
            // Measure-based: integrate through measures (same logic as compute_target_beats)
            Some(Self::compute_target_beats(
                self.initial_time_signature,
                ts_changes,
                change_m,
                change_b,
            ))
        } else {
            // Time-based: convert time to beats by integrating through tempo changes
            let mut acc_time = self.start_offset;
            let mut acc_beats = 0.0;
            let mut acc_bpm = self.initial_bpm;

            for prev_change in &self.changes {
                let prev_change_time = prev_change.position.absolute_time()?;
                if prev_change_time >= change_time {
                    break;
                }

                let time_to_prev = prev_change_time - acc_time;
                acc_beats += time_to_prev.as_secs_f64() * acc_bpm / 60.0;
                acc_time = prev_change_time;

                if let Some(new_bpm) = prev_change.bpm {
                    acc_bpm = new_bpm;
                }
            }

            let time_to_this = change_time - acc_time;
            Some(acc_beats + time_to_this.as_secs_f64() * acc_bpm / 60.0)
        }
    }

    /// Walk through tempo changes in order, accumulating time until `target_beats`
    /// is reached. Returns the absolute time corresponding to the target beat position.
    fn integrate_through_segments(
        &self,
        target_beats: f64,
        ts_changes: &[(u32, f64, TimeSignature)],
        offset_duration: Duration,
    ) -> Option<Duration> {
        let mut current_bpm = self.initial_bpm;
        let mut accumulated_time = self.start_offset;
        let mut accumulated_beats = 0.0;
        let mut current_ramp: Option<SegmentRamp> = None;

        for change in &self.changes {
            let change_time = change.position.absolute_time()? + offset_duration;
            let change_beats = self.compute_change_beats(change, change_time, ts_changes)?;

            if change_beats > target_beats {
                // Target is before this change
                break;
            }

            // Process up to this change, integrating through any gradual
            // transition still in progress.
            let beats_to_change = change_beats - accumulated_beats;
            let advanced_secs =
                segment_secs_for_beats(current_ramp.as_ref(), current_bpm, beats_to_change);
            accumulated_time += Duration::from_secs_f64(advanced_secs);
            accumulated_beats = change_beats;

            // The BPM actually in effect at this change may be a mid-ramp value.
            let bpm_at_change = match current_ramp.as_ref() {
                Some(ramp) => ramp.bpm_after_secs(advanced_secs),
                None => current_bpm,
            };
            current_ramp = current_ramp.and_then(|ramp| ramp.advanced_by(advanced_secs));

            if let Some(new_bpm) = change.bpm {
                current_ramp = self.ramp_started_by(
                    change,
                    new_bpm,
                    bpm_at_change,
                    change_time,
                    offset_duration,
                );
                current_bpm = new_bpm;
            }
        }

        // Advance the remaining beats past the last processed change.
        let remaining_beats = target_beats - accumulated_beats;
        let result_time = accumulated_time
            + Duration::from_secs_f64(segment_secs_for_beats(
                current_ramp.as_ref(),
                current_bpm,
                remaining_beats,
            ));

        Some(result_time)
    }

    /// The state [`TempoMap::integrate_through_segments`] reaches at each
    /// change, from bar 1 on, so that the time of many beat positions can be
    /// read off without walking the changes again for each. The arithmetic is
    /// that walk's own, so a position lands exactly where it puts it. `None`
    /// where that walk gives none.
    fn segment_states(
        &self,
        ts_changes: &[(u32, f64, TimeSignature)],
    ) -> Option<Vec<SegmentState>> {
        let mut state = SegmentState {
            beats: 0.0,
            time: self.start_offset,
            bpm: self.initial_bpm,
            ramp: None,
        };
        let mut states = Vec::with_capacity(self.changes.len() + 1);
        for change in &self.changes {
            let change_time = change.position.absolute_time()?;
            let change_beats = self.compute_change_beats(change, change_time, ts_changes)?;

            let advanced_secs =
                segment_secs_for_beats(state.ramp.as_ref(), state.bpm, change_beats - state.beats);
            let bpm_at_change = match state.ramp.as_ref() {
                Some(ramp) => ramp.bpm_after_secs(advanced_secs),
                None => state.bpm,
            };
            let mut next = SegmentState {
                beats: change_beats,
                time: state.time + Duration::from_secs_f64(advanced_secs),
                bpm: state.bpm,
                ramp: state.ramp.and_then(|ramp| ramp.advanced_by(advanced_secs)),
            };
            if let Some(new_bpm) = change.bpm {
                next.ramp = self.ramp_started_by(
                    change,
                    new_bpm,
                    bpm_at_change,
                    change_time,
                    Duration::ZERO,
                );
                next.bpm = new_bpm;
            }
            states.push(std::mem::replace(&mut state, next));
        }
        states.push(state);
        Some(states)
    }

    /// The ramp a tempo change to `new_bpm` starts, given the BPM actually in
    /// effect when it is reached; `None` for a snap.
    fn ramp_started_by(
        &self,
        change: &TempoChange,
        new_bpm: f64,
        bpm_at_change: f64,
        change_time: Duration,
        offset_duration: Duration,
    ) -> Option<SegmentRamp> {
        let (beats, curve) = match change.transition {
            TempoTransition::Snap => return None,
            TempoTransition::Beats(beats, curve) => (beats, curve),
            TempoTransition::Measures(measures, curve) => {
                // Matches bpm_at_time: the duration is computed with the time
                // signature in effect at the change time (which includes this
                // change's own signature).
                let ts = self.time_signature_at_time(change_time, offset_duration.as_secs_f64());
                (measures * ts.beats_per_measure(), curve)
            }
        };
        Some(SegmentRamp {
            old_bpm: bpm_at_change,
            new_bpm,
            total_secs: beats * 60.0 / bpm_at_change,
            elapsed_secs: 0.0,
            curve,
        })
    }

    /// The quarter-note beat position (0 at bar 1 beat 1) of a score time,
    /// integrating through gradual transitions. The inverse of
    /// [`TempoMap::score_time_at_beats`]; both walk the changes exactly as
    /// [`TempoMap::integrate_through_segments`] does, so a position converted
    /// one way and back lands where a cue at that position fires.
    fn score_beats_at(
        &self,
        score_time: Duration,
        ts_changes: &[(u32, f64, TimeSignature)],
    ) -> f64 {
        if score_time < self.start_offset {
            return -(self.start_offset - score_time).as_secs_f64() * self.initial_bpm / 60.0;
        }
        let mut current_bpm = self.initial_bpm;
        let mut accumulated_time = self.start_offset;
        let mut accumulated_beats = 0.0;
        let mut current_ramp: Option<SegmentRamp> = None;

        for change in &self.changes {
            let Some(change_time) = change.position.absolute_time() else {
                break;
            };
            if change_time > score_time {
                break;
            }
            let Some(change_beats) = self.compute_change_beats(change, change_time, ts_changes)
            else {
                break;
            };
            let advanced_secs = segment_secs_for_beats(
                current_ramp.as_ref(),
                current_bpm,
                change_beats - accumulated_beats,
            );
            accumulated_time += Duration::from_secs_f64(advanced_secs.max(0.0));
            accumulated_beats = change_beats;
            let bpm_at_change = match current_ramp.as_ref() {
                Some(ramp) => ramp.bpm_after_secs(advanced_secs),
                None => current_bpm,
            };
            current_ramp = current_ramp.and_then(|ramp| ramp.advanced_by(advanced_secs));
            if let Some(new_bpm) = change.bpm {
                current_ramp = self.ramp_started_by(
                    change,
                    new_bpm,
                    bpm_at_change,
                    change_time,
                    Duration::ZERO,
                );
                current_bpm = new_bpm;
            }
        }

        let secs = score_time.saturating_sub(accumulated_time).as_secs_f64();
        accumulated_beats
            + match current_ramp.as_ref() {
                Some(ramp) => ramp.beats_in_secs(secs, current_bpm),
                None => secs * current_bpm / 60.0,
            }
    }

    /// The score time of a quarter-note beat position (0 at bar 1 beat 1).
    fn score_time_at_beats(
        &self,
        beats: f64,
        ts_changes: &[(u32, f64, TimeSignature)],
    ) -> Duration {
        if beats < 0.0 {
            return self
                .start_offset
                .saturating_sub(Duration::from_secs_f64(-beats * 60.0 / self.initial_bpm));
        }
        self.integrate_through_segments(beats, ts_changes, Duration::ZERO)
            .unwrap_or(self.start_offset)
    }

    /// Measures elapsed (0 at bar 1 beat 1, fractional within a bar) at a
    /// quarter-note beat position, through the time signature changes.
    fn measures_at_beats(&self, beats: f64, ts_changes: &[(u32, f64, TimeSignature)]) -> f64 {
        if beats < 0.0 {
            return beats / self.initial_time_signature.beats_per_measure();
        }
        let mut remaining = beats;
        let mut measure: u32 = 1;
        loop {
            let per_measure =
                Self::time_signature_at_measure(self.initial_time_signature, ts_changes, measure)
                    .beats_per_measure();
            if per_measure <= 0.0 {
                return (measure - 1) as f64;
            }
            if remaining < per_measure {
                return (measure - 1) as f64 + remaining / per_measure;
            }
            remaining -= per_measure;
            measure += 1;
        }
    }

    /// The quarter-note beat position at a measure position (the inverse of
    /// [`TempoMap::measures_at_beats`]).
    fn beats_at_measures(&self, measures: f64, ts_changes: &[(u32, f64, TimeSignature)]) -> f64 {
        if measures < 0.0 {
            return measures * self.initial_time_signature.beats_per_measure();
        }
        let whole = measures.floor();
        let bar = whole as u32 + 1;
        let per_measure =
            Self::time_signature_at_measure(self.initial_time_signature, ts_changes, bar)
                .beats_per_measure();
        let beat_in_bar = 1.0 + (measures - whole) * per_measure;
        Self::compute_target_beats(self.initial_time_signature, ts_changes, bar, beat_in_bar)
    }

    /// Get BPM at a given time (accounting for tempo changes)
    /// If offset_secs is provided, it's added to tempo change times to account for timeline shifts
    pub fn bpm_at_time(&self, time: Duration, offset_secs: f64) -> f64 {
        let offset_duration = Duration::from_secs_f64(offset_secs);
        let mut bpm = self.initial_bpm;

        for change in &self.changes {
            let change_time =
                change.position.absolute_time().unwrap_or(Duration::ZERO) + offset_duration;
            if change_time <= time {
                match change.transition {
                    TempoTransition::Snap => {
                        if let Some(new_bpm) = change.bpm {
                            bpm = new_bpm;
                        }
                    }
                    TempoTransition::Beats(_, curve) | TempoTransition::Measures(_, curve) => {
                        // For gradual transitions, calculate current BPM
                        if let Some(new_bpm) = change.bpm {
                            // Get BPM before this change
                            let old_bpm = if change_time > self.start_offset + offset_duration {
                                self.bpm_at_time(change_time - Duration::from_nanos(1), offset_secs)
                            } else {
                                self.initial_bpm
                            };

                            // Calculate transition duration
                            let transition_duration = match change.transition {
                                TempoTransition::Beats(beats, _) => {
                                    Duration::from_secs_f64(beats * 60.0 / old_bpm)
                                }
                                TempoTransition::Measures(measures, _) => {
                                    let current_ts =
                                        self.time_signature_at_time(change_time, offset_secs);
                                    let beats = measures * current_ts.beats_per_measure();
                                    Duration::from_secs_f64(beats * 60.0 / old_bpm)
                                }
                                TempoTransition::Snap => Duration::ZERO, // Shouldn't happen here
                            };

                            if time < change_time + transition_duration {
                                // During transition - use curve interpolation
                                let elapsed = (time - change_time).as_secs_f64();
                                let total = transition_duration.as_secs_f64();
                                let t = (elapsed / total).clamp(0.0, 1.0);
                                bpm = curve.bpm_at(t, old_bpm, new_bpm);
                            } else {
                                bpm = new_bpm;
                            }
                        }
                    }
                }
            }
        }

        bpm
    }

    /// Get time signature at a given time
    /// If offset_secs is provided, it's added to tempo change times to account for timeline shifts
    pub fn time_signature_at_time(&self, time: Duration, offset_secs: f64) -> TimeSignature {
        let offset_duration = Duration::from_secs_f64(offset_secs);
        let mut ts = self.initial_time_signature;

        for change in &self.changes {
            let change_time =
                change.position.absolute_time().unwrap_or(Duration::ZERO) + offset_duration;
            if change_time <= time {
                if let Some(new_ts) = change.time_signature {
                    // Time signature changes are always instant (snap)
                    ts = new_ts;
                }
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
        let ts_changes = self.build_time_signature_changes();
        let score_at = at_time.saturating_sub(Duration::from_secs_f64(offset_secs.max(0.0)));
        let start_beats = snap_beats(self.score_beats_at(score_at, &ts_changes));
        let end_beats = snap_beats(start_beats + beats);
        self.score_time_at_beats(end_beats, &ts_changes)
            .saturating_sub(score_at)
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
        let ts_changes = self.build_time_signature_changes();
        let score_at = at_time.saturating_sub(Duration::from_secs_f64(offset_secs.max(0.0)));
        let start_beats = snap_beats(self.score_beats_at(score_at, &ts_changes));
        let end_measures = self.measures_at_beats(start_beats, &ts_changes) + measures;
        let end_beats = snap_beats(self.beats_at_measures(end_measures, &ts_changes));
        self.score_time_at_beats(end_beats, &ts_changes)
            .saturating_sub(score_at)
    }

    /// How long the `beats` before `end` last: the same position walk as
    /// [`Self::beats_to_duration`], run backwards from `end`.
    pub fn beats_before(&self, beats: f64, end: Duration) -> Duration {
        let ts_changes = self.build_time_signature_changes();
        let end_beats = snap_beats(self.score_beats_at(end, &ts_changes));
        let start_beats = snap_beats(end_beats - beats);
        end.saturating_sub(self.score_time_at_beats(start_beats, &ts_changes))
    }

    /// How long the `measures` before `end` last: the same position walk as
    /// [`Self::measures_to_duration`], run backwards from `end`.
    pub fn measures_before(&self, measures: f64, end: Duration) -> Duration {
        let ts_changes = self.build_time_signature_changes();
        let end_beats = snap_beats(self.score_beats_at(end, &ts_changes));
        let start_measures = self.measures_at_beats(end_beats, &ts_changes) - measures;
        let start_beats = snap_beats(self.beats_at_measures(start_measures, &ts_changes));
        end.saturating_sub(self.score_time_at_beats(start_beats, &ts_changes))
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
        let ts_changes = self.build_time_signature_changes();
        let Some(states) = self.segment_states(&ts_changes) else {
            return ticks;
        };

        let interval = 60.0 / self.initial_bpm / PULSES_PER_QUARTER;
        let start = self.start_offset.as_secs_f64();
        let lead = (start / interval).floor() as usize;
        for k in (1..=lead.min(MAX_PULSES)).rev() {
            let at = start - k as f64 * interval;
            if at >= 0.0 && at < until.as_secs_f64() {
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

        /// A count-in before bar 1 pulses at the starting tempo, in phase:
        /// bar 1 falls exactly on a pulse.
        #[test]
        fn time_before_bar_one_pulses_in_phase_with_it() {
            let ticks = secs(
                &map(Duration::from_millis(1_010), 120.0, vec![])
                    .beat_clock_ticks(Duration::from_secs(2)),
            );
            let bar_one = ticks
                .iter()
                .position(|at| (at - 1.010).abs() < 1e-6)
                .expect("a pulse on bar 1");
            // 1.010 s holds 48 whole pulses and a part one: the first pulse
            // is that part in, not at zero.
            assert_eq!(bar_one, 48);
            assert!((ticks[0] - 0.010).abs() < 1e-6, "{}", ticks[0]);
            for pair in ticks.windows(2) {
                assert!((pair[1] - pair[0] - PULSE_120).abs() < 1e-6, "{pair:?}");
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
            let ts_changes = tempo.build_time_signature_changes();
            let lead = ticks
                .iter()
                .take_while(|at| **at < Duration::from_secs(2))
                .count();
            assert!(ticks.len() > 20_000, "{}", ticks.len());
            for (k, at) in ticks[lead..].iter().enumerate() {
                let want = tempo
                    .integrate_through_segments(k as f64 / 24.0, &ts_changes, Duration::ZERO)
                    .unwrap();
                assert_eq!(*at, want, "pulse {k}");
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

    // ── TransitionCurve::beats_in_duration ──────────────────────────

    #[test]
    fn beats_in_duration_constant_bpm() {
        // When old_bpm == new_bpm, this reduces to bpm * dt / 60
        let beats = TransitionCurve::Linear.beats_in_duration(120.0, 120.0, 4.0, 2.0);
        // 120 bpm for 2 seconds = 4 beats
        assert!((beats - 4.0).abs() < 1e-9);
    }

    #[test]
    fn beats_in_duration_full_transition() {
        // Full transition from 60 to 120 bpm over 4 seconds
        // integral of (60 + 60*t/4)/60 from 0 to 4 = integral of (1 + t/4) from 0 to 4
        // = [t + t^2/8] from 0 to 4 = 4 + 16/8 = 4 + 2 = 6
        let beats = TransitionCurve::Linear.beats_in_duration(60.0, 120.0, 4.0, 4.0);
        assert!((beats - 6.0).abs() < 1e-9);
    }

    #[test]
    fn beats_in_duration_half_transition() {
        // First 2 seconds of a 4-second transition from 60 to 120 bpm
        // integral of (60 + 60*t/4)/60 from 0 to 2 = integral of (1 + t/4) from 0 to 2
        // = [t + t^2/8] from 0 to 2 = 2 + 4/8 = 2 + 0.5 = 2.5
        let beats = TransitionCurve::Linear.beats_in_duration(60.0, 120.0, 4.0, 2.0);
        assert!((beats - 2.5).abs() < 1e-9);
    }

    // ── TransitionCurve::beats_in_remaining_transition ──────────────

    #[test]
    fn beats_in_remaining_from_start() {
        // From start to end should equal beats_in_duration for full duration
        let full = TransitionCurve::Linear.beats_in_duration(60.0, 120.0, 4.0, 4.0);
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
        // beats_in_duration -> solve_duration_for_beats should round-trip
        let old_bpm = 80.0;
        let new_bpm = 160.0;
        let total = 8.0;
        let dt_input = 3.0;
        let beats = TransitionCurve::Linear.beats_in_duration(old_bpm, new_bpm, total, dt_input);
        let dt_output = TransitionCurve::Linear
            .solve_duration_for_beats(old_bpm, new_bpm, total, 0.0, beats)
            .unwrap();
        assert!((dt_input - dt_output).abs() < 1e-6);
    }

    #[test]
    fn solve_duration_zero_bpm_returns_none() {
        // Both BPMs zero should return None
        let result = TransitionCurve::Linear.solve_duration_for_beats(0.0, 0.0, 4.0, 0.0, 1.0);
        assert!(result.is_none());
    }

    #[test]
    fn solve_duration_from_start_convenience() {
        let a = TransitionCurve::Linear
            .solve_duration_for_beats(120.0, 180.0, 4.0, 0.0, 2.0)
            .unwrap();
        let b = TransitionCurve::Linear
            .solve_duration_for_beats_from_start(120.0, 180.0, 4.0, 2.0)
            .unwrap();
        assert!((a - b).abs() < 1e-12);
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
}
