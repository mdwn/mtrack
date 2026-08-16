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
    metronome::MetronomeConfig,
    midi::{self, ToMidiEvent},
    notification::SongNotificationConfig,
    pilot::PilotConfig,
    samples::{SampleDefinition, SampleTrigger, SamplesConfig},
    tempo::TempoConfig,
    track::Track,
};

/// A YAML representation of a song.
#[derive(Deserialize, Serialize)]
pub struct Song {
    /// Identifies this config file as a song.
    #[serde(default = "default_song_kind")]
    kind: super::kind::ConfigKind,
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
    /// Whether this song should loop when it finishes playing.
    #[serde(default)]
    loop_playback: bool,
    /// Named sections of the song, defined by measure boundaries.
    /// Used for section looping during playback.
    #[serde(default)]
    sections: Vec<Section>,
    /// The song's tempo map. When present, this is the canonical source of
    /// the beat grid, taking precedence over click track analysis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tempo: Option<TempoConfig>,
    /// The song's metronome: a generated click track, routable like any
    /// other track. Requires a beat grid (tempo map or click analysis).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metronome: Option<MetronomeConfig>,
    /// Voice pilot hints: labeled cues at song positions, with optional
    /// audio samples rendered onto a dedicated virtual track.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pilot: Option<PilotConfig>,
    /// Per-song notification audio overrides.
    #[serde(default)]
    notification_audio: Option<SongNotificationConfig>,
}

/// A named section of a song defined by measure boundaries.
#[derive(Deserialize, Serialize, Clone, Debug)]
pub struct Section {
    /// Display name for this section (e.g., "verse", "chorus").
    pub name: String,
    /// Start measure (1-indexed, inclusive).
    pub start_measure: usize,
    /// End measure (1-indexed, exclusive).
    pub end_measure: usize,
    /// Start beat within the start measure (1-based, fractional allowed;
    /// omitted = 1, the measure boundary).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_beat: Option<f64>,
    /// End beat within the end measure (1-based, fractional allowed;
    /// omitted = 1, keeping the end bound exclusive at the measure line).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_beat: Option<f64>,
    /// Optional display color for the web UI timeline (hex "#rrggbb").
    /// Without one the UI rotates through its palette.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
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
            kind: super::kind::ConfigKind::Song,
            name: name.to_string(),
            midi_event,
            midi_file,
            midi_playback,
            light_shows,
            lighting,
            tracks,
            samples,
            sample_triggers,
            loop_playback: false,
            sections: Vec::new(),
            tempo: None,
            metronome: None,
            pilot: None,
            notification_audio: None,
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

    /// Validates the song configuration for structural issues that can be
    /// caught without filesystem access. Call this before writing to disk.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errors = Vec::new();

        if self.name.trim().is_empty() {
            errors.push("song name must not be empty".to_string());
        }

        for (i, track) in self.tracks.iter().enumerate() {
            let label = if track.name().is_empty() {
                format!("track[{}]", i)
            } else {
                format!("track \"{}\"", track.name())
            };

            if track.name().trim().is_empty() {
                errors.push(format!("{}: name must not be empty", label));
            }
            if track.file().trim().is_empty() {
                errors.push(format!("{}: file must not be empty", label));
            }
            if let Some(ch) = track.file_channel() {
                if ch == 0 {
                    errors.push(format!(
                        "{}: file_channel must be 1 or greater (channels are 1-indexed)",
                        label
                    ));
                }
            }
        }

