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
use std::{collections::HashMap, error::Error};

use midly::live::LiveEvent;
use serde::Deserialize;

use super::midi::{self, ToMidiEvent};

/// Allows users to specify various controllers.
#[derive(Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Controller {
    Keyboard,
    Midi(MidiController),
    Multi(HashMap<String, Controller>),
}

#[derive(Deserialize)]
pub struct KeyboardController {}

/// The configuration that maps MIDI events to controller messages.
#[derive(Deserialize, Clone)]
pub struct MidiController {
    /// The MIDI event to look for to play the current song in the playlist.
    play: midi::Event,
    /// The MIDI event to look for to move the playlist to the previous item.
    prev: midi::Event,
    /// The MIDI event to look for to move the playlist to the next item.
    next: midi::Event,
    /// The MIDI event to look for to stop playback.
    stop: midi::Event,
    /// The MIDI event to look for to switch from the current playlist to an all songs playlist.
    all_songs: midi::Event,
    /// The MIDI event to look for to switch back to the current playlist.
    playlist: midi::Event,
}

impl MidiController {
    /// Gets the play event.
    pub fn play(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        self.play.to_midi_event()
    }

    /// Gets the prev event.
    pub fn prev(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        self.prev.to_midi_event()
    }

    /// Gets the next event.
    pub fn next(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        self.next.to_midi_event()
    }

    /// Gets the stop event.
    pub fn stop(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        self.stop.to_midi_event()
    }

    /// Gets the all songs event.
    pub fn all_songs(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        self.all_songs.to_midi_event()
    }

    /// Gets the playlist event.
    pub fn playlist(&self) -> Result<LiveEvent<'static>, Box<dyn Error>> {
        self.playlist.to_midi_event()
    }
}
