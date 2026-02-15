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
use super::audio::Audio;
use super::controller::Controller;
use super::dmx::Dmx;
use super::midi::Midi;
use super::profile::{filter_by_hostname, AudioProfile, MidiProfile};
use super::samples::{SampleDefinition, SampleTrigger, SamplesConfig, DEFAULT_MAX_SAMPLE_VOICES};
use super::statusevents::StatusEvents;
use super::trackmappings::TrackMappings;
use config::{Config, File};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use super::error::ConfigError;
use std::error::Error;
use tracing::{error, info};

/// Empty track mappings used as a fallback reference.
static EMPTY_TRACK_MAPPINGS: LazyLock<HashMap<String, Vec<u16>>> = LazyLock::new(HashMap::new);

fn default_midi_optional() -> bool {
    true
}

/// The configuration for the multitrack player.
#[derive(Deserialize)]
pub struct Player {
    /// The controller configuration.
    controller: Option<Controller>,
    /// The controllers configuration.
    controllers: Option<Vec<Controller>>,
    /// The audio device to use. (legacy)
    audio_device: Option<String>,
    /// The audio configuration section. (legacy)
    audio: Option<Audio>,
    /// The track mappings for the player. (legacy, now optional when using profiles)
    #[serde(default)]
    track_mappings: Option<TrackMappings>,
    /// The MIDI device to use. (legacy)
    midi_device: Option<String>,
    /// The MIDI configuration section. (legacy)
    midi: Option<Midi>,
    /// Audio hardware profiles, tried in priority order.
    audio_profiles: Option<Vec<AudioProfile>>,
    /// MIDI hardware profiles, tried in priority order.
    midi_profiles: Option<Vec<MidiProfile>>,
    /// When true, the player proceeds without MIDI if no device is available
    /// instead of retrying forever. Defaults to true (MIDI is optional).
    #[serde(default = "default_midi_optional")]
    midi_optional: bool,
    /// The DMX configuration.
    dmx: Option<Dmx>,
    /// Events to emit to report status out via MIDI.
    status_events: Option<StatusEvents>,
    /// The path to the playlist.
    playlist: Option<String>,
    /// The path to the song definitions.
    songs: String,
    /// Inline sample definitions.
    #[serde(default)]
    samples: HashMap<String, SampleDefinition>,
    /// Path to external samples configuration file.
    samples_file: Option<String>,
    /// Sample trigger mappings.
    #[serde(default)]
    sample_triggers: Vec<SampleTrigger>,
    /// Maximum number of concurrent sample voices globally.
    max_sample_voices: Option<u32>,
}

impl Player {
    #[cfg(test)]
    pub fn new(
        controllers: Vec<Controller>,
        audio: Audio,
        midi: Option<Midi>,
        dmx: Option<Dmx>,
        track_mappings: HashMap<String, Vec<u16>>,
        songs: &str,
    ) -> Player {
        let mut player = Player {
            controller: None,
            controllers: Some(controllers),
            audio_device: None,
            audio: Some(audio),
            track_mappings: Some(TrackMappings { track_mappings }),
            midi_device: None,
            midi,
            dmx,
            audio_profiles: None,
            midi_profiles: None,
            midi_optional: true,
            status_events: None,
            playlist: None,
            songs: songs.to_string(),
            samples: HashMap::new(),
            samples_file: None,
            sample_triggers: Vec::new(),
            max_sample_voices: None,
        };
        player.normalize();
        player
    }

    /// Deserializes a file from the path into a player configuration struct.
    /// Legacy configs (audio + track_mappings at top level) are normalized into profiles.
    pub fn deserialize(path: &Path) -> Result<Player, ConfigError> {
        let mut player = Config::builder()
            .add_source(File::from(path))
            .build()?
            .try_deserialize::<Player>()?;
        player.normalize();
        Ok(player)
    }

