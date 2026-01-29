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
use std::fs::File;
use std::path::Path;

use symphonia::core::audio::{AudioBuffer, AudioBufferRef, Signal};
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymphoniaError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::default::get_codecs;
use symphonia::default::get_probe;

use super::error::SampleSourceError;
use super::traits::SampleSource;

#[cfg(test)]
use super::traits::SampleSourceTestExt;

/// A sample source that reads audio files (WAV, MP3, FLAC, etc.) and provides scaled samples
/// in planar format. This uses symphonia to decode various audio formats.
pub struct AudioSampleSource {
    format_reader: Box<dyn FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    is_finished: bool,
    // Buffered reading in planar format to reduce I/O operations
    // Each Vec<f32> is one channel's samples
    planar_buffer: Vec<Vec<f32>>,
    buffer_position: usize, // Position in frames (not samples)
    buffer_size: usize,     // Target frames per buffer fill
    // Leftover frames from the last decoded packet (planar format)
    leftover_frames: Vec<Vec<f32>>,
    // WAV / PCM metadata for scaling & reporting
    bits_per_sample: u16,
    channels: u16,
    sample_rate: u32,
    sample_format: crate::audio::SampleFormat,
    duration: std::time::Duration,
}

impl SampleSource for AudioSampleSource {
    fn next_chunk(
        &mut self,
        output: &mut [Vec<f32>],
        max_frames: usize,
    ) -> Result<usize, SampleSourceError> {
        if self.is_finished || max_frames == 0 {
            return Ok(0);
        }

        let num_channels = self.channels as usize;
        if output.len() != num_channels {
            return Err(SampleSourceError::SampleConversionFailed(format!(
                "Output has {} channels, expected {}",
                output.len(),
                num_channels
            )));
        }

        // Clear output buffers
        for ch in output.iter_mut() {
            ch.clear();
        }

        let mut total_frames = 0;

        while total_frames < max_frames {
            // Check if we need to refill the internal buffer
            let buffer_frames = self.planar_buffer.first().map(|c| c.len()).unwrap_or(0);
            if self.buffer_position >= buffer_frames {
                self.refill_buffer()?;

                // If buffer is still empty after refill, we're finished
                let new_buffer_frames = self.planar_buffer.first().map(|c| c.len()).unwrap_or(0);
                if new_buffer_frames == 0 {
                    self.is_finished = true;
                    break;
                }
            }

            // Calculate how many frames we can copy in this iteration
            let buffer_frames = self.planar_buffer.first().map(|c| c.len()).unwrap_or(0);
            let available = buffer_frames - self.buffer_position;
            let remaining = max_frames - total_frames;
            let to_copy = available.min(remaining);

            // Bulk copy from internal buffer to output for each channel
            for (ch_idx, out_ch) in output.iter_mut().enumerate() {
                out_ch.extend_from_slice(
                    &self.planar_buffer[ch_idx][self.buffer_position..self.buffer_position + to_copy],
                );
            }

            self.buffer_position += to_copy;
            total_frames += to_copy;
        }

        Ok(total_frames)
    }

    fn channel_count(&self) -> u16 {
        self.channels
    }

    fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn bits_per_sample(&self) -> u16 {
        self.bits_per_sample
    }

    fn sample_format(&self) -> crate::audio::SampleFormat {
        self.sample_format
    }

    fn duration(&self) -> Option<std::time::Duration> {
        Some(self.duration)
    }
}

impl AudioSampleSource {
    /// Creates a new audio sample source from a file path
    /// Supports WAV, MP3, FLAC, and other formats supported by symphonia
    pub fn from_file<P: AsRef<Path>>(
        path: P,
        start_time: Option<std::time::Duration>,
        buffer_size: usize,
    ) -> Result<Self, SampleSourceError> {
        // Open the file
        let file = File::open(&path)?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        // Create a hint to help the format registry guess the format
        let mut hint = Hint::new();
        if let Some(extension) = path.as_ref().extension().and_then(|ext| ext.to_str()) {
            hint.with_extension(extension);
        }

        // Probe the format
        let meta_opts: MetadataOptions = Default::default();
        let fmt_opts: FormatOptions = Default::default();
        let probe = get_probe();
        let file_path = path.as_ref().to_string_lossy().to_string();
        let probed = probe
            .format(&hint, mss, &fmt_opts, &meta_opts)
            .map_err(|e| {
                SampleSourceError::SampleConversionFailed(format!("'{}': {}", file_path, e))
            })?;

        let mut format_reader = probed.format;

        // Find the first audio track (need to do this before moving format_reader)
        let track = format_reader
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| {
                SampleSourceError::SampleConversionFailed("No audio track found".to_string())
            })?;

