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
use serde::{Deserialize, Serialize};

/// A YAML representation of a track.
#[derive(Deserialize, Clone, Serialize)]
pub struct Track {
    /// The name of the track.
    name: String,
    /// The file associated with the track.
    file: String,
    /// The file channel of the track to use.
    file_channel: Option<u16>,
}

impl Track {
    /// Creates a new track config.
    pub fn new(name: String, file: &str, file_channel: Option<u16>) -> Track {
        Track {
            name,
            file: file.to_string(),
            file_channel,
        }
    }

    /// Gets the name of the track.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the file associated with the track.
    pub fn file(&self) -> &str {
        &self.file
    }

    /// Gets the file channel of the track to use.
    pub fn file_channel(&self) -> Option<u16> {
        self.file_channel
    }
}
