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

//! Offline analysis of click track audio to extract a beat grid.
//!
//! Detects onsets (clicks) in the audio and classifies them as accented
//! (measure boundaries) or normal beats. The result is a [`BeatGrid`] with
//! absolute beat times and measure boundary indices, suitable for snapping
//! loop points to musically meaningful positions.

use std::path::Path;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tracing::warn;

use super::sample_source::audio::AudioSampleSource;
use super::sample_source::traits::SampleSource;

// ── Onset detection parameters ───────────────────────────────────────────────

const WINDOW_SIZE: usize = 512;
const HOP_SIZE: usize = 256;
const LOCKOUT_MS: f64 = 30.0;
const THRESHOLD_MULTIPLIER: f32 = 3.0;
const NOISE_ALPHA: f32 = 0.001;
const MIN_THRESHOLD: f32 = 0.001;
const MIN_ONSETS_FOR_ANALYSIS: usize = 4;

// ── ZCR window for accent classification ─────────────────────────────────────

const ZCR_WINDOW_MS: f64 = 10.0;

// ── Public types ─────────────────────────────────────────────────────────────

/// A detected onset (click) in the audio.
#[derive(Debug, Clone)]
pub struct Onset {
    /// Absolute time in seconds from the start of the file.
    pub time_secs: f64,
    /// Peak amplitude of this onset (0.0–1.0).
    pub amplitude: f32,
}

