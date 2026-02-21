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

//! Trigger configuration for audio and MIDI trigger inputs.
//!
//! Supports two input kinds:
//! - `kind: audio` — piezo drum triggers via audio input channels
//! - `kind: midi` — MIDI event triggers (replaces legacy `sample_triggers`)

use std::str::FromStr;

use serde::Deserialize;
use tracing::warn;

use super::samples::SampleTrigger;
use crate::audio::format::SampleFormat;

fn default_threshold() -> f32 {
    0.1
}

fn default_retrigger_time_ms() -> u32 {
    30
}

fn default_scan_time_ms() -> u32 {
    5
}

fn default_gain() -> f32 {
    1.0
}

fn default_fixed_velocity() -> u8 {
    127
}

fn default_noise_floor_decay_ms() -> u32 {
    200
}

/// Configuration for the trigger system.
#[derive(Deserialize, Clone, Debug)]
pub struct TriggerConfig {
    /// The audio input device name (matched against cpal input devices).
    /// Only required when audio inputs are configured.
    device: Option<String>,
    /// Optional sample rate override for the input stream.
    sample_rate: Option<u32>,
    /// Target sample format ("int" or "float"). Default: device native.
    sample_format: Option<String>,
    /// Target bits per sample (16 or 32). Default: device native.
    bits_per_sample: Option<u16>,
    /// Stream buffer size in frames. Default: device default.
    buffer_size: Option<usize>,
    /// Per-channel input configurations.
    #[serde(default)]
    inputs: Vec<TriggerInput>,
    /// Crosstalk suppression window in ms after any channel fires.
    /// Both `crosstalk_window_ms` and `crosstalk_threshold` must be set to enable suppression.
    crosstalk_window_ms: Option<u32>,
    /// Threshold multiplier during crosstalk suppression window.
    /// Both `crosstalk_window_ms` and `crosstalk_threshold` must be set to enable suppression.
    crosstalk_threshold: Option<f32>,
}

impl TriggerConfig {
    /// Returns the device name, if configured.
    pub fn device(&self) -> Option<&str> {
        self.device.as_deref()
    }

    /// Returns the optional sample rate.
    pub fn sample_rate(&self) -> Option<u32> {
        self.sample_rate
    }

    /// Returns the target sample format, if configured.
    pub fn sample_format(&self) -> Option<SampleFormat> {
        self.sample_format.as_deref().and_then(|s| {
            SampleFormat::from_str(s)
                .inspect_err(|_| {
                    warn!(
                        value = s,
                        "invalid sample_format, expected 'int' or 'float'"
                    )
                })
                .ok()
        })
    }

    /// Returns the target bits per sample, if configured.
    pub fn bits_per_sample(&self) -> Option<u16> {
        self.bits_per_sample
    }

    /// Returns the stream buffer size in frames, if configured.
    pub fn buffer_size(&self) -> Option<usize> {
        self.buffer_size
    }

    /// Returns the input configurations.
    pub fn inputs(&self) -> &[TriggerInput] {
        &self.inputs
    }

    /// Returns the crosstalk suppression window in ms, if configured.
    pub fn crosstalk_window_ms(&self) -> Option<u32> {
        self.crosstalk_window_ms
    }

    /// Returns the crosstalk threshold multiplier, if configured.
    pub fn crosstalk_threshold(&self) -> Option<f32> {
        self.crosstalk_threshold
    }

    /// Returns whether any audio inputs are configured.
    pub fn has_audio_inputs(&self) -> bool {
        self.inputs
            .iter()
            .any(|i| matches!(i, TriggerInput::Audio(_)))
    }

    /// Extracts MIDI inputs as `SampleTrigger` entries for the sample engine.
    pub fn midi_triggers(&self) -> Vec<SampleTrigger> {
        self.inputs
            .iter()
            .filter_map(|i| match i {
                TriggerInput::Midi(midi) => Some(SampleTrigger::new(
                    midi.event().clone(),
                    midi.sample().to_string(),
                )),
                _ => None,
            })
            .collect()
    }

