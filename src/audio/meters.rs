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
//! Lock-free per-output-channel level metering.
//!
//! The audio callback ingests each freshly mixed buffer; a UI-rate reader
//! takes snapshots. Peaks are held until read (reset-on-snapshot), so a
//! transient between two polls is never missed. RMS is the mean square of
//! the most recent buffer only; ballistics/smoothing belong to the reader.
//!
//! Values are stored as `f32` bit patterns in atomics so the callback can
//! write them lock-free. Peak updates rely on non-negative IEEE floats
//! comparing correctly as unsigned integers, which makes `fetch_max` on the
//! bit pattern equivalent to a float max.
use std::sync::atomic::{AtomicU32, Ordering};

/// Maximum number of metered output channels. Channels beyond this are not
/// metered (the mixer's own scratch sizing makes the same assumption).
pub const MAX_METER_CHANNELS: usize = 64;

/// Shared per-output-channel meter state.
///
/// Created by the mixer at construction and shared (via `Arc`) with readers.
/// The writer side (`ingest`) is called from the audio callback and performs
/// one pass over the buffer plus one atomic op per channel and meter kind.
pub struct OutputMeters {
    /// Per-channel absolute peak since the last snapshot (f32 bits).
    peak_bits: Vec<AtomicU32>,
    /// Per-channel mean square of the most recent ingested buffer (f32 bits).
    mean_square_bits: Vec<AtomicU32>,
    /// Per-channel cumulative count of samples beyond full scale (|s| > 1.0).
    clip_counts: Vec<AtomicU32>,
}

/// A point-in-time reading of all channel meters.
#[derive(Clone, Debug, PartialEq)]
pub struct MeterSnapshot {
    /// Absolute peak per channel since the previous snapshot.
    pub peak: Vec<f32>,
    /// RMS per channel of the most recent audio buffer.
    pub rms: Vec<f32>,
    /// Cumulative clipped-sample count per channel (wraps on overflow).
    pub clip_counts: Vec<u32>,
}

impl MeterSnapshot {
    /// Whether every channel was silent since the previous snapshot.
    pub fn is_silent(&self) -> bool {
        self.peak.iter().all(|&p| p == 0.0)
    }
}

impl OutputMeters {
    /// Creates meters for `num_channels` output channels (clamped to
    /// [`MAX_METER_CHANNELS`]).
    pub fn new(num_channels: usize) -> Self {
        let n = num_channels.min(MAX_METER_CHANNELS);
        Self {
            peak_bits: (0..n).map(|_| AtomicU32::new(0)).collect(),
            mean_square_bits: (0..n).map(|_| AtomicU32::new(0)).collect(),
            clip_counts: (0..n).map(|_| AtomicU32::new(0)).collect(),
        }
    }

    /// Number of metered channels.
    pub fn channel_count(&self) -> usize {
        self.peak_bits.len()
    }

    /// Ingests one interleaved buffer of mixed output. Called from the audio
    /// callback: no locks, no allocation, one pass over the samples.
    ///
    /// Non-finite samples are ignored rather than poisoning the meters.
    pub fn ingest(&self, buffer: &[f32], num_channels: usize) {
        if num_channels == 0 || buffer.len() < num_channels {
            return;
        }
        let metered = num_channels.min(self.peak_bits.len());
        if metered == 0 {
            return;
        }

        let mut peak = [0.0f32; MAX_METER_CHANNELS];
        let mut sum_sq = [0.0f32; MAX_METER_CHANNELS];
        let mut clips = [0u32; MAX_METER_CHANNELS];

        let num_frames = buffer.len() / num_channels;
        for frame in buffer.chunks_exact(num_channels) {
            for (c, &s) in frame[..metered].iter().enumerate() {
                let a = s.abs();
                if !a.is_finite() {
                    continue;
                }
                if a > peak[c] {
                    peak[c] = a;
                }
                sum_sq[c] += s * s;
                if a > 1.0 {
                    clips[c] += 1;
                }
            }
        }

        let inv_frames = 1.0 / num_frames as f32;
        for c in 0..metered {
            // Non-negative f32 bit patterns order like the floats themselves,
            // so fetch_max on the bits is a float max.
            self.peak_bits[c].fetch_max(peak[c].to_bits(), Ordering::Relaxed);
            self.mean_square_bits[c].store((sum_sq[c] * inv_frames).to_bits(), Ordering::Relaxed);
            if clips[c] > 0 {
                self.clip_counts[c].fetch_add(clips[c], Ordering::Relaxed);
            }
        }
    }

