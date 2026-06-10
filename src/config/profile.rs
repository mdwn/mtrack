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
use std::collections::HashMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use super::audio::Audio;
use super::controller::Controller;
use super::dmx::Dmx;
use super::midi::Midi;
use super::notification::NotificationConfig;
use super::trigger::TriggerConfig;

/// Audio configuration with track mappings.
///
/// Uses `IndexMap` so insertion order is preserved through serialize/deserialize
/// round-trips — important for the web UI config editor.
#[derive(Deserialize, Serialize, Clone)]
pub struct AudioConfig {
    #[serde(flatten)]
    audio: Audio,
    track_mappings: IndexMap<String, Vec<u16>>,
    /// Per-output-track gain in dB. Tracks without an entry play at unity.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    track_gains: IndexMap<String, f32>,
}

impl AudioConfig {
    /// Creates a new AudioConfig.
    pub fn new(audio: Audio, track_mappings: IndexMap<String, Vec<u16>>) -> Self {
        AudioConfig {
            audio,
            track_mappings,
            track_gains: IndexMap::new(),
        }
    }

    /// Returns the per-track gains in dB.
    pub fn track_gains(&self) -> &IndexMap<String, f32> {
        &self.track_gains
    }

    /// Replaces the per-track gains.
    pub fn set_track_gains(&mut self, track_gains: IndexMap<String, f32>) {
        self.track_gains = track_gains;
    }

    /// Returns the audio configuration.
    pub fn audio(&self) -> &Audio {
        &self.audio
    }

    /// Returns the track mappings as an IndexMap (preserves insertion order).
    #[cfg(test)]
    pub fn track_mappings(&self) -> &IndexMap<String, Vec<u16>> {
        &self.track_mappings
    }

    /// Validates the audio configuration within a profile.
    pub fn validate(&self, errors: &mut Vec<String>) {
        if let Err(audio_errors) = self.audio.validate() {
            errors.extend(audio_errors);
        }
        for (name, channels) in &self.track_mappings {
            for &ch in channels {
                if ch == 0 {
                    errors.push(format!(
                        "track_mappings '{}': channel 0 is invalid (channels are 1-indexed)",
                        name
                    ));
                }
            }
        }
        for (name, &db) in &self.track_gains {
            if !db.is_finite() {
                errors.push(format!(
                    "track_gains '{}': gain must be a finite number",
                    name
                ));
            } else if !(crate::audio::track_gains::MIN_GAIN_DB
                ..=crate::audio::track_gains::MAX_GAIN_DB)
                .contains(&db)
            {
                errors.push(format!(
                    "track_gains '{}': gain {} dB out of range [{}, {}]",
                    name,
                    db,
                    crate::audio::track_gains::MIN_GAIN_DB,
                    crate::audio::track_gains::MAX_GAIN_DB
                ));
            }
        }
    }

    /// Returns the track mappings as a HashMap (for runtime use where order doesn't matter).
    pub fn track_mappings_hash(&self) -> HashMap<String, Vec<u16>> {
        self.track_mappings
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }
}

/// A unified hardware profile representing one complete host configuration.
/// Profiles are tried in list order; the first one whose hostname matches (or has
/// no constraint) is used.
#[derive(Deserialize, Serialize, Clone)]
pub struct Profile {
    /// Identifies this config file as a hardware profile.
    #[serde(default = "default_hardware_profile_kind")]
    kind: super::kind::ConfigKind,

    /// Optional hostname restriction.
    hostname: Option<String>,

    /// Audio configuration (optional if absent from profile).
    audio: Option<AudioConfig>,

    /// MIDI configuration (optional if absent from profile).
    midi: Option<Midi>,

    /// DMX configuration (optional if absent from profile).
    dmx: Option<Dmx>,

    /// Audio trigger configuration (optional).
    trigger: Option<TriggerConfig>,

    /// Controllers associated with this profile.
    #[serde(default)]
    controllers: Vec<Controller>,

    /// Notification audio configuration (global overrides).
    #[serde(default)]
    notifications: Option<NotificationConfig>,

    /// Status events — MIDI events emitted on player state changes.
    #[serde(default)]
    status_events: Option<super::statusevents::StatusEvents>,
}

impl Profile {
    /// Creates a new Profile.
    pub fn new(
        hostname: Option<String>,
        audio: Option<AudioConfig>,
        midi: Option<Midi>,
        dmx: Option<Dmx>,
    ) -> Self {
        Profile {
            kind: super::kind::ConfigKind::HardwareProfile,
            hostname,
            audio,
            midi,
            dmx,
            trigger: None,
            controllers: Vec::new(),
            notifications: None,
            status_events: None,
        }
    }

    /// Returns the optional hostname constraint.
    pub fn hostname(&self) -> Option<&str> {
        self.hostname.as_deref()
    }

    /// Returns the audio configuration, if present.
    pub fn audio_config(&self) -> Option<&AudioConfig> {
        self.audio.as_ref()
    }

