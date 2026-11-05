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
//! The beat grid: the absolute time of every beat of a song and which beats
//! open a measure. Built from a click track ([`super::click_analysis`]) or
//! from a `tempo:` block ([`BeatGrid::from_tempo_map`]), and the single
//! source of truth for where a measure/beat position lies in time.
//!
//! This module is pure (no audio, no I/O) so the web UI can compile it to
//! WebAssembly and place positions exactly where the player does.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A beat grid derived from click track analysis.
///
/// Contains the absolute time of every detected beat and identifies which
/// beats are measure boundaries (accented clicks). This is the ground truth
/// from the audio — no assumptions about note values or BPM.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(into_wasm_abi, from_wasm_abi)
)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BeatGrid {
    /// Absolute time in seconds of each detected beat.
    pub beats: Vec<f64>,
    /// Indices into `beats` that mark measure boundaries (accented beats).
    pub measure_starts: Vec<usize>,
}

impl BeatGrid {
    /// Returns the number of beats.
    pub fn beat_count(&self) -> usize {
        self.beats.len()
    }

    /// Returns the number of complete measures.
    pub fn measure_count(&self) -> usize {
        self.measure_starts.len().saturating_sub(1)
    }

    /// Returns the number of beats in a measure (0-indexed). The last
    /// measure runs to the end of the grid.
    pub fn beats_in_measure(&self, measure: usize) -> Option<usize> {
        let start = *self.measure_starts.get(measure)?;
        let end = self
            .measure_starts
            .get(measure + 1)
            .copied()
            .unwrap_or(self.beats.len());
        end.checked_sub(start)
    }

    /// Returns the absolute time of a (possibly fractional) beat within a
    /// measure: measure 0-indexed, beat 1-based (beat 1.0 is the measure
    /// boundary, 2.5 is halfway between the measure's second and third
    /// beats). Fractional positions interpolate linearly between grid
    /// beats. Returns `None` when the position falls outside the grid.
    ///
    /// A beat must also stay inside its own measure. `beats` is flat, so
    /// without that bound "beat 5" of a 4/4 measure would index the next
    /// measure's downbeat and quietly resolve to it — and since the config
    /// validator orders boundaries as `(measure, beat)` tuples without a
    /// grid to consult, a section written that way would pass validation
    /// and then resolve to a zero-length or inverted range. The last
    /// fractional beat may still interpolate towards the next downbeat:
    /// beat 4.5 of a 4/4 measure is inside the measure, beat 5.0 is not.
    pub fn beat_time(&self, measure: usize, beat: f64) -> Option<f64> {
        if !beat.is_finite() || beat < 1.0 {
            return None;
        }
        let offset = beat - 1.0;
        if offset.floor() as usize >= self.beats_in_measure(measure)? {
            return None;
        }
        let base = self.measure_starts.get(measure)? + offset.floor() as usize;
        let t0 = *self.beats.get(base)?;
        let frac = offset.fract();
        if frac == 0.0 {
            return Some(t0);
        }
        let t1 = *self.beats.get(base + 1)?;
        Some(t0 + (t1 - t0) * frac)
    }

    /// The position of a time in seconds, as (measure, beat): measure
    /// 0-indexed, beat 1-based and fractional, the way [`BeatGrid::beat_time`]
    /// takes them -- the beat at or before the time plus how far through it
    /// the time lies, so `beat_time` of the result is the time again. This
    /// is the one rule for reading a time off the grid; a time that has to
    /// land on a beat goes through [`BeatGrid::snapped_position_at`]. A time
    /// before the grid is its first beat and a time past it is its last.
    /// `None` for an empty grid, a time that is not a number, or a beat
    /// before the first downbeat (a pickup), which has no measure to be
    /// counted in.
    pub fn position_at(&self, secs: f64) -> Option<(usize, f64)> {
        if !secs.is_finite() {
            return None;
        }
        let index = self
            .beats
            .iter()
            .rposition(|beat| *beat <= secs)
            .unwrap_or(0);
        let t0 = *self.beats.get(index)?;
        let frac = match self.beats.get(index + 1) {
            Some(t1) if *t1 > t0 && secs > t0 => ((secs - t0) / (t1 - t0)).clamp(0.0, 1.0),
            _ => 0.0,
        };
        let measure = self
            .measure_starts
            .iter()
            .rposition(|&start| start <= index)?;
        Some((
            measure,
            (index - self.measure_starts[measure]) as f64 + 1.0 + frac,
        ))
    }

