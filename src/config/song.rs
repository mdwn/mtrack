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
use std::error::Error;

use midly::live::LiveEvent;
use serde::Deserialize;

use super::{
    midi::{self, ToMidiEvent},
    track::Track,
};

/// A YAML represetnation of a song.
#[derive(Deserialize)]
pub struct Song {
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
    tracks: Vec<Track>,
}

impl Song {
    /// Creates a new song configuration.
    #[cfg(test)]
    pub fn new(name: &str, tracks: Vec<Track>) -> Song {
        Song {
            name: name.to_string(),
            midi_event: None,
            midi_file: None,
            midi_playback: None,
            light_shows: None,
            tracks,
        }
    }

    /// Gets the name of the song.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the MIDI event associated with the song.
    pub fn midi_event(&self) -> Result<Option<LiveEvent<'static>>, Box<dyn Error>> {
        Ok(match &self.midi_event {
            Some(midi_event) => Some(midi_event.to_midi_event()?),
            None => None,
        })
    }

    /// Gets the MIDI playback associated with the song.
    pub fn midi_playback(&self) -> Option<MidiPlayback> {
        if let Some(midi_playback) = &self.midi_playback {
            return Some(midi_playback.clone());
        } else if let Some(midi_file) = &self.midi_file {
            return Some(MidiPlayback {
                file: midi_file.clone(),
                exclude_midi_channels: None,
            });
        }

        None
    }

    /// Gets the light shows associated with the song.
    pub fn light_shows(&self) -> Option<&Vec<LightShow>> {
        self.light_shows.as_ref()
    }

    /// Gets the tracks associated with the song.
    pub fn tracks(&self) -> &Vec<Track> {
        &self.tracks
    }
}

// A YAML representation of MIDI files with channel exclusions.
#[derive(Deserialize, Clone)]
pub struct MidiPlayback {
    /// The MIDI file.
    file: String,

    /// The MIDI channels to exclude from this MIDI file. Useful if you want to exclude lighting
    /// data from being played back with other MIDI automation.
    exclude_midi_channels: Option<Vec<u8>>,
}

impl MidiPlayback {
    /// Gets the file associated with the MIDI playback.
    pub fn file(&self) -> String {
        self.file.clone()
    }

    /// Gets the MIDI channels to exclude.
    pub fn exclude_midi_channels(&self) -> Vec<u8> {
        self.exclude_midi_channels.clone().unwrap_or_default()
    }
}

// A YAML representation of light shows.
#[derive(Deserialize, Clone)]
pub struct LightShow {
    /// The name of the universe. Will be matched against the universes configured in the DMX engine
    /// to determine where (if anywhere) this light show should be sent.
    universe_name: String,

    /// The associated MIDI file to interpret as DMX to play.
    dmx_file: String,

    /// The MIDI channels from this MIDI file to use as lighting data. If empty,
    /// all channels will be used.
    midi_channels: Option<Vec<u8>>,
}

impl LightShow {
    /// Gets the universe name for the light show.
    pub fn universe_name(&self) -> String {
        self.universe_name.clone()
    }

    /// Gets the DMX (MIDI) file associated with the light show.
    pub fn dmx_file(&self) -> String {
        self.dmx_file.clone()
    }

    /// Gets the MIDI channels that should be associated with light show data.
    pub fn midi_channels(&self) -> Vec<u8> {
        self.midi_channels.clone().unwrap_or_default()
    }
}
