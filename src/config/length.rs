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

//! A length a config writes in time or in the song's beats or measures: a
//! looping song's crossfade, a playlist fade. And the config's clock, the
//! one reading of a length of time (`auto_play`, ambient timings,
//! `playback_delay`, a fade's time). Std alone, so the web UI's WebAssembly
//! module reads lengths by the player's own rule.

use std::time::Duration;

/// A unit a length is written in after its number.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DurationUnit {
    Ms,
    S,
    Beats,
    Measures,
}

/// Each unit as a length spells it after a number, and after a count of
/// one: the one table, read both ways, by the config's lengths and the
/// light show DSL's (`lighting::parser::utils`).
const UNITS: [(DurationUnit, &str, &str); 4] = [
    (DurationUnit::Ms, "ms", "ms"),
    (DurationUnit::S, "s", "s"),
    (DurationUnit::Beats, "beats", "beat"),
    (DurationUnit::Measures, "measures", "measure"),
];

impl DurationUnit {
    /// The unit `count` of it is written with: `1beat`, `2beats`.
    pub fn keyword(self, count: f64) -> &'static str {
        let (_, plural, one) = UNITS.iter().find(|(u, ..)| *u == self).expect("every unit");
        if count == 1.0 {
            one
        } else {
            plural
        }
    }

    /// The unit `text` spells, in the plural or the singular.
    pub fn from_keyword(text: &str) -> Option<DurationUnit> {
        UNITS
            .iter()
            .find(|(_, plural, one)| text == *plural || text == *one)
            .map(|(unit, ..)| *unit)
    }
}

/// A value written as a number and its unit -- `2.5s`, `4beats`, `45deg`
/// -- split into the number as written and the unit: the letters it ends
/// in, `""` for a bare number. Each caller takes the units it knows and
/// reports its own errors.
pub(crate) fn split_unit(value: &str) -> (&str, &str) {
    let value = value.trim();
    let number = value.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    (number, &value[number.len()..])
}

/// The clock's units beyond the table's milliseconds and seconds: minutes
/// and hours, as seconds. Not in [`UNITS`], which the light show DSL reads
/// too: there `m` is measures (a tempo transition's `2m`).
const CLOCK_UNITS: [(&str, f64); 2] = [("m", 60.0), ("h", 3600.0)];

/// `number` of the clock unit `unit`, in seconds; `None` for no clock unit.
fn clock_unit_secs(number: f64, unit: &str) -> Option<f64> {
    match DurationUnit::from_keyword(unit) {
        Some(DurationUnit::Ms) => Some(number / 1000.0),
        Some(DurationUnit::S) => Some(number),
        Some(DurationUnit::Beats | DurationUnit::Measures) => None,
        None => CLOCK_UNITS
            .iter()
            .find(|(keyword, _)| *keyword == unit)
            .map(|(_, secs)| number * secs),
    }
}

/// Where the number leading `text` ends: at its first letter, an
/// exponent's `e` aside (`1e3s`).
fn number_end(text: &str) -> usize {
    let bytes = text.as_bytes();
    let mut at = 0;
    while at < bytes.len() {
        let exponent = at > 0
            && matches!(bytes[at], b'e' | b'E')
            && bytes
                .get(at + 1)
                .is_some_and(|next| next.is_ascii_digit() || matches!(next, b'+' | b'-'));
        if bytes[at].is_ascii_alphabetic() && !exponent {
            break;
        }
        at += 1;
    }
    at
}

/// A length of time as written on the clock, in seconds: decimal numbers,
/// each with its unit -- `ms`, `s`, `m` (minutes) or `h` -- summed
/// (`500ms`, `1.5s`, `2m`, `1m30s`; whitespace anywhere is ignored). The
/// seconds may be negative or not finite; `None` for text that is no
/// length of time.
fn parse_clock_secs(value: &str) -> Option<f64> {
    let text: String = value.chars().filter(|c| !c.is_whitespace()).collect();
    let mut rest = text.as_str();
    let mut total: Option<f64> = None;
    while !rest.is_empty() {
        let (number, tail) = rest.split_at(number_end(rest));
        let unit_end = tail
            .find(|c: char| !c.is_ascii_alphabetic())
            .unwrap_or(tail.len());
        let (unit, next) = tail.split_at(unit_end);
        // A sign leads the length, not a term within it.
        if total.is_some() && number.starts_with(['+', '-']) {
            return None;
        }
        let secs = clock_unit_secs(number.parse().ok()?, unit)?;
        total = Some(total.unwrap_or(0.0) + secs);
        rest = next;
    }
    total
}

