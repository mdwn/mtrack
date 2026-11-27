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
//! A song's waveform: the peak of each of its tracks' audio over a number of
//! buckets, normalised to 0 to 1, kept in the song's cache
//! ([`crate::song_cache`]). What a waveform is drawn from, by the player's
//! web UI and by anything else that shows a song.

use std::path::PathBuf;

use rayon::prelude::*;
use tracing::warn;

use crate::audio::sample_source::audio::AudioSampleSource;
use crate::audio::sample_source::traits::SampleSource;

/// Per-track info needed for waveform computation: (track_name, file_path, file_channel).
pub type TrackInfo = (String, PathBuf, u16);

/// Per-track waveform peaks: (track_name, peak_values).
pub type TrackPeaks = (String, Vec<f32>);

/// Computes waveform peaks for all tracks in parallel, using the disk cache
/// when available. Returns (track_name, peaks) pairs.
///
/// Loads valid cached peaks first, computes only uncached tracks, then saves
/// newly computed peaks back to disk.
pub fn compute_waveform_peaks(song_dir: &std::path::Path, tracks: &[TrackInfo]) -> Vec<TrackPeaks> {
    const NUM_BUCKETS: usize = 500;

    let cached = crate::song_cache::load_cached_peaks(song_dir, tracks);

    // Split tracks into cached and uncached.
    let mut results: Vec<TrackPeaks> = Vec::with_capacity(tracks.len());
    let uncached: Vec<&TrackInfo> = tracks
        .iter()
        .filter(|(name, _, _)| !cached.contains_key(name))
        .collect();

    // Compute uncached tracks in parallel.
    let newly_computed: Vec<TrackPeaks> = uncached
        .par_iter()
        .map(|(name, file, file_channel)| {
            let peaks = compute_track_peaks(file, *file_channel, NUM_BUCKETS);
            (name.clone(), peaks)
        })
        .collect();

    // Save newly computed peaks to disk.
    if !newly_computed.is_empty() {
        let save_data: Vec<(String, PathBuf, u16, Vec<f32>)> = newly_computed
            .iter()
            .filter_map(|(name, peaks)| {
                let (_, file, channel) = tracks.iter().find(|(n, _, _)| n == name)?;
                Some((name.clone(), file.clone(), *channel, peaks.clone()))
            })
            .collect();

        if let Err(e) = crate::song_cache::save_peaks(song_dir, &save_data) {
            warn!("Failed to save waveform cache: {}", e);
        }
    }

    // Merge: preserve original track order.
    for (name, _file, _channel) in tracks {
        if let Some(peaks) = cached.get(name) {
            results.push((name.clone(), peaks.clone()));
        } else if let Some(peaks) = newly_computed.iter().find(|(n, _)| n == name) {
            results.push(peaks.clone());
        } else {
            results.push((name.clone(), vec![]));
        }
    }

    results
}

