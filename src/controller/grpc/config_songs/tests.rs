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

//! `ConfigService`'s song calls, called as the service: the behaviour of the
//! REST routes they replaced, each held by a test here.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use prost::Message;
use tonic::{Code, Request, Status};

use super::TAKEN_NAME_KEY;
use crate::config;
use crate::controller::grpc::config_service::ConfigServer;
use crate::proto::player::v1::{self as proto, config_service_server::ConfigService};
use crate::webui::api::test_helpers::{create_test_wav, test_state};
use crate::webui::server::WebUiState;

/// A project on disk (`mtrack.yaml`, an empty `songs/` and the player's
/// "Song A", held in memory), and the service over its player, which has a
/// config store.
struct Project {
    state: WebUiState,
    dir: tempfile::TempDir,
    server: ConfigServer,
}

fn project() -> Project {
    let (state, dir) = test_state();
    let store = Arc::new(config::ConfigStore::new(
        config::Player::deserialize(&state.config_path).unwrap(),
        state.config_path.clone(),
    ));
    state.player.set_config_store(store);
    state.player.set_locked(false);
    Project {
        server: ConfigServer::new(state.player.clone()),
        state,
        dir,
    }
}

const ONE_TRACK: &str = "tracks:\n  - name: track1\n    file: track1.wav\n";

impl Project {
    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn songs(&self) -> PathBuf {
        self.state.songs_path()
    }

    /// Makes the song `name`, a folder with one WAV and a `song.yaml`
    /// declaring `name`, and loads it.
    async fn make_song(&self, name: &str) -> PathBuf {
        let dir = self.songs().join(name);
        std::fs::create_dir_all(&dir).unwrap();
        crate::testutil::write_wav(dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100).unwrap();
        std::fs::write(dir.join("song.yaml"), format!("name: {name}\n{ONE_TRACK}")).unwrap();
        self.state.player.reload_songs().await.unwrap();
        dir
    }

    /// A project folder `path` below the project root holding a WAV.
    fn folder_with_wav(&self, path: &str) -> PathBuf {
        let dir = self.root().join(path);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("track.wav"), create_test_wav()).unwrap();
        dir
    }

    async fn get(&self, name: &str) -> Result<proto::GetSongConfigResponse, Status> {
        self.server
            .get_song_config(Request::new(proto::GetSongConfigRequest {
                name: name.to_string(),
            }))
            .await
            .map(|r| r.into_inner())
    }

    async fn yaml(&self, name: &str) -> String {
        self.get(name).await.unwrap().yaml
    }

    async fn create(&self, name: &str, yaml: &str) -> Result<proto::CreateSongResponse, Status> {
        self.server
            .create_song(Request::new(proto::CreateSongRequest {
                name: name.to_string(),
                yaml: yaml.to_string(),
            }))
            .await
            .map(|r| r.into_inner())
    }

    async fn save(&self, name: &str, yaml: &str) -> Result<(), Status> {
        self.server
            .save_song_config(Request::new(proto::SaveSongConfigRequest {
                name: name.to_string(),
                yaml: yaml.to_string(),
            }))
            .await
            .map(|_| ())
    }

    async fn delete(&self, name: &str) -> Result<proto::DeleteSongResponse, Status> {
        self.server
            .delete_song(Request::new(proto::DeleteSongRequest {
                name: name.to_string(),
            }))
            .await
            .map(|r| r.into_inner())
    }

    async fn waveform(&self, name: &str) -> Result<proto::Waveform, Status> {
        self.server
            .get_song_waveform(Request::new(proto::GetSongWaveformRequest {
                name: name.to_string(),
            }))
            .await
            .map(|r| r.into_inner())
    }

    async fn guess(&self, name: &str) -> Result<proto::GuessSongTempoResponse, Status> {
        self.server
            .guess_song_tempo(Request::new(proto::GuessSongTempoRequest {
                name: name.to_string(),
            }))
            .await
            .map(|r| r.into_inner())
    }

    /// The names of a song's files, in the order listed.
    async fn files(&self, name: &str) -> Result<Vec<(String, proto::FileKind)>, Status> {
        let listed = self
            .server
            .list_song_files(Request::new(proto::ListSongFilesRequest {
                name: name.to_string(),
            }))
            .await?
            .into_inner();
        Ok(listed
            .files
            .into_iter()
            .map(|f| (f.name.clone(), f.kind()))
            .collect())
    }

    async fn import(
        &self,
        song: &str,
        path: &Path,
        filename: Option<&str>,
    ) -> Result<proto::ImportSongFileResponse, Status> {
        self.server
            .import_song_file(Request::new(proto::ImportSongFileRequest {
                name: song.to_string(),
                path: path.to_string_lossy().into_owned(),
                filename: filename.map(str::to_string),
            }))
            .await
            .map(|r| r.into_inner())
    }

    async fn browse(&self, path: &str) -> Result<proto::BrowseDirectoryResponse, Status> {
        self.server
            .browse_directory(Request::new(proto::BrowseDirectoryRequest {
                path: path.to_string(),
            }))
            .await
            .map(|r| r.into_inner())
    }

    async fn create_in(
        &self,
        path: &str,
        name: Option<&str>,
    ) -> Result<proto::CreateSongInDirectoryResponse, Status> {
        self.server
            .create_song_in_directory(Request::new(proto::CreateSongInDirectoryRequest {
                path: path.to_string(),
                name: name.map(str::to_string),
            }))
            .await
            .map(|r| r.into_inner())
    }

    async fn bulk(&self, path: &str) -> Result<proto::BulkImportSongsResponse, Status> {
        self.server
            .bulk_import_songs(Request::new(proto::BulkImportSongsRequest {
                path: path.to_string(),
            }))
            .await
            .map(|r| r.into_inner())
    }
}

/// The error of a call that must have failed, with its code.
fn refused<T: std::fmt::Debug>(result: Result<T, Status>, code: Code) -> Status {
    let status = result.expect_err("the call should be refused");
    assert_eq!(status.code(), code, "{}", status.message());
    status
}

fn midi_asset() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/song.mid")
}

// ---- GetSongConfig ----

#[tokio::test]
async fn get_config_of_an_unknown_song_is_not_found() {
    let p = project();
    for name in ["nonexistent", "FakeSong", "anything"] {
        let status = refused(p.get(name).await, Code::NotFound);
        assert!(
            status
                .message()
                .contains(&format!("unable to find song {name}")),
            "{}",
            status.message()
        );
    }
}

#[tokio::test]
async fn get_config_reads_the_file_the_song_was_loaded_from() {
    let p = project();
    // Each kind of file a song's config is in: song.yaml, song.yml, a file of
    // any name, and a flat layout's `<name>.yaml` beside the others.
    for (song, file) in [
        ("MySong", "song.yaml"),
        ("YmlSong", "song.yml"),
        ("CustomSong", "config.yaml"),
    ] {
        let dir = p.songs().join(song);
        std::fs::create_dir(&dir).unwrap();
        crate::testutil::write_wav(dir.join("track1.wav"), vec![vec![0_i32; 4410]], 44100).unwrap();
        std::fs::write(dir.join(file), format!("name: {song}\n{ONE_TRACK}")).unwrap();
    }
    crate::testutil::write_wav(p.songs().join("track1.wav"), vec![vec![0_i32; 4410]], 44100)
        .unwrap();
    std::fs::write(
        p.songs().join("FlatSong.yaml"),
        format!("name: FlatSong\n{ONE_TRACK}"),
    )
    .unwrap();
    p.state.player.reload_songs().await.unwrap();

    for song in ["MySong", "YmlSong", "CustomSong", "FlatSong"] {
        let got = p.get(song).await.unwrap();
        assert!(
            got.yaml.starts_with(&format!("name: {song}")),
            "{song}: {}",
            got.yaml
        );
        assert!(got.summary.is_none(), "{song}");
    }
}

