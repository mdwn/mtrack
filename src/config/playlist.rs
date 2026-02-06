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
use serde::Deserialize;

use super::error::ConfigError;

/// The configuration for a playlist.
#[derive(Deserialize)]
pub struct Playlist {
    /// The songs that belong to this playlist.
    songs: Vec<String>,
}

impl Playlist {
    /// Creates a new playlist configuration.
    pub fn new(songs: &[String]) -> Playlist {
        Playlist {
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
    pub fn songs(&self) -> &Vec<String> {
        &self.songs
    }
}
