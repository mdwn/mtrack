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
use std::{collections::HashMap, error::Error, io::Write, path::Path};

use config::{Config, File};
use midly::live::LiveEvent;
use serde::{Deserialize, Serialize};
use tracing::info;

use super::{
    midi::{self, ToMidiEvent},
    samples::{SampleDefinition, SampleTrigger, SamplesConfig},
    track::Track,
};

/// A YAML represetnation of a song.
#[derive(Deserialize, Serialize)]
pub struct Song {
    /// The name of the song.
    name: String,
    /// The MIDI event to emit when the song is selected.
    midi_event: Option<midi::Event>,
    /// The associated MIDI file to play.
    midi_file: Option<String>,
    /// MIDI playback configuration. Will override the midi_file field.
    midi_playback: Option<MidiPlayback>,
    /// The light show configurations.
    light_shows: Option<Vec<LightShow>>,
    /// The lighting shows for this song.
    lighting: Option<Vec<LightingShow>>,
    /// The associated tracks to play.
    tracks: Vec<Track>,
    /// Song-specific sample definitions (overrides global samples with same name).
    #[serde(default)]
    samples: HashMap<String, SampleDefinition>,
    /// Song-specific sample trigger mappings (overrides global triggers with same MIDI event).
    #[serde(default)]
    sample_triggers: Vec<SampleTrigger>,
}

impl Song {
    /// Creates a new song configuration.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &str,
        midi_event: Option<midi::Event>,
        midi_file: Option<String>,
        midi_playback: Option<MidiPlayback>,
        light_shows: Option<Vec<LightShow>>,
        lighting: Option<Vec<LightingShow>>,
        tracks: Vec<Track>,
        samples: HashMap<String, SampleDefinition>,
        sample_triggers: Vec<SampleTrigger>,
    ) -> Song {
        Song {
            name: name.to_string(),
            midi_event,
            midi_file,
            midi_playback,
            light_shows,
            lighting,
            tracks,
            samples,
            sample_triggers,
        }
    }

    /// Deserializes a file from the path into a song configuration struct.
    pub fn deserialize(path: &Path) -> Result<Song, crate::config::ConfigError> {
        Ok(Config::builder()
            .add_source(File::from(path))
            .build()?
            .try_deserialize::<Song>()?)
    }

    /// Serialize and save a song configuration struct to a file at given path.
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        let serialized = crate::util::to_yaml_string(self)?;
        info!(serialized);

        let mut file = match std::fs::File::create(path) {
            Ok(file) => file,
            Err(err) => return Err(Box::new(err)),
        };

        match file.write_all(serialized.as_bytes()) {
            Ok(_result) => Ok(()),
            Err(err) => Err(Box::new(err)),
        }
    }

    /// Gets the name of the song.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Sets the name of the song.
    pub fn set_name(&mut self, name: &str) {
        self.name = name.to_string();
    }

    /// Gets the MIDI event associated with the song.
    pub fn midi_event(&self) -> Result<Option<LiveEvent<'static>>, Box<dyn Error>> {
        Ok(match &self.midi_event {
            Some(midi_event) => Some(midi_event.to_midi_event()?),
            None => None,
        })
    }

    /// Gets the MIDI playback associated with the song.
    pub fn midi_playback(&self) -> Option<MidiPlayback> {
        if let Some(midi_playback) = &self.midi_playback {
            return Some(midi_playback.clone());
        } else if let Some(midi_file) = &self.midi_file {
            return Some(MidiPlayback {
                file: midi_file.clone(),
                exclude_midi_channels: None,
            });
        }

        None
    }

    /// Gets the light shows associated with the song.
    pub fn light_shows(&self) -> Option<&Vec<LightShow>> {
        self.light_shows.as_ref()
    }

    /// Gets the DSL lighting shows associated with the song.
    pub fn lighting(&self) -> Option<&Vec<LightingShow>> {
        self.lighting.as_ref()
    }

    /// Gets the tracks associated with the song.
    pub fn tracks(&self) -> &Vec<Track> {
        &self.tracks
    }

    /// Gets the song-specific samples configuration.
    /// Returns a SamplesConfig that can be merged with the global config.
    pub fn samples_config(&self) -> SamplesConfig {
        SamplesConfig::new(
            self.samples.clone(),
            self.sample_triggers.clone(),
            0, // Per-song config doesn't set global max_voices
        )
    }
}

