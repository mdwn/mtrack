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
use super::error::SampleSourceError;

/// A source of audio samples that processes an iterator
pub trait SampleSource: Send + Sync {
    /// Get the next sample from the source
    /// Returns Ok(Some(sample)) if a sample is available
    /// Returns Ok(None) if the source is finished
    /// Returns Err(error) if an error occurred
    fn next_sample(&mut self) -> Result<Option<f32>, SampleSourceError>;

    /// Get the number of channels in this source
    fn channel_count(&self) -> u16;

    /// Get the sample rate of this source
    fn sample_rate(&self) -> u32;

    /// Get the bits per sample of this source
    fn bits_per_sample(&self) -> u16;

    /// Get the sample format of this source
    fn sample_format(&self) -> crate::audio::SampleFormat;

    /// Get the duration of this source (if known)
    /// Returns None if the duration is unknown or infinite
    fn duration(&self) -> Option<std::time::Duration>;
}

/// Blanket implementation for Box<dyn SampleSource>
/// This allows Box<dyn SampleSource> to be used directly with generic functions
/// that require S: SampleSource, eliminating the need for wrapper types.
impl SampleSource for Box<dyn SampleSource> {
    fn next_sample(&mut self) -> Result<Option<f32>, SampleSourceError> {
        (**self).next_sample()
    }

    fn channel_count(&self) -> u16 {
        (**self).channel_count()
    }

    fn sample_rate(&self) -> u32 {
        (**self).sample_rate()
    }

    fn bits_per_sample(&self) -> u16 {
        (**self).bits_per_sample()
    }

    fn sample_format(&self) -> crate::audio::SampleFormat {
        (**self).sample_format()
    }

    fn duration(&self) -> Option<std::time::Duration> {
        (**self).duration()
    }
}

/// A sample source with explicit channel mapping information
/// This replaces the complex SongSource architecture with a simpler, more debuggable approach
pub trait ChannelMappedSampleSource: Send + Sync {
    /// Get the next sample from the source
    /// Returns Ok(Some(sample)) if a sample is available
    /// Returns Ok(None) if the source is finished
    /// Returns Err(error) if an error occurred
    fn next_sample(&mut self) -> Result<Option<f32>, SampleSourceError>;

    /// Get the next frame of samples (all channels for one time step)
    /// Writes samples directly into the provided output slice
    /// Returns Ok(Some(count)) where count is the number of samples written
    /// Returns Ok(None) if the source is finished
    /// Returns Err(error) if an error occurred
    /// The output slice must have capacity for at least source_channel_count() samples
    fn next_frame(&mut self, output: &mut [f32]) -> Result<Option<usize>, SampleSourceError> {
        let channel_count = self.source_channel_count() as usize;
        if output.len() < channel_count {
            return Err(SampleSourceError::SampleConversionFailed(format!(
                "Output buffer too small: need {} samples",
                channel_count
            )));
        }
        for out in output.iter_mut().take(channel_count) {
            match self.next_sample()? {
                Some(sample) => *out = sample,
                None => return Ok(None),
            }
        }
        Ok(Some(channel_count))
    }

    /// Get the channel mappings for this source
    /// Returns a Vec where each element corresponds to a source channel
    /// Each Vec<String> contains the labels that source channel maps to
    /// Empty Vec means that source channel is not mapped to any output
    fn channel_mappings(&self) -> &Vec<Vec<String>>;

    /// Get the number of source channels in this sample source
    fn source_channel_count(&self) -> u16;
}

#[cfg(test)]
pub trait SampleSourceTestExt {
    /// Check if the source is finished (no more samples)
    fn is_finished(&self) -> bool;
}