#[tokio::test]
async fn get_config_of_a_song_whose_file_is_gone_answers_a_summary() {
    let p = project();
    let dir = p.make_song("NoConfig").await;
    std::fs::rename(dir.join("song.yaml"), dir.join("song.bak")).unwrap();

    let got = p.get("NoConfig").await.unwrap();
    assert_eq!(got.yaml, "");
    let summary = got.summary.expect("a summary");
    assert_eq!(summary.name, "NoConfig");
    assert_eq!(summary.tracks, ["track1"]);
    assert_eq!(summary.num_channels, 1);
}

#[tokio::test]
async fn get_config_of_a_song_that_failed_to_load_reads_its_file() {
    let p = project();
    let dir = p.songs().join("Broken");
    std::fs::create_dir(&dir).unwrap();
    // Names a track file that is not there: the song does not load.
    std::fs::write(dir.join("song.yaml"), format!("name: Broken\n{ONE_TRACK}")).unwrap();
    p.state.player.reload_songs().await.unwrap();

    assert!(p.yaml("Broken").await.starts_with("name: Broken"));
}

// ---- CreateSong ----

#[tokio::test]
async fn create_makes_the_song_and_loads_it() {
    let p = project();
    let made = p
        .create("BrandNew", "name: Brand New Song\ntracks: []\n")
        .await
        .unwrap();
    assert_eq!(made.song, "BrandNew");
    assert!(p.songs().join("BrandNew").is_dir());
    assert!(p.songs().join("BrandNew/song.yaml").exists());
    // Loaded: the player has it under the name it declares.
    assert!(p.state.player.songs().get("Brand New Song").is_ok());
}

#[tokio::test]
async fn create_onto_a_song_is_refused() {
    let p = project();
    let dir = p.songs().join("Existing");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("song.yaml"), "name: Existing\ntracks: []\n").unwrap();
    let status = refused(
        p.create("Existing", "name: Existing\ntracks: []\n").await,
        Code::AlreadyExists,
    );
    assert!(
        status.message().contains("Existing"),
        "{}",
        status.message()
    );
}

/// A folder whose config is `song.yml` already has one: a create is refused,
/// and no `song.yaml` is written beside it.
#[tokio::test]
async fn create_beside_a_song_yml_is_refused() {
    let p = project();
    let dir = p.songs().join("Existing");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("song.yml"), "name: Existing\ntracks: []\n").unwrap();
    refused(
        p.create("Existing", "name: Existing\ntracks: []\n").await,
        Code::AlreadyExists,
    );
    assert!(!dir.join("song.yaml").exists());
}

/// Beside a `song.yaml` of another kind (`kind: playlist`), a create is
/// refused and leaves the file as it is.
#[tokio::test]
async fn create_keeps_a_song_yaml_of_another_kind() {
    let p = project();
    let dir = p.songs().join("Pl");
    std::fs::create_dir(&dir).unwrap();
    let playlist = "kind: playlist\nsongs: []\n";
    std::fs::write(dir.join("song.yaml"), playlist).unwrap();
    refused(
        p.create("Pl", "name: Pl\ntracks: []\n").await,
        Code::AlreadyExists,
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("song.yaml")).unwrap(),
        playlist
    );
}

#[tokio::test]
async fn create_validates_the_config_before_making_anything() {
    let p = project();
    let status = refused(
        p.create("BadSong", "not valid: [[[").await,
        Code::InvalidArgument,
    );
    assert!(!status.message().is_empty());
    assert!(!p.songs().join("BadSong").exists());
    // Every problem the validation finds is in the message, in order.
    let status = refused(
        p.create("Blank", "name: ' '\ntracks: []\n").await,
        Code::InvalidArgument,
    );
    assert!(!status.message().is_empty());
    assert!(!p.songs().join("Blank").exists());
}

#[tokio::test]
async fn create_refuses_a_name_another_song_holds() {
    let p = project();
    let status = refused(
        p.create("Dup", "name: Song A\ntracks: []\n").await,
        Code::AlreadyExists,
    );
    assert_eq!(
        status.message(),
        "a song named \"Song A\" already exists; choose another name"
    );
    let taken = status.metadata().get_bin(TAKEN_NAME_KEY).expect("the name");
    assert_eq!(taken.to_bytes().unwrap().as_ref(), b"Song A");
    assert!(!p.songs().join("Dup").exists());
}

/// #578: the song directory was made by any safe name, the new-name rules
/// unchecked, and the song loader then skipped a hidden one.
#[tokio::test]
async fn create_refuses_a_song_dir_name_new_songs_may_not_have() {
    for (name, first) in [
        (".hidden", ".hidden"),
        (" lead", " lead"),
        ("Artist/.hidden", "Artist"),
    ] {
        let p = project();
        refused(
            p.create(name, "name: X\ntracks: []\n").await,
            Code::InvalidArgument,
        );
        // Nothing made, not even the "Artist" above the bad name.
        assert!(!p.songs().join(first).exists(), "{name}: made {first:?}");
    }
}

#[tokio::test]
async fn create_nests_a_song_below_folders_and_refuses_a_path_out_of_the_songs_root() {
    let p = project();
    p.create("Artist/Album/Track", "name: Deep\ntracks: []\n")
        .await
        .unwrap();
    assert!(p.songs().join("Artist/Album/Track/song.yaml").exists());

    for name in ["../escape", "a/../../escape", "/abs", "a\\b", ""] {
        let status = p.create(name, "name: Out\ntracks: []\n").await.unwrap_err();
        assert!(
            matches!(
                status.code(),
                Code::InvalidArgument | Code::PermissionDenied | Code::NotFound
            ),
            "{name:?}: {status:?}"
        );
        assert!(!p.root().join("escape").exists(), "{name:?}");
        assert!(!p.root().join("song.yaml").exists(), "{name:?}");
    }
}

#[tokio::test]
async fn create_then_save_updates_the_config() {
    let p = project();
    p.create("MySong", "name: MySong\ntracks: []\n")
        .await
        .unwrap();
    p.save("MySong", "name: MySong Renamed\ntracks: []\n")
        .await
        .unwrap();
    let content = std::fs::read_to_string(p.songs().join("MySong/song.yaml")).unwrap();
    assert!(content.contains("MySong Renamed"));
}

// ---- SaveSongConfig ----

#[tokio::test]
async fn save_of_an_unknown_song_is_not_found() {
    let p = project();
    for name in ["nonexistent", "whatever"] {
        let status = refused(p.save(name, "name: test\n").await, Code::NotFound);
        assert!(
            status
                .message()
                .contains(&format!("unable to find song {name}")),
            "{}",
            status.message()
        );
    }
}