// A YAML representation of MIDI files with channel exclusions.
#[derive(Deserialize, Clone, Serialize)]
pub struct MidiPlayback {
    /// The MIDI file.
    file: String,

    /// The MIDI channels to exclude from this MIDI file. Useful if you want to exclude lighting
    /// data from being played back with other MIDI automation.
    exclude_midi_channels: Option<Vec<u8>>,
}

impl MidiPlayback {
    /// Gets the file associated with the MIDI playback.
    pub fn file(&self) -> String {
        self.file.clone()
    }

    /// Gets the MIDI channels to exclude.
    pub fn exclude_midi_channels(&self) -> Vec<u8> {
        self.exclude_midi_channels
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|channel| channel - 1)
            .collect()
    }
}

// A YAML representation of light shows.
#[derive(Deserialize, Clone, Serialize)]
pub struct LightShow {
    /// The name of the universe. Will be matched against the universes configured in the DMX engine
    /// to determine where (if anywhere) this light show should be sent.
    universe_name: String,

    /// The associated MIDI file to interpret as DMX to play.
    dmx_file: String,

    /// The MIDI channels from this MIDI file to use as lighting data. If empty,
    /// all channels will be used.
    midi_channels: Option<Vec<u8>>,
}

impl LightShow {
    /// Constructor function
    pub fn new(universe_name: String, dmx_file: String, midi_channels: Option<Vec<u8>>) -> Self {
        Self {
            universe_name,
            dmx_file,
            midi_channels,
        }
    }

    /// Gets the universe name for the light show.
    pub fn universe_name(&self) -> String {
        self.universe_name.clone()
    }

    /// Gets the DMX (MIDI) file associated with the light show.
    pub fn dmx_file(&self) -> String {
        self.dmx_file.clone()
    }

    /// Gets the MIDI channels that should be associated with light show data.
    pub fn midi_channels(&self) -> Vec<u8> {
        self.midi_channels
            .clone()
            .unwrap_or_default()
            .iter()
            .map(|channel| channel - 1)
            .collect()
    }
}

/// A lighting show that references a DSL file.
#[derive(Deserialize, Clone, Serialize)]
pub struct LightingShow {
    /// The path to the DSL file.
    file: String,
}

impl LightingShow {
    /// Creates a new DSL lighting show.
    pub fn new(file: String) -> Self {
        Self { file }
    }

