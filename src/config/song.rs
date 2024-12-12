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
use std::{
    error::Error,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use super::{
    midi::{self, ToMidiEvent},
    track,
};

/// A YAML represetnation of a song.
#[derive(Deserialize)]
pub(super) struct Song {
    // The path to the song configuration file.
    #[serde(skip)]
    pub song_file: PathBuf,
    /// The name of the song.
    name: String,
    /// The MIDI event to emit when the song is selected.
    midi_event: Option<midi::Event>,
    /// The associated MIDI file to play.
    midi_file: Option<String>,
    /// The light show configurations.
    light_shows: Option<Vec<LightShow>>,
    /// The associated tracks to play.
    tracks: Vec<track::Track>,
}

// A YAML representation of light shows.
#[derive(Deserialize)]
pub(super) struct LightShow {
    /// The name of the universe. Will be matched against the universes configured in the DMX engine
    /// to determine where (if anywhere) this light show should be sent.
    universe_name: String,

    /// The associated MIDI file to interpret as DMX to play.
    dmx_file: String,

    /// The MIDI channels from this MIDI file to use as lighting data. If empty,
    /// all channels will be used.
    midi_channels: Option<Vec<u8>>,
}

impl Song {
    /// Converts the config song into a proper song object.
    pub(super) fn to_song(&self) -> Result<crate::songs::Song, Box<dyn Error>> {
        // Get the absolute path to the song file and its parent path.
        let song_path = match self.song_file.canonicalize()?.parent() {
            Some(path) => path,
            None => Path::new("/"),
        }
        .to_path_buf();

        crate::songs::Song::new(
            self.name.clone(),
            self.midi_event
                .as_ref()
                .map(|event| event.to_midi_event())
                .map_or(Ok(None), |result| result.map(Some))?,
            self.midi_file
                .as_ref()
                .map(|midi_file| song_path.join(PathBuf::from(midi_file))),
            self.light_shows
                .as_ref()
                .map_or_else(Vec::new, |light_shows| {
                    light_shows
                        .iter()
                        .map(|light_show| crate::songs::LightShow {
                            universe_name: light_show.universe_name.clone(),
                            dmx_file: song_path.join(PathBuf::from(light_show.dmx_file.clone())),
                            midi_channels: light_show.midi_channels.clone().unwrap_or_default(),
                        })
                        .collect()
                }),
            self.tracks
                .iter()
                .map(|track| track.to_track(&song_path))
                .collect::<Result<Vec<crate::songs::Track>, _>>()?,
        )
    }
}