    /// Returns a mutable reference to the audio configuration, if present.
    pub fn audio_config_mut(&mut self) -> Option<&mut AudioConfig> {
        self.audio.as_mut()
    }

    /// Returns the MIDI configuration.
    pub fn midi(&self) -> Option<&Midi> {
        self.midi.as_ref()
    }

    /// Returns the DMX configuration.
    pub fn dmx(&self) -> Option<&Dmx> {
        self.dmx.as_ref()
    }

    /// Returns a mutable reference to the DMX configuration.
    pub fn dmx_mut(&mut self) -> Option<&mut Dmx> {
        self.dmx.as_mut()
    }

    /// Returns the trigger configuration, if present.
    pub fn trigger(&self) -> Option<&TriggerConfig> {
        self.trigger.as_ref()
    }

    /// Sets the trigger configuration (used during legacy config normalization).
    pub(super) fn set_trigger(&mut self, trigger: Option<TriggerConfig>) {
        self.trigger = trigger;
    }

    /// Returns the controllers associated with this profile.
    pub fn controllers(&self) -> &[Controller] {
        &self.controllers
    }

    /// Sets the controllers (used during legacy config normalization).
    pub(super) fn set_controllers(&mut self, controllers: Vec<Controller>) {
        self.controllers = controllers;
    }

    /// Returns the notification audio configuration, if present.
    pub fn notifications(&self) -> Option<&NotificationConfig> {
        self.notifications.as_ref()
    }

    /// Returns the status events configuration, if present.
    pub fn status_events(&self) -> Option<&super::statusevents::StatusEvents> {
        self.status_events.as_ref()
    }

    /// Sets the status events (used during legacy config normalization).
    pub(super) fn set_status_events(
        &mut self,
        status_events: Option<super::statusevents::StatusEvents>,
    ) {
        self.status_events = status_events;
    }

