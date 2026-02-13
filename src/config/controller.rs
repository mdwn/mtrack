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
use std::{collections::HashMap, error::Error};

use midly::live::LiveEvent;
use serde::Deserialize;

use super::midi::{self, ToMidiEvent};

pub const DEFAULT_GRPC_PORT: u16 = 43234;
pub const DEFAULT_OSC_PORT: u16 = 43235;

fn default_osc_play() -> String {
    "/mtrack/play".to_string()
}
fn default_osc_prev() -> String {
    "/mtrack/prev".to_string()
}
fn default_osc_next() -> String {
    "/mtrack/next".to_string()
}
fn default_osc_stop() -> String {
    "/mtrack/stop".to_string()
}
fn default_osc_all_songs() -> String {
    "/mtrack/all_songs".to_string()
}
fn default_osc_playlist() -> String {
    "/mtrack/playlist".to_string()
}
fn default_osc_stop_samples() -> String {
    "/mtrack/samples/stop".to_string()
}
fn default_osc_status() -> String {
    "/mtrack/status".to_string()
}
fn default_osc_playlist_current() -> String {
    "/mtrack/playlist/current".to_string()
}
fn default_osc_playlist_current_song() -> String {
    "/mtrack/playlist/current_song".to_string()
}
fn default_osc_playlist_current_song_elapsed() -> String {
    "/mtrack/playlist/current_song/elapsed".to_string()
}

/// Allows users to specify various controllers.
#[derive(Deserialize, Clone)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Controller {
    Grpc(GrpcController),
    Midi(MidiController),
    Multi(HashMap<String, Controller>),
    Osc(Box<OscController>),
}

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
#[derive(Clone, Deserialize)]
pub struct OscController {
    /// The port to listen on.
    #[serde(default = "default_osc_port")]
    port: u16,
    /// The broadcast addresses including the port.
    #[serde(default)]
    broadcast_addresses: Vec<String>,
    /// The OSC address to look for to play the current song in the playlist.
    #[serde(default = "default_osc_play")]
    play: String,
    /// The OSC address to look for to move the playlist to the previous item.
    #[serde(default = "default_osc_prev")]
    prev: String,
    /// The OSC address to look for to move the playlist to the next item.
    #[serde(default = "default_osc_next")]
    next: String,
    /// The OSC address to look for to stop playback.
    #[serde(default = "default_osc_stop")]
    stop: String,
    /// The OSC address to look for to switch from the current playlist to an all songs playlist.
    #[serde(default = "default_osc_all_songs")]
    all_songs: String,
    /// The OSC address to look for to switch back to the current playlist.
    #[serde(default = "default_osc_playlist")]
    playlist: String,
    /// The OSC address to look for to stop all triggered samples.
    #[serde(default = "default_osc_stop_samples")]
    stop_samples: String,
    /// The OSC address to broadcast to display the current player status.
    #[serde(default = "default_osc_status")]
    status: String,
    /// The OSC address to broadcast the current playlist songs.
    #[serde(default = "default_osc_playlist_current")]
    playlist_current: String,
    /// The OSC address to broadcast to display the current song on the playlist.
    #[serde(default = "default_osc_playlist_current_song")]
    playlist_current_song: String,
    /// The OSC address to broadcast to display the current song elapsed duration.
    #[serde(default = "default_osc_playlist_current_song_elapsed")]
    playlist_current_song_elapsed: String,
}

fn default_osc_port() -> u16 {
    DEFAULT_OSC_PORT
}

impl Default for OscController {
    fn default() -> Self {
        OscController {
            port: DEFAULT_OSC_PORT,
            broadcast_addresses: Vec::new(),
            play: default_osc_play(),
            prev: default_osc_prev(),
            next: default_osc_next(),
            stop: default_osc_stop(),
            all_songs: default_osc_all_songs(),
            playlist: default_osc_playlist(),
            stop_samples: default_osc_stop_samples(),
            status: default_osc_status(),
            playlist_current: default_osc_playlist_current(),
            playlist_current_song: default_osc_playlist_current_song(),
            playlist_current_song_elapsed: default_osc_playlist_current_song_elapsed(),
        }
    }
}

impl OscController {
    #[cfg(test)]
    pub fn new() -> OscController {
        OscController::default()
    }

    /// Gets the port to listen on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Gets the broadcast addresses to broadcast OSC status messages to.
    pub fn broadcast_addresses(&self) -> &[String] {
        &self.broadcast_addresses
    }

    /// Gets the play OSC address.
    pub fn play(&self) -> &str {
        &self.play
    }

    /// Gets the prev OSC address.
    pub fn prev(&self) -> &str {
        &self.prev
    }

    /// Gets the next OSC address.
    pub fn next(&self) -> &str {
        &self.next
    }

    /// Gets the stop OSC address.
    pub fn stop(&self) -> &str {
        &self.stop
    }

    /// Gets the all songs OSC address.
    pub fn all_songs(&self) -> &str {
        &self.all_songs
    }

    /// Gets the playlist OSC address.
    pub fn playlist(&self) -> &str {
        &self.playlist
    }

    /// Gets the stop samples OSC address.
    pub fn stop_samples(&self) -> &str {
        &self.stop_samples
    }

    /// Gets the player status.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// Gets the playlist current OSC address.
    pub fn playlist_current(&self) -> &str {
        &self.playlist_current
    }

    /// Gets the playlist current song OSC address.
    pub fn playlist_current_song(&self) -> &str {
        &self.playlist_current_song
    }

    /// Gets the playlist current song elapsed OSC address.
    pub fn playlist_current_song_elapsed(&self) -> &str {
        &self.playlist_current_song_elapsed
    }
}
