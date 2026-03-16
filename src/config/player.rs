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
use super::lighting::Lighting;
use super::midi::Midi;
use super::profile::{AudioConfig, Profile};
use super::samples::{SampleDefinition, SampleTrigger, SamplesConfig, DEFAULT_MAX_SAMPLE_VOICES};
use super::statusevents::StatusEvents;
use super::trackmappings::TrackMappings;
use super::trigger::{MidiTriggerInput, TriggerConfig, TriggerInput};
use config::{Config, File};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::error::ConfigError;
use std::error::Error;
use tracing::{error, info, warn};

fn default_active_playlist() -> String {
    "playlist".to_string()
}

/// The configuration for the multitrack player.
#[derive(Deserialize, Serialize, Clone)]
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
    /// The DMX configuration. (legacy)
    dmx: Option<Dmx>,
    /// Audio trigger configuration. (legacy, now in profiles)
    trigger: Option<TriggerConfig>,
    /// Unified hardware profiles, tried in priority order.
    /// Each profile contains audio (optional), MIDI (optional), and DMX (optional) configs.
    profiles: Option<Vec<Profile>>,
    /// Directory of external profile YAML files, loaded and prepended before inline profiles.
    profiles_dir: Option<String>,
    /// Events to emit to report status out via MIDI.
    status_events: Option<StatusEvents>,
    /// The path to the playlist.
    playlist: Option<String>,
    /// Directory containing playlist YAML files.
    playlists_dir: Option<String>,
    /// The active playlist name (persisted across restarts).
    #[serde(default = "default_active_playlist")]
    active_playlist: String,
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

impl Default for Player {
    fn default() -> Self {
        let mut player = Player {
            controller: None,
            controllers: None,
            audio_device: None,
            audio: None,
            track_mappings: None,
            midi_device: None,
            midi: None,
            dmx: None,
            trigger: None,
            profiles: None,
            profiles_dir: None,
            status_events: None,
            playlist: None,
            playlists_dir: None,
            active_playlist: default_active_playlist(),
            songs: ".".to_string(),
            samples: HashMap::new(),
            samples_file: None,
            sample_triggers: Vec::new(),
            max_sample_voices: None,
        };
        player.normalize();
        player
    }
}

