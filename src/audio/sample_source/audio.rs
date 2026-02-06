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
/// This uses symphonia to decode various audio formats - no transcoding logic
pub struct AudioSampleSource {
    format_reader: Box<dyn FormatReader>,
    decoder: Box<dyn symphonia::core::codecs::Decoder>,
    track_id: u32,
    is_finished: bool,
    // Buffered reading to reduce I/O operations
    sample_buffer: Vec<f32>,
    buffer_position: usize,
    buffer_size: usize,
    // Leftover samples from the last decoded packet that didn't fit in the buffer
    leftover_samples: Vec<f32>,
    // WAV / PCM metadata for scaling & reporting
    bits_per_sample: u16,
    channels: u16,
    sample_rate: u32,
    sample_format: crate::audio::SampleFormat,
    duration: std::time::Duration,
}

impl SampleSource for AudioSampleSource {
    fn next_sample(&mut self) -> Result<Option<f32>, SampleSourceError> {
        if self.is_finished {
            return Ok(None);
        }

        // Check if we need to refill the buffer
        if self.buffer_position >= self.sample_buffer.len() {
            self.refill_buffer()?;

            // If buffer is still empty after refill, we're finished
            if self.sample_buffer.is_empty() {
                self.is_finished = true;
                return Ok(None);
            }
        }

        // Return the next sample from the buffer
        let sample = self.sample_buffer[self.buffer_position];
        self.buffer_position += 1;
        Ok(Some(sample))
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
        // Open the file (include path in error so user sees which file failed)
        let path_ref = path.as_ref();
        let file = File::open(path_ref).map_err(|e| {
            SampleSourceError::IoError(std::io::Error::new(
                e.kind(),
                format!("{}: {}", path_ref.display(), e),
            ))
        })?;
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
        //
        // Here, a value of 0 means "unspecified" and is only used inside this
        // constructor; if it remains 0 after probing, we return an error and
        // never construct an AudioSampleSource with channel_count == 0.
        let channels = params.channels.map(|c| c.count() as u16).unwrap_or(0);

        // In tests we sometimes want to exercise the channel‑detection path
        // even for formats where the container/codec already reports the
        // channel count. This is controlled by an env var so production
        // behaviour is unaffected.
        let force_detect = cfg!(test) && std::env::var("MTRACK_FORCE_DETECT_CHANNELS").is_ok();

        let (channels, initial_leftover) = if channels > 0 && !force_detect {
            (channels, Vec::new())
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
            sample_buffer: Vec::with_capacity(buffer_size * channels as usize),
            buffer_position: 0,
            buffer_size,
            leftover_samples: initial_leftover,
            bits_per_sample,
            channels,
            sample_rate,
            sample_format,
            duration,
        };

        // If start_time is provided, seek to that position
        if let Some(start) = start_time {
            // Any samples decoded while probing for channels belong to the
            // beginning of the stream. If the caller requested a non‑zero
            // start time, those samples are no longer relevant and must not
            // be returned ahead of the seek target.
            source.leftover_samples.clear();

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
    /// Returns:
    /// - `Ok(Some(packet))` if a packet was successfully read
    /// - `Ok(None)` if EOF was reached (UnexpectedEof or DecodeError)
    /// - `Err(...)` if an error occurred that should be returned
    ///
    /// Note: ResetRequired errors are propagated to callers so they can reset the decoder.
    fn read_next_packet(
        format_reader: &mut dyn FormatReader,
    ) -> Result<Option<symphonia::core::formats::Packet>, SampleSourceError> {
        match format_reader.next_packet() {
            Ok(packet) => Ok(Some(packet)),
            Err(SymphoniaError::ResetRequired) => {
                // ResetRequired is propagated to callers so they can reset the decoder
                Err(SampleSourceError::AudioError(SymphoniaError::ResetRequired))
            }
            Err(SymphoniaError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                // End of file - we're done reading
                Ok(None)
            }
            Err(SymphoniaError::DecodeError(_)) => {
                // Some decoders return DecodeError at EOF instead of IoError
                Ok(None)
            }
            Err(e) => Err(SampleSourceError::AudioError(e)),
        }
    }

    /// Reads and decodes the next packet for the given track. Handles ResetRequired by
    /// resetting the decoder and retrying. Returns `Ok(Some((samples, channels)))` when
    /// a packet was decoded, `Ok(None)` on EOF, or `Err` on other errors.
    fn read_and_decode_next_packet_for_track(
        format_reader: &mut dyn FormatReader,
        decoder: &mut dyn symphonia::core::codecs::Decoder,
        track_id: u32,
    ) -> Result<Option<(Vec<f32>, usize)>, SampleSourceError> {
        loop {
            let packet = match Self::read_next_packet(format_reader) {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(None),
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
            let (samples, channels) = Self::decode_buffer_to_f32(decoded)?;
            if channels > 0 && !samples.is_empty() {
                return Ok(Some((samples, channels)));
            }
        }
    }

    /// Refills the sample buffer by reading a chunk from the audio file
    fn refill_buffer(&mut self) -> Result<(), SampleSourceError> {
        // Clear the buffer and reset position
        self.sample_buffer.clear();
        self.buffer_position = 0;

        let mut samples_read = 0;
        let target_samples = self.buffer_size * self.channels as usize;
        // First, add any leftover samples from the previous buffer fill
        if !self.leftover_samples.is_empty() {
            let to_take = target_samples.min(self.leftover_samples.len());
            self.sample_buffer
                .extend_from_slice(&self.leftover_samples[..to_take]);
            samples_read += to_take;

            // Keep the rest as leftover for next time
            if self.leftover_samples.len() > to_take {
                self.leftover_samples.drain(..to_take);
            } else {
                self.leftover_samples.clear();
            }
            // If leftover samples completely filled the buffer, we're done for this iteration
            if samples_read >= target_samples {
                return Ok(());
            }
        }

        // Read packets until we reach the end of the file or fill our buffer.
        //
        // NOTE: We intentionally *do not* special‑case "no progress" here based on
        // samples_read. Some formats (e.g. Ogg/Vorbis) have multiple header packets
        // that decode to zero PCM frames before the first audio packet. Treating
        // those as "no progress" would cause us to bail out early and never see
        // the real audio data.
        loop {
            let (samples, _decoded_channels) = match Self::read_and_decode_next_packet_for_track(
                self.format_reader.as_mut(),
                self.decoder.as_mut(),
                self.track_id,
            ) {
                Ok(Some((samples, ch))) => (samples, ch),
                Ok(None) => break,
                Err(e) => {
                    // For very small files, some errors might indicate EOF
                    if samples_read == 0 && self.sample_buffer.is_empty() {
                        break;
                    }
                    return Err(e);
                }
            };

            // Add samples to the buffer
            if !samples.is_empty() {
                let remaining = target_samples.saturating_sub(samples_read);
                if remaining > 0 {
                    let to_take = remaining.min(samples.len());
                    self.sample_buffer.extend_from_slice(&samples[..to_take]);
                    samples_read += to_take;

                    // If we have more samples than we can fit, save them as leftover
                    if samples.len() > to_take {
                        self.leftover_samples.extend_from_slice(&samples[to_take..]);
                        // Buffer is full for this iteration, break to avoid infinite loops
                        // Leftover samples will be used in the next refill_buffer call
                        break;
                    }

                    // For very small files, if we got a small number of samples (less than 32 total),
                    // the file is likely exhausted. Break immediately to avoid calling next_packet() again
                    // which might block indefinitely. This handles edge cases with tiny files
                    // (like the test file with only 3 samples).
                    // We use a fixed threshold (32) rather than channels-based to catch all tiny files.
                    // This is critical for preventing hangs on very small audio files.
                    if samples.len() < 32 {
                        break;
                    }
                } else {
                    // Buffer is full, save all samples as leftover and break
                    self.leftover_samples.extend_from_slice(&samples);
                    break;
                }
            }

            // If we've filled our target buffer, break for this iteration
            // This prevents infinite loops while still allowing us to read all samples
            // across multiple refill_buffer calls
            if samples_read >= target_samples {
                break;
            }
        }

        // If we read no samples and have no leftovers, we're at the end of the file
        if samples_read == 0 && self.leftover_samples.is_empty() {
            self.is_finished = true;
        }

        Ok(())
    }

    /// When codec/channel metadata is missing, read and decode packets until we
    /// see the first audio buffer for our track, and derive the channel count
    /// from that buffer. The decoded samples are returned so they can be used
    /// as the initial contents of the sample buffer.
    fn detect_channels_and_prime_buffer(
        format_reader: &mut dyn FormatReader,
        decoder: &mut dyn symphonia::core::codecs::Decoder,
        track_id: u32,
    ) -> Result<(u16, Vec<f32>), SampleSourceError> {
        match Self::read_and_decode_next_packet_for_track(format_reader, decoder, track_id)? {
            Some((samples, channels)) => Ok((channels as u16, samples)),
            None => Err(SampleSourceError::SampleConversionFailed(
                "Channels not specified".to_string(),
            )),
        }
    }

    /// Converts a decoded AudioBufferRef to a Vec<f32> of interleaved samples
    /// and returns the channel count as observed in the decoded buffer.
    fn decode_buffer_to_f32(
        decoded: AudioBufferRef,
    ) -> Result<(Vec<f32>, usize), SampleSourceError> {
        match decoded {
            AudioBufferRef::F32(buf) => Ok(Self::interleave_planar_samples(&buf, |sample| sample)),
            AudioBufferRef::F64(buf) => Ok(Self::interleave_planar_samples(&buf, |sample| {
                sample as f32
            })),
            AudioBufferRef::S8(buf) => Ok(Self::interleave_planar_samples(&buf, |sample| {
                Self::scale_s8(sample)
            })),
            AudioBufferRef::S16(buf) => Ok(Self::interleave_planar_samples(&buf, |sample| {
                Self::scale_s16(sample)
            })),
            AudioBufferRef::S24(buf) => Ok(Self::interleave_planar_samples(&buf, |sample| {
                Self::scale_s24(sample.inner())
            })),
            AudioBufferRef::S32(buf) => Ok(Self::interleave_planar_samples(&buf, |sample| {
                Self::scale_s32(sample)
            })),
            AudioBufferRef::U8(buf) => Ok(Self::interleave_planar_samples(&buf, |sample| {
                Self::scale_u8(sample)
            })),
            AudioBufferRef::U16(buf) => Ok(Self::interleave_planar_samples(&buf, |sample| {
                Self::scale_u16(sample)
            })),
            AudioBufferRef::U24(buf) => Ok(Self::interleave_planar_samples(&buf, |sample| {
                Self::scale_u24(sample.inner())
            })),
            AudioBufferRef::U32(buf) => Ok(Self::interleave_planar_samples(&buf, |sample| {
                Self::scale_u32(sample)
            })),
        }
    }

    /// Helper to interleave planar samples from a generic AudioBuffer.
    /// The closure receives a single sample value and returns the f32 sample value.
    fn interleave_planar_samples<T, F>(buf: &AudioBuffer<T>, convert: F) -> (Vec<f32>, usize)
    where
        T: symphonia::core::sample::Sample,
        F: Fn(T) -> f32,
    {
        let frames = buf.frames();
        let channels = buf.spec().channels.count();
        let planes = buf.planes();
        let mut samples = Vec::with_capacity(frames * channels);
        for frame_idx in 0..frames {
            for ch_idx in 0..channels {
                samples.push(convert(planes.planes()[ch_idx][frame_idx]));
            }
        }
        (samples, channels)
    }

    // Scaling helpers for all integer formats. These are `pub(crate)` so they can
    // be validated directly in unit tests.

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