    /// [`BeatGrid::position_at`] rounded to the nearest `step` of a beat (1
    /// for whole beats, 0.5 for halves): the rule a time snaps to a beat by,
    /// so a tempo change or a pilot hint captured from the playhead lands on
    /// the nearest beat and a section boundary on the nearest half. The
    /// rounded position is asked of the grid again: one rounded past the end
    /// of its measure is the next measure's downbeat, not a beat the measure
    /// does not hold, and one rounded past the grid's end is the last beat
    /// it does.
    pub fn snapped_position_at(&self, secs: f64, step: f64) -> Option<(usize, f64)> {
        if !step.is_finite() || step <= 0.0 {
            return None;
        }
        let (measure, beat) = self.position_at(secs)?;
        let rounded = 1.0 + ((beat - 1.0) / step).round() * step;
        if self.beat_time(measure, rounded).is_some() {
            return Some((measure, rounded));
        }
        if self.beat_time(measure + 1, 1.0).is_some() {
            return Some((measure + 1, 1.0));
        }
        Some((measure, beat.floor()))
    }

    /// Returns the time range (start, end) of the given measure (0-indexed).
    /// Returns `None` if the measure index is out of range.
    pub fn measure_time_range(&self, measure: usize) -> Option<(f64, f64)> {
        let start_beat = *self.measure_starts.get(measure)?;
        let end_beat = self
            .measure_starts
            .get(measure + 1)
            .copied()
            .unwrap_or(self.beats.len().saturating_sub(1));
        let start = *self.beats.get(start_beat)?;
        let end = *self.beats.get(end_beat)?;
        Some((start, end))
    }

    /// Generates a beat grid from an explicit tempo map, emitting one beat per
    /// denominator note (7/8 yields seven beats per measure) up to `duration`.
    pub fn from_tempo_map(map: &crate::tempo::TempoMap, duration: Duration) -> BeatGrid {
        // Hard cap so a degenerate map (e.g. microscopic bpm rounding) can
        // never loop unbounded; real songs are a few hundred measures.
        const MAX_MEASURES: u32 = 100_000;

        let total_secs = duration.as_secs_f64();
        let mut beats = Vec::new();
        let mut measure_starts = Vec::new();

        'measures: for measure in 1..=MAX_MEASURES {
            let Some(start) = map.measure_to_time_with_offset(measure, 1.0, 0, 0.0) else {
                break;
            };
            if start.as_secs_f64() >= total_secs {
                break;
            }

            let time_signature = map.time_signature_at_time(start, 0.0);
            measure_starts.push(beats.len());

            // Beat positions within the measure are expressed in the tempo
            // map's quarter-note units; step one denominator note at a time.
            let step = 4.0 / time_signature.denominator as f64;
            for k in 0..time_signature.grid_beats_per_measure() {
                let beat_position = 1.0 + k as f64 * step;
                let Some(time) = map.measure_to_time_with_offset(measure, beat_position, 0, 0.0)
                else {
                    break 'measures;
                };
                let secs = time.as_secs_f64();
                if secs >= total_secs {
                    break 'measures;
                }
                beats.push(secs);
            }
        }

