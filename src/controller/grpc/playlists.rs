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

//! The playlist writes of the gRPC service: the web UI's and MCP's playlist
//! saves and delete, behind the same write lock and checks, answered as
//! statuses.

use std::sync::Arc;

use tonic::Status;

use crate::config;
use crate::player::{Player, SongSources};
use crate::proto::player::v1::SavePlaylistRequest;

use super::playlist_settings::{self, Reported};
use crate::webui::api::lighting_writes::Answer;
use crate::webui::api::playlists::{self, PLAYLIST_WRITES};

/// A refusal of the web UI's playlist writers as a status, by the status
/// the web API gives it and with its message.
pub(super) async fn status_of(response: axum::response::Response) -> Status {
    let answer = Answer::of_response(response).await;
    let message = answer.message();
    match answer.status.as_u16() {
        400 | 422 => Status::invalid_argument(message),
        404 => Status::not_found(message),
        409 => Status::already_exists(message),
        _ => Status::internal(message),
    }
}

/// The directory playlists are written to and the project it is in, from
/// the sources the player reloads from, so a playlist written is one its
/// reload picks up.
async fn write_target(
    player: &Player,
    store: &config::ConfigStore,
) -> Result<(std::path::PathBuf, std::path::PathBuf), Status> {
    let sources = match player.song_sources() {
        Some(sources) => sources,
        None => SongSources::resolve(&store.read_config().await, store.path(), None),
    };
    let dir = sources
        .playlists_dir
        .ok_or_else(|| Status::failed_precondition("No playlists directory configured"))?;
    Ok((dir, crate::util::project_dir_of(store.path())))
}

/// Saves a playlist's songs, as the web UI's save of a playlist sent only
/// `songs` does: an existing file keeps its settings, each entry's with its
/// song. With `set_settings` and `set_ambient` it sets those too, as the
/// web UI's save of its settings does, and is checked as that is. With the
/// lock held from reading the file to reloading the player.
pub(super) async fn save(
    player: &Arc<Player>,
    store: &config::ConfigStore,
    req: SavePlaylistRequest,
) -> Result<(), Status> {
    let _writing = PLAYLIST_WRITES.lock().await;
    let (dir, project) = write_target(player, store).await?;
    let file = match playlists::playlist_file_to_write(&dir, &req.name).await {
        Ok(file) => file,
        Err(response) => return Err(status_of(response).await),
    };
    let exists = file.exists() || player.playlists_snapshot().contains_key(&req.name);
    if req.create && exists {
        return Err(Status::already_exists(format!(
            "Playlist '{}' already exists",
            req.name
        )));
    }
    if !req.create && !file.exists() {
        return Err(Status::not_found(format!(
            "Playlist '{}' not found",
            req.name
        )));
    }

    let playlist = if file.exists() {
        // One that does not read is not overwritten, which would lose its
        // settings. What is wrong with it stays out of the message.
        let existing = config::Playlist::deserialize(file.path()).map_err(|_| {
            Status::failed_precondition(
                "The playlist file does not read, so saving would lose its settings; \
                 fix it by hand first",
            )
        })?;
        if req.check_expected && existing.songs() != req.expected {
            return Err(changed());
        }
        // A caller that read the settings too checks them as well, by the
        // revision GetPlaylist gave it. A file whose settings do not read
        // is not the one it read.
        if req.check_expected
            && !req.expected_revision.is_empty()
            && Reported::of_file(&existing).map(|r| r.revision).as_deref()
                != Ok(req.expected_revision.as_str())
        {
            return Err(changed());
        }
        existing.with_songs(&req.songs)
    } else {
        if req.check_expected && !req.expected.is_empty() {
            return Err(changed());
        }
        config::Playlist::new(&req.songs)
    };
    let playlist = playlist_settings::apply(playlist, &req)?;
    if let Err(response) = playlists::check_playlist(&playlist, player) {
        return Err(status_of(response).await);
    }
    let yaml = match playlists::playlist_yaml(&playlist) {
        Ok(yaml) => yaml,
        Err(response) => return Err(status_of(response).await),
    };
    if let Err(response) = playlists::write_playlist_file(&file, yaml, &project).await {
        return Err(status_of(response).await);
    }
    reload(player).await
}

/// The `.light` files an ambient light can play, as paths relative to the
/// project, sorted: those the web UI's lighting list finds under the songs
/// directory, and those in the project's `lighting/` directory, where the
/// docs put ambient shows. A file outside the project is left out: an
/// ambient show is refused there.
pub(super) async fn light_shows(
    player: &Player,
    store: &config::ConfigStore,
) -> Result<Vec<String>, Status> {
    let songs = match player.song_sources() {
        Some(sources) => sources.songs,
        None => SongSources::resolve(&store.read_config().await, store.path(), None).songs,
    };
    let project = crate::util::project_dir_of(store.path());
    tokio::task::spawn_blocking(move || {
        let project = std::fs::canonicalize(&project).unwrap_or(project);
        let mut paths = std::collections::BTreeSet::new();
        for dir in [songs, project.join("lighting")] {
            let Ok(dir) = std::fs::canonicalize(&dir) else {
                continue;
            };
            let mut found = Vec::new();
            // A directory that cannot be read lists what was found in it.
            let _ = crate::webui::api::lighting_api::find_light_files(&dir, &dir, &mut found);
            for file in found {
                let path = dir.join(&file.path);
                if let Ok(relative) = path.strip_prefix(&project) {
                    paths.insert(relative.to_string_lossy().replace('\\', "/"));
                }
            }
        }
        paths.into_iter().collect()
    })
    .await
    .map_err(|e| Status::internal(format!("listing light shows failed: {e}")))
}

