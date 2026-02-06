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
mod audio;
mod controller;
mod dmx;
mod error;
pub mod lighting;
#[cfg(test)]
pub mod midi;
#[cfg(not(test))]
mod midi;
mod player;
mod playlist;
pub mod samples;
mod song;
mod statusevents;
mod track;
mod trackmappings;

pub use self::audio::Audio;
pub use self::controller::Controller;
pub use self::controller::GrpcController;
pub use self::controller::MidiController;
pub use self::controller::OscController;
pub use self::controller::DEFAULT_GRPC_PORT;
pub use self::dmx::Dmx;
pub use self::dmx::Universe;
pub use self::error::ConfigError;
pub use self::lighting::Lighting;
pub use self::midi::Midi;
pub use self::midi::MidiTransformer;
pub use self::midi::ToMidiEvent;
pub use self::player::Player;
pub use self::playlist::Playlist;
// Sample types are exported for external configuration
#[allow(unused_imports)]
pub use self::samples::{
    NoteOffBehavior, RetriggerBehavior, SampleDefinition, SampleTrigger, SamplesConfig,
    VelocityConfig, VelocityLayer, VelocityMode,
};
pub use self::song::{LightShow, LightingShow, MidiPlayback, Song};
pub use self::statusevents::StatusEvents;
pub use self::track::Track;
