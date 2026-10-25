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
    /// How songs hand over to each other, for every entry that does not say
    /// otherwise.
    #[serde(default, skip_serializing_if = "Transitions::is_empty")]
    transitions: Transitions,
    /// The songs that belong to this playlist.
    songs: Vec<PlaylistEntry>,
}

/// How songs hand over to each other: the playlist's defaults, or an entry's
/// own settings.
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Transitions {
    /// How a looping song fades out when a loop break (play, next or
    /// previous) leaves it, playing on under the song it gives way to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out: Option<FadeConfig>,
    /// How a song fades in when it starts under a song fading out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_in: Option<FadeConfig>,
}

impl Transitions {
    fn is_empty(&self) -> bool {
        self.fade_out.is_none() && self.fade_in.is_none()
    }
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

/// A playlist entry with its own transitions, which replace the playlist's
/// defaults setting by setting.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ConfiguredEntry {
    /// The song's name.
    pub song: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fade_out: Option<FadeConfig>,
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

/// An entry's transitions, parsed, with the playlist's defaults filled in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntryTransitions {
    /// How the song fades out when a loop break leaves it.
    pub fade_out: Option<Fade>,
    /// How the song fades in under a song fading out.
    pub fade_in: Option<Fade>,
}

/// A fade as written in a playlist: a length alone, or with its settings.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
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
            transitions: Transitions::default(),
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

    /// The playlist's default transitions.
    pub fn transitions(&self) -> &Transitions {
        &self.transitions
    }

    /// Each entry's transitions, in order: its own settings, each falling
    /// back to the playlist's default.
    pub fn entry_transitions(&self) -> Result<Vec<EntryTransitions>, String> {
        let parse = |fade: Option<&FadeConfig>, field: &str, lights: bool, at: &str| {
            fade.map(|fade| fade.parse(field, lights))
                .transpose()
                .map_err(|e| format!("{at}: {e}"))
        };
        let default_out = parse(
            self.transitions.fade_out.as_ref(),
            "fade_out",
            true,
            "transitions",
        )?;
        let default_in = parse(
            self.transitions.fade_in.as_ref(),
            "fade_in",
            false,
            "transitions",
        )?;
        self.songs
            .iter()
            .map(|entry| {
                let (fade_out, fade_in) = match entry {
                    PlaylistEntry::Song(_) => (None, None),
                    PlaylistEntry::Configured(entry) => {
                        (entry.fade_out.as_ref(), entry.fade_in.as_ref())
                    }
                };
                let at = format!("song {}", entry.song());
                Ok(EntryTransitions {
                    fade_out: match parse(fade_out, "fade_out", true, &at)? {
                        Some(fade) => Some(fade),
                        None => default_out.clone(),
                    },
                    fade_in: match parse(fade_in, "fade_in", false, &at)? {
                        Some(fade) => Some(fade),
                        None => default_in.clone(),
                    },
                })
            })
            .collect()
    }

    /// This playlist with `songs` as its songs, in that order, for an editor
    /// that knows only the names. The defaults carry over, and each entry's
    /// own settings with the first entry left of the same song.
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
            transitions: self.transitions.clone(),
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
    fn entries_take_transitions_and_fall_back_to_the_defaults() {
        let p = parse(
            r#"
transitions:
  fade_out: 1measure
  fade_in: 2beats
songs:
  - 1999
  - song: Prelude
    fade_out:
      length: 4s
      curve: linear
      exclude: [click]
      lights: 0s
  - song: Opener
    fade_in: 0s
"#,
        )
        .unwrap();
        assert_eq!(p.songs(), ["1999", "Prelude", "Opener"]);
        let t = p.entry_transitions().unwrap();
        assert_eq!(
            t[0].fade_out.as_ref().unwrap().length,
            LoopCrossfade::Measures(1.0)
        );
        assert_eq!(
            t[0].fade_in.as_ref().unwrap().length,
            LoopCrossfade::Beats(2.0)
        );
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
        assert_eq!(
            t[1].fade_in.as_ref().unwrap().length,
            LoopCrossfade::Beats(2.0)
        );
        // Its own setting replaces the default: zero, no fade.
        assert_eq!(
            t[2].fade_in.as_ref().unwrap().length,
            LoopCrossfade::Time(std::time::Duration::ZERO)
        );
    }

    #[test]
    fn bad_transitions_are_reported() {
        let p = parse("songs:\n  - song: A\n    fade_out: {length: 2s, curve: wobbly}\n").unwrap();
        let e = p.entry_transitions().unwrap_err();
        assert!(e.contains("song A") && e.contains("wobbly"), "{e}");
        let p = parse("transitions:\n  fade_in: {length: 2s, lights: 1s}\nsongs: []\n").unwrap();
        let e = p.entry_transitions().unwrap_err();
        assert!(e.contains("lights applies only to fade_out"), "{e}");
        assert!(parse("songs:\n  - song: A\n    fade_ot: 2s\n").is_err());
    }

    #[test]
    fn with_songs_keeps_each_entrys_transitions() {
        let p = parse(
            r#"
transitions:
  fade_out: 2s
songs:
  - A
  - song: B
    fade_in: 1s
"#,
        )
        .unwrap();
        let q = p.with_songs(&["B".to_string(), "C".to_string(), "B".to_string()]);
        assert_eq!(q.songs(), ["B", "C", "B"]);
        assert_eq!(q.transitions(), p.transitions());
        assert_eq!(q.entries()[0], p.entries()[1]);
        assert_eq!(q.entries()[2], PlaylistEntry::Song("B".to_string()));

        // And round-trips through YAML, names alone staying names alone.
        let yaml = crate::util::to_yaml_string(&q).unwrap();
        assert!(yaml.contains("- C"), "{yaml}");
        assert_eq!(parse(&yaml).unwrap(), q);
    }
}
