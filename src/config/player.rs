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
use super::samples::{SampleDefinition, SampleTrigger, SamplesConfig, DEFAULT_MAX_SAMPLE_VOICES};
use super::statusevents::StatusEvents;
use super::trackmappings::TrackMappings;
use config::{Config, File};
use serde::Deserialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use super::error::ConfigError;
use std::error::Error;
use tracing::{error, info};

/// The configuration for the multitrack player.
#[derive(Deserialize)]
pub struct Player {
    /// The controller configuration.
    controller: Option<Controller>,
    /// The controllers configuration.
    controllers: Option<Vec<Controller>>,
    /// The audio device to use.
    audio_device: Option<String>,
    /// The audio configuration section.
    audio: Option<Audio>,
    /// The track mappings for the player.
    track_mappings: TrackMappings,
    /// The MIDI device to use. (deprecated)
    midi_device: Option<String>,
    /// The MIDI configuration section.
    midi: Option<Midi>,
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
        Player {
            controller: None,
            controllers: Some(controllers),
            audio_device: None,
            audio: Some(audio),
            track_mappings: TrackMappings { track_mappings },
            midi_device: None,
            midi,
            dmx,
            status_events: None,
            playlist: None,
            songs: songs.to_string(),
            samples: HashMap::new(),
            samples_file: None,
            sample_triggers: Vec::new(),
            max_sample_voices: None,
        }
    }

    /// Deserializes a file from the path into a player configuration struct.
    pub fn deserialize(path: &Path) -> Result<Player, ConfigError> {
        Ok(Config::builder()
            .add_source(File::from(path))
            .build()?
            .try_deserialize::<Player>()?)
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

    /// Gets the audio configuration.
    pub fn audio(&self) -> Option<Audio> {
        if let Some(audio) = &self.audio {
            return Some(audio.clone());
        } else if let Some(audio_device) = &self.audio_device {
            return Some(Audio::new(audio_device));
        }

        None
    }

    /// Gets the track mapping configuration.
    pub fn track_mappings(&self) -> &HashMap<String, Vec<u16>> {
        &self.track_mappings.track_mappings
    }

    /// Gets the MIDI configuration.
    pub fn midi(&self) -> Option<Midi> {
        if let Some(midi) = &self.midi {
            return Some(midi.clone());
        } else if let Some(midi_device) = &self.midi_device {
            return Some(Midi::new(midi_device, None));
        }

        None
    }

    /// Gets the DMX configuration.
    pub fn dmx(&self) -> Option<&Dmx> {
        self.dmx.as_ref()
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
