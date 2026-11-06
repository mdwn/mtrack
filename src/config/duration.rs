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

//! A config's durations: light, so the web UI's WebAssembly module compiles
//! the config files that read them.

use std::time::Duration;

/// A duration as a config string (`500ms`, `2s`): the one parse of one,
/// for every field that takes one. The error is the parser's; a validator
/// words its own.
pub(crate) fn parse_duration_string(raw: &str) -> Result<Duration, duration_string::Error> {
    duration_string::DurationString::from_string(raw.to_string()).map(Duration::from)
}

/// Parses an optional duration string (e.g. "500ms") into a `Duration`,
/// returning `default` when `raw` is `None`.
pub(crate) fn parse_playback_delay(
    raw: &Option<String>,
    default: Duration,
) -> Result<Duration, Box<dyn std::error::Error>> {
    match raw {
        Some(s) => Ok(parse_duration_string(s)?),
        None => Ok(default),
    }
}
