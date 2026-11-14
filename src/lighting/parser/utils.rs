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

use super::super::effects::{
    BlendMode, Color, EffectLayer, LayerSet, TempoAwareFrequency, TempoAwareSpeed, TempoAwareValue,
};
use super::super::tempo::TempoMap;
use super::grammar::Rule;
use pest::iterators::Pair;

/// The items a parsed file holds -- its includes, fixture types, venues,
/// shows, sequences and tempo blocks -- out of the `file` rule's pair.
pub(crate) fn items<'i>(
    pairs: pest::iterators::Pairs<'i, Rule>,
) -> impl Iterator<Item = pest::iterators::Pair<'i, Rule>> {
    pairs.flat_map(|p| {
        if p.as_rule() == Rule::file {
            p.into_inner().collect::<Vec<_>>()
        } else {
            vec![p]
        }
    })
}

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

/// A time as written, in its unit.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum TimeValue {
    Seconds(f64),
    Beats(f64),
    Measures(f64),
}

pub(crate) use crate::config::length::split_unit;
/// The unit a time is written in: the one table, which the config's
/// lengths read too.
pub use crate::config::length::DurationUnit;

/// The units a time is written in -- `500ms` or `1.5ms`, `0.5s`, `2beats`,
/// `1measure`, or a bare number, which is seconds -- read as the number
/// written and its unit. The one table; each caller takes the units it
/// allows and reports its own errors.
pub(crate) fn parse_time_unit(
    value: &str,
) -> Result<(f64, DurationUnit), std::num::ParseFloatError> {
    let (number, unit) = split_unit(value);
    Ok(match DurationUnit::from_keyword(unit) {
        Some(unit) => (number.trim().parse()?, unit),
        // Assume seconds if no unit
        None => (value.trim().parse()?, DurationUnit::S),
    })
}

/// A time as [`parse_time_unit`] reads it, in its unit ([`TimeValue`]).
pub(crate) fn parse_time_value(value: &str) -> Result<TimeValue, std::num::ParseFloatError> {
    Ok(match parse_time_unit(value)? {
        (ms, DurationUnit::Ms) => TimeValue::Seconds(ms / 1000.0),
        (secs, DurationUnit::S) => TimeValue::Seconds(secs),
        (beats, DurationUnit::Beats) => TimeValue::Beats(beats),
        (measures, DurationUnit::Measures) => TimeValue::Measures(measures),
    })
}

/// A quoted value (`"intro"`, a focus point's `"stage"`) without its
/// quotes: the one quote rule.
pub(crate) fn unquoted(text: &str) -> &str {
    text.trim_matches('"')
}

/// A quoted name (a show's, a sequence's, an include's path) without its
/// quotes ([`unquoted`]).
pub(crate) fn unquote(pair: &Pair<Rule>) -> String {
    unquoted(pair.as_str()).to_string()
}

/// The layer a `layer:` name denotes in the given set.
pub(crate) fn parse_layer_in(layers: &LayerSet, value: &str) -> Option<EffectLayer> {
    layers.parse(value)
}

/// The error for a `layer:` name the player does not have: the name, where
/// it was written, and the layers the player has, bottom to top. `column` is
/// the 1-based column of the parameter on its line.
pub(crate) fn unknown_layer(layers: &LayerSet, name: &str, column: usize) -> String {
    format!(
        "unknown layer '{}' (column {}); this player's layers, bottom to top, are: {}",
        name,
        column,
        layers.names().join(", ")
    )
}

/// Each blend mode and its keyword, as the grammar writes it: the one
/// table both ways are read off.
pub(crate) const BLEND_KEYWORDS: [(BlendMode, &str); 5] = [
    (BlendMode::Replace, "replace"),
    (BlendMode::Multiply, "multiply"),
    (BlendMode::Add, "add"),
    (BlendMode::Overlay, "overlay"),
    (BlendMode::Screen, "screen"),
];

/// The blend mode a `blend_mode:` keyword names.
pub(crate) fn parse_blend_mode(value: &str) -> Option<BlendMode> {
    BLEND_KEYWORDS
        .iter()
        .find(|(_, k)| *k == value)
        .map(|(mode, _)| *mode)
}

impl BlendMode {
    /// The blend mode's keyword, as the grammar writes it.
    pub fn keyword(self) -> &'static str {
        BLEND_KEYWORDS
            .iter()
            .find(|(mode, _)| *mode == self)
            .map(|(_, k)| *k)
            .expect("every blend mode has a keyword")
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