        BeatGrid {
            beats,
            measure_starts,
        }
    }

    /// The detected lead-in: how long before *bar 1 beat 1* the song starts.
    ///
    /// A song with silence or a count-in ahead of bar 1 does not begin on beat
    /// one at zero, and an author writing `start: 0ms` is wrong by exactly this
    /// much with nothing to warn them.
    ///
    /// This is the first *downbeat*, not the first beat. `measure_starts` comes
    /// from accented onsets and nothing guarantees the first click is one: a
    /// count-in of plain clicks ahead of the first accent would otherwise place
    /// bar 1 early by the pickup and shift every `@bar/beat` cue with it.
    pub fn lead_in(&self) -> Duration {
        let first_downbeat = self
            .measure_starts
            .first()
            .and_then(|index| self.beats.get(*index))
            .or_else(|| self.beats.first());
        first_downbeat
            .map(|secs| Duration::from_secs_f64(secs.max(0.0)))
            .unwrap_or(Duration::ZERO)
    }

    /// Derives a tempo map from this grid, so a show can use `@bar/beat` cueing
    /// against a song whose timing came from the click track rather than a
    /// hand-written `tempo:` block.
    ///
    /// The grid is ground truth from the audio, so nothing here rounds to a
    /// "nice" BPM — that would drift a cue by half a second across a long song.
    /// Instead the measured per-measure tempo is kept, and a change is emitted
    /// only when the tempo moves more than [`TEMPO_CHANGE_TOLERANCE`] or the
    /// bar length changes. A steady song therefore yields no tempo changes at
    /// all, and a song with two 2/4 bars yields exactly the two meter changes
    /// around them.
    ///
    /// Returns `None` for a grid too small to describe a tempo — under two
    /// measure boundaries there is no complete bar to measure.
    pub fn to_tempo_map(&self) -> Option<crate::tempo::TempoMap> {
        use crate::tempo::{TempoChange, TempoChangePosition, TempoTransition, TimeSignature};

        let measures = self.measures()?;
        let first = measures.first()?;

        // Pass one: where does the tempo or the meter actually move? The
        // decision uses each bar's own measured tempo, which is why
        // `TEMPO_CHANGE_TOLERANCE` has to stay coarser than the detector's hop.
        let mut boundaries = Vec::new();
        let mut current_bpm = first.bpm;
        let mut current_beats = first.beats;
        for (index, measure) in measures.iter().enumerate().skip(1) {
            let tempo_moved = (measure.bpm - current_bpm).abs() / current_bpm.max(f64::EPSILON)
                > TEMPO_CHANGE_TOLERANCE;
            let meter_moved = measure.beats != current_beats;
            if !tempo_moved && !meter_moved {
                continue;
            }
            boundaries.push((index, tempo_moved, meter_moved));
            if tempo_moved {
                current_bpm = measure.bpm;
            }
            if meter_moved {
                current_beats = measure.beats;
            }
        }

        // Pass two: re-measure each constant-tempo run across its whole span.
        // A single bar is a poor tempo estimate — detected beats land on hop
        // boundaries, so a 2s bar carries ~0.3% of BPM error, and holding that
        // estimate for the rest of the song walks the bars off the grid
        // linearly (0.27s across 100 bars of a metronomic click). Averaging
        // over the run divides that error by the number of bars in it.
        let tempo_starts: Vec<usize> = std::iter::once(0)
            .chain(
                boundaries
                    .iter()
                    .filter(|(_, tempo_moved, _)| *tempo_moved)
                    .map(|(index, _, _)| *index),
            )
            .collect();
        let mut refined = vec![0.0_f64; measures.len()];
        for (position, &start) in tempo_starts.iter().enumerate() {
            let end = tempo_starts
                .get(position + 1)
                .copied()
                .unwrap_or(measures.len());
            let run = &measures[start..end];
            let beats: usize = run.iter().map(|m| m.beats).sum();
            let seconds = run.last().map_or(0.0, |m| m.end) - run.first().map_or(0.0, |m| m.start);
            let bpm = if beats > 0 && seconds > 0.0 {
                beats as f64 * 60.0 / seconds
            } else {
                measures[start].bpm
            };
            for slot in &mut refined[start..end] {
                *slot = bpm;
            }
        }

        let changes = boundaries
            .into_iter()
            .map(|(index, tempo_moved, meter_moved)| {
                let measure = &measures[index];
                TempoChange {
                    position: TempoChangePosition::MeasureBeat(measure.number, 1.0),
                    original_measure_beat: Some((measure.number, 1.0)),
                    bpm: tempo_moved.then_some(refined[index]),
                    time_signature: meter_moved
                        .then(|| TimeSignature::from_grid_beats(measure.beats as u32)),
                    transition: TempoTransition::Snap,
                }
            })
            .collect();

        Some(crate::tempo::TempoMap::new(
            // Bar 1 beat 1 is the first detected beat, not zero. This is the
            // lead-in an author would otherwise have to measure by hand.
            self.lead_in(),
            refined[0],
            TimeSignature::from_grid_beats(first.beats as u32),
            changes,
        ))
    }

    /// Each complete measure's 1-based number, beat count, and measured tempo.
    /// `None` when the grid holds fewer than two measure boundaries.
    fn measures(&self) -> Option<Vec<MeasureTempo>> {
        if self.measure_starts.len() < 2 {
            return None;
        }

        let mut measures = Vec::new();
        for (index, pair) in self.measure_starts.windows(2).enumerate() {
            let (start_beat, end_beat) = (pair[0], pair[1]);
            let beats = end_beat.saturating_sub(start_beat);
            let (Some(start), Some(end)) = (self.beats.get(start_beat), self.beats.get(end_beat))
            else {
                continue;
            };
            let seconds = end - start;
            if beats == 0 || seconds <= 0.0 {
                continue;
            }
            measures.push(MeasureTempo {
                number: index as u32 + 1,
                beats,
                bpm: beats as f64 * 60.0 / seconds,
                start: *start,
                end: *end,
            });
        }

        (!measures.is_empty()).then_some(measures)
    }
}

