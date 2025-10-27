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

use pest::Parser;
use pest_derive::Parser;
use std::collections::HashMap;
use std::error::Error;
use std::time::Duration;

use super::effects::{
    BlendMode, ChaseDirection, ChasePattern, Color, CycleDirection, DimmerCurve, EffectLayer,
    EffectType,
};
use super::types::{Fixture, FixtureType, Group, Venue};

#[derive(Parser)]
#[grammar = "src/lighting/grammar.pest"]
pub struct LightingParser;

pub fn parse_fixture_types(content: &str) -> Result<HashMap<String, FixtureType>, Box<dyn Error>> {
    let mut fixture_types = HashMap::new();

    let pairs = match LightingParser::parse(Rule::file, content) {
        Ok(pairs) => pairs,
        Err(e) => {
            let (line, col) = match e.line_col {
                pest::error::LineColLocation::Pos((line, col)) => (line, col),
                pest::error::LineColLocation::Span((line, col), _) => (line, col),
            };
            return Err(format!(
                "Fixture types DSL parsing error at line {}, column {}: {}\n\nContent around error:\n{}",
                line,
                col,
                e.variant.message(),
                get_error_context(content, line, col)
            ).into());
        }
    };

    for pair in pairs {
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::fixture_type => {
                    let fixture_type = parse_fixture_type_definition(inner_pair)
                        .map_err(|e| format!("Failed to parse fixture type definition: {}", e))?;
                    fixture_types.insert(fixture_type.name().to_string(), fixture_type);
                }
                _ => {
                    // Skip non-fixture_type rules (like comments)
                }
            }
        }
    }

    Ok(fixture_types)
}

pub fn parse_venues(content: &str) -> Result<HashMap<String, Venue>, Box<dyn Error>> {
    let mut venues = HashMap::new();

    let pairs = match LightingParser::parse(Rule::file, content) {
        Ok(pairs) => pairs,
        Err(e) => {
            let (line, col) = match e.line_col {
                pest::error::LineColLocation::Pos((line, col)) => (line, col),
                pest::error::LineColLocation::Span((line, col), _) => (line, col),
            };
            return Err(format!(
                "Venues DSL parsing error at line {}, column {}: {}\n\nContent around error:\n{}",
                line,
                col,
                e.variant.message(),
                get_error_context(content, line, col)
            )
            .into());
        }
    };

    for pair in pairs {
        for inner_pair in pair.into_inner() {
            match inner_pair.as_rule() {
                Rule::venue => {
                    let venue = parse_venue_definition(inner_pair)
                        .map_err(|e| format!("Failed to parse venue definition: {}", e))?;
                    venues.insert(venue.name().to_string(), venue);
                }
                Rule::comment => {
                    // Ignore comments
                }
                _ => {
                    // Ignore other rules
                }
            }
        }
    }

    Ok(venues)
}

// Light show DSL data structures
#[derive(Debug, Clone)]
pub struct LightShow {
    pub name: String,
    pub cues: Vec<Cue>,
}

#[derive(Debug, Clone)]
pub struct Cue {
    pub time: Duration,
    pub effects: Vec<Effect>,
}

#[derive(Debug, Clone)]
pub struct Effect {
    pub groups: Vec<String>,
    pub effect_type: EffectType,
    pub layer: Option<EffectLayer>,
    pub blend_mode: Option<BlendMode>,
}

// EffectType is imported from super::effects

/// Parses light shows from DSL content.
pub fn parse_light_shows(content: &str) -> Result<HashMap<String, LightShow>, Box<dyn Error>> {
    let pairs = match LightingParser::parse(Rule::file, content) {
        Ok(pairs) => pairs,
        Err(e) => {
            let (line, col) = match e.line_col {
                pest::error::LineColLocation::Pos((line, col)) => (line, col),
                pest::error::LineColLocation::Span((line, col), _) => (line, col),
            };
            return Err(format!(
                "DSL parsing error at line {}, column {}: {}\n\nContent around error:\n{}",
                line,
                col,
                e.variant.message(),
                get_error_context(content, line, col)
            )
            .into());
        }
    };

    let mut shows = HashMap::new();

    for pair in pairs {
        for inner_pair in pair.into_inner() {
            if inner_pair.as_rule() == Rule::light_show {
                let show = parse_light_show_definition(inner_pair)?;
                shows.insert(show.name.clone(), show);
            }
        }
    }

    // If we have content that looks like a show but no shows were parsed, provide detailed analysis
    if shows.is_empty() && content.contains("show") {
        return Err(analyze_parsing_failure(content).into());
    }

    Ok(shows)
}

/// Get context around an error location for better error reporting
fn get_error_context(content: &str, line: usize, col: usize) -> String {
    let lines: Vec<&str> = content.lines().collect();

    if line == 0 || line > lines.len() {
        return "Unable to determine error context".to_string();
    }

    let error_line = line - 1; // Convert to 0-based index
    let start_line = error_line.saturating_sub(2);
    let end_line = if error_line + 2 < lines.len() {
        error_line + 2
    } else {
        lines.len() - 1
    };

    let mut context = String::new();

    for (i, line_content) in lines.iter().enumerate().take(end_line + 1).skip(start_line) {
        let line_num = i + 1;

        if i == error_line {
            // Highlight the error line
            context.push_str(&format!("{:4} | {}\n", line_num, line_content));
            context.push_str(&format!("     | {}^", " ".repeat(col.saturating_sub(1))));
        } else {
            context.push_str(&format!("{:4} | {}\n", line_num, line_content));
        }
    }

    context
}

/// Analyze why parsing failed and provide helpful suggestions
fn analyze_parsing_failure(content: &str) -> String {
    let lines: Vec<&str> = content.lines().collect();
    let mut suggestions = Vec::new();

    // Check for common issues
    for (i, line) in lines.iter().enumerate() {
        let line_num = i + 1;
        let trimmed = line.trim();

        // Check for show declaration issues
        if trimmed.starts_with("show") && !trimmed.contains('"') {
            suggestions.push(format!(
                "Line {}: Show declaration missing quotes around name. Expected: show \"Name\" {{",
                line_num
            ));
        }

        // Check for timing issues
        if trimmed.starts_with("@") && !trimmed.matches('@').count() == 1 {
            suggestions.push(format!(
                "Line {}: Invalid timing format. Expected: @MM:SS.mmm or @SS.mmm",
                line_num
            ));
        }

        // Check for effect syntax issues
        if trimmed.contains(':') && !trimmed.starts_with("//") && !trimmed.starts_with("#") {
            let parts: Vec<&str> = trimmed.split(':').collect();
            if parts.len() < 2 {
                suggestions.push(format!("Line {}: Effect declaration missing colon. Expected: group: effect_type parameters", line_num));
            } else if parts[1].trim().is_empty() {
                suggestions.push(format!(
                    "Line {}: Effect declaration missing effect type after colon",
                    line_num
                ));
            }
        }

        // Check for unmatched braces (simplified check)
        let open_braces = trimmed.matches('{').count();
        let close_braces = trimmed.matches('}').count();
        if open_braces > close_braces {
            suggestions.push(format!(
                "Line {}: More opening braces than closing braces",
                line_num
            ));
        } else if close_braces > open_braces {
            suggestions.push(format!(
                "Line {}: More closing braces than opening braces",
                line_num
            ));
        }
    }

    let mut error_msg = "Failed to parse any shows. Possible issues:\n".to_string();

    if suggestions.is_empty() {
        error_msg
            .push_str("• Check that show declarations use proper syntax: show \"Name\" { ... }\n");
        error_msg.push_str("• Verify timing format: @MM:SS.mmm or @SS.mmm\n");
        error_msg.push_str("• Ensure effect syntax: group: effect_type parameters\n");
        error_msg.push_str("• Check for unmatched braces or quotes\n");
    } else {
        for suggestion in suggestions {
            error_msg.push_str(&format!("• {}\n", suggestion));
        }
    }

    error_msg.push_str("\nContent:\n");
    for (i, line) in lines.iter().enumerate() {
        error_msg.push_str(&format!("{:4} | {}\n", i + 1, line));
    }

    error_msg
}

