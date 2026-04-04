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
use std::error::Error;
use std::time::Duration;

use super::super::effects::{
    BlendMode, ChaseDirection, ChasePattern, Color, CycleDirection, CycleTransition, DimmerCurve,
    EffectLayer, EffectType, TempoAwareFrequency, TempoAwareSpeed,
};
use super::super::tempo::TempoMap;
use super::grammar::Rule;
use super::types::{Effect, ParseContext};
use super::utils::{
    parse_color_string, parse_duration_string, parse_frequency_string, parse_percentage_to_f64,
    parse_speed_string,
};
use pest::iterators::Pair;

/// Helper to convert color to normalized RGB parameters
fn color_to_normalized_params(color: &Color) -> (f64, f64, f64) {
    (
        color.r as f64 / 255.0,
        color.g as f64 / 255.0,
        color.b as f64 / 255.0,
    )
}

/// Helper to calculate score time from cue_time and offset
fn calculate_score_time(cue_time: Duration, offset_secs: f64) -> Duration {
    cue_time.saturating_sub(Duration::from_secs_f64(offset_secs))
}

/// Helper to parse duration in score space
fn parse_duration_in_score_space(
    value: &str,
    tempo_map: &Option<TempoMap>,
    cue_time: Duration,
    offset_secs: f64,
) -> Result<Duration, Box<dyn Error>> {
    let score_time = calculate_score_time(cue_time, offset_secs);
    parse_duration_string(value, tempo_map, Some(score_time), 0.0)
}

/// Helper to clean and normalize string values (strip quotes, trim, lowercase)
fn clean_string_value(value: &str) -> String {
    value.trim_matches('"').trim().to_lowercase()
}