    /// Normalizes legacy configuration fields into profiles.
    /// After normalization, `audio_profiles` and `midi_profiles` are the source of truth.
    fn normalize(&mut self) {
        // Build audio profiles from legacy fields if no profiles are defined.
        if self.audio_profiles.is_none() {
            let audio = if let Some(audio) = &self.audio {
                Some(audio.clone())
            } else {
                self.audio_device.as_ref().map(|d| Audio::new(d))
            };

            if let Some(audio) = audio {
                let track_mappings = self
                    .track_mappings
                    .as_ref()
                    .map(|tm| tm.track_mappings.clone())
                    .unwrap_or_default();
                self.audio_profiles = Some(vec![AudioProfile::new(None, audio, track_mappings)]);
            }
        }

        // Build MIDI profiles from legacy fields if no profiles are defined.
        if self.midi_profiles.is_none() {
            let midi = if let Some(midi) = &self.midi {
                Some(midi.clone())
            } else {
                self.midi_device.as_ref().map(|d| Midi::new(d, None))
            };

            if let Some(midi) = midi {
                self.midi_profiles = Some(vec![MidiProfile::new(None, midi)]);
            }
        }
    }

    /// Gets the controllers configuration.
    pub fn controllers(&self) -> Vec<Controller> {
        if let Some(controllers) = &self.controllers {
            return controllers.clone();
        } else if let Some(controller) = &self.controller {
            if let Controller::Multi(multi) = controller {
                return multi.values().cloned().collect();
            }

            return vec![controller.clone()];
        }

        vec![]
    }

    /// Returns audio profiles filtered by hostname and ordered by priority.
    /// The first matching profile whose device is available should be used.
    pub fn audio_configs(&self, hostname: &str) -> Vec<&AudioProfile> {
        match &self.audio_profiles {
            Some(profiles) => filter_by_hostname(profiles, hostname, |p| p.hostname()),
            None => vec![],
        }
    }

    /// Returns all audio profiles without hostname filtering (for verify).
    pub fn all_audio_profiles(&self) -> &[AudioProfile] {
        match &self.audio_profiles {
            Some(profiles) => profiles.as_slice(),
            None => &[],
        }
    }

    /// Returns MIDI profiles filtered by hostname and ordered by priority.
    pub fn midi_configs(&self, hostname: &str) -> Vec<&MidiProfile> {
        match &self.midi_profiles {
            Some(profiles) => filter_by_hostname(profiles, hostname, |p| p.hostname()),
            None => vec![],
        }
    }

    /// Returns whether MIDI is optional. When true and all MIDI profiles fail,
    /// the player proceeds without MIDI instead of retrying.
    pub fn midi_optional(&self) -> bool {
        self.midi_optional
    }

    /// Gets the audio configuration from the first audio profile.
    /// Kept for backward compatibility in tests.
    #[cfg(test)]
    pub fn audio(&self) -> Option<Audio> {
        if let Some(profiles) = &self.audio_profiles {
            if let Some(first) = profiles.first() {
                return Some(first.audio().clone());
            }
        }

        None
    }

    /// Gets the track mapping configuration from the first audio profile.
    /// Kept for backward compatibility.
    pub fn track_mappings(&self) -> &HashMap<String, Vec<u16>> {
        if let Some(profiles) = &self.audio_profiles {
            if let Some(first) = profiles.first() {
                return first.track_mappings();
            }
        }

        &EMPTY_TRACK_MAPPINGS
    }

    /// Gets the MIDI configuration from the first MIDI profile.
    /// Kept for backward compatibility in tests.
    #[cfg(test)]
    pub fn midi(&self) -> Option<Midi> {
        if let Some(profiles) = &self.midi_profiles {
            if let Some(first) = profiles.first() {
                return Some(first.midi().clone());
            }
        }

        None
    }

    /// Gets the DMX configuration.
    pub fn dmx(&self) -> Option<&Dmx> {
        self.dmx.as_ref()
    }

    /// Gets the DMX configuration, filtered by hostname. Returns `None` if the DMX
    /// config has an `enabled_hostnames` list that doesn't include the given hostname.
    pub fn dmx_for_hostname(&self, hostname: &str) -> Option<&Dmx> {
        let dmx = self.dmx.as_ref()?;
        if let Some(enabled) = dmx.enabled_hostnames() {
            if !enabled.iter().any(|h| h == hostname) {
                return None;
            }
        }
        Some(dmx)
    }

