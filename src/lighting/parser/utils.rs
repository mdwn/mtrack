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

use super::super::effects::{Color, TempoAwareFrequency, TempoAwareSpeed, TempoAwareValue};
use super::super::tempo::TempoMap;
use super::grammar::Rule;
use pest::iterators::Pair;

/// Parses a percentage string (e.g., "50%") to f64 (e.g., 0.5)
pub(crate) fn parse_percentage_to_f64(value: &str) -> Result<f64, Box<dyn Error>> {
    let value = value.trim();
    if value.ends_with('%') {
        let num_str = value.trim_end_matches('%');
        let num = num_str.parse::<f64>()?;
        Ok(num / 100.0)
    } else {
        Ok(value.parse::<f64>()?)
    }
}

/// Parses a tempo-aware value string into a `TempoAwareValue`.
/// Supports:
/// - Numeric values (e.g., "4.0") -> Fixed
/// - Time-based values (e.g., "1measure", "2beats", "0.5s", "500ms") -> Measures/Beats/Seconds
///
/// `kind_label` is used in error messages (e.g. "frequency" or "speed").
/// For beats/measures, requires tempo_map to be available.
pub(crate) fn parse_tempo_aware_string(
    value: &str,
    tempo_map: &Option<TempoMap>,
    kind_label: &str,
) -> Result<TempoAwareValue, Box<dyn Error>> {
    let value = value.trim();

    // Try parsing as a simple number first — fixed rate
    if let Ok(val) = value.parse::<f64>() {
        return Ok(TempoAwareValue::Fixed(val));
    }

    // Try parsing as a time-based value
    if value.ends_with("ms") {
        let num_str = value.trim_end_matches("ms");
        let num = num_str.parse::<f64>()?;
        let duration_secs = num / 1000.0;
        Ok(TempoAwareValue::Seconds(duration_secs))
    } else if value.ends_with("measures") {
        let num_str = value.trim_end_matches("measures");
        let num = num_str.parse::<f64>()?;
        if tempo_map.is_some() {
            Ok(TempoAwareValue::Measures(num))
        } else {
            Err(format!("Measure-based {kind_label} values require a tempo section").into())
        }
    } else if value.ends_with("beats") {
        let num_str = value.trim_end_matches("beats");
        let num = num_str.parse::<f64>()?;
        if tempo_map.is_some() {
            Ok(TempoAwareValue::Beats(num))
        } else {
            Err(format!("Beat-based {kind_label} values require a tempo section").into())
        }
    } else if value.ends_with('s') {
        let num_str = value.trim_end_matches('s');
        let num = num_str.parse::<f64>()?;
        Ok(TempoAwareValue::Seconds(num))
    } else {
        // Fallback: try parsing as a number
        Ok(TempoAwareValue::Fixed(value.parse::<f64>()?))
    }
}

/// Parses a frequency value to TempoAwareFrequency (type alias for TempoAwareValue).
pub(crate) fn parse_frequency_string(
    value: &str,
    tempo_map: &Option<TempoMap>,
) -> Result<TempoAwareFrequency, Box<dyn Error>> {
    parse_tempo_aware_string(value, tempo_map, "frequency")
}

