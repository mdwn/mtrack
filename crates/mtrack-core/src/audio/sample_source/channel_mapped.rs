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
use crate::config::ResamplerType;

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

    fn channel_mappings(&self) -> &[Vec<String>] {
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
    resampler_type: ResamplerType,
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
        let transcoder = AudioTranscoder::new(
            source,
            &source_format,
            &target_format,
            channel_count,
            resampler_type,
        )?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::sample_source::memory::MemorySampleSource;

    #[test]
    fn create_channel_mapped_passthrough() {
        let mem = MemorySampleSource::new(vec![0.5, 0.8], 1, 44100);
        let fmt = TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        let mappings = vec![vec!["ch".to_string()]];
        let mut src =
            create_channel_mapped_sample_source(Box::new(mem), fmt, mappings, ResamplerType::Sinc)
                .unwrap();
        assert_eq!(src.next_sample().unwrap(), Some(0.5));
        assert_eq!(src.next_sample().unwrap(), Some(0.8));
        assert_eq!(src.next_sample().unwrap(), None);
    }

    #[test]
    fn create_channel_mapped_with_resampling() {
        // Source at 48kHz, target at 44.1kHz - needs transcoding
        let num = 4800;
        let mut input = Vec::with_capacity(num);
        for i in 0..num {
            let t = i as f32 / 48000.0;
            input.push((2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.5);
        }
        let mem = MemorySampleSource::new(input, 1, 48000);
        let target_fmt = TargetFormat::new(44100, crate::audio::SampleFormat::Float, 32).unwrap();
        let mappings = vec![vec!["ch".to_string()]];
        let mut src = create_channel_mapped_sample_source(
            Box::new(mem),
            target_fmt,
            mappings,
            ResamplerType::Sinc,
        )
        .unwrap();

        let mut count = 0;
        while let Ok(Some(_)) = src.next_sample() {
            count += 1;
            if count > 10000 {
                break;
            }
        }
        assert!(count > 0, "resampled source should produce output");
    }

    #[test]
    fn channel_mapped_read_frames_default() {
        let mem = MemorySampleSource::new(vec![0.1, 0.2, 0.3, 0.4], 2, 44100);
        let mut src = ChannelMappedSource::new(
            Box::new(mem),
            vec![vec!["l".to_string()], vec!["r".to_string()]],
            2,
        );
        let mut output = vec![0.0f32; 4];
        let frames = src.read_frames(&mut output, 2).unwrap();
        assert_eq!(frames, 2);
        assert_eq!(output[0], 0.1);
        assert_eq!(output[1], 0.2);
        assert_eq!(output[2], 0.3);
        assert_eq!(output[3], 0.4);
    }
}
