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

/// A source of audio samples that processes chunks in planar format.
/// Planar format stores all samples for channel 0, then all samples for channel 1, etc.
/// This is more efficient for processing and matches the native format of most decoders.
pub trait SampleSource: Send + Sync {
    /// Get the next chunk of samples from the source in planar format.
    /// Each inner Vec corresponds to one channel.
    /// All channels will have the same number of samples written.
    /// Returns the number of frames (samples per channel) written (0 = EOF).
    /// Returns Err(error) if an error occurred.
    ///
    /// The output Vec must have exactly channel_count() elements.
    /// Each channel Vec will be cleared and filled with up to max_frames samples.
    fn next_chunk(
        &mut self,
        output: &mut [Vec<f32>],
        max_frames: usize,
    ) -> Result<usize, SampleSourceError>;

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
    fn next_chunk(
        &mut self,
        output: &mut [Vec<f32>],
        max_frames: usize,
    ) -> Result<usize, SampleSourceError> {
        (**self).next_chunk(output, max_frames)
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

/// A sample source with explicit channel mapping information.
/// Uses planar format internally for efficiency.
pub trait ChannelMappedSampleSource: Send + Sync {
    /// Get multiple frames of samples from the source in planar format.
    /// Each inner Vec corresponds to one source channel.
    /// Returns the number of frames written (0 = EOF).
    /// Returns Err(error) if an error occurred.
    ///
    /// The output Vec must have exactly source_channel_count() elements.
    fn next_frames(
        &mut self,
        output: &mut [Vec<f32>],
        max_frames: usize,
    ) -> Result<usize, SampleSourceError>;

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
