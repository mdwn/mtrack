// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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

    /// Get beats per measure
    pub fn beats_per_measure(&self) -> f64 {
        self.numerator as f64
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

    /// Convert a measure/beat position to absolute time with an offset
    /// The offset is applied to both the target position and tempo change positions
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

        // Integrate through tempo segments to reach target position
        // We need to account for time signature changes that affect beats per measure
        // Note: offset_secs is used to shift tempo change times, but NOT added to the result
        // The result is in "score space" where tempo changes are shifted but the offset isn't added
        // The parser will add applied_offset_secs separately to get absolute time
        let mut current_bpm = self.initial_bpm;
        let mut accumulated_time = self.start_offset;
        let mut accumulated_beats = 0.0;

        // Calculate target beats by integrating through measures beat-by-beat
        // This accounts for time signature changes properly
        let mut target_beats = 0.0;
        let mut current_measure = 1;
        let mut current_beat_in_measure = 1.0;

        // Process all tempo changes to build a map of when time signatures change
        // Use the original_measure_beat if available, otherwise convert from time
        let mut ts_changes: Vec<(u32, f64, TimeSignature)> = Vec::new();

        for change in &self.changes {
            if let Some(new_ts) = change.time_signature {
                // Use original measure/beat if available, otherwise convert from time
                if let Some((m, b)) = change.original_measure_beat {
                    ts_changes.push((m, b, new_ts));
                } else if let Some(change_time) = change.position.absolute_time() {
                    // Convert time back to measure/beat by integrating
                    let mut m = 1;
                    let mut b = 1.0;
                    let mut temp_time = self.start_offset;
                    let mut temp_bpm = self.initial_bpm;
                    let mut temp_ts = self.initial_time_signature;

                    // Integrate through all changes before this one
                    for prev_change in &self.changes {
                        if let Some(prev_time) = prev_change.position.absolute_time() {
                            if prev_time >= change_time {
                                break;
                            }
                            // Integrate from temp_time to prev_time
                            while temp_time < prev_time {
                                let beats_per_measure = temp_ts.beats_per_measure();
                                let time_per_measure =
                                    Duration::from_secs_f64(beats_per_measure * 60.0 / temp_bpm);
                                if temp_time + time_per_measure <= prev_time {
                                    temp_time += time_per_measure;
                                    m += 1;
                                    b = 1.0;
                                } else {
                                    let remaining = prev_time - temp_time;
                                    let remaining_beats = remaining.as_secs_f64() * temp_bpm / 60.0;
                                    b += remaining_beats;
                                    temp_time = prev_time;
                                    break;
                                }
                            }
                            if let Some(new_bpm) = prev_change.bpm {
                                temp_bpm = new_bpm;
                            }
                            if let Some(new_ts) = prev_change.time_signature {
                                temp_ts = new_ts;
                            }
                        }
                    }

                    // Integrate from temp_time to change_time
                    while temp_time < change_time {
                        let beats_per_measure = temp_ts.beats_per_measure();
                        let time_per_measure =
                            Duration::from_secs_f64(beats_per_measure * 60.0 / temp_bpm);
                        if temp_time + time_per_measure <= change_time {
                            temp_time += time_per_measure;
                            m += 1;
                            b = 1.0;
                        } else {
                            let remaining = change_time - temp_time;
                            let remaining_beats = remaining.as_secs_f64() * temp_bpm / 60.0;
                            b += remaining_beats;
                            break;
                        }
                    }
                    ts_changes.push((m, b, new_ts));
                }
            }
        }
        // Sort by measure then beat (ascending order)
        ts_changes.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        });

        // Apply offset to target measure (score measure -> playback measure)
        let playback_measure = measure + measure_offset;

        // Integrate through measures to calculate total beats
        // We need to account for fractional beats and time signature changes
        // Start from measure 1, beat 1 (which is 0 beats)
        while current_measure < playback_measure
            || (current_measure == playback_measure && current_beat_in_measure < beat)
        {
            // Determine the time signature for the CURRENT measure
            // Time signature changes apply at the START of the specified measure/beat
            // So if a change is at measure 4/1, measure 4 uses the NEW time signature
            let ts_for_this_measure = {
                // Find the most recent time signature change that applies at or before the start of this measure
                // Time signature changes apply at the START of the specified measure/beat
                // So if a change is at measure 4/1, measure 4 uses the NEW time signature
                // NOTE: Tempo/time-signature change positions are in score measures and should NOT be offset.
                let mut ts = self.initial_time_signature;
                // Iterate through sorted changes (ascending order) to find the most recent one that applies
                for (change_m, change_b, new_ts) in &ts_changes {
                    let change_playback_measure = *change_m; // do not apply measure_offset to time signatures
                                                             // If change is exactly at the start of current measure (beat 1), the measure uses the NEW time sig
                    if change_playback_measure == current_measure && (*change_b - 1.0).abs() < 0.001
                    {
                        ts = *new_ts;
                    } else if change_playback_measure < current_measure {
                        // Change was in a previous measure, so it applies to this and all subsequent measures
                        ts = *new_ts;
                    }
                    // If change_playback_measure > current_measure, it's in the future, so we keep the current ts
                }
                ts
            };

            // If we're at the target measure, calculate partial beats
            if current_measure == playback_measure {
                let beats_to_add = beat - current_beat_in_measure;
                target_beats += beats_to_add;
                break;
            }

            // We're before the target measure - add remaining beats in current measure
            // Use the time signature that applies to this measure
            let beats_per_current_measure = ts_for_this_measure.beats_per_measure();
            let beats_already_counted = current_beat_in_measure - 1.0; // e.g., beat 1 = 0 beats counted
            let beats_remaining_in_measure = beats_per_current_measure - beats_already_counted;
            target_beats += beats_remaining_in_measure;

            current_measure += 1;
            current_beat_in_measure = 1.0;
        }

        // Process tempo changes in order, building up time
        // Note: Tempo changes are specified in score measures (from the tempo section).
        // We apply the measure_offset to tempo changes so they respect the offset timeline,
        // ensuring consistent measure numbering with the target measure.
        for change in &self.changes {
            // Tempo changes are resolved to absolute time; apply offset to slide them
            let change_time = change.position.absolute_time()? + offset_duration;

            // Calculate beats to this tempo change
            // If we have original_measure_beat, use it to calculate beats directly (same way as target_beats)
            // Otherwise, convert time to beats by integrating through tempo changes
            let change_beats = if let Some((change_m, change_b)) = change.original_measure_beat {
                // Calculate beats by integrating through measures (same logic as target_beats)
                // Tempo changes are specified in score measures and should NOT be offset.
                let change_playback_measure = change_m;

                let mut change_target_beats = 0.0;
                let mut change_current_measure = 1;
                let mut change_current_beat = 1.0;

                while change_current_measure < change_playback_measure
                    || (change_current_measure == change_playback_measure
                        && change_current_beat < change_b)
                {
                    // Determine time signature for current measure
                    // Note: time signature changes are in score measures and should NOT be offset.
                    let ts_for_measure = {
                        let mut ts = self.initial_time_signature;
                        for (ts_m, ts_b, new_ts) in &ts_changes {
                            let ts_playback_measure = *ts_m; // no offset applied
                                                             // Time signature changes apply at or before the current measure.
                                                             // If the change is exactly at the start of the measure (beat 1),
                                                             // or in any previous measure, it governs this measure.
                            if ts_playback_measure < change_current_measure
                                || (ts_playback_measure == change_current_measure
                                    && (*ts_b - 1.0).abs() < 0.001)
                            {
                                ts = *new_ts;
                            }
                        }
                        ts
                    };

                    if change_current_measure == change_playback_measure {
                        let beats_to_add = change_b - change_current_beat;
                        change_target_beats += beats_to_add;
                        break;
                    }

                    let beats_per_measure = ts_for_measure.beats_per_measure();
                    let beats_remaining = beats_per_measure - (change_current_beat - 1.0);
                    change_target_beats += beats_remaining;
                    change_current_measure += 1;
                    change_current_beat = 1.0;
                }

                change_target_beats
            } else {
                // Time-based change - convert time to beats by integrating through tempo changes
                let mut change_accumulated_time = self.start_offset;
                let mut change_accumulated_beats = 0.0;
                let mut change_accumulated_bpm = self.initial_bpm;

                for prev_change in &self.changes {
                    let prev_change_time = prev_change.position.absolute_time()?;
                    if prev_change_time >= change_time {
                        break;
                    }

                    let time_to_prev = prev_change_time - change_accumulated_time;
                    let beats_to_prev = time_to_prev.as_secs_f64() * change_accumulated_bpm / 60.0;
                    change_accumulated_beats += beats_to_prev;
                    change_accumulated_time = prev_change_time;

                    if let Some(new_bpm) = prev_change.bpm {
                        change_accumulated_bpm = new_bpm;
                    }
                }

                let time_to_this_change = change_time - change_accumulated_time;
                let beats_to_this_change =
                    time_to_this_change.as_secs_f64() * change_accumulated_bpm / 60.0;
                change_accumulated_beats + beats_to_this_change
            };

            if change_beats > target_beats {
                // Target is before this change - calculate remaining
                let remaining_beats = target_beats - accumulated_beats;
                let time_for_remaining =
                    Duration::from_secs_f64(remaining_beats * 60.0 / current_bpm);
                let result_time = accumulated_time + time_for_remaining;
                #[cfg(test)]
                eprintln!(
                    "[tempo-debug] early-return target-before-change measure={} beat={} offset={} target_beats={} change_beats={} accumulated_beats={} remaining_beats={} bpm={:.6} start_offset_secs={:.6} accumulated_time_secs={:.6} result_time_secs={:.6}",
                    measure,
                    beat,
                    measure_offset,
                    target_beats,
                    change_beats,
                    accumulated_beats,
                    remaining_beats,
                    current_bpm,
                    self.start_offset.as_secs_f64(),
                    accumulated_time.as_secs_f64(),
                    result_time.as_secs_f64()
                );
                return Some(result_time);
            }

            // Process up to this change
            let beats_to_change = change_beats - accumulated_beats;
            let time_to_change = Duration::from_secs_f64(beats_to_change * 60.0 / current_bpm);
            accumulated_time += time_to_change;
            accumulated_beats = change_beats;

            // Update tempo for next segment
            if let Some(new_bpm) = change.bpm {
                current_bpm = new_bpm;
            }

            // Update position (tracked via accumulated_beats)
        }

        // Target is beyond all changes - use final tempo
        // accumulated_time already includes start_offset (but NOT offset_duration), so we just need to add the remaining time
        let remaining_beats = target_beats - accumulated_beats;
        let time_for_remaining = Duration::from_secs_f64(remaining_beats * 60.0 / current_bpm);
        let result_time = accumulated_time + time_for_remaining;

        // Emit detailed debug info in tests to diagnose timing issues
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
        let initial_time_sig = self.time_signature_at_time(at_time, offset_secs);
        let initial_beats = measures * initial_time_sig.beats_per_measure();

        // Convert measures to beats, then use beats_to_duration
        // Note: This is approximate if time signature changes during the duration
        // A more accurate implementation would integrate through time signature changes
        // but for now, we use the initial time signature
        self.beats_to_duration(initial_beats, at_time, offset_secs)
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

            // Calculate duration for this measure
            let beats = measure_ts.beats_per_measure();
            let measure_duration = Duration::from_secs_f64(beats * 60.0 / measure_bpm);
            duration += measure_duration;

            current_playback_measure += 1.0;
        }

        duration
    }
}
