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
use serde::Deserialize;

use super::controller::Controller;
use super::midi;
use super::trackmappings::TrackMappings;

/// The configuration for the multitrack player.
#[derive(Deserialize)]
pub(super) struct Player {
    /// The controller configuration.
    pub controller: Controller,
    /// The audio device to use.
    pub audio_device: String,
    /// The track mappings for the player.
    pub track_mappings: TrackMappings,
    /// The MIDI device to use.
    pub midi_device: Option<String>,
    /// Events to emit to report status out via MIDI.
    pub status_events: Option<StatusEvents>,
    /// The path to the song definitions.
    pub songs: String,
}

/// The configuration for emitting status events.
#[derive(Deserialize)]
pub(super) struct StatusEvents {
    /// The event to emit to clear the status.
    pub off_event: midi::Event,
    /// The event to emit to indicate that the player is idling and waiting for input.
    pub idling_event: midi::Event,
    /// The event to emit to indicate that the player is currently playing.
    pub playing_event: midi::Event,
}