#[tokio::test]
async fn save_with_no_songs_directory_is_an_internal_error() {
    let p = project();
    let mut sources = p.state.player.song_sources().unwrap_or_default();
    sources.songs = PathBuf::from("/nonexistent/songs");
    p.state.player.set_song_sources(sources);
    for name in ["anything", "whatever"] {
        refused(p.save(name, "name: test\n").await, Code::Internal);
    }
}

#[tokio::test]
async fn save_writes_the_file_and_reloads() {
    let p = project();
    let dir = p.make_song("EditSong").await;
    let edited = format!("name: EditSong\nloop_playback: true\n{ONE_TRACK}");
    p.save("EditSong", &edited).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(dir.join("song.yaml")).unwrap(),
        edited
    );
    assert!(p
        .state
        .player
        .songs()
        .get("EditSong")
        .unwrap()
        .loop_playback());
}

#[tokio::test]
async fn save_with_an_invalid_config_is_refused_and_leaves_the_file() {
    let p = project();
    let dir = p.make_song("BadSong").await;
    let before = std::fs::read_to_string(dir.join("song.yaml")).unwrap();
    refused(
        p.save("BadSong", "invalid yaml: [[[").await,
        Code::InvalidArgument,
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("song.yaml")).unwrap(),
        before
    );
}

/// A name that resolves to the songs root itself (`Foo/..`, `.`) is no song:
/// the save is refused and no `song.yaml` is written in the root.
#[tokio::test]
async fn save_refuses_a_name_resolving_to_the_songs_root() {
    let p = project();
    std::fs::create_dir(p.songs().join("Foo")).unwrap();
    let root_yaml = p.songs().join("song.yaml");
    for name in ["Foo/..", ".", "Foo/../Foo/.."] {
        assert!(
            p.save(name, "name: Root\ntracks: []\n").await.is_err(),
            "{name}"
        );
        assert!(
            !root_yaml.exists(),
            "{name} wrote the songs root's song.yaml"
        );
    }
}

/// A name with `..` out of the songs root is refused, and nothing outside it
/// is written, for every call that takes a song's name.
#[tokio::test]
async fn song_names_cannot_leave_the_songs_root() {
    let p = project();
    // A folder beside the songs root, with a config an escape would reach.
    let outside = p.root().join("outside");
    std::fs::create_dir(&outside).unwrap();
    let theirs = "name: Outside\ntracks: []\n";
    std::fs::write(outside.join("song.yaml"), theirs).unwrap();
    for name in ["../outside", "Foo/../../outside"] {
        assert!(p.get(name).await.is_err(), "get {name}");
        assert!(
            p.save(name, "name: Mine\ntracks: []\n").await.is_err(),
            "save {name}"
        );
        assert!(p.delete(name).await.is_err(), "delete {name}");
        assert!(p.files(name).await.is_err(), "files {name}");
        assert!(p.waveform(name).await.is_err(), "waveform {name}");
        assert!(p.guess(name).await.is_err(), "guess {name}");
        let wav = p.root().join("x.wav");
        std::fs::write(&wav, create_test_wav()).unwrap();
        assert!(p.import(name, &wav, None).await.is_err(), "import {name}");
        assert_eq!(
            std::fs::read_to_string(outside.join("song.yaml")).unwrap(),
            theirs,
            "{name}"
        );
        assert!(!outside.join("x.wav").exists(), "{name}");
    }
}

/// #689: a save that renames a song onto a name another song holds is
/// refused (naming it), the file left as it is; one that keeps the name, or
/// renames onto a free one, is saved.
#[tokio::test]
async fn a_rename_onto_another_songs_name_is_refused() {
    let p = project();
    for name in ["Foo", "Bar"] {
        let dir = p.songs().join(name);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("song.yaml"), format!("name: {name}\ntracks: []\n")).unwrap();
    }
    let foo = p.songs().join("Foo/song.yaml");

    let status = refused(
        p.save("Foo", "name: Bar\ntracks: []\n").await,
        Code::AlreadyExists,
    );
    assert!(
        status.message().contains("already exists"),
        "{}",
        status.message()
    );
    let taken = status.metadata().get_bin(TAKEN_NAME_KEY).expect("the name");
    assert_eq!(taken.to_bytes().unwrap().as_ref(), b"Bar");
    assert_eq!(
        std::fs::read_to_string(&foo).unwrap(),
        "name: Foo\ntracks: []\n"
    );

    for yaml in [
        "name: Foo\ntracks: []\nloop_playback: true\n",
        "name: Baz\ntracks: []\n",
    ] {
        p.save("Foo", yaml).await.unwrap();
        assert_eq!(std::fs::read_to_string(&foo).unwrap(), yaml);
    }
}

/// #680: a folder named as the song whose `song.yaml` declares another song
/// is that song's: saving or deleting the folder's name is refused, naming
/// the song, and the file is left as it is.
#[tokio::test]
async fn saving_or_deleting_a_folder_whose_config_is_another_songs_is_refused() {
    let p = project();
    let foo = p.songs().join("Foo");
    std::fs::create_dir(&foo).unwrap();
    let bar = "name: Bar\ntracks: []\n";
    std::fs::write(foo.join("song.yaml"), bar).unwrap();

    let saved = refused(
        p.save("Foo", "name: Foo\ntracks: []\n").await,
        Code::FailedPrecondition,
    );
    let deleted = refused(p.delete("Foo").await, Code::FailedPrecondition);
    for status in [saved, deleted] {
        assert!(
            status
                .message()
                .contains("Foo/song.yaml is the config of the song \"Bar\""),
            "{}",
            status.message()
        );
        assert_eq!(std::fs::read_to_string(foo.join("song.yaml")).unwrap(), bar);
    }
    // A save of Bar's own config under the folder's name is Bar's edit: the
    // editor's page after it renamed the song still goes by the old name.
    let renamed = "name: Bar\ntracks: []\nloop_playback: true\n";
    p.save("Foo", renamed).await.unwrap();
    assert_eq!(
        std::fs::read_to_string(foo.join("song.yaml")).unwrap(),
        renamed
    );
}

/// A folder `Foo` whose `song.yml` declares another song, `Bar`, is not
/// `Foo`'s: a delete of `Foo` finds nothing, and a save of `Foo` writes a new
/// `song.yaml`; Bar's config is left as it is.
#[tokio::test]
async fn another_songs_song_yml_is_left_alone_by_a_save_and_delete() {
    let p = project();
    let dir = p.songs().join("Foo");
    std::fs::create_dir(&dir).unwrap();
    let bar = "name: Bar\ntracks: []\n";
    std::fs::write(dir.join("song.yml"), bar).unwrap();
    p.state.player.reload_songs().await.unwrap();

    refused(p.delete("Foo").await, Code::NotFound);
    assert_eq!(std::fs::read_to_string(dir.join("song.yml")).unwrap(), bar);

    let foo = "name: Foo\ntracks: []\n";
    p.save("Foo", foo).await.unwrap();
    assert_eq!(std::fs::read_to_string(dir.join("song.yml")).unwrap(), bar);
    assert_eq!(std::fs::read_to_string(dir.join("song.yaml")).unwrap(), foo);
}

