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
use serde::{Deserialize, Serialize};

/// Default synthesized click frequency for accented (downbeat) clicks, Hz.
pub const DEFAULT_ACCENT_FREQ: f64 = 1600.0;
/// Default synthesized click frequency for normal clicks, Hz.
pub const DEFAULT_NORMAL_FREQ: f64 = 1200.0;
/// Default volume for accented clicks.
pub const DEFAULT_ACCENT_VOLUME: f64 = 1.0;
/// Default volume for normal clicks.
pub const DEFAULT_NORMAL_VOLUME: f64 = 0.8;

/// A YAML representation of a song's metronome: a virtual click track
/// generated from the song's beat grid, routable like any other track.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct MetronomeConfig {
    /// The output track name the metronome plays on. Route it by adding this
    /// name to `track_mappings` in the player profile. Defaults to
    /// "metronome".
    #[serde(
        default = "default_metronome_track",
        skip_serializing_if = "is_default_track"
    )]
    pub track: String,
    /// Optional accent grouping in beats, e.g. [3, 2, 2] accents beats 1, 4
    /// and 6 of a 7/8 measure. Without it only beat 1 is accented.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accent: Vec<u32>,
    /// Click sounds. Defaults to synthesized clicks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sounds: Option<MetronomeSounds>,
}

/// The pair of click sounds used by the metronome.
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
pub struct MetronomeSounds {
    /// The sound for accented (downbeat / group start) clicks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accent: Option<ClickSound>,
    /// The sound for normal clicks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub normal: Option<ClickSound>,
}

/// A single click sound: synthesized (freq/volume) or a sample file.
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
pub struct ClickSound {
    /// Path to a sample file (relative to the song directory). When set,
    /// `freq` is ignored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// Synthesized click frequency in Hz.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub freq: Option<f64>,
    /// Volume multiplier (0.0 to 2.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
}

fn default_metronome_track() -> String {
    "metronome".to_string()
}

fn is_default_track(track: &String) -> bool {
    track == "metronome"
}

impl Default for MetronomeConfig {
    fn default() -> Self {
        MetronomeConfig {
            track: default_metronome_track(),
            accent: Vec::new(),
            sounds: None,
        }
    }
}

impl MetronomeConfig {
    /// Validates the metronome configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.track.trim().is_empty() {
            return Err("metronome track name must not be empty".to_string());
        }
        if self.accent.contains(&0) {
            return Err("metronome accent groups must be at least 1 beat".to_string());
        }
        for (label, sound) in [
            (
                "accent",
                self.sounds.as_ref().and_then(|s| s.accent.as_ref()),
            ),
            (
                "normal",
                self.sounds.as_ref().and_then(|s| s.normal.as_ref()),
            ),
        ] {
            if let Some(sound) = sound {
                if let Some(freq) = sound.freq {
                    if !freq.is_finite() || !(20.0..=20_000.0).contains(&freq) {
                        return Err(format!(
                            "metronome {label} sound frequency must be 20-20000 Hz, got {freq}"
                        ));
                    }
                }
                if let Some(volume) = sound.volume {
                    if !volume.is_finite() || !(0.0..=2.0).contains(&volume) {
                        return Err(format!(
                            "metronome {label} sound volume must be within 0.0-2.0, got {volume}"
                        ));
                    }
                }
                if let Some(file) = &sound.file {
                    if file.trim().is_empty() {
                        return Err(format!("metronome {label} sound file must not be empty"));
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{Config, File, FileFormat};

    fn deserialize(yaml: &str) -> MetronomeConfig {
        Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap()
    }

    #[test]
    fn empty_config_uses_defaults() {
        let config = deserialize("{}");
        assert_eq!(config.track, "metronome");
        assert!(config.accent.is_empty());
        assert!(config.sounds.is_none());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn full_config() {
        let config = deserialize(
            r#"
track: click-gen
accent: [3, 2, 2]
sounds:
  accent: { freq: 1600, volume: 1.0 }
  normal: { file: clicks/lo.wav }
"#,
        );
        assert_eq!(config.track, "click-gen");
        assert_eq!(config.accent, vec![3, 2, 2]);
        let sounds = config.sounds.as_ref().unwrap();
        assert_eq!(sounds.accent.as_ref().unwrap().freq, Some(1600.0));
        assert_eq!(
            sounds.normal.as_ref().unwrap().file.as_deref(),
            Some("clicks/lo.wav")
        );
        assert!(config.validate().is_ok());
    }

    #[test]
    fn rejects_invalid_configs() {
        assert!(deserialize("track: \" \"").validate().is_err());
        assert!(deserialize("accent: [3, 0]").validate().is_err());
        assert!(deserialize("sounds:\n  accent: { freq: 5 }")
            .validate()
            .is_err());
        assert!(deserialize("sounds:\n  normal: { volume: 3.0 }")
            .validate()
            .is_err());
    }

    #[test]
    fn serialize_roundtrip() {
        let config =
            deserialize("track: metronome\naccent: [2, 2, 3]\nsounds:\n  accent: { freq: 2000 }\n");
        let serialized = crate::util::to_yaml_string(&config).unwrap();
        let deserialized = deserialize(&serialized);
        assert_eq!(deserialized, config);
    }
}