/// Parses a speed value to TempoAwareSpeed (type alias for TempoAwareValue).
pub(crate) fn parse_speed_string(
    value: &str,
    tempo_map: &Option<TempoMap>,
) -> Result<TempoAwareSpeed, Box<dyn Error>> {
    parse_tempo_aware_string(value, tempo_map, "speed")
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
    let value = value.trim();
    if value.ends_with("ms") {
        let num_str = value.trim_end_matches("ms");
        let num = num_str.parse::<u64>()?;
        Ok(Duration::from_millis(num))
    } else if value.ends_with("measures") {
        let num_str = value.trim_end_matches("measures");
        let num = num_str.parse::<f64>()?;
        if let Some(tm) = tempo_map {
            let time = at_time.unwrap_or(Duration::ZERO);
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

    if clean_value.starts_with('#') {
        // Hex color — delegate to Color::from_hex
        Color::from_hex(clean_value).ok()
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
        // Named color — delegate to the canonical Color::from_name
        Color::from_name(clean_value).ok()
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
                key = inner_pair.as_str().trim().to_string();
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
                value = inner_pair.as_str().trim().to_string();
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
    Ok(pair.as_str().trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── parse_percentage_to_f64 ────────────────────────────────────

    #[test]
    fn percentage_with_percent_sign() {
        assert!((parse_percentage_to_f64("50%").unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn percentage_100_percent() {
        assert!((parse_percentage_to_f64("100%").unwrap() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn percentage_0_percent() {
        assert!((parse_percentage_to_f64("0%").unwrap() - 0.0).abs() < 1e-9);
    }

    #[test]
    fn percentage_without_percent_sign() {
        assert!((parse_percentage_to_f64("0.75").unwrap() - 0.75).abs() < 1e-9);
    }

    #[test]
    fn percentage_with_whitespace() {
        assert!((parse_percentage_to_f64("  50%  ").unwrap() - 0.5).abs() < 1e-9);
    }

    #[test]
    fn percentage_invalid() {
        assert!(parse_percentage_to_f64("abc%").is_err());
    }

    // ── parse_frequency_string ─────────────────────────────────────

    #[test]
    fn freq_fixed_number() {
        let f = parse_frequency_string("4.0", &None).unwrap();
        assert_eq!(f, TempoAwareFrequency::Fixed(4.0));
    }

    #[test]
    fn freq_milliseconds() {
        let f = parse_frequency_string("500ms", &None).unwrap();
        assert_eq!(f, TempoAwareFrequency::Seconds(0.5));
    }

    #[test]
    fn freq_seconds() {
        let f = parse_frequency_string("2.5s", &None).unwrap();
        assert_eq!(f, TempoAwareFrequency::Seconds(2.5));
    }

    #[test]
    fn freq_measures_with_tempo() {
        let tm = TempoMap::new(
            Duration::ZERO,
            120.0,
            crate::lighting::tempo::TimeSignature::new(4, 4),
            vec![],
        );
        let f = parse_frequency_string("2measures", &Some(tm)).unwrap();
        assert_eq!(f, TempoAwareFrequency::Measures(2.0));
    }

    #[test]
    fn freq_measures_without_tempo_errors() {
        assert!(parse_frequency_string("2measures", &None).is_err());
    }

    #[test]
    fn freq_beats_with_tempo() {
        let tm = TempoMap::new(
            Duration::ZERO,
            120.0,
            crate::lighting::tempo::TimeSignature::new(4, 4),
            vec![],
        );
        let f = parse_frequency_string("4beats", &Some(tm)).unwrap();
        assert_eq!(f, TempoAwareFrequency::Beats(4.0));
    }

    #[test]
    fn freq_beats_without_tempo_errors() {
        assert!(parse_frequency_string("4beats", &None).is_err());
    }

    #[test]
    fn freq_invalid() {
        assert!(parse_frequency_string("notanumber", &None).is_err());
    }

    // ── parse_speed_string ─────────────────────────────────────────

    #[test]
    fn speed_fixed_number() {
        let s = parse_speed_string("1.5", &None).unwrap();
        assert_eq!(s, TempoAwareSpeed::Fixed(1.5));
    }

    #[test]
    fn speed_milliseconds() {
        let s = parse_speed_string("250ms", &None).unwrap();
        assert_eq!(s, TempoAwareSpeed::Seconds(0.25));
    }

    #[test]
    fn speed_seconds() {
        let s = parse_speed_string("3s", &None).unwrap();
        assert_eq!(s, TempoAwareSpeed::Seconds(3.0));
    }

    #[test]
    fn speed_measures_with_tempo() {
        let tm = TempoMap::new(
            Duration::ZERO,
            120.0,
            crate::lighting::tempo::TimeSignature::new(4, 4),
            vec![],
        );
        let s = parse_speed_string("1measures", &Some(tm)).unwrap();
        assert_eq!(s, TempoAwareSpeed::Measures(1.0));
    }

    #[test]
    fn speed_measures_without_tempo_errors() {
        assert!(parse_speed_string("1measures", &None).is_err());
    }

    #[test]
    fn speed_beats_with_tempo() {
        let tm = TempoMap::new(
            Duration::ZERO,
            120.0,
            crate::lighting::tempo::TimeSignature::new(4, 4),
            vec![],
        );
        let s = parse_speed_string("2beats", &Some(tm)).unwrap();
        assert_eq!(s, TempoAwareSpeed::Beats(2.0));
    }

    #[test]
    fn speed_beats_without_tempo_errors() {
        assert!(parse_speed_string("2beats", &None).is_err());
    }

    // ── parse_duration_string ──────────────────────────────────────

    #[test]
    fn duration_milliseconds() {
        let d = parse_duration_string("500ms", &None, None, 0.0).unwrap();
        assert_eq!(d, Duration::from_millis(500));
    }

    #[test]
    fn duration_seconds_unit() {
        let d = parse_duration_string("2.5s", &None, None, 0.0).unwrap();
        assert!((d.as_secs_f64() - 2.5).abs() < 1e-9);
    }

    #[test]
    fn duration_seconds_no_unit() {
        let d = parse_duration_string("3.0", &None, None, 0.0).unwrap();
        assert!((d.as_secs_f64() - 3.0).abs() < 1e-9);
    }

    #[test]
    fn duration_measures_with_tempo() {
        let tm = TempoMap::new(
            Duration::ZERO,
            120.0,
            crate::lighting::tempo::TimeSignature::new(4, 4),
            vec![],
        );
        let d = parse_duration_string("1measures", &Some(tm), None, 0.0).unwrap();
        // 1 measure = 4 beats at 120 BPM = 2.0 seconds
        assert!((d.as_secs_f64() - 2.0).abs() < 0.01);
    }

    #[test]
    fn duration_measures_without_tempo_errors() {
        assert!(parse_duration_string("1measures", &None, None, 0.0).is_err());
    }

    #[test]
    fn duration_beats_with_tempo() {
        let tm = TempoMap::new(
            Duration::ZERO,
            60.0,
            crate::lighting::tempo::TimeSignature::new(4, 4),
            vec![],
        );
        let d = parse_duration_string("2beats", &Some(tm), None, 0.0).unwrap();
        // 2 beats at 60 BPM = 2.0 seconds
        assert!((d.as_secs_f64() - 2.0).abs() < 0.01);
    }

    #[test]
    fn duration_beats_without_tempo_errors() {
        assert!(parse_duration_string("2beats", &None, None, 0.0).is_err());
    }

    #[test]
    fn duration_invalid() {
        assert!(parse_duration_string("notanumber", &None, None, 0.0).is_err());
    }

    // ── parse_color_string ─────────────────────────────────────────

    #[test]
    fn color_hex() {
        let c = parse_color_string("#FF0000").unwrap();
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn color_hex_quoted() {
        let c = parse_color_string("\"#00FF00\"").unwrap();
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 255);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn color_hex_lowercase() {
        let c = parse_color_string("#ff8000").unwrap();
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 128);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn color_hex_invalid_length() {
        assert!(parse_color_string("#FFF").is_none());
    }

    #[test]
    fn color_hex_invalid_chars() {
        assert!(parse_color_string("#GGHHII").is_none());
    }

    #[test]
    fn color_rgb_notation() {
        let c = parse_color_string("rgb(128,64,32)").unwrap();
        assert_eq!(c.r, 128);
        assert_eq!(c.g, 64);
        assert_eq!(c.b, 32);
    }

    #[test]
    fn color_rgb_with_spaces() {
        let c = parse_color_string("rgb(255, 0, 128)").unwrap();
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 128);
    }

    #[test]
    fn color_rgb_invalid_components() {
        // Only 2 components
        assert!(parse_color_string("rgb(128,64)").is_none());
    }

    #[test]
    fn color_named() {
        let c = parse_color_string("red").unwrap();
        assert_eq!(c.r, 255);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 0);
    }

    #[test]
    fn color_named_blue() {
        let c = parse_color_string("blue").unwrap();
        assert_eq!(c.r, 0);
        assert_eq!(c.g, 0);
        assert_eq!(c.b, 255);
    }

    #[test]
    fn color_unknown_name() {
        assert!(parse_color_string("chartreuse").is_none());
    }

    // ── parse_measure_time ─────────────────────────────────────────

    #[test]
    fn measure_time_basic() {
        let (m, b) = parse_measure_time("@12/1").unwrap();
        assert_eq!(m, 12);
        assert!((b - 1.0).abs() < 1e-9);
    }

    #[test]
    fn measure_time_fractional_beat() {
        let (m, b) = parse_measure_time("@4/2.5").unwrap();
        assert_eq!(m, 4);
        assert!((b - 2.5).abs() < 1e-9);
    }

    #[test]
    fn measure_time_no_at() {
        let (m, b) = parse_measure_time("8/3").unwrap();
        assert_eq!(m, 8);
        assert!((b - 3.0).abs() < 1e-9);
    }

    #[test]
    fn measure_time_invalid_format() {
        assert!(parse_measure_time("@12").is_err());
        assert!(parse_measure_time("@12/1/2").is_err());
    }

    #[test]
    fn measure_time_invalid_measure() {
        assert!(parse_measure_time("@abc/1").is_err());
    }

    #[test]
    fn measure_time_invalid_beat() {
        assert!(parse_measure_time("@12/xyz").is_err());
    }

    // ── parse_time_string ──────────────────────────────────────────

    #[test]
    fn time_string_mm_ss() {
        let d = parse_time_string("1:30").unwrap();
        assert_eq!(d, Duration::from_millis(90_000));
    }

    #[test]
    fn time_string_mm_ss_mmm() {
        let d = parse_time_string("2:15.500").unwrap();
        assert_eq!(d, Duration::from_millis(135_500));
    }

    #[test]
    fn time_string_ss_only() {
        let d = parse_time_string("45").unwrap();
        assert_eq!(d, Duration::from_millis(45_000));
    }

    #[test]
    fn time_string_ss_mmm() {
        let d = parse_time_string("30.250").unwrap();
        assert_eq!(d, Duration::from_millis(30_250));
    }

    #[test]
    fn time_string_with_at_prefix() {
        let d = parse_time_string("@1:00").unwrap();
        assert_eq!(d, Duration::from_millis(60_000));
    }

    #[test]
    fn time_string_fractional_ms_truncation() {
        // "30.1" → 1 digit → scaled to 100ms
        let d = parse_time_string("30.1").unwrap();
        assert_eq!(d, Duration::from_millis(30_100));
    }

    #[test]
    fn time_string_two_digit_ms() {
        // "30.25" → 2 digits → scaled to 250ms
        let d = parse_time_string("30.25").unwrap();
        assert_eq!(d, Duration::from_millis(30_250));
    }

    #[test]
    fn time_string_long_ms_truncated() {
        // "30.1234" → truncated to 3 digits → 123ms
        let d = parse_time_string("30.1234").unwrap();
        assert_eq!(d, Duration::from_millis(30_123));
    }

    #[test]
    fn time_string_zero() {
        let d = parse_time_string("0").unwrap();
        assert_eq!(d, Duration::ZERO);
    }

    #[test]
    fn time_string_zero_colon() {
        let d = parse_time_string("0:00").unwrap();
        assert_eq!(d, Duration::ZERO);
    }
}
