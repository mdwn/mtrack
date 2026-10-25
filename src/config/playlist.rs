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
use std::path::Path;

use config::{Config, File};
use serde::{Deserialize, Serialize};

use super::error::ConfigError;
use super::song::LoopCrossfade;

/// The configuration for a playlist.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Playlist {
    /// Identifies this config file as a playlist.
    #[serde(default = "default_playlist_kind")]
    kind: super::kind::ConfigKind,
    /// The songs that belong to this playlist.
    songs: Vec<PlaylistEntry>,
}

/// A song in a playlist: its name alone, or with its transitions.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum PlaylistEntry {
    /// The song's name.
    Song(String),
    /// The song's name with its transitions.
    Configured(ConfiguredEntry),
}

/// A playlist entry with its own transitions.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConfiguredEntry {
    /// The song's name.
    pub song: String,
    /// How the song fades out when a loop break (play, next or previous)
    /// leaves it, playing on under the song it gives way to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out: Option<FadeConfig>,
    /// How the song fades in when it starts under a song fading out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in: Option<FadeConfig>,
}

/// By hand rather than untagged: a song called `1999` reaches us as a
/// number, which an untagged string variant would turn away.
impl<'de> Deserialize<'de> for PlaylistEntry {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct EntryVisitor;
        impl<'de> serde::de::Visitor<'de> for EntryVisitor {
            type Value = PlaylistEntry;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a song name, or a map with `song` and its transitions")
            }
            fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
                Ok(PlaylistEntry::Song(v.to_string()))
            }
            fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
                Ok(PlaylistEntry::Song(v.to_string()))
            }
            fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(PlaylistEntry::Song(v.to_string()))
            }
            fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Ok(PlaylistEntry::Song(v.to_string()))
            }
            fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Self::Value, E> {
                Ok(PlaylistEntry::Song(v.to_string()))
            }
            fn visit_map<A: serde::de::MapAccess<'de>>(
                self,
                map: A,
            ) -> Result<Self::Value, A::Error> {
                ConfiguredEntry::deserialize(serde::de::value::MapAccessDeserializer::new(map))
                    .map(PlaylistEntry::Configured)
            }
        }
        deserializer.deserialize_any(EntryVisitor)
    }
}

impl PlaylistEntry {
    /// The song's name.
    pub fn song(&self) -> &str {
        match self {
            PlaylistEntry::Song(song) => song,
            PlaylistEntry::Configured(entry) => &entry.song,
        }
    }
}

/// An entry's transitions, parsed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntryTransitions {
    /// How the song fades out when a loop break leaves it.
    pub fade_out: Option<Fade>,
    /// How the song fades in under a song fading out.
    pub fade_in: Option<Fade>,
}

/// A fade as written in a playlist: a length alone, or with its settings.
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

/// A fade's curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FadeCurve {
    /// Quick in, slow out: the level holds up through the fade.
    EqualPower,
    /// A straight ramp in amplitude.
    Linear,
}

/// A fade's settings, parsed: see [`FadeConfig`].
#[derive(Clone, Debug, PartialEq)]
pub struct Fade {
    pub length: LoopCrossfade,
    pub curve: FadeCurve,
    /// `None` for the default.
    pub exclude: Option<Vec<String>>,
    /// `None` for the default (the fade's length); `Some(Time(0))` switches
    /// at once.
    pub lights: Option<LoopCrossfade>,
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

impl FadeConfig {
    /// Parses the fade `field` sets. Only a fade-out (`lights`) crosses the
    /// lights.
    pub(crate) fn parse(&self, field: &str, lights: bool) -> Result<Fade, String> {
        let (length, curve, exclude, light) = match self {
            FadeConfig::Length(length) => (length, None, None, None),
            FadeConfig::Full {
                length,
                curve,
                exclude,
                lights,
            } => (length, curve.as_deref(), exclude.clone(), lights.as_deref()),
        };
        let curve = match curve.map(str::trim) {
            None | Some("equal_power") => FadeCurve::EqualPower,
            Some("linear") => FadeCurve::Linear,
            Some(other) => {
                return Err(format!(
                    "{field}: unknown curve {other:?} (expected equal_power or linear)"
                ))
            }
        };
        let light = match light {
            Some(_) if !lights => {
                return Err(format!(
                    "{field}: lights applies only to fade_out, which crosses one song's \
                     show into the next"
                ))
            }
            None => None,
            Some(value) => Some(parse_length(&format!("{field}.lights"), value)?),
        };
        Ok(Fade {
            length: parse_length(field, length)?,
            curve,
            exclude,
            lights: light,
        })
    }
}

/// A fade's length: as a loop crossfade, or zero for none.
fn parse_length(field: &str, value: &str) -> Result<LoopCrossfade, String> {
    if value.trim().trim_end_matches(['m', 's']).trim() == "0" {
        return Ok(LoopCrossfade::Time(std::time::Duration::ZERO));
    }
    LoopCrossfade::parse_field(field, value)
}

impl Playlist {
    /// Creates a new playlist configuration.
    pub fn new(songs: &[String]) -> Playlist {
        Playlist {
            kind: super::kind::ConfigKind::Playlist,
            songs: songs.iter().cloned().map(PlaylistEntry::Song).collect(),
        }
    }