    /// Takes a snapshot of all channel meters, resetting the held peaks so
    /// the next snapshot reports the peak since this one.
    pub fn snapshot(&self) -> MeterSnapshot {
        MeterSnapshot {
            peak: self
                .peak_bits
                .iter()
                .map(|bits| f32::from_bits(bits.swap(0, Ordering::Relaxed)))
                .collect(),
            rms: self
                .mean_square_bits
                .iter()
                .map(|bits| f32::from_bits(bits.load(Ordering::Relaxed)).sqrt())
                .collect(),
            clip_counts: self
                .clip_counts
                .iter()
                .map(|count| count.load(Ordering::Relaxed))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn peak_and_rms_per_channel() {
        let meters = OutputMeters::new(2);
        // Channel 0: constant 0.5; channel 1: constant -0.25.
        let buffer = [0.5f32, -0.25, 0.5, -0.25, 0.5, -0.25, 0.5, -0.25];
        meters.ingest(&buffer, 2);

        let snap = meters.snapshot();
        assert_close(snap.peak[0], 0.5);
        assert_close(snap.peak[1], 0.25);
        assert_close(snap.rms[0], 0.5);
        assert_close(snap.rms[1], 0.25);
        assert_eq!(snap.clip_counts, vec![0, 0]);
        assert!(!snap.is_silent());
    }

    #[test]
    fn peak_resets_on_snapshot_rms_does_not() {
        let meters = OutputMeters::new(1);
        meters.ingest(&[0.8, 0.8], 1);

        let first = meters.snapshot();
        assert_close(first.peak[0], 0.8);

        // No ingest in between: peak was consumed, RMS still reflects the
        // most recent buffer.
        let second = meters.snapshot();
        assert_close(second.peak[0], 0.0);
        assert!(second.is_silent());
        assert_close(second.rms[0], 0.8);
    }

    #[test]
    fn peak_holds_max_across_buffers() {
        let meters = OutputMeters::new(1);
        meters.ingest(&[0.9], 1);
        meters.ingest(&[0.3], 1);

        let snap = meters.snapshot();
        assert_close(snap.peak[0], 0.9);
        // RMS reflects only the most recent buffer.
        assert_close(snap.rms[0], 0.3);
    }

    #[test]
    fn clip_counts_accumulate() {
        let meters = OutputMeters::new(1);
        meters.ingest(&[1.5, -2.0, 0.5], 1);
        meters.ingest(&[1.1], 1);

        let snap = meters.snapshot();
        assert_eq!(snap.clip_counts[0], 3);
        // Cumulative: not reset by snapshot.
        let snap = meters.snapshot();
        assert_eq!(snap.clip_counts[0], 3);
    }

    #[test]
    fn full_scale_is_not_a_clip() {
        let meters = OutputMeters::new(1);
        meters.ingest(&[1.0, -1.0], 1);
        assert_eq!(meters.snapshot().clip_counts[0], 0);
    }

    #[test]
    fn non_finite_samples_ignored() {
        let meters = OutputMeters::new(1);
        meters.ingest(&[f32::NAN, f32::INFINITY, 0.5, f32::NEG_INFINITY], 1);

        let snap = meters.snapshot();
        assert_close(snap.peak[0], 0.5);
        assert!(snap.rms[0].is_finite());
        assert_eq!(snap.clip_counts[0], 0);
    }

    #[test]
    fn channel_count_clamped_to_max() {
        let meters = OutputMeters::new(MAX_METER_CHANNELS + 16);
        assert_eq!(meters.channel_count(), MAX_METER_CHANNELS);

        // Ingesting a wider buffer meters only the first MAX channels and
        // must not panic.
        let buffer = vec![0.5f32; (MAX_METER_CHANNELS + 16) * 2];
        meters.ingest(&buffer, MAX_METER_CHANNELS + 16);
        assert_close(meters.snapshot().peak[0], 0.5);
    }

    #[test]
    fn empty_and_undersized_buffers_are_noops() {
        let meters = OutputMeters::new(2);
        meters.ingest(&[], 2);
        meters.ingest(&[0.5], 2);
        meters.ingest(&[0.5, 0.5], 0);
        assert!(meters.snapshot().is_silent());
    }
}
