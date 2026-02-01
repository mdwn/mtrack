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
use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use super::midi;

/// Default maximum number of concurrent sample voices globally.
pub const DEFAULT_MAX_SAMPLE_VOICES: u32 = 32;

/// Default velocity value when velocity mode is set to ignore.
pub const DEFAULT_VELOCITY: u8 = 100;

/// A YAML representation of a sample definition.
#[derive(Deserialize, Clone, Serialize, Debug)]
pub struct SampleDefinition {
    /// The audio file for this sample (used when velocity mode is not "layers").
    file: Option<String>,

    /// The output channels to route this sample to (1-indexed).
    output_channels: Vec<u16>,

    /// Velocity handling configuration.
    #[serde(default)]
    velocity: VelocityConfig,

    /// Behavior when a Note Off event is received.
    #[serde(default)]
    note_off: NoteOffBehavior,

    /// Behavior when the sample is retriggered while still playing.
    #[serde(default)]
    retrigger: RetriggerBehavior,

    /// Maximum number of concurrent voices for this sample.
    /// If not set, only the global limit applies.
    max_voices: Option<u32>,

    /// Fade time in milliseconds for note_off: fade behavior.
    #[serde(default = "default_fade_time_ms")]
    fade_time_ms: u32,
}

fn default_fade_time_ms() -> u32 {
    50
}

impl SampleDefinition {
    /// Gets the output channels for this sample.
    pub fn output_channels(&self) -> &[u16] {
        &self.output_channels
    }

    /// Gets the note-off behavior.
    pub fn note_off(&self) -> NoteOffBehavior {
        self.note_off
    }

    /// Gets the retrigger behavior.
    pub fn retrigger(&self) -> RetriggerBehavior {
        self.retrigger
    }

    /// Gets the maximum voices for this sample.
    pub fn max_voices(&self) -> Option<u32> {
        self.max_voices
    }

    /// Gets the fade time in milliseconds.
    /// Note: Fade behavior is not yet implemented; this config option is reserved for future use.
    #[allow(dead_code)]
    pub fn fade_time_ms(&self) -> u32 {
        self.fade_time_ms
    }

    /// Gets the file to play for a given velocity value.
    /// Returns the file path and the volume scale factor (0.0 to 1.0).
    pub fn file_for_velocity(&self, velocity: u8) -> Option<(&str, f32)> {
        match &self.velocity.mode {
            VelocityMode::Ignore => {
                let volume = self.velocity.default.unwrap_or(DEFAULT_VELOCITY) as f32 / 127.0;
                self.file.as_deref().map(|f| (f, volume))
            }
            VelocityMode::Scale => {
                let volume = velocity as f32 / 127.0;
                self.file.as_deref().map(|f| (f, volume))
            }
            VelocityMode::Layers => {
                // Find the layer that matches this velocity
                for layer in &self.velocity.layers {
                    if velocity >= layer.range[0] && velocity <= layer.range[1] {
                        let volume = if self.velocity.scale.unwrap_or(false) {
                            velocity as f32 / 127.0
                        } else {
                            1.0
                        };
                        return Some((&layer.file, volume));
                    }
                }
                None
            }
        }
    }

    /// Gets all files referenced by this sample definition (for preloading).
    pub fn all_files(&self) -> Vec<&str> {
        let mut files = Vec::new();
        if let Some(file) = &self.file {
            files.push(file.as_str());
        }
        for layer in &self.velocity.layers {
            files.push(&layer.file);
        }
        files
    }
}

#[cfg(test)]
impl SampleDefinition {
    /// Creates a new sample definition (test only).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        file: Option<String>,
        output_channels: Vec<u16>,
        velocity: VelocityConfig,
        note_off: NoteOffBehavior,
        retrigger: RetriggerBehavior,
        max_voices: Option<u32>,
        fade_time_ms: u32,
    ) -> Self {
        Self {
            file,
            output_channels,
            velocity,
            note_off,
            retrigger,
            max_voices,
            fade_time_ms,
        }
    }

    /// Gets the audio file path (test only).
    pub fn file(&self) -> Option<&str> {
        self.file.as_deref()
    }

    /// Gets the velocity configuration (test only).
    pub fn velocity(&self) -> &VelocityConfig {
        &self.velocity
    }
}

