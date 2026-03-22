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

    Some(BeatGrid {
        beats,
        measure_starts,
    })
}

/// Convenience wrapper that uses `ZcrClassifier` as the default accent classifier.
pub fn analyze_click_track_default(file: &Path, file_channel: u16) -> Option<BeatGrid> {
    analyze_click_track(file, file_channel, &ZcrClassifier)
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
            let is_accented = beat % accented_every == 0;

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
}