/// Relative tempo movement below which a derived map emits no change. Detected
/// beats jitter by a few milliseconds; without a tolerance a steady song would
/// produce a tempo change at every bar.
const TEMPO_CHANGE_TOLERANCE: f64 = 0.005;

/// One measure's measured tempo, used while deriving a tempo map.
struct MeasureTempo {
    /// 1-based measure number.
    number: u32,
    /// Beats in this measure.
    beats: usize,
    /// Tempo measured across this measure alone. Good enough to detect that the
    /// tempo moved; too noisy to hold for a whole run of bars.
    bpm: f64,
    /// Absolute time of this measure's downbeat.
    start: f64,
    /// Absolute time of the next measure's downbeat.
    end: f64,
}

#[cfg(test)]
mod to_tempo_map_tests {
    use super::*;
    use crate::tempo::TimeSignature;

    /// Builds a grid from measure beat-counts at a steady tempo, starting at
    /// `lead_in` seconds.
    fn grid(lead_in: f64, bpm: f64, measures: &[usize]) -> BeatGrid {
        let spacing = 60.0 / bpm;
        let mut beats = Vec::new();
        let mut measure_starts = Vec::new();
        let mut t = lead_in;
        for count in measures {
            measure_starts.push(beats.len());
            for _ in 0..*count {
                beats.push(t);
                t += spacing;
            }
        }
        // A trailing boundary so the last measure is complete.
        measure_starts.push(beats.len());
        beats.push(t);
        BeatGrid {
            beats,
            measure_starts,
        }
    }

    #[test]
    fn steady_song_yields_no_tempo_changes() {
        let map = grid(0.0, 120.0, &[4, 4, 4, 4]).to_tempo_map().expect("map");
        assert!((map.initial_bpm - 120.0).abs() < 0.01);
        assert_eq!(map.initial_time_signature, TimeSignature::new(4, 4));
        assert!(
            map.changes.is_empty(),
            "a steady song should need no changes: {:?}",
            map.changes
        );
    }

    #[test]
    fn a_short_bar_becomes_a_meter_change_and_back() {
        // The shape from #337: 4/4 throughout with one 2/4 bar in the middle.
        let map = grid(0.0, 80.0, &[4, 4, 4, 2, 4, 4])
            .to_tempo_map()
            .expect("map");
        assert_eq!(map.initial_time_signature, TimeSignature::new(4, 4));

        let meters: Vec<(u32, u32)> = map
            .changes
            .iter()
            .filter_map(|c| {
                c.original_measure_beat
                    .map(|(m, _)| m)
                    .zip(c.time_signature.map(|ts| ts.numerator))
            })
            .collect();
        assert_eq!(
            meters,
            vec![(4, 2), (5, 4)],
            "expected 2/4 at bar 4 and back to 4/4 at bar 5: {:?}",
            map.changes
        );
        assert!(
            map.changes.iter().all(|c| c.bpm.is_none()),
            "tempo did not move, only the meter: {:?}",
            map.changes
        );
    }

