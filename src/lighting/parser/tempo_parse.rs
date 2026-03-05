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

use std::error::Error;
use std::time::Duration;

use super::super::tempo::{
    TempoChange, TempoChangePosition, TempoMap, TempoTransition, TimeSignature, TransitionCurve,
};
use super::grammar::Rule;
use super::utils::{parse_measure_time, parse_time_string};
use pest::iterators::Pair;

pub(crate) fn parse_tempo_definition(pair: Pair<Rule>) -> Result<TempoMap, Box<dyn Error>> {
    let mut start_offset = Duration::ZERO;
    let mut bpm = crate::lighting::tempo::DEFAULT_BPM;
    let mut time_signature = TimeSignature::new(4, 4); // Default
    let mut changes = Vec::new();

    // tempo = { "tempo" ~ "{" ~ tempo_content ~ "}" }
    // tempo_content = { (tempo_start | tempo_bpm | tempo_time_signature | tempo_changes)* }
    // So we need to iterate through tempo_content
    for inner_pair in pair.into_inner() {
        if inner_pair.as_rule() == Rule::tempo_content {
            // tempo_content contains tempo_start, tempo_bpm, etc.
            for content_pair in inner_pair.into_inner() {
                match content_pair.as_rule() {
                    Rule::tempo_start => {
                        // tempo_start = { "start" ~ ":" ~ time_parameter }
                        // time_parameter is atomic (@{ time_value ~ time_unit })
                        // Since it's atomic, iterate through inner pairs to find it
                        for param_pair in content_pair.into_inner() {
                            if param_pair.as_rule() == Rule::time_parameter {
                                let time_str = param_pair.as_str().trim();
                                if time_str.is_empty() {
                                    continue;
                                }
                                start_offset = parse_time_parameter(time_str).map_err(|e| {
                                    format!("Failed to parse time_parameter '{}': {}", time_str, e)
                                })?;
                                break;
                            }
                        }
                    }
                    Rule::tempo_bpm => {
                        for value_pair in content_pair.into_inner() {
                            if value_pair.as_rule() == Rule::number_value {
                                let bpm_str = value_pair.as_str().trim();
                                bpm = bpm_str.parse().map_err(|e| {
                                    format!("Failed to parse BPM '{}': {}", bpm_str, e)
                                })?;
                            }
                        }
                    }
                    Rule::tempo_time_signature => {
                        for value_pair in content_pair.into_inner() {
                            if value_pair.as_rule() == Rule::time_sig_value {
                                let (num, den) = parse_time_signature(value_pair.as_str())?;
                                time_signature = TimeSignature::new(num, den);
                            }
                        }
                    }
                    Rule::tempo_changes => {
                        // tempo_changes = { "changes" ~ ":" ~ "[" ~ tempo_change_list? ~ "]" }
                        // tempo_change_list = { tempo_change ~ ("," ~ tempo_change)* }
                        // We need to find tempo_change_list, which is optional
                        for list_pair in content_pair.into_inner() {
                            if list_pair.as_rule() == Rule::tempo_change_list {
                                // tempo_change_list contains tempo_change pairs separated by commas
                                for change_pair in list_pair.into_inner() {
                                    if change_pair.as_rule() == Rule::tempo_change {
                                        let change = parse_tempo_change(change_pair)?;
                                        changes.push(change);
                                    }
                                    // Skip comma tokens
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(TempoMap::new(start_offset, bpm, time_signature, changes))
}

pub(crate) fn parse_tempo_change(pair: Pair<Rule>) -> Result<TempoChange, Box<dyn Error>> {
    let mut position = TempoChangePosition::Time(Duration::ZERO);
    let mut bpm = None;
    let mut time_signature = None;
    let mut transition = TempoTransition::Snap;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::time_string => {
                let time = parse_time_string(inner_pair.as_str())?;
                position = TempoChangePosition::Time(time);
            }
            Rule::measure_time => {
                let (measure, beat) = parse_measure_time(inner_pair.as_str())?;
                position = TempoChangePosition::MeasureBeat(measure, beat);
            }
            Rule::tempo_change_content => {
                for param_pair in inner_pair.into_inner() {
                    // tempo_change_content contains tempo_change_param pairs
                    // tempo_change_param is a wrapper, so we need to get its inner rule
                    // The inner rule will be one of: tempo_change_bpm, tempo_change_time_signature, tempo_change_transition
                    for actual_param in param_pair.into_inner() {
                        match actual_param.as_rule() {
                            Rule::tempo_change_bpm => {
                                // tempo_change_bpm = { "bpm" ~ ":" ~ number_value }
                                // So actual_param contains "bpm", ":", and number_value
                                // We need to find number_value
                                for value_pair in actual_param.into_inner() {
                                    if value_pair.as_rule() == Rule::number_value {
                                        let bpm_str = value_pair.as_str().trim();
                                        let bpm_value = bpm_str.parse()?;
                                        bpm = Some(bpm_value);
                                        break;
                                    }
                                }
                            }
                            Rule::tempo_change_time_signature => {
                                for value_pair in actual_param.into_inner() {
                                    if value_pair.as_rule() == Rule::time_sig_value {
                                        let (num, den) = parse_time_signature(value_pair.as_str())?;
                                        time_signature = Some(TimeSignature::new(num, den));
                                    }
                                }
                            }
                            Rule::tempo_change_transition => {
                                // tempo_change_transition = { "transition" ~ ":" ~ tempo_transition_duration }
                                // tempo_transition_duration = { tempo_transition_measures | tempo_transition_beats | tempo_transition_snap }
                                // actual_param is tempo_change_transition, which contains "transition", ":", and tempo_transition_duration
                                // We need to find tempo_transition_duration
                                for inner_pair in actual_param.into_inner() {
                                    match inner_pair.as_rule() {
                                        Rule::tempo_transition_duration => {
                                            // tempo_transition_duration is an OR of the three options
                                            for trans_pair in inner_pair.into_inner() {
                                                match trans_pair.as_rule() {
                                                    Rule::tempo_transition_snap => {
                                                        transition = TempoTransition::Snap;
                                                    }
                                                    Rule::tempo_transition_beats => {
                                                        // tempo_transition_beats = { number_value }
                                                        // So trans_pair contains number_value as inner pair
                                                        for value_pair in trans_pair.into_inner() {
                                                            if value_pair.as_rule()
                                                                == Rule::number_value
                                                            {
                                                                let beats = value_pair
                                                                    .as_str()
                                                                    .trim()
                                                                    .parse()?;
                                                                transition = TempoTransition::Beats(
                                                                    beats,
                                                                    TransitionCurve::Linear,
                                                                );
                                                                break;
                                                            }
                                                        }
                                                    }
                                                    Rule::tempo_transition_measures => {
                                                        // tempo_transition_measures is atomic, so we can get the string directly
                                                        let measure_str =
                                                            trans_pair.as_str().trim();
                                                        let num_str =
                                                            measure_str.trim_end_matches('m');
                                                        let measures = num_str.trim().parse()?;
                                                        transition = TempoTransition::Measures(
                                                            measures,
                                                            TransitionCurve::Linear,
                                                        );
                                                    }
                                                    _ => {}
                                                }
                                            }
                                        }
                                        _ => {
                                            // Skip "transition" and ":" tokens
                                        }
                                    }
                                }
                            }
                            _ => {
                                // Skip unexpected rules
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Determine original measure/beat if position is MeasureBeat
    let original_measure_beat = match position {
        TempoChangePosition::MeasureBeat(m, b) => Some((m, b)),
        TempoChangePosition::Time(_) => None,
    };

    Ok(TempoChange {
        position,
        original_measure_beat,
        bpm,
        time_signature,
        transition,
    })
}

pub(crate) fn parse_time_signature(value: &str) -> Result<(u32, u32), Box<dyn Error>> {
    let value = value.trim();
    let parts: Vec<&str> = value.split('/').collect();
    if parts.len() != 2 {
        return Err(format!("Invalid time signature format: {}", value).into());
    }
    let numerator: u32 = parts[0].trim().parse()?;
    let denominator: u32 = parts[1].trim().parse()?;
    Ok((numerator, denominator))
}

fn parse_time_parameter(value: &str) -> Result<Duration, Box<dyn Error>> {
    let trimmed = value.trim();
    if trimmed.ends_with("ms") {
        let num_str = trimmed.trim_end_matches("ms").trim();
        let num = num_str
            .parse::<u64>()
            .map_err(|e| format!("Failed to parse '{}' as u64: {}", num_str, e))?;
        Ok(Duration::from_millis(num))
    } else if trimmed.ends_with('s') {
        let num_str = trimmed.trim_end_matches('s').trim();
        let num = num_str
            .parse::<f64>()
            .map_err(|e| format!("Failed to parse '{}' as f64: {}", num_str, e))?;
        Ok(Duration::from_secs_f64(num))
    } else {
        // Assume seconds if no unit
        let num = trimmed
            .parse::<f64>()
            .map_err(|e| format!("Failed to parse '{}' as f64: {}", trimmed, e))?;
        Ok(Duration::from_secs_f64(num))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lighting::parser::grammar::{LightingParser, Rule};
    use pest::Parser;

    // ── parse_time_signature ─────────────────────────────────────

    #[test]
    fn time_signature_4_4() {
        let (num, den) = parse_time_signature("4/4").unwrap();
        assert_eq!((num, den), (4, 4));
    }

    #[test]
    fn time_signature_3_4() {
        let (num, den) = parse_time_signature("3/4").unwrap();
        assert_eq!((num, den), (3, 4));
    }

    #[test]
    fn time_signature_6_8() {
        let (num, den) = parse_time_signature("6/8").unwrap();
        assert_eq!((num, den), (6, 8));
    }

    #[test]
    fn time_signature_5_4() {
        let (num, den) = parse_time_signature("5/4").unwrap();
        assert_eq!((num, den), (5, 4));
    }

    #[test]
    fn time_signature_with_whitespace() {
        let (num, den) = parse_time_signature("  4/4  ").unwrap();
        assert_eq!((num, den), (4, 4));
    }

    #[test]
    fn time_signature_missing_slash() {
        assert!(parse_time_signature("44").is_err());
    }

    #[test]
    fn time_signature_too_many_parts() {
        assert!(parse_time_signature("4/4/4").is_err());
    }

    #[test]
    fn time_signature_non_numeric() {
        assert!(parse_time_signature("abc/4").is_err());
    }

    #[test]
    fn time_signature_empty() {
        assert!(parse_time_signature("").is_err());
    }

    #[test]
    fn time_signature_slash_only() {
        assert!(parse_time_signature("/").is_err());
    }

    #[test]
    fn time_signature_zero_numerator() {
        // Parses successfully — semantic validation is separate
        let (num, den) = parse_time_signature("0/4").unwrap();
        assert_eq!((num, den), (0, 4));
    }

    // ── parse_time_parameter ─────────────────────────────────────

    #[test]
    fn time_parameter_milliseconds() {
        let d = parse_time_parameter("500ms").unwrap();
        assert_eq!(d, Duration::from_millis(500));
    }

    #[test]
    fn time_parameter_milliseconds_zero() {
        let d = parse_time_parameter("0ms").unwrap();
        assert_eq!(d, Duration::from_millis(0));
    }

    #[test]
    fn time_parameter_seconds() {
        let d = parse_time_parameter("2.5s").unwrap();
        assert_eq!(d, Duration::from_secs_f64(2.5));
    }

    #[test]
    fn time_parameter_seconds_integer() {
        let d = parse_time_parameter("3s").unwrap();
        assert_eq!(d, Duration::from_secs(3));
    }

    #[test]
    fn time_parameter_seconds_zero() {
        let d = parse_time_parameter("0.0s").unwrap();
        assert_eq!(d, Duration::ZERO);
    }

    #[test]
    fn time_parameter_bare_number() {
        let d = parse_time_parameter("1.5").unwrap();
        assert_eq!(d, Duration::from_secs_f64(1.5));
    }

    #[test]
    fn time_parameter_with_whitespace() {
        let d = parse_time_parameter("  100ms  ").unwrap();
        assert_eq!(d, Duration::from_millis(100));
    }

    #[test]
    fn time_parameter_invalid_ms() {
        assert!(parse_time_parameter("abcms").is_err());
    }

    #[test]
    fn time_parameter_invalid_seconds() {
        assert!(parse_time_parameter("abcs").is_err());
    }

    #[test]
    fn time_parameter_invalid_bare() {
        assert!(parse_time_parameter("xyz").is_err());
    }

    // ── parse_tempo_definition (via grammar) ─────────────────────

    fn parse_tempo_from_dsl(content: &str) -> TempoMap {
        let mut pairs = LightingParser::parse(Rule::tempo, content).unwrap();
        let pair = pairs.next().unwrap();
        parse_tempo_definition(pair).unwrap()
    }

    #[test]
    fn tempo_definition_defaults() {
        let tm = parse_tempo_from_dsl("tempo { }");
        assert_eq!(
            tm.bpm_at_time(Duration::ZERO, 0.0),
            crate::lighting::tempo::DEFAULT_BPM
        );
    }

    #[test]
    fn tempo_definition_bpm_only() {
        let tm = parse_tempo_from_dsl(
            r#"tempo {
    bpm: 140
}"#,
        );
        assert_eq!(tm.bpm_at_time(Duration::ZERO, 0.0), 140.0);
    }

    #[test]
    fn tempo_definition_full() {
        let tm = parse_tempo_from_dsl(
            r#"tempo {
    start: 500ms
    bpm: 90
    time_signature: 3/4
}"#,
        );
        assert_eq!(tm.bpm_at_time(Duration::from_millis(500), 0.0), 90.0);
    }

    #[test]
    fn tempo_definition_with_changes() {
        let content = r#"tempo {
    bpm: 120
    time_signature: 4/4
    changes: [
    @8/1 { bpm: 140, transition: snap }
    ]
}"#;
        let tm = parse_tempo_from_dsl(content);
        assert_eq!(tm.bpm_at_time(Duration::ZERO, 0.0), 120.0);
    }

    #[test]
    fn tempo_definition_empty_changes() {
        let content = r#"tempo {
    bpm: 120
    time_signature: 4/4
    changes: []
}"#;
        let tm = parse_tempo_from_dsl(content);
        assert_eq!(tm.bpm_at_time(Duration::ZERO, 0.0), 120.0);
    }

    // ── parse_tempo_change (via grammar) ─────────────────────────

    fn parse_change_from_dsl(change_str: &str) -> TempoChange {
        let mut pairs = LightingParser::parse(Rule::tempo_change, change_str).unwrap();
        let pair = pairs.next().unwrap();
        parse_tempo_change(pair).unwrap()
    }

    #[test]
    fn tempo_change_bpm_snap() {
        let change = parse_change_from_dsl("@8/1 { bpm: 140, transition: snap }");
        assert_eq!(change.bpm, Some(140.0));
        assert!(matches!(change.transition, TempoTransition::Snap));
        assert_eq!(change.original_measure_beat, Some((8, 1.0)));
    }

    #[test]
    fn tempo_change_time_signature() {
        let change = parse_change_from_dsl("@16/1 { time_signature: 6/8 }");
        assert!(change.time_signature.is_some());
        assert_eq!(change.original_measure_beat, Some((16, 1.0)));
    }

    #[test]
    fn tempo_change_beat_transition() {
        let change = parse_change_from_dsl("@8/1 { bpm: 140, transition: 2.5 }");
        assert_eq!(change.bpm, Some(140.0));
        assert!(matches!(change.transition, TempoTransition::Beats(..)));
    }

    #[test]
    fn tempo_change_measure_transition() {
        let change = parse_change_from_dsl("@8/1 { bpm: 160, transition: 1.5m }");
        assert_eq!(change.bpm, Some(160.0));
        assert!(matches!(change.transition, TempoTransition::Measures(..)));
    }

    #[test]
    fn tempo_change_absolute_time() {
        let change = parse_change_from_dsl("@00:30.000 { bpm: 140 }");
        assert_eq!(change.bpm, Some(140.0));
        assert_eq!(change.original_measure_beat, None);
        assert!(matches!(
            change.position,
            TempoChangePosition::Time(d) if d == Duration::from_secs(30)
        ));
    }

    #[test]
    fn tempo_change_ss_mmm_time() {
        let change = parse_change_from_dsl("@45.500 { bpm: 160 }");
        assert_eq!(change.bpm, Some(160.0));
        assert!(matches!(
            change.position,
            TempoChangePosition::Time(d) if d == Duration::from_secs_f64(45.5)
        ));
    }
}