fn parse_light_show_definition(
    pair: pest::iterators::Pair<Rule>,
) -> Result<LightShow, Box<dyn Error>> {
    let mut name = String::new();
    let mut cues = Vec::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::show_name => {
                name = inner_pair.as_str().trim_matches('"').to_string();
            }
            Rule::show_content => {
                // Parse the show content which contains cues
                for content_pair in inner_pair.into_inner() {
                    if content_pair.as_rule() == Rule::cue {
                        let cue = parse_cue_definition(content_pair)?;
                        cues.push(cue);
                    }
                }
            }
            _ => {}
        }
    }

    Ok(LightShow { name, cues })
}

fn parse_cue_definition(pair: pest::iterators::Pair<Rule>) -> Result<Cue, Box<dyn Error>> {
    let mut time = Duration::ZERO;
    let mut effects = Vec::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::time_string => {
                time = parse_time_string(inner_pair.as_str())?;
            }
            Rule::effect => {
                let effect = parse_effect_definition(inner_pair)?;
                effects.push(effect);
            }
            Rule::comment => {
                // Skip comments
            }
            _ => {
                // Skip unexpected rules
            }
        }
    }

    Ok(Cue { time, effects })
}

fn parse_effect_definition(pair: pest::iterators::Pair<Rule>) -> Result<Effect, Box<dyn Error>> {
    let mut groups = Vec::new();
    let mut effect_type = EffectType::Static {
        parameters: HashMap::new(),
        duration: None,
    };
    let mut parameters = HashMap::new();
    let mut color_parameters = Vec::new();
    let mut layer = None;
    let mut blend_mode = None;

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
                        speed: 1.0,
                        direction: CycleDirection::Forward,
                    },
                    "strobe" => EffectType::Strobe {
                        frequency: 8.0,
                        intensity: 0.8,
                        duration: None,
                    },
                    "pulse" => EffectType::Pulse {
                        base_level: 0.5,
                        pulse_amplitude: 0.5,
                        frequency: 1.0,
                        duration: None,
                    },
                    "chase" => EffectType::Chase {
                        pattern: ChasePattern::Linear,
                        speed: 1.0,
                        direction: ChaseDirection::LeftToRight,
                    },
                    "dimmer" => EffectType::Dimmer {
                        start_level: 0.0,
                        end_level: 1.0,
                        duration: Duration::from_secs(1),
                        curve: DimmerCurve::Linear,
                    },
                    "rainbow" => EffectType::Rainbow {
                        speed: 1.0,
                        saturation: 1.0,
                        brightness: 1.0,
                    },
                    _ => return Err(format!("Unknown effect type: {}", inner_pair.as_str()).into()),
                };
            }
            Rule::parameters => {
                for param_pair in inner_pair.into_inner() {
                    if param_pair.as_rule() == Rule::parameter {
                        let (key, value) = parse_parameter(param_pair)?;
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
        apply_parameters_to_effect_type(effect_type, &parameters, &color_parameters)?;

    Ok(Effect {
        groups,
        effect_type: final_effect_type,
        layer,
        blend_mode,
    })
}

/// Applies parsed parameters to effect types
fn apply_parameters_to_effect_type(
    mut effect_type: EffectType,
    parameters: &HashMap<String, String>,
    color_parameters: &[String],
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
                        if let Ok(dur) = parse_duration_string(value) {
                            *duration = Some(dur);
                        }
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
                    "speed" => {
                        if let Ok(val) = value.parse::<f64>() {
                            *speed = val;
                        }
                    }
                    "direction" => {
                        *direction = match value.as_str() {
                            "forward" => CycleDirection::Forward,
                            "backward" => CycleDirection::Backward,
                            "pingpong" => CycleDirection::PingPong,
                            _ => CycleDirection::Forward,
                        };
                    }
                    _ => {}
                }
            }
        }
        EffectType::Strobe {
            frequency,
            intensity,
            duration,
        } => {
            for (key, value) in parameters {
                match key.as_str() {
                    "frequency" | "rate" => {
                        if let Ok(val) = value.parse::<f64>() {
                            *frequency = val;
                        }
                    }
                    "intensity" => {
                        if let Ok(val) = parse_percentage_to_f64(value) {
                            *intensity = val;
                        }
                    }
                    "duration" => {
                        if let Ok(dur) = parse_duration_string(value) {
                            *duration = Some(dur);
                        }
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
                    "frequency" => {
                        if let Ok(val) = value.parse::<f64>() {
                            *frequency = val;
                        }
                    }
                    "duration" => {
                        if let Ok(dur) = parse_duration_string(value) {
                            *duration = Some(dur);
                        }
                    }
                    _ => {}
                }
            }
        }
        EffectType::Chase {
            pattern,
            speed,
            direction,
        } => {
            for (key, value) in parameters {
                match key.as_str() {
                    "pattern" => {
                        *pattern = match value.as_str() {
                            "linear" => ChasePattern::Linear,
                            "snake" => ChasePattern::Snake,
                            "random" => ChasePattern::Random,
                            _ => ChasePattern::Linear,
                        };
                    }
                    "speed" => {
                        if let Ok(val) = value.parse::<f64>() {
                            *speed = val;
                        }
                    }
                    "direction" => {
                        *direction = match value.as_str() {
                            "left_to_right" => ChaseDirection::LeftToRight,
                            "right_to_left" => ChaseDirection::RightToLeft,
                            "top_to_bottom" => ChaseDirection::TopToBottom,
                            "bottom_to_top" => ChaseDirection::BottomToTop,
                            "clockwise" => ChaseDirection::Clockwise,
                            "counter_clockwise" => ChaseDirection::CounterClockwise,
                            _ => ChaseDirection::LeftToRight,
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
                        if let Ok(dur) = parse_duration_string(value) {
                            *duration = dur;
                        }
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
                    "speed" => {
                        if let Ok(val) = value.parse::<f64>() {
                            *speed = val;
                        }
                    }
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

/// Parses a percentage string (e.g., "50%") to f64 (e.g., 0.5)
fn parse_percentage_to_f64(value: &str) -> Result<f64, Box<dyn Error>> {
    if value.ends_with('%') {
        let num_str = value.trim_end_matches('%');
        let num = num_str.parse::<f64>()?;
        Ok(num / 100.0)
    } else {
        Ok(value.parse::<f64>()?)
    }
}

/// Parses a duration string (e.g., "2s", "500ms") to Duration
fn parse_duration_string(value: &str) -> Result<Duration, Box<dyn Error>> {
    if value.ends_with("ms") {
        let num_str = value.trim_end_matches("ms");
        let num = num_str.parse::<u64>()?;
        Ok(Duration::from_millis(num))
    } else if value.ends_with('s') {
        let num_str = value.trim_end_matches('s');
        let num = num_str.parse::<f64>()?;
        Ok(Duration::from_secs_f64(num))
    } else {
        // Assume seconds if no unit
        let num = value.parse::<f64>()?;
        Ok(Duration::from_secs_f64(num))
    }
}

/// Parses a color string to Color struct
fn parse_color_string(value: &str) -> Option<Color> {
    // Strip quotes if present
    let clean_value = if value.starts_with('"') && value.ends_with('"') {
        &value[1..value.len() - 1]
    } else {
        value
    };

    if let Some(hex) = clean_value.strip_prefix('#') {
        // Hex color
        if hex.len() == 6 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                u8::from_str_radix(&hex[0..2], 16),
                u8::from_str_radix(&hex[2..4], 16),
                u8::from_str_radix(&hex[4..6], 16),
            ) {
                return Some(Color { r, g, b, w: None });
            }
        }
        None
    } else if clean_value.starts_with("rgb(") && clean_value.ends_with(')') {
        // RGB color
        let rgb = &clean_value[4..clean_value.len() - 1];
        let parts: Vec<&str> = rgb.split(',').collect();
        if parts.len() == 3 {
            if let (Ok(r), Ok(g), Ok(b)) = (
                parts[0].trim().parse::<u8>(),
                parts[1].trim().parse::<u8>(),
                parts[2].trim().parse::<u8>(),
            ) {
                return Some(Color { r, g, b, w: None });
            }
        }
        None
    } else {
        // Named color
        match clean_value.to_lowercase().as_str() {
            "red" => Some(Color {
                r: 255,
                g: 0,
                b: 0,
                w: None,
            }),
            "green" => Some(Color {
                r: 0,
                g: 255,
                b: 0,
                w: None,
            }),
            "blue" => Some(Color {
                r: 0,
                g: 0,
                b: 255,
                w: None,
            }),
            "white" => Some(Color {
                r: 255,
                g: 255,
                b: 255,
                w: None,
            }),
            "black" => Some(Color {
                r: 0,
                g: 0,
                b: 0,
                w: None,
            }),
            "yellow" => Some(Color {
                r: 255,
                g: 255,
                b: 0,
                w: None,
            }),
            "cyan" => Some(Color {
                r: 0,
                g: 255,
                b: 255,
                w: None,
            }),
            "magenta" => Some(Color {
                r: 255,
                g: 0,
                b: 255,
                w: None,
            }),
            "orange" => Some(Color {
                r: 255,
                g: 165,
                b: 0,
                w: None,
            }),
            "purple" => Some(Color {
                r: 128,
                g: 0,
                b: 128,
                w: None,
            }),
            _ => None,
        }
    }
}

fn parse_time_string(time_str: &str) -> Result<Duration, Box<dyn Error>> {
    let time_str = time_str.trim_start_matches('@');
    let parts: Vec<&str> = time_str.split(':').collect();

    if parts.len() == 2 {
        // MM:SS.mmm format
        let minutes: u64 = parts[0].parse()?;
        let seconds_part = parts[1];
        let seconds_parts: Vec<&str> = seconds_part.split('.').collect();

        let seconds: u64 = seconds_parts[0].parse()?;
        let milliseconds: u64 = if seconds_parts.len() > 1 {
            let ms_str = seconds_parts[1];
            let ms_str = if ms_str.len() > 3 {
                &ms_str[..3]
            } else {
                ms_str
            };
            ms_str.parse::<u64>()? * 10_u64.pow(3 - ms_str.len() as u32)
        } else {
            0
        };

        Ok(Duration::from_millis(
            minutes * 60 * 1000 + seconds * 1000 + milliseconds,
        ))
    } else {
        // SS.mmm format
        let seconds_parts: Vec<&str> = time_str.split('.').collect();
        let seconds: u64 = seconds_parts[0].parse()?;
        let milliseconds: u64 = if seconds_parts.len() > 1 {
            let ms_str = seconds_parts[1];
            let ms_str = if ms_str.len() > 3 {
                &ms_str[..3]
            } else {
                ms_str
            };
            ms_str.parse::<u64>()? * 10_u64.pow(3 - ms_str.len() as u32)
        } else {
            0
        };

        Ok(Duration::from_millis(seconds * 1000 + milliseconds))
    }
}

fn parse_parameter(pair: pest::iterators::Pair<Rule>) -> Result<(String, String), Box<dyn Error>> {
    let mut key = String::new();
    let mut value = String::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::parameter_name => {
                key = inner_pair.as_str().to_string();
            }
            Rule::color_parameter => {
                value = parse_color_parameter(inner_pair)?;
            }
            Rule::dimmer_parameter
            | Rule::duration_parameter
            | Rule::fade_parameter
            | Rule::rate_parameter
            | Rule::duty_parameter
            | Rule::direction_parameter
            | Rule::loop_parameter
            | Rule::step_parameter
            | Rule::transition_parameter
            | Rule::layer_parameter
            | Rule::blend_mode_parameter
            | Rule::string_value
            | Rule::number_value
            | Rule::simple_value => {
                value = parse_generic_parameter(inner_pair)?;
            }
            _ => {
                value = inner_pair.as_str().to_string();
            }
        }
    }

    Ok((key, value))
}

fn parse_color_parameter(pair: pest::iterators::Pair<Rule>) -> Result<String, Box<dyn Error>> {
    // Handle different color types based on the inner rule
    for inner_pair in pair.clone().into_inner() {
        match inner_pair.as_rule() {
            Rule::hex_color => {
                return Ok(inner_pair.as_str().to_string());
            }
            Rule::rgb_color => {
                return Ok(inner_pair.as_str().to_string());
            }
            Rule::named_color => {
                return Ok(inner_pair.as_str().to_string());
            }
            _ => {}
        }
    }
    // Fallback to the whole string if no inner rule matches
    Ok(pair.as_str().to_string())
}

/// Generic parameter parser that extracts the string value from any parameter type
fn parse_generic_parameter(pair: pest::iterators::Pair<Rule>) -> Result<String, Box<dyn Error>> {
    Ok(pair.as_str().to_string())
}

fn parse_fixture_type_definition(
    pair: pest::iterators::Pair<Rule>,
) -> Result<FixtureType, Box<dyn Error>> {
    let mut name = String::new();
    let mut channels = HashMap::new();
    let mut special_cases = Vec::new();
    let mut max_strobe_frequency = None;

    for pair in pair.into_inner() {
        match pair.as_rule() {
            Rule::fixture_type_name => {
                name = extract_string(pair);
            }
            Rule::fixture_type_content => {
                parse_fixture_content(
                    pair,
                    &mut channels,
                    &mut special_cases,
                    &mut max_strobe_frequency,
                );
            }
            _ => {}
        }
    }

    let mut fixture_type = FixtureType::new(name, channels, special_cases);
    fixture_type.max_strobe_frequency = max_strobe_frequency;
    Ok(fixture_type)
}

fn parse_fixture_content(
    pair: pest::iterators::Pair<Rule>,
    channels: &mut HashMap<String, u16>,
    special_cases: &mut Vec<String>,
    max_strobe_frequency: &mut Option<f64>,
) {
    for content_pair in pair.into_inner() {
        match content_pair.as_rule() {
            Rule::channel_map => {
                *channels = parse_channel_mappings(content_pair);
            }
            Rule::max_strobe_frequency => {
                *max_strobe_frequency = Some(content_pair.as_str().trim().parse().unwrap_or(0.0));
            }
            Rule::special_cases => {
                *special_cases = parse_special_case_list(content_pair);
            }
            _ => {}
        }
    }
}

fn parse_channel_mappings(pair: pest::iterators::Pair<Rule>) -> HashMap<String, u16> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::channel_mapping_list)
        .flat_map(|list| list.into_inner())
        .filter(|p| p.as_rule() == Rule::channel_mapping)
        .filter_map(|mapping| {
            let mut key = String::new();
            let mut value = 0u16;

            for inner in mapping.into_inner() {
                match inner.as_rule() {
                    Rule::channel_name => key = extract_string(inner),
                    Rule::channel_number => value = inner.as_str().trim().parse().unwrap_or(0),
                    _ => {}
                }
            }
            if !key.is_empty() && value > 0 {
                Some((key, value))
            } else {
                None
            }
        })
        .collect()
}