    #[test]
    fn a_real_tempo_change_is_captured() {
        let mut g = grid(0.0, 100.0, &[4, 4]);
        // Append two bars at 140bpm.
        let spacing = 60.0 / 140.0;
        let mut t = *g.beats.last().expect("beats");
        for _ in 0..2 {
            g.measure_starts.push(g.beats.len());
            for _ in 0..4 {
                g.beats.push(t);
                t += spacing;
            }
        }
        g.measure_starts.push(g.beats.len());
        g.beats.push(t);

        let map = g.to_tempo_map().expect("map");
        let tempo_changes: Vec<f64> = map.changes.iter().filter_map(|c| c.bpm).collect();
        assert_eq!(tempo_changes.len(), 1, "{:?}", map.changes);
        assert!(
            (tempo_changes[0] - 140.0).abs() < 1.0,
            "expected ~140bpm, got {}",
            tempo_changes[0]
        );
    }

    #[test]
    fn jitter_below_tolerance_emits_no_change() {
        // Detected beats are never exact. A few milliseconds of wobble must not
        // produce a tempo change at every bar.
        let mut g = grid(0.0, 120.0, &[4, 4, 4, 4]);
        for (i, beat) in g.beats.iter_mut().enumerate() {
            *beat += if i % 2 == 0 { 0.001 } else { -0.001 };
        }
        let map = g.to_tempo_map().expect("map");
        assert!(
            map.changes.is_empty(),
            "jitter should not read as tempo changes: {:?}",
            map.changes
        );
    }

    #[test]
    fn the_lead_in_becomes_the_tempo_origin() {
        // Bar 1 beat 1 is where the click actually starts, not zero.
        let g = grid(2.5, 120.0, &[4, 4]);
        assert_eq!(g.lead_in(), Duration::from_secs_f64(2.5));

        let map = g.to_tempo_map().expect("map");
        let bar1 = map
            .measure_to_time_with_offset(1, 1.0, 0, 0.0)
            .expect("bar 1");
        assert!(
            (bar1.as_secs_f64() - 2.5).abs() < 0.01,
            "bar 1 should land on the first beat, got {bar1:?}"
        );
    }

    #[test]
    fn the_lead_in_is_the_first_downbeat_not_the_first_click() {
        // A count-in of unaccented clicks ahead of bar 1: `measure_starts[0]`
        // is not 0, so taking `beats[0]` as the origin puts bar 1 early by the
        // whole pickup and drags every cue with it.
        let spacing = 0.5_f64; // 120bpm
        let mut beats = Vec::new();
        // Four count-in clicks, none accented.
        for i in 0..4 {
            beats.push(i as f64 * spacing);
        }
        let mut measure_starts = Vec::new();
        for measure in 0..3 {
            measure_starts.push(beats.len());
            for beat in 0..4 {
                beats.push((4 + measure * 4 + beat) as f64 * spacing);
            }
        }
        measure_starts.push(beats.len());
        beats.push((4 + 12) as f64 * spacing);

        let grid = BeatGrid {
            beats,
            measure_starts,
        };
        // The first downbeat is after the four-click count-in.
        assert_eq!(grid.lead_in(), Duration::from_secs_f64(2.0));

        let map = grid.to_tempo_map().expect("map");
        let bar1 = map
            .measure_to_time_with_offset(1, 1.0, 0, 0.0)
            .expect("bar 1");
        assert!(
            (bar1.as_secs_f64() - 2.0).abs() < 0.01,
            "bar 1 should land on the first downbeat, got {bar1:?}"
        );
    }

    #[test]
    fn round_trips_through_the_forward_derivation() {
        // to_tempo_map and from_tempo_map are inverses over a grid's own span.
        let original = grid(0.0, 96.0, &[4, 4, 4, 4]);
        let map = original.to_tempo_map().expect("map");
        let rebuilt = BeatGrid::from_tempo_map(&map, Duration::from_secs_f64(10.0));

        assert_eq!(rebuilt.measure_starts.len(), 4, "{rebuilt:?}");
        for (a, b) in original.beats.iter().zip(rebuilt.beats.iter()) {
            assert!((a - b).abs() < 0.01, "beat drift: {a} vs {b}");
        }
    }