    match parse_time_value(value)? {
        TimeValue::Seconds(secs) => Ok(TempoAwareValue::Seconds(secs)),
        TimeValue::Measures(num) => {
            if tempo_map.is_some() {
                Ok(TempoAwareValue::Measures(num))
            } else {
                Err(format!("Measure-based {kind_label} values require a tempo section").into())
            }
        }
        TimeValue::Beats(num) => {
            if tempo_map.is_some() {
                Ok(TempoAwareValue::Beats(num))
            } else {
                Err(format!("Beat-based {kind_label} values require a tempo section").into())
            }
        }
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
    match parse_time_value(value)? {
        TimeValue::Seconds(secs) => Ok(Duration::from_secs_f64(secs)),
        TimeValue::Measures(num) => {
            if let Some(tm) = tempo_map {
                let time = at_time.unwrap_or(Duration::ZERO);
                Ok(tm.measures_to_duration(num, time, offset_secs))
            } else {
                Err("Measure-based durations require a tempo section".into())
            }
        }
        TimeValue::Beats(num) => {
            if let Some(tm) = tempo_map {
                let time = at_time.unwrap_or(Duration::ZERO);
                Ok(tm.beats_to_duration(num, time, offset_secs))
            } else {
                Err("Beat-based durations require a tempo section".into())
            }
        }
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

/// An `mm:ss.mmm` time's minutes, and its seconds as written (`ss.mmm`);
/// the minutes are 0 when it has none. The one reader of the form: a cue's
/// `@` time and a playback position a client sends as text
/// ([`crate::util::parse_position`]) are both written in it.
pub(crate) fn split_minutes(time: &str) -> Result<(u64, &str), std::num::ParseIntError> {
    match time.split_once(':') {
        Some((minutes, seconds)) => Ok((minutes.parse()?, seconds)),
        None => Ok((0, time)),
    }
}

/// A cue's `@mm:ss.mmm` or `@ss.mmm` time, to the whole millisecond: digits
/// past the third are dropped.
pub(crate) fn parse_time_string(time_str: &str) -> Result<Duration, Box<dyn Error>> {
    let (minutes, seconds_part) = split_minutes(time_str.trim_start_matches('@'))?;
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
}

pub(crate) fn parse_parameter(pair: Pair<Rule>) -> Result<(String, String), Box<dyn Error>> {
    let mut key = String::new();
    let mut value = String::new();

    for inner_pair in pair.into_inner() {
        match inner_pair.as_rule() {
            Rule::parameter_name => {
                // A quoted key ("dimmer:section_a") names a channel whose
                // name is not an identifier; the quotes are not part of it.
                key = inner_pair.as_str().trim().trim_matches('"').to_string();
            }
            Rule::color_parameter => {
                value = parse_color_parameter(inner_pair)?;
            }
            Rule::degrees_parameter
            | Rule::percentage
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
    fn milliseconds_may_be_fractional_and_whole_ones_are_as_they_were() {
        assert_eq!(
            parse_duration_string("1.5ms", &None, None, 0.0).unwrap(),
            Duration::from_micros(1500)
        );
        assert_eq!(
            parse_speed_string("2.5ms", &None).unwrap(),
            TempoAwareSpeed::Seconds(0.0025)
        );
        // Whole milliseconds were read as an integer into `from_millis`;
        // through the one table they land on the same nanosecond, a day
        // in included.
        for ms in (0..5000).chain([59_999, 60_000, 3_600_000, 86_400_000]) {
            assert_eq!(
                parse_duration_string(&format!("{ms}ms"), &None, None, 0.0).unwrap(),
                Duration::from_millis(ms),
                "{ms}ms"
            );
        }
        assert_eq!(
            parse_time_value("3 s").unwrap(),
            TimeValue::Seconds(3.0),
            "the number is trimmed, as a tempo start's was"
        );
    }

    #[test]
    fn duration_invalid() {
        assert!(parse_duration_string("notanumber", &None, None, 0.0).is_err());
        // A unit written twice is no unit; it used to be stripped twice over.
        assert!(parse_duration_string("5msms", &None, None, 0.0).is_err());
        assert!(parse_duration_string("2ss", &None, None, 0.0).is_err());
    }

    // ── split_unit ───────────────────────────────────────────────

    #[test]
    fn split_unit_takes_the_trailing_letters() {
        assert_eq!(split_unit(" 2.5s "), ("2.5", "s"));
        assert_eq!(split_unit("4beats"), ("4", "beats"));
        assert_eq!(split_unit("-20.5deg"), ("-20.5", "deg"));
        assert_eq!(split_unit("3 measures"), ("3 ", "measures"));
        assert_eq!(split_unit("1e3"), ("1e3", ""));
        assert_eq!(split_unit("0.5"), ("0.5", ""));
        assert_eq!(split_unit("abc"), ("", "abc"));
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
    fn time_string_counts_the_digits_not_a_float() {
        // 2.01 s is 2009.999… ms through an f64: read off the digits, 2010.
        assert_eq!(
            parse_time_string("@2.01").unwrap(),
            Duration::from_millis(2_010)
        );
        assert_eq!(
            parse_time_string("@1:04.3519").unwrap(),
            Duration::from_millis(64_351)
        );
        assert!(parse_time_string("@99999999999999999999:00.000").is_err());
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
