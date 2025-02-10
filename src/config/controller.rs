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

pub const DEFAULT_GRPC_PORT: u16 = 43234;
pub const DEFAULT_OSC_PORT: u16 = 43235;

const DEFAULT_OSC_PLAY: &str = "/mtrack/play";
const DEFAULT_OSC_PREV: &str = "/mtrack/prev";
const DEFAULT_OSC_NEXT: &str = "/mtrack/next";
const DEFAULT_OSC_STOP: &str = "/mtrack/stop";
const DEFAULT_OSC_ALL_SONGS: &str = "/mtrack/all_songs";
const DEFAULT_OSC_PLAYLIST: &str = "/mtrack/playlist";
const DEFAULT_OSC_STATUS: &str = "/mtrack/status";
const DEFAULT_OSC_PLAYLIST_CURRENT: &str = "/mtrack/playlist/current";
const DEFAULT_OSC_PLAYLIST_CURRENT_SONG: &str = "/mtrack/playlist/current_song";
const DEFAULT_OSC_PLAYLIST_CURRENT_SONG_ELAPSED: &str = "/mtrack/playlist/current_song/elapsed";

/// Allows users to specify various controllers.
#[derive(Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Controller {
    Grpc(GrpcController),
    Keyboard,
    Midi(MidiController),
    Multi(HashMap<String, Controller>),
    Osc(Box<OscController>),
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
    #[cfg(test)]
    pub fn new(
        play: midi::Event,
        prev: midi::Event,
        next: midi::Event,
        stop: midi::Event,
        all_songs: midi::Event,
        playlist: midi::Event,
    ) -> MidiController {
        MidiController {
            play,
            prev,
            next,
            stop,
            all_songs,
            playlist,
        }
    }
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

/// The configuration for the multitrack player gRPC server.
#[derive(Clone, Default, Deserialize)]
pub struct GrpcController {
    /// The port to listen on.
    port: Option<u16>,
}

impl GrpcController {
    #[cfg(test)]
    pub fn new(port: u16) -> GrpcController {
        GrpcController { port: Some(port) }
    }

    /// Gets the port to listen on.
    pub fn port(&self) -> u16 {
        self.port.unwrap_or(DEFAULT_GRPC_PORT)
    }
}

/// The configuration for the multitrack player OSC server.
#[derive(Clone, Default, Deserialize)]
pub struct OscController {
    /// The port to listen on.
    port: Option<u16>,
    /// The broadcast addresses including the port.
    broadcast_addresses: Option<Vec<String>>,
    /// The OSC address to look for to play the current song in the playlist.
    play: Option<String>,
    /// The OSC address to look for to move the playlist to the previous item.
    prev: Option<String>,
    /// The OSC address to look for to move the playlist to the next item.
    next: Option<String>,
    /// The OSC address to look for to stop playback.
    stop: Option<String>,
    /// The OSC address to look for to switch from the current playlist to an all songs playlist.
    all_songs: Option<String>,
    /// The OSC address to look for to switch back to the current playlist.
    playlist: Option<String>,
    /// The OSC address to broadcast to display the current player status.
    status: Option<String>,
    /// The OSC address to broadcast the current playlist songs.
    playlist_current: Option<String>,
    /// The OSC address to broadcast to display the current song on the playlist.
    playlist_current_song: Option<String>,
    /// The OSC address to broadcast to display the current song elapsed duration.
    playlist_current_song_elapsed: Option<String>,
}

impl OscController {
    #[cfg(test)]
    pub fn new() -> OscController {
        OscController {
            port: None,
            broadcast_addresses: None,
            play: None,
            prev: None,
            next: None,
            stop: None,
            all_songs: None,
            playlist: None,
            status: None,
            playlist_current: None,
            playlist_current_song: None,
            playlist_current_song_elapsed: None,
        }
    }

    /// Gets the port to listen on.
    pub fn port(&self) -> u16 {
        self.port.unwrap_or(DEFAULT_OSC_PORT)
    }

    /// Gets the broadcast addresses to broadcast OSC status messages to.
    pub fn broadcast_addresses(&self) -> Vec<String> {
        self.broadcast_addresses.clone().unwrap_or_default()
    }

    /// Gets the play OSC address.
    pub fn play(&self) -> String {
        self.play.clone().unwrap_or(DEFAULT_OSC_PLAY.to_string())
    }

    /// Gets the prev OSC address.
    pub fn prev(&self) -> String {
        self.prev.clone().unwrap_or(DEFAULT_OSC_PREV.to_string())
    }

    /// Gets the next OSC address.
    pub fn next(&self) -> String {
        self.next.clone().unwrap_or(DEFAULT_OSC_NEXT.to_string())
    }

    /// Gets the stop OSC address.
    pub fn stop(&self) -> String {
        self.stop.clone().unwrap_or(DEFAULT_OSC_STOP.to_string())
    }

    /// Gets the all songs OSC address.
    pub fn all_songs(&self) -> String {
        self.all_songs
            .clone()
            .unwrap_or(DEFAULT_OSC_ALL_SONGS.to_string())
    }

    /// Gets the playlist OSC address.
    pub fn playlist(&self) -> String {
        self.playlist
            .clone()
            .unwrap_or(DEFAULT_OSC_PLAYLIST.to_string())
    }

    /// Gets the player status.
    pub fn status(&self) -> String {
        self.status
            .clone()
            .unwrap_or(DEFAULT_OSC_STATUS.to_string())
    }

    /// Gets the playlist current OSC address.
    pub fn playlist_current(&self) -> String {
        self.playlist_current
            .clone()
            .unwrap_or(DEFAULT_OSC_PLAYLIST_CURRENT.to_string())
    }

    /// Gets the playlist current song OSC address.
    pub fn playlist_current_song(&self) -> String {
        self.playlist_current_song
            .clone()
            .unwrap_or(DEFAULT_OSC_PLAYLIST_CURRENT_SONG.to_string())
    }

    /// Gets the playlist current song elapsed OSC address.
    pub fn playlist_current_song_elapsed(&self) -> String {
        self.playlist_current_song_elapsed
            .clone()
            .unwrap_or(DEFAULT_OSC_PLAYLIST_CURRENT_SONG_ELAPSED.to_string())
    }
}
