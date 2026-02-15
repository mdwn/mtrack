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
use super::midi::Midi;

/// An audio hardware profile. Each entry in `audio_profiles` represents one
/// possible audio device configuration with its associated track mappings.
/// Profiles are tried in list order; the first one whose device is available wins.
#[derive(Deserialize, Clone)]
pub struct AudioProfile {
    /// Optional hostname restriction. If set, this profile only applies on
    /// hosts whose hostname matches this value. If omitted, matches any host.
    hostname: Option<String>,

    /// The audio configuration (device name, sample rate, etc.).
    /// Flattened so that device, sample_rate, etc. appear at the profile level in YAML.
    #[serde(flatten)]
    audio: Audio,

    /// The track mappings specific to this audio profile.
    track_mappings: HashMap<String, Vec<u16>>,
}

impl AudioProfile {
    /// Creates a new AudioProfile.
    pub fn new(
        hostname: Option<String>,
        audio: Audio,
        track_mappings: HashMap<String, Vec<u16>>,
    ) -> Self {
        AudioProfile {
            hostname,
            audio,
            track_mappings,
        }
    }

    /// Returns the optional hostname constraint.
    pub fn hostname(&self) -> Option<&str> {
        self.hostname.as_deref()
    }

    /// Returns the audio configuration.
    pub fn audio(&self) -> &Audio {
        &self.audio
    }

    /// Returns the track mappings for this profile.
    pub fn track_mappings(&self) -> &HashMap<String, Vec<u16>> {
        &self.track_mappings
    }
}

/// A MIDI hardware profile. Each entry in `midi_profiles` represents one
/// possible MIDI device with its configuration. Profiles are tried in list order.
#[derive(Deserialize, Clone)]
pub struct MidiProfile {
    /// Optional hostname restriction.
    hostname: Option<String>,

    /// The MIDI configuration (device name, playback_delay, midi_to_dmx).
    /// Flattened so that device, playback_delay, etc. appear at the profile level in YAML.
    #[serde(flatten)]
    midi: Midi,
}

impl MidiProfile {
    /// Creates a new MidiProfile.
    pub fn new(hostname: Option<String>, midi: Midi) -> Self {
        MidiProfile { hostname, midi }
    }

    /// Returns the optional hostname constraint.
    pub fn hostname(&self) -> Option<&str> {
        self.hostname.as_deref()
    }

    /// Returns the MIDI configuration.
    pub fn midi(&self) -> &Midi {
        &self.midi
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
    fn test_audio_profile_deserialize() {
        let yaml = r#"
            hostname: pi-a
            device: mock-device
            sample_rate: 48000
            track_mappings:
              drums:
                - 1
              synth:
                - 2
        "#;

        let profile: AudioProfile = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(profile.hostname(), Some("pi-a"));
        assert_eq!(profile.audio().device(), "mock-device");
        assert_eq!(profile.audio().sample_rate(), 48000);
        assert_eq!(profile.track_mappings().get("drums"), Some(&vec![1u16]));
        assert_eq!(profile.track_mappings().get("synth"), Some(&vec![2u16]));
    }

    #[test]
    fn test_audio_profile_no_hostname() {
        let yaml = r#"
            device: mock-device
            track_mappings:
              click:
                - 1
        "#;

        let profile: AudioProfile = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(profile.hostname(), None);
        assert_eq!(profile.audio().device(), "mock-device");
    }

    #[test]
    fn test_midi_profile_deserialize() {
        let yaml = r#"
            hostname: pi-a
            device: mock-midi
            playback_delay: 500ms
        "#;

        let profile: MidiProfile = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(profile.hostname(), Some("pi-a"));
        assert_eq!(profile.midi().device(), "mock-midi");
    }

    #[test]
    fn test_midi_profile_no_hostname() {
        let yaml = r#"
            device: mock-midi
        "#;

        let profile: MidiProfile = Config::builder()
            .add_source(File::from_str(yaml, FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();

        assert_eq!(profile.hostname(), None);
        assert_eq!(profile.midi().device(), "mock-midi");
    }

    #[test]
    fn test_filter_by_hostname() {
        let profiles = vec![
            AudioProfile::new(
                Some("pi-a".to_string()),
                Audio::new("device-a"),
                HashMap::from([("drums".to_string(), vec![1])]),
            ),
            AudioProfile::new(
                Some("pi-b".to_string()),
                Audio::new("device-b"),
                HashMap::from([("drums".to_string(), vec![11])]),
            ),
            AudioProfile::new(
                None,
                Audio::new("fallback"),
                HashMap::from([("drums".to_string(), vec![1])]),
            ),
        ];

        // pi-a matches hostname-specific + wildcard
        let filtered = filter_by_hostname(&profiles, "pi-a", |p| p.hostname());
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].audio().device(), "device-a");
        assert_eq!(filtered[1].audio().device(), "fallback");

        // pi-b matches hostname-specific + wildcard
        let filtered = filter_by_hostname(&profiles, "pi-b", |p| p.hostname());
        assert_eq!(filtered.len(), 2);
        assert_eq!(filtered[0].audio().device(), "device-b");
        assert_eq!(filtered[1].audio().device(), "fallback");

        // unknown host only matches wildcard
        let filtered = filter_by_hostname(&profiles, "pi-c", |p| p.hostname());
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].audio().device(), "fallback");
    }
}
