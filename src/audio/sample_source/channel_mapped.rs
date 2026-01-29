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

/// A wrapper that implements ChannelMappedSampleSource for any SampleSource (planar format)
pub struct ChannelMappedSource {
    source: Box<dyn SampleSource>,
    channel_mappings: Vec<Vec<String>>,
    source_channel_count: u16,
    /// Pre-allocated buffer for reading from source (reused to avoid allocation)
    read_buffer: Vec<Vec<f32>>,
}

impl ChannelMappedSource {
    /// Create a new ChannelMappedSource
    pub fn new(
        source: Box<dyn SampleSource>,
        channel_mappings: Vec<Vec<String>>,
        source_channel_count: u16,
    ) -> Self {
        // Pre-allocate read buffer for the number of channels
        let read_buffer = vec![Vec::new(); source_channel_count as usize];
        Self {
            source,
            channel_mappings,
            source_channel_count,
            read_buffer,
        }
    }
}

impl ChannelMappedSampleSource for ChannelMappedSource {
    fn next_frames(
        &mut self,
        output: &mut [Vec<f32>],
        max_frames: usize,
    ) -> Result<usize, SampleSourceError> {
        let channel_count = self.source_channel_count as usize;

        if output.len() != channel_count {
            return Err(SampleSourceError::SampleConversionFailed(format!(
                "Output has {} channels, expected {}",
                output.len(),
                channel_count
            )));
        }

        // Clear output buffers
        for ch in output.iter_mut() {
            ch.clear();
        }

        // Ensure read buffer has the right number of channels
        if self.read_buffer.len() != channel_count {
            self.read_buffer = vec![Vec::new(); channel_count];
        }

        // Read planar data from the underlying source
        let frames_read = self
            .source
            .next_chunk(&mut self.read_buffer, max_frames)?;

        // Copy to output
        for (ch_idx, out_ch) in output.iter_mut().enumerate() {
            if ch_idx < self.read_buffer.len() {
                out_ch.extend_from_slice(&self.read_buffer[ch_idx]);
            }
        }

        Ok(frames_read)
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
        let transcoder =
            AudioTranscoder::new(source, &source_format, &target_format, channel_count)?;
        Box::new(transcoder)
    } else {
        source
    };

    Ok(Box::new(ChannelMappedSource::new(
        sample_source,
        channel_mappings,
        channel_count,
    )))
}