    #[test]
    fn bar_times_track_the_grid_through_a_sustained_ramp() {
        // A one-signed accelerando is where re-integrating each change from the
        // previous anchor at the stale BPM accumulates: the error never
        // cancels. Anchoring changes to their measured downbeat keeps every bar
        // on the grid it came from.
        let mut beats = Vec::new();
        let mut measure_starts = Vec::new();
        let mut t = 0.0_f64;
        let mut downbeats = Vec::new();
        for measure in 0..32 {
            measure_starts.push(beats.len());
            downbeats.push(t);
            // 100bpm ramping to ~140 over the span.
            let bpm = 100.0 + (measure as f64) * 40.0 / 32.0;
            let spacing = 60.0 / bpm;
            for _ in 0..4 {
                beats.push(t);
                t += spacing;
            }
        }
        measure_starts.push(beats.len());
        beats.push(t);

        let grid = BeatGrid {
            beats,
            measure_starts,
        };
        let map = grid.to_tempo_map().expect("map");

        let mut worst = 0.0_f64;
        for (index, expected) in downbeats.iter().enumerate() {
            let got = map
                .measure_to_time_with_offset(index as u32 + 1, 1.0, 0, 0.0)
                .expect("bar")
                .as_secs_f64();
            worst = worst.max((got - expected).abs());
        }
        println!("MEASURED worst drift = {worst:.6}s");
        assert!(
            worst < 0.020,
            "bar times drifted from the grid by {worst:.3}s across the ramp"
        );
    }

    #[test]
    fn a_steady_click_derives_no_tempo_changes_despite_detector_quantization() {
        // Detected beat times land on hop boundaries (256 samples @ 44.1k =
        // 5.805ms), so even a metronomic click reports a per-bar BPM that
        // wobbles. Any emit threshold finer than that hop mistakes the
        // quantization for tempo movement: a 5ms drift bound derived 93 changes
        // across these 100 bars. The relative tolerance has to stay coarser
        // than the detector's own resolution.
        let hop = 256.0 / 44100.0;
        let spacing = 0.5_f64; // 120bpm
        let mut beats = Vec::new();
        let mut measure_starts = Vec::new();
        for measure in 0..100 {
            measure_starts.push(beats.len());
            for beat in 0..4 {
                let exact = (measure * 4 + beat) as f64 * spacing;
                beats.push((exact / hop).round() * hop);
            }
        }
        measure_starts.push(beats.len());
        beats.push(400.0 * spacing);

        let grid = BeatGrid {
            beats,
            measure_starts,
        };
        let map = grid.to_tempo_map().expect("map");

        let mut worst = 0.0_f64;
        for measure in 0..100u32 {
            let got = map
                .measure_to_time_with_offset(measure + 1, 1.0, 0, 0.0)
                .expect("bar")
                .as_secs_f64();
            let expected = measure as f64 * 4.0 * spacing;
            worst = worst.max((got - expected).abs());
        }
        assert!(
            worst < 0.005,
            "bar times drifted {worst:.4}s from a metronomic click"
        );
    }

    #[test]
    fn a_ramp_gentler_than_the_tempo_tolerance_still_tracks_the_grid() {
        // Each bar only 0.4% faster than the last — under the relative
        // threshold, so this whole span reads as one constant-tempo run. Fitting
        // that run across its full extent puts the map through the middle of the
        // ramp instead of pinning it to bar 1's tempo and walking away from the
        // grid.
        let mut beats = Vec::new();
        let mut measure_starts = Vec::new();
        let mut downbeats = Vec::new();
        let mut t = 0.0_f64;
        let mut bpm = 100.0_f64;
        for _ in 0..64 {
            measure_starts.push(beats.len());
            downbeats.push(t);
            let spacing = 60.0 / bpm;
            for _ in 0..4 {
                beats.push(t);
                t += spacing;
            }
            bpm *= 1.004;
        }
        measure_starts.push(beats.len());
        beats.push(t);

        let map = BeatGrid {
            beats,
            measure_starts,
        }
        .to_tempo_map()
        .expect("map");

        let mut worst = 0.0_f64;
        for (index, expected) in downbeats.iter().enumerate() {
            let got = map
                .measure_to_time_with_offset(index as u32 + 1, 1.0, 0, 0.0)
                .expect("bar")
                .as_secs_f64();
            worst = worst.max((got - expected).abs());
        }
        assert!(
            worst < 0.010,
            "bar times drifted {worst:.4}s from the grid across a gradual ramp"
        );
    }

