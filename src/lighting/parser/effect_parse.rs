// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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
use std::error::Error;
use std::time::Duration;

use super::super::effects::{
    BlendMode, ChaseDirection, ChasePattern, CycleDirection, CycleTransition, DimmerCurve,
    EffectLayer, EffectType,
};
use super::super::tempo::TempoMap;
use super::grammar::Rule;
use super::types::Effect;
use super::utils::{
    parse_color_string, parse_duration_string, parse_frequency_string, parse_percentage_to_f64,
    parse_speed_string,
};
use pest::iterators::Pair;

pub(crate) fn parse_effect_definition(
    pair: Pair<Rule>,
    tempo_map: &Option<TempoMap>,
    cue_time: Duration,
    offset_secs: f64,
    unshifted_score_time: Option<Duration>,
    score_measure: Option<u32>,
    measure_offset: u32,
) -> Result<Effect, Box<dyn Error>> {
    let mut groups = Vec::new();
    let mut effect_type = EffectType::Static {
        parameters: HashMap::new(),
        duration: None,
    };
    let mut parameters = HashMap::new();
    let mut color_parameters = Vec::new();
    let mut layer = None;
    let mut blend_mode = None;
    let mut up_time = None;
    let mut hold_time = None;
    let mut down_time = None;

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::group_list => {
                for group_pair in inner_pair.into_inner() {
                    if group_pair.as_rule() == Rule::group_name {
                        groups.push(group_pair.as_str().to_string());
                    }
                }
            }
            Rule::effect_type => {
                effect_type = match inner_pair.as_str() {
                    "static" => EffectType::Static {
                        parameters: HashMap::new(),
                        duration: None,
                    },
                    "cycle" => EffectType::ColorCycle {
                        colors: Vec::new(),
                        speed: super::super::effects::TempoAwareSpeed::Fixed(1.0),
                        direction: CycleDirection::Forward,
                        transition: super::super::effects::CycleTransition::Snap,
                    },
                    "strobe" => EffectType::Strobe {
                        frequency: super::super::effects::TempoAwareFrequency::Fixed(8.0),
                        duration: None,
                    },
                    "pulse" => EffectType::Pulse {
                        base_level: 0.5,
                        pulse_amplitude: 0.5,
                        frequency: super::super::effects::TempoAwareFrequency::Fixed(1.0),
                        duration: None,
                    },
                    "chase" => EffectType::Chase {
                        pattern: ChasePattern::Linear,
                        speed: super::super::effects::TempoAwareSpeed::Fixed(1.0),
                        direction: ChaseDirection::LeftToRight,
                        transition: super::super::effects::CycleTransition::Snap,
                    },
                    "dimmer" => EffectType::Dimmer {
                        start_level: 0.0,
                        end_level: 1.0,
                        duration: Duration::from_secs(1),
                        curve: DimmerCurve::Linear,
                    },
                    "rainbow" => EffectType::Rainbow {
                        speed: super::super::effects::TempoAwareSpeed::Fixed(1.0),
                        saturation: 1.0,
                        brightness: 1.0,
                    },
                    _ => return Err(format!("Unknown effect type: {}", inner_pair.as_str()).into()),
                };
            }
            Rule::parameters => {
                for param_pair in inner_pair.into_inner() {
                    if param_pair.as_rule() == Rule::parameter {
                        let (key, value) = super::utils::parse_parameter(param_pair)?;
                        match key.as_str() {
                            "color" if matches!(effect_type, EffectType::ColorCycle { .. }) => {
                                color_parameters.push(value);
                            }
                            "layer" => {
                                layer = Some(match value.as_str() {
                                    "background" => EffectLayer::Background,
                                    "midground" => EffectLayer::Midground,
                                    "foreground" => EffectLayer::Foreground,
                                    _ => return Err(format!("Invalid layer: '{}' (expected: background, midground, foreground)", value).into()),
                                });
                            }
                            "blend_mode" => {
                                blend_mode = Some(match value.as_str() {
                                    "replace" => BlendMode::Replace,
                                    "multiply" => BlendMode::Multiply,
                                    "add" => BlendMode::Add,
                                    "overlay" => BlendMode::Overlay,
                                    "screen" => BlendMode::Screen,
                                    _ => {
                                        return Err(format!("Invalid blend mode: {}", value).into())
                                    }
                                });
                            }
                            "up_time" => {
                                // Use unshifted_score_time for tempo lookup to get correct tempo
                                // Duration is independent of offsets - it's calculated in score space
                                let tempo_lookup_time = unshifted_score_time.unwrap_or_else(|| {
                                    cue_time.saturating_sub(Duration::from_secs_f64(offset_secs))
                                });
                                let duration = parse_duration_string(
                                    value.as_str(),
                                    tempo_map,
                                    Some(tempo_lookup_time),
                                    0.0, // Use 0.0 offset since we're using score-space time
                                )?;
                                up_time = Some(duration);
                            }
                            "hold_time" => {
                                // For hold_time: 30measures means 30 PLAYBACK measures
                                // Calculate duration in playback measure space
                                let duration = if value.ends_with("measures") {
                                    let num_str = value.trim_end_matches("measures");
                                    if let Ok(playback_measures) = num_str.parse::<f64>() {
                                        if let Some(tm) = tempo_map {
                                            if let Some(score_measure_val) = score_measure {
                                                // Calculate duration for N playback measures
                                                tm.playback_measures_to_duration(
                                                    score_measure_val,
                                                    playback_measures,
                                                    measure_offset,
                                                )
                                            } else {
                                                // Fallback to time-based calculation if no score measure
                                                let score_time_for_calc = unshifted_score_time
                                                    .unwrap_or_else(|| {
                                                        cue_time.saturating_sub(
                                                            Duration::from_secs_f64(offset_secs),
                                                        )
                                                    });
                                                parse_duration_string(
                                                    value.as_str(),
                                                    tempo_map,
                                                    Some(score_time_for_calc),
                                                    0.0,
                                                )?
                                            }
                                        } else {
                                            return Err(
                                                "Measure-based durations require a tempo section"
                                                    .into(),
                                            );
                                        }
                                    } else {
                                        return Err(
                                            format!("Invalid measure count: {}", num_str).into()
                                        );
                                    }
                                } else {
                                    // For non-measure durations, use standard calculation
                                    let score_time_for_calc =
                                        unshifted_score_time.unwrap_or_else(|| {
                                            cue_time.saturating_sub(Duration::from_secs_f64(
                                                offset_secs,
                                            ))
                                        });
                                    parse_duration_string(
                                        value.as_str(),
                                        tempo_map,
                                        Some(score_time_for_calc),
                                        0.0,
                                    )?
                                };
                                hold_time = Some(duration);
                            }
                            "down_time" => {
                                let duration = parse_duration_string(
                                    value.as_str(),
                                    tempo_map,
                                    Some(cue_time),
                                    offset_secs,
                                )?;
                                down_time = Some(duration);
                            }
                            _ => {
                                parameters.insert(key, value);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    // Apply parameters to the effect type
    let final_effect_type = apply_parameters_to_effect_type(
        effect_type,
        &parameters,
        &color_parameters,
        tempo_map,
        cue_time,
        offset_secs,
    )?;

    Ok(Effect {
        groups,
        effect_type: final_effect_type,
        layer,
        blend_mode,
        up_time,
        hold_time,
        down_time,
        sequence_name: None, // Will be set when expanding sequences
    })
}

/// Applies parsed parameters to effect types
pub(crate) fn apply_parameters_to_effect_type(
    mut effect_type: EffectType,
    parameters: &HashMap<String, String>,
    color_parameters: &[String],
    tempo_map: &Option<TempoMap>,
    cue_time: Duration,
    offset_secs: f64,
) -> Result<EffectType, Box<dyn Error>> {
    match &mut effect_type {
        EffectType::Static {
            parameters: static_params,
            duration,
        } => {
            for (key, value) in parameters {
                match key.as_str() {
                    "dimmer" => {
                        if let Ok(val) = parse_percentage_to_f64(value) {
                            static_params.insert("dimmer".to_string(), val);
                        }
                    }
                    "red" | "green" | "blue" | "white" => {
                        if let Ok(val) = parse_percentage_to_f64(value) {
                            static_params.insert(key.clone(), val);
                        }
                    }
                    "color" => {
                        if let Some(color) = parse_color_string(value) {
                            static_params.insert("red".to_string(), color.r as f64 / 255.0);
                            static_params.insert("green".to_string(), color.g as f64 / 255.0);
                            static_params.insert("blue".to_string(), color.b as f64 / 255.0);
                        }
                    }
                    "duration" => {
                        // Convert shifted cue_time back to score-space for duration calculation
                        let score_time =
                            cue_time.saturating_sub(Duration::from_secs_f64(offset_secs));
                        let dur = parse_duration_string(value, tempo_map, Some(score_time), 0.0)?;
                        *duration = Some(dur);
                    }
                    _ => {
                        if let Ok(val) = value.parse::<f64>() {
                            static_params.insert(key.clone(), val);
                        }
                    }
                }
            }
        }
        EffectType::ColorCycle {
            colors,
            speed,
            direction,
            transition,
        } => {
            // Add all color parameters
            for color_str in color_parameters {
                if let Some(color) = parse_color_string(color_str) {
                    colors.push(color);
                }
            }

            // Handle other parameters
            for (key, value) in parameters {
                match key.as_str() {
                    "speed" => match parse_speed_string(value, tempo_map) {
                        Ok(val) => *speed = val,
                        Err(e) => {
                            return Err(format!("Invalid speed value '{}': {}", value, e).into());
                        }
                    },
                    "direction" => {
                        *direction = match value.as_str() {
                            "forward" => CycleDirection::Forward,
                            "backward" => CycleDirection::Backward,
                            "pingpong" => CycleDirection::PingPong,
                            _ => CycleDirection::Forward,
                        };
                    }
                    "transition" => {
                        *transition = match value.as_str() {
                            "snap" => CycleTransition::Snap,
                            "fade" => CycleTransition::Fade,
                            _ => CycleTransition::Snap,
                        };
                    }
                    _ => {}
                }
            }
        }
        EffectType::Strobe {
            frequency,
            duration,
        } => {
            for (key, value) in parameters {
                match key.as_str() {
                    "frequency" | "rate" => match parse_frequency_string(value, tempo_map) {
                        Ok(val) => *frequency = val,
                        Err(e) => {
                            return Err(
                                format!("Invalid frequency value '{}': {}", value, e).into()
                            );
                        }
                    },
                    "duration" => {
                        // Convert shifted cue_time back to score-space for duration calculation
                        let score_time =
                            cue_time.saturating_sub(Duration::from_secs_f64(offset_secs));
                        let dur = parse_duration_string(value, tempo_map, Some(score_time), 0.0)?;
                        *duration = Some(dur);
                    }
                    _ => {}
                }
            }
        }
        EffectType::Pulse {
            base_level,
            pulse_amplitude,
            frequency,
            duration,
        } => {
            for (key, value) in parameters {
                match key.as_str() {
                    "base_level" => {
                        if let Ok(val) = parse_percentage_to_f64(value) {
                            *base_level = val;
                        }
                    }
                    "pulse_amplitude" | "intensity" => {
                        if let Ok(val) = parse_percentage_to_f64(value) {
                            *pulse_amplitude = val;
                        }
                    }
                    "frequency" => match parse_frequency_string(value, tempo_map) {
                        Ok(val) => *frequency = val,
                        Err(e) => {
                            return Err(
                                format!("Invalid frequency value '{}': {}", value, e).into()
                            );
                        }
                    },
                    "duration" => {
                        // Convert shifted cue_time back to score-space for duration calculation
                        let score_time =
                            cue_time.saturating_sub(Duration::from_secs_f64(offset_secs));
                        let dur = parse_duration_string(value, tempo_map, Some(score_time), 0.0)?;
                        *duration = Some(dur);
                    }
                    _ => {}
                }
            }
        }
        EffectType::Chase {
            pattern,
            speed,
            direction,
            transition,
        } => {
            for (key, value) in parameters {
                match key.as_str() {
                    "pattern" => {
                        // Strip quotes if present, trim whitespace, and convert to lowercase for case-insensitive matching
                        let clean_value = value.trim_matches('"').trim().to_lowercase();
                        *pattern = match clean_value.as_str() {
                            "linear" => ChasePattern::Linear,
                            "snake" => ChasePattern::Snake,
                            "random" => ChasePattern::Random,
                            _ => ChasePattern::Linear, // Default to Linear if pattern doesn't match
                        };
                    }
                    "speed" => match parse_speed_string(value, tempo_map) {
                        Ok(val) => *speed = val,
                        Err(e) => {
                            return Err(format!("Invalid speed value '{}': {}", value, e).into());
                        }
                    },
                    "direction" => {
                        // Strip quotes if present (e.g., "right_to_left" -> right_to_left)
                        let clean_value = value.trim_matches('"').trim();
                        *direction = match clean_value {
                            "left_to_right" => ChaseDirection::LeftToRight,
                            "right_to_left" => ChaseDirection::RightToLeft,
                            "top_to_bottom" => ChaseDirection::TopToBottom,
                            "bottom_to_top" => ChaseDirection::BottomToTop,
                            "clockwise" => ChaseDirection::Clockwise,
                            "counter_clockwise" => ChaseDirection::CounterClockwise,
                            _ => ChaseDirection::LeftToRight,
                        };
                    }
                    "transition" => {
                        *transition = match value.as_str() {
                            "snap" => CycleTransition::Snap,
                            "fade" => CycleTransition::Fade,
                            _ => CycleTransition::Snap,
                        };
                    }
                    _ => {}
                }
            }
        }
        EffectType::Dimmer {
            start_level,
            end_level,
            duration,
            curve,
        } => {
            for (key, value) in parameters {
                match key.as_str() {
                    "start" | "start_level" => {
                        if let Ok(val) = parse_percentage_to_f64(value) {
                            *start_level = val;
                        }
                    }
                    "end" | "end_level" => {
                        if let Ok(val) = parse_percentage_to_f64(value) {
                            *end_level = val;
                        }
                    }
                    "duration" => {
                        let dur =
                            parse_duration_string(value, tempo_map, Some(cue_time), offset_secs)?;
                        *duration = dur;
                    }
                    "curve" => {
                        *curve = match value.as_str() {
                            "linear" => DimmerCurve::Linear,
                            "exponential" => DimmerCurve::Exponential,
                            "logarithmic" => DimmerCurve::Logarithmic,
                            "sine" => DimmerCurve::Sine,
                            "cosine" => DimmerCurve::Cosine,
                            _ => DimmerCurve::Linear,
                        };
                    }
                    _ => {}
                }
            }
        }
        EffectType::Rainbow {
            speed,
            saturation,
            brightness,
        } => {
            for (key, value) in parameters {
                match key.as_str() {
                    "speed" => match parse_speed_string(value, tempo_map) {
                        Ok(val) => *speed = val,
                        Err(e) => {
                            return Err(format!("Invalid speed value '{}': {}", value, e).into());
                        }
                    },
                    "saturation" => {
                        if let Ok(val) = parse_percentage_to_f64(value) {
                            *saturation = val;
                        }
                    }
                    "brightness" => {
                        if let Ok(val) = parse_percentage_to_f64(value) {
                            *brightness = val;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(effect_type)
}
