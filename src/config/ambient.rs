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

//! Ambient light: a show the rig plays while no song is.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// How long the last song's frame holds before ambient fades in.
pub const DEFAULT_AMBIENT_DELAY: Duration = Duration::ZERO;
/// How long ambient takes to fade in.
pub const DEFAULT_AMBIENT_FADE_IN: Duration = Duration::from_secs(2);
/// How long ambient takes to cross into a song's show.
pub const DEFAULT_AMBIENT_FADE_OUT: Duration = Duration::from_secs(1);

/// Ambient light as written: under `dmx.lighting.ambient`, or a playlist's
/// `ambient`, whose settings override the player's one by one.
#[derive(Deserialize, Serialize, Clone, Debug, Default, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Ambient {
    /// The `.light` file to play, relative to the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    show: Option<String>,
    /// How long the last song's frame holds before ambient fades in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    delay: Option<String>,
    /// How long ambient takes to fade in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fade_in: Option<String>,
    /// How long ambient takes to cross into a song's show.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fade_out: Option<String>,
}

/// A playlist's `ambient`: `false` for none, or settings of its own.
#[derive(Clone, Debug, PartialEq)]
pub enum PlaylistAmbient {
    /// No ambient light with this playlist.
    Off,
    /// The player's ambient light, with these settings overriding it.
    On(Ambient),
}

/// Ambient light, resolved: the show to play and its timings.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedAmbient {
    /// The `.light` file, as written (relative to the project).
    pub show: String,
    pub delay: Duration,
    pub fade_in: Duration,
    pub fade_out: Duration,
}

impl Ambient {
    /// Ambient light built in code.
    pub fn new(
        show: Option<String>,
        delay: Option<String>,
        fade_in: Option<String>,
        fade_out: Option<String>,
    ) -> Self {
        Ambient {
            show,
            delay,
            fade_in,
            fade_out,
        }
    }

    /// The `.light` file, as written.
    pub fn show(&self) -> Option<&str> {
        self.show.as_deref()
    }

    /// Checks the timings read, naming the first that does not.
    pub fn validate(&self) -> Result<(), String> {
        for (field, value) in [
            ("delay", &self.delay),
            ("fade_in", &self.fade_in),
            ("fade_out", &self.fade_out),
        ] {
            if let Some(value) = value {
                parse_duration(field, value)?;
            }
        }
        if self
            .show
            .as_deref()
            .is_some_and(|show| show.trim().is_empty())
        {
            return Err("ambient show: empty".to_string());
        }
        Ok(())
    }
}

fn parse_duration(field: &str, value: &str) -> Result<Duration, String> {
    duration_string::DurationString::from_string(value.to_string())
        .map(Duration::from)
        .map_err(|_| format!("ambient {field}: '{value}' is not a duration (e.g. 2s, 500ms)"))
}

