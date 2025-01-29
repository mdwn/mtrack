use std::{error::Error, fs, path::PathBuf};

// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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
use serde::Deserialize;

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
    pub fn deserialize(file: &PathBuf) -> Result<Playlist, Box<dyn Error>> {
        Ok(serde_yaml::from_str(&fs::read_to_string(file)?)?)
    }

    /// Get all songs in the playlist.
    pub fn songs(&self) -> &Vec<String> {
        &self.songs
    }
}
