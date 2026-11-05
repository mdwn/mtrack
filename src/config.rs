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
mod audio;
mod controller;
mod dmx;
mod error;
mod fade;
mod hostname;
pub mod lighting;
pub mod metronome;
#[cfg(test)]
pub mod midi;
#[cfg(not(test))]
mod midi;
mod midi_event;
pub mod notification;
pub mod pilot;
mod player;
mod playlist;
mod playlist_settings;
mod profile;
pub mod samples;
mod section;
mod song;
mod statusevents;
pub mod store;
mod tempo;
mod track;
mod trackmappings;
pub mod trigger;

pub use self::audio::{Audio, ResamplerType, StreamBufferSize};
pub use self::controller::Controller;
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
// `Dmx::ola_port`) is excluded from test builds; the harness links the
// non-test build and so still sees it.
#[cfg(not(test))]
pub use self::dmx::DEFAULT_OLA_PORT;
pub use self::error::ConfigError;
pub use self::lighting::Lighting;
pub use self::metronome::MetronomeConfig;
pub use self::midi::BeatClockSeams;
pub use self::midi::Midi;
pub use self::midi::ToMidiEvent;
pub use self::pilot::PilotConfig;
pub use self::player::{MetronomeDefaults, Player};
#[allow(unused_imports)]
pub use self::playlist::{
    AutoPlayConfig, ConfiguredEntry, EntryFades, EntryTransitions, Fade, FadeConfig, FadeCurve,
    FadeStatus, Playlist, PlaylistEntry,
};
// Sample types are exported for external configuration
pub use self::hostname::resolve_hostname;
pub use self::notification::{NotificationConfig, SongNotificationConfig};
pub(crate) use self::player::list_profile_files;
pub use self::profile::Profile;
#[allow(unused_imports)]
pub use self::samples::{
    ReleaseBehavior, RetriggerBehavior, SampleDefinition, SampleTrigger, SamplesConfig,
    VelocityConfig, VelocityLayer, VelocityMode,
};
pub use self::section::Section;
pub use self::song::{LightShow, LightingShow, LoopCrossfade, MidiPlayback, Song};
pub use self::statusevents::StatusEvents;
pub use self::store::ConfigStore;
pub use self::tempo::{parse_time_signature, TempoChangeConfig, TempoConfig, TransitionConfig};
pub use self::track::Track;

mod kind;
pub use self::kind::{peek_kind, ConfigKind};

use std::time::Duration;

/// A duration as a config string (`500ms`, `2s`): the one parse of one,
/// for every field that takes one. The error is the parser's; a validator
/// words its own.
pub(crate) fn parse_duration_string(raw: &str) -> Result<Duration, duration_string::Error> {
    duration_string::DurationString::from_string(raw.to_string()).map(Duration::from)
}

/// Parses an optional duration string (e.g. "500ms") into a `Duration`,
/// returning `default` when `raw` is `None`.
pub(crate) fn parse_playback_delay(
    raw: &Option<String>,
    default: Duration,
) -> Result<Duration, Box<dyn std::error::Error>> {
    match raw {
        Some(s) => Ok(parse_duration_string(s)?),
        None => Ok(default),
    }
}

#[cfg(test)]
mod duration_string_test {
    use super::parse_duration_string;
    use std::time::Duration;

    #[test]
    fn every_config_duration_parses_by_one_rule() {
        assert_eq!(
            parse_duration_string("500ms"),
            Ok(Duration::from_millis(500))
        );
        assert_eq!(parse_duration_string("2s"), Ok(Duration::from_secs(2)));
        assert!(parse_duration_string("soon").is_err());
        assert!(parse_duration_string("").is_err());
    }
}
