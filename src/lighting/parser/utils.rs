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

use std::error::Error;
use std::time::Duration;

use super::super::effects::{Color, TempoAwareFrequency, TempoAwareSpeed};
use super::super::tempo::TempoMap;
use super::grammar::Rule;
use pest::iterators::Pair;

/// Parses a percentage string (e.g., "50%") to f64 (e.g., 0.5)
pub(crate) fn parse_percentage_to_f64(value: &str) -> Result<f64, Box<dyn Error>> {
    if value.ends_with('%') {
        let num_str = value.trim_end_matches('%');
        let num = num_str.parse::<f64>()?;
        Ok(num / 100.0)
    } else {
        Ok(value.parse::<f64>()?)
    }
}

/// Parses a frequency value to TempoAwareFrequency
/// Supports:
/// - Numeric values (e.g., "4.0") -> Fixed Hz
/// - Time-based values (e.g., "1measure", "2beats", "0.5s") -> TempoAwareFrequency
///
/// For beats/measures, requires tempo_map to be available.
pub(crate) fn parse_frequency_string(
    value: &str,
    tempo_map: &Option<TempoMap>,
) -> Result<TempoAwareFrequency, Box<dyn Error>> {
    let value = value.trim();

    // Try parsing as a simple number first (Hz) - fixed frequency
    if let Ok(val) = value.parse::<f64>() {
        return Ok(TempoAwareFrequency::Fixed(val));
    }

    // Try parsing as a time-based value
    if value.ends_with("ms") {
        let num_str = value.trim_end_matches("ms");
        let num = num_str.parse::<f64>()?;
        let duration_secs = num / 1000.0;
        Ok(TempoAwareFrequency::Seconds(duration_secs))
    } else if value.ends_with("measures") {
        let num_str = value.trim_end_matches("measures");
        let num = num_str.parse::<f64>()?;
        if tempo_map.is_some() {
            Ok(TempoAwareFrequency::Measures(num))
        } else {
            Err("Measure-based frequencies require a tempo section".into())
        }
    } else if value.ends_with("beats") {
        let num_str = value.trim_end_matches("beats");
        let num = num_str.parse::<f64>()?;
        if tempo_map.is_some() {
            Ok(TempoAwareFrequency::Beats(num))
        } else {
            Err("Beat-based frequencies require a tempo section".into())
        }
    } else if value.ends_with('s') {
        let num_str = value.trim_end_matches('s');
        let num = num_str.parse::<f64>()?;
        Ok(TempoAwareFrequency::Seconds(num))
    } else {
        // Fallback: try parsing as a number
        Ok(TempoAwareFrequency::Fixed(value.parse::<f64>()?))
    }
}

/// Parses a speed value to TempoAwareSpeed
/// Supports:
/// - Numeric values (e.g., "1.5") -> Fixed cycles per second
/// - Time-based values (e.g., "1measure", "2beats", "0.5s") -> TempoAwareSpeed
///
/// For beats/measures, requires tempo_map to be available.
pub(crate) fn parse_speed_string(
    value: &str,
    tempo_map: &Option<TempoMap>,
) -> Result<TempoAwareSpeed, Box<dyn Error>> {
    let value = value.trim();

    // Try parsing as a simple number first (cycles per second) - fixed speed
    if let Ok(val) = value.parse::<f64>() {
        return Ok(TempoAwareSpeed::Fixed(val));
    }

    // Try parsing as a time-based value
    if value.ends_with("ms") {
        let num_str = value.trim_end_matches("ms");
        let num = num_str.parse::<f64>()?;
        let duration_secs = num / 1000.0;
        Ok(TempoAwareSpeed::Seconds(duration_secs))
    } else if value.ends_with("measures") {
        let num_str = value.trim_end_matches("measures");
        let num = num_str.parse::<f64>()?;
        if tempo_map.is_some() {
            Ok(TempoAwareSpeed::Measures(num))
        } else {
            Err("Measure-based speeds require a tempo section".into())
        }
    } else if value.ends_with("beats") {
        let num_str = value.trim_end_matches("beats");
        let num = num_str.parse::<f64>()?;
        if tempo_map.is_some() {
            Ok(TempoAwareSpeed::Beats(num))
        } else {
            Err("Beat-based speeds require a tempo section".into())
        }
    } else if value.ends_with('s') {
        let num_str = value.trim_end_matches('s');
        let num = num_str.parse::<f64>()?;
        Ok(TempoAwareSpeed::Seconds(num))
    } else {
        // Fallback: try parsing as a number
        Ok(TempoAwareSpeed::Fixed(value.parse::<f64>()?))
    }
}

