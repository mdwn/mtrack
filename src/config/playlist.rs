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
use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use config::Config;
use serde::{Deserialize, Serialize};

use super::error::ConfigError;
pub use super::fade::{FadeCurve, FadeStatus};
use super::length::{parse_length, LoopCrossfade};
pub use super::playlist_settings::{AutoPlayConfig, EntryFades, FadeConfig};

/// The configuration for a playlist.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq)]
pub struct Playlist {
    /// Identifies this config file as a playlist.
    #[serde(default = "default_playlist_kind")]
    kind: super::kind::ConfigKind,
    /// The songs that belong to this playlist.
    songs: Vec<PlaylistEntry>,
    /// Ambient light with this playlist: `false` for none, or settings
    /// overriding the player's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ambient: Option<super::ambient::PlaylistAmbient>,
}

/// A song in a playlist: its name alone, or with its transitions.
#[derive(Serialize, Clone, Debug, PartialEq)]
#[serde(untagged)]
pub enum PlaylistEntry {
    /// The song's name.
    Song(String),
    /// The song's name with its transitions.
    Configured(Box<ConfiguredEntry>),
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
    /// Whether the song starts by itself when the playlist reaches it -- the
    /// song before it ending, or next/previous while stopped -- and after
    /// how long a gap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_play: Option<AutoPlayConfig>,
    /// Ambient light in the gap before the song, while the player waits on
    /// it: `false` for none, or settings overriding the playlist's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ambient: Option<super::ambient::PlaylistAmbient>,
    /// Lighting palette overrides for this song in this playlist: a slot to
    /// the colour it plays in, laid over the song's own overrides and the
    /// show's defaults. A colour is anything the lighting DSL reads.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub palette: BTreeMap<String, String>,
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
                    .map(|entry| PlaylistEntry::Configured(Box::new(entry)))
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
    /// The gap before the song starts by itself when the playlist reaches
    /// it; `None` when it waits for play.
    pub auto_play: Option<std::time::Duration>,
    /// The palette slots the entry recolours, their colours read.
    pub palette: HashMap<String, crate::lighting::effects::Color>,
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

impl Fade {
    /// The fade as configured, for status reports.
    pub fn status(&self) -> FadeStatus {
        FadeStatus {
            length: self.length.to_string(),
            curve: self.curve,
            exclude: self.exclude.clone(),
            lights: self.lights.map(|l| l.to_string()),
        }
    }
}

impl Playlist {
    /// Creates a new playlist configuration.
    pub fn new(songs: &[String]) -> Playlist {
        Playlist {
            kind: super::kind::ConfigKind::Playlist,
            songs: songs.iter().cloned().map(PlaylistEntry::Song).collect(),
            ambient: None,
        }
    }

    /// The playlist's ambient light, as written.
    pub fn ambient(&self) -> Option<&super::ambient::PlaylistAmbient> {
        self.ambient.as_ref()
    }

    /// Parse a playlist from a YAML file.
    pub fn deserialize(path: &Path) -> Result<Playlist, ConfigError> {
        Ok(Config::builder()
            .add_source(crate::util::yaml_source(path)?)
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
                    auto_play: entry
                        .auto_play
                        .as_ref()
                        .map(AutoPlayConfig::parse)
                        .transpose()
                        .map_err(|e| format!("song {}: {e}", entry.song))?
                        .flatten(),
                    palette: crate::lighting::parser::parse_overrides(&entry.palette)
                        .map_err(|e| format!("song {}: {e}", entry.song))?,
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
            ambient: self.ambient.clone(),
        }
    }

    /// Each entry's fades as written, in order.
    pub fn entry_fades(&self) -> Vec<EntryFades> {
        self.songs
            .iter()
            .map(|entry| match entry {
                PlaylistEntry::Song(_) => EntryFades::default(),
                PlaylistEntry::Configured(entry) => EntryFades {
                    fade_out: entry.fade_out.clone(),
                    fade_in: entry.fade_in.clone(),
                },
            })
            .collect()
    }

    /// This playlist with each entry's `auto_play` set from `auto_play`, in
    /// entry order, for an editor that sets it per entry. An entry left with
    /// no settings is written as its name alone.
    pub fn with_auto_play(self, auto_play: &[Option<AutoPlayConfig>]) -> Playlist {
        self.edit_entries(auto_play, |configured, auto_play| {
            configured.auto_play = auto_play
                .clone()
                .filter(|a| *a != AutoPlayConfig::On(false));
        })
    }

    /// This playlist with each entry's `fade_out` and `fade_in` set from
    /// `fades`, in entry order, as [`Playlist::with_auto_play`] sets auto
    /// play.
    pub fn with_fades(self, fades: &[EntryFades]) -> Playlist {
        self.edit_entries(fades, |configured, fades| {
            configured.fade_out = fades.fade_out.clone();
            configured.fade_in = fades.fade_in.clone();
        })
    }

