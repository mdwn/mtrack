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
pub mod ambient;
pub mod audio;
pub mod controller;
pub mod dmx;
mod duration;
pub mod error;
mod fade;
pub mod hostname;
mod kind_data;
pub mod length;
pub mod lighting;
pub mod metronome;
pub mod midi;
mod midi_event;
pub mod notification;
pub mod pilot;
pub mod player;
mod playlist;
mod playlist_settings;
pub mod profile;
pub mod samples;
mod section;
mod song;
mod statusevents;
mod tempo;
mod track;
mod trackmappings;
pub mod trigger;

pub use self::audio::{Audio, ResamplerType, StreamBufferSize};
pub use self::controller::BleController;
pub use self::controller::Controller;
pub use self::controller::ControllerKind;
pub use self::controller::CustomModel;
pub use self::controller::GrpcController;
pub use self::controller::McpController;
pub use self::controller::MidiController;
pub use self::controller::MorningstarConfig;
pub use self::controller::MorningstarModel;
pub use self::controller::OscController;
pub use self::controller::DEFAULT_GRPC_PORT;
pub use self::controller::DEFAULT_MCP_PORT;
pub use self::dmx::Dmx;
pub use self::dmx::Universe;
// Re-exported for the hardware harness, which needs the default port to probe
// for a running OLA daemon. Gated to match the constant itself, which (with
// `Dmx::ola_port`) is left out of this crate's own test build; the player
// and the harness link the non-test build and so see it.
#[cfg(not(test))]
pub use self::dmx::DEFAULT_OLA_PORT;
pub use self::error::ConfigError;
pub use self::lighting::Lighting;
pub use self::metronome::MetronomeConfig;
pub use self::midi::BeatClockSeams;
pub use self::midi::Midi;
pub use self::midi::ToMidiEvent;
/// What the web UI's MIDI event editor offers; built by the rules module.
#[cfg(feature = "tsify")]
pub use self::midi_event::MidiEventShapes;
pub use self::pilot::PilotConfig;
pub use self::player::{MetronomeDefaults, Player};
#[allow(unused_imports)]
pub use self::playlist::{
    AutoPlayConfig, ConfiguredEntry, EntryFades, EntryTransitions, Fade, FadeConfig, FadeCurve,
    FadeStatus, Playlist, PlaylistEntry,
};
// Sample types are exported for external configuration
pub use self::duration::{parse_playback_delay, playback_delay_problem};
pub use self::hostname::resolve_hostname;
pub use self::notification::{
    section_key, NotificationConfig, NotificationKind, SongNotificationConfig,
};
pub use self::player::{list_profile_files, profile_file};
pub use self::profile::Profile;
#[allow(unused_imports)]
pub use self::samples::{
    ReleaseBehavior, RetriggerBehavior, SampleDefinition, SampleTrigger, SamplesConfig,
    VelocityConfig, VelocityLayer, VelocityMode,
};
pub use self::section::Section;
pub use self::song::{LightShow, LightingShow, LoopCrossfade, MidiPlayback, Song};
pub use self::statusevents::StatusEvents;
pub use self::tempo::{parse_time_signature, TempoChangeConfig, TempoConfig, TransitionConfig};
pub use self::track::Track;

mod kind;
pub use self::kind::{peek_kind, ConfigKind};
