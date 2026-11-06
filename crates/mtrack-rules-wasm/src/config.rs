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
//! The player's `crate::config` as the web UI's types see it: the plain
//! data of a config -- what a file says, a profile's sections, a song's
//! sections and tracks, a playlist's settings -- each source compiled as it
//! is in the player. Every `#[path]` here names a player source; the
//! Makefile reads them for the UI build's dependencies.

#[path = "../../../src/config/audio.rs"]
mod audio;
#[path = "../../../src/config/controller.rs"]
mod controller;
#[path = "../../../src/config/dmx.rs"]
mod dmx;
#[path = "../../../src/config/duration.rs"]
mod duration;
#[path = "../../../src/config/fade.rs"]
mod fade;
#[path = "../../../src/config/kind_data.rs"]
mod kind_data;
#[path = "../../../src/config/lighting.rs"]
mod lighting;
#[path = "../../../src/config/midi.rs"]
mod midi;
#[path = "../../../src/config/midi_event.rs"]
mod midi_event;
#[path = "../../../src/config/notification.rs"]
mod notification;
#[path = "../../../src/config/pilot.rs"]
pub mod pilot;
#[path = "../../../src/config/playlist_settings.rs"]
mod playlist_settings;
#[path = "../../../src/config/profile.rs"]
mod profile;
#[path = "../../../src/config/samples.rs"]
mod samples;
#[path = "../../../src/config/section.rs"]
mod section;
#[path = "../../../src/config/statusevents.rs"]
mod statusevents;
#[path = "../../../src/config/track.rs"]
mod track;
#[path = "../../../src/config/trackmappings.rs"]
mod trackmappings;
#[path = "../../../src/config/trigger.rs"]
mod trigger;

pub use audio::StreamBufferSize;
pub use controller::MidiController;
use duration::{parse_duration_string, parse_playback_delay};
pub use fade::{FadeCurve, FadeStatus};
pub use midi_event::MidiEventShapes;
pub use playlist_settings::{AutoPlayConfig, EntryFades, FadeConfig};
pub use section::Section;

mod kind {
    pub use super::kind_data::ConfigKind;
}

pub mod ambient {
    pub use super::playlist_settings::{Ambient, PlaylistAmbient};
}
