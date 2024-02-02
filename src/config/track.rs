// Copyright (C) 2024 Michael Wilson <mike@mdwn.dev>
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
use std::{error::Error, path::Path};

use serde::Deserialize;

/// A YAML representation of a track.
#[derive(Deserialize)]
pub(super) struct Track {
    /// The name of the track.
    name: String,
    /// The file associated with the track.
    file: String,
    /// The file channel of the track to use.
    file_channel: Option<u16>,
    /// The output channel on the audio interface to use.
    channel: u16,
}

impl Track {
    /// Converts this track configuration into a Track object.
    pub(super) fn to_track(&self, song_path: &Path) -> Result<crate::songs::Track, Box<dyn Error>> {
        crate::songs::Track::new(
            self.name.clone(),
            Path::join(song_path, self.file.clone()),
            self.file_channel,
            self.channel,
        )
    }
}