/// A beat grid derived from click track analysis.
///
/// Contains the absolute time of every detected beat and identifies which
/// beats are measure boundaries (accented clicks). This is the ground truth
/// from the audio — no assumptions about note values or BPM.
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
            for k in 0..time_signature.numerator {
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

    /// The detected lead-in: how long before the first beat the song starts.
    ///
    /// A song with silence or a count-in ahead of bar 1 does not begin on beat
    /// one at zero, and an author writing `start: 0ms` is wrong by exactly this
    /// much with nothing to warn them.
    pub fn lead_in(&self) -> Duration {
        self.beats
            .first()
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
        let (first, rest) = measures.split_first()?;

        let mut changes = Vec::new();
        let mut current_bpm = first.bpm;
        let mut current_beats = first.beats;

        for measure in rest {
            let tempo_moved = (measure.bpm - current_bpm).abs() / current_bpm.max(f64::EPSILON)
                > TEMPO_CHANGE_TOLERANCE;
            let meter_moved = measure.beats != current_beats;
            if !tempo_moved && !meter_moved {
                continue;
            }
            changes.push(TempoChange {
                position: TempoChangePosition::MeasureBeat(measure.number, 1.0),
                original_measure_beat: Some((measure.number, 1.0)),
                bpm: tempo_moved.then_some(measure.bpm),
                time_signature: meter_moved.then(|| TimeSignature::new(measure.beats as u32, 4)),
                transition: TempoTransition::Snap,
            });
            if tempo_moved {
                current_bpm = measure.bpm;
            }
            if meter_moved {
                current_beats = measure.beats;
            }
        }

        Some(crate::tempo::TempoMap::new(
            // Bar 1 beat 1 is the first detected beat, not zero. This is the
            // lead-in an author would otherwise have to measure by hand.
            self.lead_in(),
            first.bpm,
            TimeSignature::new(first.beats as u32, 4),
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
    /// Tempo measured across this measure.
    bpm: f64,
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

/// Classifies onsets into "accented" (downbeat) vs "normal".
///
/// Returns a bool vec parallel to the input onsets where `true` = accented.
/// The `samples` parameter provides the full mono audio buffer for analysis.
pub trait AccentClassifier {
    fn classify(&self, onsets: &[Onset], samples: &[f32], sample_rate: u32) -> Vec<bool>;
}

/// Classifies accents by zero-crossing rate. Different click sounds (e.g.,
/// a higher-pitched cowbell vs a lower woodblock) have different ZCR values.
pub struct ZcrClassifier;

/// Classifies accents by amplitude. Louder clicks are considered accented.
pub struct AmplitudeClassifier;

// ── Onset detection ──────────────────────────────────────────────────────────

/// Reads audio from `file`, extracts a single channel, and returns the mono sample buffer.
fn read_mono_samples(file: &Path, file_channel: u16) -> Option<(Vec<f32>, u32)> {
    let mut source = match AudioSampleSource::from_file(file, None, 4096) {
        Ok(s) => s,
        Err(e) => {
            warn!("Failed to open click track {}: {}", file.display(), e);
            return None;
        }
    };

    let sample_rate = source.sample_rate();
    let channel_count = source.channel_count() as usize;
    if channel_count == 0 {
        return None;
    }

    let target_channel = (file_channel as usize).saturating_sub(1);
    if target_channel >= channel_count {
        warn!(
            "file_channel {} exceeds channel count {} for {}",
            file_channel,
            channel_count,
            file.display()
        );
        return None;
    }

    let mut mono = Vec::new();
    let mut buf = vec![0.0_f32; 16384];
    let mut interleaved_idx: usize = 0;

    loop {
        let n = match source.read_samples(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };

        for &sample in &buf[..n] {
            let ch = interleaved_idx % channel_count;
            interleaved_idx += 1;
            if ch == target_channel {
                mono.push(sample);
            }
        }
    }

    if mono.is_empty() {
        return None;
    }

    Some((mono, sample_rate))
}

/// Detects onsets in a mono audio buffer using short-time energy with adaptive
/// noise floor tracking.
pub fn detect_onsets(samples: &[f32], sample_rate: u32) -> Vec<Onset> {
    if samples.len() < WINDOW_SIZE {
        return vec![];
    }

    let lockout_samples = (LOCKOUT_MS / 1000.0 * sample_rate as f64) as usize;
    let mut onsets = Vec::new();
    let mut noise_floor: f32 = 0.0;
    let mut lockout_remaining: usize = 0;
    let mut in_onset = false;
    let mut onset_peak: f32 = 0.0;
    let mut onset_start_sample: usize = 0;

    let mut pos = 0;
    while pos + WINDOW_SIZE <= samples.len() {
        // Compute short-time energy for this window.
        let window = &samples[pos..pos + WINDOW_SIZE];
        let energy: f32 = window.iter().map(|s| s * s).sum::<f32>() / WINDOW_SIZE as f32;
        let rms = energy.sqrt();

        if lockout_remaining > 0 {
            lockout_remaining = lockout_remaining.saturating_sub(HOP_SIZE);
            pos += HOP_SIZE;
            continue;
        }

        let threshold = (noise_floor * THRESHOLD_MULTIPLIER).max(MIN_THRESHOLD);

        if rms > threshold {
            if !in_onset {
                in_onset = true;
                onset_peak = rms;
                onset_start_sample = pos;
            } else if rms > onset_peak {
                onset_peak = rms;
                onset_start_sample = pos;
            }
        } else if in_onset {
            // End of onset — record it.
            let time_secs =
                (onset_start_sample as f64 + WINDOW_SIZE as f64 / 2.0) / sample_rate as f64;
            onsets.push(Onset {
                time_secs,
                amplitude: onset_peak,
            });
            in_onset = false;
            lockout_remaining = lockout_samples;
        }

        // Update noise floor only when below threshold (quiet regions).
        if rms < threshold {
            noise_floor = noise_floor * (1.0 - NOISE_ALPHA) + rms * NOISE_ALPHA;
        }

        pos += HOP_SIZE;
    }

    // If we ended mid-onset, capture it.
    if in_onset {
        let time_secs = (onset_start_sample as f64 + WINDOW_SIZE as f64 / 2.0) / sample_rate as f64;
        onsets.push(Onset {
            time_secs,
            amplitude: onset_peak,
        });
    }

    onsets
}

// ── Accent classification ────────────────────────────────────────────────────

/// Computes the zero-crossing rate of a short window around sample index `center`.
fn compute_zcr(samples: &[f32], center: usize, half_window: usize) -> f32 {
    let start = center.saturating_sub(half_window);
    let end = (center + half_window).min(samples.len());
    let window = &samples[start..end];

    if window.len() < 2 {
        return 0.0;
    }

    let crossings = window
        .windows(2)
        .filter(|w| (w[0] >= 0.0) != (w[1] >= 0.0))
        .count();

    crossings as f32 / (window.len() - 1) as f32
}

/// Finds the optimal threshold to split values into two classes by finding
/// the largest gap in sorted values.
fn split_threshold(values: &[f32]) -> f32 {
    if values.len() < 2 {
        return values.first().cloned().unwrap_or(0.0);
    }

    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let mut best_gap = 0.0_f32;
    let mut best_threshold = sorted[0];

    for w in sorted.windows(2) {
        let gap = w[1] - w[0];
        if gap > best_gap {
            best_gap = gap;
            best_threshold = (w[0] + w[1]) / 2.0;
        }
    }

    best_threshold
}

impl AccentClassifier for ZcrClassifier {
    fn classify(&self, onsets: &[Onset], samples: &[f32], sample_rate: u32) -> Vec<bool> {
        if onsets.is_empty() {
            return vec![];
        }

        let half_window = ((ZCR_WINDOW_MS / 1000.0) * sample_rate as f64 / 2.0) as usize;

        let zcr_values: Vec<f32> = onsets
            .iter()
            .map(|onset| {
                let center = (onset.time_secs * sample_rate as f64) as usize;
                compute_zcr(samples, center, half_window)
            })
            .collect();

        let threshold = split_threshold(&zcr_values);

        // Determine which class (above or below threshold) is the minority —
        // the minority class is the accented one (downbeats are less frequent).
        let above_count = zcr_values.iter().filter(|&&v| v >= threshold).count();
        let below_count = zcr_values.len() - above_count;
        let accent_is_above = above_count <= below_count;

        zcr_values
            .iter()
            .map(|&v| {
                if accent_is_above {
                    v >= threshold
                } else {
                    v < threshold
                }
            })
            .collect()
    }
}

impl AccentClassifier for AmplitudeClassifier {
    fn classify(&self, onsets: &[Onset], _samples: &[f32], _sample_rate: u32) -> Vec<bool> {
        if onsets.is_empty() {
            return vec![];
        }

        let amplitudes: Vec<f32> = onsets.iter().map(|o| o.amplitude).collect();
        let threshold = split_threshold(&amplitudes);

        let above_count = amplitudes.iter().filter(|&&v| v >= threshold).count();
        let below_count = amplitudes.len() - above_count;
        let accent_is_above = above_count <= below_count;

        amplitudes
            .iter()
            .map(|&v| {
                if accent_is_above {
                    v >= threshold
                } else {
                    v < threshold
                }
            })
            .collect()
    }
}

// ── Main entry point ─────────────────────────────────────────────────────────

/// Analyzes a click track audio file and returns a beat grid.
///
/// The `classifier` determines how accented (downbeat) clicks are distinguished
/// from normal beats. Pass `&ZcrClassifier` for timbral classification or
/// `&AmplitudeClassifier` for volume-based classification.
pub fn analyze_click_track(
    file: &Path,
    file_channel: u16,
    classifier: &dyn AccentClassifier,
) -> Option<BeatGrid> {
    let (samples, sample_rate) = read_mono_samples(file, file_channel)?;
    let onsets = detect_onsets(&samples, sample_rate);

    if onsets.len() < MIN_ONSETS_FOR_ANALYSIS {
        warn!(
            "Click track {} has only {} onsets, need at least {}",
            file.display(),
            onsets.len(),
            MIN_ONSETS_FOR_ANALYSIS
        );
        return None;
    }

    let accents = classifier.classify(&onsets, &samples, sample_rate);

    let beats: Vec<f64> = onsets.iter().map(|o| o.time_secs).collect();
    let measure_starts: Vec<usize> = accents
        .iter()
        .enumerate()
        .filter(|(_, &a)| a)
        .map(|(i, _)| i)
        .collect();

    let mut grid = BeatGrid {
        beats,
        measure_starts,
    };

    // Refine beat positions to remove onset detection jitter
    refine_beat_grid(&mut grid);

    Some(grid)
}

/// Convenience wrapper that uses `ZcrClassifier` as the default accent classifier.
pub fn analyze_click_track_default(file: &Path, file_channel: u16) -> Option<BeatGrid> {
    analyze_click_track(file, file_channel, &ZcrClassifier)
}

/// Refine beat positions by snapping beats in stable tempo sections to an
/// ideal grid. This removes onset detection jitter (typically ±5-10ms) which
/// can cause BPM miscalculations at section boundaries.
///
/// Only beats within stable sections (where consecutive intervals are within
/// `band_bpm` of each other) are adjusted. Beats in ramps/transitions are
/// left at their detected positions.
pub fn refine_beat_grid(grid: &mut BeatGrid) {
    if grid.beats.len() < 2 {
        return;
    }

    // Configuration
    let band_bpm = 4.0; // max BPM deviation within a stable section
    let min_stable = 8; // minimum beats to form a stable section
    let max_snap_secs = 0.015; // don't move a beat more than 15ms

    // Compute instantaneous BPM at each beat
    let mut bpms: Vec<f64> = grid
        .beats
        .windows(2)
        .map(|w| {
            let interval = w[1] - w[0];
            if interval > 0.0 {
                60.0 / interval
            } else {
                0.0
            }
        })
        .collect();
    if let Some(&last) = bpms.last() {
        bpms.push(last);
    }

    // Find stable sections using fixed-mean approach
    let mut sections: Vec<(usize, usize, f64)> = Vec::new(); // (start, end_exclusive, mean_bpm)
    let mut pos = 0;
    while pos < bpms.len() {
        let seed_end = (pos + min_stable).min(bpms.len());
        if seed_end > bpms.len() || seed_end - pos < min_stable {
            break;
        }
        let seed_bpms = &bpms[pos..seed_end];
        let seed_mean = seed_bpms.iter().sum::<f64>() / seed_bpms.len() as f64;
        let max_dev = seed_bpms
            .iter()
            .map(|b| (b - seed_mean).abs())
            .fold(0.0f64, f64::max);
        if max_dev > band_bpm {
            pos += 1;
            continue;
        }

        // Extend using fixed mean
        let mut end = seed_end;
        while end < bpms.len() {
            if (bpms[end] - seed_mean).abs() > band_bpm {
                break;
            }
            end += 1;
        }

        if end - pos >= min_stable {
            // Recompute mean from the full section for precision
            let mean = bpms[pos..end].iter().sum::<f64>() / (end - pos) as f64;
            sections.push((pos, end, mean));
            pos = end;
        } else {
            pos += 1;
        }
    }

    // Snap beats within each stable section to the ideal grid.
    // Anchor from a few beats into the section (not the very first beat,
    // which may be at the boundary and jittery). Snap both forward and
    // backward from the anchor.
    for &(start, end, mean_bpm) in &sections {
        let ideal_interval = 60.0 / mean_bpm;
        let section_len = end - start;

        // Anchor a few beats in (or at the midpoint for short sections)
        let anchor_offset = min_stable.min(section_len / 2).max(1);
        let anchor_idx = start + anchor_offset;
        let anchor_time = grid.beats[anchor_idx];

        // Snap forward from anchor
        for i in (anchor_idx + 1)..end.min(grid.beats.len()) {
            let beats_from_anchor = (i - anchor_idx) as f64;
            let ideal_time = anchor_time + beats_from_anchor * ideal_interval;
            if (ideal_time - grid.beats[i]).abs() <= max_snap_secs {
                grid.beats[i] = ideal_time;
            }
        }

        // Snap backward from anchor (including the first beat)
        for i in (start..anchor_idx).rev() {
            let beats_from_anchor = (anchor_idx - i) as f64;
            let ideal_time = anchor_time - beats_from_anchor * ideal_interval;
            if (ideal_time - grid.beats[i]).abs() <= max_snap_secs {
                grid.beats[i] = ideal_time;
            }
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_SAMPLE_RATE: u32 = 44100;

    /// Generates a mono click track with impulses at the given BPM.
    /// `accented_every` controls accent pattern (e.g., 4 = accent every 4 beats).
    /// Accented clicks use a higher frequency burst, normal clicks use lower.
    fn generate_click_track(
        bpm: f64,
        duration_secs: f64,
        accented_every: u32,
        sample_rate: u32,
    ) -> Vec<f32> {
        let total_samples = (duration_secs * sample_rate as f64) as usize;
        let mut samples = vec![0.0_f32; total_samples];
        let beat_interval = 60.0 / bpm;
        let click_duration_samples = (0.005 * sample_rate as f64) as usize; // 5ms click

        let mut beat = 0u32;
        let mut t = 0.0;

        while t < duration_secs {
            let sample_idx = (t * sample_rate as f64) as usize;
            let is_accented = beat.is_multiple_of(accented_every);

            // Accented: ~4kHz burst. Normal: ~1kHz burst.
            let freq = if is_accented { 4000.0 } else { 1000.0 };
            let amplitude = if is_accented { 0.9 } else { 0.7 };

            for i in 0..click_duration_samples {
                let idx = sample_idx + i;
                if idx < total_samples {
                    let phase = 2.0 * std::f64::consts::PI * freq * i as f64 / sample_rate as f64;
                    samples[idx] = (phase.sin() * amplitude) as f32;
                }
            }

            beat += 1;
            t += beat_interval;
        }

        samples
    }

    #[test]
    fn detect_onsets_silence() {
        let samples = vec![0.0_f32; TEST_SAMPLE_RATE as usize * 2];
        let onsets = detect_onsets(&samples, TEST_SAMPLE_RATE);
        assert!(onsets.is_empty());
    }

    #[test]
    fn detect_onsets_regular() {
        let samples = generate_click_track(120.0, 5.0, 4, TEST_SAMPLE_RATE);
        let onsets = detect_onsets(&samples, TEST_SAMPLE_RATE);

        // 120 BPM for 5 seconds = 10 beats.
        assert!(
            onsets.len() >= 8 && onsets.len() <= 12,
            "Expected ~10 onsets, got {}",
            onsets.len()
        );

        // Check timing: inter-onset intervals should be ~0.5s (120 BPM).
        for w in onsets.windows(2) {
            let ioi = w[1].time_secs - w[0].time_secs;
            assert!(
                (ioi - 0.5).abs() < 0.05,
                "IOI should be ~0.5s, got {:.3}",
                ioi
            );
        }
    }

    #[test]
    fn detect_onsets_empty_input() {
        let onsets = detect_onsets(&[], TEST_SAMPLE_RATE);
        assert!(onsets.is_empty());
    }

    #[test]
    fn detect_onsets_short_input() {
        let samples = vec![0.0_f32; 100]; // shorter than WINDOW_SIZE
        let onsets = detect_onsets(&samples, TEST_SAMPLE_RATE);
        assert!(onsets.is_empty());
    }

    #[test]
    fn zcr_classifier_separates_frequencies() {
        let samples = generate_click_track(120.0, 5.0, 4, TEST_SAMPLE_RATE);
        let onsets = detect_onsets(&samples, TEST_SAMPLE_RATE);

        let classifier = ZcrClassifier;
        let accents = classifier.classify(&onsets, &samples, TEST_SAMPLE_RATE);

        assert_eq!(accents.len(), onsets.len());

        let accent_count = accents.iter().filter(|&&a| a).count();
        assert!(
            accent_count > 0 && accent_count < accents.len(),
            "Should have mix of accented ({}) and normal ({})",
            accent_count,
            accents.len() - accent_count
        );
    }

    #[test]
    fn amplitude_classifier_separates_volumes() {
        let samples = generate_click_track(120.0, 5.0, 4, TEST_SAMPLE_RATE);
        let onsets = detect_onsets(&samples, TEST_SAMPLE_RATE);

        let classifier = AmplitudeClassifier;
        let accents = classifier.classify(&onsets, &samples, TEST_SAMPLE_RATE);

        assert_eq!(accents.len(), onsets.len());

        let accent_count = accents.iter().filter(|&&a| a).count();
        assert!(
            accent_count > 0 && accent_count < accents.len(),
            "Should have mix of accented ({}) and normal ({})",
            accent_count,
            accents.len() - accent_count
        );
    }

    #[test]
    fn custom_classifier_works() {
        struct AlternatingClassifier;
        impl AccentClassifier for AlternatingClassifier {
            fn classify(&self, onsets: &[Onset], _samples: &[f32], _sample_rate: u32) -> Vec<bool> {
                onsets.iter().enumerate().map(|(i, _)| i % 3 == 0).collect()
            }
        }

        let onsets: Vec<Onset> = (0..9)
            .map(|i| Onset {
                time_secs: i as f64 * 0.5,
                amplitude: 0.8,
            })
            .collect();

        let classifier = AlternatingClassifier;
        let accents = classifier.classify(&onsets, &[], 44100);

        assert_eq!(
            accents,
            vec![true, false, false, true, false, false, true, false, false]
        );
    }

    #[test]
    fn beat_grid_4_4() {
        let samples = generate_click_track(120.0, 10.0, 4, TEST_SAMPLE_RATE);
        let onsets = detect_onsets(&samples, TEST_SAMPLE_RATE);

        let classifier = ZcrClassifier;
        let accents = classifier.classify(&onsets, &samples, TEST_SAMPLE_RATE);

        let beats: Vec<f64> = onsets.iter().map(|o| o.time_secs).collect();
        let measure_starts: Vec<usize> = accents
            .iter()
            .enumerate()
            .filter(|(_, &a)| a)
            .map(|(i, _)| i)
            .collect();

        let grid = BeatGrid {
            beats,
            measure_starts,
        };

        // Should have ~20 beats (120 BPM * 10s / 60).
        assert!(
            grid.beat_count() >= 18 && grid.beat_count() <= 22,
            "Expected ~20 beats, got {}",
            grid.beat_count()
        );

        // Measure starts should be every 4 beats.
        assert!(
            grid.measure_starts.len() >= 4,
            "Expected at least 4 measure starts, got {}",
            grid.measure_starts.len()
        );

        // Beats between consecutive measure starts should be ~4.
        for w in grid.measure_starts.windows(2) {
            let gap = w[1] - w[0];
            assert_eq!(gap, 4, "Expected 4 beats per measure, got {}", gap);
        }
    }

    #[test]
    fn beat_grid_3_pattern() {
        let samples = generate_click_track(120.0, 10.0, 3, TEST_SAMPLE_RATE);
        let onsets = detect_onsets(&samples, TEST_SAMPLE_RATE);

        let classifier = ZcrClassifier;
        let accents = classifier.classify(&onsets, &samples, TEST_SAMPLE_RATE);

        let measure_starts: Vec<usize> = accents
            .iter()
            .enumerate()
            .filter(|(_, &a)| a)
            .map(|(i, _)| i)
            .collect();

        // Beats between consecutive measure starts should be ~3.
        for w in measure_starts.windows(2) {
            let gap = w[1] - w[0];
            assert_eq!(gap, 3, "Expected 3 beats per measure, got {}", gap);
        }
    }

    #[test]
    fn beat_grid_measure_time_range() {
        let grid = BeatGrid {
            beats: vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
            measure_starts: vec![0, 4],
        };

        assert_eq!(grid.measure_count(), 1);

        let (start, end) = grid.measure_time_range(0).unwrap();
        assert!((start - 0.0).abs() < f64::EPSILON);
        assert!((end - 2.0).abs() < f64::EPSILON);

        // Out of range.
        assert!(grid.measure_time_range(2).is_none());
    }

    #[test]
    fn beat_grid_beat_time() {
        let grid = BeatGrid {
            beats: vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
            measure_starts: vec![0, 4],
        };

        // Whole beats land on grid entries.
        assert_eq!(grid.beat_time(0, 1.0), Some(0.0));
        assert_eq!(grid.beat_time(0, 3.0), Some(1.0));
        assert_eq!(grid.beat_time(1, 1.0), Some(2.0));
        // Fractional beats interpolate linearly.
        assert_eq!(grid.beat_time(0, 4.5), Some(1.75));
        // The final grid beat is addressable; past it is not.
        assert_eq!(grid.beat_time(1, 4.0), Some(3.5));
        assert!(grid.beat_time(1, 4.5).is_none());
        // A beat stays inside its measure: beat 5 of a four-beat measure is
        // the next measure's downbeat, not a position in this one.
        assert!(grid.beat_time(0, 5.0).is_none());
        assert!(grid.beat_time(0, 6.5).is_none());
        // Invalid inputs.
        assert!(grid.beat_time(0, 0.5).is_none());
        assert!(grid.beat_time(2, 1.0).is_none());
        assert!(grid.beat_time(0, f64::NAN).is_none());
    }

    #[test]
    fn beat_grid_beats_in_measure() {
        let grid = BeatGrid {
            beats: vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0],
            measure_starts: vec![0, 4],
        };

        assert_eq!(grid.beats_in_measure(0), Some(4));
        // The last measure runs to the end of the grid, however short.
        assert_eq!(grid.beats_in_measure(1), Some(3));
        assert_eq!(grid.beats_in_measure(2), None);
    }

    #[test]
    fn beat_grid_measure_count() {
        let grid = BeatGrid {
            beats: vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5],
            measure_starts: vec![0, 3, 6],
        };

        assert_eq!(grid.measure_count(), 2);
        assert_eq!(grid.beat_count(), 8);
    }

    #[test]
    fn full_analysis_end_to_end() {
        let samples = generate_click_track(120.0, 10.0, 4, TEST_SAMPLE_RATE);

        let dir = tempfile::tempdir().unwrap();
        let wav_path = dir.path().join("click.wav");

        let samples_i32: Vec<i32> = samples
            .iter()
            .map(|&s| (s * i32::MAX as f32) as i32)
            .collect();
        crate::testutil::write_wav(wav_path.clone(), vec![samples_i32], TEST_SAMPLE_RATE).unwrap();

        let result = analyze_click_track_default(&wav_path, 1);
        assert!(result.is_some(), "Analysis should succeed");

        let grid = result.unwrap();
        assert!(grid.beat_count() >= 18, "Should have at least 18 beats");
        assert!(
            grid.measure_starts.len() >= 4,
            "Should have at least 4 measure starts"
        );

        // Verify measure boundaries are every 4 beats.
        for w in grid.measure_starts.windows(2) {
            assert_eq!(w[1] - w[0], 4);
        }
    }

    #[test]
    fn analysis_returns_none_for_silence() {
        let dir = tempfile::tempdir().unwrap();
        let wav_path = dir.path().join("silence.wav");

        let samples = vec![0_i32; TEST_SAMPLE_RATE as usize * 5];
        crate::testutil::write_wav(wav_path.clone(), vec![samples], TEST_SAMPLE_RATE).unwrap();

        let result = analyze_click_track_default(&wav_path, 1);
        assert!(
            result.is_none(),
            "Should return None for silent click track"
        );
    }

    #[test]
    fn split_threshold_separates_bimodal() {
        let values: Vec<f32> = vec![0.1, 0.12, 0.11, 0.09, 0.8, 0.85, 0.82, 0.78];
        let threshold = split_threshold(&values);

        for &v in &[0.09_f32, 0.1, 0.11, 0.12] {
            assert!(
                v < threshold,
                "Low value {} should be below threshold {}",
                v,
                threshold
            );
        }
        for &v in &[0.78_f32, 0.8, 0.82, 0.85] {
            assert!(
                v >= threshold,
                "High value {} should be at/above threshold {}",
                v,
                threshold
            );
        }
    }

    #[test]
    fn split_threshold_empty() {
        assert_eq!(split_threshold(&[]), 0.0);
    }

    #[test]
    fn split_threshold_uniform() {
        let values = vec![0.5, 0.5, 0.5];
        let threshold = split_threshold(&values);
        assert!((threshold - 0.5).abs() < 0.1);
    }

    mod from_tempo_map {
        use super::super::BeatGrid;
        use crate::tempo::{
            TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
            TransitionCurve,
        };
        use std::time::Duration;

        fn assert_close(actual: f64, expected: f64, context: &str) {
            assert!(
                (actual - expected).abs() < 1e-6,
                "{context}: expected {expected}, got {actual}"
            );
        }

        #[test]
        fn simple_four_four() {
            // 120 BPM 4/4 for 4 seconds = 8 beats, 2 measures.
            let map = TempoMap::new(Duration::ZERO, 120.0, TimeSignature::new(4, 4), Vec::new());
            let grid = BeatGrid::from_tempo_map(&map, Duration::from_secs(4));
            assert_eq!(grid.beats.len(), 8);
            assert_eq!(grid.measure_starts, vec![0, 4]);
            for (i, beat) in grid.beats.iter().enumerate() {
                assert_close(*beat, i as f64 * 0.5, "beat time");
            }
        }

        #[test]
        fn start_offset_shifts_grid() {
            let map = TempoMap::new(
                Duration::from_secs_f64(0.35),
                120.0,
                TimeSignature::new(4, 4),
                Vec::new(),
            );
            let grid = BeatGrid::from_tempo_map(&map, Duration::from_secs(2));
            assert_close(grid.beats[0], 0.35, "first beat at start offset");
            assert_close(grid.beats[1], 0.85, "second beat");
        }

        #[test]
        fn seven_eight_emits_denominator_notes() {
            // 7/8 at quarter = 120 -> eighth = 0.25s. 7 beats per measure,
            // measures are 1.75s long.
            let map = TempoMap::new(Duration::ZERO, 120.0, TimeSignature::new(7, 8), Vec::new());
            let grid = BeatGrid::from_tempo_map(&map, Duration::from_secs_f64(3.5));
            assert_eq!(grid.beats.len(), 14);
            assert_eq!(grid.measure_starts, vec![0, 7]);
            for (i, beat) in grid.beats.iter().enumerate() {
                assert_close(*beat, i as f64 * 0.25, "eighth-note beat time");
            }
        }

        #[test]
        fn snap_tempo_change() {
            // 4/4: 2 measures at 120 (2s each -> beats every 0.5s), then 60
            // (beats every 1s) from measure 3.
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
            let grid = BeatGrid::from_tempo_map(&map, Duration::from_secs(8));
            // Measures 1-2: 8 beats over 0..4s; measure 3: beats at 4,5,6,7.
            assert_eq!(grid.beats.len(), 12);
            assert_eq!(grid.measure_starts, vec![0, 4, 8]);
            assert_close(grid.beats[8], 4.0, "measure 3 downbeat");
            assert_close(grid.beats[9], 5.0, "measure 3 beat 2");
        }

        #[test]
        fn time_signature_change_alters_beats_per_measure() {
            // Measure 1 in 4/4, measures 2+ in 6/8 (six eighth notes each).
            let map = TempoMap::new(
                Duration::ZERO,
                120.0,
                TimeSignature::new(4, 4),
                vec![TempoChange {
                    position: TempoChangePosition::MeasureBeat(2, 1.0),
                    original_measure_beat: None,
                    bpm: None,
                    time_signature: Some(TimeSignature::new(6, 8)),
                    transition: TempoTransition::Snap,
                }],
            );
            // Measure 1: 2s; 6/8 measures at 120 quarter-bpm: 1.5s each.
            let grid = BeatGrid::from_tempo_map(&map, Duration::from_secs_f64(3.5));
            assert_eq!(grid.measure_starts, vec![0, 4]);
            // Measure 2 has 6 beats spaced 0.25s starting at 2.0.
            assert_eq!(grid.beats.len(), 4 + 6);
            for k in 0..6 {
                assert_close(grid.beats[4 + k], 2.0 + k as f64 * 0.25, "6/8 eighth note");
            }
        }

        #[test]
        fn gradual_transition_tightens_spacing() {
            // Ramp from 120 to 240 over measure 2; beat spacing must strictly
            // decrease through the transition and settle at 0.25s.
            let map = TempoMap::new(
                Duration::ZERO,
                120.0,
                TimeSignature::new(4, 4),
                vec![TempoChange {
                    position: TempoChangePosition::MeasureBeat(2, 1.0),
                    original_measure_beat: None,
                    bpm: Some(240.0),
                    time_signature: None,
                    transition: TempoTransition::Measures(1.0, TransitionCurve::Linear),
                }],
            );
            let grid = BeatGrid::from_tempo_map(&map, Duration::from_secs(6));
            let spacing: Vec<f64> = grid.beats.windows(2).map(|w| w[1] - w[0]).collect();
            // Measure 1 is steady at 120.
            assert_close(spacing[0], 0.5, "pre-transition spacing");
            assert_close(spacing[2], 0.5, "pre-transition spacing");
            // Through measure 2 the spacing strictly decreases.
            for w in spacing[3..7].windows(2) {
                assert!(
                    w[1] < w[0] - 1e-9,
                    "transition spacing should decrease, got {:?}",
                    &spacing[3..7]
                );
            }
            // After the transition we are at 240 BPM.
            let last = *spacing.last().unwrap();
            assert_close(last, 0.25, "post-transition spacing");
        }

        #[test]
        fn truncates_at_duration() {
            let map = TempoMap::new(Duration::ZERO, 120.0, TimeSignature::new(4, 4), Vec::new());
            // 1.2s cuts off mid-measure: beats at 0.0, 0.5, 1.0.
            let grid = BeatGrid::from_tempo_map(&map, Duration::from_secs_f64(1.2));
            assert_eq!(grid.beats.len(), 3);
            assert_eq!(grid.measure_starts, vec![0]);
        }

        #[test]
        fn zero_duration_is_empty() {
            let map = TempoMap::new(Duration::ZERO, 120.0, TimeSignature::new(4, 4), Vec::new());
            let grid = BeatGrid::from_tempo_map(&map, Duration::ZERO);
            assert!(grid.beats.is_empty());
            assert!(grid.measure_starts.is_empty());
        }
    }
}