        let track_id = track.id;
        let params = &track.codec_params;

        // Get the sample rate and bits per sample
        let sample_rate = params.sample_rate.ok_or_else(|| {
            SampleSourceError::SampleConversionFailed("Sample rate not specified".to_string())
        })?;
        let bits_per_sample = params.bits_per_sample.unwrap_or(16) as u16; // Default to 16-bit if not specified

        // Determine sample format from codec
        let sample_format = if params.codec == symphonia::core::codecs::CODEC_TYPE_PCM_F32LE
            || params.codec == symphonia::core::codecs::CODEC_TYPE_PCM_F32BE
            || params.codec == symphonia::core::codecs::CODEC_TYPE_PCM_F64LE
            || params.codec == symphonia::core::codecs::CODEC_TYPE_PCM_F64BE
        {
            crate::audio::SampleFormat::Float
        } else {
            crate::audio::SampleFormat::Int
        };

        // Calculate duration
        let duration = if let Some(n_frames) = params.n_frames {
            std::time::Duration::from_secs_f64(n_frames as f64 / sample_rate as f64)
        } else {
            // If duration is unknown, we'll set it to zero and update it as we read
            std::time::Duration::ZERO
        };

        // Create the decoder
        let decoder_opts: DecoderOptions = Default::default();
        let mut decoder = get_codecs().make(params, &decoder_opts).map_err(|e| {
            SampleSourceError::SampleConversionFailed(format!("'{}': {}", file_path, e))
        })?;

        // Determine channels. Prefer container/codec metadata, but if it's
        // missing we proactively decode the first audio packet to derive the
        // actual channel count. If we still can't determine it, we fail.
        let channels = params.channels.map(|c| c.count() as u16).unwrap_or(0);

        // In tests we sometimes want to exercise the channel‑detection path
        let force_detect = cfg!(test) && std::env::var("MTRACK_FORCE_DETECT_CHANNELS").is_ok();

        let (channels, initial_leftover) = if channels > 0 && !force_detect {
            (channels, vec![Vec::new(); channels as usize])
        } else {
            Self::detect_channels_and_prime_buffer(
                format_reader.as_mut(),
                decoder.as_mut(),
                track_id,
            )?
        };

        let mut source = Self {
            format_reader,
            decoder,
            track_id,
            is_finished: false,
            planar_buffer: vec![Vec::with_capacity(buffer_size); channels as usize],
            buffer_position: 0,
            buffer_size,
            leftover_frames: initial_leftover,
            bits_per_sample,
            channels,
            sample_rate,
            sample_format,
            duration,
        };

        // If start_time is provided, seek to that position
        if let Some(start) = start_time {
            // Clear leftover frames when seeking
            for ch in source.leftover_frames.iter_mut() {
                ch.clear();
            }

            use symphonia::core::units::Time;
            let seek_to = SeekTo::Time {
                time: Time::from(start),
                track_id: Some(track_id),
            };
            source.format_reader.seek(SeekMode::Accurate, seek_to)?;
        }

