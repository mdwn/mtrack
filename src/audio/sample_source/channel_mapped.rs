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
use crate::audio::TargetFormat;

use super::error::TranscodingError;
use super::traits::{ChannelMappedSampleSource, SampleSource};
use super::transcoder::AudioTranscoder;

/// A wrapper that implements ChannelMappedSampleSource for any SampleSource
pub struct ChannelMappedSource {
    source: Box<dyn SampleSource>,
    channel_mappings: Vec<Vec<String>>,
    source_channel_count: u16,
}

impl ChannelMappedSource {
    /// Create a new ChannelMappedSource
    pub fn new(
        source: Box<dyn SampleSource>,
        channel_mappings: Vec<Vec<String>>,
        source_channel_count: u16,
    ) -> Self {
        Self {
            source,
            channel_mappings,
            source_channel_count,
        }
    }
}

impl ChannelMappedSampleSource for ChannelMappedSource {
    fn next_sample(&mut self) -> Result<Option<f32>, TranscodingError> {
        self.source.next_sample()
    }

    fn next_frame(&mut self, output: &mut [f32]) -> Result<Option<usize>, TranscodingError> {
        let channel_count = self.source_channel_count as usize;
        if output.len() < channel_count {
            return Err(TranscodingError::SampleConversionFailed(format!(
                "Output buffer too small: need {} samples",
                channel_count
            )));
        }
        for out in output.iter_mut().take(channel_count) {
            match self.source.next_sample()? {
                Some(sample) => *out = sample,
                None => return Ok(None),
            }
        }
        Ok(Some(channel_count))
    }

    fn channel_mappings(&self) -> &Vec<Vec<String>> {
        &self.channel_mappings
    }

    fn source_channel_count(&self) -> u16 {
        self.source_channel_count
    }
}

/// A wrapper that makes Box<dyn SampleSource> work with AudioTranscoder
struct SampleSourceWrapper {
    source: Box<dyn SampleSource>,
}

impl SampleSource for SampleSourceWrapper {
    fn next_sample(&mut self) -> Result<Option<f32>, TranscodingError> {
        self.source.next_sample()
    }

    fn channel_count(&self) -> u16 {
        self.source.channel_count()
    }

    fn sample_rate(&self) -> u32 {
        self.source.sample_rate()
    }

    fn bits_per_sample(&self) -> u16 {
        self.source.bits_per_sample()
    }

    fn sample_format(&self) -> crate::audio::SampleFormat {
        self.source.sample_format()
    }

    fn duration(&self) -> Option<std::time::Duration> {
        self.source.duration()
    }
}

/// Create a ChannelMappedSampleSource from a generic SampleSource
pub fn create_channel_mapped_sample_source(
    source: Box<dyn SampleSource>,
    target_format: TargetFormat,
    channel_mappings: Vec<Vec<String>>,
    _buffer_size: usize,
    _buffer_threshold: usize,
) -> Result<Box<dyn ChannelMappedSampleSource>, TranscodingError> {
    let source_format = TargetFormat::new(
        source.sample_rate(),
        source.sample_format(),
        source.bits_per_sample(),
    )
    .map_err(|e| TranscodingError::SampleConversionFailed(e.to_string()))?;

    let needs_transcoding = source_format.sample_rate != target_format.sample_rate
        || source_format.sample_format != target_format.sample_format
        || source_format.bits_per_sample != target_format.bits_per_sample;

    let channel_count = source.channel_count();
    let sample_source: Box<dyn SampleSource> = if needs_transcoding {
        // Create a wrapper that can be used with AudioTranscoder
        let wrapper = SampleSourceWrapper { source };
        let transcoder =
            AudioTranscoder::new(wrapper, &source_format, &target_format, channel_count)?;
        Box::new(transcoder)
    } else {
        source
    };

    // Use the sample source directly - buffering adds complexity without benefit
    // since the resampler already handles buffering internally
    Ok(Box::new(ChannelMappedSource::new(
        sample_source,
        channel_mappings,
        channel_count,
    )))
}