    /// Adds an input to this config.
    pub fn add_input(&mut self, input: TriggerInput) {
        self.inputs.push(input);
    }

    /// Creates an empty TriggerConfig with no device (MIDI-only).
    pub(crate) fn new_midi_only(inputs: Vec<TriggerInput>) -> Self {
        Self {
            device: None,
            sample_rate: None,
            sample_format: None,
            bits_per_sample: None,
            buffer_size: None,
            inputs,
            crosstalk_window_ms: None,
            crosstalk_threshold: None,
        }
    }
}

#[cfg(test)]
impl TriggerConfig {
    /// Creates a new TriggerConfig (test only).
    pub(crate) fn new(
        device: Option<&str>,
        sample_rate: Option<u32>,
        inputs: Vec<TriggerInput>,
    ) -> Self {
        Self {
            device: device.map(|d| d.to_string()),
            sample_rate,
            sample_format: None,
            bits_per_sample: None,
            buffer_size: None,
            inputs,
            crosstalk_window_ms: None,
            crosstalk_threshold: None,
        }
    }
}

/// A trigger input, discriminated by `kind`.
#[derive(Deserialize, Clone, Debug)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TriggerInput {
    /// Audio trigger input (piezo, line-level, etc.).
    Audio(AudioTriggerInput),
    /// MIDI event trigger input.
    Midi(MidiTriggerInput),
}

/// What action an input channel performs when triggered.
#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TriggerInputAction {
    /// Fire a sample (default).
    #[default]
    Trigger,
    /// Release voices in the named release group.
    Release,
}

/// How amplitude maps to velocity (0-127).
#[derive(Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VelocityCurve {
    /// Linear mapping: velocity = peak * 127.
    #[default]
    Linear,
    /// Logarithmic mapping: threshold→1.0 maps log-scaled to 1→127.
    Logarithmic,
    /// Fixed velocity: always returns the configured fixed_velocity.
    Fixed,
}

/// Configuration for a single audio input channel used as a trigger.
#[derive(Deserialize, Clone, Debug)]
pub struct AudioTriggerInput {
    /// 1-indexed input channel number.
    channel: u16,
    /// Sample name to trigger (from samples config). Required when action is "trigger".
    sample: Option<String>,
    /// Minimum amplitude to trigger (0.0-1.0).
    #[serde(default = "default_threshold")]
    threshold: f32,
    /// Lockout period after trigger fires, in milliseconds.
    #[serde(default = "default_retrigger_time_ms")]
    retrigger_time_ms: u32,
    /// Peak detection window after threshold crossing, in milliseconds.
    #[serde(default = "default_scan_time_ms")]
    scan_time_ms: u32,
    /// Input gain multiplier.
    #[serde(default = "default_gain")]
    gain: f32,
    /// How amplitude maps to velocity.
    #[serde(default)]
    velocity_curve: VelocityCurve,
    /// Velocity value when velocity_curve is "fixed".
    #[serde(default = "default_fixed_velocity")]
    fixed_velocity: u8,
    /// Optional release group name for voice management.
    release_group: Option<String>,
    /// What action this input performs (trigger or release).
    #[serde(default)]
    action: TriggerInputAction,
    /// High-pass filter cutoff frequency in Hz.
    highpass_freq: Option<f32>,
    /// Decay time for adaptive/dynamic threshold in ms.
    dynamic_threshold_decay_ms: Option<u32>,
    /// Noise floor tracking sensitivity multiplier. The adaptive noise floor
    /// estimate is multiplied by this value to derive a minimum threshold.
    /// Default: None (disabled). Typical value: 5.0.
    noise_floor_sensitivity: Option<f32>,
    /// Time constant for noise floor EMA in ms. Controls how quickly the
    /// estimate tracks changing ambient levels.
    #[serde(default = "default_noise_floor_decay_ms")]
    noise_floor_decay_ms: u32,
}

impl AudioTriggerInput {
    /// Returns the 1-indexed channel number.
    pub fn channel(&self) -> u16 {
        self.channel
    }

    /// Returns the sample name, if configured.
    pub fn sample(&self) -> Option<&str> {
        self.sample.as_deref()
    }

