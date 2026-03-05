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
use std::{error::Error, str::FromStr, time::Duration};

use duration_string::DurationString;
use serde::{Deserialize, Serialize};

use crate::audio::SampleFormat;

const DEFAULT_AUDIO_PLAYBACK_DELAY: Duration = Duration::ZERO;
const DEFAULT_BUFFER_SIZE: usize = 1024;
const DEFAULT_BUFFER_THREADS: usize = 2;

/// Which resampling algorithm to use when source and output sample rates differ.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ResamplerType {
    /// High-quality sinc interpolation (lower latency, higher CPU). This is the default.
    #[default]
    Sinc,
    /// FFT-based resampling (considerably faster for fixed-ratio resampling).
    Fft,
}

/// How to choose the CPAL stream buffer size (period size). Affects latency vs underrun tolerance.
#[derive(Deserialize, Serialize, Clone, Debug)]
#[serde(untagged)]
pub enum StreamBufferSize {
    /// Use the backend's default (may be high latency on some systems).
    #[serde(rename = "default")]
    Default,
    /// Use the device's minimum supported period size (lowest latency, most jitter-sensitive).
    #[serde(rename = "min")]
    Min,
    /// Use a fixed size in frames (same as buffer_size when not set).
    Fixed(usize),
}

/// A YAML representation of the audio configuration.
#[derive(Deserialize, Serialize, Clone)]
pub struct Audio {
    /// The audio device.
    device: String,

    /// Controls how long to wait before playback of an audio file starts.
    playback_delay: Option<String>,

    /// Target sample rate in Hz (default: 44100)
    sample_rate: Option<u32>,

    /// Target sample format (default: "int")
    sample_format: Option<String>,

    /// Target bits per sample (default: 32)
    bits_per_sample: Option<u16>,

    /// Buffer size for decoded audio samples (default: 1024 samples per channel)
    buffer_size: Option<usize>,

    /// CPAL stream buffer: "default" (backend default), "min" (lowest latency), or a number (frames).
    /// When unset, uses buffer_size. Lower values = lower latency but more sensitive to callback jitter.
    stream_buffer_size: Option<StreamBufferSize>,

    /// Number of worker threads for buffered song sources.
    /// Defaults to a small fixed value; must be >= 1.
    buffer_threads: Option<usize>,

    /// Resampling algorithm: "sinc" (default, high quality) or "fft" (faster on low-power hardware).
    resampler: Option<ResamplerType>,
}

impl Audio {
    /// New will create a new Audio configuration.
    pub fn new(device: &str) -> Audio {
        Audio {
            device: device.to_string(),
            playback_delay: None,
            sample_rate: None,
            sample_format: None,
            bits_per_sample: None,
            buffer_size: None,
            stream_buffer_size: None,
            buffer_threads: None,
            resampler: None,
        }
    }

    /// Returns the device from the configuration.
    pub fn device(&self) -> &str {
        &self.device
    }

    /// Returns the playback delay from the configuration.
    pub fn playback_delay(&self) -> Result<Duration, Box<dyn Error>> {
        match &self.playback_delay {
            Some(playback_delay) => Ok(DurationString::from_string(playback_delay.clone())?.into()),
            None => Ok(DEFAULT_AUDIO_PLAYBACK_DELAY),
        }
    }

