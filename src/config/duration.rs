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

/// Parses an optional duration string (e.g. "500ms") by the config's clock
/// ([`super::length::parse_clock`]) into a `Duration`, returning `default`
/// when `raw` is `None`.
pub(crate) fn parse_playback_delay(
    raw: &Option<String>,
    default: Duration,
) -> Result<Duration, Box<dyn std::error::Error>> {
    match raw {
        Some(s) => super::length::parse_clock(s)
            .ok_or_else(|| format!("playback_delay '{s}' is not a valid duration").into()),
        None => Ok(default),
    }
}