    /// Returns the amplitude threshold.
    pub fn threshold(&self) -> f32 {
        self.threshold
    }

    /// Returns the retrigger lockout time in milliseconds.
    pub fn retrigger_time_ms(&self) -> u32 {
        self.retrigger_time_ms
    }

    /// Returns the scan time (peak detection window) in milliseconds.
    pub fn scan_time_ms(&self) -> u32 {
        self.scan_time_ms
    }

    /// Returns the input gain multiplier.
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Returns the velocity curve type.
    pub fn velocity_curve(&self) -> VelocityCurve {
        self.velocity_curve
    }

    /// Returns the fixed velocity value.
    pub fn fixed_velocity(&self) -> u8 {
        self.fixed_velocity
    }

    /// Returns the release group name, if configured.
    pub fn release_group(&self) -> Option<&str> {
        self.release_group.as_deref()
    }

    /// Returns the action this input performs.
    pub fn action(&self) -> TriggerInputAction {
        self.action
    }

    /// Returns the high-pass filter cutoff frequency in Hz, if configured.
    pub fn highpass_freq(&self) -> Option<f32> {
        self.highpass_freq
    }

    /// Returns the dynamic threshold decay time in ms, if configured.
    pub fn dynamic_threshold_decay_ms(&self) -> Option<u32> {
        self.dynamic_threshold_decay_ms
    }

    /// Returns the noise floor tracking sensitivity multiplier, if configured.
    pub fn noise_floor_sensitivity(&self) -> Option<f32> {
        self.noise_floor_sensitivity
    }

    /// Returns the noise floor EMA decay time in ms.
    pub fn noise_floor_decay_ms(&self) -> u32 {
        self.noise_floor_decay_ms
    }
}

#[cfg(test)]
impl AudioTriggerInput {
    /// Creates a new trigger input for a sample trigger (test only).
    pub(crate) fn new_trigger(channel: u16, sample: &str, release_group: Option<&str>) -> Self {
        Self {
            channel,
            sample: Some(sample.to_string()),
            threshold: default_threshold(),
            retrigger_time_ms: default_retrigger_time_ms(),
            scan_time_ms: default_scan_time_ms(),
            gain: default_gain(),
            velocity_curve: VelocityCurve::default(),
            fixed_velocity: default_fixed_velocity(),
            release_group: release_group.map(|s| s.to_string()),
            action: TriggerInputAction::Trigger,
            highpass_freq: None,
            dynamic_threshold_decay_ms: None,
            noise_floor_sensitivity: None,
            noise_floor_decay_ms: default_noise_floor_decay_ms(),
        }
    }

    /// Creates a new trigger input for a release action (test only).
    pub(crate) fn new_release(channel: u16, release_group: &str) -> Self {
        Self {
            channel,
            sample: None,
            threshold: 0.05,
            retrigger_time_ms: default_retrigger_time_ms(),
            scan_time_ms: default_scan_time_ms(),
            gain: default_gain(),
            velocity_curve: VelocityCurve::default(),
            fixed_velocity: default_fixed_velocity(),
            release_group: Some(release_group.to_string()),
            action: TriggerInputAction::Release,
            highpass_freq: None,
            dynamic_threshold_decay_ms: None,
            noise_floor_sensitivity: None,
            noise_floor_decay_ms: default_noise_floor_decay_ms(),
        }
    }
}

/// Configuration for a MIDI event trigger input.
#[derive(Deserialize, Clone, Debug)]
pub struct MidiTriggerInput {
    /// The MIDI event to listen for.
    event: super::midi::Event,
    /// The sample name to trigger.
    sample: String,
}

impl MidiTriggerInput {
    /// Returns the MIDI event.
    pub fn event(&self) -> &super::midi::Event {
        &self.event
    }

    /// Returns the sample name.
    pub fn sample(&self) -> &str {
        &self.sample
    }

    /// Creates a new MidiTriggerInput.
    pub fn new(event: super::midi::Event, sample: String) -> Self {
        Self { event, sample }
    }
}

#[cfg(test)]
mod tests {
    use config::{Config, File, FileFormat};

    use super::*;

