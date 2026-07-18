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

/// A YAML representation of a song's voice pilot hints: short cues ("bridge
/// in 3..2..1") placed at positions within the song. Each hint can carry an
/// audio sample rendered onto a dedicated virtual track, and its label is
/// shown in the web UI around the hint position. Moving a hint is a config
/// edit — no re-recording of a full-length pilot track.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct PilotConfig {
    /// The output track name hint audio plays on. Route it by adding this
    /// name to `track_mappings` in the player profile. Defaults to "pilot".
    #[serde(
        default = "default_pilot_track",
        skip_serializing_if = "is_default_track"
    )]
    pub track: String,
    /// The hints, in any order.
    #[serde(default)]
    pub hints: Vec<PilotHint>,
}

/// A single pilot hint.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct PilotHint {
    /// Where the hint anchors in the song.
    pub at: HintPosition,
    /// Label shown in the UI around the hint position.
    pub label: String,
    /// Optional audio sample (path relative to the song directory).
    /// Label-only hints are purely visual.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// How the sample aligns to the anchor: `end` (default — a countdown
    /// finishes exactly at the anchor) or `start`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub align: Option<HintAlign>,
    /// Fine adjustment of the anchor in seconds (can be negative).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<f64>,
}

/// A hint position: a measure/beat on the beat grid, or an absolute time.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum HintPosition {
    /// A measure (1-indexed) and optional beat within it (1-indexed grid
    /// beats, i.e. denominator notes — beat 3 in 7/8 is the third eighth).
    MeasureBeat {
        measure: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        beat: Option<u32>,
    },
    /// An absolute time in seconds from the start of the song.
    Time { time: f64 },
}

/// Alignment of a hint's audio relative to its anchor position.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum HintAlign {
    /// The sample ends at the anchor (default).
    End,
    /// The sample starts at the anchor.
    Start,
}

fn default_pilot_track() -> String {
    "pilot".to_string()
}

fn is_default_track(track: &String) -> bool {
    track == "pilot"
}

impl PilotConfig {
    /// Validates the pilot configuration.
    pub fn validate(&self) -> Result<(), String> {
        if self.track.trim().is_empty() {
            return Err("pilot track name must not be empty".to_string());
        }
        for (i, hint) in self.hints.iter().enumerate() {
            let label = if hint.label.trim().is_empty() {
                return Err(format!("pilot hint[{}]: label must not be empty", i));
            } else {
                &hint.label
            };
            match &hint.at {
                HintPosition::MeasureBeat { measure, beat } => {
                    if *measure < 1 {
                        return Err(format!("pilot hint \"{}\": measures are 1-indexed", label));
                    }
                    if let Some(beat) = beat {
                        if *beat < 1 {
                            return Err(format!("pilot hint \"{}\": beats are 1-indexed", label));
                        }
                    }
                }
                HintPosition::Time { time } => {
                    if !time.is_finite() || *time < 0.0 {
                        return Err(format!(
                            "pilot hint \"{}\": time must be non-negative",
                            label
                        ));
                    }
                }
            }
            if let Some(offset) = hint.offset {
                if !offset.is_finite() {
                    return Err(format!("pilot hint \"{}\": offset must be finite", label));
                }
            }
            if let Some(file) = &hint.file {
                if file.trim().is_empty() {
                    return Err(format!("pilot hint \"{}\": file must not be empty", label));
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

    fn deserialize(yaml: &str) -> PilotConfig {
        Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap()
    }

    #[test]
    fn full_config() {
        let config = deserialize(
            r#"
track: cue
hints:
  - at: { measure: 25 }
    label: "bridge in 3..2..1"
    file: hints/bridge.wav
  - at: { measure: 30, beat: 3 }
    label: "half-time"
    align: start
    offset: -0.5
  - at: { time: 84.2 }
    label: "solo"
"#,
        );
        assert_eq!(config.track, "cue");
        assert_eq!(config.hints.len(), 3);
        assert_eq!(
            config.hints[0].at,
            HintPosition::MeasureBeat {
                measure: 25,
                beat: None
            }
        );
        assert_eq!(config.hints[0].file.as_deref(), Some("hints/bridge.wav"));
        assert_eq!(config.hints[1].align, Some(HintAlign::Start));
        assert_eq!(config.hints[1].offset, Some(-0.5));
        assert_eq!(config.hints[2].at, HintPosition::Time { time: 84.2 });
        assert!(config.hints[2].file.is_none());
        assert!(config.validate().is_ok());
    }

    #[test]
    fn default_track_name() {
        let config = deserialize("hints: []");
        assert_eq!(config.track, "pilot");
        assert!(config.validate().is_ok());
    }

    #[test]
    fn rejects_invalid_configs() {
        // Empty label.
        assert!(
            deserialize("hints:\n  - at: { time: 1.0 }\n    label: \" \"\n")
                .validate()
                .is_err()
        );
        // Negative time.
        assert!(
            deserialize("hints:\n  - at: { time: -1.0 }\n    label: x\n")
                .validate()
                .is_err()
        );
        // Zero measure.
        assert!(
            deserialize("hints:\n  - at: { measure: 0 }\n    label: x\n")
                .validate()
                .is_err()
        );
        // Empty file.
        assert!(
            deserialize("hints:\n  - at: { time: 1.0 }\n    label: x\n    file: \"\"\n")
                .validate()
                .is_err()
        );
    }

    #[test]
    fn serialize_roundtrip() {
        let config = deserialize(
            r#"
hints:
  - at: { measure: 25, beat: 3 }
    label: bridge
    file: hints/bridge.wav
    align: end
    offset: 0.25
  - at: { time: 12.5 }
    label: solo
"#,
        );
        let serialized = crate::util::to_yaml_string(&config).unwrap();
        let deserialized = deserialize(&serialized);
        assert_eq!(deserialized, config);
    }
}
