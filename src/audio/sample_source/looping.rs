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
use std::sync::Arc;

use super::error::SampleSourceError;
use super::traits::SampleSource;

/// A sample source that loops its sample data indefinitely.
/// Used for stress testing and continuous audio generation.
#[allow(dead_code)]
pub struct LoopingSampleSource {
    /// The samples stored in an Arc for efficient cloning.
    samples: Arc<Vec<f32>>,
    /// Current playback position.
    current_index: usize,
    /// Number of channels.
    channel_count: u16,
    /// Sample rate.
    sample_rate: u32,
    /// Volume scale factor (0.0 to 1.0).
    volume: f32,
}

impl LoopingSampleSource {
    /// Creates a new looping sample source from shared sample data.
    /// The source will loop indefinitely, wrapping back to the beginning
    /// when the end of the sample data is reached.
    #[allow(dead_code)]
    pub fn from_shared(
        samples: Arc<Vec<f32>>,
        channel_count: u16,
        sample_rate: u32,
        volume: f32,
    ) -> Self {
        Self {
            samples,
            current_index: 0,
            channel_count,
            sample_rate,
            volume,
        }
    }
}

impl SampleSource for LoopingSampleSource {
    fn next_sample(&mut self) -> Result<Option<f32>, SampleSourceError> {
        if self.samples.is_empty() {
            return Ok(None);
        }
        let sample = self.samples[self.current_index] * self.volume;
        self.current_index += 1;
        if self.current_index >= self.samples.len() {
            self.current_index = 0;
        }
        Ok(Some(sample))
    }

    fn channel_count(&self) -> u16 {
        self.channel_count
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn bits_per_sample(&self) -> u16 {
        32
    }

    fn sample_format(&self) -> crate::audio::SampleFormat {
        crate::audio::SampleFormat::Float
    }

    fn duration(&self) -> Option<std::time::Duration> {
        None // Infinite source
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_looping_wraps_around() {
        let samples = Arc::new(vec![0.1, 0.2, 0.3, 0.4]);
        let mut source = LoopingSampleSource::from_shared(samples, 2, 44100, 1.0);

        // First pass through the data
        assert_eq!(source.next_sample().unwrap(), Some(0.1));
        assert_eq!(source.next_sample().unwrap(), Some(0.2));
        assert_eq!(source.next_sample().unwrap(), Some(0.3));
        assert_eq!(source.next_sample().unwrap(), Some(0.4));

        // Should wrap around
        assert_eq!(source.next_sample().unwrap(), Some(0.1));
        assert_eq!(source.next_sample().unwrap(), Some(0.2));
    }

    #[test]
    fn test_looping_volume_scaling() {
        let samples = Arc::new(vec![1.0, -1.0]);
        let mut source = LoopingSampleSource::from_shared(samples, 1, 44100, 0.5);

        assert_eq!(source.next_sample().unwrap(), Some(0.5));
        assert_eq!(source.next_sample().unwrap(), Some(-0.5));

        // Loops with volume still applied
        assert_eq!(source.next_sample().unwrap(), Some(0.5));
    }

    #[test]
    fn test_looping_empty_samples() {
        let samples = Arc::new(vec![]);
        let mut source = LoopingSampleSource::from_shared(samples, 1, 44100, 1.0);

        assert_eq!(source.next_sample().unwrap(), None);
    }

    #[test]
    fn test_looping_metadata() {
        let samples = Arc::new(vec![0.0; 100]);
        let source = LoopingSampleSource::from_shared(samples, 2, 48000, 0.8);

        assert_eq!(source.channel_count(), 2);
        assert_eq!(source.sample_rate(), 48000);
        assert_eq!(source.bits_per_sample(), 32);
        assert_eq!(source.sample_format(), crate::audio::SampleFormat::Float);
        assert!(source.duration().is_none());
    }

    #[test]
    fn test_looping_many_cycles() {
        let samples = Arc::new(vec![0.5, -0.5]);
        let mut source = LoopingSampleSource::from_shared(samples, 1, 44100, 1.0);

        // Run through many cycles to verify no drift or off-by-one
        for i in 0..10000 {
            let expected = if i % 2 == 0 { 0.5 } else { -0.5 };
            assert_eq!(
                source.next_sample().unwrap(),
                Some(expected),
                "Mismatch at sample {}",
                i
            );
        }
    }
}
