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
use crate::webui::api::lighting_writes::Answer;
use crate::webui::api::playlists::{self, PLAYLIST_WRITES};

/// A refusal of the web UI's playlist writers as a status, by the status
/// the web API gives it and with its message.
async fn status_of(response: axum::response::Response) -> Status {
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
/// song. With the lock held from reading the file to reloading the player.
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
        existing.with_songs(&req.songs)
    } else {
        if req.check_expected && !req.expected.is_empty() {
            return Err(changed());
        }
        config::Playlist::new(&req.songs)
    };
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
        player_service_server::PlayerService, DeletePlaylistRequest, GetPlaylistResponse,
        SavePlaylistRequest,
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
}