/// Deletes a playlist, as the web UI's delete does: its file goes, and the
/// player's reload leaves the active playlist on all_songs if it was that.
pub(super) async fn delete(
    player: &Arc<Player>,
    store: &config::ConfigStore,
    name: &str,
) -> Result<(), Status> {
    if let Err(response) = playlists::validate_playlist_name(name) {
        return Err(status_of(response).await);
    }
    let _writing = PLAYLIST_WRITES.lock().await;
    let (dir, _) = write_target(player, store).await?;
    if let Err(response) = playlists::remove_playlist_file(&dir, name).await {
        return Err(status_of(response).await);
    }
    reload(player).await
}

/// The refusal of a save made from a list that is no longer the playlist.
/// The app matches the start of the message, as for `NavigateToSong`.
fn changed() -> Status {
    Status::failed_precondition("the playlist has changed since it was read")
}

/// Takes the change up in the player, which says so if it could not.
async fn reload(player: &Player) -> Result<(), Status> {
    player.reload_songs().await.map_err(|e| {
        Status::internal(format!(
            "the change was written, but the player could not reload its songs: {e}"
        ))
    })
}

#[cfg(test)]
mod tests {
    use tonic::{Code, Request};

    use super::super::PlayerServer;
    use crate::proto::player::v1::{
        player_service_server::PlayerService, AmbientSetting, AutoPlay, DeletePlaylistRequest,
        EntrySettings, Fade, GetPlaylistResponse, ListLightShowsRequest, PlaylistEntry,
        SavePlaylistRequest, TrackNames,
    };
    use crate::testutil::player::{playlist_project, PlaylistProject};

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn server(project: &PlaylistProject) -> PlayerServer {
        PlayerServer::with_config_store(project.player.clone(), project.store.clone())
    }

    fn save_request(name: &str, songs: &[&str], create: bool) -> SavePlaylistRequest {
        SavePlaylistRequest {
            name: name.to_string(),
            songs: names(songs),
            create,
            ..Default::default()
        }
    }

    async fn save(
        project: &PlaylistProject,
        req: SavePlaylistRequest,
    ) -> Result<GetPlaylistResponse, tonic::Status> {
        server(project)
            .save_playlist(Request::new(req))
            .await
            .map(|r| r.into_inner())
    }

