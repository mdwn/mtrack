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

/// The configuration for a playlist.
#[derive(Deserialize, Serialize)]
pub struct Playlist {
    /// Identifies this config file as a playlist.
    #[serde(default = "default_playlist_kind")]
    kind: super::kind::ConfigKind,
    /// The songs that belong to this playlist.
    songs: Vec<String>,
}

impl Playlist {
    /// Creates a new playlist configuration.
    pub fn new(songs: &[String]) -> Playlist {
        Playlist {
            kind: super::kind::ConfigKind::Playlist,
            songs: songs.to_owned(),
        }
    }

    /// Parse a playlist from a YAML file.
    pub fn deserialize(path: &Path) -> Result<Playlist, ConfigError> {
        Ok(Config::builder()
            .add_source(File::from(path))
            .build()?
            .try_deserialize::<Playlist>()?)
    }

    /// Get all songs in the playlist.
    pub fn songs(&self) -> &[String] {
        &self.songs
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
}