/// A length of time as a config writes it (see [`parse_clock_secs`]):
/// the one reading, for every field that takes one. `None` for text that
/// is no length, or a negative one or one too long to hold; the field
/// words its own error.
pub fn parse_clock(value: &str) -> Option<Duration> {
    let secs = parse_clock_secs(value)?;
    if secs >= 0.0 {
        // `+ 0.0`: a `-0s` is none.
        Duration::try_from_secs_f64(secs + 0.0).ok()
    } else {
        None
    }
}

/// A length of time as the clock writes it, in seconds: `1.5s`. It reads
/// back as itself ([`parse_clock`]).
pub fn clock_text(length: Duration) -> String {
    let secs = length.as_secs_f64();
    format!("{secs}{}", DurationUnit::S.keyword(secs))
}

/// How long consecutive passes of a looping song overlap.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LoopCrossfade {
    /// A fixed length of time.
    Time(std::time::Duration),
    /// A number of beats, counted back from the end of the song.
    Beats(f64),
    /// A number of measures, counted back from the end of the song.
    Measures(f64),
}

impl std::fmt::Display for LoopCrossfade {
    /// As it would be written in song.yaml.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoopCrossfade::Time(length) => f.write_str(&clock_text(*length)),
            LoopCrossfade::Beats(beats) => {
                write!(f, "{beats}{}", DurationUnit::Beats.keyword(*beats))
            }
            LoopCrossfade::Measures(measures) => {
                write!(f, "{measures}{}", DurationUnit::Measures.keyword(*measures))
            }
        }
    }
}

impl LoopCrossfade {
    /// Parses `"6s"`, `"500ms"`, `"2m"`, `"8beats"` or `"2measures"`
    /// (singular units too; a time as [`parse_clock`] reads one). The
    /// length must be greater than zero.
    pub fn parse(value: &str) -> Result<LoopCrossfade, String> {
        LoopCrossfade::parse_field("loop_crossfade", value)
    }

    /// Parses a length as [`LoopCrossfade::parse`] does, errors naming
    /// `field`.
    pub fn parse_field(field: &str, value: &str) -> Result<LoopCrossfade, String> {
        let value = value.trim();
        let invalid = || {
            format!(
                "{field}: invalid length {value:?} (expected e.g. \"6s\", \"500ms\", \
                 \"8beats\" or \"2measures\")"
            )
        };
        let bounded = |n: f64| -> Result<f64, String> {
            if !(n.is_finite() && n > 0.0) {
                return Err(format!("{field}: {value:?} must be greater than zero"));
            }
            // Far past any song, in any unit; a bound, so a typo cannot
            // overflow the time arithmetic downstream.
            if n > 1e6 {
                return Err(format!("{field}: {value:?} is longer than any song"));
            }
            Ok(n)
        };
        let (number, unit) = split_unit(value);
        let count = || number.trim().parse().map_err(|_| invalid());
        match DurationUnit::from_keyword(unit) {
            Some(DurationUnit::Beats) => Ok(LoopCrossfade::Beats(bounded(count()?)?)),
            Some(DurationUnit::Measures) => Ok(LoopCrossfade::Measures(bounded(count()?)?)),
            _ => {
                let secs = bounded(parse_clock_secs(value).ok_or_else(invalid)?)?;
                Ok(LoopCrossfade::Time(Duration::from_secs_f64(secs)))
            }
        }
    }
}

impl LoopCrossfade {
    /// The length as a reader is shown it: `4s`, `1 beat`, `2 measures`.
    pub fn text(&self) -> String {
        match self {
            LoopCrossfade::Time(_) => self.to_string(),
            LoopCrossfade::Beats(beats) => {
                format!("{beats} {}", DurationUnit::Beats.keyword(*beats))
            }
            LoopCrossfade::Measures(measures) => {
                format!("{measures} {}", DurationUnit::Measures.keyword(*measures))
            }
        }
    }