    fn unwrap_audio(input: &TriggerInput) -> &AudioTriggerInput {
        match input {
            TriggerInput::Audio(audio) => audio,
            _ => panic!("Expected TriggerInput::Audio"),
        }
    }

    #[test]
    fn test_trigger_config_deserialize() {
        let yaml = r#"
            device: "UltraLite-mk5"
            sample_rate: 44100
            inputs:
              - kind: audio
                channel: 1
                sample: "kick"
                threshold: 0.1
                retrigger_time_ms: 30
                scan_time_ms: 5
                gain: 1.0
                velocity_curve: linear
                release_group: "kick"
              - kind: audio
                channel: 3
                sample: "cymbal"
                threshold: 0.08
                release_group: "cymbal"
              - kind: audio
                channel: 4
                action: release
                release_group: "cymbal"
                threshold: 0.05
        "#;

        let config: TriggerConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(config.device(), Some("UltraLite-mk5"));
        assert_eq!(config.sample_rate(), Some(44100));
        assert_eq!(config.inputs().len(), 3);

        let input0 = unwrap_audio(&config.inputs()[0]);
        assert_eq!(input0.channel(), 1);
        assert_eq!(input0.sample(), Some("kick"));
        assert!((input0.threshold() - 0.1).abs() < 0.001);
        assert_eq!(input0.retrigger_time_ms(), 30);
        assert_eq!(input0.scan_time_ms(), 5);
        assert!((input0.gain() - 1.0).abs() < 0.001);
        assert_eq!(input0.velocity_curve(), VelocityCurve::Linear);
        assert_eq!(input0.release_group(), Some("kick"));
        assert_eq!(input0.action(), TriggerInputAction::Trigger);

        let input1 = unwrap_audio(&config.inputs()[1]);
        assert_eq!(input1.channel(), 3);
        assert_eq!(input1.sample(), Some("cymbal"));
        assert_eq!(input1.release_group(), Some("cymbal"));
        assert_eq!(input1.action(), TriggerInputAction::Trigger);

        let input2 = unwrap_audio(&config.inputs()[2]);
        assert_eq!(input2.channel(), 4);
        assert_eq!(input2.sample(), None);
        assert_eq!(input2.release_group(), Some("cymbal"));
        assert_eq!(input2.action(), TriggerInputAction::Release);
    }

    #[test]
    fn test_trigger_config_defaults() {
        let yaml = r#"
            device: "test-device"
            inputs:
              - kind: audio
                channel: 1
                sample: "kick"
        "#;

        let config: TriggerConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(config.device(), Some("test-device"));
        assert_eq!(config.sample_rate(), None);

        let input = unwrap_audio(&config.inputs()[0]);
        assert!((input.threshold() - 0.1).abs() < 0.001);
        assert_eq!(input.retrigger_time_ms(), 30);
        assert_eq!(input.scan_time_ms(), 5);
        assert!((input.gain() - 1.0).abs() < 0.001);
        assert_eq!(input.velocity_curve(), VelocityCurve::Linear);
        assert_eq!(input.fixed_velocity(), 127);
        assert_eq!(input.release_group(), None);
        assert_eq!(input.action(), TriggerInputAction::Trigger);
    }

