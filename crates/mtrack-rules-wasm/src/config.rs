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

#[path = "../../../src/config/audio_data.rs"]
mod audio_data;
#[path = "../../../src/config/controller_data.rs"]
mod controller_data;
#[path = "../../../src/config/dmx_data.rs"]
mod dmx_data;
#[path = "../../../src/config/fade.rs"]
mod fade;
#[path = "../../../src/config/kind_data.rs"]
mod kind_data;
#[path = "../../../src/config/lighting_data.rs"]
mod lighting_data;
#[path = "../../../src/config/midi_data.rs"]
mod midi_data;
#[path = "../../../src/config/midi_event.rs"]
mod midi_event;
#[path = "../../../src/config/notification_data.rs"]
mod notification_data;
#[path = "../../../src/config/pilot.rs"]
pub mod pilot;
#[path = "../../../src/config/playlist_settings.rs"]
mod playlist_settings;
#[path = "../../../src/config/profile_data.rs"]
mod profile_data;
#[path = "../../../src/config/samples_data.rs"]
mod samples_data;
#[path = "../../../src/config/section.rs"]
mod section;
#[path = "../../../src/config/statusevents_data.rs"]
mod statusevents_data;
#[path = "../../../src/config/track.rs"]
mod track;
#[path = "../../../src/config/trackmappings_data.rs"]
mod trackmappings_data;
#[path = "../../../src/config/trigger_data.rs"]
mod trigger_data;

pub use audio_data::StreamBufferSize;
pub use fade::{FadeCurve, FadeStatus};
pub use playlist_settings::{AutoPlayConfig, EntryFades, FadeConfig};
pub use section::Section;

pub mod ambient {
    pub use super::playlist_settings::{Ambient, PlaylistAmbient};
}
