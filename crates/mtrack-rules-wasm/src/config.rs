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
#[path = "../../../src/config/length.rs"]
pub mod length;
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

pub use crate::tempo_config::TempoConfig;
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

/// The values each of a config's choices takes: every variant of the
/// player's own enums (`VariantArray`), as `serde` writes them: what the
/// config editor's selects offer.
#[derive(serde::Serialize, tsify_next::Tsify)]
#[tsify(into_wasm_abi)]
pub struct ConfigChoices {
    pub trigger_actions: Vec<trigger::TriggerInputAction>,
    pub velocity_curves: Vec<trigger::VelocityCurve>,
    pub resamplers: Vec<audio::ResamplerType>,
    pub sample_formats: Vec<&'static str>,
    pub release_behaviors: Vec<samples::ReleaseBehavior>,
    pub retrigger_behaviors: Vec<samples::RetriggerBehavior>,
    pub velocity_modes: Vec<samples::VelocityMode>,
    pub beat_clock_seams: Vec<midi::BeatClockSeams>,
    pub fade_curves: Vec<FadeCurve>,
    pub hint_aligns: Vec<pilot::HintAlign>,
    pub strobe_curves: Vec<crate::lighting::fixture_data::StrobeCurve>,
    /// The velocity a `fixed` curve plays when the config names none.
    pub fixed_velocity: u8,
    /// The lowest and highest velocity a trigger hit plays at
    /// (`trigger::TRIGGER_VELOCITY`).
    pub trigger_velocity: [u8; 2],
}

impl Default for ConfigChoices {
    fn default() -> Self {
        use strum::VariantArray;
        ConfigChoices {
            trigger_actions: trigger::TriggerInputAction::VARIANTS.to_vec(),
            velocity_curves: trigger::VelocityCurve::VARIANTS.to_vec(),
            resamplers: audio::ResamplerType::VARIANTS.to_vec(),
            sample_formats: crate::audio::SampleFormat::VARIANTS
                .iter()
                .map(|f| f.as_str())
                .collect(),
            release_behaviors: samples::ReleaseBehavior::VARIANTS.to_vec(),
            retrigger_behaviors: samples::RetriggerBehavior::VARIANTS.to_vec(),
            velocity_modes: samples::VelocityMode::VARIANTS.to_vec(),
            beat_clock_seams: midi::BeatClockSeams::VARIANTS.to_vec(),
            fade_curves: FadeCurve::VARIANTS.to_vec(),
            hint_aligns: pilot::HintAlign::VARIANTS.to_vec(),
            strobe_curves: crate::lighting::fixture_data::StrobeCurve::VARIANTS.to_vec(),
            fixed_velocity: trigger::default_fixed_velocity(),
            trigger_velocity: [
                *trigger::TRIGGER_VELOCITY.start(),
                *trigger::TRIGGER_VELOCITY.end(),
            ],
        }
    }
}
