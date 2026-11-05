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
pub use super::samples_data::*;
use std::collections::HashMap;

use super::midi;

/// Default velocity value when velocity mode is set to ignore.
pub const DEFAULT_VELOCITY: u8 = 100;

impl SampleDefinition {
    /// Gets the output channels for this sample.
    pub fn output_channels(&self) -> &[u16] {
        &self.output_channels
    }

    /// Gets the output track name for profile-based routing.
    pub fn output_track(&self) -> Option<&str> {
        self.output_track.as_deref()
    }

    /// Gets the release behavior.
    pub fn release_behavior(&self) -> ReleaseBehavior {
        self.release_behavior
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
        release_behavior: ReleaseBehavior,
        retrigger: RetriggerBehavior,
        max_voices: Option<u32>,
        fade_time_ms: u32,
    ) -> Self {
        Self {
            file,
            output_channels,
            output_track: None,
            velocity,
            release_behavior,
            retrigger,
            max_voices,
            fade_time_ms,
        }
    }

    /// Gets the audio file path (test only).
    pub fn file(&self) -> Option<&str> {
        self.file.as_deref()
    }
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
}

#[cfg(test)]
impl VelocityLayer {
    /// Creates a new velocity layer (test only).
    pub fn new(range: [u8; 2], file: String) -> Self {
        Self { range, file }
    }
}

impl SampleTrigger {
    /// Creates a new SampleTrigger.
    pub fn new(trigger: midi::Event, sample: String) -> Self {
        Self { trigger, sample }
    }

    /// Gets the MIDI event that triggers the sample.
    pub fn trigger(&self) -> &midi::Event {
        &self.trigger
    }

    /// Gets the name of the sample to trigger.
    pub fn sample(&self) -> &str {
        &self.sample
    }
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