/// Configuration for velocity handling.
#[derive(Deserialize, Clone, Serialize, Debug, Default)]
pub struct VelocityConfig {
    /// The velocity handling mode.
    #[serde(default)]
    mode: VelocityMode,

    /// Default velocity value when mode is "ignore".
    default: Option<u8>,

    /// Whether to also scale volume by velocity when using layers.
    scale: Option<bool>,

    /// Velocity layers (used when mode is "layers").
    #[serde(default)]
    layers: Vec<VelocityLayer>,
}

#[cfg(test)]
impl VelocityConfig {
    /// Creates a new velocity config with ignore mode (test only).
    pub fn ignore(default: Option<u8>) -> Self {
        Self {
            mode: VelocityMode::Ignore,
            default,
            scale: None,
            layers: Vec::new(),
        }
    }

    /// Creates a new velocity config with scale mode (test only).
    pub fn scale() -> Self {
        Self {
            mode: VelocityMode::Scale,
            default: None,
            scale: None,
            layers: Vec::new(),
        }
    }

    /// Creates a new velocity config with layers mode (test only).
    pub fn with_layers(layers: Vec<VelocityLayer>, scale: bool) -> Self {
        Self {
            mode: VelocityMode::Layers,
            default: None,
            scale: Some(scale),
            layers,
        }
    }

    /// Gets the velocity mode (test only).
    pub fn mode(&self) -> &VelocityMode {
        &self.mode
    }

    /// Gets the default velocity (test only).
    pub fn default(&self) -> Option<u8> {
        self.default
    }

    /// Gets whether to scale volume in layers mode (test only).
    pub fn scale_enabled(&self) -> bool {
        self.scale.unwrap_or(false)
    }

    /// Gets the velocity layers (test only).
    pub fn layers(&self) -> &[VelocityLayer] {
        &self.layers
    }
}

/// Velocity handling mode.
#[derive(Deserialize, Clone, Copy, Serialize, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VelocityMode {
    /// Ignore velocity, play at default volume.
    #[default]
    Ignore,
    /// Scale volume by velocity (velocity/127).
    Scale,
    /// Select different sample files based on velocity ranges.
    Layers,
}

/// A velocity layer that maps a velocity range to a sample file.
#[derive(Deserialize, Clone, Serialize, Debug)]
pub struct VelocityLayer {
    /// The velocity range [min, max] inclusive (0-127).
    range: [u8; 2],

    /// The audio file for this velocity layer.
    file: String,
}

#[cfg(test)]
impl VelocityLayer {
    /// Creates a new velocity layer (test only).
    pub fn new(range: [u8; 2], file: String) -> Self {
        Self { range, file }
    }

    /// Gets the velocity range (test only).
    pub fn range(&self) -> [u8; 2] {
        self.range
    }

    /// Gets the file for this layer (test only).
    pub fn file(&self) -> &str {
        &self.file
    }
}

/// Behavior when a Note Off event is received for a playing sample.
#[derive(Deserialize, Clone, Copy, Serialize, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NoteOffBehavior {
    /// Let the sample play to completion, ignoring Note Off.
    #[default]
    PlayToCompletion,
    /// Immediately stop the sample on Note Off.
    Stop,
    /// Fade out the sample over a short duration on Note Off.
    Fade,
}

/// Behavior when a sample is triggered while it's already playing.
#[derive(Deserialize, Clone, Copy, Serialize, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RetriggerBehavior {
    /// Stop the previous voice and start a new one.
    #[default]
    Cut,
    /// Allow multiple voices to play simultaneously.
    Polyphonic,
}

/// A trigger that maps a MIDI event to a sample.
#[derive(Deserialize, Clone, Serialize, Debug)]
pub struct SampleTrigger {
    /// The MIDI event that triggers the sample.
    trigger: midi::Event,