    /// Gets the status events configuration.
    pub fn status_events(&self) -> Option<StatusEvents> {
        self.status_events.clone()
    }

    /// Gets the path to the playlist.
    pub fn playlist(&self) -> Option<PathBuf> {
        self.playlist.as_ref().map(PathBuf::from)
    }

    /// Gets the path to the song definitions.
    pub fn songs(&self, player_path: &Path) -> PathBuf {
        let songs_path_config = PathBuf::from(&self.songs);
        if songs_path_config.is_absolute() {
            return songs_path_config;
        }
        let player_path_directory = match player_path.parent() {
            Some(path) => path,
            None => {
                error!("Could not find parent of player path {player_path:?}");
                return songs_path_config;
            }
        };
        player_path_directory.join(&self.songs)
    }

    /// Gets the samples configuration, merging inline definitions with any external file.
    /// The player_path is used to resolve relative paths.
    pub fn samples_config(&self, player_path: &Path) -> Result<SamplesConfig, Box<dyn Error>> {
        let mut config = SamplesConfig::new(
            self.samples.clone(),
            self.sample_triggers.clone(),
            self.max_sample_voices.unwrap_or(DEFAULT_MAX_SAMPLE_VOICES),
        );

        // Load external samples file if specified
        if let Some(samples_file) = &self.samples_file {
            let samples_path = if Path::new(samples_file).is_absolute() {
                PathBuf::from(samples_file)
            } else {
                let player_dir = player_path.parent().unwrap_or(Path::new("."));
                player_dir.join(samples_file)
            };

            info!(path = ?samples_path, "Loading external samples configuration");

            let external_config: SamplesConfig = Config::builder()
                .add_source(File::from(samples_path.as_path()))
                .build()?
                .try_deserialize()?;

            // External config is loaded first, then inline config overrides it
            let mut merged = external_config;
            merged.merge(config);
            config = merged;
        }

        Ok(config)
    }

    /// Gets the maximum sample voices limit.
    pub fn max_sample_voices(&self) -> u32 {
        self.max_sample_voices.unwrap_or(DEFAULT_MAX_SAMPLE_VOICES)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::path::Path;

    use super::*;

    /// Helper to create a Player from a YAML string via a temp file.
    fn player_from_yaml(yaml: &str) -> Player {
        let mut temp = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        temp.write_all(yaml.as_bytes()).unwrap();
        Player::deserialize(temp.path()).expect("Failed to deserialize")
    }

    #[test]
    fn test_legacy_config_normalizes_into_profiles() {
        let player = player_from_yaml(
            r#"
songs: songs
audio:
  device: mock-device
  sample_rate: 48000
track_mappings:
  click: [1]
  cue: [2]
midi:
  device: mock-midi
  playback_delay: 500ms
"#,
        );

        // audio_profiles should have been created from legacy fields.
        let profiles = player.all_audio_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].audio().device(), "mock-device");
        assert_eq!(profiles[0].audio().sample_rate(), 48000);
        assert_eq!(profiles[0].track_mappings().get("click"), Some(&vec![1u16]));
        assert_eq!(profiles[0].track_mappings().get("cue"), Some(&vec![2u16]));
        assert!(profiles[0].hostname().is_none());