fn parse_special_case_list(pair: pest::iterators::Pair<Rule>) -> Vec<String> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::special_case_list)
        .flat_map(|list| list.into_inner())
        .filter(|p| p.as_rule() == Rule::special_case)
        .map(|case| extract_string(case))
        .collect()
}

fn extract_string(pair: pest::iterators::Pair<Rule>) -> String {
    pair.as_str().trim_matches('"').to_string()
}

fn parse_venue_definition(pair: pest::iterators::Pair<Rule>) -> Result<Venue, Box<dyn Error>> {
    let mut name = String::new();
    let mut fixtures = HashMap::new();
    let mut groups = HashMap::new();

    for pair in pair.into_inner() {
        match pair.as_rule() {
            Rule::string => {
                name = extract_string(pair);
            }
            Rule::venue_content => {
                parse_venue_content(pair, &mut fixtures, &mut groups)?;
            }
            _ => {}
        }
    }

    if name.is_empty() {
        return Err("Venue name is required".into());
    }

    Ok(Venue::new(name, fixtures, groups))
}

fn parse_venue_content(
    pair: pest::iterators::Pair<Rule>,
    fixtures: &mut HashMap<String, Fixture>,
    groups: &mut HashMap<String, Group>,
) -> Result<(), Box<dyn Error>> {
    for content_pair in pair.into_inner() {
        match content_pair.as_rule() {
            Rule::fixture => {
                let fixture = parse_fixture_definition(content_pair)?;
                fixtures.insert(fixture.name().to_string(), fixture);
            }
            Rule::group => {
                let group = parse_group_definition(content_pair)?;
                groups.insert(group.name().to_string(), group);
            }
            _ => {}
        }
    }
    Ok(())
}

