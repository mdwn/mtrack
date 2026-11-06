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

//! Ambient light's mode, as the operator sets it and every surface reports
//! it: light, so the web UI's WebAssembly module compiles it as it is.

use serde::Serialize;

/// What the operator asks of ambient light. Set at run time and not kept:
/// every start is [`AmbientMode::Auto`].
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AmbientMode {
    /// Between songs and under a song without lights, after its delay.
    #[default]
    Auto,
    /// Now, over whatever plays -- a song's own show included -- until the
    /// mode goes back to `Auto`.
    On,
    /// None: ambient playing fades out to dark. Songs light as ever.
    Off,
}

impl AmbientMode {
    /// The mode's name: `auto`, `on` or `off`.
    pub fn as_str(self) -> &'static str {
        match self {
            AmbientMode::Auto => "auto",
            AmbientMode::On => "on",
            AmbientMode::Off => "off",
        }
    }

    /// The mode named `name` (`auto`, `on` or `off`, any case).
    pub fn parse(name: &str) -> Option<AmbientMode> {
        match name.trim().to_ascii_lowercase().as_str() {
            "auto" => Some(AmbientMode::Auto),
            "on" => Some(AmbientMode::On),
            "off" => Some(AmbientMode::Off),
            _ => None,
        }
    }
}
