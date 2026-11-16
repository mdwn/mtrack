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

//! A playback position as text: read from what a client sends, and shown
//! as minutes and seconds.

use std::time::Duration;

/// Outputs the given duration in a minutes:seconds format.
pub fn duration_minutes_seconds(duration: Duration) -> String {
    minutes_seconds(duration.as_secs() as f64, 0)
}

/// `secs` as minutes and seconds, `1:15`, with `decimals` places of a
/// second when asked (`1:15.500`): cut, never rounded up, so a time shows
/// the second it is in. A negative time is 0; `decimals` past 6 are 6.
pub fn minutes_seconds(secs: f64, decimals: u32) -> String {
    let decimals = decimals.min(6);
    let scale = 10u64.pow(decimals);
    // A hair over, so a time written to these places (1.001) is not cut
    // to the one below it by its binary representation.
    let units = (secs.max(0.0) * scale as f64 + 1e-6).floor() as u64;
    let (minutes, rest) = (units / (60 * scale), units % (60 * scale));
    let (whole, part) = (rest / scale, rest % scale);
    match decimals {
        0 => format!("{minutes}:{whole:02}"),
        d => format!("{minutes}:{whole:02}.{part:0width$}", width = d as usize),
    }
}

/// A duration of `secs` seconds, or `None` for one no duration holds: a
/// negative or non-finite number, or one too large. For seconds a client
/// sends: `Duration::from_secs_f64` panics on these, and a panic there takes
/// down the task serving the client.
pub fn duration_from_secs(secs: f64) -> Option<Duration> {
    Duration::try_from_secs_f64(secs).ok()
}

/// A span of `secs` seconds as a `Duration`, saturating: none for a
/// negative (or NaN) span, the longest there is for one too long to hold.
/// For spans the player computes, where [`duration_from_secs`] is for
/// seconds a client sends.
pub fn duration_from_secs_saturating(secs: f64) -> Duration {
    Duration::try_from_secs_f64(secs.max(0.0)).unwrap_or(Duration::MAX)
}

/// A playback position a client sent as text: seconds (`90`, `90s`,
/// `1.5`, `1,5`) or minutes and seconds (`1:30`, `1:30.5s`, `1:30,5`), with
/// or without the `@` a cue position is written with (`@1:30`). The one
/// parser for a position as text; a position sent as a number of seconds
/// goes through [`duration_from_secs`], as this does. Errors are the
/// reason, for the surface to answer with.
pub fn parse_position(value: &str) -> Result<Duration, String> {
    let trimmed = value.trim();
    let trimmed = trimmed.strip_prefix('@').unwrap_or(trimmed).trim();
    // A decimal comma (`83,5`) is a decimal point.
    let decimal = trimmed.replacen(',', ".", 1);
    let trimmed = if trimmed.contains('.') {
        trimmed
    } else {
        decimal.as_str()
    };
    let seconds = if let Some(stripped) = trimmed
        .strip_suffix('s')
        .or_else(|| trimmed.strip_suffix('S'))
    {
        if stripped.contains(':') {
            parse_mm_ss(stripped)?
        } else {
            stripped
                .parse::<f64>()
                .map_err(|e| format!("invalid seconds: {e}"))?
        }
    } else if trimmed.contains(':') {
        parse_mm_ss(trimmed)?
    } else {
        trimmed
            .parse::<f64>()
            .map_err(|e| format!("invalid duration: {e}"))?
    };
    // Too large a number is turned down with the rest: converting it
    // panicked, failing the request with no answer.
    duration_from_secs(seconds)
        .ok_or_else(|| "duration must be a finite, non-negative number of seconds in range".into())
}

/// The seconds in an `mm:ss.mmm` time, to the bit: a position is not
/// rounded to the millisecond, as a cue's time is.
fn parse_mm_ss(s: &str) -> Result<f64, String> {
    let (minutes, ss) = crate::lighting::parser::utils::split_minutes(s)
        .map_err(|e| format!("invalid minutes: {e}"))?;
    let seconds: f64 = ss.parse().map_err(|e| format!("invalid seconds: {e}"))?;
    Ok((minutes as f64) * 60.0 + seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn minutes_seconds_cuts_to_its_places() {
        assert_eq!(minutes_seconds(75.5, 0), "1:15");
        assert_eq!(minutes_seconds(75.5, 1), "1:15.5");
        assert_eq!(minutes_seconds(75.5, 3), "1:15.500");
        assert_eq!(minutes_seconds(1.001, 3), "0:01.001");
        // The second it is in, never the next one's 60.
        assert_eq!(minutes_seconds(59.9996, 3), "0:59.999");
        assert_eq!(minutes_seconds(59.96, 1), "0:59.9");
        assert_eq!(minutes_seconds(-3.0, 0), "0:00");
        assert_eq!(minutes_seconds(3600.0, 0), "60:00");
    }

    #[test]
    fn a_position_takes_a_decimal_comma() {
        assert_eq!(parse_position("83,5"), Ok(Duration::from_secs_f64(83.5)));
        assert_eq!(parse_position("1:23,5"), Ok(Duration::from_secs_f64(83.5)));
        assert!(parse_position("1,2,3").is_err());
        assert!(parse_position("1.5,5").is_err());
    }
}
