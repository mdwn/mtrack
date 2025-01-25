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
use std::{
    error::Error,
    path::{Path, PathBuf},
};

use serde::Deserialize;

use crate::songs;

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
    /// MIDI playback configuration. Will override the midi_file field.
    midi_playback: Option<MidiPlayback>,
    /// The light show configurations.
    light_shows: Option<Vec<LightShow>>,
    /// The associated tracks to play.
    tracks: Vec<track::Track>,
}

// A YAML representation of MIDI files with channel exclusions.
#[derive(Deserialize)]
pub(super) struct MidiPlayback {
    /// The MIDI file.
    file: String,

    /// The MIDI channels to exclude from this MIDI file. Useful if you want to exclude lighting
    /// data from being played back with other MIDI automation.
    exclude_midi_channels: Option<Vec<u8>>,
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

        let midi_playback = if let Some(midi_playback) = &self.midi_playback {
            Some(crate::songs::MidiPlayback {
                file: song_path.join(PathBuf::from(midi_playback.file.clone())),
                exclude_midi_channels: midi_playback
                    .exclude_midi_channels
                    .as_ref()
                    .map_or_else(Vec::new, |channels| {
                        channels.clone().iter().map(|val| val - 1).collect()
                    }),
            })
        } else {
            self.midi_file
                .as_ref()
                .map(|midi_file| crate::songs::MidiPlayback {
                    file: song_path.join(PathBuf::from(midi_file)),
                    exclude_midi_channels: Vec::new(),
                })
        };

        crate::songs::Song::new(
            self.name.clone(),
            self.midi_event
                .as_ref()
                .map(|event| event.to_midi_event())
                .map_or(Ok(None), |result| result.map(Some))?,
            midi_playback,
            self.light_shows
                .as_ref()
                .map_or_else(Vec::new, |light_shows| {
                    light_shows
                        .iter()
                        .map(|light_show| crate::songs::LightShow {
                            universe_name: light_show.universe_name.clone(),
                            dmx_file: song_path.join(PathBuf::from(light_show.dmx_file.clone())),
                            midi_channels: light_show
                                .midi_channels
                                .clone()
                                .unwrap_or_default()
                                .iter()
                                .map(|val| val - 1)
                                .collect(),
                        })
                        .collect()
                }),
            self.tracks
                .iter()
                .map(|track| songs::Track::new(track.with_song_path(&song_path)))
                .collect::<Result<Vec<crate::songs::Track>, _>>()?,
        )
    }
}