    #[test]
    fn test_trigger_config_audio_knobs() {
        let yaml = r#"
            device: "UltraLite-mk5"
            sample_format: int
            bits_per_sample: 16
            buffer_size: 512
            inputs:
              - kind: audio
                channel: 1
                sample: "kick"
        "#;

        let config: TriggerConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(config.sample_format(), Some(SampleFormat::Int));
        assert_eq!(config.bits_per_sample(), Some(16));
        assert_eq!(config.buffer_size(), Some(512));

        // Float format.
        let yaml_float = r#"
            device: "test"
            sample_format: float
            bits_per_sample: 32
            inputs: []
        "#;

        let config: TriggerConfig = Config::builder()
            .add_source(File::from_str(yaml_float, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(config.sample_format(), Some(SampleFormat::Float));
        assert_eq!(config.bits_per_sample(), Some(32));
        assert_eq!(config.buffer_size(), None);
    }

    #[test]
    fn test_signal_conditioning_fields_deserialize() {
        let yaml = r#"
            device: "UltraLite-mk5"
            crosstalk_window_ms: 4
            crosstalk_threshold: 3.0
            inputs:
              - kind: audio
                channel: 1
                sample: "kick"
                highpass_freq: 80.0
                dynamic_threshold_decay_ms: 50
              - kind: audio
                channel: 2
                sample: "snare"
        "#;

        let config: TriggerConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(config.crosstalk_window_ms(), Some(4));
        assert!((config.crosstalk_threshold().unwrap() - 3.0).abs() < 0.001);

        let input0 = unwrap_audio(&config.inputs()[0]);
        assert!((input0.highpass_freq().unwrap() - 80.0).abs() < 0.001);
        assert_eq!(input0.dynamic_threshold_decay_ms(), Some(50));

        // Second input should default to None for all new fields.
        let input1 = unwrap_audio(&config.inputs()[1]);
        assert_eq!(input1.highpass_freq(), None);
        assert_eq!(input1.dynamic_threshold_decay_ms(), None);
    }

    #[test]
    fn test_signal_conditioning_defaults_to_none() {
        let yaml = r#"
            device: "test-device"
            inputs:
              - kind: audio
                channel: 1
                sample: "kick"
        "#;

        let config: TriggerConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(config.crosstalk_window_ms(), None);
        assert_eq!(config.crosstalk_threshold(), None);
        assert_eq!(unwrap_audio(&config.inputs()[0]).highpass_freq(), None);
        assert_eq!(
            unwrap_audio(&config.inputs()[0]).dynamic_threshold_decay_ms(),
            None
        );
    }

    #[test]
    fn test_velocity_curves_deserialize() {
        let yaml = r#"
            device: "test"
            inputs:
              - kind: audio
                channel: 1
                sample: "a"
                velocity_curve: linear
              - kind: audio
                channel: 2
                sample: "b"
                velocity_curve: logarithmic
              - kind: audio
                channel: 3
                sample: "c"
                velocity_curve: fixed
                fixed_velocity: 100
        "#;

        let config: TriggerConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(
            unwrap_audio(&config.inputs()[0]).velocity_curve(),
            VelocityCurve::Linear
        );
        assert_eq!(
            unwrap_audio(&config.inputs()[1]).velocity_curve(),
            VelocityCurve::Logarithmic
        );
        assert_eq!(
            unwrap_audio(&config.inputs()[2]).velocity_curve(),
            VelocityCurve::Fixed
        );
        assert_eq!(unwrap_audio(&config.inputs()[2]).fixed_velocity(), 100);
    }

    #[test]
    fn test_midi_trigger_input_deserialize() {
        let yaml = r#"
            inputs:
              - kind: midi
                event:
                  type: note_on
                  channel: 10
                  key: 60
                sample: kick
        "#;

        let config: TriggerConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(config.device(), None);
        assert_eq!(config.inputs().len(), 1);
        assert!(!config.has_audio_inputs());
        assert_eq!(config.midi_triggers().len(), 1);
        assert_eq!(config.midi_triggers()[0].sample(), "kick");
    }

    #[test]
    fn test_mixed_audio_and_midi_inputs() {
        let yaml = r#"
            device: "UltraLite-mk5"
            inputs:
              - kind: audio
                channel: 1
                sample: "kick"
              - kind: midi
                event:
                  type: note_on
                  channel: 10
                  key: 60
                sample: snare
        "#;

        let config: TriggerConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(config.inputs().len(), 2);
        assert!(config.has_audio_inputs());
        assert_eq!(config.midi_triggers().len(), 1);
        assert_eq!(config.midi_triggers()[0].sample(), "snare");
    }

    #[test]
    fn test_device_optional_for_midi_only() {
        let yaml = r#"
            inputs:
              - kind: midi
                event:
                  type: note_on
                  channel: 10
                  key: 60
                sample: kick
        "#;

        let config: TriggerConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(config.device(), None);
        assert!(!config.has_audio_inputs());
    }
}
