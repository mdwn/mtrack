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

        for change in sorted_changes {
            let absolute_time = match &change.position {
                TempoChangePosition::Time(t) => *t,
                TempoChangePosition::MeasureBeat(m, b) => {
                    // Convert measure/beat to time using current tempo state
                    let total_beats =
                        (*m - 1) as f64 * current_time_sig.beats_per_measure() + (*b - 1.0);
                    let beats_from_last_change = total_beats - accumulated_beats;
                    let time_from_beats =
                        Duration::from_secs_f64(beats_from_last_change * 60.0 / current_bpm);
                    accumulated_time + time_from_beats
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

            // Update tempo state for next iteration
            if let Some(new_bpm) = change.bpm {
                current_bpm = new_bpm;
            }
            if let Some(new_ts) = change.time_signature {
                current_time_sig = new_ts;
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
        self.integrate_through_segments(
            target_beats,
            &ts_changes,
            offset_duration,
            measure,
            beat,
            measure_offset,
        )
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
    fn time_to_measure_beat(&self, target_time: Duration) -> (u32, f64) {
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
        #[allow(unused_variables)] measure: u32,
        #[allow(unused_variables)] beat: f64,
        #[allow(unused_variables)] measure_offset: u32,
    ) -> Option<Duration> {
        let mut current_bpm = self.initial_bpm;
        let mut accumulated_time = self.start_offset;
        let mut accumulated_beats = 0.0;

        for change in &self.changes {
            let change_time = change.position.absolute_time()? + offset_duration;
            let change_beats = self.compute_change_beats(change, change_time, ts_changes)?;

            if change_beats > target_beats {
                // Target is before this change
                let remaining_beats = target_beats - accumulated_beats;
                let result_time = accumulated_time
                    + Duration::from_secs_f64(remaining_beats * 60.0 / current_bpm);
                #[cfg(test)]
                eprintln!(
                    "[tempo-debug] early-return target-before-change measure={} beat={} offset={} target_beats={} change_beats={} accumulated_beats={} remaining_beats={} bpm={:.6} start_offset_secs={:.6} accumulated_time_secs={:.6} result_time_secs={:.6}",
                    measure, beat, measure_offset, target_beats, change_beats, accumulated_beats,
                    remaining_beats, current_bpm, self.start_offset.as_secs_f64(),
                    accumulated_time.as_secs_f64(), result_time.as_secs_f64()
                );
                return Some(result_time);
            }

            // Process up to this change
            let beats_to_change = change_beats - accumulated_beats;
            accumulated_time += Duration::from_secs_f64(beats_to_change * 60.0 / current_bpm);
            accumulated_beats = change_beats;

            if let Some(new_bpm) = change.bpm {
                current_bpm = new_bpm;
            }
        }

        // Target is beyond all changes - use final tempo
        let remaining_beats = target_beats - accumulated_beats;
        let result_time =
            accumulated_time + Duration::from_secs_f64(remaining_beats * 60.0 / current_bpm);

        #[cfg(test)]
        eprintln!(
            "[tempo-debug] measure_to_time_with_offset measure={} beat={} offset={} \
                 target_beats={} change_beats={} remaining_beats={} start_offset_secs={:.6} \
                 accumulated_time_secs={:.6} current_bpm={:.6} result_time_secs={:.6}",
            measure,
            beat,
            measure_offset,
            target_beats,
            accumulated_beats,
            remaining_beats,
            self.start_offset.as_secs_f64(),
            accumulated_time.as_secs_f64(),
            current_bpm,
            result_time.as_secs_f64()
        );

        Some(result_time)
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

    /// Convert a duration in beats to absolute Duration at a given time
    /// This integrates through tempo changes during the duration
    /// If offset_secs is provided, it's used to adjust tempo change lookups
    pub fn beats_to_duration(&self, beats: f64, at_time: Duration, offset_secs: f64) -> Duration {
        let mut remaining_beats = beats;
        let mut current_time = at_time;
        let mut current_bpm = self.bpm_at_time(at_time, offset_secs);

        // Find all tempo changes that occur during the duration
        // We'll process them in order, integrating through each segment
        let offset_duration = Duration::from_secs_f64(offset_secs);
        let mut relevant_changes: Vec<&TempoChange> = self
            .changes
            .iter()
            .filter(|change| {
                if let Some(change_time) = change.position.absolute_time() {
                    (change_time + offset_duration) >= at_time
                } else {
                    false
                }
            })
            .collect();

        // Sort by time (with offset applied)
        relevant_changes.sort_by(|a, b| {
            let time_a = a.position.absolute_time().unwrap_or(Duration::ZERO) + offset_duration;
            let time_b = b.position.absolute_time().unwrap_or(Duration::ZERO) + offset_duration;
            time_a.cmp(&time_b)
        });

        // Process each tempo change that occurs during the duration
        for change in relevant_changes {
            if remaining_beats <= 0.0 {
                break;
            }

            let change_time = change.position.absolute_time().unwrap() + offset_duration;
            if change_time <= current_time {
                // This change already happened, update current BPM
                match change.transition {
                    TempoTransition::Snap => {
                        if let Some(new_bpm) = change.bpm {
                            current_bpm = new_bpm;
                        }
                    }
                    TempoTransition::Beats(_, curve) | TempoTransition::Measures(_, curve) => {
                        // Check if we're still in the transition
                        let old_bpm = if change_time > self.start_offset + offset_duration {
                            self.bpm_at_time(change_time - Duration::from_nanos(1), offset_secs)
                        } else {
                            self.initial_bpm
                        };

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
                            TempoTransition::Snap => Duration::ZERO,
                        };

                        if current_time < change_time + transition_duration {
                            // Still in transition - need to integrate through the curve
                            let elapsed = (current_time - change_time).as_secs_f64();
                            let total = transition_duration.as_secs_f64();
                            let new_bpm = change.bpm.unwrap_or(old_bpm);

                            // Calculate how many beats remain in the transition using curve
                            let beats_in_remaining_transition = curve
                                .beats_in_remaining_transition(old_bpm, new_bpm, total, elapsed);

                            if remaining_beats <= beats_in_remaining_transition {
                                // Remaining beats fit within the remaining transition
                                // Solve for duration using curve
                                if let Some(dt) = curve.solve_duration_for_beats(
                                    old_bpm,
                                    new_bpm,
                                    total,
                                    elapsed,
                                    remaining_beats,
                                ) {
                                    let duration_for_remaining = Duration::from_secs_f64(dt);
                                    return current_time + duration_for_remaining - at_time;
                                } else {
                                    // Fallback to average BPM if calculation fails
                                    let t = (elapsed / total).clamp(0.0, 1.0);
                                    let current_bpm_at_start = curve.bpm_at(t, old_bpm, new_bpm);
                                    let dt = remaining_beats * 60.0
                                        / ((current_bpm_at_start + new_bpm) / 2.0);
                                    let duration_for_remaining = Duration::from_secs_f64(dt);
                                    return current_time + duration_for_remaining - at_time;
                                }
                            }

                            // Consume all beats in the remaining transition
                            remaining_beats -= beats_in_remaining_transition;
                            current_time = change_time + transition_duration;
                            current_bpm = new_bpm;
                        } else {
                            // Transition complete
                            if let Some(new_bpm) = change.bpm {
                                current_bpm = new_bpm;
                            }
                        }
                    }
                }
                continue;
            }

            // Calculate how many beats occur before this change
            let change_time_playback = change_time; // change_time already has offset applied
            let time_to_change = change_time_playback - current_time;
            let beats_in_segment = time_to_change.as_secs_f64() * current_bpm / 60.0;

            // Use a small epsilon for floating point comparison to handle precision issues
            // This ensures that when remaining_beats is very close to beats_in_segment,
            // we treat it as equal and end the effect exactly at the tempo change
            const EPSILON: f64 = 1e-6;
            let beats_diff = (remaining_beats - beats_in_segment).abs();

            if remaining_beats < beats_in_segment {
                // All remaining beats fit in this segment (constant BPM)
                let duration_for_remaining =
                    Duration::from_secs_f64(remaining_beats * 60.0 / current_bpm);
                return current_time + duration_for_remaining - at_time;
            } else if beats_diff < EPSILON {
                // remaining_beats is essentially equal to beats_in_segment (within epsilon)
                // Effect ends exactly at the tempo change
                return change_time - at_time;
            }

            // Consume all beats in this segment
            remaining_beats -= beats_in_segment;
            current_time = change_time;

            // Handle the transition at this change
            match change.transition {
                TempoTransition::Snap => {
                    if let Some(new_bpm) = change.bpm {
                        current_bpm = new_bpm;
                    }
                }
                TempoTransition::Beats(_, curve) | TempoTransition::Measures(_, curve) => {
                    let old_bpm = current_bpm;
                    let new_bpm = change.bpm.unwrap_or(old_bpm);

                    // Calculate transition duration in time
                    let transition_duration = match change.transition {
                        TempoTransition::Beats(beats, _) => {
                            Duration::from_secs_f64(beats * 60.0 / old_bpm)
                        }
                        TempoTransition::Measures(measures, _) => {
                            let current_ts = self.time_signature_at_time(change_time, offset_secs);
                            let beats = measures * current_ts.beats_per_measure();
                            Duration::from_secs_f64(beats * 60.0 / old_bpm)
                        }
                        TempoTransition::Snap => Duration::ZERO,
                    };

                    // Calculate how many beats occur during the transition using curve
                    let total_duration = transition_duration.as_secs_f64();
                    let beats_in_transition =
                        curve.beats_in_duration(old_bpm, new_bpm, total_duration, total_duration);

                    if remaining_beats <= beats_in_transition {
                        // Remaining beats fit within the transition
                        // Solve for duration using curve
                        if let Some(dt) = curve.solve_duration_for_beats_from_start(
                            old_bpm,
                            new_bpm,
                            total_duration,
                            remaining_beats,
                        ) {
                            let duration_for_remaining = Duration::from_secs_f64(dt);
                            return current_time + duration_for_remaining - at_time;
                        } else {
                            // Fallback to average BPM if calculation fails
                            let dt = remaining_beats * 60.0 / ((old_bpm + new_bpm) / 2.0);
                            let duration_for_remaining = Duration::from_secs_f64(dt);
                            return current_time + duration_for_remaining - at_time;
                        }
                    }

                    // Consume all beats in the transition
                    remaining_beats -= beats_in_transition;
                    current_time += transition_duration;
                    current_bpm = new_bpm;
                }
            }
        }

        // Remaining beats after all changes - use final BPM
        let duration_for_remaining = Duration::from_secs_f64(remaining_beats * 60.0 / current_bpm);
        current_time + duration_for_remaining - at_time
    }

    /// Convert a duration in measures to absolute Duration at a given time
    /// This integrates through tempo and time signature changes during the duration
    /// If offset_secs is provided, it's used to adjust tempo change lookups
    pub fn measures_to_duration(
        &self,
        measures: f64,
        at_time: Duration,
        offset_secs: f64,
    ) -> Duration {
        // Integrate through time signature changes to compute total beats
        let offset_duration = Duration::from_secs_f64(offset_secs);
        let mut remaining_measures = measures;
        let mut current_time = at_time;
        let mut current_bpm = self.bpm_at_time(at_time, offset_secs);
        let mut current_ts = self.time_signature_at_time(at_time, offset_secs);
        let mut total_duration = Duration::ZERO;

        // Collect time-signature changes that occur after at_time
        let mut ts_changes: Vec<(Duration, TimeSignature, f64)> = Vec::new();
        for change in &self.changes {
            if let Some(change_time) = change.position.absolute_time() {
                let shifted = change_time + offset_duration;
                if shifted > at_time {
                    if let Some(new_ts) = change.time_signature {
                        let new_bpm = change.bpm.unwrap_or(self.bpm_at_time(shifted, offset_secs));
                        ts_changes.push((shifted, new_ts, new_bpm));
                    } else if let Some(new_bpm) = change.bpm {
                        // Tempo-only change — still need to track for bpm updates
                        let ts_at = self.time_signature_at_time(shifted, offset_secs);
                        ts_changes.push((shifted, ts_at, new_bpm));
                    }
                }
            }
        }
        ts_changes.sort_by_key(|(t, _, _)| *t);

        for (change_time, new_ts, new_bpm) in &ts_changes {
            if remaining_measures <= 0.0 {
                break;
            }

            // How many measures fit between current_time and change_time at current tempo/ts?
            let beats_per_measure = current_ts.beats_per_measure();
            let segment_secs = change_time.saturating_sub(current_time).as_secs_f64();
            let segment_beats = segment_secs * current_bpm / 60.0;
            let segment_measures = if beats_per_measure > 0.0 {
                segment_beats / beats_per_measure
            } else {
                0.0
            };

            if remaining_measures <= segment_measures {
                // All remaining measures fit in this segment
                let beats_needed = remaining_measures * beats_per_measure;
                let time_needed = Duration::from_secs_f64(beats_needed * 60.0 / current_bpm);
                total_duration += time_needed;
                remaining_measures = 0.0;
                break;
            }

            // Consume this segment
            remaining_measures -= segment_measures;
            total_duration += change_time.saturating_sub(current_time);
            current_time = *change_time;
            current_ts = *new_ts;
            current_bpm = *new_bpm;
        }

        // Remaining measures after all changes
        if remaining_measures > 0.0 {
            let beats_per_measure = current_ts.beats_per_measure();
            let beats_needed = remaining_measures * beats_per_measure;
            let time_needed = Duration::from_secs_f64(beats_needed * 60.0 / current_bpm);
            total_duration += time_needed;
        }

        total_duration
    }

    /// Calculate duration for N playback measures
    /// score_start_measure: The score measure where the effect starts (e.g., 88)
    /// playback_measures: Number of playback measures (e.g., 30)
    /// measure_offset: The offset in measures (playback_measure = score_measure + measure_offset)
    ///
    /// This calculates duration by iterating through playback measures and finding tempo changes
    /// at their playback measure positions (which are the same as score measure positions for tempo changes)
    pub fn playback_measures_to_duration(
        &self,
        score_start_measure: u32,
        playback_measures: f64,
        measure_offset: u32,
    ) -> Duration {
        let playback_start_measure = score_start_measure as f64 + measure_offset as f64;
        let playback_end_measure = playback_start_measure + playback_measures;

        // Calculate duration by integrating through playback measures
        // Tempo changes are at fixed score measures, which correspond to the same playback measures
        let mut duration = Duration::ZERO;
        let mut current_playback_measure = playback_start_measure;
        let mut current_bpm = self.bpm_at_time(
            self.measure_to_time_with_offset(score_start_measure, 1.0, 0, 0.0)
                .unwrap_or(self.start_offset),
            0.0,
        );
        let mut current_ts = self.time_signature_at_time(
            self.measure_to_time_with_offset(score_start_measure, 1.0, 0, 0.0)
                .unwrap_or(self.start_offset),
            0.0,
        );

        while current_playback_measure < playback_end_measure {
            let playback_measure_int = current_playback_measure as u32;

            // Check if there's a tempo change at this playback measure
            // Tempo changes are at score measures, which are the same as playback measures
            // (offsets don't affect tempo change positions)
            let mut measure_bpm = current_bpm;
            let mut measure_ts = current_ts;

            for change in &self.changes {
                if let Some((score_measure, beat)) = change.original_measure_beat {
                    // Tempo changes are at score measures, which equal playback measures
                    if score_measure == playback_measure_int && (beat - 1.0).abs() < 0.001 {
                        if let Some(new_bpm) = change.bpm {
                            measure_bpm = new_bpm;
                            current_bpm = new_bpm;
                        }
                        if let Some(new_ts) = change.time_signature {
                            measure_ts = new_ts;
                            current_ts = new_ts;
                        }
                    }
                }
            }

            // Calculate how much of this measure we need
            let measures_remaining = playback_end_measure - current_playback_measure;
            let measures_in_this_iteration = measures_remaining.min(1.0);

            // Calculate duration for this measure (full or partial)
            let beats_per_measure = measure_ts.beats_per_measure();
            let beats_in_this_iteration = beats_per_measure * measures_in_this_iteration;
            let measure_duration =
                Duration::from_secs_f64(beats_in_this_iteration * 60.0 / measure_bpm);
            duration += measure_duration;

            current_playback_measure += 1.0;
        }

        duration
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    // ── TempoMap::measures_to_duration ────────────────────────────────

    #[test]
    fn measures_to_duration_constant_tempo_4_4() {
        let map = simple_tempo_map(120.0);
        // 2 measures of 4/4 at 120 bpm = 8 beats = 4 seconds
        let dur = map.measures_to_duration(2.0, Duration::ZERO, 0.0);
        assert!((dur.as_secs_f64() - 4.0).abs() < 1e-9);
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

    // ── TempoMap::playback_measures_to_duration ──────────────────────

    #[test]
    fn playback_measures_to_duration_constant() {
        let map = simple_tempo_map(120.0);
        // 4 playback measures of 4/4 at 120 bpm from score measure 1
        // = 16 beats = 8 seconds
        let dur = map.playback_measures_to_duration(1, 4.0, 0);
        assert!((dur.as_secs_f64() - 8.0).abs() < 1e-6);
    }

    #[test]
    fn playback_measures_to_duration_with_tempo_change() {
        let map = TempoMap::new(
            Duration::ZERO,
            120.0,
            TimeSignature::new(4, 4),
            vec![TempoChange {
                position: TempoChangePosition::MeasureBeat(3, 1.0),
                original_measure_beat: Some((3, 1.0)),
                bpm: Some(60.0),
                time_signature: None,
                transition: TempoTransition::Snap,
            }],
        );
        // 4 measures from score measure 1:
        // Measures 1-2 at 120 bpm: 8 beats = 4s
        // Measures 3-4 at 60 bpm: 8 beats = 8s
        // Total = 12s
        let dur = map.playback_measures_to_duration(1, 4.0, 0);
        assert!((dur.as_secs_f64() - 12.0).abs() < 1e-6);
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