// ---- DeleteSong ----

#[tokio::test]
async fn delete_removes_the_config_and_the_song_from_the_playlists() {
    let p = project();
    let dir = p.make_song("Gone").await;
    p.make_song("Kept").await;
    // The playlists directory the player reloads from (the config's), and the
    // legacy playlist file.
    let mut sources = p.state.player.song_sources().unwrap();
    let playlists = sources
        .playlists_dir
        .clone()
        .expect("a playlists directory");
    std::fs::create_dir_all(&playlists).unwrap();
    std::fs::write(playlists.join("set.yaml"), "songs:\n- Gone\n- Kept\n").unwrap();
    std::fs::write(playlists.join("other.yml"), "songs:\n- Kept\n- Gone\n").unwrap();
    let legacy = p.root().join("playlist.yaml");
    std::fs::write(&legacy, "songs:\n- Gone\n- Kept\n").unwrap();
    sources.legacy_playlist = Some(legacy.clone());
    p.state.player.set_song_sources(sources);

    let deleted = p.delete("Gone").await.unwrap();
    assert_eq!(deleted.name, "Gone");
    assert!(!dir.join("song.yaml").exists());
    // The song's audio is left in place.
    assert!(dir.join("track1.wav").exists());
    assert!(p.state.player.songs().get("Gone").is_err());
    for file in [
        playlists.join("set.yaml"),
        playlists.join("other.yml"),
        legacy,
    ] {
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains("Gone"), "{file:?}: {text}");
        assert!(text.contains("Kept"), "{file:?}: {text}");
    }
}

#[tokio::test]
async fn delete_of_an_unknown_song_is_not_found() {
    let p = project();
    refused(p.delete("nonexistent").await, Code::NotFound);
    // A folder with no config: the song.yaml is not there to delete.
    std::fs::create_dir(p.songs().join("Empty")).unwrap();
    let status = refused(p.delete("Empty").await, Code::NotFound);
    assert_eq!(status.message(), "song.yaml not found for this song");
}

#[tokio::test]
async fn delete_of_the_song_playing_is_refused() {
    let (state, dir) = crate::webui::api::test_helpers::test_state_with_audio_device("mock-device");
    let store = Arc::new(config::ConfigStore::new(
        config::Player::deserialize(&state.config_path).unwrap(),
        state.config_path.clone(),
    ));
    state.player.set_config_store(store);
    state.player.set_locked(false);
    let server = ConfigServer::new(state.player.clone());
    // "Song A" is in memory only, with no file to delete: a folder with its
    // config stands for it on disk.
    let song_dir = state.songs_path().join("Song A");
    std::fs::create_dir(&song_dir).unwrap();
    std::fs::write(song_dir.join("song.yaml"), "name: Song A\ntracks: []\n").unwrap();

    state.player.play().await.unwrap();
    assert!(state.player.is_playing().await);
    let status = server
        .delete_song(Request::new(proto::DeleteSongRequest {
            name: "Song A".to_string(),
        }))
        .await
        .expect_err("a song playing is not deleted");
    state.player.stop().await;
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert_eq!(
        status.message(),
        "Cannot delete a song that is currently playing"
    );
    assert!(song_dir.join("song.yaml").exists());
    drop(dir);
}

// ---- GetSongWaveform ----

#[tokio::test]
async fn waveform_answers_a_track_with_its_peaks() {
    let p = project();
    p.make_song("WaveSong").await;
    let wave = p.waveform("WaveSong").await.unwrap();
    assert_eq!(wave.song_name, "WaveSong");
    assert_eq!(wave.tracks.len(), 1);
    assert_eq!(wave.tracks[0].name, "track1");
    // At most 500 buckets: a track shorter than its buckets' size has fewer.
    assert!((400..=500).contains(&wave.tracks[0].peaks.len()));
}

#[tokio::test]
async fn waveform_of_an_unknown_song_is_not_found() {
    let p = project();
    refused(p.waveform("nonexistent").await, Code::NotFound);
}

/// The second ask is served from the hub's cache, which the first filled;
/// the answer is the same.
#[tokio::test]
async fn waveform_is_cached_after_the_first_ask() {
    let p = project();
    p.make_song("CachedSong").await;
    let cache = p.state.player.watch_hub().waveform_cache().clone();
    assert!(cache.lock().get("CachedSong").is_none());
    let first = p.waveform("CachedSong").await.unwrap();
    assert!(cache.lock().get("CachedSong").is_some());
    // Marked in the cache, to tell that the second answer came from it.
    cache.lock().get_mut("CachedSong").unwrap()[0].1[0] = 0.75;
    let second = p.waveform("CachedSong").await.unwrap();
    assert_eq!(second.song_name, "CachedSong");
    assert_eq!(second.tracks.len(), first.tracks.len());
    assert_eq!(second.tracks[0].peaks[0], 0.75);
}

/// The longest waveform a song can have: 16 tracks, at most 500 peaks to a
/// track whatever the song's length. Its encoded size is far under every limit on
/// the path (tonic's 4 MiB decode limit is the request side; replies have
/// none, and Connect's browser transport reads up to 4 GiB).
#[tokio::test]
async fn a_sixteen_track_waveform_is_a_small_message() {
    let p = project();
    let dir = p.songs().join("Big");
    std::fs::create_dir(&dir).unwrap();
    let mut yaml = String::from("name: Big\ntracks:\n");
    for i in 1..=16 {
        crate::testutil::write_wav(
            dir.join(format!("track-{i:02}.wav")),
            vec![vec![i * 1000; 44100]],
            44100,
        )
        .unwrap();
        yaml.push_str(&format!(
            "  - name: Track number {i:02}\n    file: track-{i:02}.wav\n"
        ));
    }
    std::fs::write(dir.join("song.yaml"), yaml).unwrap();
    p.state.player.reload_songs().await.unwrap();

    let wave = p.waveform("Big").await.unwrap();
    assert_eq!(wave.tracks.len(), 16);
    assert!(wave
        .tracks
        .iter()
        .all(|t| (400..=500).contains(&t.peaks.len())));
    // Packed floats: 4 bytes a peak, and a few bytes of framing a track.
    let size = wave.encoded_len();
    assert!(size > 16 * 400 * 4, "{size}");
    assert!(size < 16 * (500 * 4 + 40), "{size}");
    // Well under the smallest limit on the path.
    assert!(size < 64 * 1024, "{size}");
}

// ---- GuessSongTempo ----

#[tokio::test]
async fn guess_of_an_unknown_song_is_not_found() {
    let p = project();
    refused(p.guess("nonexistent").await, Code::NotFound);
}

#[tokio::test]
async fn guess_of_a_song_with_no_midi_or_beat_grid_is_not_found() {
    let p = project();
    p.make_song("Plain").await;
    let status = refused(p.guess("Plain").await, Code::NotFound);
    assert_eq!(status.message(), "Song has no MIDI file or beat grid");
}

