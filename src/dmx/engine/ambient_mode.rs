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

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// What the operator asks of ambient light. Set at run time and not kept:
/// every start is [`AmbientMode::Auto`].
///
/// Its names come from one place, strum's `serialize_all`: [`Self::as_str`]
/// and [`Self::parse`] use them, and serde writes and reads through those,
/// so every surface takes a mode the same way (any case, spaces trimmed).
/// The TypeScript type says the same names (the `serde` attribute below is
/// read only by tsify).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify))]
#[cfg_attr(feature = "tsify", serde(rename_all = "lowercase"))]
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    strum::IntoStaticStr,
    strum::EnumString,
    strum::VariantArray,
)]
#[strum(serialize_all = "lowercase", ascii_case_insensitive)]
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
    /// Every mode, in the order their names are listed (derived, so a new
    /// mode is in it).
    pub const ALL: &'static [AmbientMode] = <AmbientMode as strum::VariantArray>::VARIANTS;

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
        self.into()
    }

    /// The mode named `name` (`auto`, `on` or `off`, any case, spaces
    /// trimmed).
    pub fn parse(name: &str) -> Option<AmbientMode> {
        name.trim().parse().ok()
    }

    /// [`Self::parse`], or the one refusal every surface gives for a name
    /// that is not a mode.
    pub fn parse_or_error(name: &str) -> Result<AmbientMode, String> {
        AmbientMode::parse(name).ok_or_else(|| {
            format!(
                "unknown ambient mode '{name}': use {}",
                AmbientMode::names()
            )
        })
    }
}

impl Serialize for AmbientMode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for AmbientMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        AmbientMode::parse_or_error(&name).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::AmbientMode;

    #[test]
    fn every_mode_parses_from_its_name_and_is_listed() {
        for &mode in AmbientMode::ALL {
            assert_eq!(AmbientMode::parse(mode.as_str()), Some(mode));
            assert_eq!(
                AmbientMode::parse(&mode.as_str().to_ascii_uppercase()),
                Some(mode)
            );
        }
        assert_eq!(AmbientMode::names(), "auto, on or off");
        assert_eq!(AmbientMode::parse("dim"), None);
    }

    /// serde writes a mode's name and reads it as `parse` does, so MCP
    /// takes `On` as gRPC does.
    #[test]
    fn serde_goes_through_the_names() {
        for &mode in AmbientMode::ALL {
            assert_eq!(serde_json::to_value(mode).unwrap(), mode.as_str());
            let upper = serde_json::json!(format!(" {} ", mode.as_str().to_uppercase()));
            assert_eq!(serde_json::from_value::<AmbientMode>(upper).unwrap(), mode);
        }
        assert!(serde_json::from_value::<AmbientMode>(serde_json::json!("dim")).is_err());
    }
}
