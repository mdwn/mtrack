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
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::tempo::{
    TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature, TransitionCurve,
};

/// A YAML representation of a song's tempo map. This is the canonical
/// description of a song's tempo and meter, used to derive the beat grid
/// (sections, metronome, visual click) when present.
///
/// BPM is expressed in quarter notes per minute, and mid-measure `beat`
/// positions are in quarter-note units, matching the lighting DSL's
/// `tempo {}` block.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct TempoConfig {
    /// The initial tempo in quarter-note beats per minute.
    pub bpm: f64,
    /// The initial time signature, e.g. "4/4" or "7/8".
    #[serde(default = "default_time_signature")]
    pub time_signature: String,
    /// Offset in seconds of measure 1 beat 1 within the audio (lead-in).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<f64>,
    /// Tempo and/or time signature changes, in ascending measure order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<TempoChangeConfig>,
}

/// A tempo and/or time signature change at a measure position.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct TempoChangeConfig {
    /// The measure (1-indexed) at which the change takes effect.
    pub measure: u32,
    /// The beat within the measure (1-indexed, quarter-note units).
    /// Defaults to 1 (the downbeat).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub beat: Option<f64>,
    /// The new tempo in quarter-note beats per minute, if changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bpm: Option<f64>,
    /// The new time signature, e.g. "6/8", if changed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_signature: Option<String>,
    /// How to transition to the new tempo. Omit for an instant change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<TransitionConfig>,
}

/// How a tempo change transitions from the previous tempo.
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TransitionConfig {
    /// Ramp linearly over the given number of quarter-note beats.
    Beats(f64),
    /// Ramp linearly over the given number of measures.
    Measures(f64),
}

fn default_time_signature() -> String {
    "4/4".to_string()
}

/// Parses a "numerator/denominator" time signature string.
pub fn parse_time_signature(raw: &str) -> Result<TimeSignature, String> {
    let mut parts = raw.split('/');
    let (numerator, denominator) = match (parts.next(), parts.next(), parts.next()) {
        (Some(n), Some(d), None) => (
            n.trim()
                .parse::<u32>()
                .map_err(|_| format!("invalid time signature numerator in {raw:?}"))?,
            d.trim()
                .parse::<u32>()
                .map_err(|_| format!("invalid time signature denominator in {raw:?}"))?,
        ),
        _ => {
            return Err(format!(
                "time signature {raw:?} must be numerator/denominator"
            ))
        }
    };
    if numerator == 0 || numerator > 64 {
        return Err(format!("time signature numerator {numerator} out of range"));
    }
    if !matches!(denominator, 1 | 2 | 4 | 8 | 16 | 32) {
        return Err(format!(
            "time signature denominator {denominator} must be a power of two up to 32"
        ));
    }
    Ok(TimeSignature::new(numerator, denominator))
}

impl TempoConfig {
    /// Validates the tempo configuration without building a tempo map.
    pub fn validate(&self) -> Result<(), String> {
        self.to_tempo_map().map(|_| ())
    }