    /// Gets the DSL file path.
    pub fn file(&self) -> &str {
        &self.file
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::track::Track;

    fn minimal_song() -> Song {
        Song::new(
            "Test Song",
            None,
            None,
            None,
            None,
            None,
            vec![Track::new("track1".to_string(), "track1.wav", None)],
            HashMap::new(),
            vec![],
        )
    }

    #[test]
    fn test_lighting_show_creation() {
        let show = LightingShow::new("test.light".to_string());
        assert_eq!(show.file(), "test.light");
    }

    #[test]
    fn song_name() {
        assert_eq!(minimal_song().name(), "Test Song");
    }

    #[test]
    fn song_tracks() {
        let song = minimal_song();
        assert_eq!(song.tracks().len(), 1);
        assert_eq!(song.tracks()[0].name(), "track1");
    }

    #[test]
    fn midi_event_none() {
        let song = minimal_song();
        assert!(song.midi_event().unwrap().is_none());
    }

    #[test]
    fn midi_event_some() {
        let song = Song::new(
            "test",
            Some(midi::note_on(1, 60, 127)),
            None,
            None,
            None,
            None,
            vec![Track::new("t".to_string(), "t.wav", None)],
            HashMap::new(),
            vec![],
        );
        let event = song.midi_event().unwrap();
        assert!(event.is_some());
    }

    #[test]
    fn midi_playback_none() {
        let song = minimal_song();
        assert!(song.midi_playback().is_none());
    }

    #[test]
    fn midi_playback_from_midi_file_field() {
        let song = Song::new(
            "test",
            None,
            Some("song.mid".to_string()),
            None,
            None,
            None,
            vec![Track::new("t".to_string(), "t.wav", None)],
            HashMap::new(),
            vec![],
        );
        let playback = song.midi_playback().unwrap();
        assert_eq!(playback.file(), "song.mid");
        assert!(playback.exclude_midi_channels().is_empty());
    }

    #[test]
    fn midi_playback_field_overrides_midi_file() {
        let mp = MidiPlayback {
            file: "override.mid".to_string(),
            exclude_midi_channels: Some(vec![10]),
        };
        let song = Song::new(
            "test",
            None,
            Some("fallback.mid".to_string()),
            Some(mp),
            None,
            None,
            vec![Track::new("t".to_string(), "t.wav", None)],
            HashMap::new(),
            vec![],
        );
        let playback = song.midi_playback().unwrap();
        assert_eq!(playback.file(), "override.mid");
    }

    #[test]
    fn exclude_midi_channels_subtracts_one() {
        let mp = MidiPlayback {
            file: "test.mid".to_string(),
            exclude_midi_channels: Some(vec![1, 10, 16]),
        };
        let excluded = mp.exclude_midi_channels();
        assert_eq!(excluded, vec![0, 9, 15]);
    }

    #[test]
    fn exclude_midi_channels_empty_default() {
        let mp = MidiPlayback {
            file: "test.mid".to_string(),
            exclude_midi_channels: None,
        };
        assert!(mp.exclude_midi_channels().is_empty());
    }

    #[test]
    fn light_shows_none() {
        let song = minimal_song();
        assert!(song.light_shows().is_none());
    }

    #[test]
    fn light_shows_some() {
        let song = Song::new(
            "test",
            None,
            None,
            None,
            Some(vec![LightShow::new(
                "universe1".to_string(),
                "lights.mid".to_string(),
                Some(vec![10]),
            )]),
            None,
            vec![Track::new("t".to_string(), "t.wav", None)],
            HashMap::new(),
            vec![],
        );
        let shows = song.light_shows().unwrap();
        assert_eq!(shows.len(), 1);
        assert_eq!(shows[0].universe_name(), "universe1");
        assert_eq!(shows[0].dmx_file(), "lights.mid");
    }

    #[test]
    fn light_show_midi_channels_subtracts_one() {
        let ls = LightShow::new("u".to_string(), "f.mid".to_string(), Some(vec![1, 10]));
        assert_eq!(ls.midi_channels(), vec![0, 9]);
    }

    #[test]
    fn light_show_midi_channels_empty_default() {
        let ls = LightShow::new("u".to_string(), "f.mid".to_string(), None);
        assert!(ls.midi_channels().is_empty());
    }

    #[test]
    fn lighting_none() {
        let song = minimal_song();
        assert!(song.lighting().is_none());
    }

    #[test]
    fn lighting_some() {
        let song = Song::new(
            "test",
            None,
            None,
            None,
            None,
            Some(vec![LightingShow::new("show.light".to_string())]),
            vec![Track::new("t".to_string(), "t.wav", None)],
            HashMap::new(),
            vec![],
        );
        let lighting = song.lighting().unwrap();
        assert_eq!(lighting.len(), 1);
        assert_eq!(lighting[0].file(), "show.light");
    }

    #[test]
    fn samples_config_empty() {
        let song = minimal_song();
        let sc = song.samples_config();
        assert!(sc.samples().is_empty());
        assert!(sc.sample_triggers().is_empty());
    }

    #[test]
    fn serde_deserialize_minimal() {
        let yaml = r#"
            name: "Minimal Song"
            tracks:
              - name: track1
                file: track1.wav
        "#;
        let song: Song = config::Config::builder()
            .add_source(config::File::from_str(yaml, config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        assert_eq!(song.name(), "Minimal Song");
        assert_eq!(song.tracks().len(), 1);
        assert!(song.midi_playback().is_none());
        assert!(song.light_shows().is_none());
    }

    #[test]
    fn serde_deserialize_with_midi_playback() {
        let yaml = r#"
            name: "MIDI Song"
            tracks:
              - name: track1
                file: track1.wav
            midi_playback:
              file: song.mid
              exclude_midi_channels: [10, 16]
        "#;
        let song: Song = config::Config::builder()
            .add_source(config::File::from_str(yaml, config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        let mp = song.midi_playback().unwrap();
        assert_eq!(mp.file(), "song.mid");
        assert_eq!(mp.exclude_midi_channels(), vec![9, 15]);
    }

    #[test]
    fn save_creates_file() {
        let song = minimal_song();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("song.yaml");
        song.save(&path).unwrap();
        assert!(path.exists());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("Test Song"));
    }

    #[test]
    fn save_fails_on_invalid_path() {
        let song = minimal_song();
        let result = song.save(Path::new("/nonexistent/directory/song.yaml"));
        assert!(result.is_err());
    }
}
