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
mod audio;
mod controller;
mod dmx;
#[cfg(test)]
pub mod midi;
#[cfg(not(test))]
mod midi;
mod player;
mod playlist;
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
pub use self::midi::Midi;
pub use self::player::Player;
pub use self::playlist::Playlist;
pub use self::song::LightShow;
pub use self::song::MidiPlayback;
pub use self::song::Song;
pub use self::statusevents::StatusEvents;
pub use self::track::Track;