    fn songs_of(response: &GetPlaylistResponse) -> Vec<&str> {
        response
            .entries
            .iter()
            .map(|e| e.song_name.as_str())
            .collect()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_playlist_is_written_loaded_and_answered() {
        let project = playlist_project().await.unwrap();
        let saved = save(&project, save_request("set", &["Song 3", "Song 1"], true))
            .await
            .unwrap();
        assert_eq!(saved.name, "set");
        assert_eq!(songs_of(&saved), ["Song 3", "Song 1"]);
        assert!(saved.entries.iter().all(|e| e.duration.is_some()));
        assert!(project.file("set").exists());
        // As GetPlaylist reports it now.
        let got = server(&project)
            .get_playlist(Request::new(crate::proto::player::v1::GetPlaylistRequest {
                name: "set".into(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(got, saved);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn create_refuses_an_existing_playlist_and_save_a_missing_one() {
        let project = playlist_project().await.unwrap();
        project.write("set", "songs:\n- Song 1\n").await;
        let before = std::fs::read_to_string(project.file("set")).unwrap();
        let err = save(&project, save_request("set", &["Song 2"], true))
            .await
            .unwrap_err();
        assert_eq!(err.code(), Code::AlreadyExists);
        assert_eq!(
            std::fs::read_to_string(project.file("set")).unwrap(),
            before
        );

        let err = save(&project, save_request("other", &["Song 2"], false))
            .await
            .unwrap_err();
        assert_eq!(err.code(), Code::NotFound);
        assert!(!project.file("other").exists());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_reorder_keeps_each_entrys_settings_and_the_playlists() {
        let project = playlist_project().await.unwrap();
        project
            .write(
                "set",
                "songs:\n- Song 1\n- song: Song 3\n  auto_play: 2s\n- Song 5\n",
            )
            .await;
        let saved = save(
            &project,
            save_request("set", &["Song 3", "Song 5", "Song 1"], false),
        )
        .await
        .unwrap();
        assert_eq!(songs_of(&saved), ["Song 3", "Song 5", "Song 1"]);
        let file = crate::config::Playlist::deserialize(&project.file("set")).unwrap();
        let first = &file.entries()[0];
        assert_eq!(first.song(), "Song 3");
        let transitions = file.entry_transitions().unwrap();
        assert!(transitions[0].auto_play.is_some(), "kept with its song");
        assert!(transitions[1].auto_play.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn refusals_leave_the_files_alone() {
        let project = playlist_project().await.unwrap();
        let err = save(
            &project,
            save_request("set", &["Song 1", "No Such Song"], true),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);
        assert!(err.message().contains("No Such Song"), "{}", err.message());
        assert!(!project.file("set").exists());

        for name in ["all_songs", "", "../x", "a/b"] {
            let err = save(&project, save_request(name, &["Song 1"], true))
                .await
                .unwrap_err();
            assert_eq!(err.code(), Code::InvalidArgument, "{name:?}");
        }

        // A file that does not read is not written over.
        std::fs::write(project.file("broken"), "songs: [\n").unwrap();
        let err = save(&project, save_request("broken", &["Song 1"], false))
            .await
            .unwrap_err();
        assert_eq!(err.code(), Code::FailedPrecondition);
        assert_eq!(
            std::fs::read_to_string(project.file("broken")).unwrap(),
            "songs: [\n"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn expected_is_checked_against_the_file_when_asked() {
        let project = playlist_project().await.unwrap();
        project.write("set", "songs:\n- Song 1\n- Song 3\n").await;
        let guarded = |expected: &[&str]| SavePlaylistRequest {
            expected: names(expected),
            check_expected: true,
            ..save_request("set", &["Song 3", "Song 1"], false)
        };
        let saved = save(&project, guarded(&["Song 1", "Song 3"]))
            .await
            .unwrap();
        assert_eq!(songs_of(&saved), ["Song 3", "Song 1"]);

        // Someone else edited the file after it was read.
        std::fs::write(project.file("set"), "songs:\n- Song 5\n").unwrap();
        let err = save(&project, guarded(&["Song 3", "Song 1"]))
            .await
            .unwrap_err();
        assert_eq!(err.code(), Code::FailedPrecondition);
        assert!(
            err.message().starts_with("the playlist has changed"),
            "{}",
            err.message()
        );
        assert_eq!(
            std::fs::read_to_string(project.file("set")).unwrap(),
            "songs:\n- Song 5\n"
        );

        // An empty list is a legal thing to have seen, and without the
        // flag nothing is checked.
        std::fs::write(project.file("empty"), "songs: []\n").unwrap();
        let empty = SavePlaylistRequest {
            check_expected: true,
            ..save_request("empty", &["Song 1"], false)
        };
        assert!(save(&project, empty).await.is_ok());
        let unchecked = SavePlaylistRequest {
            expected: names(&["Song 9"]),
            ..save_request("empty", &["Song 2"], false)
        };
        assert!(save(&project, unchecked).await.is_ok());
    }

    async fn get(project: &PlaylistProject, name: &str) -> GetPlaylistResponse {
        server(project)
            .get_playlist(Request::new(crate::proto::player::v1::GetPlaylistRequest {
                name: name.into(),
                ..Default::default()
            }))
            .await
            .unwrap()
            .into_inner()
    }

    fn settings_of(response: &GetPlaylistResponse) -> Vec<EntrySettings> {
        response
            .entries
            .iter()
            .map(|e| e.settings.clone().expect("always present"))
            .collect()
    }

    /// A playlist with every setting there is, in the file's own spellings.
    const SET: &str = "songs:
- song: Song 1
  fade_out:
    length: 2beats
    curve: linear
    exclude:
    - track 1
    lights: 0s
  auto_play: 1500ms
- song: Song 3
  fade_in: 250ms
  ambient: false
- song: Song 5
  ambient:
    show: lighting/walk_in.light
    fade_in: 6s
ambient:
  show: lighting/ambient.light
  delay: 1s
";

    #[tokio::test(flavor = "multi_thread")]
    async fn get_playlist_answers_each_entrys_settings_and_the_playlists() {
        let project = playlist_project().await.unwrap();
        project.write("set", SET).await;
        let got = get(&project, "set").await;
        let settings = settings_of(&got);
        let out = settings[0].fade_out.as_ref().unwrap();
        assert_eq!(out.length, "2beats");
        assert_eq!(out.curve.as_deref(), Some("linear"));
        assert_eq!(out.exclude.as_ref().unwrap().names, ["track 1"]);
        assert_eq!(out.lights.as_deref(), Some("0s"));
        assert_eq!(settings[0].auto_play.as_ref().unwrap().gap, "1500ms");
        assert_eq!(settings[1].fade_in.as_ref().unwrap().length, "250ms");
        assert!(!settings[1].ambient.as_ref().unwrap().on);
        let own = settings[2].ambient.as_ref().unwrap();
        assert!(own.on);
        assert_eq!(own.show.as_deref(), Some("lighting/walk_in.light"));
        let ambient = got.ambient.as_ref().unwrap();
        assert_eq!(ambient.show.as_deref(), Some("lighting/ambient.light"));
        assert_eq!(ambient.delay.as_deref(), Some("1s"));
        assert_eq!(got.revision.len(), 16);

        // A playlist with none has empty settings on each entry, so a
        // phone can tell it from a player that sends none at all.
        let all = get(&project, "all_songs").await;
        assert!(settings_of(&all)
            .iter()
            .all(|s| *s == EntrySettings::default()));
        assert_eq!(all.ambient, None);
        assert!(!all.revision.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn settings_are_set_kept_through_a_reorder_and_round_trip() {
        let project = playlist_project().await.unwrap();
        project.write("set", SET).await;
        let read = get(&project, "set").await;
        let mut settings = settings_of(&read);
        // Change Song 3's fade in and auto play, and move it first; Song
        // 1's fade-out details, which a phone does not edit, go back as
        // they came.
        settings[1].fade_in.as_mut().unwrap().length = "4beats".into();
        settings[1].auto_play = Some(AutoPlay { gap: String::new() });
        settings.swap(0, 1);
        let req = SavePlaylistRequest {
            expected: names(&["Song 1", "Song 3", "Song 5"]),
            check_expected: true,
            expected_revision: read.revision.clone(),
            settings: settings.clone(),
            set_settings: true,
            ambient: Some(AmbientSetting::default()),
            set_ambient: true,
            ..save_request("set", &["Song 3", "Song 1", "Song 5"], false)
        };
        let saved = save(&project, req).await.unwrap();
        assert_eq!(songs_of(&saved), ["Song 3", "Song 1", "Song 5"]);
        assert_eq!(settings_of(&saved), settings);
        assert!(!saved.ambient.as_ref().unwrap().on);
        assert_ne!(saved.revision, read.revision);
        let written = std::fs::read_to_string(project.file("set")).unwrap();
        for part in [
            "fade_in: 4beats",
            "auto_play: true",
            "- track 1",
            "lights: 0s",
            "curve: linear",
            "ambient: false\n",
        ] {
            assert!(written.contains(part), "{part}: {written}");
        }
        assert_eq!(get(&project, "set").await, saved);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_without_the_flags_keeps_every_setting() {
        // What an app from before settings sends: names, and the names it
        // expected.
        let project = playlist_project().await.unwrap();
        project.write("set", SET).await;
        let before = get(&project, "set").await;
        let req = SavePlaylistRequest {
            expected: names(&["Song 1", "Song 3", "Song 5"]),
            check_expected: true,
            ..save_request("set", &["Song 5", "Song 3", "Song 1"], false)
        };
        let saved = save(&project, req).await.unwrap();
        let by_song = |r: &GetPlaylistResponse| {
            let mut all: Vec<_> = r
                .entries
                .iter()
                .map(|e| (e.song_name.clone(), e.settings.clone()))
                .collect();
            all.sort_by(|a, b| a.0.cmp(&b.0));
            all
        };
        assert_eq!(by_song(&saved), by_song(&before));
        assert_eq!(saved.ambient, before.ambient);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_settings_change_since_the_read_is_refused() {
        let project = playlist_project().await.unwrap();
        project.write("set", SET).await;
        let read = get(&project, "set").await;
        // Someone else changes a setting, not a song.
        let edited = SET.replace("auto_play: 1500ms", "auto_play: 3s");
        std::fs::write(project.file("set"), &edited).unwrap();
        let req = SavePlaylistRequest {
            expected: names(&["Song 1", "Song 3", "Song 5"]),
            check_expected: true,
            expected_revision: read.revision.clone(),
            ..save_request("set", &["Song 1", "Song 3"], false)
        };
        let err = save(&project, req.clone()).await.unwrap_err();
        assert_eq!(err.code(), Code::FailedPrecondition);
        assert!(err.message().starts_with("the playlist has changed"));
        assert_eq!(
            std::fs::read_to_string(project.file("set")).unwrap(),
            edited
        );

        // The names alone, as an older app checks, still pass.
        let names_only = SavePlaylistRequest {
            expected_revision: String::new(),
            ..req
        };
        assert!(save(&project, names_only).await.is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn bad_settings_are_refused_saying_why() {
        let project = playlist_project().await.unwrap();
        project.write("set", "songs:\n- Song 1\n- Song 3\n").await;
        let before = std::fs::read_to_string(project.file("set")).unwrap();
        let fade = |length: &str, lights: Option<&str>| {
            Some(Fade {
                length: length.into(),
                lights: lights.map(str::to_string),
                ..Default::default()
            })
        };
        let cases = [
            (
                EntrySettings {
                    fade_out: fade("soon", None),
                    ..Default::default()
                },
                "fade_out",
            ),
            (
                EntrySettings {
                    auto_play: Some(AutoPlay {
                        gap: "later".into(),
                    }),
                    ..Default::default()
                },
                "auto_play",
            ),
            (
                EntrySettings {
                    ambient: Some(AmbientSetting {
                        on: true,
                        show: Some("../outside.light".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                "inside the project",
            ),
            (
                EntrySettings {
                    fade_in: fade("1s", Some("1s")),
                    ..Default::default()
                },
                "lights applies only to fade_out",
            ),
        ];
        for (bad, says) in cases {
            let req = SavePlaylistRequest {
                settings: vec![bad, EntrySettings::default()],
                set_settings: true,
                ..save_request("set", &["Song 1", "Song 3"], false)
            };
            let err = save(&project, req).await.unwrap_err();
            assert_eq!(err.code(), Code::InvalidArgument, "{says}");
            assert!(err.message().contains(says), "{says}: {}", err.message());
            assert_eq!(
                std::fs::read_to_string(project.file("set")).unwrap(),
                before
            );
        }
        let ambient = SavePlaylistRequest {
            ambient: Some(AmbientSetting {
                on: true,
                delay: Some("whenever".into()),
                ..Default::default()
            }),
            set_ambient: true,
            ..save_request("set", &["Song 1"], false)
        };
        let err = save(&project, ambient).await.unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);
        assert!(err.message().contains("delay"), "{}", err.message());
    }

    #[test]
    fn fifty_songs_with_every_setting_fit_a_bluetooth_message() {
        use prost::Message;
        let long = |s: &str| format!("{s} {}", "x".repeat(40));
        let settings = EntrySettings {
            fade_out: Some(Fade {
                length: "1measure".into(),
                curve: Some("equal_power".into()),
                exclude: Some(TrackNames {
                    names: (0..4).map(|i| long(&format!("track {i}"))).collect(),
                }),
                lights: Some("500ms".into()),
            }),
            fade_in: Some(Fade {
                length: "2beats".into(),
                curve: Some("linear".into()),
                exclude: Some(TrackNames {
                    names: vec![long("click")],
                }),
                lights: None,
            }),
            auto_play: Some(AutoPlay {
                gap: "1500ms".into(),
            }),
            ambient: Some(AmbientSetting {
                on: true,
                show: Some(long("lighting/ambient/encore") + ".light"),
                delay: Some("1s".into()),
                fade_in: Some("2s".into()),
                fade_out: Some("1s".into()),
            }),
        };
        let songs: Vec<String> = (0..50).map(|i| long(&format!("Song {i}"))).collect();
        let response = GetPlaylistResponse {
            name: long("playlist"),
            entries: songs
                .iter()
                .map(|s| PlaylistEntry {
                    song_name: s.clone(),
                    duration: Some(prost_types::Duration {
                        seconds: 300,
                        nanos: 500_000_000,
                    }),
                    settings: Some(settings.clone()),
                })
                .collect(),
            position: Some(49),
            ambient: settings.ambient.clone(),
            revision: "0123456789abcdef".into(),
            ..Default::default()
        };
        let request = SavePlaylistRequest {
            name: long("playlist"),
            songs: songs.clone(),
            expected: songs,
            check_expected: true,
            settings: vec![settings.clone(); 50],
            set_settings: true,
            ambient: settings.ambient.clone(),
            set_ambient: true,
            expected_revision: "0123456789abcdef".into(),
            ..Default::default()
        };
        // Half the frame at most, leaving the envelope room to spare.
        let cap = crate::controller::ble::codec::MAX_FRAME;
        for (what, size) in [
            ("GetPlaylistResponse", response.encoded_len()),
            ("SavePlaylistRequest", request.encoded_len()),
        ] {
            println!("{what}: {size} bytes for 50 songs with every setting");
            assert!(size < cap / 2, "{what} is {size} bytes");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn light_shows_are_listed_relative_to_the_project() {
        let project = playlist_project().await.unwrap();
        let root = project.dir.path();
        std::fs::create_dir_all(root.join("assets/songs/shows")).unwrap();
        std::fs::write(root.join("assets/songs/shows/opener.light"), "").unwrap();
        std::fs::create_dir_all(root.join("lighting")).unwrap();
        std::fs::write(root.join("lighting/ambient.light"), "").unwrap();
        std::fs::write(root.join("lighting/notes.txt"), "").unwrap();
        let listed = server(&project)
            .list_light_shows(Request::new(ListLightShowsRequest {}))
            .await
            .unwrap()
            .into_inner();
        assert_eq!(
            listed.paths,
            ["assets/songs/shows/opener.light", "lighting/ambient.light"]
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn duplicated_songs_are_kept_as_the_web_save_keeps_them() {
        let project = playlist_project().await.unwrap();
        let saved = save(
            &project,
            save_request("twice", &["Song 1", "Song 3", "Song 1"], true),
        )
        .await
        .unwrap();
        assert_eq!(songs_of(&saved), ["Song 1", "Song 3", "Song 1"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn delete_removes_the_file_and_the_playlist() {
        let project = playlist_project().await.unwrap();
        project.write("set", "songs:\n- Song 1\n").await;
        project.player.switch_to_playlist("set").await.unwrap();
        server(&project)
            .delete_playlist(Request::new(DeletePlaylistRequest { name: "set".into() }))
            .await
            .unwrap();
        assert!(!project.file("set").exists());
        assert!(!project.player.playlists_snapshot().contains_key("set"));
        // The active playlist falls back, as after the web UI's delete.
        assert_eq!(project.player.get_playlist().name(), "all_songs");

        for (name, code) in [
            ("set", Code::NotFound),
            ("all_songs", Code::InvalidArgument),
        ] {
            let err = server(&project)
                .delete_playlist(Request::new(DeletePlaylistRequest { name: name.into() }))
                .await
                .unwrap_err();
            assert_eq!(err.code(), code, "{name}");
        }
    }

    // What the web UI's playlist routes did, which these calls now are.

    fn read(project: &PlaylistProject, name: &str) -> String {
        std::fs::read_to_string(project.file(name)).unwrap()
    }

    /// A playlist kept as `.yml` is saved into that file, keeping its
    /// settings, not into a new `.yaml` beside it, and the reloaded player
    /// plays the songs saved; with both there, the `.yaml` the loader reads
    /// is saved and the `.yml` left alone.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_goes_to_the_file_the_name_has() {
        let project = playlist_project().await.unwrap();
        let dir = project.dir.path().join("playlists");
        let old = "songs:\n  - Song 1\nambient: false\n";
        std::fs::write(dir.join("mine.yml"), old).unwrap();
        project.player.reload_songs().await.unwrap();
        save(&project, save_request("mine", &["Song 2"], false))
            .await
            .unwrap();
        assert!(!dir.join("mine.yaml").exists());
        let written = std::fs::read_to_string(dir.join("mine.yml")).unwrap();
        assert!(written.contains("Song 2"), "{written}");
        assert!(written.contains("ambient: false"), "{written}");
        assert_eq!(
            get(&project, "mine").await.entries[0].song_name,
            "Song 2",
            "the reloaded player has it"
        );

        std::fs::write(dir.join("both.yaml"), "songs:\n  - Song 1\n").unwrap();
        std::fs::write(dir.join("both.yml"), old).unwrap();
        project.player.reload_songs().await.unwrap();
        save(&project, save_request("both", &["Song 3"], false))
            .await
            .unwrap();
        assert!(read(&project, "both").contains("Song 3"));
        assert_eq!(std::fs::read_to_string(dir.join("both.yml")).unwrap(), old);
    }

    /// A new playlist named as a file there but for case (`set` beside
    /// `Set.yaml`, which a case-insensitive filesystem would open for it)
    /// is refused, and `Set.yaml` left alone.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_new_name_differing_only_in_case_is_refused() {
        let project = playlist_project().await.unwrap();
        let old = "songs:\n  - Song 1\n";
        let dir = project.dir.path().join("playlists");
        std::fs::write(dir.join("Set.yaml"), old).unwrap();
        let err = save(&project, save_request("set", &[], true))
            .await
            .unwrap_err();
        assert_eq!(err.code(), Code::AlreadyExists);
        assert!(
            err.message()
                .contains("`Set.yaml` already exists; names differ only in case"),
            "{}",
            err.message()
        );
        assert_eq!(std::fs::read_to_string(dir.join("Set.yaml")).unwrap(), old);
        assert!(!dir.join("set.yaml").exists());
    }

    /// A delete removes the name's file: its `.yml` when that is all there
    /// is, its `.yaml` when both are there.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_delete_removes_the_names_file() {
        let project = playlist_project().await.unwrap();
        let dir = project.dir.path().join("playlists");
        std::fs::write(dir.join("old.yml"), "songs: []\n").unwrap();
        std::fs::write(dir.join("both.yaml"), "songs: []\n").unwrap();
        std::fs::write(dir.join("both.yml"), "songs: []\n").unwrap();
        project.player.reload_songs().await.unwrap();
        for name in ["old", "both"] {
            server(&project)
                .delete_playlist(Request::new(DeletePlaylistRequest { name: name.into() }))
                .await
                .unwrap();
        }
        assert!(!dir.join("old.yml").exists());
        assert!(!dir.join("both.yaml").exists());
        assert!(dir.join("both.yml").exists());
    }

    /// Refused saves leave the file as it was: a song the player does not
    /// have (#578: saved, and then the playlist vanished, being one the
    /// player would not load), settings that do not go one to a song
    /// (#578: laid over the entries by position, the first lost its fade_in
    /// and the rest kept theirs), and an ambient show outside the project
    /// (#578: read from wherever it pointed on a switch).
    #[tokio::test(flavor = "multi_thread")]
    async fn refused_saves_leave_an_existing_file_as_it_was() {
        let project = playlist_project().await.unwrap();
        let existing = "songs:\n- song: Song 1\n  fade_in: 1s\n- Song 3\n- Song 5\n";
        project.write("set", existing).await;
        let unchanged = || assert_eq!(read(&project, "set"), existing);

        let err = save(
            &project,
            save_request("set", &["Song 1", "No Such Song"], false),
        )
        .await
        .unwrap_err();
        assert_eq!(err.code(), Code::InvalidArgument);
        unchanged();

        for settings in [1, 2, 4] {
            let req = SavePlaylistRequest {
                settings: vec![EntrySettings::default(); settings],
                set_settings: true,
                ..save_request("set", &["Song 1", "Song 3", "Song 5"], false)
            };
            let err = save(&project, req).await.unwrap_err();
            assert_eq!(err.code(), Code::InvalidArgument, "{settings}");
            assert!(err.message().contains("one for each"), "{}", err.message());
            unchanged();
        }

        for show in [
            "../../../etc/passwd",
            "/etc/passwd",
            "lighting/../../x.light",
        ] {
            let ambient = AmbientSetting {
                on: true,
                show: Some(show.into()),
                ..Default::default()
            };
            let playlist = SavePlaylistRequest {
                ambient: Some(ambient.clone()),
                set_ambient: true,
                ..save_request("set", &["Song 1"], false)
            };
            let entry = SavePlaylistRequest {
                settings: vec![EntrySettings {
                    ambient: Some(ambient),
                    ..Default::default()
                }],
                set_settings: true,
                ..save_request("set", &["Song 1"], false)
            };
            for req in [playlist, entry] {
                let err = save(&project, req).await.unwrap_err();
                assert_eq!(err.code(), Code::InvalidArgument, "{show}");
                unchanged();
            }
        }
    }

    /// Two editors saving at once take turns: #578, they shared the
    /// write's sidecar, and all but one got an error.
    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_saves_to_one_playlist_all_succeed() {
        let project = playlist_project().await.unwrap();
        project.write("set", "songs:\n- Song 1\n").await;
        let project = std::sync::Arc::new(project);
        let saves: Vec<_> = (0..12)
            .map(|i| {
                let project = project.clone();
                tokio::spawn(async move {
                    let songs: &[&str] = if i % 2 == 0 {
                        &["Song 1"]
                    } else {
                        &["Song 1", "Song 1"]
                    };
                    save(&project, save_request("set", songs, false)).await
                })
            })
            .collect();
        for save in saves {
            save.await.unwrap().unwrap();
        }
    }

    /// What a save leaves out it keeps, and what it sets to none it clears:
    /// the playlist's ambient light (set with `set_ambient`), and each
    /// entry's fades (set with `set_settings`).
    #[tokio::test(flavor = "multi_thread")]
    async fn a_save_keeps_or_clears_ambient_and_fades() {
        let project = playlist_project().await.unwrap();
        let existing = "ambient: false\nsongs:\n- song: Song 1\n  auto_play: 2s\n  fade_out: 200ms\n  fade_in: 100ms\n";
        project.write("set", existing).await;
        // Left out: kept, each entry's with it.
        save(&project, save_request("set", &["Song 1"], false))
            .await
            .unwrap();
        let kept = read(&project, "set");
        for part in [
            "ambient: false",
            "auto_play: 2s",
            "fade_out: 200ms",
            "fade_in: 100ms",
        ] {
            assert!(kept.contains(part), "{part}: {kept}");
        }
        // Ambient set to none removes it, so the profile's plays.
        let req = SavePlaylistRequest {
            set_ambient: true,
            ..save_request("set", &["Song 1"], false)
        };
        save(&project, req).await.unwrap();
        let cleared = read(&project, "set");
        assert!(!cleared.contains("ambient"), "{cleared}");
        assert!(cleared.contains("fade_out: 200ms"), "{cleared}");
        // Entries set to none lose their fades, leaving the song's name.
        let req = SavePlaylistRequest {
            settings: vec![EntrySettings::default()],
            set_settings: true,
            ..save_request("set", &["Song 1"], false)
        };
        save(&project, req).await.unwrap();
        let bare = read(&project, "set");
        assert!(!bare.contains("fade"), "{bare}");
        assert!(!bare.contains("auto_play"), "{bare}");
        assert!(bare.contains("- Song 1"), "{bare}");
    }

    /// A fade the player would refuse is refused before it is written: an
    /// unknown curve, a track the song does not have (on either fade), and
    /// lights on a fade in.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_bad_fade_is_refused_and_nothing_is_written() {
        let project = playlist_project().await.unwrap();
        let existing = "songs:\n- Song 1\n";
        project.write("set", existing).await;
        let fade = |curve: Option<&str>, exclude: Option<&[&str]>, lights: Option<&str>| Fade {
            length: "200ms".into(),
            curve: curve.map(str::to_string),
            exclude: exclude.map(|names| TrackNames {
                names: names.iter().map(|n| n.to_string()).collect(),
            }),
            lights: lights.map(str::to_string),
        };
        let cases = [
            (
                EntrySettings {
                    fade_out: Some(fade(Some("wobbly"), None, None)),
                    ..Default::default()
                },
                "wobbly",
            ),
            (
                EntrySettings {
                    fade_out: Some(fade(None, Some(&["cowbell"]), None)),
                    ..Default::default()
                },
                "cowbell",
            ),
            (
                EntrySettings {
                    fade_in: Some(fade(None, Some(&["cowbell"]), None)),
                    ..Default::default()
                },
                "cowbell",
            ),
            (
                EntrySettings {
                    fade_in: Some(fade(None, None, Some("100ms"))),
                    ..Default::default()
                },
                "lights applies only to fade_out",
            ),
        ];
        for (bad, says) in cases {
            let req = SavePlaylistRequest {
                settings: vec![bad.clone()],
                set_settings: true,
                ..save_request("set", &["Song 1"], false)
            };
            let err = save(&project, req).await.unwrap_err();
            assert_eq!(err.code(), Code::InvalidArgument, "{bad:?}");
            assert!(err.message().contains(says), "{says}: {}", err.message());
            assert_eq!(read(&project, "set"), existing, "{bad:?}");
        }
        // The song's own track is a fine thing to leave out.
        let good = SavePlaylistRequest {
            settings: vec![EntrySettings {
                fade_out: Some(fade(Some("linear"), Some(&["track 1"]), Some("0s"))),
                ..Default::default()
            }],
            set_settings: true,
            ..save_request("set", &["Song 1"], false)
        };
        save(&project, good).await.unwrap();
    }

    /// Names from before #578's rules for new names: they load and are
    /// listed, and the editor must still save and delete them. A new
    /// playlist cannot be given one.
    #[tokio::test(flavor = "multi_thread")]
    async fn playlists_already_on_disk_stay_savable_and_deletable() {
        for (file, name) in [(".warmup.yaml", ".warmup"), ("Encore .yaml", "Encore ")] {
            let project = playlist_project().await.unwrap();
            let path = project.dir.path().join("playlists").join(file);
            let err = save(&project, save_request(name, &["Song 1"], true))
                .await
                .unwrap_err();
            assert_eq!(err.code(), Code::InvalidArgument, "{file}");
            assert!(!path.exists(), "{file}");

            std::fs::write(&path, "songs:\n  - \"Song 1\"\n").unwrap();
            project.player.reload_songs().await.unwrap();
            save(&project, save_request(name, &["Song 1", "Song 1"], false))
                .await
                .unwrap();
            server(&project)
                .delete_playlist(Request::new(DeletePlaylistRequest { name: name.into() }))
                .await
                .unwrap();
            assert!(!path.exists(), "{file}");
        }
    }

    /// The editor's read: each entry's fades as written, the songs the
    /// playlist could have, and each song's tracks for a fade to leave out;
    /// left off unless asked for, as a phone asks.
    #[tokio::test(flavor = "multi_thread")]
    async fn get_playlist_with_the_library_lists_the_songs_and_their_tracks() {
        let project = playlist_project().await.unwrap();
        project
            .write(
                "set",
                "songs:\n- song: Song 1\n  fade_out: {length: 1measure, exclude: [track 1]}\n- Song 1\n",
            )
            .await;
        let project = &project;
        let ask = |with_library: bool| async move {
            server(project)
                .get_playlist(Request::new(crate::proto::player::v1::GetPlaylistRequest {
                    name: "set".into(),
                    with_library,
                }))
                .await
        };
        let plain = ask(false).await.unwrap().into_inner();
        assert!(plain.available_songs.is_empty());
        assert!(plain.song_tracks.is_empty());

        let full = ask(true).await.unwrap().into_inner();
        let out = settings_of(&full)[0].fade_out.clone().unwrap();
        assert_eq!(out.length, "1measure");
        assert_eq!(out.exclude.unwrap().names, ["track 1"]);
        assert_eq!(settings_of(&full)[1], EntrySettings::default());
        // Every song the player has, sorted, each with its tracks.
        let library = project.player.songs();
        let expected: Vec<String> = library
            .sorted_list()
            .iter()
            .map(|s| s.name().to_string())
            .collect();
        assert_eq!(full.available_songs, expected);
        assert!(expected.len() > 1);
        assert_eq!(full.song_tracks.len(), expected.len());
        assert_eq!(full.song_tracks["Song 1"].names, ["track 1"]);
        // What else it says is what the plain read says.
        let strip = |mut r: GetPlaylistResponse| {
            r.available_songs.clear();
            r.song_tracks.clear();
            r
        };
        assert_eq!(strip(full), plain);
    }

    /// The list of playlists, as the web UI's shows them: each with its
    /// song count, the player's own flagged reserved, and the active one
    /// named. Brief unless asked, as a phone asks.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_detailed_list_gives_each_playlists_count_and_reserved() {
        use crate::proto::player::v1::{ListPlaylistsRequest, PlaylistSummary};
        let project = playlist_project().await.unwrap();
        project.write("mine", "songs:\n- Song 1\n- Song 3\n").await;
        std::fs::write(
            project.dir.path().join("playlists/ymllist.yml"),
            "songs: []\n",
        )
        .unwrap();
        project.player.reload_songs().await.unwrap();
        let project = &project;
        let list = |detailed| async move {
            server(project)
                .list_playlists(Request::new(ListPlaylistsRequest { detailed }))
                .await
        };

        let brief = list(false).await.unwrap().into_inner();
        assert!(brief.playlists.is_empty());
        assert_eq!(brief.names, ["all_songs", "mine", "ymllist"]);

        let detailed = list(true).await.unwrap().into_inner();
        assert_eq!(detailed.names, brief.names);
        assert_eq!(detailed.active, "all_songs");
        let all = project.player.songs().sorted_list().len() as u32;
        assert_eq!(
            detailed.playlists,
            [
                PlaylistSummary {
                    name: "all_songs".into(),
                    song_count: all,
                    reserved: true
                },
                PlaylistSummary {
                    name: "mine".into(),
                    song_count: 2,
                    reserved: false
                },
                PlaylistSummary {
                    name: "ymllist".into(),
                    song_count: 0,
                    reserved: false
                },
            ]
        );
        // An unknown playlist is not found.
        let err = server(project)
            .get_playlist(Request::new(crate::proto::player::v1::GetPlaylistRequest {
                name: "nonexistent".into(),
                ..Default::default()
            }))
            .await
            .unwrap_err();
        assert_eq!(err.code(), Code::NotFound);
    }
}
