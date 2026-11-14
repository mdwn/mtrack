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

pub(crate) mod bluetooth;
pub(crate) mod browse;
pub(crate) mod devices;
pub(crate) mod fixture_settings;
pub(crate) mod fixture_test;
pub(crate) mod helpers;
pub(crate) mod lighting_api;
pub(crate) mod lighting_writes;
pub(crate) mod mvr_api;
pub(crate) mod playlists;
pub(crate) mod profiles;
pub(crate) mod sample_files;
pub(crate) mod songs_api;
pub(crate) mod status;
pub(crate) mod write_lock;

#[cfg(test)]
pub(crate) mod test_helpers {
    use crate::webui::server::WebUiState;
    use http_body_util::BodyExt;

    /// Creates a WebUiState with a test player and temp directories.
    /// The player's song registry contains "Song A" (in-memory only, not on disk).
    pub fn test_state() -> (WebUiState, tempfile::TempDir) {
        use crate::songs::{Song, Songs};
        use std::collections::HashMap;

        let mut map = HashMap::new();
        map.insert(
            "Song A".to_string(),
            std::sync::Arc::new(Song::new_for_test("Song A", &["kick", "snare"])),
        );
        let songs = std::sync::Arc::new(Songs::new(map));
        test_state_with_registry(songs)
    }

    /// Creates a WebUiState whose player holds an open mock audio device.
    ///
    /// The default test state holds none, which is the wrong shape for anything
    /// that behaves differently when a device is already in use.
    pub fn test_state_with_audio_device(name: &str) -> (WebUiState, tempfile::TempDir) {
        use crate::songs::{Song, Songs};
        use std::collections::HashMap;

        let mut map = HashMap::new();
        map.insert(
            "Song A".to_string(),
            // An hour long: the tests that play it look while it plays, and a
            // zero-length song has often ended by then on a loaded machine.
            std::sync::Arc::new(Song::new_for_test_lasting(
                "Song A",
                &["kick", "snare"],
                std::time::Duration::from_secs(3600),
            )),
        );
        test_state_inner(
            std::sync::Arc::new(Songs::new(map)),
            Some(std::sync::Arc::new(crate::audio::mock::Device::get(name))),
        )
    }

    /// Creates a WebUiState with the given song registry.
    pub fn test_state_with_registry(
        songs: std::sync::Arc<crate::songs::Songs>,
    ) -> (WebUiState, tempfile::TempDir) {
        test_state_inner(songs, None)
    }

    /// Creates a WebUiState whose player runs the given DMX engine.
    pub fn test_state_with_dmx(
        songs: std::sync::Arc<crate::songs::Songs>,
        dmx_engine: std::sync::Arc<crate::dmx::engine::Engine>,
    ) -> (WebUiState, tempfile::TempDir) {
        test_state_with_hardware(songs, None, Some(dmx_engine))
    }

    fn test_state_inner(
        songs: std::sync::Arc<crate::songs::Songs>,
        audio: Option<std::sync::Arc<dyn crate::audio::Device>>,
    ) -> (WebUiState, tempfile::TempDir) {
        test_state_with_hardware(songs, audio, None)
    }

    fn test_state_with_hardware(
        songs: std::sync::Arc<crate::songs::Songs>,
        audio: Option<std::sync::Arc<dyn crate::audio::Device>>,
        dmx_engine: Option<std::sync::Arc<crate::dmx::engine::Engine>>,
    ) -> (WebUiState, tempfile::TempDir) {
        let all_songs_playlist = crate::playlist::from_songs(songs).unwrap();
        let mut playlists = std::collections::HashMap::new();
        playlists.insert("all_songs".to_string(), all_songs_playlist);
        test_state_with_playlists(playlists, "all_songs", audio, dmx_engine)
    }

    fn test_state_with_playlists(
        playlists: std::collections::HashMap<String, std::sync::Arc<crate::playlist::Playlist>>,
        active: &str,
        audio: Option<std::sync::Arc<dyn crate::audio::Device>>,
        dmx_engine: Option<std::sync::Arc<crate::dmx::engine::Engine>>,
    ) -> (WebUiState, tempfile::TempDir) {
        test_state_on_disk(playlists, active, audio, dmx_engine, "songs: []\n", true)
    }

    /// A WebUiState whose player has `playlists`, `active` playing, in a
    /// temporary directory holding `mtrack.yaml`, an empty `songs/`, and
    /// the legacy `playlist.yaml` holding `legacy_playlist`; the directory
    /// the player's playlists directory too if `playlists_dir`.
    pub(in crate::webui) fn test_state_on_disk(
        playlists: std::collections::HashMap<String, std::sync::Arc<crate::playlist::Playlist>>,
        active: &str,
        audio: Option<std::sync::Arc<dyn crate::audio::Device>>,
        dmx_engine: Option<std::sync::Arc<crate::dmx::engine::Engine>>,
        legacy_playlist: &str,
        playlists_dir: bool,
    ) -> (WebUiState, tempfile::TempDir) {
        use crate::player::PlayerDevices;

        let dir = tempfile::tempdir().unwrap();

        // Create a minimal config file
        let config_path = dir.path().join("mtrack.yaml");
        std::fs::write(&config_path, "songs: songs\n").unwrap();

        // Create a minimal playlist file
        let playlist_path = dir.path().join("playlist.yaml");
        std::fs::write(&playlist_path, legacy_playlist).unwrap();

        // Create songs directory
        let songs_path = dir.path().join("songs");
        std::fs::create_dir(&songs_path).unwrap();

        // Mappings with the device, or the player counts no audio to play
        // and a song finishes the moment it starts, whatever its length.
        let mappings = audio
            .as_ref()
            .map(|_| std::sync::Arc::new(std::collections::HashMap::new()));
        let devices = PlayerDevices {
            audio,
            mappings,
            dmx_engine,
            ..crate::testutil::player::no_devices()
        };
        let player = std::sync::Arc::new(
            crate::player::Player::new_with_devices(devices, playlists, active.to_string(), None)
                .unwrap(),
        );

        player.set_song_sources(crate::player::SongSources {
            songs: songs_path,
            playlists_dir: playlists_dir.then(|| dir.path().to_path_buf()),
            legacy_playlist: Some(playlist_path),
        });
        let state = WebUiState::new(super::super::server::WebUiStateParams {
            player,
            config_path,
        });

        (state, dir)
    }

    pub async fn response_body(response: axum::response::Response) -> String {
        let body = response.into_body();
        let bytes = body.collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    /// Creates a minimal valid WAV file for upload tests.
    pub fn create_test_wav() -> Vec<u8> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wav");
        crate::testutil::write_wav(path.clone(), vec![vec![0_i32; 4410]], 44100).unwrap();
        std::fs::read(&path).unwrap()
    }
}