        // Backward compat getters should still work.
        assert_eq!(player.audio().unwrap().device(), "mock-device");
        assert_eq!(player.track_mappings().get("click"), Some(&vec![1u16]));
        assert_eq!(player.midi().unwrap().device(), "mock-midi");
    }

    #[test]
    fn test_legacy_audio_device_string_normalizes() {
        let player = player_from_yaml(
            r#"
songs: songs
audio_device: mock-device
track_mappings:
  drums: [1]
"#,
        );

        let profiles = player.all_audio_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].audio().device(), "mock-device");
        assert_eq!(profiles[0].track_mappings().get("drums"), Some(&vec![1u16]));
    }

    #[test]
    fn test_legacy_midi_device_string_normalizes() {
        let player = player_from_yaml(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
midi_device: mock-midi
"#,
        );

        let midi = player.midi();
        assert!(midi.is_some());
        assert_eq!(midi.unwrap().device(), "mock-midi");
    }

    #[test]
    fn test_audio_profiles_parse() {
        let player = player_from_yaml(
            r#"
songs: songs
audio_profiles:
  - hostname: pi-a
    device: mock-device-a
    sample_rate: 48000
    track_mappings:
      drums: [1]
      synth: [2]
  - hostname: pi-b
    device: mock-device-b
    track_mappings:
      drums: [11]
      synth: [12]
  - device: mock-fallback
    track_mappings:
      drums: [1]
"#,
        );

        let profiles = player.all_audio_profiles();
        assert_eq!(profiles.len(), 3);

        assert_eq!(profiles[0].hostname(), Some("pi-a"));
        assert_eq!(profiles[0].audio().device(), "mock-device-a");
        assert_eq!(profiles[0].audio().sample_rate(), 48000);

        assert_eq!(profiles[1].hostname(), Some("pi-b"));
        assert_eq!(profiles[1].audio().device(), "mock-device-b");

        assert_eq!(profiles[2].hostname(), None);
        assert_eq!(profiles[2].audio().device(), "mock-fallback");
    }

    #[test]
    fn test_audio_configs_hostname_filter() {
        let player = player_from_yaml(
            r#"
songs: songs
audio_profiles:
  - hostname: pi-a
    device: mock-device-a
    track_mappings:
      drums: [1]
  - hostname: pi-b
    device: mock-device-b
    track_mappings:
      drums: [11]
  - device: mock-fallback
    track_mappings:
      drums: [1]
"#,
        );

        // pi-a sees its own profile + the wildcard.
        let pi_a = player.audio_configs("pi-a");
        assert_eq!(pi_a.len(), 2);
        assert_eq!(pi_a[0].audio().device(), "mock-device-a");
        assert_eq!(pi_a[1].audio().device(), "mock-fallback");

        // pi-b sees its own profile + the wildcard.
        let pi_b = player.audio_configs("pi-b");
        assert_eq!(pi_b.len(), 2);
        assert_eq!(pi_b[0].audio().device(), "mock-device-b");
        assert_eq!(pi_b[1].audio().device(), "mock-fallback");

        // Unknown host only sees the wildcard.
        let unknown = player.audio_configs("pi-c");
        assert_eq!(unknown.len(), 1);
        assert_eq!(unknown[0].audio().device(), "mock-fallback");
    }

    #[test]
    fn test_midi_profiles_parse_and_filter() {
        let player = player_from_yaml(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
midi_profiles:
  - hostname: pi-a
    device: mock-midi-a
    playback_delay: 500ms
  - device: mock-midi-fallback
"#,
        );

        let pi_a = player.midi_configs("pi-a");
        assert_eq!(pi_a.len(), 2);
        assert_eq!(pi_a[0].midi().device(), "mock-midi-a");
        assert_eq!(pi_a[1].midi().device(), "mock-midi-fallback");

        let unknown = player.midi_configs("pi-c");
        assert_eq!(unknown.len(), 1);
        assert_eq!(unknown[0].midi().device(), "mock-midi-fallback");
    }

    #[test]
    fn test_profiles_take_precedence_over_legacy() {
        let player = player_from_yaml(
            r#"
songs: songs
audio:
  device: legacy-device
track_mappings:
  click: [99]
audio_profiles:
  - device: profile-device
    track_mappings:
      click: [1]
"#,
        );

        // Profiles should be used, not legacy.
        let profiles = player.all_audio_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].audio().device(), "profile-device");
        assert_eq!(profiles[0].track_mappings().get("click"), Some(&vec![1u16]));

        // Backward compat getters return from the first profile.
        assert_eq!(player.audio().unwrap().device(), "profile-device");
        assert_eq!(player.track_mappings().get("click"), Some(&vec![1u16]));
    }

    #[test]
    fn test_no_audio_profiles_when_no_audio_config() {
        let player = player_from_yaml(
            r#"
songs: songs
"#,
        );

        // No audio at all.
        assert!(player.all_audio_profiles().is_empty());
        assert!(player.audio().is_none());
        assert!(player.track_mappings().is_empty());
    }

    #[test]
    fn test_profiles_without_top_level_track_mappings() {
        let player = player_from_yaml(
            r#"
songs: songs
audio_profiles:
  - device: mock-device
    track_mappings:
      drums: [1]
      synth: [2]
"#,
        );

        // Should work without top-level track_mappings.
        let profiles = player.all_audio_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].track_mappings().get("drums"), Some(&vec![1u16]));

        // Backward compat getter returns from profile.
        assert_eq!(player.track_mappings().get("drums"), Some(&vec![1u16]));
    }

    #[test]
    fn test_hostname_deconfliction() {
        let player = player_from_yaml(
            r#"
songs: songs
audio_profiles:
  - hostname: pi-a
    device: "Behringer WING"
    track_mappings:
      drums: [1]
      synth: [2]
  - hostname: pi-b
    device: "Behringer WING"
    track_mappings:
      drums: [11]
      synth: [12]
"#,
        );

        let pi_a = player.audio_configs("pi-a");
        assert_eq!(pi_a.len(), 1);
        assert_eq!(pi_a[0].track_mappings().get("drums"), Some(&vec![1u16]));

        let pi_b = player.audio_configs("pi-b");
        assert_eq!(pi_b.len(), 1);
        assert_eq!(pi_b[0].track_mappings().get("drums"), Some(&vec![11u16]));

        // Different device name, same mappings — ensures isolation.
        let pi_c = player.audio_configs("pi-c");
        assert!(pi_c.is_empty());
    }

    #[test]
    fn test_midi_optional_defaults_true() {
        let player = player_from_yaml(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
midi:
  device: mock-midi
"#,
        );

        assert!(player.midi_optional());
    }

    #[test]
    fn test_midi_optional_explicit_false() {
        let player = player_from_yaml(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
midi:
  device: mock-midi
midi_optional: false
"#,
        );

        assert!(!player.midi_optional());
    }

    #[test]
    fn test_midi_optional_explicit_true() {
        let player = player_from_yaml(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
midi_profiles:
  - device: mock-midi-a
  - device: mock-midi-b
midi_optional: true
"#,
        );

        assert!(player.midi_optional());
    }

    #[test]
    fn test_dmx_for_hostname_no_filter() {
        let player = player_from_yaml(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
dmx:
  universes:
  - universe: 1
    name: light-show
"#,
        );

        // No enabled_hostnames — DMX enabled for any host.
        assert!(player.dmx_for_hostname("any-host").is_some());
    }

    #[test]
    fn test_dmx_for_hostname_matches() {
        let player = player_from_yaml(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
dmx:
  enabled_hostnames:
    - pi-a
    - pi-b
  universes:
  - universe: 1
    name: light-show
"#,
        );

        assert!(player.dmx_for_hostname("pi-a").is_some());
        assert!(player.dmx_for_hostname("pi-b").is_some());
    }

    #[test]
    fn test_dmx_for_hostname_no_match() {
        let player = player_from_yaml(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
dmx:
  enabled_hostnames:
    - pi-a
  universes:
  - universe: 1
    name: light-show
"#,
        );

        assert!(player.dmx_for_hostname("pi-c").is_none());
    }

    #[test]
    fn test_dmx_for_hostname_no_dmx_config() {
        let player = player_from_yaml(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
"#,
        );

        assert!(player.dmx_for_hostname("any-host").is_none());
    }

    #[test]
    fn test_example_config_backwards_compat() {
        // The existing examples/mtrack.yaml must still parse without error.
        let config =
            Player::deserialize(Path::new("examples/mtrack.yaml")).expect("example config failed");

        assert!(config.audio().is_some());
        assert_eq!(config.audio().unwrap().device(), "UltraLite-mk5");
        assert!(config.midi().is_some());
        assert!(!config.track_mappings().is_empty());
        assert!(config.dmx().is_some());

        // Should have been normalized into a single audio profile.
        let profiles = config.all_audio_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].audio().device(), "UltraLite-mk5");
    }
}