    /// Returns the target sample rate (default: 44100)
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate.unwrap_or(44100)
    }

    /// Returns the target sample format (default: Float)
    pub fn sample_format(&self) -> Result<SampleFormat, Box<dyn Error>> {
        match self.sample_format.as_deref() {
            Some(format) => SampleFormat::from_str(format),
            None => Ok(SampleFormat::Int),
        }
    }

    /// Returns the target bits per sample (default: 32)
    pub fn bits_per_sample(&self) -> u16 {
        self.bits_per_sample.unwrap_or(32)
    }

    /// Returns the buffer size for decoded audio samples (default: 1024 samples per channel)
    pub fn buffer_size(&self) -> usize {
        self.buffer_size.unwrap_or(DEFAULT_BUFFER_SIZE)
    }

    /// Returns the number of worker threads used for buffered song sources.
    pub fn buffer_threads(&self) -> usize {
        self.buffer_threads.unwrap_or(DEFAULT_BUFFER_THREADS).max(1)
    }

    /// Returns the stream buffer size choice for CPAL (default/min/fixed).
    /// When None, the stream uses buffer_size() as a fixed frame count.
    pub fn stream_buffer_size(&self) -> Option<StreamBufferSize> {
        self.stream_buffer_size.clone()
    }

    /// Returns the resampling algorithm to use (default: Sinc).
    pub fn resampler(&self) -> ResamplerType {
        self.resampler.unwrap_or_default()
    }

    /// Sets the target sample rate.
    #[allow(dead_code)]
    pub fn with_sample_rate(mut self, sample_rate: u32) -> Self {
        self.sample_rate = Some(sample_rate);
        self
    }

    /// Sets the buffer size for decoded audio samples.
    #[allow(dead_code)]
    pub fn with_buffer_size(mut self, buffer_size: usize) -> Self {
        self.buffer_size = Some(buffer_size);
        self
    }

    /// Sets the target sample format ("float" or "int").
    #[allow(dead_code)]
    pub fn with_sample_format(mut self, format: &str) -> Self {
        self.sample_format = Some(format.to_string());
        self
    }

    /// Sets the target bits per sample.
    #[allow(dead_code)]
    pub fn with_bits_per_sample(mut self, bits: u16) -> Self {
        self.bits_per_sample = Some(bits);
        self
    }

    /// Sets the CPAL stream buffer size.
    #[allow(dead_code)]
    pub fn with_stream_buffer_size(mut self, sbs: StreamBufferSize) -> Self {
        self.stream_buffer_size = Some(sbs);
        self
    }

    /// Sets the resampling algorithm.
    #[allow(dead_code)]
    pub fn with_resampler(mut self, resampler: ResamplerType) -> Self {
        self.resampler = Some(resampler);
        self
    }
}

#[cfg(test)]
mod test {
    use std::time::Duration;

    use super::*;

    #[test]
    fn defaults() {
        let audio = Audio::new("test-device");
        assert_eq!(audio.device(), "test-device");
        assert_eq!(audio.sample_rate(), 44100);
        assert_eq!(audio.bits_per_sample(), 32);
        assert_eq!(audio.sample_format().unwrap(), SampleFormat::Int);
        assert_eq!(audio.buffer_size(), DEFAULT_BUFFER_SIZE);
        assert_eq!(audio.buffer_threads(), DEFAULT_BUFFER_THREADS);
        assert_eq!(audio.playback_delay().unwrap(), Duration::ZERO);
        assert!(audio.stream_buffer_size().is_none());
        assert_eq!(audio.resampler(), ResamplerType::Sinc);
    }

    #[test]
    fn builder_sample_rate() {
        let audio = Audio::new("dev").with_sample_rate(48000);
        assert_eq!(audio.sample_rate(), 48000);
    }

    #[test]
    fn builder_buffer_size() {
        let audio = Audio::new("dev").with_buffer_size(2048);
        assert_eq!(audio.buffer_size(), 2048);
    }

    #[test]
    fn builder_bits_per_sample() {
        let audio = Audio::new("dev").with_bits_per_sample(16);
        assert_eq!(audio.bits_per_sample(), 16);
    }

    #[test]
    fn builder_sample_format_float() {
        let audio = Audio::new("dev").with_sample_format("float");
        assert_eq!(audio.sample_format().unwrap(), SampleFormat::Float);
    }

    #[test]
    fn builder_sample_format_int() {
        let audio = Audio::new("dev").with_sample_format("int");
        assert_eq!(audio.sample_format().unwrap(), SampleFormat::Int);
    }

    #[test]
    fn sample_format_invalid() {
        let audio = Audio::new("dev").with_sample_format("wav");
        assert!(audio.sample_format().is_err());
    }

