// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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
use std::fs;
use std::time::Duration;
use std::{error::Error, io::Write, path::Path};

use config::{Config, File};
use midly::live::LiveEvent;
use serde::{Deserialize, Serialize};
use serde_yml::Value;
use tracing::info;

use super::{
    midi::{self, ToMidiEvent},
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
}

impl Song {
    /// Creates a new song configuration.
    pub fn new(
        name: &str,
        midi_event: Option<midi::Event>,
        midi_file: Option<String>,
        midi_playback: Option<MidiPlayback>,
        light_shows: Option<Vec<LightShow>>,
        lighting: Option<Vec<LightingShow>>,
        tracks: Vec<Track>,
    ) -> Song {
        Song {
            name: name.to_string(),
            midi_event,
            midi_file,
            midi_playback,
            light_shows,
            lighting,
            tracks,
        }
    }

    /// Deserializes a file from the path into a song configuration struct.
    pub fn deserialize(path: &Path) -> Result<Song, Box<dyn Error>> {
        Ok(Config::builder()
            .add_source(File::from(path))
            .build()?
            .try_deserialize::<Song>()?)
    }

    /// Serialize and save a song configuration struct to a file at given path.
    pub fn save(&self, path: &Path) -> Result<(), Box<dyn Error>> {
        let serialized = serde_yml::to_string(self)?;
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
#[allow(dead_code)]
pub struct LightingShow {
    /// The name of the lighting show.
    name: String,
    /// The path to the DSL file.
    file: String,
}

impl LightingShow {
    /// Creates a new DSL lighting show.
    #[allow(dead_code)]
    pub fn new(name: String, file: String) -> Self {
        Self { name, file }
    }

    /// Gets the name of the lighting show.
    #[allow(dead_code)]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the DSL file path.
    #[allow(dead_code)]
    pub fn file(&self) -> &str {
        &self.file
    }
}

/// A YAML representation of lighting configuration for a song.
#[derive(Deserialize, Clone, Serialize)]
#[allow(dead_code)]
pub struct LightingConfiguration {
    /// The lighting cues for this song.
    cues: Vec<LightingCue>,
}

impl LightingConfiguration {
    /// Creates a new lighting configuration.
    #[allow(dead_code)]
    pub fn new(cues: Vec<LightingCue>) -> Self {
        Self { cues }
    }

    /// Gets the lighting cues.
    #[allow(dead_code)]
    pub fn cues(&self) -> &Vec<LightingCue> {
        &self.cues
    }
}

/// A lighting cue with timing and effects.
#[derive(Deserialize, Clone, Serialize)]
#[allow(dead_code)]
pub struct LightingCue {
    /// The time when this cue should trigger (MM:SS.mmm format).
    time: String,
    /// Optional description of this cue.
    description: Option<String>,
    /// The lighting effects to apply at this cue.
    effects: Vec<LightingEffect>,
}

impl LightingCue {
    /// Creates a new lighting cue.
    #[allow(dead_code)]
    pub fn new(time: String, description: Option<String>, effects: Vec<LightingEffect>) -> Self {
        Self {
            time,
            description,
            effects,
        }
    }

    /// Gets the time of this cue.
    #[allow(dead_code)]
    pub fn time(&self) -> &str {
        &self.time
    }

    /// Gets the description of this cue.
    #[allow(dead_code)]
    pub fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Gets the effects for this cue.
    #[allow(dead_code)]
    pub fn effects(&self) -> &Vec<LightingEffect> {
        &self.effects
    }
}

/// A lighting effect with groups and parameters.
#[derive(Deserialize, Clone, Serialize)]
#[allow(dead_code)]
pub struct LightingEffect {
    /// The type of effect (static, cycle, strobe, etc.).
    effect_type: String,
    /// The groups this effect applies to.
    groups: Vec<String>,
    /// The parameters for this effect.
    parameters: HashMap<String, Value>,
}

impl LightingEffect {
    /// Creates a new lighting effect.
    #[allow(dead_code)]
    pub fn new(
        effect_type: String,
        groups: Vec<String>,
        parameters: HashMap<String, Value>,
    ) -> Self {
        Self {
            effect_type,
            groups,
            parameters,
        }
    }

    /// Gets the effect type.
    #[allow(dead_code)]
    pub fn effect_type(&self) -> &str {
        &self.effect_type
    }

    /// Gets the groups this effect applies to.
    #[allow(dead_code)]
    pub fn groups(&self) -> &Vec<String> {
        &self.groups
    }

    /// Gets the parameters for this effect.
    #[allow(dead_code)]
    pub fn parameters(&self) -> &HashMap<String, Value> {
        &self.parameters
    }
}

/// Converts DSL light shows to YAML lighting configuration.
#[allow(dead_code)]
pub fn convert_dsl_to_lighting_configuration(
    dsl_shows: HashMap<String, crate::lighting::parser::LightShow>,
) -> Result<LightingConfiguration, Box<dyn Error>> {
    let mut all_cues = Vec::new();

    for (show_name, light_show) in dsl_shows {
        for cue in light_show.cues {
            let mut effects = Vec::new();

            for effect in cue.effects {
                let mut parameters = HashMap::new();

                // Convert effect parameters to YAML values
                for (key, value) in effect.parameters {
                    parameters.insert(key, Value::String(value));
                }

                let lighting_effect = LightingEffect::new(
                    match effect.effect_type {
                        crate::lighting::parser::EffectType::Static => "static".to_string(),
                        crate::lighting::parser::EffectType::Cycle => "cycle".to_string(),
                        crate::lighting::parser::EffectType::Strobe => "strobe".to_string(),
                        crate::lighting::parser::EffectType::Pulse => "pulse".to_string(),
                        crate::lighting::parser::EffectType::Chase => "chase".to_string(),
                        crate::lighting::parser::EffectType::Dimmer => "dimmer".to_string(),
                        crate::lighting::parser::EffectType::Rainbow => "rainbow".to_string(),
                    },
                    effect.groups,
                    parameters,
                );

                effects.push(lighting_effect);
            }

            // Convert Duration to MM:SS.mmm format
            let time_str = format_duration_as_time_string(cue.time);

            let lighting_cue =
                LightingCue::new(time_str, Some(format!("Cue from {}", show_name)), effects);

            all_cues.push(lighting_cue);
        }
    }

    // Sort cues by time
    all_cues.sort_by(|a, b| {
        let time_a = parse_time_string_to_duration(a.time()).unwrap_or(Duration::ZERO);
        let time_b = parse_time_string_to_duration(b.time()).unwrap_or(Duration::ZERO);
        time_a.cmp(&time_b)
    });

    Ok(LightingConfiguration::new(all_cues))
}

/// Formats a Duration as MM:SS.mmm time string.
#[allow(dead_code)]
fn format_duration_as_time_string(duration: Duration) -> String {
    let total_ms = duration.as_millis() as u64;
    let minutes = total_ms / (60 * 1000);
    let seconds = (total_ms % (60 * 1000)) / 1000;
    let milliseconds = total_ms % 1000;

    format!("{:02}:{:02}.{:03}", minutes, seconds, milliseconds)
}

/// Parses a time string (MM:SS.mmm) to Duration.
#[allow(dead_code)]
fn parse_time_string_to_duration(time_str: &str) -> Result<Duration, Box<dyn Error>> {
    let parts: Vec<&str> = time_str.split(':').collect();

    if parts.len() == 2 {
        // MM:SS.mmm format
        let minutes: u64 = parts[0].parse()?;
        let seconds_part = parts[1];
        let seconds_parts: Vec<&str> = seconds_part.split('.').collect();

        let seconds: u64 = seconds_parts[0].parse()?;
        let milliseconds: u64 = if seconds_parts.len() > 1 {
            let ms_str = seconds_parts[1];
            let ms_str = if ms_str.len() > 3 {
                &ms_str[..3]
            } else {
                ms_str
            };
            ms_str.parse::<u64>()? * 10_u64.pow(3 - ms_str.len() as u32)
        } else {
            0
        };

        Ok(Duration::from_millis(
            minutes * 60 * 1000 + seconds * 1000 + milliseconds,
        ))
    } else {
        // SS.mmm format
        let seconds_parts: Vec<&str> = time_str.split('.').collect();
        let seconds: u64 = seconds_parts[0].parse()?;
        let milliseconds: u64 = if seconds_parts.len() > 1 {
            let ms_str = seconds_parts[1];
            let ms_str = if ms_str.len() > 3 {
                &ms_str[..3]
            } else {
                ms_str
            };
            ms_str.parse::<u64>()? * 10_u64.pow(3 - ms_str.len() as u32)
        } else {
            0
        };

        Ok(Duration::from_millis(seconds * 1000 + milliseconds))
    }
}

/// Loads and parses DSL lighting files for a song.
#[allow(dead_code)]
pub fn load_dsl_lighting_files(
    song_path: &Path,
    dsl_file_paths: &[String],
) -> Result<LightingConfiguration, Box<dyn Error>> {
    let mut all_dsl_shows = HashMap::new();

    for dsl_file_path in dsl_file_paths {
        // Resolve relative paths relative to the song file
        let full_path = if dsl_file_path.starts_with('/') {
            Path::new(dsl_file_path).to_path_buf()
        } else {
            song_path
                .parent()
                .unwrap_or(Path::new(""))
                .join(dsl_file_path)
        };

        // Read and parse the DSL file
        let content = fs::read_to_string(&full_path)
            .map_err(|e| format!("Failed to read DSL file {}: {}", full_path.display(), e))?;

        let dsl_shows = crate::lighting::parser::parse_light_shows(&content)
            .map_err(|e| format!("Failed to parse DSL file {}: {}", full_path.display(), e))?;

        // Merge into the main collection
        for (name, show) in dsl_shows {
            all_dsl_shows.insert(name, show);
        }
    }

    // Convert to lighting configuration
    convert_dsl_to_lighting_configuration(all_dsl_shows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_dsl_to_lighting_configuration() {
        use crate::lighting::parser::{Cue, Effect, EffectType, LightShow};
        use std::collections::HashMap;
        use std::time::Duration;

        let mut dsl_shows = HashMap::new();

        let mut effects = Vec::new();
        let mut parameters = HashMap::new();
        parameters.insert("color".to_string(), "blue".to_string());
        parameters.insert("dimmer".to_string(), "60%".to_string());

        effects.push(Effect {
            groups: vec!["front_wash".to_string()],
            effect_type: EffectType::Static,
            parameters,
        });

        let cues = vec![Cue {
            time: Duration::from_millis(0),
            effects,
        }];

        let light_show = LightShow {
            name: "Test Show".to_string(),
            cues,
        };

        dsl_shows.insert("Test Show".to_string(), light_show);

        let result = convert_dsl_to_lighting_configuration(dsl_shows);
        assert!(result.is_ok());

        let config = result.unwrap();
        assert_eq!(config.cues().len(), 1);

        let cue = &config.cues()[0];
        assert_eq!(cue.time(), "00:00.000");
        assert_eq!(cue.effects().len(), 1);

        let effect = &cue.effects()[0];
        assert_eq!(effect.effect_type(), "static");
        assert_eq!(effect.groups(), &vec!["front_wash".to_string()]);
    }

    #[test]
    fn test_format_duration_as_time_string() {
        let duration = Duration::from_millis(90500);
        let result = format_duration_as_time_string(duration);
        assert_eq!(result, "01:30.500");

        let duration2 = Duration::from_millis(30500);
        let result2 = format_duration_as_time_string(duration2);
        assert_eq!(result2, "00:30.500");

        let duration3 = Duration::from_millis(0);
        let result3 = format_duration_as_time_string(duration3);
        assert_eq!(result3, "00:00.000");
    }

    #[test]
    fn test_parse_time_string_to_duration() {
        let result = parse_time_string_to_duration("01:30.500");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().as_millis(), 90500);

        let result2 = parse_time_string_to_duration("30.500");
        assert!(result2.is_ok());
        assert_eq!(result2.unwrap().as_millis(), 30500);

        let result3 = parse_time_string_to_duration("00:00.000");
        assert!(result3.is_ok());
        assert_eq!(result3.unwrap().as_millis(), 0);

        // Test invalid format
        let result4 = parse_time_string_to_duration("invalid");
        assert!(result4.is_err());
    }

    #[test]
    fn test_lighting_show_creation() {
        let show = LightingShow::new("Test Show".to_string(), "test.light".to_string());
        assert_eq!(show.name(), "Test Show");
        assert_eq!(show.file(), "test.light");
    }

    #[test]
    fn test_lighting_cue_creation() {
        let mut parameters = HashMap::new();
        parameters.insert(
            "color".to_string(),
            serde_yml::Value::String("blue".to_string()),
        );
        parameters.insert(
            "dimmer".to_string(),
            serde_yml::Value::String("60%".to_string()),
        );

        let effect = LightingEffect::new(
            "static".to_string(),
            vec!["front_wash".to_string()],
            parameters,
        );

        let cue = LightingCue::new(
            "00:00.000".to_string(),
            Some("Test cue".to_string()),
            vec![effect],
        );

        assert_eq!(cue.time(), "00:00.000");
        assert_eq!(cue.description(), Some("Test cue"));
        assert_eq!(cue.effects().len(), 1);
    }

    #[test]
    fn test_lighting_effect_creation() {
        let mut parameters = HashMap::new();
        parameters.insert(
            "color".to_string(),
            serde_yml::Value::String("blue".to_string()),
        );
        parameters.insert(
            "dimmer".to_string(),
            serde_yml::Value::String("60%".to_string()),
        );

        let effect = LightingEffect::new(
            "static".to_string(),
            vec!["front_wash".to_string()],
            parameters,
        );

        assert_eq!(effect.effect_type(), "static");
        assert_eq!(effect.groups(), &vec!["front_wash".to_string()]);
        assert_eq!(effect.parameters().len(), 2);
    }
}