    /// Converts this configuration into a resolved tempo map.
    pub fn to_tempo_map(&self) -> Result<TempoMap, String> {
        if !self.bpm.is_finite() || self.bpm <= 0.0 {
            return Err(format!("tempo bpm must be positive, got {}", self.bpm));
        }
        let initial_time_signature = parse_time_signature(&self.time_signature)?;
        let start = self.start.unwrap_or(0.0);
        if !start.is_finite() || start < 0.0 {
            return Err(format!("tempo start must be non-negative, got {start}"));
        }

        let mut changes = Vec::with_capacity(self.changes.len());
        let mut last_position = (0u32, 0.0f64);
        for change in &self.changes {
            if change.measure < 1 {
                return Err("tempo change measures are 1-indexed".to_string());
            }
            let beat = change.beat.unwrap_or(1.0);
            if !beat.is_finite() || beat < 1.0 {
                return Err(format!(
                    "tempo change at measure {} has invalid beat {beat} (beats are 1-indexed)",
                    change.measure
                ));
            }
            if (change.measure, beat) <= last_position {
                return Err(format!(
                    "tempo changes must be in ascending order (measure {} beat {beat} repeats or goes backwards)",
                    change.measure
                ));
            }
            last_position = (change.measure, beat);

            if let Some(bpm) = change.bpm {
                if !bpm.is_finite() || bpm <= 0.0 {
                    return Err(format!(
                        "tempo change at measure {} has invalid bpm {bpm}",
                        change.measure
                    ));
                }
            }
            if change.bpm.is_none() && change.time_signature.is_none() {
                return Err(format!(
                    "tempo change at measure {} changes neither bpm nor time signature",
                    change.measure
                ));
            }
            let time_signature = change
                .time_signature
                .as_deref()
                .map(parse_time_signature)
                .transpose()?;
            let transition = match change.transition {
                None => TempoTransition::Snap,
                Some(TransitionConfig::Beats(beats)) => {
                    if !beats.is_finite() || beats <= 0.0 {
                        return Err(format!(
                            "tempo change at measure {} has invalid transition length",
                            change.measure
                        ));
                    }
                    TempoTransition::Beats(beats, TransitionCurve::Linear)
                }
                Some(TransitionConfig::Measures(measures)) => {
                    if !measures.is_finite() || measures <= 0.0 {
                        return Err(format!(
                            "tempo change at measure {} has invalid transition length",
                            change.measure
                        ));
                    }
                    TempoTransition::Measures(measures, TransitionCurve::Linear)
                }
            };

            changes.push(TempoChange {
                position: TempoChangePosition::MeasureBeat(change.measure, beat),
                original_measure_beat: None,
                bpm: change.bpm,
                time_signature,
                transition,
            });
        }

        Ok(TempoMap::new(
            Duration::from_secs_f64(start),
            self.bpm,
            initial_time_signature,
            changes,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{Config, File, FileFormat};

    fn deserialize(yaml: &str) -> TempoConfig {
        Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap()
    }

    #[test]
    fn parse_time_signatures() {
        assert_eq!(
            parse_time_signature("4/4").unwrap(),
            TimeSignature::new(4, 4)
        );
        assert_eq!(
            parse_time_signature("7/8").unwrap(),
            TimeSignature::new(7, 8)
        );
        assert_eq!(
            parse_time_signature(" 6 / 8 ").unwrap(),
            TimeSignature::new(6, 8)
        );
        assert!(parse_time_signature("44").is_err());
        assert!(parse_time_signature("4/3").is_err());
        assert!(parse_time_signature("0/4").is_err());
        assert!(parse_time_signature("4/4/4").is_err());
    }

    #[test]
    fn minimal_config() {
        let config = deserialize("bpm: 152\ntime_signature: 7/8\n");
        let map = config.to_tempo_map().unwrap();
        assert_eq!(map.initial_bpm, 152.0);
        assert_eq!(map.initial_time_signature, TimeSignature::new(7, 8));
        assert_eq!(map.start_offset, Duration::ZERO);
        assert!(map.changes.is_empty());
    }

    #[test]
    fn default_time_signature_is_common_time() {
        let config = deserialize("bpm: 120\n");
        let map = config.to_tempo_map().unwrap();
        assert_eq!(map.initial_time_signature, TimeSignature::new(4, 4));
    }

    #[test]
    fn changes_with_transitions() {
        let config = deserialize(
            r#"
bpm: 118
time_signature: 4/4
start: 0.35
changes:
  - measure: 33
    bpm: 126
    transition: { measures: 2 }
  - measure: 65
    bpm: 96
    time_signature: 6/8
"#,
        );
        let map = config.to_tempo_map().unwrap();
        assert_eq!(map.start_offset, Duration::from_secs_f64(0.35));
        assert_eq!(map.changes.len(), 2);
        assert_eq!(map.changes[0].bpm, Some(126.0));
        assert_eq!(
            map.changes[0].transition,
            TempoTransition::Measures(2.0, TransitionCurve::Linear)
        );
        assert_eq!(map.changes[0].original_measure_beat, Some((33, 1.0)));
        assert_eq!(
            map.changes[1].time_signature,
            Some(TimeSignature::new(6, 8))
        );
        assert_eq!(map.changes[1].transition, TempoTransition::Snap);
    }

    #[test]
    fn beats_transition() {
        let config = deserialize(
            "bpm: 100\nchanges:\n  - measure: 5\n    bpm: 140\n    transition: { beats: 4 }\n",
        );
        let map = config.to_tempo_map().unwrap();
        assert_eq!(
            map.changes[0].transition,
            TempoTransition::Beats(4.0, TransitionCurve::Linear)
        );
    }

    #[test]
    fn rejects_invalid_configs() {
        assert!(deserialize("bpm: 0\n").to_tempo_map().is_err());
        assert!(deserialize("bpm: -10\n").to_tempo_map().is_err());
        assert!(deserialize("bpm: 120\nstart: -1\n").to_tempo_map().is_err());
        assert!(deserialize("bpm: 120\ntime_signature: 5/3\n")
            .to_tempo_map()
            .is_err());
        // Changes out of order.
        assert!(deserialize(
            "bpm: 120\nchanges:\n  - measure: 10\n    bpm: 100\n  - measure: 5\n    bpm: 90\n"
        )
        .to_tempo_map()
        .is_err());
        // Duplicate position.
        assert!(deserialize(
            "bpm: 120\nchanges:\n  - measure: 5\n    bpm: 100\n  - measure: 5\n    bpm: 90\n"
        )
        .to_tempo_map()
        .is_err());
        // No-op change.
        assert!(deserialize("bpm: 120\nchanges:\n  - measure: 5\n")
            .to_tempo_map()
            .is_err());
        // Zero-length transition.
        assert!(deserialize(
            "bpm: 120\nchanges:\n  - measure: 5\n    bpm: 100\n    transition: { measures: 0 }\n"
        )
        .to_tempo_map()
        .is_err());
    }

    #[test]
    fn serialize_roundtrip() {
        let config = deserialize(
            r#"
bpm: 118
time_signature: 4/4
start: 0.35
changes:
  - measure: 33
    bpm: 126
    transition: { measures: 2 }
  - measure: 65
    bpm: 96
    time_signature: 6/8
"#,
        );
        let serialized = crate::util::to_yaml_string(&config).unwrap();
        let deserialized = deserialize(&serialized);
        assert_eq!(deserialized, config);
    }
}
