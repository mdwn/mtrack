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

//! Players for tests: on the repo's `assets/` fixtures, or on songs held in
//! memory with no devices at all.

use std::{collections::HashMap, error::Error, path::Path, sync::Arc};

use crate::{
    config,
    dmx::engine::Engine,
    player::{Player, PlayerDevices},
    playlist::{self, Playlist},
    songs::{self, Song, Songs},
};

/// `playlist` under its own name, beside `all_songs` of `songs` if asked.
fn playlists_of(
    songs: Arc<Songs>,
    playlist: Arc<Playlist>,
    all_songs: bool,
) -> Result<HashMap<String, Arc<Playlist>>, Box<dyn Error>> {
    let mut playlists = HashMap::new();
    if all_songs {
        playlists.insert("all_songs".to_string(), playlist::from_songs(songs)?);
    }
    playlists.insert(playlist.name().to_string(), playlist);
    Ok(playlists)
}

/// The `assets/` fixtures' songs.
pub fn assets_songs() -> Result<Arc<Songs>, Box<dyn Error>> {
    songs::get_all_songs(Path::new("assets/songs"))
}

/// `assets/playlist.yaml` over `songs`, named `name`.
pub fn assets_playlist(name: &str, songs: Arc<Songs>) -> Result<Arc<Playlist>, Box<dyn Error>> {
    Ok(Playlist::new(
        name,
        &config::Playlist::deserialize(Path::new("assets/playlist.yaml"))?,
        songs,
    )?)
}

/// The `assets/` fixtures' playlists: `assets/playlist.yaml` as `playlist`,
/// and `all_songs`.
pub fn assets_playlists() -> Result<HashMap<String, Arc<Playlist>>, Box<dyn Error>> {
    let songs = assets_songs()?;
    playlists_of(songs.clone(), assets_playlist("playlist", songs)?, true)
}

/// A player on the `assets/` fixtures, built with [`AssetsPlayer::build`]
/// or [`AssetsPlayer::ready`]: `assets/playlist.yaml` playing, as
/// `playlist`, beside `all_songs`; a mock audio device and nothing else,
/// unless set.
pub struct AssetsPlayer {
    audio: Option<config::Audio>,
    midi: Option<config::Midi>,
    dmx: Option<config::Dmx>,
    track_mappings: HashMap<String, Vec<u16>>,
    all_songs_only: bool,
    playlist_yaml: Option<String>,
}

/// See [`AssetsPlayer`].
pub fn assets_player() -> AssetsPlayer {
    AssetsPlayer {
        audio: Some(config::Audio::new("mock-device")),
        midi: None,
        dmx: None,
        track_mappings: HashMap::new(),
        all_songs_only: false,
        playlist_yaml: None,
    }
}

impl AssetsPlayer {
    /// The audio device, or none.
    pub fn audio(mut self, audio: Option<config::Audio>) -> Self {
        self.audio = audio;
        self
    }

    /// A mock MIDI device of this name (`mock-midi-device` is the plain one).
    pub fn midi(mut self, device: &str) -> Self {
        self.midi = Some(config::Midi::new(device, None));
        self
    }

    /// The MIDI device's configuration, or none.
    pub fn midi_config(mut self, midi: Option<config::Midi>) -> Self {
        self.midi = midi;
        self
    }

    /// The DMX configuration, or none.
    pub fn dmx(mut self, dmx: Option<config::Dmx>) -> Self {
        self.dmx = dmx;
        self
    }

    pub fn track_mappings(mut self, track_mappings: HashMap<String, Vec<u16>>) -> Self {
        self.track_mappings = track_mappings;
        self
    }

    /// `playlist` as this YAML gives it, in place of `assets/playlist.yaml`.
    pub fn playlist_yaml(mut self, yaml: &str) -> Self {
        self.playlist_yaml = Some(yaml.to_string());
        self
    }

    /// Only `all_songs`, playing: no `playlist`.
    pub fn all_songs_only(mut self) -> Self {
        self.all_songs_only = true;
        self
    }

    /// The player, its hardware perhaps still starting.
    pub fn build(self) -> Result<Arc<Player>, Box<dyn Error>> {
        let (playlists, active) = if self.all_songs_only {
            let mut playlists = HashMap::new();
            playlists.insert(
                "all_songs".to_string(),
                playlist::from_songs(assets_songs()?)?,
            );
            (playlists, "all_songs")
        } else if let Some(yaml) = &self.playlist_yaml {
            let config: config::Playlist = ::config::Config::builder()
                .add_source(::config::File::from_str(yaml, ::config::FileFormat::Yaml))
                .build()?
                .try_deserialize()?;
            let songs = assets_songs()?;
            let playlist = Playlist::new("playlist", &config, songs.clone())?;
            (playlists_of(songs, playlist, true)?, "playlist")
        } else {
            (assets_playlists()?, "playlist")
        };
        Player::new(
            playlists,
            active.to_string(),
            &config::Player::new(
                vec![],
                self.audio,
                self.midi,
                self.dmx,
                self.track_mappings,
                "assets/songs",
            ),
            None,
        )
    }

    /// The player, its hardware started.
    pub async fn ready(self) -> Result<Arc<Player>, Box<dyn Error>> {
        let player = self.build()?;
        player.await_hardware_ready().await;
        Ok(player)
    }
}