    /// Validates the profile configuration for semantic issues.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if let Some(ref audio_config) = self.audio {
            audio_config.validate(&mut errors);
        }
        if let Some(ref midi) = self.midi {
            if let Err(midi_errors) = midi.validate() {
                errors.extend(midi_errors);
            }
        }
        if let Some(ref dmx) = self.dmx {
            if let Err(dmx_errors) = dmx.validate() {
                errors.extend(dmx_errors);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

fn default_hardware_profile_kind() -> super::kind::ConfigKind {
    super::kind::ConfigKind::HardwareProfile
}

/// Filters profiles by hostname. Returns profiles that either have no hostname
/// constraint or whose hostname matches the given value.
#[cfg(test)]
fn filter_by_hostname<'a, P, F>(profiles: &'a [P], hostname: &str, get_hostname: F) -> Vec<&'a P>
where
    F: Fn(&'a P) -> Option<&'a str>,
{
    profiles
        .iter()
        .filter(|p| match get_hostname(p) {
            Some(h) => h == hostname,
            None => true,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use config::{Config, File, FileFormat};

    use super::*;

    #[test]
    fn test_track_gains_round_trip() {
        let yaml = r#"
            audio:
              device: mock-device
              track_mappings:
                click: [1]
                keys: [2, 3]
              track_gains:
                click: -6.0
                keys: 2.5
        "#;
        let profile: Profile = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        let audio_config = profile.audio_config().unwrap();
        assert_eq!(audio_config.track_gains()["click"], -6.0);
        assert_eq!(audio_config.track_gains()["keys"], 2.5);

        // Round-trips through serialization.
        let serialized = crate::util::to_yaml_string(&profile).unwrap();
        assert!(serialized.contains("track_gains"));
        let reparsed: Profile = Config::builder()
            .add_source(File::from_str(&serialized, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(
            reparsed.audio_config().unwrap().track_gains()["click"],
            -6.0
        );
    }

    #[test]
    fn test_track_gains_absent_and_omitted() {
        let yaml = r#"
            audio:
              device: mock-device
              track_mappings:
                click: [1]
        "#;
        let profile: Profile = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert!(profile.audio_config().unwrap().track_gains().is_empty());
        // Empty map is omitted on serialize to keep configs clean.
        let serialized = crate::util::to_yaml_string(&profile).unwrap();
        assert!(!serialized.contains("track_gains"));
    }

    #[test]
    fn test_track_gains_validation() {
        let mut track_mappings = IndexMap::new();
        track_mappings.insert("click".to_string(), vec![1u16]);
        let audio = Audio::new("mock-device");
        let mut audio_config = AudioConfig::new(audio, track_mappings);

        audio_config.set_track_gains(IndexMap::from([("click".to_string(), -120.0f32)]));
        let mut errors = Vec::new();
        audio_config.validate(&mut errors);
        assert!(
            errors.iter().any(|e| e.contains("track_gains 'click'")),
            "expected out-of-range error, got {:?}",
            errors
        );

        audio_config.set_track_gains(IndexMap::from([("click".to_string(), f32::NAN)]));
        let mut errors = Vec::new();
        audio_config.validate(&mut errors);
        assert!(
            errors.iter().any(|e| e.contains("finite")),
            "expected non-finite error, got {:?}",
            errors
        );

        audio_config.set_track_gains(IndexMap::from([("click".to_string(), -6.0f32)]));
        let mut errors = Vec::new();
        audio_config.validate(&mut errors);
        assert!(errors.is_empty(), "expected no errors, got {:?}", errors);
    }

    #[test]
    fn test_profile_deserialize() {
        let yaml = r#"
            hostname: pi-a
            audio:
              device: mock-device
              sample_rate: 48000
              track_mappings:
                drums:
                  - 1
                synth:
                  - 2
            midi:
              device: mock-midi
            dmx:
              universes:
                - universe: 1
                  name: light-show
            controllers:
              - kind: grpc
                port: 43234
              - kind: osc
        "#;

        let profile: Profile = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(profile.hostname(), Some("pi-a"));
        let audio_config = profile.audio_config().unwrap();
        assert_eq!(audio_config.audio().device(), "mock-device");
        assert_eq!(audio_config.audio().sample_rate(), 48000);
        assert_eq!(
            audio_config.track_mappings().get("drums"),
            Some(&vec![1u16])
        );
        assert_eq!(
            audio_config.track_mappings().get("synth"),
            Some(&vec![2u16])
        );
        assert!(profile.midi().is_some());
        assert_eq!(profile.midi().unwrap().device(), "mock-midi");
        assert!(profile.dmx().is_some());
        assert_eq!(profile.controllers().len(), 2);
    }

    #[test]
    fn test_profile_without_audio() {
        let yaml = r#"
            hostname: lighting-node
            midi:
              device: mock-midi
            dmx:
              universes:
                - universe: 1
                  name: light-show
        "#;

        let profile: Profile = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(profile.hostname(), Some("lighting-node"));
        assert!(profile.audio_config().is_none());
        assert!(profile.midi().is_some());
        assert!(profile.dmx().is_some());
    }

    #[test]
    fn test_profile_with_all_subsystems() {
        let audio = Audio::new("test-device");
        let track_mappings = IndexMap::from([("drums".to_string(), vec![1, 2])]);
        let audio_config = AudioConfig::new(audio, track_mappings);
        let midi = Some(Midi::new("midi-device", None));
        let dmx = Some(Dmx::new(None, None, None, vec![], None));

        let profile = Profile::new(Some("pi-a".to_string()), Some(audio_config), midi, dmx);

        assert_eq!(profile.hostname(), Some("pi-a"));
        assert!(profile.audio_config().is_some());
        assert!(profile.midi().is_some());
        assert!(profile.dmx().is_some());
    }

    #[test]
    fn test_profile_without_midi_dmx() {
        let audio = Audio::new("test-device");
        let track_mappings = IndexMap::from([("drums".to_string(), vec![1])]);
        let audio_config = AudioConfig::new(audio, track_mappings);

        let profile = Profile::new(None, Some(audio_config), None, None);

        assert_eq!(profile.hostname(), None);
        assert!(profile.audio_config().is_some());
        assert!(profile.midi().is_none());
        assert!(profile.dmx().is_none());
    }

    #[test]
    fn test_filter_by_hostname() {
        let profiles = vec![
            Profile::new(
                Some("pi-a".to_string()),
                Some(AudioConfig::new(
                    Audio::new("device-a"),
                    IndexMap::from([("drums".to_string(), vec![1])]),
                )),
                None,
                None,
            ),
            Profile::new(
                Some("pi-b".to_string()),
                Some(AudioConfig::new(
                    Audio::new("device-b"),
                    IndexMap::from([("drums".to_string(), vec![11])]),
                )),
                None,
                None,
            ),
            Profile::new(
                None,
                Some(AudioConfig::new(
                    Audio::new("fallback"),
                    IndexMap::from([("drums".to_string(), vec![1])]),
                )),
                None,
                None,
            ),
        ];

        // pi-a matches hostname-specific + wildcard
        let filtered = filter_by_hostname(&profiles, "pi-a", |p| p.hostname());
        assert_eq!(filtered.len(), 2);
        assert_eq!(
            filtered[0].audio_config().unwrap().audio().device(),
            "device-a"
        );
        assert_eq!(
            filtered[1].audio_config().unwrap().audio().device(),
            "fallback"
        );

        // pi-b matches hostname-specific + wildcard
        let filtered = filter_by_hostname(&profiles, "pi-b", |p| p.hostname());
        assert_eq!(filtered.len(), 2);
        assert_eq!(
            filtered[0].audio_config().unwrap().audio().device(),
            "device-b"
        );
        assert_eq!(
            filtered[1].audio_config().unwrap().audio().device(),
            "fallback"
        );

        // unknown host only matches wildcard
        let filtered = filter_by_hostname(&profiles, "pi-c", |p| p.hostname());
        assert_eq!(filtered.len(), 1);
        assert_eq!(
            filtered[0].audio_config().unwrap().audio().device(),
            "fallback"
        );
    }
}