    /// Parse a playlist from a YAML file.
    pub fn deserialize(path: &Path) -> Result<Playlist, ConfigError> {
        Ok(Config::builder()
            .add_source(File::from(path))
            .build()?
            .try_deserialize::<Playlist>()?)
    }

    /// The names of the songs in the playlist, in order.
    pub fn songs(&self) -> Vec<String> {
        self.songs.iter().map(|e| e.song().to_string()).collect()
    }

    /// The playlist's entries, in order.
    pub fn entries(&self) -> &[PlaylistEntry] {
        &self.songs
    }

    /// Each entry's transitions, parsed, in order.
    pub fn entry_transitions(&self) -> Result<Vec<EntryTransitions>, String> {
        self.songs
            .iter()
            .map(|entry| {
                let PlaylistEntry::Configured(entry) = entry else {
                    return Ok(EntryTransitions::default());
                };
                let parse = |fade: &Option<FadeConfig>, field: &str, lights: bool| {
                    fade.as_ref()
                        .map(|fade| fade.parse(field, lights))
                        .transpose()
                        .map_err(|e| format!("song {}: {e}", entry.song))
                };
                Ok(EntryTransitions {
                    fade_out: parse(&entry.fade_out, "fade_out", true)?,
                    fade_in: parse(&entry.fade_in, "fade_in", false)?,
                })
            })
            .collect()
    }

    /// This playlist with `songs` as its songs, in that order, for an editor
    /// that knows only the names. Each entry's settings carry over with the
    /// first entry left of the same song.
    pub fn with_songs(&self, songs: &[String]) -> Playlist {
        let mut left: Vec<Option<&PlaylistEntry>> = self.songs.iter().map(Some).collect();
        let songs = songs
            .iter()
            .map(|name| {
                left.iter_mut()
                    .find(|e| e.is_some_and(|e| e.song() == name))
                    .and_then(Option::take)
                    .cloned()
                    .unwrap_or_else(|| PlaylistEntry::Song(name.clone()))
            })
            .collect();
        Playlist {
            kind: self.kind.clone(),
            songs,
        }
    }
}

fn default_playlist_kind() -> super::kind::ConfigKind {
    super::kind::ConfigKind::Playlist
}

#[cfg(test)]
mod tests {
    use super::*;
    use config::{Config, File, FileFormat};

    #[test]
    fn new_creates_playlist() {
        let songs = vec!["song1".to_string(), "song2".to_string()];
        let p = Playlist::new(&songs);
        assert_eq!(p.songs().len(), 2);
        assert_eq!(p.songs()[0], "song1");
        assert_eq!(p.songs()[1], "song2");
    }

    #[test]
    fn new_empty() {
        let p = Playlist::new(&[]);
        assert!(p.songs().is_empty());
    }

    #[test]
    fn deserialize_yaml() {
        let yaml = r#"
songs:
  - "Track A"
  - "Track B"
  - "Track C"
"#;
        let p: Playlist = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(p.songs().len(), 3);
        assert_eq!(p.songs()[0], "Track A");
    }

    #[test]
    fn deserialize_empty_songs() {
        let yaml = r#"
songs: []
"#;
        let p: Playlist = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert!(p.songs().is_empty());
    }