    /// Whether the length is none: a cut, not a fade.
    pub fn is_cut(&self) -> bool {
        *self == LoopCrossfade::Time(Duration::ZERO)
    }
}

/// A fade's length: as a loop crossfade, or zero for none (`0`, `0s`,
/// `0ms`, `0m`).
pub fn parse_length(field: &str, value: &str) -> Result<LoopCrossfade, String> {
    if value.trim() == "0" || parse_clock(value) == Some(Duration::ZERO) {
        return Ok(LoopCrossfade::Time(Duration::ZERO));
    }
    LoopCrossfade::parse_field(field, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_clock_reads_decimals_in_ms_s_m_and_h() {
        let at = |text| parse_clock(text);
        assert_eq!(at("500ms"), Some(Duration::from_millis(500)));
        assert_eq!(at("2s"), Some(Duration::from_secs(2)));
        assert_eq!(at("1.5s"), Some(Duration::from_millis(1500)));
        assert_eq!(at("2.5ms"), Some(Duration::from_micros(2500)));
        assert_eq!(at("2m"), Some(Duration::from_secs(120)));
        assert_eq!(at("1h"), Some(Duration::from_secs(3600)));
        assert_eq!(at("1m30s"), Some(Duration::from_secs(90)));
        assert_eq!(at(" 1 m 30 s "), Some(Duration::from_secs(90)));
        assert_eq!(at("1e3ms"), Some(Duration::from_secs(1)));
        assert_eq!(at("0s"), Some(Duration::ZERO));
        assert_eq!(at("-0s"), Some(Duration::ZERO));
        for refused in [
            "",
            "soon",
            "2",
            "5beats",
            "2measures",
            "-1s",
            "1s-1s",
            "1d",
            "100us",
            "infs",
            "1x",
            "s",
        ] {
            assert_eq!(at(refused), None, "{refused:?}");
        }
    }

    #[test]
    fn a_clock_length_reads_back_as_written() {
        for length in [
            Duration::from_millis(1500),
            Duration::from_millis(1),
            Duration::from_secs(120),
            Duration::from_nanos(123_456_789),
        ] {
            assert_eq!(parse_clock(&clock_text(length)), Some(length));
            let fade = LoopCrossfade::Time(length);
            assert_eq!(LoopCrossfade::parse(&fade.to_string()), Ok(fade));
        }
        for fade in [
            LoopCrossfade::Beats(1.0),
            LoopCrossfade::Beats(2.5),
            LoopCrossfade::Measures(1.0),
            LoopCrossfade::Measures(4.0),
        ] {
            assert_eq!(LoopCrossfade::parse(&fade.to_string()), Ok(fade));
        }
        assert_eq!(LoopCrossfade::Beats(1.0).text(), "1 beat");
        assert_eq!(LoopCrossfade::Measures(2.0).text(), "2 measures");
        assert_eq!(
            LoopCrossfade::Time(Duration::from_millis(1500)).text(),
            "1.5s"
        );
    }

    #[test]
    fn a_fade_takes_the_clock_and_beats_or_measures() {
        assert_eq!(
            LoopCrossfade::parse("2m"),
            Ok(LoopCrossfade::Time(Duration::from_secs(120)))
        );
        assert_eq!(
            LoopCrossfade::parse("1.5s"),
            Ok(LoopCrossfade::Time(Duration::from_millis(1500)))
        );
        assert_eq!(
            LoopCrossfade::parse("8 beats"),
            Ok(LoopCrossfade::Beats(8.0))
        );
        let err = |text| LoopCrossfade::parse(text).unwrap_err();
        assert!(err("0s").contains("must be greater than zero"));
        assert!(err("-2s").contains("must be greater than zero"));
        assert!(err("2000000s").contains("longer than any song"));
        assert!(err("soon").contains("invalid length"));
        for cut in ["0", "0s", "0ms", "0m", " 0 s "] {
            assert!(parse_length("fade_out", cut).unwrap().is_cut(), "{cut:?}");
        }
    }
}
