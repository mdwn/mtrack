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

use serde::{Deserialize, Serialize};

/// What the operator asks of ambient light. Set at run time and not kept:
/// every start is [`AmbientMode::Auto`].
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
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
    /// Every mode, in the order their names are listed.
    pub const ALL: [AmbientMode; 3] = [AmbientMode::Auto, AmbientMode::On, AmbientMode::Off];

    /// The modes' names as a sentence lists them: `auto, on or off`.
    pub fn names() -> String {
        let names: Vec<&str> = AmbientMode::ALL.iter().map(|m| m.as_str()).collect();
        match names.split_last() {
            Some((last, [])) => last.to_string(),
            Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
            None => String::new(),
        }
    }

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
        let name = name.trim();
        AmbientMode::ALL
            .into_iter()
            .find(|mode| mode.as_str().eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use super::AmbientMode;

    #[test]
    fn every_mode_parses_from_its_name_and_is_listed() {
        for mode in AmbientMode::ALL {
            assert_eq!(AmbientMode::parse(mode.as_str()), Some(mode));
            assert_eq!(
                AmbientMode::parse(&mode.as_str().to_ascii_uppercase()),
                Some(mode)
            );
        }
        assert_eq!(AmbientMode::names(), "auto, on or off");
        assert_eq!(AmbientMode::parse("dim"), None);
    }
}
