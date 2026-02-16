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

use serde::Deserialize;

use super::audio::Audio;
use super::dmx::Dmx;
use super::midi::Midi;

/// Audio configuration with track mappings.
#[derive(Deserialize, Clone)]
pub struct AudioConfig {
    #[serde(flatten)]
    audio: Audio,
    track_mappings: HashMap<String, Vec<u16>>,
}

impl AudioConfig {
    /// Creates a new AudioConfig.
    pub fn new(audio: Audio, track_mappings: HashMap<String, Vec<u16>>) -> Self {
        AudioConfig {
            audio,
            track_mappings,
        }
    }

    /// Returns the audio configuration.
    pub fn audio(&self) -> &Audio {
        &self.audio
    }

    /// Returns the track mappings.
    pub fn track_mappings(&self) -> &HashMap<String, Vec<u16>> {
        &self.track_mappings
    }
}

/// A unified hardware profile representing one complete host configuration.
/// Profiles are tried in list order; the first one whose hostname matches (or has
/// no constraint) is used.
#[derive(Deserialize, Clone)]
pub struct Profile {
    /// Optional hostname restriction.
    hostname: Option<String>,

    /// Audio configuration (optional if absent from profile).
    audio: Option<AudioConfig>,

    /// MIDI configuration (optional if absent from profile).
    midi: Option<Midi>,

    /// DMX configuration (optional if absent from profile).
    dmx: Option<Dmx>,
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
            hostname,
            audio,
            midi,
            dmx,
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

    /// Returns the MIDI configuration.
    pub fn midi(&self) -> Option<&Midi> {
        self.midi.as_ref()
    }

    /// Returns the DMX configuration.
    pub fn dmx(&self) -> Option<&Dmx> {
        self.dmx.as_ref()
    }
}

/// Filters profiles by hostname. Returns profiles that either have no hostname
/// constraint or whose hostname matches the given value.
pub fn filter_by_hostname<'a, P, F>(
    profiles: &'a [P],
    hostname: &str,
    get_hostname: F,
) -> Vec<&'a P>
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
        let track_mappings = HashMap::from([("drums".to_string(), vec![1, 2])]);
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
        let track_mappings = HashMap::from([("drums".to_string(), vec![1])]);
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
                    HashMap::from([("drums".to_string(), vec![1])]),
                )),
                None,
                None,
            ),
            Profile::new(
                Some("pi-b".to_string()),
                Some(AudioConfig::new(
                    Audio::new("device-b"),
                    HashMap::from([("drums".to_string(), vec![11])]),
                )),
                None,
                None,
            ),
            Profile::new(
                None,
                Some(AudioConfig::new(
                    Audio::new("fallback"),
                    HashMap::from([("drums".to_string(), vec![1])]),
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