/// Parses a duration string (e.g., "2s", "500ms", "4beats", "2measures") to Duration
/// For beats/measures, uses tempo_map if available. If not available, returns an error.
/// offset_secs is used to adjust tempo change lookups when calculating measure/beat durations.
pub(crate) fn parse_duration_string(
    value: &str,
    tempo_map: &Option<TempoMap>,
    at_time: Option<Duration>,
    offset_secs: f64,
) -> Result<Duration, Box<dyn Error>> {
    if value.ends_with("ms") {
        let num_str = value.trim_end_matches("ms");
        let num = num_str.parse::<u64>()?;
        Ok(Duration::from_millis(num))
    } else if value.ends_with("measures") {
        let num_str = value.trim_end_matches("measures");
        let num = num_str.parse::<f64>()?;
        if let Some(tm) = tempo_map {
            let time = at_time.unwrap_or(Duration::ZERO);
            // Only debug for 30measures
            if num == 30.0 {
                eprintln!(
                    "[parse-duration-string] measures={} at_time={:.6}s offset_secs={:.6}",
                    num,
                    time.as_secs_f64(),
                    offset_secs
                );
            }
            Ok(tm.measures_to_duration(num, time, offset_secs))
        } else {
            Err("Measure-based durations require a tempo section".into())
        }
    } else if value.ends_with("beats") {
        let num_str = value.trim_end_matches("beats");
        let num = num_str.parse::<f64>()?;
        if let Some(tm) = tempo_map {
            let time = at_time.unwrap_or(Duration::ZERO);
            Ok(tm.beats_to_duration(num, time, offset_secs))
        } else {
            Err("Beat-based durations require a tempo section".into())
        }
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
pub(crate) fn parse_color_string(value: &str) -> Option<Color> {
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

/// Parse measure/beat notation (e.g., "@12/1" or "@12/1.5")
pub(crate) fn parse_measure_time(time_str: &str) -> Result<(u32, f64), Box<dyn Error>> {
    let trimmed = time_str.trim_start_matches('@');
    let parts: Vec<&str> = trimmed.split('/').collect();

    if parts.len() != 2 {
        return Err(format!("Invalid measure/beat format: {}", time_str).into());
    }

    let measure_str = parts[0].trim();
    let beat_str = parts[1].trim();
    let measure: u32 = measure_str
        .parse()
        .map_err(|e| format!("Failed to parse measure '{}': {}", measure_str, e))?;
    let beat: f64 = beat_str
        .parse()
        .map_err(|e| format!("Failed to parse beat '{}': {}", beat_str, e))?;

    Ok((measure, beat))
}

pub(crate) fn parse_time_string(time_str: &str) -> Result<Duration, Box<dyn Error>> {
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

pub(crate) fn parse_parameter(pair: Pair<Rule>) -> Result<(String, String), Box<dyn Error>> {
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
            Rule::percentage
            | Rule::time_parameter
            | Rule::direction_parameter
            | Rule::chase_pattern_parameter
            | Rule::loop_parameter
            | Rule::step_parameter
            | Rule::transition_parameter
            | Rule::layer_parameter
            | Rule::blend_mode_parameter
            | Rule::string
            | Rule::number_value
            | Rule::bare_identifier => {
                value = parse_generic_parameter(inner_pair)?;
            }
            _ => {
                value = inner_pair.as_str().to_string();
            }
        }
    }

    Ok((key, value))
}

pub(crate) fn parse_color_parameter(pair: Pair<Rule>) -> Result<String, Box<dyn Error>> {
    // Handle different color types based on the inner rule
    for inner_pair in pair.clone().into_inner() {
        match inner_pair.as_rule() {
            Rule::quoted_hex_color => {
                // Strip quotes from quoted hex color
                let s = inner_pair.as_str();
                return Ok(s.to_string());
            }
            Rule::quoted_rgb_color => {
                // Strip quotes from quoted rgb color
                let s = inner_pair.as_str();
                return Ok(s.to_string());
            }
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
pub(crate) fn parse_generic_parameter(pair: Pair<Rule>) -> Result<String, Box<dyn Error>> {
    Ok(pair.as_str().to_string())
}