impl Player {
    #[cfg(test)]
    pub fn new(
        controllers: Vec<Controller>,
        audio: Option<Audio>,
        midi: Option<Midi>,
        dmx: Option<Dmx>,
        track_mappings: HashMap<String, Vec<u16>>,
        songs: &str,
    ) -> Player {
        let mut player = Player {
            controller: None,
            controllers: Some(controllers),
            audio_device: None,
            audio,
            track_mappings: Some(TrackMappings {
                track_mappings: track_mappings.into_iter().collect(),
            }),
            midi_device: None,
            midi,
            dmx,
            trigger: None,
            profiles: None,
            profiles_dir: None,
            status_events: None,
            playlist: None,
            playlists_dir: None,
            active_playlist: default_active_playlist(),
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
        player.load_profiles_dir(path)?;
        player.normalize();
        Ok(player)
    }

    /// Loads profiles from the profiles_dir, if configured.
    /// Directory profiles replace inline profiles entirely. If the directory is
    /// empty, inline profiles are kept as a fallback.
    fn load_profiles_dir(&mut self, config_path: &Path) -> Result<(), ConfigError> {
        let profiles_dir_str = match &self.profiles_dir {
            Some(dir) => dir.clone(),
            None => return Ok(()),
        };

        let dir_path = if Path::new(&profiles_dir_str).is_absolute() {
            PathBuf::from(&profiles_dir_str)
        } else {
            let config_dir = config_path.parent().unwrap_or(Path::new("."));
            config_dir.join(&profiles_dir_str)
        };

        // codeql[rust/path-injection] profiles_dir comes from the local config file on disk.
        let entries = std::fs::read_dir(&dir_path).map_err(|source| ConfigError::Io {
            path: dir_path.clone(),
            source,
        })?;

        let mut yaml_paths: Vec<PathBuf> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|source| ConfigError::Io {
                path: dir_path.clone(),
                source,
            })?;
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "yaml" || ext == "yml" {
                        yaml_paths.push(path);
                    }
                }
            }
        }
        yaml_paths.sort_by(|a, b| a.file_name().cmp(&b.file_name()));

        let mut dir_profiles: Vec<Profile> = Vec::new();
        for path in &yaml_paths {
            let profile = Config::builder()
                .add_source(File::from(path.as_path()))
                .build()
                .and_then(|c| c.try_deserialize::<Profile>())
                .map_err(|source| ConfigError::ProfileParse {
                    path: path.clone(),
                    source,
                })?;
            dir_profiles.push(profile);
        }

        if !dir_profiles.is_empty() {
            // Directory profiles win — replace any inline profiles entirely.
            if self.profiles.is_some() {
                warn!("inline 'profiles' ignored; using profiles_dir");
            }
            self.profiles = Some(dir_profiles);
        }
        // If directory is empty, fall back to inline profiles (backward compat).

        Ok(())
    }

    /// Gets the profiles directory, resolved relative to the given config path.
    pub fn profiles_dir_resolved(&self, config_path: &Path) -> Option<PathBuf> {
        let dir_str = self.profiles_dir.as_ref()?;
        let dir_path = PathBuf::from(dir_str);
        if dir_path.is_absolute() {
            Some(dir_path)
        } else {
            let config_dir = config_path.parent().unwrap_or(Path::new("."));
            Some(config_dir.join(dir_path))
        }
    }

    /// Normalizes legacy configuration fields into profiles.
    /// After normalization, `profiles` is the source of truth.
    fn normalize(&mut self) {
        if self.profiles.is_some() {
            // Warn about legacy fields that will be ignored.
            if self.audio.is_some() || self.audio_device.is_some() {
                warn!("top-level 'audio'/'audio_device' ignored when 'profiles' is present");
            }
            if self.midi.is_some() || self.midi_device.is_some() {
                warn!("top-level 'midi'/'midi_device' ignored when 'profiles' is present");
            }
            if self.dmx.is_some() {
                warn!("top-level 'dmx' ignored when 'profiles' is present");
            }
            if self.trigger.is_some() {
                warn!("top-level 'trigger' ignored when 'profiles' is present");
            }
            if self.track_mappings.is_some() {
                warn!("top-level 'track_mappings' ignored when 'profiles' is present");
            }
            if !self.sample_triggers.is_empty() {
                warn!("top-level 'sample_triggers' ignored when 'profiles' is present");
            }
            if self.controller.is_some() || self.controllers.is_some() {
                warn!("top-level 'controller'/'controllers' ignored when 'profiles' is present");
            }
            return;
        }

        // Build a single profile from legacy fields.
        let audio = if let Some(audio) = &self.audio {
            Some(audio.clone())
        } else {
            self.audio_device.as_ref().map(|d| Audio::new(d))
        };

        let audio_config = audio.map(|audio| {
            let track_mappings = self
                .track_mappings
                .as_ref()
                .map(|tm| tm.track_mappings.clone())
                .unwrap_or_default();
            AudioConfig::new(audio, track_mappings)
        });

        let midi = if let Some(midi) = &self.midi {
            Some(midi.clone())
        } else {
            self.midi_device.as_ref().map(|d| Midi::new(d, None))
        };

        let dmx = self.dmx.clone();
        let mut trigger = self.trigger.clone();

        // Convert legacy sample_triggers → TriggerInput::Midi entries
        if !self.sample_triggers.is_empty() {
            let trigger_config =
                trigger.get_or_insert_with(|| TriggerConfig::new_midi_only(vec![]));
            for st in &self.sample_triggers {
                trigger_config.add_input(TriggerInput::Midi(MidiTriggerInput::new(
                    st.trigger().clone(),
                    st.sample().to_string(),
                )));
            }
        }

        // Collect controllers from legacy top-level fields.
        let controllers = self.collect_controllers();

        // Create a profile if any subsystem is configured.
        if audio_config.is_some()
            || midi.is_some()
            || dmx.is_some()
            || trigger.is_some()
            || !controllers.is_empty()
        {
            let mut profile = Profile::new(None, audio_config, midi, dmx);
            profile.set_trigger(trigger);
            profile.set_controllers(controllers);
            self.profiles = Some(vec![profile]);
        }
    }

    /// Collects controllers from legacy top-level fields.
    fn collect_controllers(&self) -> Vec<Controller> {
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

    /// Gets the controllers configuration from the first profile.
    /// Kept for backward compatibility in tests.
    #[cfg(test)]
    pub fn controllers(&self) -> Vec<Controller> {
        if let Some(profiles) = &self.profiles {
            if let Some(first) = profiles.first() {
                return first.controllers().to_vec();
            }
        }

        vec![]
    }

    /// Returns profiles filtered by hostname and ordered by priority.
    /// The first matching profile should be used.
    pub fn profiles(&self, hostname: &str) -> Vec<&Profile> {
        match &self.profiles {
            Some(profiles) => profiles
                .iter()
                .filter(|p| match p.hostname() {
                    Some(h) => h == hostname,
                    None => true,
                })
                .collect(),
            None => vec![],
        }
    }

    /// Returns all profiles without hostname filtering (for verify command).
    pub fn all_profiles(&self) -> &[Profile] {
        match &self.profiles {
            Some(profiles) => profiles.as_slice(),
            None => &[],
        }
    }

    /// Gets the audio configuration from the first profile.
    /// Kept for backward compatibility in tests.
    #[cfg(test)]
    pub fn audio(&self) -> Option<Audio> {
        if let Some(profiles) = &self.profiles {
            if let Some(first) = profiles.first() {
                return first.audio_config().map(|ac| ac.audio().clone());
            }
        }

        None
    }

    /// Gets the track mapping configuration from the first profile.
    /// Kept for backward compatibility. Returns a HashMap since callers
    /// (verify, CLI) don't need insertion-order preservation.
    pub fn track_mappings(&self) -> HashMap<String, Vec<u16>> {
        if let Some(profiles) = &self.profiles {
            if let Some(first) = profiles.first() {
                if let Some(audio_config) = first.audio_config() {
                    return audio_config.track_mappings_hash();
                }
            }
        }

        HashMap::new()
    }

    /// Gets the MIDI configuration from the first profile.
    /// Kept for backward compatibility in tests.
    #[cfg(test)]
    pub fn midi(&self) -> Option<Midi> {
        if let Some(profiles) = &self.profiles {
            if let Some(first) = profiles.first() {
                return first.midi().cloned();
            }
        }

        None
    }

    /// Gets the DMX configuration from the first profile.
    /// Kept for backward compatibility.
    pub fn dmx(&self) -> Option<&Dmx> {
        if let Some(profiles) = &self.profiles {
            if let Some(first) = profiles.first() {
                return first.dmx();
            }
        }

        None
    }

    /// Gets the status events configuration.
    pub fn status_events(&self) -> Option<StatusEvents> {
        self.status_events.clone()
    }

    /// Gets the path to the playlist.
    pub fn playlist(&self) -> Option<PathBuf> {
        self.playlist.as_ref().map(PathBuf::from)
    }

    /// Gets the playlists directory, resolved relative to the given config path.
    pub fn playlists_dir(&self, config_path: &Path) -> Option<PathBuf> {
        let dir_str = self.playlists_dir.as_ref()?;
        let dir_path = PathBuf::from(dir_str);
        if dir_path.is_absolute() {
            Some(dir_path)
        } else {
            let config_dir = config_path.parent().unwrap_or(Path::new("."));
            Some(config_dir.join(dir_path))
        }
    }

    /// Gets the active playlist name.
    pub fn active_playlist(&self) -> &str {
        &self.active_playlist
    }

    /// Sets the active playlist name (for config store mutations).
    pub fn set_active_playlist(&mut self, name: String) {
        self.active_playlist = name;
    }

    /// Sets the songs path (relative or absolute).
    pub fn set_songs(&mut self, path: &str) {
        self.songs = path.to_string();
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
            Vec::new(),
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

    /// Sets the audio configuration.
    pub fn set_audio(&mut self, audio: Option<Audio>) {
        self.audio = audio;
    }

    /// Sets the MIDI configuration.
    pub fn set_midi(&mut self, midi: Option<Midi>) {
        self.midi = midi;
    }

    /// Sets the DMX configuration.
    pub fn set_dmx(&mut self, dmx: Option<Dmx>) {
        self.dmx = dmx;
    }

    /// Sets the controllers configuration. Pass an empty vec or None to clear.
    pub fn set_controllers(&mut self, controllers: Vec<Controller>) {
        if controllers.is_empty() {
            self.controllers = None;
        } else {
            self.controllers = Some(controllers);
        }
    }

    /// Returns a mutable reference to the profiles list.
    pub fn profiles_mut(&mut self) -> &mut Option<Vec<Profile>> {
        &mut self.profiles
    }

    /// Sets the inline sample definitions.
    pub fn set_samples(&mut self, samples: HashMap<String, SampleDefinition>) {
        self.samples = samples;
    }

    /// Returns the raw `profiles_dir` value (before path resolution).
    pub fn profiles_dir_raw(&self) -> Option<&str> {
        self.profiles_dir.as_deref()
    }

    /// Returns the raw `playlist` value (before path resolution).
    pub fn playlist_raw(&self) -> Option<&str> {
        self.playlist.as_deref()
    }

    /// Returns the raw inline profiles (may be None if not set or already cleared).
    pub fn inline_profiles(&self) -> Option<&[Profile]> {
        self.profiles.as_deref()
    }

    /// Sets the profiles_dir field.
    pub fn set_profiles_dir(&mut self, dir: String) {
        self.profiles_dir = Some(dir);
    }

    /// Clears inline profiles.
    pub fn clear_inline_profiles(&mut self) {
        self.profiles = None;
    }

    /// Sets the playlists_dir field.
    pub fn set_playlists_dir_value(&mut self, dir: String) {
        self.playlists_dir = Some(dir);
    }

    /// Clears the playlist field.
    pub fn clear_playlist(&mut self) {
        self.playlist = None;
    }

    /// Clears all legacy top-level fields that have been normalized into profiles.
    pub fn clear_legacy_fields(&mut self) {
        self.audio_device = None;
        self.audio = None;
        self.midi_device = None;
        self.midi = None;
        self.dmx = None;
        self.trigger = None;
        self.track_mappings = None;
        self.controller = None;
        self.controllers = None;
        self.sample_triggers = Vec::new();
    }

    /// Returns a mutable reference to the DMX config's lighting section.
    pub fn lighting_mut(&mut self) -> Option<&mut Lighting> {
        self.dmx.as_mut().and_then(|d| d.lighting_mut())
    }

    /// Returns a reference to the DMX config's lighting section (through all profiles).
    /// Checks the first profile's DMX config for lighting.
    pub fn lighting_from_profiles(&self) -> Option<&Lighting> {
        self.profiles
            .as_ref()
            .and_then(|ps| ps.first())
            .and_then(|p| p.dmx())
            .and_then(|d| d.lighting())
    }

    /// Deserializes a config file without running normalize() or loading profiles_dir.
    /// Used by the migrate command to inspect raw inline fields.
    pub fn deserialize_raw(path: &Path) -> Result<Player, ConfigError> {
        let player = Config::builder()
            .add_source(File::from(path))
            .build()?
            .try_deserialize::<Player>()?;
        Ok(player)
    }

    /// Returns the raw top-level DMX field (before normalization into profiles).
    pub fn dmx_raw(&self) -> Option<&Dmx> {
        self.dmx.as_ref()
    }

    /// Returns whether there are any legacy top-level fields set.
    pub fn has_legacy_fields(&self) -> bool {
        self.audio_device.is_some()
            || self.audio.is_some()
            || self.midi_device.is_some()
            || self.midi.is_some()
            || self.dmx.is_some()
            || self.trigger.is_some()
            || self.track_mappings.is_some()
            || self.controller.is_some()
            || self.controllers.is_some()
            || !self.sample_triggers.is_empty()
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

        // Unified profiles should have been created from legacy fields.
        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(
            profiles[0].audio_config().unwrap().audio().device(),
            "mock-device"
        );
        assert_eq!(
            profiles[0].audio_config().unwrap().audio().sample_rate(),
            48000
        );
        assert_eq!(
            profiles[0]
                .audio_config()
                .unwrap()
                .track_mappings()
                .get("click"),
            Some(&vec![1u16])
        );
        assert_eq!(
            profiles[0]
                .audio_config()
                .unwrap()
                .track_mappings()
                .get("cue"),
            Some(&vec![2u16])
        );
        assert!(profiles[0].hostname().is_none());
        assert!(profiles[0].midi().is_some());
        assert_eq!(profiles[0].midi().unwrap().device(), "mock-midi");

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

        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(
            profiles[0].audio_config().unwrap().audio().device(),
            "mock-device"
        );
        assert_eq!(
            profiles[0]
                .audio_config()
                .unwrap()
                .track_mappings()
                .get("drums"),
            Some(&vec![1u16])
        );
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
    fn test_profiles_parse() {
        let player = player_from_yaml(
            r#"
songs: songs
profiles:
  - hostname: pi-a
    audio:
      device: mock-device-a
      sample_rate: 48000
      track_mappings:
        drums: [1]
        synth: [2]
    midi:
      device: mock-midi-a
  - hostname: pi-b
    audio:
      device: mock-device-b
      track_mappings:
        drums: [11]
        synth: [12]
    midi:
      device: mock-midi-b
    dmx:
      universes:
        - universe: 1
          name: light-show
  - audio:
      device: mock-fallback
      track_mappings:
        drums: [1]
"#,
        );

        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 3);

        assert_eq!(profiles[0].hostname(), Some("pi-a"));
        assert_eq!(
            profiles[0].audio_config().unwrap().audio().device(),
            "mock-device-a"
        );
        assert_eq!(
            profiles[0].audio_config().unwrap().audio().sample_rate(),
            48000
        );
        assert!(profiles[0].midi().is_some());
        assert!(profiles[0].dmx().is_none());

        assert_eq!(profiles[1].hostname(), Some("pi-b"));
        assert_eq!(
            profiles[1].audio_config().unwrap().audio().device(),
            "mock-device-b"
        );
        assert!(profiles[1].midi().is_some());
        assert!(profiles[1].dmx().is_some());

        assert_eq!(profiles[2].hostname(), None);
        assert_eq!(
            profiles[2].audio_config().unwrap().audio().device(),
            "mock-fallback"
        );
        assert!(profiles[2].midi().is_none());
        assert!(profiles[2].dmx().is_none());
    }

    #[test]
    fn test_profiles_filter_by_hostname() {
        let player = player_from_yaml(
            r#"
songs: songs
profiles:
  - hostname: pi-a
    audio:
      device: mock-device-a
      track_mappings:
        drums: [1]
  - hostname: pi-b
    audio:
      device: mock-device-b
      track_mappings:
        drums: [11]
  - audio:
      device: mock-fallback
      track_mappings:
        drums: [1]
"#,
        );

        // pi-a sees its own profile + the wildcard.
        let pi_a = player.profiles("pi-a");
        assert_eq!(pi_a.len(), 2);
        assert_eq!(
            pi_a[0].audio_config().unwrap().audio().device(),
            "mock-device-a"
        );
        assert_eq!(
            pi_a[1].audio_config().unwrap().audio().device(),
            "mock-fallback"
        );

        // pi-b sees its own profile + the wildcard.
        let pi_b = player.profiles("pi-b");
        assert_eq!(pi_b.len(), 2);
        assert_eq!(
            pi_b[0].audio_config().unwrap().audio().device(),
            "mock-device-b"
        );
        assert_eq!(
            pi_b[1].audio_config().unwrap().audio().device(),
            "mock-fallback"
        );

        // Unknown host only sees the wildcard.
        let unknown = player.profiles("pi-c");
        assert_eq!(unknown.len(), 1);
        assert_eq!(
            unknown[0].audio_config().unwrap().audio().device(),
            "mock-fallback"
        );
    }

    #[test]
    fn test_profile_without_midi_dmx() {
        let player = player_from_yaml(
            r#"
songs: songs
profiles:
  - audio:
      device: mock-device
      track_mappings:
        drums: [1]
"#,
        );

        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert!(profiles[0].midi().is_none());
        assert!(profiles[0].dmx().is_none());
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
profiles:
  - audio:
      device: profile-device
      track_mappings:
        click: [1]
"#,
        );

        // Profiles should be used, not legacy.
        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(
            profiles[0].audio_config().unwrap().audio().device(),
            "profile-device"
        );
        assert_eq!(
            profiles[0]
                .audio_config()
                .unwrap()
                .track_mappings()
                .get("click"),
            Some(&vec![1u16])
        );

        // Backward compat getters return from the first profile.
        assert_eq!(player.audio().unwrap().device(), "profile-device");
        assert_eq!(player.track_mappings().get("click"), Some(&vec![1u16]));
    }

    #[test]
    fn test_no_profiles_when_no_audio_config() {
        let player = player_from_yaml(
            r#"
songs: songs
"#,
        );

        // No audio at all.
        assert!(player.all_profiles().is_empty());
        assert!(player.audio().is_none());
        assert!(player.track_mappings().is_empty());
    }

    #[test]
    fn test_profiles_without_top_level_track_mappings() {
        let player = player_from_yaml(
            r#"
songs: songs
profiles:
  - audio:
      device: mock-device
      track_mappings:
        drums: [1]
        synth: [2]
"#,
        );

        // Should work without top-level track_mappings.
        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(
            profiles[0]
                .audio_config()
                .unwrap()
                .track_mappings()
                .get("drums"),
            Some(&vec![1u16])
        );

        // Backward compat getter returns from profile.
        assert_eq!(player.track_mappings().get("drums"), Some(&vec![1u16]));
    }

    #[test]
    fn test_hostname_deconfliction() {
        let player = player_from_yaml(
            r#"
songs: songs
profiles:
  - hostname: pi-a
    audio:
      device: "Behringer WING"
      track_mappings:
        drums: [1]
        synth: [2]
  - hostname: pi-b
    audio:
      device: "Behringer WING"
      track_mappings:
        drums: [11]
        synth: [12]
"#,
        );

        let pi_a = player.profiles("pi-a");
        assert_eq!(pi_a.len(), 1);
        assert_eq!(
            pi_a[0]
                .audio_config()
                .unwrap()
                .track_mappings()
                .get("drums"),
            Some(&vec![1u16])
        );

        let pi_b = player.profiles("pi-b");
        assert_eq!(pi_b.len(), 1);
        assert_eq!(
            pi_b[0]
                .audio_config()
                .unwrap()
                .track_mappings()
                .get("drums"),
            Some(&vec![11u16])
        );

        // Different device name, same mappings — ensures isolation.
        let pi_c = player.profiles("pi-c");
        assert!(pi_c.is_empty());
    }

    #[test]
    fn test_normalization_creates_profile() {
        let player = player_from_yaml(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
dmx:
  dim_speed_modifier: 0.25
  universes:
  - universe: 1
    name: light-show
"#,
        );

        // Legacy dmx config should be normalized into unified profile
        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert!(profiles[0].dmx().is_some());
        assert_eq!(profiles[0].dmx().unwrap().dimming_speed_modifier(), 0.25);
    }

    #[test]
    fn test_trigger_only_normalizes_into_profile() {
        let player = player_from_yaml(
            r#"
songs: songs
trigger:
  device: "UltraLite-mk5"
  inputs:
    - kind: audio
      channel: 1
      sample: "kick"
"#,
        );

        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert!(profiles[0].audio_config().is_none());
        assert!(profiles[0].midi().is_none());
        assert!(profiles[0].dmx().is_none());
        assert!(profiles[0].trigger().is_some());
        assert_eq!(
            profiles[0].trigger().unwrap().device(),
            Some("UltraLite-mk5")
        );
    }

    #[test]
    fn test_trigger_with_audio_normalizes_into_profile() {
        let player = player_from_yaml(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
trigger:
  device: "UltraLite-mk5"
  inputs:
    - kind: audio
      channel: 1
      sample: "kick"
"#,
        );

        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert!(profiles[0].audio_config().is_some());
        assert!(profiles[0].trigger().is_some());
        assert_eq!(
            profiles[0].audio_config().unwrap().audio().device(),
            "mock-device"
        );
        assert_eq!(
            profiles[0].trigger().unwrap().device(),
            Some("UltraLite-mk5")
        );
    }

    #[test]
    fn test_legacy_controllers_normalize_into_profile() {
        let player = player_from_yaml(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
controllers:
  - kind: grpc
    port: 43234
  - kind: osc
"#,
        );

        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].controllers().len(), 2);
        assert_eq!(player.controllers().len(), 2);
    }

    #[test]
    fn test_controllers_only_normalize_into_profile() {
        let player = player_from_yaml(
            r#"
songs: songs
controllers:
  - kind: grpc
"#,
        );

        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].controllers().len(), 1);
    }

    #[test]
    fn test_profile_controllers_not_overridden_by_legacy() {
        let player = player_from_yaml(
            r#"
songs: songs
controllers:
  - kind: grpc
profiles:
  - audio:
      device: mock-device
      track_mappings:
        drums: [1]
    controllers:
      - kind: osc
"#,
        );

        // Profile controllers should be used, not legacy.
        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].controllers().len(), 1);
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

        // Should have been normalized into a single unified profile.
        let profiles = config.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(
            profiles[0].audio_config().unwrap().audio().device(),
            "UltraLite-mk5"
        );
        assert!(profiles[0].midi().is_some());
        assert!(profiles[0].dmx().is_some());
    }

    /// Helper to create a Player from a YAML string with an associated temp directory.
    /// The `dir_setup` closure receives the temp dir path for creating profile files.
    fn player_with_dir(yaml: &str, dir_setup: impl FnOnce(&Path)) -> Player {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("mtrack.yaml");
        std::fs::write(&config_path, yaml).unwrap();
        dir_setup(dir.path());
        Player::deserialize(&config_path).expect("Failed to deserialize")
    }

    fn write_profile(dir: &Path, filename: &str, yaml: &str) {
        std::fs::write(dir.join(filename), yaml).unwrap();
    }

    #[test]
    fn test_profiles_dir_loads_profiles() {
        let player = player_with_dir("songs: songs\nprofiles_dir: profiles/\n", |dir| {
            std::fs::create_dir(dir.join("profiles")).unwrap();
            write_profile(
                &dir.join("profiles"),
                "pi-a.yaml",
                "hostname: pi-a\naudio:\n  device: device-a\n  track_mappings:\n    drums: [1]\n",
            );
            write_profile(
                &dir.join("profiles"),
                "pi-b.yml",
                "hostname: pi-b\naudio:\n  device: device-b\n  track_mappings:\n    drums: [11]\n",
            );
        });

        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 2);
        assert_eq!(profiles[0].hostname(), Some("pi-a"));
        assert_eq!(
            profiles[0].audio_config().unwrap().audio().device(),
            "device-a"
        );
        assert_eq!(profiles[1].hostname(), Some("pi-b"));
        assert_eq!(
            profiles[1].audio_config().unwrap().audio().device(),
            "device-b"
        );
    }

    #[test]
    fn test_profiles_dir_replaces_inline() {
        let player = player_with_dir(
            concat!(
                "songs: songs\n",
                "profiles_dir: profiles/\n",
                "profiles:\n",
                "  - audio:\n",
                "      device: inline-fallback\n",
                "      track_mappings:\n",
                "        drums: [1]\n",
            ),
            |dir| {
                std::fs::create_dir(dir.join("profiles")).unwrap();
                write_profile(
                    &dir.join("profiles"),
                    "pi-a.yaml",
                    "hostname: pi-a\naudio:\n  device: dir-device\n  track_mappings:\n    drums: [1]\n",
                );
            },
        );

        let profiles = player.all_profiles();
        // Directory profiles replace inline entirely.
        assert_eq!(profiles.len(), 1);
        assert_eq!(
            profiles[0].audio_config().unwrap().audio().device(),
            "dir-device"
        );
        assert_eq!(profiles[0].hostname(), Some("pi-a"));
    }

    #[test]
    fn test_profiles_dir_no_duplication_on_roundtrip() {
        // Regression test: serializing and re-deserializing a config with
        // profiles_dir must not duplicate the directory-loaded profiles.
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("mtrack.yaml");
        std::fs::write(&config_path, "songs: songs\nprofiles_dir: profiles/\n").unwrap();
        std::fs::create_dir(dir.path().join("profiles")).unwrap();
        write_profile(
            &dir.path().join("profiles"),
            "pi-a.yaml",
            "hostname: pi-a\naudio:\n  device: dir-device\n  track_mappings:\n    drums: [1]\n",
        );

        let player = Player::deserialize(&config_path).unwrap();
        assert_eq!(player.all_profiles().len(), 1);

        // Serialize and write back (simulates config store save).
        let yaml = crate::util::to_yaml_string(&player).unwrap();
        std::fs::write(&config_path, &yaml).unwrap();

        // Re-deserialize: should still have exactly 1 profile, not 2.
        let player2 = Player::deserialize(&config_path).unwrap();
        assert_eq!(
            player2.all_profiles().len(),
            1,
            "profiles should not be duplicated after roundtrip"
        );

        assert_eq!(
            player2.all_profiles()[0]
                .audio_config()
                .unwrap()
                .audio()
                .device(),
            "dir-device"
        );
    }

    #[test]
    fn test_profiles_dir_only_serializes_correctly() {
        // When the config has profiles_dir but no inline profiles, the
        // directory profiles should appear in serialized YAML output.
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("mtrack.yaml");
        std::fs::write(&config_path, "songs: songs\nprofiles_dir: profiles/\n").unwrap();
        std::fs::create_dir(dir.path().join("profiles")).unwrap();
        write_profile(
            &dir.path().join("profiles"),
            "pi-a.yaml",
            "hostname: pi-a\naudio:\n  device: device-a\n  track_mappings:\n    drums: [1]\n",
        );

        let player = Player::deserialize(&config_path).unwrap();
        assert_eq!(player.all_profiles().len(), 1);

        // Serialized YAML must include the profile from the directory.
        let yaml = crate::util::to_yaml_string(&player).unwrap();
        assert!(
            yaml.contains("pi-a"),
            "serialized YAML should contain dir profile hostname"
        );
        assert!(
            yaml.contains("device-a"),
            "serialized YAML should contain dir profile device"
        );

        // Roundtrip: re-deserialize should still have exactly 1 profile.
        std::fs::write(&config_path, &yaml).unwrap();
        let player2 = Player::deserialize(&config_path).unwrap();
        assert_eq!(
            player2.all_profiles().len(),
            1,
            "profiles should not be duplicated after roundtrip"
        );
    }

    #[test]
    fn test_profiles_dir_empty_directory() {
        let player = player_with_dir(
            concat!(
                "songs: songs\n",
                "profiles_dir: profiles/\n",
                "profiles:\n",
                "  - audio:\n",
                "      device: inline-device\n",
                "      track_mappings:\n",
                "        drums: [1]\n",
            ),
            |dir| {
                std::fs::create_dir(dir.join("profiles")).unwrap();
            },
        );

        // Only the inline profile should be present.
        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(
            profiles[0].audio_config().unwrap().audio().device(),
            "inline-device"
        );
    }

    #[test]
    fn test_profiles_dir_missing_directory_errors() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("mtrack.yaml");
        std::fs::write(&config_path, "songs: songs\nprofiles_dir: nonexistent/\n").unwrap();

        match Player::deserialize(&config_path) {
            Err(ConfigError::Io { path, .. }) => {
                assert!(path.to_string_lossy().contains("nonexistent"));
            }
            Err(other) => panic!("expected ConfigError::Io, got: {other}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[test]
    fn test_profiles_dir_invalid_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("mtrack.yaml");
        std::fs::write(&config_path, "songs: songs\nprofiles_dir: profiles/\n").unwrap();
        std::fs::create_dir(dir.path().join("profiles")).unwrap();
        write_profile(
            &dir.path().join("profiles"),
            "bad.yaml",
            "this is not valid profile yaml: [[[",
        );

        match Player::deserialize(&config_path) {
            Err(ConfigError::ProfileParse { path, .. }) => {
                assert!(
                    path.to_string_lossy().contains("bad.yaml"),
                    "should mention filename: {path:?}"
                );
            }
            Err(other) => panic!("expected ConfigError::ProfileParse, got: {other}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[test]
    fn test_profiles_dir_ignores_non_yaml_files() {
        let player = player_with_dir("songs: songs\nprofiles_dir: profiles/\n", |dir| {
            std::fs::create_dir(dir.join("profiles")).unwrap();
            write_profile(
                &dir.join("profiles"),
                "pi-a.yaml",
                "hostname: pi-a\naudio:\n  device: device-a\n  track_mappings:\n    drums: [1]\n",
            );
            // These should be ignored.
            write_profile(&dir.join("profiles"), "notes.txt", "just some notes");
            write_profile(
                &dir.join("profiles"),
                "data.json",
                r#"{"not": "a profile"}"#,
            );
        });

        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].hostname(), Some("pi-a"));
    }

    #[test]
    fn test_profiles_dir_sorts_by_filename() {
        let player = player_with_dir("songs: songs\nprofiles_dir: profiles/\n", |dir| {
            std::fs::create_dir(dir.join("profiles")).unwrap();
            // Write in reverse order to verify sorting.
            write_profile(
                &dir.join("profiles"),
                "03-fallback.yml",
                "audio:\n  device: fallback\n  track_mappings:\n    drums: [1]\n",
            );
            write_profile(
                &dir.join("profiles"),
                "01-pi-a.yaml",
                "hostname: pi-a\naudio:\n  device: device-a\n  track_mappings:\n    drums: [1]\n",
            );
            write_profile(
                &dir.join("profiles"),
                "02-pi-b.yaml",
                "hostname: pi-b\naudio:\n  device: device-b\n  track_mappings:\n    drums: [11]\n",
            );
        });

        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 3);
        assert_eq!(
            profiles[0].audio_config().unwrap().audio().device(),
            "device-a"
        );
        assert_eq!(
            profiles[1].audio_config().unwrap().audio().device(),
            "device-b"
        );
        assert_eq!(
            profiles[2].audio_config().unwrap().audio().device(),
            "fallback"
        );
    }

    #[test]
    fn test_profiles_dir_with_hostname_filtering() {
        // With profiles_dir set, directory profiles replace inline entirely.
        // Hostname filtering works on the directory profiles only.
        let player = player_with_dir(
            concat!(
                "songs: songs\n",
                "profiles_dir: profiles/\n",
                "profiles:\n",
                "  - audio:\n",
                "      device: inline-fallback\n",
                "      track_mappings:\n",
                "        drums: [1]\n",
            ),
            |dir| {
                std::fs::create_dir(dir.join("profiles")).unwrap();
                write_profile(
                    &dir.join("profiles"),
                    "01-pi-a.yaml",
                    "hostname: pi-a\naudio:\n  device: device-a\n  track_mappings:\n    drums: [1]\n",
                );
                write_profile(
                    &dir.join("profiles"),
                    "02-pi-b.yaml",
                    "hostname: pi-b\naudio:\n  device: device-b\n  track_mappings:\n    drums: [11]\n",
                );
                // A fallback profile with no hostname in the directory.
                write_profile(
                    &dir.join("profiles"),
                    "99-fallback.yaml",
                    "audio:\n  device: dir-fallback\n  track_mappings:\n    drums: [1]\n",
                );
            },
        );

        // pi-a sees its directory profile + dir fallback.
        let pi_a = player.profiles("pi-a");
        assert_eq!(pi_a.len(), 2);
        assert_eq!(pi_a[0].audio_config().unwrap().audio().device(), "device-a");
        assert_eq!(
            pi_a[1].audio_config().unwrap().audio().device(),
            "dir-fallback"
        );

        // pi-b sees its directory profile + dir fallback.
        let pi_b = player.profiles("pi-b");
        assert_eq!(pi_b.len(), 2);
        assert_eq!(pi_b[0].audio_config().unwrap().audio().device(), "device-b");
        assert_eq!(
            pi_b[1].audio_config().unwrap().audio().device(),
            "dir-fallback"
        );

        // Unknown host sees only dir fallback.
        let unknown = player.profiles("pi-c");
        assert_eq!(unknown.len(), 1);
        assert_eq!(
            unknown[0].audio_config().unwrap().audio().device(),
            "dir-fallback"
        );
    }

    #[test]
    fn test_playlist_getter() {
        let player = player_from_yaml(
            r#"
songs: songs
playlist: my_playlist.yaml
"#,
        );
        assert_eq!(
            player.playlist().unwrap(),
            std::path::PathBuf::from("my_playlist.yaml")
        );
    }

    #[test]
    fn test_playlist_none() {
        let player = player_from_yaml(
            r#"
songs: songs
"#,
        );
        assert!(player.playlist().is_none());
    }

    #[test]
    fn test_songs_absolute_path() {
        let player = player_from_yaml(
            r#"
songs: /absolute/path/to/songs
"#,
        );
        let songs_path = player.songs(Path::new("/some/config.yaml"));
        assert_eq!(
            songs_path,
            std::path::PathBuf::from("/absolute/path/to/songs")
        );
    }

    #[test]
    fn test_songs_relative_path() {
        let player = player_from_yaml(
            r#"
songs: relative/songs
"#,
        );
        let songs_path = player.songs(Path::new("/config/dir/mtrack.yaml"));
        assert_eq!(
            songs_path,
            std::path::PathBuf::from("/config/dir/relative/songs")
        );
    }

    #[test]
    fn test_dmx_none_without_profiles() {
        let player = player_from_yaml(
            r#"
songs: songs
"#,
        );
        assert!(player.dmx().is_none());
    }

    #[test]
    fn test_profiles_none_returns_empty() {
        let player = player_from_yaml(
            r#"
songs: songs
"#,
        );
        assert!(player.profiles("any-host").is_empty());
    }

    #[test]
    fn test_legacy_single_controller_normalizes() {
        let player = player_from_yaml(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
controller:
  kind: grpc
  port: 43234
"#,
        );
        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].controllers().len(), 1);
    }

    #[test]
    fn test_max_sample_voices_default() {
        let player = player_from_yaml(
            r#"
songs: songs
"#,
        );
        assert_eq!(player.max_sample_voices(), super::DEFAULT_MAX_SAMPLE_VOICES);
    }

    #[test]
    fn test_max_sample_voices_custom() {
        let player = player_from_yaml(
            r#"
songs: songs
max_sample_voices: 64
"#,
        );
        assert_eq!(player.max_sample_voices(), 64);
    }

    #[test]
    fn test_samples_config_inline() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("mtrack.yaml");
        std::fs::write(
            &config_path,
            r#"
songs: songs
samples:
  kick:
    file: kick.wav
    output_channels: [1, 2]
"#,
        )
        .unwrap();
        let player = Player::deserialize(&config_path).unwrap();
        let sc = player.samples_config(&config_path).unwrap();
        assert!(sc.samples().contains_key("kick"));
        assert_eq!(sc.samples().get("kick").unwrap().file(), Some("kick.wav"));
    }

    #[test]
    fn test_samples_config_with_external_file() {
        let dir = tempfile::tempdir().unwrap();

        // Write the external samples file.
        let samples_path = dir.path().join("samples.yaml");
        std::fs::write(
            &samples_path,
            r#"
samples:
  snare:
    file: snare.wav
    output_channels: [3, 4]
"#,
        )
        .unwrap();

        // Write the main config that references the external file.
        let config_path = dir.path().join("mtrack.yaml");
        std::fs::write(
            &config_path,
            r#"
songs: songs
samples_file: samples.yaml
samples:
  kick:
    file: kick.wav
    output_channels: [1, 2]
"#,
        )
        .unwrap();

        let player = Player::deserialize(&config_path).unwrap();
        let sc = player.samples_config(&config_path).unwrap();
        // Both inline and external samples should be present.
        assert!(sc.samples().contains_key("kick"));
        assert!(sc.samples().contains_key("snare"));
    }

    #[test]
    fn test_profiles_dir_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let profiles_dir = dir.path().join("abs_profiles");
        std::fs::create_dir(&profiles_dir).unwrap();
        std::fs::write(
            profiles_dir.join("host.yaml"),
            "hostname: pi-x\naudio:\n  device: dev-x\n  track_mappings:\n    drums: [1]\n",
        )
        .unwrap();

        let config_path = dir.path().join("mtrack.yaml");
        std::fs::write(
            &config_path,
            format!(
                "songs: songs\nprofiles_dir: {}\n",
                profiles_dir.to_str().unwrap()
            ),
        )
        .unwrap();

        let player = Player::deserialize(&config_path).unwrap();
        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].hostname(), Some("pi-x"));
    }

    #[test]
    fn test_legacy_sample_triggers_normalize_into_trigger_config() {
        let player = player_from_yaml(
            r#"
songs: songs
audio:
  device: mock-device
track_mappings:
  click: [1]
sample_triggers:
  - trigger:
      type: note_on
      channel: 1
      key: 60
      velocity: 127
    sample: kick
"#,
        );
        let profiles = player.all_profiles();
        assert_eq!(profiles.len(), 1);
        assert!(profiles[0].trigger().is_some());
    }

    #[test]
    fn test_songs_relative_path_no_parent() {
        // When player_path has no parent (e.g. a bare filename), the songs
        // path falls back to the raw config value.
        let player = player_from_yaml(
            r#"
songs: my_songs
"#,
        );
        // A path like "" has no parent
        let result = player.songs(Path::new(""));
        // With no parent, should return the raw songs path
        assert_eq!(result, PathBuf::from("my_songs"));
    }

    #[test]
    fn test_samples_config_absolute_external_file() {
        let dir = tempfile::tempdir().unwrap();

        // Write the external samples file at an absolute path
        let samples_path = dir.path().join("abs_samples.yaml");
        std::fs::write(
            &samples_path,
            r#"
samples:
  hat:
    file: hat.wav
    output_channels: [5, 6]
"#,
        )
        .unwrap();

        // Main config references external file via absolute path
        let config_path = dir.path().join("mtrack.yaml");
        std::fs::write(
            &config_path,
            format!(
                "songs: songs\nsamples_file: {}\n",
                samples_path.to_str().unwrap()
            ),
        )
        .unwrap();

        let player = Player::deserialize(&config_path).unwrap();
        let sc = player.samples_config(&config_path).unwrap();
        assert!(sc.samples().contains_key("hat"));
    }

    #[test]
    fn test_serialize_deserialize_round_trip() {
        let yaml = r#"
songs: songs
profiles:
  - hostname: pi-a
    audio:
      device: mock-device
      sample_rate: 48000
      track_mappings:
        click: [1]
        cue: [2]
    midi:
      device: mock-midi
      playback_delay: 500ms
    dmx:
      universes:
        - universe: 1
          name: main
    controllers:
      - kind: grpc
        port: 43234
      - kind: osc
  - audio:
      device: fallback
      track_mappings:
        drums: [1, 2]
"#;

        let player = player_from_yaml(yaml);

        // Serialize to YAML via util::to_yaml_string
        let serialized =
            crate::util::to_yaml_string(&player).expect("serialization should succeed");

        // Deserialize the serialized YAML back
        let mut temp = tempfile::NamedTempFile::with_suffix(".yaml").unwrap();
        temp.write_all(serialized.as_bytes()).unwrap();
        let round_tripped =
            Player::deserialize(temp.path()).expect("round-trip deserialization should succeed");

        // Compare by serializing both to JSON (deterministic field order)
        let json1 = serde_json::to_value(&player).unwrap();
        let json2 = serde_json::to_value(&round_tripped).unwrap();
        assert_eq!(json1, json2, "round-trip should preserve all config values");
    }
}