#[tokio::test]
async fn guess_reads_the_tempo_from_the_songs_midi_file() {
    let p = project();
    let dir = p.make_song("Scored").await;
    // A format 0 file at 480 ticks a beat, with a tempo of 120 and a 4/4.
    let mut midi = b"MThd\0\0\0\x06\0\0\0\x01\x01\xE0MTrk\0\0\0\x13".to_vec();
    midi.extend([0, 0xFF, 0x51, 3, 0x07, 0xA1, 0x20]);
    midi.extend([0, 0xFF, 0x58, 4, 4, 2, 24, 8]);
    midi.extend([0, 0xFF, 0x2F, 0]);
    std::fs::write(dir.join("song.mid"), midi).unwrap();
    std::fs::write(
        dir.join("song.yaml"),
        format!("name: Scored\nmidi_playback:\n  file: song.mid\n{ONE_TRACK}"),
    )
    .unwrap();
    p.state.player.reload_songs().await.unwrap();

    let guess = p.guess("Scored").await.unwrap();
    assert_eq!(guess.source(), proto::TempoGuessSource::Midi);
    let tempo = guess.tempo.expect("the guess");
    let config = guess.config.expect("the config");
    assert_eq!(tempo.bpm, 120.0);
    assert_eq!(
        (
            tempo.time_signature_numerator,
            tempo.time_signature_denominator
        ),
        (4, 4)
    );
    // The config is the guess as a `tempo:` block.
    assert_eq!(config.bpm, tempo.bpm);
    assert_eq!(
        config.time_signature,
        format!(
            "{}/{}",
            tempo.time_signature_numerator, tempo.time_signature_denominator
        )
    );
    assert_eq!(tempo.changes.len(), config.changes.len());
}

// ---- ListSongFiles ----

#[tokio::test]
async fn files_are_listed_by_name_with_their_kinds() {
    let p = project();
    let dir = p.make_song("FilesSong").await;
    std::fs::copy(midi_asset(), dir.join("notes.mid")).unwrap();
    std::fs::write(dir.join("show.light"), "show \"Test\" {}\n").unwrap();
    std::fs::write(dir.join("readme.txt"), "hello").unwrap();
    std::fs::copy(midi_asset(), dir.join("Other.MID")).unwrap();
    std::fs::write(dir.join("Bass.WAV"), "not read").unwrap();
    p.state.player.reload_songs().await.unwrap();

    use proto::FileKind::*;
    // song.yaml is left out; the rest by name, in byte order.
    assert_eq!(
        p.files("FilesSong").await.unwrap(),
        [
            ("Bass.WAV".to_string(), Audio),
            ("Other.MID".to_string(), Midi),
            ("notes.mid".to_string(), Midi),
            ("readme.txt".to_string(), Other),
            ("show.light".to_string(), Lighting),
            ("track1.wav".to_string(), Audio),
        ]
    );
}

/// The listing names a hidden file the song's config names
/// (`file: .click.wav`), and leaves out one it does not.
#[tokio::test]
async fn files_list_a_hidden_file_the_config_names() {
    let p = project();
    let dir = p.songs().join("Clicky");
    std::fs::create_dir(&dir).unwrap();
    crate::testutil::write_wav(dir.join(".click.wav"), vec![vec![0_i32; 4410]], 44100).unwrap();
    std::fs::write(dir.join(".draft.light"), "draft").unwrap();
    std::fs::write(
        dir.join("song.yaml"),
        "name: Clicky\ntracks:\n  - name: click\n    file: .click.wav\n",
    )
    .unwrap();
    p.state.player.reload_songs().await.unwrap();

    let names: Vec<_> = p
        .files("Clicky")
        .await
        .unwrap()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, [".click.wav"]);
}

/// The listing hides the song's config by name, `song.yml` as well as
/// `song.yaml`, and hidden files.
#[tokio::test]
async fn files_hide_a_song_yml_config_and_hidden_files() {
    let p = project();
    let dir = p.songs().join("YmlFiles");
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("song.yml"), "name: YmlFiles\ntracks: []\n").unwrap();
    std::fs::write(dir.join("readme.txt"), "hello").unwrap();
    std::fs::write(dir.join(".draft.light"), "draft").unwrap();
    std::fs::write(dir.join("._readme.txt"), "fork").unwrap();
    p.state.player.reload_songs().await.unwrap();

    let names: Vec<_> = p
        .files("YmlFiles")
        .await
        .unwrap()
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(names, ["readme.txt"]);
}

#[tokio::test]
async fn files_of_an_unknown_song_are_not_found() {
    let p = project();
    refused(p.files("nonexistent").await, Code::NotFound);
}

// ---- ImportSongFile ----

#[tokio::test]
async fn import_copies_a_project_file_into_a_new_song() {
    let p = project();
    let source = p.root().join("import_me.wav");
    std::fs::write(&source, create_test_wav()).unwrap();

    let imported = p
        .import("TestSong", &source.canonicalize().unwrap(), None)
        .await
        .unwrap();
    assert_eq!(imported.file, "import_me.wav");
    assert_eq!(imported.song, "TestSong");
    assert!(p.songs().join("TestSong/import_me.wav").exists());
    // The song has a config and is loaded.
    assert!(p.songs().join("TestSong/song.yaml").exists());
    assert!(p.state.player.songs().get("TestSong").is_ok());
}

/// A new song's name is the folder's unless a song has it already (#674).
#[tokio::test]
async fn import_into_a_name_another_song_holds_names_the_new_song_past_it() {
    let p = project();
    let source = p.root().join("a.wav");
    std::fs::write(&source, create_test_wav()).unwrap();
    // "Song A" is held in memory; a folder of that name is a new song.
    let imported = p
        .import("Song A", &source.canonicalize().unwrap(), None)
        .await
        .unwrap();
    assert_eq!(imported.song, "Song A (2)");
}

#[tokio::test]
async fn import_outside_the_project_is_refused() {
    let p = project();
    for path in ["/etc/hosts", "/etc/../etc/hosts", "../../../etc/hosts"] {
        let status = refused(
            p.import("TestSong", Path::new(path), None).await,
            Code::InvalidArgument,
        );
        assert!(
            status.message().contains("outside the project directory"),
            "{path}: {}",
            status.message()
        );
    }
    assert!(!p.songs().join("TestSong").exists());
}

/// A symlink in the project to a file outside it is no way to read that
/// file: the source resolves through the link and is refused.
#[cfg(unix)]
#[tokio::test]
async fn import_through_a_symlink_out_of_the_project_is_refused() {
    let p = project();
    let outside = tempfile::tempdir().unwrap();
    let secret = outside.path().join("secret.wav");
    std::fs::write(&secret, create_test_wav()).unwrap();
    std::os::unix::fs::symlink(&secret, p.root().join("link.wav")).unwrap();
    std::os::unix::fs::symlink(outside.path(), p.root().join("linked_dir")).unwrap();

    for source in [
        p.root().join("link.wav"),
        p.root().join("linked_dir/secret.wav"),
    ] {
        let status = refused(
            p.import("TestSong", &source, None).await,
            Code::InvalidArgument,
        );
        assert!(
            status.message().contains("outside the project directory"),
            "{}",
            status.message()
        );
    }
    assert!(!p.songs().join("TestSong").exists());
}