    /// The name of the sample to trigger (references a SampleDefinition).
    sample: String,
}

impl SampleTrigger {
    /// Gets the MIDI event that triggers the sample.
    pub fn trigger(&self) -> &midi::Event {
        &self.trigger
    }

    /// Gets the name of the sample to trigger.
    pub fn sample(&self) -> &str {
        &self.sample
    }
}

#[cfg(test)]
impl SampleTrigger {
    /// Creates a new sample trigger (test only).
    pub fn new(trigger: midi::Event, sample: String) -> Self {
        Self { trigger, sample }
    }
}

/// Global samples configuration that can be embedded in player config or loaded from a file.
#[derive(Deserialize, Clone, Serialize, Debug, Default)]
pub struct SamplesConfig {
    /// Sample definitions by name.
    #[serde(default)]
    samples: HashMap<String, SampleDefinition>,

    /// Sample triggers.
    #[serde(default)]
    sample_triggers: Vec<SampleTrigger>,

    /// Maximum number of concurrent sample voices globally.
    #[serde(default = "default_max_sample_voices")]
    max_sample_voices: u32,
}

fn default_max_sample_voices() -> u32 {
    DEFAULT_MAX_SAMPLE_VOICES
}

impl SamplesConfig {
    /// Creates a new samples configuration.
    pub fn new(
        samples: HashMap<String, SampleDefinition>,
        sample_triggers: Vec<SampleTrigger>,
        max_sample_voices: u32,
    ) -> Self {
        Self {
            samples,
            sample_triggers,
            max_sample_voices,
        }
    }

    /// Gets the sample definitions.
    pub fn samples(&self) -> &HashMap<String, SampleDefinition> {
        &self.samples
    }

    /// Gets the sample triggers.
    pub fn sample_triggers(&self) -> &[SampleTrigger] {
        &self.sample_triggers
    }