    /// Each entry's ambient light as written, in order: `None` where the
    /// entry sets none, and the playlist's plays.
    pub fn entry_ambient(&self) -> Vec<Option<super::ambient::PlaylistAmbient>> {
        self.songs
            .iter()
            .map(|entry| match entry {
                PlaylistEntry::Song(_) => None,
                PlaylistEntry::Configured(entry) => entry.ambient.clone(),
            })
            .collect()
    }

    /// This playlist with each entry's `ambient` set from `ambient`, in
    /// entry order, as [`Playlist::with_auto_play`] sets auto play.
    pub fn with_entry_ambient(
        self,
        ambient: &[Option<super::ambient::PlaylistAmbient>],
    ) -> Playlist {
        self.edit_entries(ambient, |configured, ambient| {
            configured.ambient = ambient.clone();
        })
    }

    /// Each entry's palette overrides as written, in order.
    pub fn entry_palettes(&self) -> Vec<BTreeMap<String, String>> {
        self.songs
            .iter()
            .map(|entry| match entry {
                PlaylistEntry::Song(_) => BTreeMap::new(),
                PlaylistEntry::Configured(entry) => entry.palette.clone(),
            })
            .collect()
    }

    /// This playlist with each entry's `palette` set from `palettes`, in
    /// entry order, as [`Playlist::with_auto_play`] sets auto play; `None`
    /// keeps the entry's own.
    pub fn with_entry_palettes(self, palettes: &[Option<BTreeMap<String, String>>]) -> Playlist {
        self.edit_entries(palettes, |configured, palette| {
            if let Some(palette) = palette {
                configured.palette = palette.clone();
            }
        })
    }

    /// Applies `edit` to each entry with its value from `values`, in order,
    /// writing an entry left with no settings as its name alone.
    fn edit_entries<T>(
        mut self,
        values: &[T],
        edit: impl Fn(&mut ConfiguredEntry, &T),
    ) -> Playlist {
        for (entry, value) in self.songs.iter_mut().zip(values) {
            let mut configured = match std::mem::replace(entry, PlaylistEntry::Song(String::new()))
            {
                PlaylistEntry::Song(song) => ConfiguredEntry {
                    song,
                    fade_out: None,
                    fade_in: None,
                    auto_play: None,
                    ambient: None,
                    palette: BTreeMap::new(),
                },
                PlaylistEntry::Configured(configured) => *configured,
            };
            edit(&mut configured, value);
            *entry = if configured.fade_out.is_none()
                && configured.fade_in.is_none()
                && configured.auto_play.is_none()
                && configured.ambient.is_none()
                && configured.palette.is_empty()
            {
                PlaylistEntry::Song(configured.song)
            } else {
                PlaylistEntry::Configured(Box::new(configured))
            };
        }
        self
    }

