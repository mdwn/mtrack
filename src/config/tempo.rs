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
    check_bpm, check_change_bpm, check_transition, measure_beat_order, parse_signature_in,
    SignatureRange, TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature,
    TransitionCurve,
};

/// A YAML representation of a song's tempo map. This is the canonical
/// description of a song's tempo and meter, used to derive the beat grid
/// (sections, metronome, visual click) when present.
///
/// BPM is expressed in quarter notes per minute, and mid-measure `beat`
/// positions are in quarter-note units, matching the lighting DSL's
/// `tempo {}` block.
#[cfg_attr(
    feature = "tsify",
    derive(tsify_next::Tsify),
    tsify(into_wasm_abi, from_wasm_abi)
)]
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
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
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
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Deserialize, Serialize, Clone, Copy, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TransitionConfig {
    /// Ramp linearly over the given number of quarter-note beats.
    Beats(f64),
    /// Ramp linearly over the given number of measures.
    Measures(f64),
}

fn default_time_signature() -> String {
    let meter = crate::tempo::DEFAULT_TIME_SIGNATURE;
    format!("{}/{}", meter.numerator, meter.denominator)
}

/// Parses a "numerator/denominator" time signature string, in the range a
/// song's `tempo:` block takes.
pub fn parse_time_signature(raw: &str) -> Result<TimeSignature, String> {
    parse_signature_in(raw, SignatureRange::Song)
}

impl TempoConfig {
    /// Validates the tempo configuration without building a tempo map.
    pub fn validate(&self) -> Result<(), String> {
        self.to_tempo_map().map(|_| ())
    }

    /// How many beats a measure (1-indexed) holds, as the beat grid counts
    /// them: one per numerator unit, so 6/8 is six beats and not three.
    /// This is the unit `start_beat` / `end_beat` are expressed in. `None`
    /// if the configuration does not make a tempo map (`validate` says why).
    ///
    /// A change anchored mid-measure does not govern the measure it lands
    /// in — `BeatGrid::from_tempo_map` reads the signature at the measure's
    /// downbeat — so only changes at or before that downbeat count.
    pub fn beats_in_measure(&self, measure: u32) -> Option<u32> {
        let map = self.to_tempo_map().ok()?;
        Some(
            map.time_signature_at_measure(measure)
                .grid_beats_per_measure(),
        )
    }

    /// This configuration with its changes in the order [`Self::to_tempo_map`]
    /// requires -- ascending (measure, beat), `measure_beat_order` -- and one
    /// change per position: the last written, since `TempoMap::new` applies
    /// changes in order and that is the one that was in effect. Returns it
    /// with how many changes were dropped for repeating a position. An
    /// editor that adds a change out of turn, or an import whose changes
    /// share a position, goes through here before a save.
    pub fn normalized(&self) -> (TempoConfig, usize) {
        let mut changes = self.changes.clone();
        changes.sort_by(|a, b| {
            measure_beat_order(
                (a.measure, a.beat.unwrap_or(1.0)),
                (b.measure, b.beat.unwrap_or(1.0)),
            )
        });
        let before = changes.len();
        let mut kept: Vec<TempoChangeConfig> = Vec::with_capacity(before);
        for change in changes {
            let position = (change.measure, change.beat.unwrap_or(1.0));
            if kept
                .last()
                .is_some_and(|last| (last.measure, last.beat.unwrap_or(1.0)) == position)
            {
                kept.pop();
            }
            kept.push(change);
        }
        let dropped = before - kept.len();
        (
            TempoConfig {
                changes: kept,
                ..self.clone()
            },
            dropped,
        )
    }

    /// Converts this configuration into a resolved tempo map.
    pub fn to_tempo_map(&self) -> Result<TempoMap, String> {
        check_bpm(self.bpm)?;
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

            let at = format!("measure {}", change.measure);
            if let Some(bpm) = change.bpm {
                check_change_bpm(&at, bpm)?;
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
                    TempoTransition::Beats(beats, TransitionCurve::Linear)
                }
                Some(TransitionConfig::Measures(measures)) => {
                    TempoTransition::Measures(measures, TransitionCurve::Linear)
                }
            };
            check_transition(&at, transition)?;

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
    fn normalized_orders_and_keeps_the_last_of_a_position() {
        let config = deserialize(
            "bpm: 120\nchanges:\n  - measure: 10\n    bpm: 100\n  - measure: 5\n    bpm: 90\n  - measure: 5\n    beat: 1\n    bpm: 80\n",
        );
        assert!(config.to_tempo_map().is_err());
        let (fixed, dropped) = config.normalized();
        assert_eq!(dropped, 1);
        assert!(fixed.to_tempo_map().is_ok());
        assert_eq!(
            fixed
                .changes
                .iter()
                .map(|c| (c.measure, c.bpm))
                .collect::<Vec<_>>(),
            vec![(5, Some(80.0)), (10, Some(100.0))]
        );
        // A valid config is left as it is.
        assert_eq!(fixed.normalized(), (fixed.clone(), 0));
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