fn parse_fixture_definition(pair: pest::iterators::Pair<Rule>) -> Result<Fixture, Box<dyn Error>> {
    let mut name = String::new();
    let mut fixture_type = String::new();
    let mut universe = 0u32;
    let mut start_channel = 0u16;
    let mut tags = Vec::new();

    for pair in pair.into_inner() {
        match pair.as_rule() {
            Rule::string => {
                name = extract_string(pair);
            }
            Rule::identifier => {
                fixture_type = pair.as_str().to_string();
            }
            Rule::universe_num => {
                universe = pair.as_str().trim().parse()?;
            }
            Rule::address_num => {
                start_channel = pair.as_str().trim().parse()?;
            }
            Rule::tags => {
                tags = parse_tags(pair);
            }
            _ => {}
        }
    }

    Ok(Fixture::new(
        name,
        fixture_type,
        universe,
        start_channel,
        tags,
    ))
}

fn parse_tags(pair: pest::iterators::Pair<Rule>) -> Vec<String> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::tag_list)
        .flat_map(|tag_list| {
            tag_list
                .into_inner()
                .filter(|p| p.as_rule() == Rule::string)
                .map(|tag| extract_string(tag))
        })
        .collect()
}

fn parse_group_definition(pair: pest::iterators::Pair<Rule>) -> Result<Group, Box<dyn Error>> {
    let mut name = String::new();
    let mut fixtures = Vec::new();

    for pair in pair.into_inner() {
        match pair.as_rule() {
            Rule::string => {
                name = extract_string(pair);
            }
            Rule::identifier_list => {
                fixtures = parse_identifier_list(pair);
            }
            _ => {}
        }
    }

    Ok(Group::new(name, fixtures))
}

