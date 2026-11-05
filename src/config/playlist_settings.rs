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

//! A playlist's transitions and ambient light as written -- its entries'
//! fades and auto play, its own and its entries' ambient settings -- with
//! how they read and write: light, so the web UI's WebAssembly module
//! compiles them as they are. What they mean is `playlist`'s and
//! `ambient`'s.

use serde::{Deserialize, Serialize};

/// A fade as written in a playlist: a length alone, or with its settings.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum FadeConfig {
    /// `"3s"`, `"500ms"`, `"6beats"`, `"1measure"`; `"0s"` for none.
    Length(String),
    /// The length with its settings.
    Full {
        /// As for the short form.
        length: String,
        /// `equal_power` (the default) or `linear`. Equal power fades in
        /// quickly and out slowly, holding the level up; linear is a straight
        /// ramp in amplitude.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        curve: Option<String>,
        /// Tracks that do not fade: on a fade-out they cut at once, on a
        /// fade-in they start at full level. A fade-out defaults to the
        /// song's `loop_crossfade_cut` tracks, a fade-in to none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exclude: Option<Vec<String>>,
        /// A fade-out only: how long the lights take to cross into the next
        /// song's show, `0s` to switch at once. Defaults to `length`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lights: Option<String>,
    },
}

/// [`FadeConfig::Full`] as read, unknown settings refused.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FullFade {
    #[serde(deserialize_with = "string_or_number")]
    length: String,
    #[serde(default)]
    curve: Option<String>,
    #[serde(default)]
    exclude: Option<Vec<String>>,
    #[serde(default, deserialize_with = "optional_string_or_number")]
    lights: Option<String>,
}

/// By hand rather than untagged: `fade_out: 0` reaches us as a number, and
/// a misspelt setting should be refused, not ignored.
impl<'de> Deserialize<'de> for FadeConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct FadeVisitor;
        impl<'de> serde::de::Visitor<'de> for FadeVisitor {
            type Value = FadeConfig;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a fade length, or a map with `length` and its settings")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(FadeConfig::Length(v.to_string()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(FadeConfig::Length(v.to_string()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(FadeConfig::Length(v.to_string()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Ok(FadeConfig::Length(v.to_string()))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<Self::Value, A::Error> {
                let full =
                    FullFade::deserialize(serde::de::value::MapAccessDeserializer::new(map))?;
                Ok(FadeConfig::Full {
                    length: full.length,
                    curve: full.curve,
                    exclude: full.exclude,
                    lights: full.lights,
                })
            }
        }
        deserializer.deserialize_any(FadeVisitor)
    }
}

/// A length written as a string or, for a bare `0`, a number.
fn string_or_number<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    match FadeConfig::deserialize(deserializer)? {
        FadeConfig::Length(length) => Ok(length),
        FadeConfig::Full { .. } => Err(serde::de::Error::custom("expected a length")),
    }
}

fn optional_string_or_number<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    string_or_number(deserializer).map(Some)
}

/// An entry's fades as written, for an editor that sets them per entry:
/// `None` for none.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct EntryFades {
    #[serde(default)]
    pub fade_out: Option<FadeConfig>,
    #[serde(default)]
    pub fade_in: Option<FadeConfig>,
}

/// An entry's `auto_play` as written: `true` (at once), `false`, or the gap
/// before it starts (`2s`, `500ms`).
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum AutoPlayConfig {
    On(bool),
    Gap(String),
}

/// By hand: a bool or a gap, a bare number being seconds.
impl<'de> Deserialize<'de> for AutoPlayConfig {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct AutoPlayVisitor;
        impl<'de> serde::de::Visitor<'de> for AutoPlayVisitor {
            type Value = AutoPlayConfig;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("true, false, or the gap before the song starts (e.g. 2s)")
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(AutoPlayConfig::On(v))
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(match v.trim() {
                    "true" => AutoPlayConfig::On(true),
                    "false" => AutoPlayConfig::On(false),
                    _ => AutoPlayConfig::Gap(v.to_string()),
                })
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(AutoPlayConfig::Gap(format!("{v}s")))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(AutoPlayConfig::Gap(format!("{v}s")))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Ok(AutoPlayConfig::Gap(format!("{}ms", (v * 1000.0).round())))
            }
        }
        deserializer.deserialize_any(AutoPlayVisitor)
    }
}

/// Ambient light as written: under `dmx.lighting.ambient`, or a playlist's
/// `ambient`, whose settings override the player's one by one.
#[cfg_attr(feature = "tsify", derive(tsify_next::Tsify), tsify(missing_as_null))]
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Ambient {
    /// The `.light` file to play, relative to the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) show: Option<String>,
    /// How long the last song's frame holds before ambient fades in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) delay: Option<String>,
    /// How long ambient takes to fade in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) fade_in: Option<String>,
    /// How long ambient takes to cross into a song's show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) fade_out: Option<String>,
}

/// A playlist's `ambient`: `false` for none, or settings of its own.
/// Its TypeScript type is `false | Ambient`, which the payloads that carry
/// one state (the serializer is by hand).
#[derive(Clone, Debug, PartialEq)]
pub enum PlaylistAmbient {
    /// No ambient light with this playlist.
    Off,
    /// The player's ambient light, with these settings overriding it.
    On(Ambient),
}

impl Serialize for PlaylistAmbient {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            PlaylistAmbient::Off => serializer.serialize_bool(false),
            PlaylistAmbient::On(ambient) => ambient.serialize(serializer),
        }
    }
}

/// By hand: `false` turns ambient off, `true` keeps the player's, and a map
/// overrides its settings.
impl<'de> Deserialize<'de> for PlaylistAmbient {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct AmbientVisitor;
        impl<'de> serde::de::Visitor<'de> for AmbientVisitor {
            type Value = PlaylistAmbient;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("false, or a map of ambient settings")
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(if v {
                    PlaylistAmbient::On(Ambient::default())
                } else {
                    PlaylistAmbient::Off
                })
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<Self::Value, A::Error> {
                Ambient::deserialize(serde::de::value::MapAccessDeserializer::new(map))
                    .map(PlaylistAmbient::On)
            }
        }
        deserializer.deserialize_any(AmbientVisitor)
    }
}