        for (i, section) in self.sections.iter().enumerate() {
            let label = if section.name.is_empty() {
                format!("section[{}]", i)
            } else {
                format!("section \"{}\"", section.name)
            };

            if section.name.trim().is_empty() {
                errors.push(format!("{}: name must not be empty", label));
            }
            if section.start_measure == 0 {
                errors.push(format!(
                    "{}: start_measure must be 1 or greater (measures are 1-indexed)",
                    label
                ));
            }
            if let Some(color) = &section.color {
                let valid = color.len() == 7
                    && color.starts_with('#')
                    && color[1..].chars().all(|c| c.is_ascii_hexdigit());
                if !valid {
                    errors.push(format!(
                        "{}: color must be a hex color like \"#5ecaea\", got {:?}",
                        label, color
                    ));
                }
            }
            for (field, beat, measure) in [
                ("start_beat", section.start_beat, section.start_measure),
                ("end_beat", section.end_beat, section.end_measure),
            ] {
                let Some(beat) = beat else { continue };
                if !beat.is_finite() || beat < 1.0 {
                    errors.push(format!(
                        "{}: {} must be 1 or greater (beats are 1-indexed within the measure), got {}",
                        label, field, beat
                    ));
                    continue;
                }
                // A beat past its measure's length is not a position in that
                // measure — it is the next measure's downbeat by another
                // name, and the ordering check below (which compares
                // (measure, beat) tuples) cannot see that. The beat grid is
                // not built yet here, but a `tempo:` block gives the meter,
                // which is where the grid's measure lengths come from too.
                // Without one the grid comes from click analysis and its
                // measures are only known at load, where `beat_time`
                // rejects the same position.
                let beats = self
                    .tempo
                    .as_ref()
                    .zip(u32::try_from(measure).ok())
                    .and_then(|(tempo, measure)| tempo.beats_in_measure(measure));
                if let Some(beats) = beats {
                    if beat >= f64::from(beats) + 1.0 {
                        errors.push(format!(
                            "{}: {} {} is past the end of measure {}, which has {} beats — use measure {} beat 1 for the next downbeat",
                            label, field, beat, measure, beats, measure + 1
                        ));
                    }
                }
            }
            // The end bound must come after the start; equal measures are
            // fine when the beats are ordered (a section within one measure).
            let start_beat = section.start_beat.unwrap_or(1.0);
            let end_beat = section.end_beat.unwrap_or(1.0);
            if section.end_measure < section.start_measure
                || (section.end_measure == section.start_measure && end_beat <= start_beat)
            {
                errors.push(format!(
                    "{}: the end position (measure {} beat {}) must come after the start (measure {} beat {})",
                    label, section.end_measure, end_beat, section.start_measure, start_beat
                ));
            }
        }

        if let Some(tempo) = &self.tempo {
            if let Err(err) = tempo.validate() {
                errors.push(format!("tempo: {}", err));
            }
        }

        if let Some(metronome) = &self.metronome {
            if let Err(err) = metronome.validate() {
                errors.push(format!("metronome: {}", err));
            }
            if self
                .tracks
                .iter()
                .any(|track| track.name() == metronome.track)
            {
                errors.push(format!(
                    "metronome: track name \"{}\" collides with an audio track",
                    metronome.track
                ));
            }
        }

        if let Some(pilot) = &self.pilot {
            if let Err(err) = pilot.validate() {
                errors.push(format!("pilot: {}", err));
            }
            if self.tracks.iter().any(|track| track.name() == pilot.track) {
                errors.push(format!(
                    "pilot: track name \"{}\" collides with an audio track",
                    pilot.track
                ));
            }
            if let Some(metronome) = &self.metronome {
                if metronome.track == pilot.track {
                    errors.push(format!(
                        "pilot: track name \"{}\" collides with the metronome track",
                        pilot.track
                    ));
                }
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
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
    pub fn light_shows(&self) -> Option<&[LightShow]> {
        self.light_shows.as_deref()
    }

    /// Gets the DSL lighting shows associated with the song.
    pub fn lighting(&self) -> Option<&[LightingShow]> {
        self.lighting.as_deref()
    }

    /// Gets the tracks associated with the song.
    pub fn tracks(&self) -> &[Track] {
        &self.tracks
    }

    /// Returns whether this song should loop when it finishes playing.
    pub fn loop_playback(&self) -> bool {
        self.loop_playback
    }

    /// Gets the named sections of this song.
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// Gets the song's tempo map configuration.
    pub fn tempo(&self) -> Option<&TempoConfig> {
        self.tempo.as_ref()
    }

    /// Gets the song's metronome configuration.
    pub fn metronome(&self) -> Option<&MetronomeConfig> {
        self.metronome.as_ref()
    }

    /// Gets the song's pilot hints configuration.
    pub fn pilot(&self) -> Option<&PilotConfig> {
        self.pilot.as_ref()
    }

    /// Gets the per-song notification audio overrides.
    pub fn notification_audio(&self) -> Option<&SongNotificationConfig> {
        self.notification_audio.as_ref()
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
    pub fn file(&self) -> &str {
        &self.file
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
    pub fn universe_name(&self) -> &str {
        &self.universe_name
    }

    /// Gets the DMX (MIDI) file associated with the light show.
    pub fn dmx_file(&self) -> &str {
        &self.dmx_file
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

fn default_song_kind() -> super::kind::ConfigKind {
    super::kind::ConfigKind::Song
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tempo::TempoChangeConfig;
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

    #[test]
    fn validate_ok_for_valid_song() {
        let song = minimal_song();
        assert!(song.validate().is_ok());
    }

    #[test]
    fn validate_rejects_empty_name() {
        let song = Song::new(
            "",
            None,
            None,
            None,
            None,
            None,
            vec![Track::new("t".to_string(), "t.wav", None)],
            HashMap::new(),
            vec![],
        );
        let errors = song.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("song name")));
    }

    #[test]
    fn validate_rejects_file_channel_zero() {
        let song = Song::new(
            "test",
            None,
            None,
            None,
            None,
            None,
            vec![Track::new("t".to_string(), "t.wav", Some(0))],
            HashMap::new(),
            vec![],
        );
        let errors = song.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("file_channel")));
    }

    #[test]
    fn validate_rejects_empty_track_name() {
        let song = Song::new(
            "test",
            None,
            None,
            None,
            None,
            None,
            vec![Track::new("".to_string(), "t.wav", None)],
            HashMap::new(),
            vec![],
        );
        let errors = song.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("name must not be empty")));
    }

