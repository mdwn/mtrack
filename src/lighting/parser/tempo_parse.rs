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
    let mut bpm = 120.0; // Default
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