    #[test]
    fn a_grid_too_small_to_describe_a_tempo_yields_none() {
        assert!(BeatGrid {
            beats: vec![],
            measure_starts: vec![]
        }
        .to_tempo_map()
        .is_none());
        assert!(BeatGrid {
            beats: vec![0.0, 0.5],
            measure_starts: vec![0]
        }
        .to_tempo_map()
        .is_none());
    }
}

#[cfg(test)]
mod position_at_tests {
    use super::*;

    fn grid() -> BeatGrid {
        // Two 4/4 measures at 120 BPM, then a trailing downbeat.
        BeatGrid {
            beats: vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0],
            measure_starts: vec![0, 4, 8],
        }
    }

    #[test]
    fn position_is_the_beat_and_the_fraction_through_it() {
        let grid = grid();
        assert_eq!(grid.position_at(0.0), Some((0, 1.0)));
        assert_eq!(grid.position_at(0.25), Some((0, 1.5)));
        assert_eq!(grid.position_at(1.9), Some((0, 4.8)));
        assert_eq!(grid.position_at(2.0), Some((1, 1.0)));
        assert_eq!(grid.position_at(3.75), Some((1, 4.5)));
    }

    #[test]
    fn position_round_trips_through_beat_time() {
        let grid = grid();
        for i in 0..80 {
            let secs = i as f64 * 0.05;
            let (measure, beat) = grid.position_at(secs).expect("on the grid");
            let back = grid.beat_time(measure, beat).expect("resolves");
            assert!(
                (back - secs).abs() < 1e-9,
                "{secs} -> {measure}/{beat} -> {back}"
            );
        }
    }

    #[test]
    fn clamps_to_the_grid() {
        assert_eq!(grid().position_at(-5.0), Some((0, 1.0)));
        assert_eq!(grid().position_at(99.0), Some((2, 1.0)));
    }

    #[test]
    fn snaps_to_the_nearest_step_and_rolls_into_the_next_measure() {
        let grid = grid();
        assert_eq!(grid.snapped_position_at(0.2, 1.0), Some((0, 1.0)));
        assert_eq!(grid.snapped_position_at(0.3, 1.0), Some((0, 2.0)));
        assert_eq!(grid.snapped_position_at(0.3, 0.5), Some((0, 1.5)));
        // Three quarters through beat 4: nearest beat is the next downbeat,
        // never "beat 5" of a four-beat measure.
        assert_eq!(grid.snapped_position_at(1.9, 1.0), Some((1, 1.0)));
        assert_eq!(grid.snapped_position_at(1.8, 0.5), Some((0, 4.5)));
        assert_eq!(grid.snapped_position_at(1.9, 0.5), Some((1, 1.0)));
        // Past the grid there is nothing to roll into.
        assert_eq!(grid.snapped_position_at(99.0, 1.0), Some((2, 1.0)));
        assert_eq!(grid.snapped_position_at(1.0, 0.0), None);
    }

    #[test]
    fn empty_or_unmeasured_grid_has_no_position() {
        let empty = BeatGrid {
            beats: vec![],
            measure_starts: vec![],
        };
        assert_eq!(empty.position_at(1.0), None);
        let unmeasured = BeatGrid {
            beats: vec![0.0, 0.5],
            measure_starts: vec![],
        };
        assert_eq!(unmeasured.position_at(0.0), None);
        // A pickup beat before the first downbeat has no measure.
        let pickup = BeatGrid {
            beats: vec![0.0, 0.5, 1.0, 1.5],
            measure_starts: vec![1],
        };
        assert_eq!(pickup.position_at(0.0), None);
        assert_eq!(pickup.position_at(0.5), Some((0, 1.0)));
        assert_eq!(grid().position_at(f64::NAN), None);
    }
}