    /// This playlist with `ambient` as its ambient light (`None` for the
    /// profile's).
    pub fn with_ambient(mut self, ambient: Option<super::ambient::PlaylistAmbient>) -> Playlist {
        self.ambient = ambient;
        self
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
    fn auto_play_reads_true_false_or_a_gap_and_writes_back() {
        let yaml = "songs:\n- song: A\n  auto_play: true\n- song: B\n  auto_play: 2s\n\
                    - song: C\n  auto_play: false\n- song: D\n  auto_play: 3\n- E\n";
        let read = |text: &str| -> Playlist {
            Config::builder()
                .add_source(File::from_str(text, FileFormat::Yaml))
                .build()
                .unwrap()
                .try_deserialize()
                .unwrap()
        };
        let p = read(yaml);
        let gaps: Vec<_> = p
            .entry_transitions()
            .unwrap()
            .into_iter()
            .map(|t| t.auto_play)
            .collect();
        use std::time::Duration;
        assert_eq!(
            gaps,
            vec![
                Some(Duration::ZERO),
                Some(Duration::from_secs(2)),
                None,
                Some(Duration::from_secs(3)),
                None
            ]
        );
        let written = crate::util::to_yaml_string(&p).unwrap();
        assert_eq!(read(&written), p, "{written}");

        let bad = read("songs:\n- song: A\n  auto_play: soon\n");
        assert!(bad.entry_transitions().unwrap_err().contains("auto_play"));
    }

    #[test]
    fn an_entrys_ambient_reads_and_writes_back() {
        let yaml = "songs:\n- song: A\n  ambient: false\n- song: B\n  ambient:\n    show: b.light\n    fade_in: 2s\n- C\n";
        let playlist = parse(yaml).unwrap();
        let ambient = playlist.entry_ambient();
        assert_eq!(
            ambient[0],
            Some(super::super::ambient::PlaylistAmbient::Off)
        );
        assert!(matches!(
            &ambient[1],
            Some(super::super::ambient::PlaylistAmbient::On(a)) if a.show() == Some("b.light")
        ));
        assert_eq!(ambient[2], None);
        let written = crate::util::to_yaml_string(&playlist).unwrap();
        assert_eq!(parse(&written).unwrap(), playlist, "{written}");

        // Cleared, an entry with nothing else is its name alone again.
        let cleared = playlist.with_entry_ambient(&[None, None, None]);
        assert_eq!(cleared.songs(), ["A", "B", "C"]);
        assert!(cleared
            .entries()
            .iter()
            .all(|e| matches!(e, PlaylistEntry::Song(_))));
        assert!(parse("songs:\n- song: A\n  ambient:\n    shw: a.light\n").is_err());
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
    fn with_fades_sets_each_entrys_and_leaves_bare_names_bare() {
        let p = parse("songs:\n  - song: A\n    fade_in: 1s\n    auto_play: true\n  - song: B\n    fade_out: 2s\n  - C\n").unwrap();
        let out = FadeConfig::Full {
            length: "1measure".to_string(),
            curve: Some("linear".to_string()),
            exclude: Some(vec!["click".to_string()]),
            lights: Some("0s".to_string()),
        };
        let q = p.with_fades(&[
            EntryFades::default(),
            EntryFades::default(),
            EntryFades {
                fade_out: Some(out.clone()),
                fade_in: None,
            },
        ]);
        // A keeps its auto play, B is left with nothing, C takes the fade.
        assert_eq!(
            q.entry_fades(),
            vec![
                EntryFades::default(),
                EntryFades::default(),
                EntryFades {
                    fade_out: Some(out),
                    fade_in: None
                }
            ]
        );
        assert!(matches!(q.entries()[0], PlaylistEntry::Configured(_)));
        assert_eq!(q.entries()[1], PlaylistEntry::Song("B".to_string()));
        let yaml = crate::util::to_yaml_string(&q).unwrap();
        assert_eq!(parse(&yaml).unwrap(), q, "{yaml}");
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

    #[test]
    fn an_entrys_palette_is_read_and_checked() {
        use crate::lighting::effects::Color;
        let p = parse(
            r##"
songs:
  - A
  - song: B
    palette:
      primary: "#2040ff"
      secondary: cyan
"##,
        )
        .unwrap();
        let transitions = p.entry_transitions().unwrap();
        assert!(transitions[0].palette.is_empty());
        assert_eq!(
            transitions[1].palette["primary"],
            Color::new(0x20, 0x40, 0xff)
        );
        assert_eq!(transitions[1].palette["secondary"], Color::new(0, 255, 255));

        let bad = parse("songs:\n  - song: B\n    palette:\n      primary: nope\n").unwrap();
        let err = bad.entry_transitions().unwrap_err();
        assert!(err.contains("song B"), "{err}");
        assert!(err.contains("'primary'"), "{err}");
    }

    #[test]
    fn an_entrys_palette_is_kept_by_the_editors_and_is_settings_enough() {
        let p = parse("songs:\n  - A\n  - B\n").unwrap();
        let palettes = vec![
            BTreeMap::new(),
            BTreeMap::from([("primary".to_string(), "#2040ff".to_string())]),
        ];
        let some: Vec<_> = palettes.iter().cloned().map(Some).collect();
        let q = p.with_entry_palettes(&some);
        // A palette alone makes an entry a configured one, and none makes
        // it a name again.
        assert_eq!(q.entries()[0], PlaylistEntry::Song("A".to_string()));
        assert!(matches!(q.entries()[1], PlaylistEntry::Configured(_)));
        assert_eq!(q.entry_palettes(), palettes);
        let yaml = crate::util::to_yaml_string(&q).unwrap();
        assert_eq!(parse(&yaml).unwrap(), q, "{yaml}");

        // The other editors leave it, and a reorder carries it with its song.
        let r = q
            .with_auto_play(&[None, Some(AutoPlayConfig::On(true))])
            .with_songs(&["B".to_string(), "A".to_string()]);
        assert_eq!(r.entry_palettes()[0], palettes[1]);
        assert!(r.entry_palettes()[1].is_empty());
        let kept = r.clone().with_entry_palettes(&[None, None]);
        assert_eq!(kept.entry_palettes(), r.entry_palettes());
        let cleared = r.with_entry_palettes(&[Some(BTreeMap::new()), Some(BTreeMap::new())]);
        assert!(cleared.entry_palettes().iter().all(BTreeMap::is_empty));
    }
}