        Ok(source)
    }

    /// Helper function to read the next packet with common error handling.
    fn read_next_packet(
        format_reader: &mut dyn FormatReader,
    ) -> Result<Option<symphonia::core::formats::Packet>, SampleSourceError> {
        match format_reader.next_packet() {
            Ok(packet) => Ok(Some(packet)),
            Err(SymphoniaError::ResetRequired) => {
                Err(SampleSourceError::AudioError(SymphoniaError::ResetRequired))
            }
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                Ok(None)
            }
            Err(SymphoniaError::DecodeError(_)) => Ok(None),
            Err(e) => Err(SampleSourceError::AudioError(e)),
        }
    }

    /// Refills the planar buffer by reading chunks from the audio file
    fn refill_buffer(&mut self) -> Result<(), SampleSourceError> {

        // Clear the buffer and reset position
        for ch in self.planar_buffer.iter_mut() {
            ch.clear();
        }
        self.buffer_position = 0;

        let mut frames_read = 0;
        let target_frames = self.buffer_size;

        // First, add any leftover frames from the previous buffer fill
        let leftover_frames = self.leftover_frames.first().map(|c| c.len()).unwrap_or(0);
        if leftover_frames > 0 {
            let to_take = target_frames.min(leftover_frames);
            for (ch_idx, ch) in self.planar_buffer.iter_mut().enumerate() {
                ch.extend_from_slice(&self.leftover_frames[ch_idx][..to_take]);
            }
            frames_read += to_take;

            // Keep the rest as leftover for next time
            if leftover_frames > to_take {
                for ch in self.leftover_frames.iter_mut() {
                    ch.drain(..to_take);
                }
            } else {
                for ch in self.leftover_frames.iter_mut() {
                    ch.clear();
                }
            }

            if frames_read >= target_frames {
                return Ok(());
            }
        }

        // Read packets until we reach the end of the file or fill our buffer
        loop {
            let packet = match Self::read_next_packet(self.format_reader.as_mut()) {
                Ok(Some(packet)) => packet,
                Ok(None) => break,
                Err(SampleSourceError::AudioError(SymphoniaError::ResetRequired)) => {
                    self.decoder.reset();
                    continue;
                }
                Err(e) => {
                    if frames_read == 0 && self.planar_buffer.first().map(|c| c.is_empty()).unwrap_or(true) {
                        break;
                    }
                    return Err(e);
                }
            };

            if packet.track_id() != self.track_id {
                continue;
            }

            let decoded = match self.decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(SymphoniaError::ResetRequired) => {
                    self.decoder.reset();
                    match self.decoder.decode(&packet) {
                        Ok(decoded) => decoded,
                        Err(e) => return Err(SampleSourceError::AudioError(e)),
                    }
                }
                Err(e) => return Err(SampleSourceError::AudioError(e)),
            };

            // Decode to planar format directly
            let (planar_samples, _decoded_channels) = Self::decode_buffer_to_planar(decoded)?;
            let decoded_frames = planar_samples.first().map(|c| c.len()).unwrap_or(0);

            if decoded_frames > 0 {
                let remaining = target_frames.saturating_sub(frames_read);
                if remaining > 0 {
                    let to_take = remaining.min(decoded_frames);
                    for (ch_idx, ch) in self.planar_buffer.iter_mut().enumerate() {
                        if ch_idx < planar_samples.len() {
                            ch.extend_from_slice(&planar_samples[ch_idx][..to_take]);
                        }
                    }
                    frames_read += to_take;

                    // Save leftover frames
                    if decoded_frames > to_take {
                        for (ch_idx, ch) in self.leftover_frames.iter_mut().enumerate() {
                            if ch_idx < planar_samples.len() {
                                ch.extend_from_slice(&planar_samples[ch_idx][to_take..]);
                            }
                        }
                        break;
                    }

                    // Small file optimization
                    if decoded_frames < 32 {
                        break;
                    }
                } else {
                    // Buffer is full, save all as leftover
                    for (ch_idx, ch) in self.leftover_frames.iter_mut().enumerate() {
                        if ch_idx < planar_samples.len() {
                            ch.extend_from_slice(&planar_samples[ch_idx]);
                        }
                    }
                    break;
                }
            }

            if frames_read >= target_frames {
                break;
            }
        }

        // If we read no frames and have no leftovers, we're at the end of the file
        let leftover_remaining = self.leftover_frames.first().map(|c| c.len()).unwrap_or(0);
        if frames_read == 0 && leftover_remaining == 0 {
            self.is_finished = true;
        }

        Ok(())
    }

    /// When codec/channel metadata is missing, read and decode packets until we
    /// see the first audio buffer for our track.
    fn detect_channels_and_prime_buffer(
        format_reader: &mut dyn FormatReader,
        decoder: &mut dyn symphonia::core::codecs::Decoder,
        track_id: u32,
    ) -> Result<(u16, Vec<Vec<f32>>), SampleSourceError> {
        loop {
            let packet = match Self::read_next_packet(format_reader) {
                Ok(Some(packet)) => packet,
                Ok(None) => break,
                Err(SampleSourceError::AudioError(SymphoniaError::ResetRequired)) => {
                    decoder.reset();
                    continue;
                }
                Err(e) => return Err(e),
            };

            if packet.track_id() != track_id {
                continue;
            }

            let decoded = match decoder.decode(&packet) {
                Ok(decoded) => decoded,
                Err(SymphoniaError::ResetRequired) => {
                    decoder.reset();
                    match decoder.decode(&packet) {
                        Ok(decoded) => decoded,
                        Err(e) => return Err(SampleSourceError::AudioError(e)),
                    }
                }
                Err(e) => return Err(SampleSourceError::AudioError(e)),
            };

            let (planar_samples, channels) = Self::decode_buffer_to_planar(decoded)?;
            let frames = planar_samples.first().map(|c| c.len()).unwrap_or(0);
            if channels > 0 && frames > 0 {
                return Ok((channels as u16, planar_samples));
            }
        }

        Err(SampleSourceError::SampleConversionFailed(
            "Channels not specified".to_string(),
        ))
    }

    /// Converts a decoded AudioBufferRef to planar Vec<Vec<f32>> format.
    /// Each inner Vec contains all samples for one channel.
    fn decode_buffer_to_planar(
        decoded: AudioBufferRef,
    ) -> Result<(Vec<Vec<f32>>, usize), SampleSourceError> {
        match decoded {
            AudioBufferRef::F32(buf) => Ok(Self::copy_planar_samples(&buf, |sample| sample)),
            AudioBufferRef::F64(buf) => Ok(Self::copy_planar_samples(&buf, |sample| sample as f32)),
            AudioBufferRef::S8(buf) => Ok(Self::copy_planar_samples(&buf, Self::scale_s8)),
            AudioBufferRef::S16(buf) => Ok(Self::copy_planar_samples(&buf, Self::scale_s16)),
            AudioBufferRef::S24(buf) => {
                Ok(Self::copy_planar_samples(&buf, |sample| Self::scale_s24(sample.inner())))
            }
            AudioBufferRef::S32(buf) => Ok(Self::copy_planar_samples(&buf, Self::scale_s32)),
            AudioBufferRef::U8(buf) => Ok(Self::copy_planar_samples(&buf, Self::scale_u8)),
            AudioBufferRef::U16(buf) => Ok(Self::copy_planar_samples(&buf, Self::scale_u16)),
            AudioBufferRef::U24(buf) => {
                Ok(Self::copy_planar_samples(&buf, |sample| Self::scale_u24(sample.inner())))
            }
            AudioBufferRef::U32(buf) => Ok(Self::copy_planar_samples(&buf, Self::scale_u32)),
        }
    }

    /// Helper to copy planar samples from a generic AudioBuffer without interleaving.
    fn copy_planar_samples<T, F>(buf: &AudioBuffer<T>, convert: F) -> (Vec<Vec<f32>>, usize)
    where
        T: symphonia::core::sample::Sample,
        F: Fn(T) -> f32,
    {
        let frames = buf.frames();
        let channels = buf.spec().channels.count();
        let planes = buf.planes();

        let mut planar_output = Vec::with_capacity(channels);
        for ch_idx in 0..channels {
            let mut channel_samples = Vec::with_capacity(frames);
            for frame_idx in 0..frames {
                channel_samples.push(convert(planes.planes()[ch_idx][frame_idx]));
            }
            planar_output.push(channel_samples);
        }

        (planar_output, channels)
    }

    // Scaling helpers for all integer formats.

    #[inline]
    pub(crate) fn scale_s8(sample: i8) -> f32 {
        sample as f32 / (1i64 << 7) as f32
    }

    #[inline]
    pub(crate) fn scale_s16(sample: i16) -> f32 {
        sample as f32 / (1i64 << 15) as f32
    }

    #[inline]
    pub(crate) fn scale_s24(sample: i32) -> f32 {
        sample as f32 / (1i64 << 23) as f32
    }

    #[inline]
    pub(crate) fn scale_s32(sample: i32) -> f32 {
        sample as f32 / (1i64 << 31) as f32
    }

    #[inline]
    pub(crate) fn scale_u8(sample: u8) -> f32 {
        (sample as f32 / u8::MAX as f32) * 2.0 - 1.0
    }

    #[inline]
    pub(crate) fn scale_u16(sample: u16) -> f32 {
        (sample as f32 / u16::MAX as f32) * 2.0 - 1.0
    }

    #[inline]
    pub(crate) fn scale_u24(sample: u32) -> f32 {
        let max = (1u32 << 24) - 1;
        (sample as f32 / max as f32) * 2.0 - 1.0
    }

    #[inline]
    pub(crate) fn scale_u32(sample: u32) -> f32 {
        (sample as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

#[cfg(test)]
impl SampleSourceTestExt for AudioSampleSource {
    fn is_finished(&self) -> bool {
        self.is_finished
    }
}
