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

//! Estimate a tempo map from a beat grid detected by click track analysis.
//!
//! Two-pass approach:
//!   1. **Find stable sections** — runs of beats with BPM within a tight band
//!   2. **Classify gaps** — gaps between stable sections are snap transitions
//!
//! Detected boundaries are snapped to the nearest measure boundary.
//! Time signature changes are detected separately from measure spacing.

use serde::Serialize;

use super::click_analysis::BeatGrid;

/// A guessed tempo map derived from beat grid analysis.
#[derive(Debug, Clone, Serialize)]
pub struct GuessedTempo {
    pub start_seconds: f64,
    pub bpm: u32,
    pub time_signature: [u32; 2],
    pub changes: Vec<GuessedTempoChange>,
}

/// A single tempo or time-signature change.
#[derive(Debug, Clone, Serialize)]
pub struct GuessedTempoChange {
    pub measure: u32,
    pub beat: u32,
    pub bpm: u32,
    pub time_signature: [u32; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transition_beats: Option<u32>,
}

// ── Configuration ───────────────────────────────────────────────────────────

/// Maximum BPM deviation from the seed mean for a beat to be "stable."
const STABLE_BAND: f64 = 4.0;

/// Minimum number of consecutive in-band beats to form a stable section.
const MIN_STABLE_BEATS: usize = 8;

/// How many beats a boundary can be nudged to snap to a measure start.
const SNAP_TOLERANCE: usize = 3;

/// Maximum gap between adjacent sections to merge (jitter repair).
const MAX_MERGE_GAP: usize = 8;

// ── Public API ──────────────────────────────────────────────────────────────

pub fn guess_tempo(grid: &BeatGrid) -> Option<GuessedTempo> {
    if grid.beats.len() < 2 {
        return None;
    }

    let bpms = compute_instantaneous_bpms(grid);
    let measure_lengths = compute_measure_lengths(grid);

    // Pass 1: Find stable sections
    let mut stables = find_stable_sections(&bpms);

    // Merge adjacent fragments at similar BPM (jitter repair)
    merge_nearby_sections(&mut stables, &bpms);

    // Force splits at time signature changes
    let ts_changes = find_time_sig_change_beats(&measure_lengths);
    split_at_time_sig_changes(&mut stables, &ts_changes, &bpms);

    if stables.is_empty() {
        return None;
    }

    // Snap section starts to measure boundaries
    for sec in stables.iter_mut() {
        sec.start = snap_to_measure(sec.start, &grid.measure_starts, SNAP_TOLERANCE);
    }

    // Pass 2: Build tempo changes from gaps between stable sections
    let base_bpm = round_bpm(stables[0].bpm);
    let base_ts = beats_per_measure_at(stables[0].start, &measure_lengths);
    let changes = build_changes(&stables, &measure_lengths, base_bpm, base_ts);

    Some(GuessedTempo {
        start_seconds: grid.beats[0],
        bpm: base_bpm,
        time_signature: [base_ts, 4],
        changes,
    })
}

// ── Pass 1: Find stable sections ────────────────────────────────────────────

#[derive(Debug, Clone)]
struct StableSection {
    start: usize,
    end: usize,
    bpm: f64,
}

fn find_stable_sections(bpms: &[f64]) -> Vec<StableSection> {
    let mut sections = Vec::new();
    let mut pos = 0;

    while pos < bpms.len() {
        let section_end = extend_stable(bpms, pos);
        if section_end - pos >= MIN_STABLE_BEATS {
            let mean = mean_of(&bpms[pos..section_end]);
            sections.push(StableSection {
                start: pos,
                end: section_end,
                bpm: mean,
            });
            pos = section_end;
        } else {
            pos += 1;
        }
    }

    sections
}

/// Extend a stable section from `pos` using a fixed seed mean.
fn extend_stable(bpms: &[f64], pos: usize) -> usize {
    if pos >= bpms.len() {
        return pos;
    }

    let seed_end = (pos + MIN_STABLE_BEATS).min(bpms.len());
    if seed_end - pos < MIN_STABLE_BEATS {
        return pos;
    }

    let seed_bpms = &bpms[pos..seed_end];
    let seed_mean = seed_bpms.iter().sum::<f64>() / seed_bpms.len() as f64;
    let max_dev = seed_bpms
        .iter()
        .map(|b| (b - seed_mean).abs())
        .fold(0.0f64, f64::max);
    if max_dev > STABLE_BAND {
        return pos;
    }

    let mut end = seed_end;
    while end < bpms.len() {
        if (bpms[end] - seed_mean).abs() > STABLE_BAND {
            break;
        }
        end += 1;
    }

    end
}

/// Merge adjacent stable sections at similar BPM separated by small gaps.
fn merge_nearby_sections(stables: &mut Vec<StableSection>, bpms: &[f64]) {
    let mut i = 0;
    while i + 1 < stables.len() {
        let gap = stables[i + 1].start.saturating_sub(stables[i].end);
        let bpm_diff = (stables[i].bpm - stables[i + 1].bpm).abs();

        let gap_is_transitional = if gap > 0 && stables[i].end < bpms.len() {
            let gap_end = stables[i + 1].start.min(bpms.len());
            let gap_bpms = &bpms[stables[i].end..gap_end];
            !gap_bpms.is_empty()
                && gap_bpms
                    .iter()
                    .any(|b| (b - stables[i].bpm).abs() > STABLE_BAND * 2.0)
        } else {
            false
        };

        if gap <= MAX_MERGE_GAP && bpm_diff <= STABLE_BAND && !gap_is_transitional {
            stables[i].end = stables[i + 1].end;
            stables[i].bpm = mean_of(&bpms[stables[i].start..stables[i].end]);
            stables.remove(i + 1);
        } else {
            i += 1;
        }
    }
}

/// Split stable sections at time signature change boundaries.
fn split_at_time_sig_changes(
    sections: &mut Vec<StableSection>,
    ts_change_beats: &[usize],
    bpms: &[f64],
) {
    for &beat in ts_change_beats {
        let mut i = 0;
        while i < sections.len() {
            let sec = &sections[i];
            if beat > sec.start && beat < sec.end {
                let first = StableSection {
                    start: sec.start,
                    end: beat,
                    bpm: mean_of(&bpms[sec.start..beat]),
                };
                let second = StableSection {
                    start: beat,
                    end: sec.end,
                    bpm: mean_of(&bpms[beat..sec.end]),
                };
                sections.splice(i..=i, [first, second]);
                i += 2;
            } else {
                i += 1;
            }
        }
    }
}

// ── Boundary snapping ───────────────────────────────────────────────────────

/// Snap a beat index to the nearest measure start within tolerance,
/// preferring the forward (at or after) direction.
fn snap_to_measure(beat: usize, measure_starts: &[usize], tolerance: usize) -> usize {
    let mut best = beat;
    let mut best_dist = tolerance + 1;
    let mut best_forward = false;

    for &ms in measure_starts {
        let dist = (ms as isize - beat as isize).unsigned_abs();
        if dist > tolerance {
            if ms > beat {
                break;
            }
            continue;
        }
        let is_forward = ms >= beat;
        if dist < best_dist || (dist == best_dist && is_forward && !best_forward) {
            best = ms;
            best_dist = dist;
            best_forward = is_forward;
        }
    }

    if best < beat {
        for &ms in measure_starts {
            if ms >= beat {
                let forward_dist = ms - beat;
                if forward_dist <= tolerance {
                    return ms;
                }
                break;
            }
        }
    }

    best
}

// ── Pass 2: Build tempo changes ─────────────────────────────────────────────

fn build_changes(
    stables: &[StableSection],
    measure_lengths: &[(usize, u32)],
    base_bpm: u32,
    base_ts: u32,
) -> Vec<GuessedTempoChange> {
    let mut changes = Vec::new();
    let mut current_bpm = base_bpm;
    let mut current_ts = base_ts;

    for sec in stables.iter().skip(1) {
        let bpm = round_bpm(sec.bpm);
        let ts = beats_per_measure_at(sec.start, measure_lengths);

        if bpm != current_bpm || ts != current_ts {
            let (measure, beat) = beat_to_measure_beat(sec.start, measure_lengths);
            changes.push(GuessedTempoChange {
                measure,
                beat,
                bpm,
                time_signature: [ts, 4],
                transition_beats: None,
            });
            current_bpm = bpm;
            current_ts = ts;
        }
    }

    changes
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn compute_instantaneous_bpms(grid: &BeatGrid) -> Vec<f64> {
    let mut bpms = Vec::with_capacity(grid.beats.len());
    for i in 0..grid.beats.len() - 1 {
        let interval = grid.beats[i + 1] - grid.beats[i];
        bpms.push(if interval > 0.0 { 60.0 / interval } else { 0.0 });
    }
    if let Some(&last) = bpms.last() {
        bpms.push(last);
    }
    bpms
}

fn compute_measure_lengths(grid: &BeatGrid) -> Vec<(usize, u32)> {
    let mut result = Vec::new();
    if grid.measure_starts.len() >= 2 {
        for i in 0..grid.measure_starts.len() {
            let len = if i + 1 < grid.measure_starts.len() {
                (grid.measure_starts[i + 1] - grid.measure_starts[i]) as u32
            } else if i > 0 {
                (grid.measure_starts[i] - grid.measure_starts[i - 1]) as u32
            } else {
                4
            };
            result.push((grid.measure_starts[i], len.max(1)));
        }
    }
    result
}

fn beats_per_measure_at(beat_idx: usize, measure_lengths: &[(usize, u32)]) -> u32 {
    let mut result = 4u32;
    for &(start, len) in measure_lengths {
        if start <= beat_idx {
            result = len;
        }
    }
    result
}

fn find_time_sig_change_beats(measure_lengths: &[(usize, u32)]) -> Vec<usize> {
    let mut changes = Vec::new();
    if measure_lengths.len() < 2 {
        return changes;
    }
    let mut prev = measure_lengths[0].1;
    for &(start, len) in measure_lengths.iter().skip(1) {
        if len != prev {
            changes.push(start);
            prev = len;
        }
    }
    changes
}

fn beat_to_measure_beat(beat_idx: usize, measure_lengths: &[(usize, u32)]) -> (u32, u32) {
    let mut measure: u32 = 1;
    let mut current_beat: usize = 0;
    let mut current_bpm = 4u32;

    for &(start, len) in measure_lengths {
        if start > beat_idx {
            break;
        }
        if start > current_beat {
            let beats_in_section = start - current_beat;
            measure += (beats_in_section / current_bpm as usize) as u32;
            current_beat = start;
        }
        current_bpm = len;
    }

    let remaining = beat_idx - current_beat;
    measure += (remaining / current_bpm as usize) as u32;
    let beat = (remaining % current_bpm as usize) as u32 + 1;

    (measure, beat)
}

fn mean_of(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    values.iter().sum::<f64>() / values.len() as f64
}

fn round_bpm(bpm: f64) -> u32 {
    bpm.round().max(1.0) as u32
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_grid(bpm: f64, beats: usize, beats_per_measure: usize) -> BeatGrid {
        let interval = 60.0 / bpm;
        let beat_times: Vec<f64> = (0..beats).map(|i| i as f64 * interval).collect();
        let measure_starts: Vec<usize> = (0..beats).step_by(beats_per_measure).collect();
        BeatGrid {
            beats: beat_times,
            measure_starts,
        }
    }

    fn make_grid_with_offset(
        bpm: f64,
        beats: usize,
        beats_per_measure: usize,
        offset: f64,
    ) -> BeatGrid {
        let interval = 60.0 / bpm;
        let beat_times: Vec<f64> = (0..beats).map(|i| offset + i as f64 * interval).collect();
        let measure_starts: Vec<usize> = (0..beats).step_by(beats_per_measure).collect();
        BeatGrid {
            beats: beat_times,
            measure_starts,
        }
    }

    #[test]
    fn too_few_beats_returns_none() {
        assert!(guess_tempo(&BeatGrid {
            beats: vec![0.0],
            measure_starts: vec![0]
        })
        .is_none());
    }

    #[test]
    fn constant_tempo() {
        let result = guess_tempo(&make_grid(120.0, 64, 4)).unwrap();
        assert_eq!(result.bpm, 120);
        assert_eq!(result.time_signature, [4, 4]);
        assert!(result.changes.is_empty(), "Got: {:?}", result.changes);
    }

    #[test]
    fn start_offset_captured() {
        let result = guess_tempo(&make_grid_with_offset(120.0, 64, 4, 1.5)).unwrap();
        assert!((result.start_seconds - 1.5).abs() < 0.001);
    }

    #[test]
    fn detects_time_signature_change() {
        let mut beats = Vec::new();
        let mut measure_starts = Vec::new();
        let mut t = 0.0;
        let interval = 60.0 / 120.0;
        for i in 0..32 {
            if i % 4 == 0 {
                measure_starts.push(beats.len());
            }
            beats.push(t);
            t += interval;
        }
        for i in 0..24 {
            if i % 3 == 0 {
                measure_starts.push(beats.len());
            }
            beats.push(t);
            t += interval;
        }
        let result = guess_tempo(&BeatGrid {
            beats,
            measure_starts,
        })
        .unwrap();
        assert_eq!(result.time_signature, [4, 4]);
        let ts = result.changes.iter().find(|c| c.time_signature != [4, 4]);
        assert!(
            ts.is_some(),
            "Expected time sig change, got: {:?}",
            result.changes
        );
        assert_eq!(ts.unwrap().time_signature, [3, 4]);
    }

    #[test]
    fn detects_snap_tempo_change() {
        let mut beats = Vec::new();
        let mut t = 0.0;
        for _ in 0..32 {
            beats.push(t);
            t += 60.0 / 120.0;
        }
        for _ in 0..32 {
            beats.push(t);
            t += 60.0 / 160.0;
        }
        let measure_starts: Vec<usize> = (0..64).step_by(4).collect();
        let result = guess_tempo(&BeatGrid {
            beats,
            measure_starts,
        })
        .unwrap();
        assert_eq!(result.bpm, 120);
        let c = result.changes.iter().find(|c| c.bpm >= 158);
        assert!(c.is_some(), "Expected ~160, got: {:?}", result.changes);
        assert!(c.unwrap().transition_beats.is_none(), "Should be snap");
        assert_eq!(c.unwrap().beat, 1, "Should snap to beat 1");
    }

    #[test]
    fn jitter_not_detected() {
        let mut beats = Vec::new();
        let mut t = 0.0;
        for i in 0..64 {
            beats.push(t);
            let jitter = if i % 2 == 0 { 0.98 } else { 1.02 };
            t += (60.0 / 120.0) * jitter;
        }
        let measure_starts: Vec<usize> = (0..64).step_by(4).collect();
        let result = guess_tempo(&BeatGrid {
            beats,
            measure_starts,
        })
        .unwrap();
        assert!(
            (result.bpm as i32 - 120).unsigned_abs() <= 2,
            "Base BPM {} should be ~120",
            result.bpm
        );
        assert!(result.changes.is_empty(), "Got: {:?}", result.changes);
    }

    #[test]
    fn saxon_shore_click_track() {
        let click_path = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
            .join("src/backing-tracks/Isenmor/Saxon Shore/Click.flac");
        if !click_path.exists() {
            eprintln!("Skipping: not found");
            return;
        }

        let grid =
            crate::audio::click_analysis::analyze_click_track_default(&click_path, 0).unwrap();
        let result = guess_tempo(&grid).unwrap();

        eprintln!("Base: {} BPM, {:?}", result.bpm, result.time_signature);
        for c in &result.changes {
            eprintln!(
                "  m{}/{}  {} BPM  ts={}/{}  transition={:?}",
                c.measure,
                c.beat,
                c.bpm,
                c.time_signature[0],
                c.time_signature[1],
                c.transition_beats
            );
        }

        assert_eq!(result.bpm, 150, "Base should be 150");
        let c160 = result.changes.iter().find(|c| c.bpm >= 158);
        assert!(c160.is_some(), "Expected ~160, got: {:?}", result.changes);
        assert!(
            c160.unwrap().transition_beats.is_none(),
            "150->160 should be snap, got {:?}",
            c160.unwrap().transition_beats
        );
    }

    #[test]
    fn sigurds_song_click_track() {
        let click_path = std::path::Path::new(&std::env::var("HOME").unwrap_or_default())
            .join("src/backing-tracks/Isenmor/Sigurd's Song/Click.flac");
        if !click_path.exists() {
            eprintln!("Skipping: not found");
            return;
        }

        let grid =
            crate::audio::click_analysis::analyze_click_track_default(&click_path, 0).unwrap();
        let result = guess_tempo(&grid).unwrap();

        eprintln!("Base: {} BPM, {:?}", result.bpm, result.time_signature);
        for c in &result.changes {
            eprintln!(
                "  m{}/{}  {} BPM  ts={}/{}  transition={:?}",
                c.measure,
                c.beat,
                c.bpm,
                c.time_signature[0],
                c.time_signature[1],
                c.transition_beats
            );
        }

        assert_eq!(result.bpm, 120, "Base should be 120");
        let ts_change = result.changes.iter().find(|c| c.time_signature != [4, 4]);
        assert!(
            ts_change.is_some(),
            "Expected time sig change, got: {:?}",
            result.changes
        );
        let c155 = result.changes.iter().find(|c| c.bpm >= 153);
        assert!(c155.is_some(), "Expected ~155, got: {:?}", result.changes);
    }
}