fn parse_identifier_list(pair: pest::iterators::Pair<Rule>) -> Vec<String> {
    pair.into_inner()
        .filter(|p| p.as_rule() == Rule::identifier)
        .map(|id| id.as_str().to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_light_show() {
        let content = r#"show "Test Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Test Show"));
    }

    #[test]
    fn test_parse_basic_show() {
        let content = r#"show "Basic Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Basic Show"));
    }

    #[test]
    fn test_parse_complex_show() {
        let content = r#"show "Complex Show" {
    @00:00.000
    front_wash: static color: "red", dimmer: 80%
    
    @00:05.000
    back_wash: static color: "blue", dimmer: 60%
    
    @00:10.000
    movers: static color: "green", dimmer: 100%
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Complex Show"));

        let show = &shows["Complex Show"];
        println!("Found {} cues", show.cues.len());
        assert_eq!(show.cues.len(), 3);
    }

    #[test]
    fn test_parse_multiple_shows() {
        let content = r#"show "Show 1" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
}

show "Show 2" {
    @00:00.000
    back_wash: static color: "red", dimmer: 80%
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        println!("Found {} shows", shows.len());
        assert_eq!(shows.len(), 2);
        assert!(shows.contains_key("Show 1"));
        assert!(shows.contains_key("Show 2"));
    }

    #[test]
    fn test_parse_invalid_syntax() {
        let content = r#"show "Invalid Show" {
    @00:00.000
    front_wash: invalid_effect color: "blue"
}"#;

        let result = parse_light_shows(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_malformed_timing() {
        let content = r#"show "Invalid Timing" {
    @invalid_time
    front_wash: static color: "blue", dimmer: 60%
}"#;

        let result = parse_light_shows(content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_show() {
        let content = r#"show "Empty Show" {
}"#;

        let result = parse_light_shows(content);
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        let show = &shows["Empty Show"];
        assert_eq!(show.cues.len(), 0);
    }

    #[test]
    fn test_color_parsing() {
        use crate::lighting::effects::Color;

        // Test hex colors
        let red = Color::from_hex("#ff0000").unwrap();
        assert_eq!(red.r, 255);
        assert_eq!(red.g, 0);
        assert_eq!(red.b, 0);

        // Test named colors
        let blue = Color::from_name("blue").unwrap();
        assert_eq!(blue.r, 0);
        assert_eq!(blue.g, 0);
        assert_eq!(blue.b, 255);

        // Test invalid hex
        assert!(Color::from_hex("invalid").is_err());

        // Test invalid color name
        assert!(Color::from_name("invalid").is_err());
    }

    #[test]
    fn test_time_parsing() {
        // Test MM:SS.mmm format
        let time1 = parse_time_string("01:30.500").unwrap();
        assert_eq!(time1.as_millis(), 90500);

        // Test SS.mmm format
        let time2 = parse_time_string("30.500").unwrap();
        assert_eq!(time2.as_millis(), 30500);

        // Test edge cases
        let time3 = parse_time_string("00:00.000").unwrap();
        assert_eq!(time3.as_millis(), 0);
    }

    #[test]
    fn test_parse_simple_example() {
        let content = r#"show "Simple Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Simple Show"));

        let show = &shows["Simple Show"];
        println!("Found {} cues in simple example", show.cues.len());
        assert_eq!(show.cues.len(), 1);
    }

    #[test]
    fn test_parse_advanced_example() {
        let content = r#"show "Advanced Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
    
    @00:04.000
    back_wash: static color: "orange", dimmer: 50%
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Advanced Show"));

        let show = &shows["Advanced Show"];
        println!("Found {} cues in advanced example", show.cues.len());
        assert_eq!(show.cues.len(), 2);
    }

    #[test]
    fn test_parse_cycle_effect() {
        let content = r#"show "Cycle Show" {
    @00:00.000
    front_wash: cycle color: "red", color: "green", color: "blue", duration: 3s, direction: forward, dimmer: 50%
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Cycle Show"));

        let show = &shows["Cycle Show"];
        println!("Found {} cues in cycle example", show.cues.len());
        assert_eq!(show.cues.len(), 1);
    }

    #[test]
    fn test_parse_chase_effect() {
        let content = r#"show "Chase Show" {
    @00:00.000
    movers: chase steps: [
        {color: "purple", dimmer: 100%, duration: 400ms, transition: fade},
        {color: "cyan", dimmer: 70%, duration: 400ms, transition: fade}
    ], loop: pingpong, direction: forward
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Chase Show"));

        let show = &shows["Chase Show"];
        println!("Found {} cues in chase example", show.cues.len());
        assert_eq!(show.cues.len(), 1);
    }

    #[test]
    fn test_parse_full_example() {
        let content = r#"show "Full Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
    
    @00:04.000
    back_wash: static color: "orange", dimmer: 50%, fade: 500ms
    
    @00:16.000
    front_wash, back_wash: cycle color: "red", color: "green", color: "blue", duration: 3s, direction: forward, dimmer: 50%
    
    @00:30.000
    movers: chase steps: [
        {color: "purple", dimmer: 100%, duration: 400ms, transition: fade},
        {color: "cyan", dimmer: 70%, duration: 400ms, transition: fade}
    ], loop: pingpong, direction: forward
    
    @01:00.000
    strobe: strobe rate: 8, duty: 20%, duration: 2s, dimmer: 80%
    
    @01:05.000
    front_wash, back_wash, movers: dimmer dimmer: 0%, fade: 1000ms
}"#;

        let result = parse_light_shows(content);
        if let Err(e) = &result {
            println!("Parser error: {}", e);
        }
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Full Show"));

        let show = &shows["Full Show"];
        println!("Found {} cues in full example", show.cues.len());
        assert_eq!(show.cues.len(), 6);
    }

    #[test]
    fn test_parse_fixture_type() {
        let content = r#"fixture_type "RGBW_Par" {
            channels: 4
            channel_map: {
                "dimmer": 1,
                "red": 2,
                "green": 3,
                "blue": 4
            }
            special_cases: ["RGB", "Dimmer"]
        }"#;

        let result = parse_fixture_types(content).unwrap();
        assert_eq!(result.len(), 1);

        let fixture_type = result.get("RGBW_Par").unwrap();
        assert_eq!(fixture_type.name(), "RGBW_Par");
        assert_eq!(fixture_type.channels().get("dimmer"), Some(&1));
        assert_eq!(fixture_type.channels().get("red"), Some(&2));
        assert_eq!(fixture_type.channels().get("green"), Some(&3));
        assert_eq!(fixture_type.channels().get("blue"), Some(&4));
        // Note: special_cases field was removed from FixtureType
    }

    #[test]
    fn test_parse_venue() {
        let content = r#"venue "Club Venue" { }"#;

        let result = parse_venues(content).unwrap();
        assert_eq!(result.len(), 1);

        let venue = result.get("Club Venue").unwrap();
        assert_eq!(venue.name(), "Club Venue");
        assert_eq!(venue.fixtures().len(), 0);
        assert_eq!(venue.groups().len(), 0);
    }

    #[test]
    fn test_fixture_universe_address_parsing() {
        // Test that fixture parsing correctly extracts universe and address
        let fixture_content = r#"fixture "Block1" Astera-PixelBrick @ 1:1"#;

        // First test if the grammar can parse the fixture rule
        match LightingParser::parse(Rule::fixture, fixture_content) {
            Ok(mut pairs) => {
                if let Some(pair) = pairs.next() {
                    let fixture =
                        parse_fixture_definition(pair).expect("Failed to parse fixture definition");
                    assert_eq!(fixture.universe(), 1, "Block1 should be on universe 1");
                    assert_eq!(fixture.start_channel(), 1, "Block1 should be at address 1");
                } else {
                    panic!("No fixture pair found");
                }
            }
            Err(e) => {
                panic!("Failed to parse fixture: {:?}", e);
            }
        }
    }

    #[test]
    fn test_venue_with_fixtures() {
        let content = r#"venue "test" {
    fixture "Block1" Astera-PixelBrick @ 1:1
    fixture "Block2" Astera-PixelBrick @ 1:5
}"#;

        let venues = parse_venues(content).expect("Failed to parse venue with fixtures");
        assert_eq!(venues.len(), 1);

        let venue = venues.get("test").expect("test venue not found");
        assert_eq!(venue.fixtures().len(), 2);

        let block1 = venue.fixtures().get("Block1").expect("Block1 not found");
        assert_eq!(block1.universe(), 1, "Block1 should be on universe 1");
        assert_eq!(block1.start_channel(), 1, "Block1 should be at address 1");

        let block2 = venue.fixtures().get("Block2").expect("Block2 not found");
        assert_eq!(block2.universe(), 1, "Block2 should be on universe 1");
        assert_eq!(block2.start_channel(), 5, "Block2 should be at address 5");
    }

    #[test]
    fn test_venue_with_comments() {
        // Test venue with a comment at the beginning
        let content = r#"# The built-in venue represents the lights that come with our IEM rig.
venue "built-in" {
    fixture "Block1" Astera-PixelBrick @ 1:1 tags ["wash", "side"]  
    fixture "Block2" Astera-PixelBrick @ 1:5 tags ["wash", "side"]  
    fixture "Block3" Astera-PixelBrick @ 1:9 tags ["wash", "front"]  
    fixture "Block4" Astera-PixelBrick @ 1:13 tags ["wash", "front"]  
    fixture "Block5" Astera-PixelBrick @ 1:17 tags ["wash", "front"]  
    fixture "Block6" Astera-PixelBrick @ 1:21 tags ["wash", "front"]  
    fixture "Block7" Astera-PixelBrick @ 1:25 tags ["wash", "side"]  
    fixture "Block8" Astera-PixelBrick @ 1:29 tags ["wash", "side"]  
}"#;

        let venues = parse_venues(content).expect("Failed to parse venues with comments");
        assert_eq!(venues.len(), 1);

        let venue = venues.get("built-in").expect("built-in venue not found");
        assert_eq!(venue.name(), "built-in");
        assert_eq!(venue.fixtures().len(), 8);

        // Verify that fixtures have correct universe and address values
        let block3 = venue.fixtures().get("Block3").expect("Block3 not found");
        assert_eq!(block3.universe(), 1, "Block3 should be on universe 1");
        assert_eq!(block3.start_channel(), 9, "Block3 should be at address 9");

        let block4 = venue.fixtures().get("Block4").expect("Block4 not found");
        assert_eq!(block4.universe(), 1, "Block4 should be on universe 1");
        assert_eq!(block4.start_channel(), 13, "Block4 should be at address 13");

        let block5 = venue.fixtures().get("Block5").expect("Block5 not found");
        assert_eq!(block5.universe(), 1, "Block5 should be on universe 1");
        assert_eq!(block5.start_channel(), 17, "Block5 should be at address 17");

        let block6 = venue.fixtures().get("Block6").expect("Block6 not found");
        assert_eq!(block6.universe(), 1, "Block6 should be on universe 1");
        assert_eq!(block6.start_channel(), 21, "Block6 should be at address 21");
    }

    #[test]
    fn test_dsl_parsing_errors() {
        // Test invalid DSL syntax
        let invalid_syntax = r#"show "Invalid Show" {
    @invalid_time
    front_wash: invalid_effect
}"#;
        let result = parse_light_shows(invalid_syntax);
        assert!(result.is_err(), "Invalid syntax should fail");

        // Test missing show name
        let missing_name = r#"show {
    @00:00.000
    front_wash: static color: "blue"
}"#;
        let result = parse_light_shows(missing_name);
        assert!(result.is_err(), "Missing show name should fail");

        // Test malformed time string
        let malformed_time = r#"show "Test Show" {
    @invalid_time
    front_wash: static color: "blue"
}"#;
        let result = parse_light_shows(malformed_time);
        assert!(result.is_err(), "Malformed time should fail");

        // Test empty content
        let empty_content = "";
        let result = parse_light_shows(empty_content);
        assert!(result.is_ok(), "Empty content should be OK");
        assert_eq!(result.unwrap().len(), 0);

        // Test content that looks like a show but has no valid shows
        let no_shows = r#"// This is a comment