    #[test]
    fn playback_delay_valid() {
        let audio = Audio {
            playback_delay: Some("500ms".to_string()),
            ..Audio::new("dev")
        };
        assert_eq!(audio.playback_delay().unwrap(), Duration::from_millis(500));
    }

    #[test]
    fn playback_delay_invalid() {
        let audio = Audio {
            playback_delay: Some("not-a-duration".to_string()),
            ..Audio::new("dev")
        };
        assert!(audio.playback_delay().is_err());
    }

    #[test]
    fn buffer_threads_clamped_to_one() {
        let audio = Audio {
            buffer_threads: Some(0),
            ..Audio::new("dev")
        };
        assert_eq!(audio.buffer_threads(), 1);
    }

    #[test]
    fn buffer_threads_custom() {
        let audio = Audio {
            buffer_threads: Some(4),
            ..Audio::new("dev")
        };
        assert_eq!(audio.buffer_threads(), 4);
    }

    #[test]
    fn builder_resampler_fft() {
        let audio = Audio::new("dev").with_resampler(ResamplerType::Fft);
        assert_eq!(audio.resampler(), ResamplerType::Fft);
    }

    #[test]
    fn builder_stream_buffer_size() {
        let audio = Audio::new("dev").with_stream_buffer_size(StreamBufferSize::Min);
        assert!(matches!(
            audio.stream_buffer_size(),
            Some(StreamBufferSize::Min)
        ));
    }

    #[test]
    fn builder_chaining() {
        let audio = Audio::new("dev")
            .with_sample_rate(96000)
            .with_buffer_size(512)
            .with_bits_per_sample(24)
            .with_sample_format("float")
            .with_resampler(ResamplerType::Fft);

        assert_eq!(audio.sample_rate(), 96000);
        assert_eq!(audio.buffer_size(), 512);
        assert_eq!(audio.bits_per_sample(), 24);
        assert_eq!(audio.sample_format().unwrap(), SampleFormat::Float);
        assert_eq!(audio.resampler(), ResamplerType::Fft);
    }

    fn from_yaml(yaml: &str) -> Audio {
        config::Config::builder()
            .add_source(config::File::from_str(yaml, config::FileFormat::Yaml))
            .build()
            .expect("build config")
            .try_deserialize::<Audio>()
            .expect("deserialize")
    }

    #[test]
    fn serde_defaults_from_minimal_yaml() {
        let audio = from_yaml("device: minimal-device\n");

        assert_eq!(audio.device(), "minimal-device");
        assert_eq!(audio.sample_rate(), 44100);
        assert_eq!(audio.bits_per_sample(), 32);
        assert_eq!(audio.buffer_size(), DEFAULT_BUFFER_SIZE);
        assert_eq!(audio.resampler(), ResamplerType::Sinc);
    }

    #[test]
    fn serde_full_yaml() {
        let audio = from_yaml(
            r#"
            device: my-device
            sample_rate: 48000
            buffer_size: 512
            bits_per_sample: 24
            sample_format: float
            resampler: fft
            buffer_threads: 4
            playback_delay: 100ms
            "#,
        );

        assert_eq!(audio.device(), "my-device");
        assert_eq!(audio.sample_rate(), 48000);
        assert_eq!(audio.buffer_size(), 512);
        assert_eq!(audio.bits_per_sample(), 24);
        assert_eq!(audio.sample_format().unwrap(), SampleFormat::Float);
        assert_eq!(audio.resampler(), ResamplerType::Fft);
        assert_eq!(audio.buffer_threads(), 4);
        assert_eq!(audio.playback_delay().unwrap(), Duration::from_millis(100));
    }

    #[test]
    fn serde_resampler_variants() {
        let audio = from_yaml("device: dev\nresampler: sinc\n");
        assert_eq!(audio.resampler(), ResamplerType::Sinc);

        let audio = from_yaml("device: dev\nresampler: fft\n");
        assert_eq!(audio.resampler(), ResamplerType::Fft);
    }
}