pub(crate) fn parse_effect_definition(
    pair: Pair<Rule>,
    ctx: &ParseContext,
) -> Result<Effect, Box<dyn Error>> {
    let tempo_map = &ctx.tempo_map;
    let cue_time = ctx.cue_time;
    let offset_secs = ctx.offset_secs;
    let unshifted_score_time = ctx.unshifted_score_time;
    let score_measure = ctx.score_measure;
    let measure_offset = ctx.measure_offset;
    let mut groups = Vec::new();
    let mut effect_type = EffectType::Static {
        parameters: HashMap::new(),
        duration: Duration::ZERO,
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
                        groups.push(group_pair.as_str().trim().to_string());
                    }
                }
            }
            Rule::effect_type => {
                effect_type = match inner_pair.as_str() {
                    "static" => EffectType::Static {
                        parameters: HashMap::new(),
                        duration: Duration::ZERO,
                    },
                    "cycle" => EffectType::ColorCycle {
                        colors: Vec::new(),
                        speed: TempoAwareSpeed::Fixed(1.0),
                        direction: CycleDirection::Forward,
                        transition: CycleTransition::Snap,
                        duration: Duration::ZERO,
                    },
                    "strobe" => EffectType::Strobe {
                        frequency: TempoAwareFrequency::Fixed(8.0),
                        duration: Duration::ZERO,
                    },
                    "pulse" => EffectType::Pulse {
                        base_level: 0.5,
                        pulse_amplitude: 0.5,
                        frequency: TempoAwareFrequency::Fixed(1.0),
                        duration: Duration::ZERO,
                    },
                    "chase" => EffectType::Chase {
                        pattern: ChasePattern::Linear,
                        speed: TempoAwareSpeed::Fixed(1.0),
                        direction: ChaseDirection::LeftToRight,
                        transition: CycleTransition::Snap,
                        duration: Duration::ZERO,
                    },
                    "dimmer" => EffectType::Dimmer {
                        start_level: 0.0,
                        end_level: 1.0,
                        duration: Duration::from_secs(1),
                        curve: DimmerCurve::Linear,
                    },
                    "rainbow" => EffectType::Rainbow {
                        speed: TempoAwareSpeed::Fixed(1.0),
                        saturation: 1.0,
                        brightness: 1.0,
                        duration: Duration::ZERO,
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
                                // Use score-space time consistent with up_time
                                let duration = parse_duration_in_score_space(
                                    value.as_str(),
                                    tempo_map,
                                    cue_time,
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
    let final_effect_type =
        apply_parameters_to_effect_type(effect_type, &parameters, &color_parameters, ctx)?;

    // Validate that every effect has an explicit duration.
    // Dimmer always has a duration (defaults to 1s). For all other types,
    // either the effect's duration field or hold_time must be set.
    if !matches!(&final_effect_type, EffectType::Dimmer { .. }) {
        let effect_duration = final_effect_type.duration();
        if effect_duration.is_zero() && hold_time.is_none() {
            let effect_name = match &final_effect_type {
                EffectType::Static { .. } => "static",
                EffectType::ColorCycle { .. } => "cycle",
                EffectType::Strobe { .. } => "strobe",
                EffectType::Pulse { .. } => "pulse",
                EffectType::Chase { .. } => "chase",
                EffectType::Rainbow { .. } => "rainbow",
                EffectType::Dimmer { .. } => unreachable!(),
            };
            return Err(format!(
                "Effect '{}' requires a 'duration' or 'hold_time' parameter. \
                 All effects must have an explicit, finite duration.",
                effect_name
            )
            .into());
        }
    }

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
    ctx: &ParseContext,
) -> Result<EffectType, Box<dyn Error>> {
    let tempo_map = &ctx.tempo_map;
    let cue_time = ctx.cue_time;
    let offset_secs = ctx.offset_secs;
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
                            let (r, g, b) = color_to_normalized_params(&color);
                            static_params.insert("red".to_string(), r);
                            static_params.insert("green".to_string(), g);
                            static_params.insert("blue".to_string(), b);
                        }
                    }
                    "duration" => {
                        // Convert shifted cue_time back to score-space for duration calculation
                        let dur =
                            parse_duration_in_score_space(value, tempo_map, cue_time, offset_secs)?;
                        *duration = dur;
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
            duration,
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
                        *direction = match clean_string_value(value).as_str() {
                            "forward" => CycleDirection::Forward,
                            "backward" => CycleDirection::Backward,
                            "pingpong" => CycleDirection::PingPong,
                            other => return Err(format!("Invalid cycle direction: '{}' (expected: forward, backward, pingpong)", other).into()),
                        };
                    }
                    "transition" => {
                        *transition = match clean_string_value(value).as_str() {
                            "snap" => CycleTransition::Snap,
                            "fade" | "crossfade" => CycleTransition::Fade,
                            other => {
                                return Err(format!(
                                    "Invalid transition: '{}' (expected: snap, fade, crossfade)",
                                    other
                                )
                                .into())
                            }
                        };
                    }
                    "duration" => {
                        let dur =
                            parse_duration_in_score_space(value, tempo_map, cue_time, offset_secs)?;
                        *duration = dur;
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
                        let dur =
                            parse_duration_in_score_space(value, tempo_map, cue_time, offset_secs)?;
                        *duration = dur;
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
                        let dur =
                            parse_duration_in_score_space(value, tempo_map, cue_time, offset_secs)?;
                        *duration = dur;
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
            duration,
        } => {
            for (key, value) in parameters {
                match key.as_str() {
                    "pattern" => {
                        *pattern = match clean_string_value(value).as_str() {
                            "linear" => ChasePattern::Linear,
                            "snake" => ChasePattern::Snake,
                            "random" => ChasePattern::Random,
                            other => {
                                return Err(format!(
                                    "Invalid chase pattern: '{}' (expected: linear, snake, random)",
                                    other
                                )
                                .into())
                            }
                        };
                    }
                    "speed" => match parse_speed_string(value, tempo_map) {
                        Ok(val) => *speed = val,
                        Err(e) => {
                            return Err(format!("Invalid speed value '{}': {}", value, e).into());
                        }
                    },
                    "direction" => {
                        *direction = match clean_string_value(value).as_str() {
                            "left_to_right" => ChaseDirection::LeftToRight,
                            "right_to_left" => ChaseDirection::RightToLeft,
                            "top_to_bottom" => ChaseDirection::TopToBottom,
                            "bottom_to_top" => ChaseDirection::BottomToTop,
                            "clockwise" => ChaseDirection::Clockwise,
                            "counter_clockwise" => ChaseDirection::CounterClockwise,
                            other => return Err(format!("Invalid chase direction: '{}' (expected: left_to_right, right_to_left, top_to_bottom, bottom_to_top, clockwise, counter_clockwise)", other).into()),
                        };
                    }
                    "transition" => {
                        *transition = match clean_string_value(value).as_str() {
                            "snap" => CycleTransition::Snap,
                            "fade" | "crossfade" => CycleTransition::Fade,
                            other => {
                                return Err(format!(
                                    "Invalid transition: '{}' (expected: snap, fade, crossfade)",
                                    other
                                )
                                .into())
                            }
                        };
                    }
                    "duration" => {
                        let dur =
                            parse_duration_in_score_space(value, tempo_map, cue_time, offset_secs)?;
                        *duration = dur;
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
                            other => return Err(format!("Invalid dimmer curve: '{}' (expected: linear, exponential, logarithmic, sine, cosine)", other).into()),
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
            duration,
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
                    "duration" => {
                        let dur =
                            parse_duration_in_score_space(value, tempo_map, cue_time, offset_secs)?;
                        *duration = dur;
                    }
                    _ => {}
                }
            }
        }
    }

    Ok(effect_type)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Default ParseContext used by unit tests that don't need tempo/offset.
    fn default_ctx() -> ParseContext {
        ParseContext {
            tempo_map: None,
            cue_time: Duration::ZERO,
            offset_secs: 0.0,
            unshifted_score_time: None,
            score_measure: None,
            measure_offset: 0,
        }
    }

    // ── color_to_normalized_params ─────────────────────────────────

    #[test]
    fn color_normalize_white() {
        let c = Color {
            r: 255,
            g: 255,
            b: 255,
            w: None,
        };
        let (r, g, b) = color_to_normalized_params(&c);
        assert!((r - 1.0).abs() < 1e-9);
        assert!((g - 1.0).abs() < 1e-9);
        assert!((b - 1.0).abs() < 1e-9);
    }

    #[test]
    fn color_normalize_black() {
        let c = Color {
            r: 0,
            g: 0,
            b: 0,
            w: None,
        };
        let (r, g, b) = color_to_normalized_params(&c);
        assert!((r - 0.0).abs() < 1e-9);
        assert!((g - 0.0).abs() < 1e-9);
        assert!((b - 0.0).abs() < 1e-9);
    }

    #[test]
    fn color_normalize_half() {
        let c = Color {
            r: 128,
            g: 64,
            b: 0,
            w: None,
        };
        let (r, g, b) = color_to_normalized_params(&c);
        assert!((r - 128.0 / 255.0).abs() < 1e-9);
        assert!((g - 64.0 / 255.0).abs() < 1e-9);
        assert!((b - 0.0).abs() < 1e-9);
    }

    // ── calculate_score_time ───────────────────────────────────────

    #[test]
    fn score_time_no_offset() {
        let result = calculate_score_time(Duration::from_secs(10), 0.0);
        assert_eq!(result, Duration::from_secs(10));
    }

    #[test]
    fn score_time_with_offset() {
        let result = calculate_score_time(Duration::from_secs(10), 3.0);
        assert_eq!(result, Duration::from_secs(7));
    }

    #[test]
    fn score_time_offset_larger_than_cue() {
        // saturating_sub prevents underflow
        let result = calculate_score_time(Duration::from_secs(2), 5.0);
        assert_eq!(result, Duration::ZERO);
    }

    // ── clean_string_value ─────────────────────────────────────────

    #[test]
    fn clean_strips_quotes_and_lowercases() {
        assert_eq!(clean_string_value("\"Forward\""), "forward");
    }

    #[test]
    fn clean_trims_whitespace() {
        assert_eq!(clean_string_value("  hello  "), "hello");
    }

    #[test]
    fn clean_no_quotes() {
        assert_eq!(clean_string_value("snap"), "snap");
    }

    #[test]
    fn clean_mixed() {
        assert_eq!(clean_string_value("\"  PingPong  \""), "pingpong");
    }

    // ── apply_parameters_to_effect_type — Static ───────────────────

    #[test]
    fn apply_static_dimmer() {
        let et = EffectType::Static {
            parameters: HashMap::new(),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("dimmer".to_string(), "50%".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Static { parameters, .. } = result {
            assert!((parameters["dimmer"] - 0.5).abs() < 1e-9);
        } else {
            panic!("Expected Static");
        }
    }

    #[test]
    fn apply_static_color() {
        let et = EffectType::Static {
            parameters: HashMap::new(),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("color".to_string(), "#FF8000".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Static { parameters, .. } = result {
            assert!((parameters["red"] - 1.0).abs() < 1e-9);
            assert!((parameters["green"] - 128.0 / 255.0).abs() < 1e-2);
            assert!((parameters["blue"] - 0.0).abs() < 1e-9);
        } else {
            panic!("Expected Static");
        }
    }

    #[test]
    fn apply_static_rgb_channels() {
        let et = EffectType::Static {
            parameters: HashMap::new(),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("red".to_string(), "100%".to_string());
        params.insert("green".to_string(), "50%".to_string());
        params.insert("blue".to_string(), "0%".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Static { parameters, .. } = result {
            assert!((parameters["red"] - 1.0).abs() < 1e-9);
            assert!((parameters["green"] - 0.5).abs() < 1e-9);
            assert!((parameters["blue"] - 0.0).abs() < 1e-9);
        } else {
            panic!("Expected Static");
        }
    }

    #[test]
    fn apply_static_duration() {
        let et = EffectType::Static {
            parameters: HashMap::new(),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("duration".to_string(), "2s".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Static { duration, .. } = result {
            assert_eq!(duration, Duration::from_secs(2));
        } else {
            panic!("Expected Static");
        }
    }

    // ── apply_parameters_to_effect_type — ColorCycle ───────────────

    #[test]
    fn apply_color_cycle_colors() {
        let et = EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let colors = vec!["red".to_string(), "#0000FF".to_string()];
        let result =
            apply_parameters_to_effect_type(et, &HashMap::new(), &colors, &default_ctx()).unwrap();
        if let EffectType::ColorCycle { colors, .. } = result {
            assert_eq!(colors.len(), 2);
            assert_eq!(colors[0].r, 255);
            assert_eq!(colors[1].b, 255);
        } else {
            panic!("Expected ColorCycle");
        }
    }

    #[test]
    fn apply_color_cycle_direction() {
        let et = EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("direction".to_string(), "backward".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::ColorCycle { direction, .. } = result {
            assert_eq!(direction, CycleDirection::Backward);
        } else {
            panic!("Expected ColorCycle");
        }
    }

    #[test]
    fn apply_color_cycle_transition_crossfade() {
        let et = EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("transition".to_string(), "crossfade".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::ColorCycle { transition, .. } = result {
            assert_eq!(transition, CycleTransition::Fade);
        } else {
            panic!("Expected ColorCycle");
        }
    }

    // ── apply_parameters_to_effect_type — Strobe ───────────────────

    #[test]
    fn apply_strobe_frequency() {
        let et = EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(8.0),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("frequency".to_string(), "15.0".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Strobe { frequency, .. } = result {
            assert_eq!(frequency, TempoAwareFrequency::Fixed(15.0));
        } else {
            panic!("Expected Strobe");
        }
    }

    #[test]
    fn apply_strobe_rate_alias() {
        let et = EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(8.0),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("rate".to_string(), "20.0".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Strobe { frequency, .. } = result {
            assert_eq!(frequency, TempoAwareFrequency::Fixed(20.0));
        } else {
            panic!("Expected Strobe");
        }
    }

    // ── apply_parameters_to_effect_type — Pulse ────────────────────

    #[test]
    fn apply_pulse_params() {
        let et = EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.5,
            frequency: TempoAwareFrequency::Fixed(1.0),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("base_level".to_string(), "20%".to_string());
        params.insert("intensity".to_string(), "80%".to_string());
        params.insert("frequency".to_string(), "4.0".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Pulse {
            base_level,
            pulse_amplitude,
            frequency,
            ..
        } = result
        {
            assert!((base_level - 0.2).abs() < 1e-9);
            assert!((pulse_amplitude - 0.8).abs() < 1e-9);
            assert_eq!(frequency, TempoAwareFrequency::Fixed(4.0));
        } else {
            panic!("Expected Pulse");
        }
    }

    // ── apply_parameters_to_effect_type — Chase ────────────────────

    #[test]
    fn apply_chase_params() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("pattern".to_string(), "snake".to_string());
        params.insert("direction".to_string(), "clockwise".to_string());
        params.insert("transition".to_string(), "fade".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Chase {
            pattern,
            direction,
            transition,
            ..
        } = result
        {
            assert_eq!(pattern, ChasePattern::Snake);
            assert!(matches!(direction, ChaseDirection::Clockwise));
            assert_eq!(transition, CycleTransition::Fade);
        } else {
            panic!("Expected Chase");
        }
    }

    // ── apply_parameters_to_effect_type — Dimmer ───────────────────

    #[test]
    fn apply_dimmer_params() {
        let et = EffectType::Dimmer {
            start_level: 0.0,
            end_level: 1.0,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        };
        let mut params = HashMap::new();
        params.insert("start".to_string(), "25%".to_string());
        params.insert("end".to_string(), "75%".to_string());
        params.insert("duration".to_string(), "3s".to_string());
        params.insert("curve".to_string(), "exponential".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Dimmer {
            start_level,
            end_level,
            duration,
            curve,
        } = result
        {
            assert!((start_level - 0.25).abs() < 1e-9);
            assert!((end_level - 0.75).abs() < 1e-9);
            assert_eq!(duration, Duration::from_secs(3));
            assert!(matches!(curve, DimmerCurve::Exponential));
        } else {
            panic!("Expected Dimmer");
        }
    }

    #[test]
    fn apply_dimmer_invalid_curve() {
        let et = EffectType::Dimmer {
            start_level: 0.0,
            end_level: 1.0,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        };
        let mut params = HashMap::new();
        params.insert("curve".to_string(), "invalid_curve".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    // ── apply_parameters_to_effect_type — Rainbow ──────────────────

    #[test]
    fn apply_rainbow_params() {
        let et = EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(1.0),
            saturation: 1.0,
            brightness: 1.0,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("saturation".to_string(), "80%".to_string());
        params.insert("brightness".to_string(), "60%".to_string());
        params.insert("speed".to_string(), "2.0".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Rainbow {
            speed,
            saturation,
            brightness,
            ..
        } = result
        {
            assert_eq!(speed, TempoAwareSpeed::Fixed(2.0));
            assert!((saturation - 0.8).abs() < 1e-9);
            assert!((brightness - 0.6).abs() < 1e-9);
        } else {
            panic!("Expected Rainbow");
        }
    }

    // ── Error cases ────────────────────────────────────────────────

    #[test]
    fn apply_invalid_direction_error() {
        let et = EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("direction".to_string(), "sideways".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    #[test]
    fn apply_invalid_chase_direction_error() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("direction".to_string(), "diagonal".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    #[test]
    fn apply_invalid_transition_error() {
        let et = EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("transition".to_string(), "dissolve".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    // ── Dimmer curve variants ───────────────────────────────────────

    #[test]
    fn apply_dimmer_curve_logarithmic() {
        let et = EffectType::Dimmer {
            start_level: 0.0,
            end_level: 1.0,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        };
        let mut params = HashMap::new();
        params.insert("curve".to_string(), "logarithmic".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Dimmer { curve, .. } = result {
            assert!(matches!(curve, DimmerCurve::Logarithmic));
        } else {
            panic!("Expected Dimmer");
        }
    }

    // ── Cycle direction PingPong ─────────────────────────────────────

    #[test]
    fn apply_color_cycle_direction_pingpong() {
        let et = EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("direction".to_string(), "pingpong".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::ColorCycle { direction, .. } = result {
            assert_eq!(direction, CycleDirection::PingPong);
        } else {
            panic!("Expected ColorCycle");
        }
    }

    // ── Chase direction variants ─────────────────────────────────────

    #[test]
    fn apply_chase_direction_right_to_left() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("direction".to_string(), "right_to_left".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Chase { direction, .. } = result {
            assert!(matches!(direction, ChaseDirection::RightToLeft));
        } else {
            panic!("Expected Chase");
        }
    }

    // ── Chase pattern random ─────────────────────────────────────────

    #[test]
    fn apply_chase_pattern_random() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("pattern".to_string(), "random".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Chase { pattern, .. } = result {
            assert!(matches!(pattern, ChasePattern::Random));
        } else {
            panic!("Expected Chase");
        }
    }

    // ── Invalid chase pattern ────────────────────────────────────────

    #[test]
    fn apply_invalid_chase_pattern_error() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("pattern".to_string(), "invalid_pattern".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    // ── Chase transition crossfade alias ────────────────────────────

    #[test]
    fn apply_chase_transition_crossfade() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("transition".to_string(), "crossfade".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Chase { transition, .. } = result {
            assert_eq!(transition, CycleTransition::Fade);
        } else {
            panic!("Expected Chase");
        }
    }

    #[test]
    fn apply_invalid_chase_transition_error() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("transition".to_string(), "wipe".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    // ── Static effect with generic key fallback ──────────────────────

    #[test]
    fn apply_static_unknown_key_numeric() {
        // Unknown key with numeric value should still be stored
        let et = EffectType::Static {
            parameters: HashMap::new(),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("custom_param".to_string(), "0.75".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Static { parameters, .. } = result {
            assert!((parameters["custom_param"] - 0.75).abs() < 1e-9);
        } else {
            panic!("Expected Static");
        }
    }

    // ── Dimmer start_level alias ──────────────────────────────────────

    #[test]
    fn apply_dimmer_start_level_alias() {
        let et = EffectType::Dimmer {
            start_level: 0.0,
            end_level: 1.0,
            duration: Duration::from_secs(1),
            curve: DimmerCurve::Linear,
        };
        let mut params = HashMap::new();
        params.insert("start_level".to_string(), "30%".to_string());
        params.insert("end_level".to_string(), "90%".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Dimmer {
            start_level,
            end_level,
            ..
        } = result
        {
            assert!((start_level - 0.3).abs() < 1e-9);
            assert!((end_level - 0.9).abs() < 1e-9);
        } else {
            panic!("Expected Dimmer");
        }
    }

    // ── Pulse duration ───────────────────────────────────────────────

    #[test]
    fn apply_pulse_duration() {
        let et = EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.5,
            frequency: TempoAwareFrequency::Fixed(1.0),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("duration".to_string(), "5s".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Pulse { duration, .. } = result {
            assert_eq!(duration, Duration::from_secs(5));
        } else {
            panic!("Expected Pulse");
        }
    }

    // ── Strobe duration ──────────────────────────────────────────────

    #[test]
    fn apply_strobe_duration() {
        let et = EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(10.0),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("duration".to_string(), "3s".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Strobe { duration, .. } = result {
            assert_eq!(duration, Duration::from_secs(3));
        } else {
            panic!("Expected Strobe");
        }
    }

    // ── Color cycle speed ────────────────────────────────────────────

    #[test]
    fn apply_color_cycle_speed() {
        let et = EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("speed".to_string(), "2.5".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::ColorCycle { speed, .. } = result {
            assert_eq!(speed, TempoAwareSpeed::Fixed(2.5));
        } else {
            panic!("Expected ColorCycle");
        }
    }

    // ── Chase speed ──────────────────────────────────────────────────

    #[test]
    fn apply_chase_speed() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("speed".to_string(), "3.0".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Chase { speed, .. } = result {
            assert_eq!(speed, TempoAwareSpeed::Fixed(3.0));
        } else {
            panic!("Expected Chase");
        }
    }

    // ── Invalid speed errors ────────────────────────────────────────

    #[test]
    fn apply_color_cycle_invalid_speed_error() {
        let et = EffectType::ColorCycle {
            colors: Vec::new(),
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: CycleDirection::Forward,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("speed".to_string(), "not_a_number".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    #[test]
    fn apply_chase_invalid_speed_error() {
        let et = EffectType::Chase {
            pattern: ChasePattern::Linear,
            speed: TempoAwareSpeed::Fixed(1.0),
            direction: ChaseDirection::LeftToRight,
            transition: CycleTransition::Snap,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("speed".to_string(), "invalid".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    #[test]
    fn apply_rainbow_invalid_speed_error() {
        let et = EffectType::Rainbow {
            speed: TempoAwareSpeed::Fixed(1.0),
            saturation: 1.0,
            brightness: 1.0,
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("speed".to_string(), "bad".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    // ── Invalid frequency errors ────────────────────────────────────

    #[test]
    fn apply_strobe_invalid_frequency_error() {
        let et = EffectType::Strobe {
            frequency: TempoAwareFrequency::Fixed(8.0),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("frequency".to_string(), "not_valid".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    #[test]
    fn apply_pulse_invalid_frequency_error() {
        let et = EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.5,
            frequency: TempoAwareFrequency::Fixed(1.0),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("frequency".to_string(), "xyz".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx());
        assert!(result.is_err());
    }

    // ── Pulse pulse_amplitude alias ─────────────────────────────────

    #[test]
    fn apply_pulse_pulse_amplitude_key() {
        let et = EffectType::Pulse {
            base_level: 0.5,
            pulse_amplitude: 0.5,
            frequency: TempoAwareFrequency::Fixed(1.0),
            duration: Duration::ZERO,
        };
        let mut params = HashMap::new();
        params.insert("pulse_amplitude".to_string(), "70%".to_string());
        let result = apply_parameters_to_effect_type(et, &params, &[], &default_ctx()).unwrap();
        if let EffectType::Pulse {
            pulse_amplitude, ..
        } = result
        {
            assert!((pulse_amplitude - 0.7).abs() < 1e-9);
        } else {
            panic!("Expected Pulse");
        }
    }
}