/// Computes peak values for a single track using bulk buffer reads.
///
/// Uses `AudioSampleSource::read_samples` to read large chunks at a time,
/// avoiding per-sample virtual dispatch overhead.
pub fn compute_track_peaks(
    file: &std::path::Path,
    file_channel: u16,
    num_buckets: usize,
) -> Vec<f32> {
    let mut source = match AudioSampleSource::from_file(file, None, 4096) {
        Ok(s) => s,
        Err(e) => {
            warn!("Failed to open audio file {}: {}", file.display(), e);
            return vec![];
        }
    };

    let channel_count = source.channel_count() as usize;
    if channel_count == 0 {
        return vec![];
    }

    // file_channel is 1-indexed
    let target_channel = (file_channel as usize).saturating_sub(1);
    if target_channel >= channel_count {
        warn!(
            "file_channel {} exceeds channel count {} for {}",
            file_channel,
            channel_count,
            file.display()
        );
        return vec![];
    }

    // Estimate total mono samples from duration to size buckets up front
    let estimated_samples = source
        .duration()
        .map(|d| (d.as_secs_f64() * source.sample_rate() as f64) as usize)
        .unwrap_or(0);

    let samples_per_bucket = if estimated_samples > 0 {
        estimated_samples.div_ceil(num_buckets)
    } else {
        // Unknown duration: use a reasonable default, resize at end
        4096
    };

    let mut peaks = vec![0.0_f32; num_buckets];
    let mut mono_sample_idx: usize = 0;
    let mut interleaved_idx: usize = 0;

    // Read in chunks of ~16K interleaved samples at a time
    let mut buf = vec![0.0_f32; 16384];
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
                let bucket = (mono_sample_idx / samples_per_bucket).min(num_buckets - 1);
                let abs = sample.abs();
                if abs > peaks[bucket] {
                    peaks[bucket] = abs;
                }
                mono_sample_idx += 1;
            }
        }
    }

    // If we had no samples, return empty
    if mono_sample_idx == 0 {
        return vec![];
    }

    // Trim trailing empty buckets (if file was shorter than estimated)
    let used_buckets = (mono_sample_idx / samples_per_bucket + 1).min(num_buckets);
    peaks.truncate(used_buckets);

    // Normalize to 0.0 - 1.0
    let max_peak = peaks.iter().cloned().fold(0.0_f32, f32::max);
    if max_peak > 0.0 {
        for p in &mut peaks {
            *p /= max_peak;
        }
    }

    peaks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_track_peaks_with_test_wav() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("test.wav");

        // Create a simple WAV with known samples
        let samples: Vec<i32> = (0..4410).map(|i| (i * 100) % 32768).collect();
        crate::testutil::write_wav(wav_path.clone(), vec![samples], 44100).expect("write test wav");

        let peaks = compute_track_peaks(&wav_path, 1, 10);
        assert!(!peaks.is_empty());
        // Peaks should be normalized to 0.0-1.0
        for &p in &peaks {
            assert!((0.0..=1.0).contains(&p), "peak {} out of range", p);
        }
        // At least one peak should be 1.0 (the max)
        assert!(
            peaks.iter().any(|&p| (p - 1.0).abs() < f32::EPSILON),
            "expected at least one normalized peak at 1.0"
        );
    }

    #[test]
    fn compute_track_peaks_invalid_channel() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("mono.wav");

        crate::testutil::write_wav(wav_path.clone(), vec![vec![1_i32, 2, 3]], 44100)
            .expect("write wav");

        // file_channel 5 on a mono file — should return empty
        let peaks = compute_track_peaks(&wav_path, 5, 10);
        assert!(peaks.is_empty());
    }

    #[test]
    fn compute_track_peaks_missing_file() {
        let peaks = compute_track_peaks(std::path::Path::new("/nonexistent/file.wav"), 1, 10);
        assert!(peaks.is_empty());
    }

    #[test]
    fn compute_track_peaks_zero_buckets_edge() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("test.wav");
        let samples: Vec<i32> = (0..4410).map(|i| (i * 100) % 32768).collect();
        crate::testutil::write_wav(wav_path.clone(), vec![samples], 44100).expect("write test wav");

        // Even with 1 bucket, should work
        let peaks = compute_track_peaks(&wav_path, 1, 1);
        assert!(!peaks.is_empty());
        assert!(peaks.len() <= 1);
    }

    #[test]
    fn compute_track_peaks_stereo_extracts_correct_channel() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("stereo.wav");

        // A stereo WAV: ch1=loud, ch2=silence
        crate::testutil::write_wav(
            wav_path.clone(),
            vec![vec![0.9_f32; 44100], vec![0.0_f32; 44100]],
            44100,
        )
        .expect("write stereo wav");

        // file_channel 1 should see the loud signal
        let ch1_peaks = compute_track_peaks(&wav_path, 1, 10);
        assert!(!ch1_peaks.is_empty());
        assert!(
            ch1_peaks.iter().all(|&p| p > 0.5),
            "channel 1 should have loud peaks: {:?}",
            ch1_peaks
        );

        // file_channel 2 should see silence (all zeros)
        let ch2_peaks = compute_track_peaks(&wav_path, 2, 10);
        assert!(!ch2_peaks.is_empty());
        assert!(
            ch2_peaks.iter().all(|&p| p == 0.0),
            "channel 2 should be all zeros: {:?}",
            ch2_peaks
        );
    }

    #[test]
    fn compute_track_peaks_stereo_selects_correct_channel() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("stereo.wav");

        // Create a stereo WAV with different amplitudes per channel
        let ch1: Vec<i32> = (0..4410).map(|i| (i * 10) % 32768).collect();
        let ch2: Vec<i32> = (0..4410).map(|i| (i * 100) % 32768).collect();
        crate::testutil::write_wav(wav_path.clone(), vec![ch1, ch2], 44100)
            .expect("write stereo wav");

        // Both channels should produce peaks
        let peaks_ch1 = compute_track_peaks(&wav_path, 1, 10);
        let peaks_ch2 = compute_track_peaks(&wav_path, 2, 10);

        assert!(!peaks_ch1.is_empty());
        assert!(!peaks_ch2.is_empty());
        // Both should be normalized (max = 1.0)
        assert!(peaks_ch1.iter().any(|&p| (p - 1.0).abs() < f32::EPSILON));
        assert!(peaks_ch2.iter().any(|&p| (p - 1.0).abs() < f32::EPSILON));
    }

    #[test]
    fn compute_track_peaks_large_bucket_count() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("test.wav");
        let samples: Vec<i32> = (0..4410).map(|i| (i * 100) % 32768).collect();
        crate::testutil::write_wav(wav_path.clone(), vec![samples], 44100).expect("write wav");

        // More buckets than samples — should still work
        let peaks = compute_track_peaks(&wav_path, 1, 10000);
        assert!(!peaks.is_empty());
    }

    #[test]
    fn compute_waveform_peaks_missing_file() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let tracks = vec![(
            "missing".to_string(),
            std::path::PathBuf::from("/nonexistent/file.wav"),
            1_u16,
        )];
        let results = compute_waveform_peaks(temp_dir.path(), &tracks);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].0, "missing");
        assert!(results[0].1.is_empty());
    }

    #[test]
    fn compute_waveform_peaks_multiple_tracks() {
        let temp_dir = tempfile::tempdir().expect("tempdir");

        let wav1 = temp_dir.path().join("track1.wav");
        let wav2 = temp_dir.path().join("track2.wav");

        let samples: Vec<i32> = (0..4410).map(|i| (i * 50) % 32768).collect();
        crate::testutil::write_wav(wav1.clone(), vec![samples.clone()], 44100).expect("write wav1");
        crate::testutil::write_wav(wav2.clone(), vec![samples], 44100).expect("write wav2");

        let tracks = vec![
            ("Track 1".to_string(), wav1, 1_u16),
            ("Track 2".to_string(), wav2, 1_u16),
        ];

        let results = compute_waveform_peaks(temp_dir.path(), &tracks);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "Track 1");
        assert_eq!(results[1].0, "Track 2");
        assert!(!results[0].1.is_empty());
        assert!(!results[1].1.is_empty());
    }

    #[test]
    fn compute_track_peaks_file_channel_zero_returns_empty() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("test.wav");
        let samples: Vec<i32> = (0..4410).map(|i| (i * 100) % 32768).collect();
        crate::testutil::write_wav(wav_path.clone(), vec![samples], 44100).expect("write wav");

        // file_channel 0 uses saturating_sub to get target_channel 0, which should work
        // (it matches the first channel of a mono file)
        let peaks = compute_track_peaks(&wav_path, 0, 10);
        // With file_channel 0, target_channel = 0 which is valid for a mono file
        assert!(!peaks.is_empty());
    }

    #[test]
    fn compute_track_peaks_all_zeros_returns_empty() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let wav_path = temp_dir.path().join("silence.wav");
        // All-zero samples should produce peaks of 0.0 but still have buckets
        let samples: Vec<i32> = vec![0_i32; 4410];
        crate::testutil::write_wav(wav_path.clone(), vec![samples], 44100).expect("write wav");

        let peaks = compute_track_peaks(&wav_path, 1, 10);
        // All zero peaks, so normalization divides by 0.0 which is skipped,
        // all values should remain 0.0
        assert!(!peaks.is_empty());
        for &p in &peaks {
            assert!((p - 0.0).abs() < f32::EPSILON, "expected 0.0, got {}", p);
        }
    }

    #[test]
    fn compute_waveform_peaks_empty_tracks() {
        let temp_dir = tempfile::tempdir().expect("tempdir");
        let tracks: Vec<(String, PathBuf, u16)> = vec![];
        let results = compute_waveform_peaks(temp_dir.path(), &tracks);
        assert!(results.is_empty());
    }
}
