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
/// Default synthesized click frequency for half-accent clicks, Hz.
pub const DEFAULT_HALF_FREQ: f64 = 1400.0;
/// Default volume for half-accent clicks.
pub const DEFAULT_HALF_VOLUME: f64 = 0.9;
/// Default synthesized click frequency for subdivision ticks, Hz.
pub const DEFAULT_SUB_FREQ: f64 = 1000.0;
/// Default volume for subdivision ticks.
pub const DEFAULT_SUB_VOLUME: f64 = 0.45;

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
    /// Optional per-beat accent levels, one entry per beat of the measure:
    /// 0 = silent, 1 = normal (baseline), 2 = half accent, 3 = accent.
    /// Takes precedence over `accent` grouping; measures with more beats
    /// than the pattern pad the tail with the baseline level, measures with
    /// fewer truncate it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accents: Vec<u8>,
    /// Subdivision: even clicks per beat (1 = none, 2 = eighths,
    /// 3 = triplets, ...) or a clave pattern (`son` / `rumba`, a two-measure
    /// cycle). Subdivision ticks use the sub sound.
    #[serde(
        default = "default_subdivision",
        skip_serializing_if = "is_default_subdivision"
    )]
    pub subdivision: Subdivision,
    /// Positional changes to the accent pattern and/or subdivision, anchored
    /// at measures; each stays in effect until the next change.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<MetronomeChange>,
    /// Master click volume (0.0-2.0): scales every click sound uniformly,
    /// preserving the accent/half/normal/sub level ordering.
    #[serde(default = "default_volume", skip_serializing_if = "is_default_volume")]
    pub volume: f64,
    /// Click sounds. Defaults to synthesized clicks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sounds: Option<MetronomeSounds>,
}

/// The metronome subdivision: even ticks per beat or a clave pattern.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum Subdivision {
    /// Even ticks per beat: 1 = none, 2 = eighths, 3 = triplets, ...
    Even(u32),
    /// A named clave pattern spanning a two-measure cycle.
    Clave(ClaveKind),
}

/// The supported clave patterns (3-2 direction).
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClaveKind {
    /// Son clave 3-2.
    Son,
    /// Rumba clave 3-2.
    Rumba,
}

impl Default for Subdivision {
    fn default() -> Self {
        Subdivision::Even(1)
    }
}

/// A positional change to the metronome feel, anchored at a measure.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct MetronomeChange {
    /// The 1-indexed measure the change takes effect at.
    pub measure: u32,
    /// New per-beat accent levels (same encoding as the top-level `accents`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accents: Option<Vec<u8>>,
    /// New subdivision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subdivision: Option<Subdivision>,
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
    /// The sound for half-accent clicks (accent level 2).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub half: Option<ClickSound>,
    /// The sound for subdivision ticks (even subdivisions and claves).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub: Option<ClickSound>,
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

fn default_subdivision() -> Subdivision {
    Subdivision::default()
}

fn default_volume() -> f64 {
    1.0
}

fn is_default_volume(volume: &f64) -> bool {
    *volume == 1.0
}

fn is_default_subdivision(subdivision: &Subdivision) -> bool {
    *subdivision == Subdivision::Even(1)
}

impl Default for MetronomeConfig {
    fn default() -> Self {
        MetronomeConfig {
            track: default_metronome_track(),
            accent: Vec::new(),
            accents: Vec::new(),
            subdivision: Subdivision::default(),
            changes: Vec::new(),
            volume: 1.0,
            sounds: None,
        }
    }
}

fn validate_subdivision(subdivision: &Subdivision) -> Result<(), String> {
    if let Subdivision::Even(n) = subdivision {
        if !(1..=12).contains(n) {
            return Err(format!("metronome subdivision must be 1-12, got {n}"));
        }
    }
    Ok(())
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
        if self.accents.iter().any(|&level| level > 3) {
            return Err("metronome accent levels must be 0-3".to_string());
        }
        validate_subdivision(&self.subdivision)?;
        if !self.volume.is_finite() || !(0.0..=2.0).contains(&self.volume) {
            return Err(format!(
                "metronome volume must be within 0.0-2.0, got {}",
                self.volume
            ));
        }
        for change in &self.changes {
            if change.measure < 1 {
                return Err("metronome change measures are 1-indexed".to_string());
            }
            if let Some(accents) = &change.accents {
                if accents.iter().any(|&level| level > 3) {
                    return Err("metronome accent levels must be 0-3".to_string());
                }
            }
            if let Some(subdivision) = &change.subdivision {
                validate_subdivision(subdivision)?;
            }
        }
        if let Some(sounds) = &self.sounds {
            sounds.validate()?;
        }
        Ok(())
    }
}

