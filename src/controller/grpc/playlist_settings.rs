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

//! A playlist's settings on the wire: each entry's fades, auto play and
//! ambient light and the playlist's own ambient, as `GetPlaylist` reports
//! them and `SavePlaylist` sets them, and the revision that tells a save
//! the playlist is still the one its caller read.
//!
//! Lengths go as the file writes them; the player checks what is saved with
//! the web UI's own check (`playlist::validate_for_write`), so nothing here
//! parses one.

use std::time::Duration;

use prost::Message;
use sha2::{Digest, Sha256};
use tonic::Status;

use crate::config::{self, ambient::PlaylistAmbient, AutoPlayConfig, EntryFades, FadeConfig};
use crate::proto::player::v1::{
    AmbientSetting, AutoPlay, EntrySettings, Fade, SavePlaylistRequest, TrackNames,
};
use crate::webui::api::playlists::format_gap;

/// A playlist's settings as reported: one per entry, the playlist's own
/// ambient, and the revision of those with the songs.
#[derive(Debug, PartialEq)]
pub(super) struct Reported {
    pub(super) entries: Vec<EntrySettings>,
    pub(super) ambient: Option<AmbientSetting>,
    pub(super) revision: String,
}

impl Reported {
    /// The settings of a playlist the player has loaded.
    pub(super) fn of_loaded(playlist: &crate::playlist::Playlist) -> Reported {
        Reported::new(
            playlist.songs(),
            playlist.entry_fades(),
            &playlist.entry_auto_play(),
            playlist.entry_ambient(),
            playlist.ambient(),
        )
    }

    /// The settings of a playlist file, read as the player would load it,
    /// so a file unchanged since it was loaded has the revision the loaded
    /// playlist reports. Its auto play is refused as the load refuses it.
    pub(super) fn of_file(playlist: &config::Playlist) -> Result<Reported, String> {
        let auto_play: Vec<Option<Duration>> = playlist
            .entry_transitions()?
            .into_iter()
            .map(|t| t.auto_play)
            .collect();
        Ok(Reported::new(
            &playlist.songs(),
            &playlist.entry_fades(),
            &auto_play,
            &playlist.entry_ambient(),
            playlist.ambient(),
        ))
    }

    fn new(
        songs: &[String],
        fades: &[EntryFades],
        auto_play: &[Option<Duration>],
        entry_ambient: &[Option<PlaylistAmbient>],
        ambient: Option<&PlaylistAmbient>,
    ) -> Reported {
        let entries: Vec<EntrySettings> = (0..songs.len())
            .map(|i| EntrySettings {
                fade_out: fades
                    .get(i)
                    .and_then(|f| f.fade_out.as_ref())
                    .map(fade_message),
                fade_in: fades
                    .get(i)
                    .and_then(|f| f.fade_in.as_ref())
                    .map(fade_message),
                auto_play: auto_play.get(i).copied().flatten().map(|gap| AutoPlay {
                    gap: if gap.is_zero() {
                        String::new()
                    } else {
                        format_gap(gap)
                    },
                }),
                ambient: entry_ambient
                    .get(i)
                    .and_then(Option::as_ref)
                    .map(ambient_message),
            })
            .collect();
        let ambient = ambient.map(ambient_message);
        let revision = revision(songs, &entries, ambient.as_ref());
        Reported {
            entries,
            ambient,
            revision,
        }
    }
}