#[tokio::test]
async fn import_of_a_missing_source_is_refused() {
    let p = project();
    let missing = p.root().join("does_not_exist.wav");
    let status = refused(
        p.import("TestSong", &missing, None).await,
        Code::InvalidArgument,
    );
    assert!(status.message().contains("does not exist"));
}

#[tokio::test]
async fn import_of_a_directory_is_refused() {
    let p = project();
    let subdir = p.root().join("a_directory");
    std::fs::create_dir(&subdir).unwrap();
    let status = refused(
        p.import("TestSong", &subdir.canonicalize().unwrap(), None)
            .await,
        Code::InvalidArgument,
    );
    assert!(status.message().contains("not a file"));
}

#[tokio::test]
async fn import_can_rename_the_copy() {
    let p = project();
    let source = p.root().join("original.mid");
    std::fs::copy(midi_asset(), &source).unwrap();
    let imported = p
        .import(
            "TestSong",
            &source.canonicalize().unwrap(),
            Some("dmx_lightshow.mid"),
        )
        .await
        .unwrap();
    assert_eq!(imported.file, "dmx_lightshow.mid");
    assert!(p.songs().join("TestSong/dmx_lightshow.mid").exists());
}

/// An import's name is read in any case: `dmx_Lights.MID` is a MIDI file.
/// `.midi` is not one to the player, so it is refused.
#[tokio::test]
async fn import_reads_extensions_in_any_case() {
    let p = project();
    let source = p.root().join("original.mid");
    std::fs::copy(midi_asset(), &source).unwrap();
    let source = source.canonicalize().unwrap();

    p.import("TestSong", &source, Some("dmx_Lights.MID"))
        .await
        .unwrap();
    refused(
        p.import("TestSong", &source, Some("dmx_lights.midi")).await,
        Code::InvalidArgument,
    );
    assert!(p.songs().join("TestSong/dmx_Lights.MID").exists());
}

#[tokio::test]
async fn import_refuses_an_unsupported_extension_or_an_unsafe_name() {
    let p = project();
    let source = p.root().join("notes.wav");
    std::fs::write(&source, create_test_wav()).unwrap();
    let source = source.canonicalize().unwrap();

    let status = refused(
        p.import("TestSong", &source, Some("readme.txt")).await,
        Code::InvalidArgument,
    );
    assert!(status.message().contains("Unsupported file type"));
    for name in ["../escape.wav", "a/b.wav", ".hidden.wav", ""] {
        let status = refused(
            p.import("TestSong", &source, Some(name)).await,
            Code::InvalidArgument,
        );
        assert!(
            status.message().contains("Invalid filename")
                || status.message().contains("Unsupported file type"),
            "{name:?}: {}",
            status.message()
        );
    }
    assert!(!p.root().join("escape.wav").exists());
    assert!(!p.songs().join("escape.wav").exists());
}

// ---- BrowseDirectory ----

#[tokio::test]
async fn browse_defaults_to_the_project_root() {
    let p = project();
    for path in ["", "/"] {
        let listing = p.browse(path).await.unwrap();
        assert_eq!(listing.path, "/");
        assert_eq!(
            Path::new(&listing.root),
            p.root().canonicalize().unwrap().as_path()
        );
        assert!(listing.entries.iter().any(|e| e.name == "mtrack.yaml"));
        assert!(listing
            .entries
            .iter()
            .any(|e| e.name == "songs" && e.is_dir));
    }
}

#[tokio::test]
async fn browse_lists_a_subdirectory_with_project_relative_paths() {
    let p = project();
    let subdir = p.root().join("subdir");
    std::fs::create_dir(&subdir).unwrap();
    std::fs::write(subdir.join("file.txt"), "hello").unwrap();
    let listing = p.browse("/subdir").await.unwrap();
    assert_eq!(listing.path, "/subdir");
    assert_eq!(listing.entries.len(), 1);
    assert_eq!(listing.entries[0].path, "/subdir/file.txt");
    // A path with no leading slash is the same folder.
    assert_eq!(p.browse("subdir").await.unwrap().path, "/subdir");
}

#[tokio::test]
async fn browse_cannot_leave_the_project() {
    let p = project();
    for path in [
        "/../../../etc",
        "/..",
        "../",
        "/songs/../..",
        "/etc",
        "//etc",
        "/nonexistent",
    ] {
        let status = p.browse(path).await.unwrap_err();
        assert!(
            matches!(status.code(), Code::NotFound | Code::PermissionDenied),
            "{path}: {status:?}"
        );
    }
    // A file is no folder.
    refused(p.browse("/mtrack.yaml").await, Code::NotFound);
}

/// A symlink in the project to a folder outside it is not a way out.
#[cfg(unix)]
#[tokio::test]
async fn browse_through_a_symlink_out_of_the_project_is_refused() {
    let p = project();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "x").unwrap();
    std::os::unix::fs::symlink(outside.path(), p.root().join("linked")).unwrap();
    for path in ["/linked", "/linked/"] {
        let status = p.browse(path).await.unwrap_err();
        assert!(
            matches!(status.code(), Code::NotFound | Code::PermissionDenied),
            "{path}: {status:?}"
        );
    }
    // Neither can a song be made of it, nor a bulk import run on it.
    assert!(p.create_in("/linked", None).await.is_err());
    assert!(p.bulk("/linked").await.is_err());
    assert!(!outside.path().join("song.yaml").exists());
}

#[tokio::test]
async fn browse_hides_hidden_entries() {
    let p = project();
    let subdir = p.root().join("hiddendir");
    std::fs::create_dir_all(subdir.join(".git")).unwrap();
    std::fs::write(subdir.join("track.wav"), create_test_wav()).unwrap();
    std::fs::write(subdir.join("._track.wav"), "fork").unwrap();
    std::fs::write(subdir.join(".draft.light"), "draft").unwrap();

    let names: Vec<_> = p
        .browse("/hiddendir")
        .await
        .unwrap()
        .entries
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["track.wav"]);
}

#[tokio::test]
async fn browse_classifies_file_types() {
    let p = project();
    let subdir = p.root().join("typedir");
    std::fs::create_dir(&subdir).unwrap();
    for name in [
        "track.wav",
        "notes.mid",
        "show.light",
        "readme.txt",
        "Bass.WAV",
        "Notes.MID",
        "Show.LIGHT",
        "notes.midi",
    ] {
        std::fs::write(subdir.join(name), "data").unwrap();
    }
    std::fs::create_dir(subdir.join("sub")).unwrap();

    use proto::FileKind::*;
    let kinds: std::collections::HashMap<_, _> = p
        .browse("/typedir")
        .await
        .unwrap()
        .entries
        .into_iter()
        .map(|e| (e.name.clone(), e.kind()))
        .collect();
    for (name, kind) in [
        ("track.wav", Audio),
        ("notes.mid", Midi),
        ("show.light", Lighting),
        ("readme.txt", Other),
        ("Bass.WAV", Audio),
        ("Notes.MID", Midi),
        ("Show.LIGHT", Lighting),
        ("notes.midi", Other),
        ("sub", Directory),
    ] {
        assert_eq!(kinds[name], kind, "{name}");
    }
}