impl MetronomeSounds {
    /// Validates the click sound definitions (shared between song-level
    /// metronome configs and the player-wide defaults).
    pub fn validate(&self) -> Result<(), String> {
        for (label, sound) in [
            ("accent", self.accent.as_ref()),
            ("normal", self.normal.as_ref()),
            ("half", self.half.as_ref()),
            ("sub", self.sub.as_ref()),
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
        assert!(deserialize("accents: [4]").validate().is_err());
        assert!(deserialize("subdivision: 0").validate().is_err());
        assert!(deserialize("subdivision: 13").validate().is_err());
        assert!(deserialize("sounds:\n  accent: { freq: 5 }")
            .validate()
            .is_err());
        assert!(deserialize("sounds:\n  normal: { volume: 3.0 }")
            .validate()
            .is_err());
        assert!(deserialize("sounds:\n  half: { volume: 3.0 }")
            .validate()
            .is_err());
        assert!(deserialize("volume: 3.0").validate().is_err());
        assert!(deserialize("volume: -0.5").validate().is_err());
    }

    #[test]
    fn master_volume_roundtrip() {
        let config = deserialize("volume: 0.7");
        assert_eq!(config.volume, 0.7);
        assert!(config.validate().is_ok());
        let serialized = crate::util::to_yaml_string(&config).unwrap();
        assert_eq!(deserialize(&serialized), config);
        // The default stays out of the YAML.
        let plain = crate::util::to_yaml_string(&deserialize("{}")).unwrap();
        assert!(!plain.contains("volume"));
    }

    #[test]
    fn accent_levels_and_subdivision_roundtrip() {
        let config = deserialize(
            "accents: [3, 0, 1, 2]\nsubdivision: 3\nsounds:\n  half: { freq: 900, volume: 0.5 }\n",
        );
        assert_eq!(config.accents, vec![3, 0, 1, 2]);
        assert_eq!(config.subdivision, Subdivision::Even(3));
        assert_eq!(
            config.sounds.as_ref().unwrap().half.as_ref().unwrap().freq,
            Some(900.0)
        );
        assert!(config.validate().is_ok());
        let serialized = crate::util::to_yaml_string(&config).unwrap();
        assert_eq!(deserialize(&serialized), config);
        // Defaults stay out of the YAML.
        assert!(!serialized.contains("subdivision: 1"));
    }

    #[test]
    fn clave_subdivision_and_changes_roundtrip() {
        let config = deserialize(
            r#"
subdivision: son
changes:
  - measure: 9
    accents: [3, 1, 2, 1]
  - measure: 17
    subdivision: rumba
  - measure: 25
    subdivision: 2
"#,
        );
        assert_eq!(config.subdivision, Subdivision::Clave(ClaveKind::Son));
        assert_eq!(config.changes.len(), 3);
        assert_eq!(
            config.changes[0].accents.as_deref(),
            Some(&[3, 1, 2, 1][..])
        );
        assert_eq!(
            config.changes[1].subdivision,
            Some(Subdivision::Clave(ClaveKind::Rumba))
        );
        assert_eq!(config.changes[2].subdivision, Some(Subdivision::Even(2)));
        assert!(config.validate().is_ok());
        let serialized = crate::util::to_yaml_string(&config).unwrap();
        assert_eq!(deserialize(&serialized), config);

        // Invalid nested values are rejected.
        assert!(deserialize("changes:\n  - measure: 2\n    accents: [5]")
            .validate()
            .is_err());
        assert!(deserialize("changes:\n  - measure: 2\n    subdivision: 0")
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