/// A fingerprint of the songs and their settings: the first 64 bits of a
/// SHA-256 over their encoding, in hex. Changing a song, its place or any
/// setting changes it.
fn revision(
    songs: &[String],
    entries: &[EntrySettings],
    ambient: Option<&AmbientSetting>,
) -> String {
    let mut hash = Sha256::new();
    for (song, settings) in songs.iter().zip(entries) {
        hash.update((song.len() as u64).to_le_bytes());
        hash.update(song.as_bytes());
        hash.update(settings.encode_length_delimited_to_vec());
    }
    match ambient {
        None => hash.update([0]),
        Some(ambient) => {
            hash.update([1]);
            hash.update(ambient.encode_length_delimited_to_vec());
        }
    }
    hash.finalize()[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn fade_message(fade: &FadeConfig) -> Fade {
    match fade {
        FadeConfig::Length(length) => Fade {
            length: length.clone(),
            ..Default::default()
        },
        FadeConfig::Full {
            length,
            curve,
            exclude,
            lights,
        } => Fade {
            length: length.clone(),
            curve: curve.clone(),
            exclude: exclude.clone().map(|names| TrackNames { names }),
            lights: lights.clone(),
        },
    }
}

/// The fade as the file writes it: its length alone when it has no other
/// settings.
fn fade_config(fade: &Fade) -> FadeConfig {
    if fade.curve.is_none() && fade.exclude.is_none() && fade.lights.is_none() {
        return FadeConfig::Length(fade.length.clone());
    }
    FadeConfig::Full {
        length: fade.length.clone(),
        curve: fade.curve.clone(),
        exclude: fade.exclude.as_ref().map(|e| e.names.clone()),
        lights: fade.lights.clone(),
    }
}

fn ambient_message(ambient: &PlaylistAmbient) -> AmbientSetting {
    match ambient {
        PlaylistAmbient::Off => AmbientSetting::default(),
        PlaylistAmbient::On(ambient) => AmbientSetting {
            on: true,
            show: ambient.show().map(str::to_string),
            delay: ambient.delay().map(str::to_string),
            fade_in: ambient.fade_in().map(str::to_string),
            fade_out: ambient.fade_out().map(str::to_string),
        },
    }
}

fn ambient_config(ambient: &AmbientSetting) -> PlaylistAmbient {
    if !ambient.on {
        return PlaylistAmbient::Off;
    }
    PlaylistAmbient::On(config::ambient::Ambient::new(
        ambient.show.clone(),
        ambient.delay.clone(),
        ambient.fade_in.clone(),
        ambient.fade_out.clone(),
    ))
}

fn auto_play_config(auto_play: &AutoPlay) -> AutoPlayConfig {
    match auto_play.gap.trim() {
        "" => AutoPlayConfig::On(true),
        _ => AutoPlayConfig::Gap(auto_play.gap.clone()),
    }
}

/// `playlist` with the settings `req` sets: each entry's with
/// `set_settings`, the playlist's ambient with `set_ambient`. What it does
/// not set stays as it was. Not checked: the save checks the result as the
/// web UI's does.
#[allow(clippy::result_large_err)]
pub(super) fn apply(
    playlist: config::Playlist,
    req: &SavePlaylistRequest,
) -> Result<config::Playlist, Status> {
    let mut playlist = playlist;
    if req.set_settings {
        // One for each song, as the web UI's save holds its lists to: a
        // list of another length would set some entries and leave others.
        if req.settings.len() != req.songs.len() {
            return Err(Status::invalid_argument(format!(
                "settings has {} entries, but there are {} songs: one for each",
                req.settings.len(),
                req.songs.len()
            )));
        }
        let fades: Vec<EntryFades> = req
            .settings
            .iter()
            .map(|s| EntryFades {
                fade_out: s.fade_out.as_ref().map(fade_config),
                fade_in: s.fade_in.as_ref().map(fade_config),
            })
            .collect();
        let auto_play: Vec<Option<AutoPlayConfig>> = req
            .settings
            .iter()
            .map(|s| s.auto_play.as_ref().map(auto_play_config))
            .collect();
        let ambient: Vec<Option<PlaylistAmbient>> = req
            .settings
            .iter()
            .map(|s| s.ambient.as_ref().map(ambient_config))
            .collect();
        playlist = playlist
            .with_fades(&fades)
            .with_auto_play(&auto_play)
            .with_entry_ambient(&ambient);
    }
    if req.set_ambient {
        playlist = playlist.with_ambient(req.ambient.as_ref().map(ambient_config));
    }
    Ok(playlist)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> config::Playlist {
        ::config::Config::builder()
            .add_source(::config::File::from_str(yaml, ::config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap()
    }

    const FULL: &str = "songs:
- song: A
  fade_out:
    length: 4s
    curve: linear
    exclude: []
    lights: 0s
  fade_in: 2beats
  auto_play: true
  ambient: false
- song: B
  auto_play: 1500ms
  ambient:
    show: lighting/b.light
    fade_in: 6s
- C
ambient:
  show: lighting/ambient.light
  delay: 1s
";

    #[test]
    fn a_file_reads_as_written() {
        let reported = Reported::of_file(&parse(FULL)).unwrap();
        let a = &reported.entries[0];
        let out = a.fade_out.as_ref().unwrap();
        assert_eq!(out.length, "4s");
        assert_eq!(out.curve.as_deref(), Some("linear"));
        // Present and empty is not absent: no track is left out.
        assert_eq!(out.exclude, Some(TrackNames { names: vec![] }));
        assert_eq!(out.lights.as_deref(), Some("0s"));
        assert_eq!(
            a.fade_in,
            Some(Fade {
                length: "2beats".into(),
                ..Default::default()
            })
        );
        assert_eq!(a.auto_play, Some(AutoPlay { gap: String::new() }));
        assert_eq!(a.ambient, Some(AmbientSetting::default()));
        let b = &reported.entries[1];
        assert_eq!(b.auto_play.as_ref().unwrap().gap, "1500ms");
        let own = b.ambient.as_ref().unwrap();
        assert!(own.on);
        assert_eq!(own.show.as_deref(), Some("lighting/b.light"));
        assert_eq!(own.fade_in.as_deref(), Some("6s"));
        assert_eq!(own.delay, None);
        assert_eq!(reported.entries[2], EntrySettings::default());
        let ambient = reported.ambient.as_ref().unwrap();
        assert_eq!(ambient.delay.as_deref(), Some("1s"));
        assert_eq!(reported.revision.len(), 16);
    }

    #[test]
    fn settings_read_and_set_back_leave_the_file_as_it_was() {
        let file = parse(FULL);
        let reported = Reported::of_file(&file).unwrap();
        let req = SavePlaylistRequest {
            songs: file.songs(),
            settings: reported.entries.clone(),
            set_settings: true,
            ambient: reported.ambient.clone(),
            set_ambient: true,
            ..Default::default()
        };
        let again = apply(file.clone(), &req).unwrap();
        assert_eq!(again, file);
        assert_eq!(Reported::of_file(&again).unwrap(), reported);
    }

    #[test]
    fn the_revision_moves_with_any_change() {
        let base = Reported::of_file(&parse(FULL)).unwrap().revision;
        for changed in [
            FULL.replace("curve: linear", "curve: equal_power"),
            FULL.replace("exclude: []", "exclude: [click]"),
            FULL.replace("1500ms", "2s"),
            FULL.replace("delay: 1s", "delay: 2s"),
            FULL.replace("- C\n", "- B\n"),
            FULL.replace("  ambient: false\n", ""),
        ] {
            let other = Reported::of_file(&parse(&changed)).unwrap().revision;
            assert_ne!(other, base, "{changed}");
        }
        // The same playlist, written another way, is the same.
        let same = FULL.replace("auto_play: 1500ms", "auto_play: 1.5s");
        assert_eq!(Reported::of_file(&parse(&same)).unwrap().revision, base);
    }

    #[test]
    fn settings_set_and_cleared() {
        let file = parse("songs:\n- song: A\n  fade_in: 1s\n- B\n");
        let req = SavePlaylistRequest {
            songs: file.songs(),
            settings: vec![
                EntrySettings::default(),
                EntrySettings {
                    fade_out: Some(Fade {
                        length: "1measure".into(),
                        ..Default::default()
                    }),
                    auto_play: Some(AutoPlay { gap: "2s".into() }),
                    ambient: Some(AmbientSetting {
                        on: true,
                        show: Some("x.light".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ],
            set_settings: true,
            set_ambient: true,
            ambient: Some(AmbientSetting::default()),
            ..Default::default()
        };
        let out = apply(file, &req).unwrap();
        let yaml = crate::util::to_yaml_string(&out).unwrap();
        // A is its name alone again; B has what was set and nothing else.
        let expected = parse(
            "songs:\n- A\n- song: B\n  fade_out: 1measure\n  auto_play: 2s\n  ambient:\n    show: x.light\nambient: false\n",
        );
        assert_eq!(out, expected, "{yaml}");
        assert!(yaml.contains("  - A\n"), "{yaml}");
        // Without the flags nothing changes.
        let untouched = SavePlaylistRequest {
            songs: out.songs(),
            settings: vec![],
            ..Default::default()
        };
        assert_eq!(apply(out.clone(), &untouched).unwrap(), out);
        // And with set_ambient, absent is the profile's.
        let inherit = SavePlaylistRequest {
            set_ambient: true,
            ..Default::default()
        };
        assert_eq!(apply(out, &inherit).unwrap().ambient(), None);
    }

    #[test]
    fn settings_of_another_length_are_refused() {
        let file = parse("songs:\n- A\n- B\n");
        let req = SavePlaylistRequest {
            songs: file.songs(),
            settings: vec![EntrySettings::default()],
            set_settings: true,
            ..Default::default()
        };
        let err = apply(file, &req).unwrap_err();
        assert_eq!(err.code(), tonic::Code::InvalidArgument);
        assert!(err.message().contains("one for each"), "{}", err.message());
    }
}