    #[test]
    fn serialize_roundtrip() {
        let songs = vec![
            "Alpha".to_string(),
            "Bravo".to_string(),
            "Charlie".to_string(),
        ];
        let p = Playlist::new(&songs);
        let serialized = crate::util::to_yaml_string(&p).unwrap();
        let deserialized: Playlist = config::Config::builder()
            .add_source(config::File::from_str(
                &serialized,
                config::FileFormat::Yaml,
            ))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(deserialized.songs().len(), 3);
        assert_eq!(deserialized.songs()[0], "Alpha");
        assert_eq!(deserialized.songs()[1], "Bravo");
        assert_eq!(deserialized.songs()[2], "Charlie");
    }

    #[test]
    fn deserialize_single_song() {
        let yaml = r#"
songs:
  - "Only Song"
"#;
        let p: Playlist = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(p.songs().len(), 1);
        assert_eq!(p.songs()[0], "Only Song");
    }

    fn parse(yaml: &str) -> Result<Playlist, config::ConfigError> {
        Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()?
            .try_deserialize()
    }

    #[test]
    fn entries_take_their_own_transitions() {
        let p = parse(
            r#"
songs:
  - 1999
  - song: Prelude
    fade_out:
      length: 4s
      curve: linear
      exclude: [click]
      lights: 0s
  - song: Opener
    fade_in: 2beats
"#,
        )
        .unwrap();
        assert_eq!(p.songs(), ["1999", "Prelude", "Opener"]);
        let t = p.entry_transitions().unwrap();
        assert_eq!(t[0], EntryTransitions::default());
        let out = t[1].fade_out.as_ref().unwrap();
        assert_eq!(
            out.length,
            LoopCrossfade::Time(std::time::Duration::from_secs(4))
        );
        assert_eq!(out.curve, FadeCurve::Linear);
        assert_eq!(out.exclude.as_deref(), Some(&["click".to_string()][..]));
        assert_eq!(
            out.lights,
            Some(LoopCrossfade::Time(std::time::Duration::ZERO))
        );
        assert!(t[1].fade_in.is_none());
        assert_eq!(
            t[2].fade_in.as_ref().unwrap().length,
            LoopCrossfade::Beats(2.0)
        );
        assert!(t[2].fade_out.is_none());
    }

    #[test]
    fn bad_transitions_are_reported() {
        let p = parse("songs:\n  - song: A\n    fade_out: {length: 2s, curve: wobbly}\n").unwrap();
        let e = p.entry_transitions().unwrap_err();
        assert!(e.contains("song A") && e.contains("wobbly"), "{e}");
        let p = parse("songs:\n  - song: A\n    fade_in: {length: 2s, lights: 1s}\n").unwrap();
        let e = p.entry_transitions().unwrap_err();
        assert!(e.contains("lights applies only to fade_out"), "{e}");
        assert!(parse("songs:\n  - song: A\n    fade_ot: 2s\n").is_err());
        assert!(
            parse("songs:\n  - song: A\n    fade_out: {length: 2s, exlude: [click]}\n").is_err(),
            "a misspelt setting is refused"
        );
    }

    #[test]
    fn a_bare_zero_turns_a_fade_off() {
        let p = parse("songs:\n  - song: A\n    fade_out: 0\n  - song: B\n    fade_out: {length: 2s, lights: 0}\n").unwrap();
        let t = p.entry_transitions().unwrap();
        let zero = LoopCrossfade::Time(std::time::Duration::ZERO);
        assert_eq!(t[0].fade_out.as_ref().unwrap().length, zero);
        assert_eq!(t[1].fade_out.as_ref().unwrap().lights, Some(zero));
    }

    #[test]
    fn with_songs_keeps_each_entrys_transitions() {
        let p = parse(
            r#"
songs:
  - A
  - song: B
    fade_in: 1s
"#,
        )
        .unwrap();
        let q = p.with_songs(&["B".to_string(), "C".to_string(), "B".to_string()]);
        assert_eq!(q.songs(), ["B", "C", "B"]);
        assert_eq!(q.entries()[0], p.entries()[1]);
        assert_eq!(q.entries()[2], PlaylistEntry::Song("B".to_string()));

        // And round-trips through YAML, names alone staying names alone.
        let yaml = crate::util::to_yaml_string(&q).unwrap();
        assert!(yaml.contains("- C"), "{yaml}");
        assert_eq!(parse(&yaml).unwrap(), q);
    }
}