/// Resolves ambient light from the player's settings and the playlist's: a
/// playlist setting wins, a missing one falls back to the player's, then to
/// the default. `None` with no show, or when the playlist turns it off.
pub fn resolve(
    player: Option<&Ambient>,
    playlist: Option<&PlaylistAmbient>,
) -> Result<Option<ResolvedAmbient>, String> {
    let playlist = match playlist {
        Some(PlaylistAmbient::Off) => return Ok(None),
        Some(PlaylistAmbient::On(ambient)) => Some(ambient),
        None => None,
    };
    let pick = |get: fn(&Ambient) -> &Option<String>| {
        playlist
            .and_then(|a| get(a).as_deref())
            .or_else(|| player.and_then(|a| get(a).as_deref()))
    };
    let Some(show) = pick(|a| &a.show) else {
        return Ok(None);
    };
    let timing = |field, get: fn(&Ambient) -> &Option<String>, default| {
        pick(get).map_or(Ok(default), |value| parse_duration(field, value))
    };
    Ok(Some(ResolvedAmbient {
        show: show.to_string(),
        delay: timing("delay", |a| &a.delay, DEFAULT_AMBIENT_DELAY)?,
        fade_in: timing("fade_in", |a| &a.fade_in, DEFAULT_AMBIENT_FADE_IN)?,
        fade_out: timing("fade_out", |a| &a.fade_out, DEFAULT_AMBIENT_FADE_OUT)?,
    }))
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

impl PlaylistAmbient {
    /// Checks the timings read.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            PlaylistAmbient::Off => Ok(()),
            PlaylistAmbient::On(ambient) => ambient.validate(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Deserialize)]
    struct Wrapper<T> {
        ambient: T,
    }

    /// Reads `ambient: <yaml>` the way config files are read.
    fn read<T: serde::de::DeserializeOwned>(yaml: &str) -> Result<T, config::ConfigError> {
        let text = format!("ambient:\n{}", indent(yaml));
        config::Config::builder()
            .add_source(config::File::from_str(&text, config::FileFormat::Yaml))
            .build()?
            .try_deserialize::<Wrapper<T>>()
            .map(|w| w.ambient)
    }

    fn indent(yaml: &str) -> String {
        if !yaml.contains(':') {
            return format!("  {yaml}\n");
        }
        yaml.lines().map(|line| format!("  {line}\n")).collect()
    }

    fn ambient(yaml: &str) -> Ambient {
        read(yaml).expect("ambient")
    }

    fn playlist(yaml: &str) -> PlaylistAmbient {
        read(yaml).expect("playlist ambient")
    }

    #[test]
    fn the_player_alone_fills_in_the_defaults() {
        let player = ambient("show: lighting/ambient.light");
        let resolved = resolve(Some(&player), None).unwrap().unwrap();
        assert_eq!(
            resolved,
            ResolvedAmbient {
                show: "lighting/ambient.light".to_string(),
                delay: DEFAULT_AMBIENT_DELAY,
                fade_in: DEFAULT_AMBIENT_FADE_IN,
                fade_out: DEFAULT_AMBIENT_FADE_OUT,
            }
        );
    }

    #[test]
    fn a_playlist_overrides_setting_by_setting() {
        let player = ambient("show: a.light\ndelay: 3s\nfade_in: 4s");
        let playlist = playlist("show: b.light\nfade_in: 500ms");
        let resolved = resolve(Some(&player), Some(&playlist)).unwrap().unwrap();
        assert_eq!(resolved.show, "b.light");
        assert_eq!(resolved.delay, Duration::from_secs(3));
        assert_eq!(resolved.fade_in, Duration::from_millis(500));
        assert_eq!(resolved.fade_out, DEFAULT_AMBIENT_FADE_OUT);
    }

    #[test]
    fn a_playlist_can_have_ambient_without_the_player() {
        let playlist = playlist("show: b.light");
        assert_eq!(
            resolve(None, Some(&playlist)).unwrap().unwrap().show,
            "b.light"
        );
    }

    #[test]
    fn false_turns_it_off_and_true_keeps_the_players() {
        let player = ambient("show: a.light");
        let off = playlist("false");
        assert_eq!(off, PlaylistAmbient::Off);
        assert_eq!(resolve(Some(&player), Some(&off)).unwrap(), None);
        let on = playlist("true");
        assert_eq!(
            resolve(Some(&player), Some(&on)).unwrap().unwrap().show,
            "a.light"
        );
    }

    #[test]
    fn no_show_is_no_ambient() {
        let player = ambient("fade_in: 3s");
        assert_eq!(resolve(Some(&player), None).unwrap(), None);
        assert_eq!(resolve(None, None).unwrap(), None);
    }

    #[test]
    fn a_misspelt_setting_or_a_bad_duration_is_refused() {
        assert!(read::<Ambient>("show: a.light\nfadein: 2s").is_err());
        assert!(read::<PlaylistAmbient>("show: a.light\nfadein: 2s").is_err());
        let bad = ambient("show: a.light\nfade_out: soon");
        assert!(bad.validate().unwrap_err().contains("fade_out"));
        assert!(resolve(Some(&bad), None).is_err());
    }

    #[test]
    fn a_playlist_writes_its_ambient_back_as_it_was_read() {
        for ambient in ["false", "\n    show: b.light\n    delay: 1s"] {
            let text = format!("songs:\n  - One\nambient: {ambient}\n");
            let read = |text: &str| -> super::super::Playlist {
                config::Config::builder()
                    .add_source(config::File::from_str(text, config::FileFormat::Yaml))
                    .build()
                    .unwrap()
                    .try_deserialize()
                    .unwrap()
            };
            let playlist = read(&text);
            assert!(playlist.ambient().is_some(), "{text}");
            let written = crate::util::to_yaml_string(&playlist).unwrap();
            assert!(!written.contains("fade_in"), "{written}");
            assert_eq!(read(&written), playlist, "{written}");
        }
        let plain =
            crate::util::to_yaml_string(&super::super::Playlist::new(&["One".into()])).unwrap();
        assert!(!plain.contains("ambient"), "{plain}");
    }
}