    /// Merges another config into this one. The other config's values override.
    pub fn merge(&mut self, other: SamplesConfig) {
        // Merge sample definitions (other overrides)
        for (name, definition) in other.samples {
            self.samples.insert(name, definition);
        }

        // Merge triggers - other's triggers override matching ones
        // A trigger matches if it has the same MIDI event
        for other_trigger in other.sample_triggers {
            // Remove any existing trigger with the same MIDI event
            self.sample_triggers
                .retain(|t| t.trigger != other_trigger.trigger);
            self.sample_triggers.push(other_trigger);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_velocity_ignore() {
        let def = SampleDefinition::new(
            Some("test.wav".to_string()),
            vec![1, 2],
            VelocityConfig::ignore(Some(100)),
            NoteOffBehavior::PlayToCompletion,
            RetriggerBehavior::Cut,
            None,
            50,
        );

        let (file, volume) = def.file_for_velocity(50).unwrap();
        assert_eq!(file, "test.wav");
        assert!((volume - 100.0 / 127.0).abs() < 0.001);

        // Velocity value doesn't matter in ignore mode
        let (_, volume2) = def.file_for_velocity(127).unwrap();
        assert!((volume - volume2).abs() < 0.001);
    }

    #[test]
    fn test_velocity_scale() {
        let def = SampleDefinition::new(
            Some("test.wav".to_string()),
            vec![1, 2],
            VelocityConfig::scale(),
            NoteOffBehavior::PlayToCompletion,
            RetriggerBehavior::Cut,
            None,
            50,
        );

        let (file, volume) = def.file_for_velocity(64).unwrap();
        assert_eq!(file, "test.wav");
        assert!((volume - 64.0 / 127.0).abs() < 0.001);

        let (_, volume2) = def.file_for_velocity(127).unwrap();
        assert!((volume2 - 1.0).abs() < 0.001);
    }

    #[test]
    fn test_velocity_layers() {
        let layers = vec![
            VelocityLayer::new([1, 60], "soft.wav".to_string()),
            VelocityLayer::new([61, 100], "medium.wav".to_string()),
            VelocityLayer::new([101, 127], "hard.wav".to_string()),
        ];

        let def = SampleDefinition::new(
            None,
            vec![1, 2],
            VelocityConfig::with_layers(layers, false),
            NoteOffBehavior::PlayToCompletion,
            RetriggerBehavior::Polyphonic,
            Some(4),
            50,
        );

        let (file, volume) = def.file_for_velocity(45).unwrap();
        assert_eq!(file, "soft.wav");
        assert!((volume - 1.0).abs() < 0.001); // No scaling

        let (file, _) = def.file_for_velocity(80).unwrap();
        assert_eq!(file, "medium.wav");

        let (file, _) = def.file_for_velocity(120).unwrap();
        assert_eq!(file, "hard.wav");
    }

    #[test]
    fn test_velocity_layers_with_scale() {
        let layers = vec![
            VelocityLayer::new([1, 60], "soft.wav".to_string()),
            VelocityLayer::new([61, 127], "hard.wav".to_string()),
        ];

        let def = SampleDefinition::new(
            None,
            vec![1, 2],
            VelocityConfig::with_layers(layers, true), // Scale enabled
            NoteOffBehavior::PlayToCompletion,
            RetriggerBehavior::Polyphonic,
            None,
            50,
        );

        let (file, volume) = def.file_for_velocity(45).unwrap();
        assert_eq!(file, "soft.wav");
        assert!((volume - 45.0 / 127.0).abs() < 0.001); // Scaled to full range

        let (file, volume) = def.file_for_velocity(100).unwrap();
        assert_eq!(file, "hard.wav");
        assert!((volume - 100.0 / 127.0).abs() < 0.001);
    }

    #[test]
    fn test_all_files() {
        let layers = vec![
            VelocityLayer::new([1, 60], "soft.wav".to_string()),
            VelocityLayer::new([61, 127], "hard.wav".to_string()),
        ];

        let def = SampleDefinition::new(
            Some("default.wav".to_string()),
            vec![1, 2],
            VelocityConfig::with_layers(layers, false),
            NoteOffBehavior::PlayToCompletion,
            RetriggerBehavior::Cut,
            None,
            50,
        );

        let files = def.all_files();
        assert_eq!(files.len(), 3);
        assert!(files.contains(&"default.wav"));
        assert!(files.contains(&"soft.wav"));
        assert!(files.contains(&"hard.wav"));
    }

    #[test]
    fn test_merge_configs() {
        let mut config1 = SamplesConfig::new(
            HashMap::from([
                (
                    "kick".to_string(),
                    SampleDefinition::new(
                        Some("kick1.wav".to_string()),
                        vec![1],
                        VelocityConfig::ignore(None),
                        NoteOffBehavior::PlayToCompletion,
                        RetriggerBehavior::Cut,
                        None,
                        50,
                    ),
                ),
                (
                    "snare".to_string(),
                    SampleDefinition::new(
                        Some("snare1.wav".to_string()),
                        vec![2],
                        VelocityConfig::ignore(None),
                        NoteOffBehavior::PlayToCompletion,
                        RetriggerBehavior::Cut,
                        None,
                        50,
                    ),
                ),
            ]),
            vec![],
            32,
        );

        let config2 = SamplesConfig::new(
            HashMap::from([(
                "kick".to_string(),
                SampleDefinition::new(
                    Some("kick2.wav".to_string()), // Override kick
                    vec![1, 2],
                    VelocityConfig::scale(),
                    NoteOffBehavior::Stop,
                    RetriggerBehavior::Polyphonic,
                    Some(4),
                    100,
                ),
            )]),
            vec![],
            32,
        );

        config1.merge(config2);

        // Kick should be overridden
        assert_eq!(
            config1.samples.get("kick").unwrap().file(),
            Some("kick2.wav")
        );
        // Snare should remain
        assert_eq!(
            config1.samples.get("snare").unwrap().file(),
            Some("snare1.wav")
        );
    }
}