/// A project on disk for the playlist writes: a temporary directory with the
/// `assets/` songs and a config beside them, an empty `playlists/` and a
/// player that reloads from there, with a config store and unlocked.
pub struct PlaylistProject {
    /// Holds the project; dropped with it.
    pub dir: tempfile::TempDir,
    pub player: Arc<Player>,
    pub store: Arc<config::ConfigStore>,
}

impl PlaylistProject {
    /// The file of the playlist `name`.
    pub fn file(&self, name: &str) -> std::path::PathBuf {
        self.dir
            .path()
            .join("playlists")
            .join(format!("{name}.yaml"))
    }

    /// Writes the playlist `name` as `yaml` and reloads the player.
    pub async fn write(&self, name: &str, yaml: &str) {
        std::fs::write(self.file(name), yaml).expect("write playlist");
        self.player.reload_songs().await.expect("reload");
    }
}

/// See [`PlaylistProject`].
pub async fn playlist_project() -> Result<PlaylistProject, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    std::fs::create_dir_all(dir.path().join("playlists"))?;
    // The songs name their files relative to themselves, in `assets/`.
    let assets = dir.path().join("assets");
    std::fs::create_dir_all(assets.join("songs"))?;
    for entry in std::fs::read_dir("assets")? {
        let entry = entry?;
        if entry.path().is_file() {
            std::fs::copy(entry.path(), assets.join(entry.file_name()))?;
        }
    }
    for entry in std::fs::read_dir("assets/songs")? {
        let entry = entry?;
        std::fs::copy(entry.path(), assets.join("songs").join(entry.file_name()))?;
    }
    let songs_dir = assets.join("songs");
    let config_path = dir.path().join("mtrack.yaml");
    std::fs::write(&config_path, "songs: assets/songs\n")?;
    let config = config::Player::deserialize(&config_path)?;
    let songs = songs::get_all_songs(&songs_dir)?;
    let playlists =
        crate::player::load_playlists(Some(&dir.path().join("playlists")), None, songs)?;
    let player = Player::new(
        playlists,
        "all_songs".to_string(),
        &config,
        Some(dir.path()),
    )?;
    let store = Arc::new(config::ConfigStore::new(config, config_path));
    player.set_config_store(store.clone());
    player.await_hardware_ready().await;
    player.set_locked(false);
    Ok(PlaylistProject { dir, player, store })
}

/// A player on the `assets/` fixtures' songs with the given devices, and no
/// configuration: `assets/playlist.yaml` playing as `playlist`, beside
/// `all_songs`.
pub fn assets_player_with_devices(devices: PlayerDevices) -> Result<Player, Box<dyn Error>> {
    Player::new_with_devices(devices, assets_playlists()?, "playlist".to_string(), None)
}

/// No devices at all.
pub fn no_devices() -> PlayerDevices {
    PlayerDevices {
        audio: None,
        mappings: None,
        midi: None,
        dmx_engine: None,
        sample_engine: None,
        trigger_engine: None,
    }
}

/// A player with no devices on songs held in memory, built with
/// [`MemoryPlayer::build`]: each named song with one track, `track1`, in a
/// playlist `test`, playing, beside `all_songs`, unless set.
pub struct MemoryPlayer {
    songs: Vec<String>,
    tracks: Vec<String>,
    playlist: String,
    all_songs: bool,
    dmx_engine: Option<Arc<Engine>>,
}

/// See [`MemoryPlayer`].
pub fn memory_player(songs: &[&str]) -> MemoryPlayer {
    MemoryPlayer {
        songs: songs.iter().map(|s| s.to_string()).collect(),
        tracks: vec!["track1".to_string()],
        playlist: "test".to_string(),
        all_songs: true,
        dmx_engine: None,
    }
}

impl MemoryPlayer {
    /// Every song's tracks.
    pub fn tracks(mut self, tracks: &[&str]) -> Self {
        self.tracks = tracks.iter().map(|t| t.to_string()).collect();
        self
    }

    /// The playlist's name.
    pub fn playlist(mut self, name: &str) -> Self {
        self.playlist = name.to_string();
        self
    }

    /// Only the playlist: no `all_songs`.
    pub fn without_all_songs(mut self) -> Self {
        self.all_songs = false;
        self
    }

    pub fn dmx_engine(mut self, engine: Arc<Engine>) -> Self {
        self.dmx_engine = Some(engine);
        self
    }

    /// The playlists, by name, to build a player on some other way.
    pub fn playlists(&self) -> HashMap<String, Arc<Playlist>> {
        let tracks: Vec<&str> = self.tracks.iter().map(String::as_str).collect();
        let songs = Arc::new(Songs::new(
            self.songs
                .iter()
                .map(|name| (name.clone(), Arc::new(Song::new_for_test(name, &tracks))))
                .collect(),
        ));
        let playlist = Playlist::new(
            &self.playlist,
            &config::Playlist::new(&self.songs),
            songs.clone(),
        )
        .unwrap();
        playlists_of(songs, playlist, self.all_songs).unwrap()
    }

    pub fn build(self) -> Arc<Player> {
        let playlists = self.playlists();
        let devices = PlayerDevices {
            dmx_engine: self.dmx_engine,
            ..no_devices()
        };
        Arc::new(Player::new_with_devices(devices, playlists, self.playlist, None).unwrap())
    }
}