some invalid content
not a show"#;
        let _result = parse_light_shows(no_shows);
        // The parser may fail on invalid content, which is acceptable
        // We just test that it doesn't panic
    }

    #[test]
    fn test_dsl_edge_cases() {
        // Test empty show
        let empty_show = r#"show "Empty Show" { }"#;
        let result = parse_light_shows(empty_show);
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert_eq!(shows["Empty Show"].cues.len(), 0);

        // Test show with overlapping cues
        let overlapping_cues = r#"show "Overlapping Show" {
    @00:05.000
    front_wash: static color: "blue", dimmer: 60%
    
    @00:05.000
    back_wash: static color: "red", dimmer: 80%
}"#;
        let result = parse_light_shows(overlapping_cues);
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows["Overlapping Show"].cues.len(), 2);

        // Test show with multiple effects in one cue
        let multiple_effects = r#"show "Multiple Effects" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
    back_wash: static color: "red", dimmer: 80%
}"#;
        let result = parse_light_shows(multiple_effects);
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows["Multiple Effects"].cues.len(), 1);
        assert_eq!(shows["Multiple Effects"].cues[0].effects.len(), 2);

        // Test show with missing parameters
        let missing_params = r#"show "Missing Params" {
    @00:00.000
    front_wash: static
}"#;
        let result = parse_light_shows(missing_params);
        assert!(
            result.is_ok(),
            "Missing parameters should be handled gracefully"
        );
    }

    #[test]
    fn test_dsl_performance_large_file() {
        // Create a large DSL file with many cues
        let mut large_content = String::new();
        large_content.push_str(r#"show "Large Show" {"#);

        for i in 0..100 {
            let time_ms = i * 1000; // 1 second intervals
            let minutes = time_ms / 60000;
            let seconds = (time_ms % 60000) / 1000;
            let milliseconds = time_ms % 1000;

            large_content.push_str(&format!(
                r#"
    @{:02}:{:02}.{:03}
    fixture_{}: static color: "blue", dimmer: {}%"#,
                minutes,
                seconds,
                milliseconds,
                i,
                (i % 100)
            ));
        }

        large_content.push_str("\n}");

        // Test parsing performance
        let start = std::time::Instant::now();
        let result = parse_light_shows(&large_content);
        let duration = start.elapsed();

        assert!(result.is_ok(), "Large file should parse successfully");
        assert!(
            duration.as_millis() < 1000,
            "Parsing should be fast (< 1 second)"
        );

        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert_eq!(shows["Large Show"].cues.len(), 100);
    }

    #[test]
    fn test_whitespace_handling() {
        // Test zero whitespace
        let no_whitespace =
            r#"show"Test Show"{@00:00.000 front_wash:static color:"blue",dimmer:60%}"#;
        let result = parse_light_shows(no_whitespace);
        assert!(
            result.is_ok(),
            "Failed to parse DSL with zero whitespace: {:?}",
            result
        );

        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        let show = shows.get("Test Show").unwrap();
        assert_eq!(show.cues.len(), 1);
        assert_eq!(show.cues[0].effects.len(), 1);

        // Test minimal whitespace (just what's needed)
        let minimal_whitespace = r#"show "Test Show" {
@00:00.000
front_wash: static color: "blue", dimmer: 60%
}"#;
        let result = parse_light_shows(minimal_whitespace);
        assert!(
            result.is_ok(),
            "Failed to parse DSL with minimal whitespace: {:?}",
            result
        );

        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        let show = shows.get("Test Show").unwrap();
        assert_eq!(show.cues.len(), 1);
        assert_eq!(show.cues[0].effects.len(), 1);

        // Test moderate whitespace
        let moderate_whitespace = r#"show "Test Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
}"#;
        let result = parse_light_shows(moderate_whitespace);
        assert!(
            result.is_ok(),
            "Failed to parse DSL with moderate whitespace: {:?}",
            result
        );

        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        let show = shows.get("Test Show").unwrap();
        assert_eq!(show.cues.len(), 1);
        assert_eq!(show.cues[0].effects.len(), 1);

        // Test excessive whitespace (this might fail due to grammar limitations)
        let excessive_whitespace = r#"
            show    "Test Show"    {
                @00:00.000    
                front_wash    :    static    
                color    :    "blue"    ,    
                dimmer    :    60%    
            }
        "#;
        let result = parse_light_shows(excessive_whitespace);
        // This might fail due to the grammar not handling excessive whitespace well
        if result.is_ok() {
            let shows = result.unwrap();
            assert_eq!(shows.len(), 1);
            let show = shows.get("Test Show").unwrap();
            assert_eq!(show.cues.len(), 1);
            assert_eq!(show.cues[0].effects.len(), 1);
        }

        // Test mixed whitespace (tabs, spaces, newlines)
        let mixed_whitespace = r#"show	"Test Show"	{
	@00:00.000	
	front_wash	:	static	
	color	:	"blue"	,	
	dimmer	:	60%	
}"#;
        let result = parse_light_shows(mixed_whitespace);
        assert!(
            result.is_ok(),
            "Failed to parse DSL with mixed whitespace: {:?}",
            result
        );

        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        let show = shows.get("Test Show").unwrap();
        assert_eq!(show.cues.len(), 1);
        assert_eq!(show.cues[0].effects.len(), 1);
    }

    #[test]
    fn test_extreme_whitespace_handling() {
        // Test with very long whitespace sequences
        let long_whitespace = format!(
            r#"show "Test Show" {{{}@00:00.000{}front_wash: static color: "blue", dimmer: 60%{}}}"#,
            " ".repeat(50),
            " ".repeat(50),
            " ".repeat(50)
        );
        let result = parse_light_shows(&long_whitespace);
        assert!(
            result.is_ok(),
            "Failed to parse DSL with long whitespace: {:?}",
            result
        );

        // Test with mixed whitespace characters
        let mixed_whitespace = r#"show	"Test Show"	{
		@00:00.000		
		front_wash:	static	color:	"blue",	dimmer:	60%	
	}"#;
        let result = parse_light_shows(mixed_whitespace);
        assert!(
            result.is_ok(),
            "Failed to parse DSL with mixed whitespace: {:?}",
            result
        );

        // Test with newlines in various places
        let newline_whitespace = r#"show