    /// Adds MIDI triggers, replacing any existing triggers with the same MIDI event.
    pub fn add_triggers(&mut self, triggers: Vec<SampleTrigger>) {
        for trigger in triggers {
            self.sample_triggers
                .retain(|t| t.trigger != trigger.trigger);
            self.sample_triggers.push(trigger);
        }
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
    use config::{Config, File, FileFormat};

    use super::*;

    #[test]
    fn test_velocity_ignore() {
        let def = SampleDefinition::new(
            Some("test.wav".to_string()),
            vec![1, 2],
            VelocityConfig::ignore(Some(100)),
            ReleaseBehavior::PlayToCompletion,
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
            ReleaseBehavior::PlayToCompletion,
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
            ReleaseBehavior::PlayToCompletion,
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
            ReleaseBehavior::PlayToCompletion,
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
            ReleaseBehavior::PlayToCompletion,
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
                        ReleaseBehavior::PlayToCompletion,
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
                        ReleaseBehavior::PlayToCompletion,
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
                    ReleaseBehavior::Stop,
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

    #[test]
    fn test_release_behavior_yaml_keys() {
        // The new "release_behavior" key should work
        let yaml = r#"
            samples:
              kick:
                file: kick.wav
                output_channels: [1]
                release_behavior: stop
        "#;
        let config: SamplesConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(
            config.samples.get("kick").unwrap().release_behavior(),
            ReleaseBehavior::Stop,
        );

        // The legacy "note_off" key should also work
        let yaml = r#"
            samples:
              kick:
                file: kick.wav
                output_channels: [1]
                note_off: fade
        "#;
        let config: SamplesConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(
            config.samples.get("kick").unwrap().release_behavior(),
            ReleaseBehavior::Fade,
        );
    }

    #[test]
    fn test_output_track_deserialization() {
        // output_track should deserialize correctly
        let yaml = r#"
            samples:
              kick:
                file: kick.wav
                output_track: kick-out
        "#;
        let config: SamplesConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        let kick = config.samples.get("kick").unwrap();
        assert_eq!(kick.output_track(), Some("kick-out"));
        assert!(kick.output_channels().is_empty());

        // output_channels without output_track should still work
        let yaml = r#"
            samples:
              snare:
                file: snare.wav
                output_channels: [3, 4]
        "#;
        let config: SamplesConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        let snare = config.samples.get("snare").unwrap();
        assert_eq!(snare.output_track(), None);
        assert_eq!(snare.output_channels(), &[3, 4]);

        // Both set: output_track should be present, output_channels also present
        let yaml = r#"
            samples:
              both:
                file: both.wav
                output_track: both-out
                output_channels: [5, 6]
        "#;
        let config: SamplesConfig = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        let both = config.samples.get("both").unwrap();
        assert_eq!(both.output_track(), Some("both-out"));
        assert_eq!(both.output_channels(), &[5, 6]);
    }

    #[test]
    fn test_retrigger_getter() {
        let def = SampleDefinition::new(
            Some("test.wav".to_string()),
            vec![1],
            VelocityConfig::ignore(None),
            ReleaseBehavior::PlayToCompletion,
            RetriggerBehavior::Polyphonic,
            None,
            50,
        );
        assert_eq!(def.retrigger(), RetriggerBehavior::Polyphonic);

        let def2 = SampleDefinition::new(
            Some("test.wav".to_string()),
            vec![1],
            VelocityConfig::ignore(None),
            ReleaseBehavior::PlayToCompletion,
            RetriggerBehavior::Cut,
            None,
            50,
        );
        assert_eq!(def2.retrigger(), RetriggerBehavior::Cut);
    }

    #[test]
    fn test_fade_time_ms_getter() {
        let def = SampleDefinition::new(
            Some("test.wav".to_string()),
            vec![1],
            VelocityConfig::ignore(None),
            ReleaseBehavior::Fade,
            RetriggerBehavior::Cut,
            None,
            200,
        );
        assert_eq!(def.fade_time_ms(), 200);
    }

    #[test]
    fn test_velocity_layers_no_match_returns_none() {
        let layers = vec![VelocityLayer::new([10, 50], "mid.wav".to_string())];
        let def = SampleDefinition::new(
            None,
            vec![1],
            VelocityConfig::with_layers(layers, false),
            ReleaseBehavior::PlayToCompletion,
            RetriggerBehavior::Cut,
            None,
            50,
        );
        // Velocity 5 is below the only layer range [10, 50]
        assert!(def.file_for_velocity(5).is_none());
        // Velocity 51 is above the only layer range
        assert!(def.file_for_velocity(51).is_none());
    }

    #[test]
    fn test_add_triggers_replaces_matching() {
        let trigger1 = SampleTrigger::new(midi::note_on(1, 60, 127), "kick".to_string());
        let trigger2 = SampleTrigger::new(midi::note_on(1, 61, 127), "snare".to_string());

        let mut config = SamplesConfig::new(HashMap::new(), vec![trigger1], 32);
        assert_eq!(config.sample_triggers().len(), 1);

        // Add a trigger with a different event — should append.
        config.add_triggers(vec![trigger2]);
        assert_eq!(config.sample_triggers().len(), 2);

        // Add a trigger with the same event as trigger1 — should replace.
        let trigger1_replacement =
            SampleTrigger::new(midi::note_on(1, 60, 127), "kick_v2".to_string());
        config.add_triggers(vec![trigger1_replacement]);
        assert_eq!(config.sample_triggers().len(), 2);
        // The replacement should be the one with "kick_v2".
        let kick_trigger = config
            .sample_triggers()
            .iter()
            .find(|t| t.sample() == "kick_v2");
        assert!(kick_trigger.is_some());
    }

    #[test]
    fn test_merge_triggers_dedup() {
        let trigger1 = SampleTrigger::new(midi::note_on(1, 60, 127), "kick".to_string());
        let trigger2 = SampleTrigger::new(midi::note_on(1, 61, 127), "snare".to_string());

        let mut base = SamplesConfig::new(HashMap::new(), vec![trigger1.clone()], 32);

        // Merge config with same trigger event but different sample name.
        let override_trigger =
            SampleTrigger::new(midi::note_on(1, 60, 127), "kick_override".to_string());
        let other = SamplesConfig::new(HashMap::new(), vec![override_trigger, trigger2], 32);

        base.merge(other);

        // Should have 2 triggers (not 3), with the override replacing the original.
        assert_eq!(base.sample_triggers().len(), 2);
        let kick = base
            .sample_triggers()
            .iter()
            .find(|t| t.sample() == "kick_override");
        assert!(kick.is_some(), "override trigger should be present");
        let original_kick = base.sample_triggers().iter().find(|t| t.sample() == "kick");
        assert!(original_kick.is_none(), "original should be replaced");
    }
}