#[tokio::test]
async fn browse_sorts_directories_first_then_by_name() {
    let p = project();
    let subdir = p.root().join("sorttest");
    std::fs::create_dir(&subdir).unwrap();
    std::fs::write(subdir.join("aaa_file.txt"), "data").unwrap();
    std::fs::create_dir(subdir.join("zzz_dir")).unwrap();
    std::fs::write(subdir.join("Bbb_file.wav"), create_test_wav()).unwrap();
    std::fs::create_dir(subdir.join("aaa_dir")).unwrap();

    let entries = p.browse("/sorttest").await.unwrap().entries;
    let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        ["aaa_dir", "zzz_dir", "aaa_file.txt", "Bbb_file.wav"]
    );
    assert_eq!(
        entries.iter().map(|e| e.is_dir).collect::<Vec<_>>(),
        [true, true, false, false]
    );
}

// ---- CreateSongInDirectory ----

#[tokio::test]
async fn create_in_directory_scans_the_folder_and_loads_the_song() {
    let p = project();
    let dir = p.folder_with_wav("songs/mysong");
    let made = p.create_in("/songs/mysong", None).await.unwrap();
    assert_eq!(made.song, "mysong");
    assert!(Path::new(&made.path).ends_with("songs/mysong/song.yaml"));
    assert!(dir.join("song.yaml").exists());
    assert!(p.state.player.songs().get("mysong").is_ok());
}

#[tokio::test]
async fn create_in_directory_takes_a_name() {
    let p = project();
    let dir = p.folder_with_wav("songs/override_test");
    let made = p
        .create_in("/songs/override_test", Some("Custom Name"))
        .await
        .unwrap();
    assert_eq!(made.song, "Custom Name");
    let config = std::fs::read_to_string(dir.join("song.yaml")).unwrap();
    assert!(config.contains("Custom Name"), "{config}");
}

/// #674: a name given that a song has already, loaded or configured under
/// the songs root, is refused naming it, and nothing is written.
#[tokio::test]
async fn create_in_directory_refuses_a_name_taken() {
    let p = project();
    let other = p.songs().join("Other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("song.yaml"), "name: Fresh\ntracks: []\n").unwrap();
    let dir = p.folder_with_wav("songs/new");
    for taken in ["Song A", "Fresh"] {
        let status = refused(
            p.create_in("/songs/new", Some(taken)).await,
            Code::AlreadyExists,
        );
        assert_eq!(
            status.message(),
            format!("a song named \"{taken}\" already exists; choose another name")
        );
        let name = status.metadata().get_bin(TAKEN_NAME_KEY).expect("the name");
        assert_eq!(name.to_bytes().unwrap().as_ref(), taken.as_bytes());
        assert!(!dir.join("song.yaml").exists());
    }
    assert_eq!(
        p.create_in("/songs/new", Some("Free")).await.unwrap().song,
        "Free"
    );
}

/// #674: with no name given, the folder's, by the bulk import's rule: past a
/// name taken, the first free suffix.
#[tokio::test]
async fn create_in_directory_with_no_name_is_named_past_the_names_taken() {
    let p = project();
    let dir = p.folder_with_wav("songs/Song A");
    let made = p.create_in("/songs/Song A", None).await.unwrap();
    assert_eq!(made.song, "Song A (2)");
    let config = config::Song::deserialize(&dir.join("song.yaml")).unwrap();
    assert_eq!(config.name(), "Song A (2)");
}

#[tokio::test]
async fn create_in_directory_refuses_a_folder_that_has_a_config() {
    let p = project();
    // A song.yaml, a song.yml, and a flat song's `<name>.yaml` at the root.
    let existing = p.songs().join("existing");
    std::fs::create_dir_all(&existing).unwrap();
    std::fs::write(existing.join("song.yaml"), "name: existing\ntracks: []\n").unwrap();
    let yml = p.songs().join("yml");
    std::fs::create_dir_all(&yml).unwrap();
    std::fs::write(yml.join("song.yml"), "name: yml\ntracks: []\n").unwrap();

    let status = refused(
        p.create_in("/songs/existing", None).await,
        Code::AlreadyExists,
    );
    assert_eq!(
        status.message(),
        "a song config already exists in this directory"
    );
    refused(p.create_in("/songs/yml", None).await, Code::AlreadyExists);
    assert!(!yml.join("song.yaml").exists());

    std::fs::write(p.songs().join("flat.yaml"), "name: Flat\ntracks: []\n").unwrap();
    refused(p.create_in("/songs", None).await, Code::AlreadyExists);
    assert!(!p.songs().join("song.yaml").exists());
}

/// Beside a `song.yaml` of another kind the folder is refused and the file
/// left (`songs::song_config_to_write`); it was written over on main.
#[tokio::test]
async fn create_in_directory_keeps_a_song_yaml_of_another_kind() {
    let p = project();
    let dir = p.folder_with_wav("Pl");
    let playlist = "kind: playlist\nsongs: []\n";
    std::fs::write(dir.join("song.yaml"), playlist).unwrap();
    refused(p.create_in("/Pl", None).await, Code::AlreadyExists);
    assert_eq!(
        std::fs::read_to_string(dir.join("song.yaml")).unwrap(),
        playlist
    );
}

#[tokio::test]
async fn create_in_directory_refuses_what_is_not_a_project_folder() {
    let p = project();
    refused(p.create_in("/nonexistent/dir", None).await, Code::NotFound);
    // A path out of the project is no folder of it.
    for path in ["/../..", "/..", "/etc"] {
        assert!(p.create_in(path, None).await.is_err(), "{path}");
    }
    // A file is no folder.
    let status = refused(
        p.create_in("/mtrack.yaml", None).await,
        Code::InvalidArgument,
    );
    assert_eq!(status.message(), "Path is not a directory");
}

// ---- BulkImportSongs ----

#[tokio::test]
async fn bulk_import_creates_songs_and_skips_folders_with_configs() {
    let p = project();
    let parent = p.root().join("songs");
    for dir in ["Alpha", "Beta", "Delta"] {
        std::fs::create_dir_all(parent.join(dir)).unwrap();
        std::fs::write(parent.join(dir).join("track.wav"), create_test_wav()).unwrap();
    }
    // No audio and no subfolders: not a candidate.
    std::fs::create_dir_all(parent.join("Gamma")).unwrap();
    std::fs::write(parent.join("Delta/song.yaml"), "name: Delta\ntracks: []\n").unwrap();

    let imported = p.bulk("/songs").await.unwrap();
    assert_eq!(imported.created, ["Alpha", "Beta"]);
    assert_eq!(imported.skipped, ["Delta"]);
    assert!(imported.failed.is_empty());
    assert!(parent.join("Alpha/song.yaml").exists());
    assert!(parent.join("Beta/song.yaml").exists());
    assert!(!parent.join("Gamma/song.yaml").exists());
    // Loaded once, after.
    assert!(p.state.player.songs().get("Alpha").is_ok());
}

/// The bulk import passes over a hidden directory.
#[tokio::test]
async fn bulk_import_passes_over_hidden_dirs() {
    let p = project();
    let parent = p.root().join("songs");
    for song in ["Alpha", ".trash"] {
        std::fs::create_dir_all(parent.join(song)).unwrap();
        std::fs::write(parent.join(song).join("track.wav"), create_test_wav()).unwrap();
    }
    p.bulk("/songs").await.unwrap();
    assert!(parent.join("Alpha/song.yaml").exists());
    assert!(!parent.join(".trash/song.yaml").exists());
}

/// A directory whose only audio is `Bass.WAV` is imported with that track.
#[tokio::test]
async fn bulk_import_reads_extensions_in_any_case() {
    let p = project();
    let song = p.root().join("loud/Shout");
    std::fs::create_dir_all(&song).unwrap();
    std::fs::write(song.join("Bass.WAV"), create_test_wav()).unwrap();
    let imported = p.bulk("/loud").await.unwrap();
    assert_eq!(imported.created, ["Shout"]);
    let config = std::fs::read_to_string(song.join("song.yaml")).unwrap();
    assert!(config.contains("Bass.WAV"), "{config}");
}

/// The bulk import takes the walk's song folders (`songs::song_dirs`): a
/// MIDI-only folder is imported; a folder holding only hidden media is not;
/// one with a `song.yml` is skipped and left as it is.
#[tokio::test]
async fn bulk_import_takes_the_song_folders_walk() {
    let p = project();
    let parent = p.root().join("lib");
    for dir in ["Midi", "Hid", "Yml"] {
        std::fs::create_dir_all(parent.join(dir)).unwrap();
    }
    std::fs::write(parent.join("Midi/notes.mid"), "midi data").unwrap();
    std::fs::write(parent.join("Hid/._track.wav"), [0_u8, 5, 22, 7]).unwrap();
    std::fs::write(parent.join("Yml/track.wav"), create_test_wav()).unwrap();
    let yml = "name: Yml Live\ntracks: []\n";
    std::fs::write(parent.join("Yml/song.yml"), yml).unwrap();

    let imported = p.bulk("/lib").await.unwrap();
    assert_eq!(imported.created, ["Midi"]);
    assert_eq!(imported.skipped, ["Yml"]);
    assert!(parent.join("Midi/song.yaml").exists());
    assert!(!parent.join("Hid/song.yaml").exists());
    assert!(!parent.join("Yml/song.yaml").exists());
    assert_eq!(
        std::fs::read_to_string(parent.join("Yml/song.yml")).unwrap(),
        yml
    );
}

/// An unreadable subdirectory is passed over; an unreadable directory to
/// import from is an error.
#[cfg(unix)]
#[tokio::test]
async fn bulk_import_passes_over_an_unreadable_subdirectory() {
    use std::os::unix::fs::PermissionsExt;
    let p = project();
    let parent = p.root().join("lib");
    for dir in ["Foo", "Locked/Bar"] {
        std::fs::create_dir_all(parent.join(dir)).unwrap();
        std::fs::write(parent.join(dir).join("track.wav"), create_test_wav()).unwrap();
    }
    let lock = |path: &Path, mode| {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap()
    };
    lock(&parent.join("Locked"), 0o000);
    if std::fs::read_dir(parent.join("Locked")).is_ok() {
        // Running as root: the folder reads all the same.
        lock(&parent.join("Locked"), 0o755);
        return;
    }
    let all = p.bulk("/lib").await;
    let locked = p.bulk("/lib/Locked").await;
    lock(&parent.join("Locked"), 0o755);

    assert_eq!(all.unwrap().created, ["Foo"]);
    refused(locked, Code::Internal);
    assert!(!parent.join("Locked/Bar/song.yaml").exists());
}

/// The bulk import does not follow a symlinked folder, so it writes nothing
/// outside the tree; it looks into a folder holding only a `song.yaml` of
/// another kind and leaves that file; and a folder named as a song the
/// player has ("Song A") is named past it.
#[cfg(unix)]
#[tokio::test]
async fn bulk_import_symlinks_other_kinds_and_taken_names() {
    let p = project();
    let parent = p.root().join("lib");
    let outside = p.root().join("outside/Song");
    for dir in [&parent.join("Pl/Inner"), &parent.join("Song A"), &outside] {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("track.wav"), create_test_wav()).unwrap();
    }
    let playlist = "kind: playlist\nsongs: []\n";
    std::fs::write(parent.join("Pl/song.yaml"), playlist).unwrap();
    std::os::unix::fs::symlink(&outside, parent.join("Linked")).unwrap();

    let imported = p.bulk("/lib").await.unwrap();
    assert_eq!(imported.created, ["Pl/Inner", "Song A"]);
    assert!(imported.skipped.is_empty());
    assert!(!outside.join("song.yaml").exists());
    assert_eq!(
        std::fs::read_to_string(parent.join("Pl/song.yaml")).unwrap(),
        playlist
    );
    let named = config::Song::deserialize(&parent.join("Song A/song.yaml")).unwrap();
    assert_eq!(named.name(), "Song A (2)");
}

