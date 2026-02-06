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
use crate::audio::TargetFormat;

use super::error::SampleSourceError;
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
    fn next_sample(&mut self) -> Result<Option<f32>, SampleSourceError> {
        self.source.next_sample()
    }

    fn channel_mappings(&self) -> &Vec<Vec<String>> {
        &self.channel_mappings
    }

    fn source_channel_count(&self) -> u16 {
        self.source_channel_count
    }
}

/// Create a ChannelMappedSampleSource from a generic SampleSource
pub fn create_channel_mapped_sample_source(
    source: Box<dyn SampleSource>,
    target_format: TargetFormat,
    channel_mappings: Vec<Vec<String>>,
) -> Result<Box<dyn ChannelMappedSampleSource>, SampleSourceError> {
    let source_format = TargetFormat::new(
        source.sample_rate(),
        source.sample_format(),
        source.bits_per_sample(),
    )
    .map_err(|e| SampleSourceError::SampleConversionFailed(e.to_string()))?;

    let needs_transcoding = source_format.sample_rate != target_format.sample_rate
        || source_format.sample_format != target_format.sample_format
        || source_format.bits_per_sample != target_format.bits_per_sample;

    let channel_count = source.channel_count();
    let sample_source: Box<dyn SampleSource> = if needs_transcoding {
        // Box<dyn SampleSource> now implements SampleSource directly, so we can use it with AudioTranscoder
        let transcoder =
            AudioTranscoder::new(source, &source_format, &target_format, channel_count)?;
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