    #[test]
    fn validate_rejects_empty_track_file() {
        let song = Song::new(
            "test",
            None,
            None,
            None,
            None,
            None,
            vec![Track::new("t".to_string(), "", None)],
            HashMap::new(),
            vec![],
        );
        let errors = song.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("file must not be empty")));
    }

    #[test]
    fn validate_collects_multiple_errors() {
        let song = Song::new(
            "",
            None,
            None,
            None,
            None,
            None,
            vec![Track::new("".to_string(), "", Some(0))],
            HashMap::new(),
            vec![],
        );
        let errors = song.validate().unwrap_err();
        assert!(
            errors.len() >= 4,
            "Expected at least 4 errors, got: {:?}",
            errors
        );
    }

    #[test]
    fn sections_default_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("song.yaml");
        std::fs::write(
            &path,
            "name: test\ntracks:\n  - name: t1\n    file: t1.wav\n",
        )
        .unwrap();
        let song = Song::deserialize(&path).unwrap();
        assert!(song.sections().is_empty());
    }

    #[test]
    fn sections_deserialize_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("song.yaml");
        std::fs::write(
            &path,
            "name: test\ntracks:\n  - name: t1\n    file: t1.wav\nsections:\n  - name: verse\n    start_measure: 1\n    end_measure: 8\n  - name: chorus\n    start_measure: 9\n    end_measure: 16\n",
        )
        .unwrap();
        let song = Song::deserialize(&path).unwrap();
        assert_eq!(song.sections().len(), 2);
        assert_eq!(song.sections()[0].name, "verse");
        assert_eq!(song.sections()[0].start_measure, 1);
        assert_eq!(song.sections()[0].end_measure, 8);
        assert_eq!(song.sections()[1].name, "chorus");
        assert_eq!(song.sections()[1].start_measure, 9);
        assert_eq!(song.sections()[1].end_measure, 16);
    }

    #[test]
    fn validate_rejects_section_start_measure_zero() {
        let mut song = minimal_song();
        song.sections = vec![Section {
            name: "bad".to_string(),
            start_measure: 0,
            end_measure: 4,
            start_beat: None,
            end_beat: None,
            color: None,
        }];
        let errors = song.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("start_measure must be 1")));
    }

    #[test]
    fn validate_rejects_section_end_not_greater_than_start() {
        let mut song = minimal_song();
        song.sections = vec![Section {
            name: "bad".to_string(),
            start_measure: 5,
            end_measure: 5,
            start_beat: None,
            end_beat: None,
            color: None,
        }];
        let errors = song.validate().unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.contains("must come after the start")));
    }

    #[test]
    fn validate_rejects_section_end_less_than_start() {
        let mut song = minimal_song();
        song.sections = vec![Section {
            name: "bad".to_string(),
            start_measure: 8,
            end_measure: 4,
            start_beat: None,
            end_beat: None,
            color: None,
        }];
        let errors = song.validate().unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.contains("must come after the start")));
    }

    #[test]
    fn validate_accepts_section_within_one_measure_via_beats() {
        let mut song = minimal_song();
        song.sections = vec![Section {
            name: "pickup".to_string(),
            start_measure: 5,
            end_measure: 5,
            start_beat: Some(2.0),
            end_beat: Some(4.5),
            color: None,
        }];
        assert!(song.validate().is_ok());
    }

    #[test]
    fn validate_rejects_section_same_position_beats() {
        let mut song = minimal_song();
        song.sections = vec![Section {
            name: "bad".to_string(),
            start_measure: 5,
            end_measure: 5,
            start_beat: Some(3.0),
            end_beat: Some(3.0),
            color: None,
        }];
        let errors = song.validate().unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.contains("must come after the start")));
    }

    #[test]
    fn validate_rejects_section_beat_below_one() {
        let mut song = minimal_song();
        song.sections = vec![Section {
            name: "bad".to_string(),
            start_measure: 1,
            end_measure: 4,
            start_beat: Some(0.5),
            end_beat: None,
            color: None,
        }];
        let errors = song.validate().unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.contains("start_beat must be 1 or greater")));
    }

    fn song_in(time_signature: &str) -> Song {
        let mut song = minimal_song();
        song.tempo = Some(TempoConfig {
            bpm: 120.0,
            time_signature: time_signature.to_string(),
            start: None,
            changes: vec![],
        });
        song
    }

    #[test]
    fn validate_rejects_section_beat_past_its_measure() {
        // Beat 5 of a 4/4 measure is the next downbeat wearing a disguise:
        // the grid would resolve it there, making an ordered pair of tuples
        // into a zero-length section.
        let mut song = song_in("4/4");
        song.sections = vec![Section {
            name: "spill".to_string(),
            start_measure: 1,
            end_measure: 2,
            start_beat: Some(5.0),
            end_beat: None,
            color: None,
        }];
        let errors = song.validate().unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.contains("start_beat 5 is past the end of measure 1, which has 4 beats")));
    }

    #[test]
    fn validate_accepts_a_beat_inside_its_measure() {
        // 4.5 is between the last beat and the next downbeat — still inside.
        let mut song = song_in("4/4");
        song.sections = vec![Section {
            name: "late".to_string(),
            start_measure: 1,
            end_measure: 2,
            start_beat: Some(4.5),
            end_beat: None,
            color: None,
        }];
        assert!(song.validate().is_ok());
    }

    #[test]
    fn validate_counts_beats_the_way_the_grid_does() {
        // 6/8 is six grid beats, not the three quarter-notes
        // `beats_per_measure` reports.
        let mut song = song_in("6/8");
        song.sections = vec![Section {
            name: "compound".to_string(),
            start_measure: 1,
            end_measure: 2,
            start_beat: Some(6.0),
            end_beat: None,
            color: None,
        }];
        assert!(song.validate().is_ok());

        song.sections[0].start_beat = Some(7.0);
        let errors = song.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("which has 6 beats")));
    }

    #[test]
    fn validate_uses_the_meter_in_effect_at_the_measure() {
        let mut song = song_in("4/4");
        song.tempo.as_mut().unwrap().changes = vec![TempoChangeConfig {
            measure: 9,
            beat: None,
            bpm: None,
            time_signature: Some("7/8".to_string()),
            transition: None,
        }];
        song.sections = vec![Section {
            name: "odd".to_string(),
            start_measure: 9,
            end_measure: 12,
            start_beat: Some(7.0),
            end_beat: None,
            color: None,
        }];
        assert!(song.validate().is_ok());

        // The same beat back in the 4/4 stretch is past the measure's end.
        song.sections[0].start_measure = 5;
        song.sections[0].end_measure = 8;
        let errors = song.validate().unwrap_err();
        assert!(errors
            .iter()
            .any(|e| e.contains("past the end of measure 5, which has 4 beats")));
    }

    #[test]
    fn validate_rejects_section_empty_name() {
        let mut song = minimal_song();
        song.sections = vec![Section {
            name: "".to_string(),
            start_measure: 1,
            end_measure: 4,
            start_beat: None,
            end_beat: None,
            color: None,
        }];
        let errors = song.validate().unwrap_err();
        assert!(errors.iter().any(|e| e.contains("name must not be empty")));
    }

    #[test]
    fn validate_accepts_valid_sections() {
        let mut song = minimal_song();
        song.sections = vec![
            Section {
                name: "verse".to_string(),
                start_measure: 1,
                end_measure: 8,
                start_beat: None,
                end_beat: None,
                color: None,
            },
            Section {
                name: "chorus".to_string(),
                start_measure: 9,
                end_measure: 16,
                start_beat: None,
                end_beat: None,
                color: None,
            },
        ];
        assert!(song.validate().is_ok());
    }

    #[test]
    fn loop_playback_deserializes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("song.yaml");
        std::fs::write(
            &path,
            "name: test\ntracks:\n  - name: t1\n    file: t1.wav\nloop_playback: true\n",
        )
        .unwrap();
        let song = Song::deserialize(&path).unwrap();
        assert!(song.loop_playback());
    }
}