/// A bulk import names a folder past a song configured under the songs
/// folder that the player has not loaded.
#[tokio::test]
async fn bulk_import_names_past_songs_configured_under_the_songs_folder() {
    let p = project();
    let later = p.songs().join("Later");
    std::fs::create_dir_all(&later).unwrap();
    std::fs::write(later.join("song.yaml"), "name: Encore\ntracks: []\n").unwrap();
    assert!(p.state.player.songs().get("Encore").is_err());
    let encore = p.root().join("inbox/Encore");
    std::fs::create_dir_all(&encore).unwrap();
    std::fs::write(encore.join("track.wav"), create_test_wav()).unwrap();

    assert_eq!(p.bulk("/inbox").await.unwrap().created, ["Encore"]);
    let named = config::Song::deserialize(&encore.join("song.yaml")).unwrap();
    assert_eq!(named.name(), "Encore (2)");
}

#[tokio::test]
async fn bulk_import_of_a_missing_or_foreign_path_is_refused() {
    let p = project();
    refused(p.bulk("/nonexistent").await, Code::NotFound);
    for path in ["/..", "/../..", "/etc"] {
        assert!(p.bulk(path).await.is_err(), "{path}");
    }
    let status = refused(p.bulk("/mtrack.yaml").await, Code::InvalidArgument);
    assert_eq!(status.message(), "Path is not a directory");
}

#[tokio::test]
async fn bulk_import_of_an_empty_directory_creates_nothing() {
    let p = project();
    std::fs::create_dir_all(p.root().join("empty_parent")).unwrap();
    let imported = p.bulk("/empty_parent").await.unwrap();
    assert!(imported.created.is_empty() && imported.skipped.is_empty());
}

#[tokio::test]
async fn bulk_import_finds_songs_nested_below_folders() {
    let p = project();
    let parent = p.root().join("music");
    let deep = parent.join("Artist/Album/Track1");
    let shallow = parent.join("MySong");
    for dir in [&deep, &shallow] {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("track.wav"), create_test_wav()).unwrap();
    }
    let imported = p.bulk("/music").await.unwrap();
    assert!(imported.created.iter().any(|s| s.contains("Track1")));
    assert!(imported.created.iter().any(|s| s.contains("MySong")));
    assert!(deep.join("song.yaml").exists());
    assert!(shallow.join("song.yaml").exists());
    // Folders without audio get no config.
    assert!(!parent.join("Artist/song.yaml").exists());
    assert!(!parent.join("Artist/Album/song.yaml").exists());
}