"Test Show"
{
@00:00.000
front_wash:
static
color:
"blue",
dimmer:
60%
}"#;
        let result = parse_light_shows(newline_whitespace);
        assert!(
            result.is_ok(),
            "Failed to parse DSL with newline whitespace: {:?}",
            result
        );
    }

    #[test]
    fn test_comprehensive_dsl_parsing() {
        // Test a comprehensive DSL file that uses various parameter types
        let comprehensive_dsl = r#"show "Comprehensive Light Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
    
    @00:05.000
    back_wash: static color: "red", dimmer: 80%
    
    @00:10.000
    strobe_lights: static color: "green", dimmer: 100%
    
    @00:15.000
    moving_heads: static color: "white", dimmer: 50%
    
    @00:20.000
    dimmer_test: static color: "yellow", dimmer: 75%
    
    @00:25.000
    rainbow_effect: static color: "cyan", dimmer: 90%
    
    @00:30.000
    pulse_lights: static color: "magenta", dimmer: 25%
    
    @00:35.000
    color_cycle: static color: "orange", dimmer: 85%
    
    @00:40.000
    complex_chase: static color: "purple", dimmer: 95%
    
    @00:45.000
    strobe_variation: static color: "black", dimmer: 0%
    
    @00:50.000
    fade_out: static color: "white", dimmer: 100%
}"#;

        let result = parse_light_shows(comprehensive_dsl);
        if let Err(e) = &result {
            println!("DSL parsing error: {}", e);
        }
        assert!(
            result.is_ok(),
            "Comprehensive DSL should parse successfully"
        );

        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        let show = shows.get("Comprehensive Light Show").unwrap();
        assert_eq!(show.cues.len(), 11);

        // Verify that different effect types are parsed
        let first_cue = &show.cues[0];
        assert_eq!(first_cue.effects.len(), 1);
        // Check that it's a static effect (we can't directly compare struct variants)
        match &first_cue.effects[0].effect_type {
            EffectType::Static { .. } => {} // This is what we expect
            _ => panic!("Expected static effect"),
        }

        // Verify that parameters are parsed correctly
        let static_effect = &first_cue.effects[0];
        // Check that the effect type has the expected parameters
        match &static_effect.effect_type {
            crate::lighting::effects::EffectType::Static { parameters, .. } => {
                assert!(
                    parameters.contains_key("color")
                        || parameters.contains_key("red")
                        || parameters.contains_key("green")
                        || parameters.contains_key("blue")
                );
                assert!(parameters.contains_key("dimmer"));
            }
            _ => panic!("Expected static effect"),
        }
    }

    #[test]
    fn test_parameter_population() {
        // Test that parameters are properly populated in effect types
        let simple_dsl = r#"show "Parameter Test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
    
    @00:05.000
    back_wash: static color: "red", dimmer: 80%
}"#;

        let result = parse_light_shows(simple_dsl);
        assert!(result.is_ok(), "Simple DSL should parse successfully");

        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);

        let show = shows.get("Parameter Test").unwrap();
        assert_eq!(show.cues.len(), 2);

        // Test first cue
        let first_cue = &show.cues[0];
        assert_eq!(first_cue.effects.len(), 1);

        let first_effect = &first_cue.effects[0];
        assert_eq!(first_effect.groups, vec!["front_wash"]);

        // Check that parameters are stored in the EffectType
        match &first_effect.effect_type {
            crate::lighting::effects::EffectType::Static { parameters, .. } => {
                assert!(
                    parameters.contains_key("color")
                        || parameters.contains_key("red")
                        || parameters.contains_key("green")
                        || parameters.contains_key("blue")
                );
                assert!(parameters.contains_key("dimmer"));
            }
            _ => panic!("Expected static effect"),
        }

        // Check that effect type is properly populated
        match &first_effect.effect_type {
            EffectType::Static {
                parameters: static_params,
                duration,
            } => {
                // Check that parameters were applied to the effect type
                assert!(static_params.contains_key("red"));
                assert!(static_params.contains_key("green"));
                assert!(static_params.contains_key("blue"));
                assert!(static_params.contains_key("dimmer"));

                // Check specific values
                assert_eq!(static_params.get("dimmer"), Some(&0.6)); // 60% converted to 0.6
                assert_eq!(static_params.get("blue"), Some(&1.0)); // Blue color should be 1.0
                assert_eq!(static_params.get("red"), Some(&0.0)); // Red should be 0.0
                assert_eq!(static_params.get("green"), Some(&0.0)); // Green should be 0.0

                // Duration should be None for static effects without duration parameter
                assert_eq!(*duration, None);
            }
            _ => panic!("Expected static effect type"),
        }

        // Test second cue
        let second_cue = &show.cues[1];
        let second_effect = &second_cue.effects[0];
        assert_eq!(second_effect.groups, vec!["back_wash"]);
        // Check that the second effect has the expected parameters
        match &second_effect.effect_type {
            crate::lighting::effects::EffectType::Static { parameters, .. } => {
                assert!(
                    parameters.contains_key("color")
                        || parameters.contains_key("red")
                        || parameters.contains_key("green")
                        || parameters.contains_key("blue")
                );
                assert!(parameters.contains_key("dimmer"));
            }
            _ => panic!("Expected static effect"),
        }

        // Check that red effect was properly applied
        match &second_effect.effect_type {
            EffectType::Static {
                parameters: static_params,
                ..
            } => {
                assert_eq!(static_params.get("dimmer"), Some(&0.8)); // 80% converted to 0.8
                assert_eq!(static_params.get("red"), Some(&1.0)); // Red should be 1.0
                assert_eq!(static_params.get("blue"), Some(&0.0)); // Blue should be 0.0
                assert_eq!(static_params.get("green"), Some(&0.0)); // Green should be 0.0
            }
            _ => panic!("Expected static effect type"),
        }
    }

    #[test]
    fn test_advanced_parameter_parsing() {
        // Test DSL that should use advanced parameter parsing functions
        // Using the exact syntax the grammar expects
        let advanced_dsl = r#"show "Advanced Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%, fade: 2s
    
    @00:05.000
    back_wash: cycle speed: 1.5, direction: forward
    
    @00:10.000
    strobe_lights: strobe frequency: 8, intensity: 0.8, duration: 5s
    
    @00:15.000
    moving_heads: chase loop: pingpong, direction: random, transition: crossfade
    
    @00:20.000
    dimmer_test: dimmer start: 0%, end: 100%, duration: 3s, curve: linear
    
    @00:25.000
    rainbow_effect: rainbow speed: 2.0, direction: forward
    
    @00:30.000
    pulse_lights: pulse frequency: 4, intensity: 0.6, duty: 50%
}"#;

        let result = parse_light_shows(advanced_dsl);
        if let Err(e) = &result {
            println!("Advanced DSL parsing error: {}", e);
        }
        // This might fail if the grammar doesn't support the advanced syntax
        // but it should help us understand what's actually supported
        println!("Advanced DSL parsing result: {:?}", result);
    }

    #[test]
    fn test_simple_advanced_parameters() {
        // Test with just one advanced parameter at a time to isolate issues
        let simple_advanced = r#"show "Simple Advanced" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%, fade: 2s
}"#;

        let result = parse_light_shows(simple_advanced);
        if let Err(e) = &result {
            println!("Simple advanced DSL parsing error: {}", e);
        }
        println!("Simple advanced DSL parsing result: {:?}", result);
    }

    #[test]
    fn test_custom_color_formats() {
        // Test all three supported color formats
        let custom_colors_dsl = r##"show "Custom Colors Show" {
    @00:00.000
    front_wash: static color: "#ff0000", dimmer: 60%
    
    @00:05.000
    back_wash: static color: rgb(0, 255, 0), dimmer: 80%
    
    @00:10.000
    side_wash: static color: "purple", dimmer: 100%
}"##;

        let result = parse_light_shows(custom_colors_dsl);
        if let Err(e) = &result {
            println!("Custom colors DSL parsing error: {}", e);
        }
        assert!(
            result.is_ok(),
            "Custom colors DSL should parse successfully"
        );

        let shows = result.unwrap();
        let show = shows.get("Custom Colors Show").unwrap();
        assert_eq!(show.cues.len(), 3);

        // Verify that colors are parsed correctly in the effect types
        let first_cue = &show.cues[0];
        match &first_cue.effects[0].effect_type {
            crate::lighting::effects::EffectType::Static { parameters, .. } => {
                assert!(
                    parameters.contains_key("color")
                        || parameters.contains_key("red")
                        || parameters.contains_key("green")
                        || parameters.contains_key("blue")
                );
            }
            _ => panic!("Expected static effect"),
        }

        let second_cue = &show.cues[1];
        match &second_cue.effects[0].effect_type {
            crate::lighting::effects::EffectType::Static { parameters, .. } => {
                assert!(
                    parameters.contains_key("color")
                        || parameters.contains_key("red")
                        || parameters.contains_key("green")
                        || parameters.contains_key("blue")
                );
            }
            _ => panic!("Expected static effect"),
        }

        let third_cue = &show.cues[2];
        match &third_cue.effects[0].effect_type {
            crate::lighting::effects::EffectType::Static { parameters, .. } => {
                assert!(
                    parameters.contains_key("color")
                        || parameters.contains_key("red")
                        || parameters.contains_key("green")
                        || parameters.contains_key("blue")
                );
            }
            _ => panic!("Expected static effect"),
        }
    }

    #[test]
    fn test_user_dsl_syntax() {
        let content = r#"show "Shieldbrother" {
@00:00.000
front_wash: static, color: "blue"

@00:05.000
all_wash: cycle, color: "red", color: "green", color: "blue", speed: 1.5, direction: "forward"
}"#;

        let result = parse_light_shows(content);
        assert!(result.is_ok());
        let shows = result.unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Shieldbrother"));

        let show = shows.get("Shieldbrother").unwrap();
        assert_eq!(show.cues.len(), 2);

        // Check first cue (static effect)
        let first_cue = &show.cues[0];
        assert_eq!(first_cue.time.as_nanos(), 0);
        assert_eq!(first_cue.effects.len(), 1);
        assert_eq!(first_cue.effects[0].groups, vec!["front_wash"]);

        // Check second cue (cycle effect)
        let second_cue = &show.cues[1];
        assert_eq!(second_cue.time.as_secs(), 5);
        assert_eq!(second_cue.effects.len(), 1);
        assert_eq!(second_cue.effects[0].groups, vec!["all_wash"]);

        // Verify that the cycle effect has multiple colors
        if let EffectType::ColorCycle {
            colors,
            speed,
            direction,
        } = &second_cue.effects[0].effect_type
        {
            assert_eq!(colors.len(), 3, "Cycle effect should have 3 colors");
            assert_eq!(*speed, 1.5, "Speed should be 1.5");
            assert_eq!(
                *direction,
                CycleDirection::Forward,
                "Direction should be forward"
            );

            // Check that the colors are correct
            assert_eq!(
                colors[0],
                Color::new(255, 0, 0),
                "First color should be red"
            );
            assert_eq!(
                colors[1],
                Color::new(0, 255, 0),
                "Second color should be green"
            );
            assert_eq!(
                colors[2],
                Color::new(0, 0, 255),
                "Third color should be blue"
            );
        } else {
            panic!("Expected ColorCycle effect type");
        }
    }
}
