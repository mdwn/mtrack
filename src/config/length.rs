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
//! looping song's crossfade, a playlist fade. Std alone, so the web UI's
//! WebAssembly module reads lengths by the player's own rule.

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
            LoopCrossfade::Time(length) => write!(f, "{}s", length.as_secs_f64()),
            LoopCrossfade::Beats(beats) if *beats == 1.0 => write!(f, "1beat"),
            LoopCrossfade::Beats(beats) => write!(f, "{beats}beats"),
            LoopCrossfade::Measures(measures) if *measures == 1.0 => write!(f, "1measure"),
            LoopCrossfade::Measures(measures) => write!(f, "{measures}measures"),
        }
    }
}

impl LoopCrossfade {
    /// Parses `"6s"`, `"500ms"`, `"8beats"` or `"2measures"` (singular units
    /// too). The length must be greater than zero.
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
        let number = |n: &str| -> Result<f64, String> {
            let n: f64 = n.trim().parse().map_err(|_| invalid())?;
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
        if let Some(n) = value
            .strip_suffix("measures")
            .or_else(|| value.strip_suffix("measure"))
        {
            Ok(LoopCrossfade::Measures(number(n)?))
        } else if let Some(n) = value
            .strip_suffix("beats")
            .or_else(|| value.strip_suffix("beat"))
        {
            Ok(LoopCrossfade::Beats(number(n)?))
        } else if let Some(n) = value.strip_suffix("ms") {
            Ok(LoopCrossfade::Time(std::time::Duration::from_secs_f64(
                number(n)? / 1000.0,
            )))
        } else if let Some(n) = value.strip_suffix('s') {
            Ok(LoopCrossfade::Time(std::time::Duration::from_secs_f64(
                number(n)?,
            )))
        } else {
            Err(invalid())
        }
    }
}

impl LoopCrossfade {
    /// The length as a reader is shown it: `4s`, `1 beat`, `2 measures`.
    pub fn text(&self) -> String {
        match self {
            LoopCrossfade::Time(_) => self.to_string(),
            LoopCrossfade::Beats(beats) if *beats == 1.0 => "1 beat".to_string(),
            LoopCrossfade::Beats(beats) => format!("{beats} beats"),
            LoopCrossfade::Measures(measures) if *measures == 1.0 => "1 measure".to_string(),
            LoopCrossfade::Measures(measures) => format!("{measures} measures"),
        }
    }

    /// Whether the length is none: a cut, not a fade.
    pub fn is_cut(&self) -> bool {
        *self == LoopCrossfade::Time(std::time::Duration::ZERO)
    }
}

/// A fade's length: as a loop crossfade, or zero for none (`0`, `0s`,
/// `0ms`).
pub fn parse_length(field: &str, value: &str) -> Result<LoopCrossfade, String> {
    if value.trim().trim_end_matches(['m', 's']).trim() == "0" {
        return Ok(LoopCrossfade::Time(std::time::Duration::ZERO));
    }
    LoopCrossfade::parse_field(field, value)
}
