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
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// A YAML representation of a track.
#[derive(Deserialize, Clone)]
pub(crate) struct Track {
    /// The name of the track.
    name: String,
    /// The file associated with the track.
    file: String,
    /// The file channel of the track to use.
    file_channel: Option<u16>,
}

impl Track {
    /// Creates a new track config.
    pub(crate) fn new(name: String, file: String, file_channel: Option<u16>) -> Track {
        Track {
            name,
            file,
            file_channel,
        }
    }

    /// Gets the name of the track.
    pub(crate) fn name(&self) -> String {
        self.name.clone()
    }

    /// Gets the file associated with the track.
    pub(crate) fn file(&self) -> String {
        self.file.clone()
    }

    /// Gets the file channel of the track to use.
    pub(crate) fn file_channel(&self) -> Option<u16> {
        self.file_channel
    }

    /// Creates a new copy of track with the song path prefixed to the file path.
    pub(crate) fn with_song_path(&self, song_path: &PathBuf) -> Track {
        Self::new(
            self.name.clone(),
            Path::join(song_path, self.file.clone())
                .to_str()
                .expect("unable to decode song path")
                .into(),
            self.file_channel,
        )
    }
}
