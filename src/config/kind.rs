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

use serde::{Deserialize, Serialize};

/// Identifies the type of an mtrack YAML configuration file.
#[derive(Deserialize, Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConfigKind {
    Song,
    Playlist,
    #[serde(rename = "hardware_profile")]
    HardwareProfile,
}

impl std::fmt::Display for ConfigKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigKind::Song => write!(f, "song"),
            ConfigKind::Playlist => write!(f, "playlist"),
            ConfigKind::HardwareProfile => write!(f, "hardware_profile"),
        }
    }
}

/// Lightweight YAML peek: reads only the `kind` field without deserializing the
/// entire file. Returns `None` if the file has no `kind` field or cannot be read.
pub fn peek_kind(path: &Path) -> Option<ConfigKind> {
    #[derive(Deserialize)]
    struct KindOnly {
        kind: Option<ConfigKind>,
    }

    config::Config::builder()
        .add_source(config::File::from(path))
        .build()
        .ok()?
        .try_deserialize::<KindOnly>()
        .ok()?
        .kind
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peek_kind_song() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("song.yaml");
        std::fs::write(&path, "kind: song\nname: test\ntracks: []\n").unwrap();
        assert_eq!(peek_kind(&path), Some(ConfigKind::Song));
    }

    #[test]
    fn peek_kind_playlist() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("playlist.yaml");
        std::fs::write(&path, "kind: playlist\nsongs:\n  - song1\n").unwrap();
        assert_eq!(peek_kind(&path), Some(ConfigKind::Playlist));
    }

    #[test]
    fn peek_kind_absent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("song.yaml");
        std::fs::write(&path, "name: test\ntracks: []\n").unwrap();
        assert_eq!(peek_kind(&path), None);
    }

    #[test]
    fn peek_kind_invalid_yaml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad.yaml");
        std::fs::write(&path, "{{not valid yaml!!").unwrap();
        assert_eq!(peek_kind(&path), None);
    }

    #[test]
    fn peek_kind_nonexistent() {
        assert_eq!(peek_kind(Path::new("/tmp/does_not_exist_12345.yaml")), None);
    }

    #[test]
    fn peek_kind_hardware_profile() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("profile.yaml");
        std::fs::write(&path, "kind: hardware_profile\nhostname: pi-a\n").unwrap();
        assert_eq!(peek_kind(&path), Some(ConfigKind::HardwareProfile));
    }

    #[test]
    fn display_kinds() {
        assert_eq!(ConfigKind::Song.to_string(), "song");
        assert_eq!(ConfigKind::Playlist.to_string(), "playlist");
        assert_eq!(ConfigKind::HardwareProfile.to_string(), "hardware_profile");
    }
}
