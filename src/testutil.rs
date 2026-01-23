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

#[cfg(test)]
use std::{
    error::Error,
    fs::File,
    path::PathBuf,
    thread,
    time::{Duration, SystemTime},
};

#[cfg(test)]
use hound::{SampleFormat, WavSpec, WavWriter};

/// Wait for the given predicate to return true or fail.
#[inline]
#[cfg(test)]
pub fn eventually<F>(predicate: F, error_msg: &str)
where
    F: Fn() -> bool,
{
    let start = SystemTime::now();
    let mut tick = Duration::from_millis(5); // Start with shorter interval
    let timeout = Duration::from_secs(10); // Increased timeout for complex operations
    let max_tick = Duration::from_millis(100); // Cap the polling interval

    loop {
        let elapsed = start.elapsed();
        if elapsed.is_err() {
            panic!("System time error");
        }
        let elapsed = elapsed.unwrap();

        if elapsed > timeout {
            panic!("{}", error_msg);
        }
        if predicate() {
            return;
        }

        // Exponential backoff to reduce CPU contention
        thread::sleep(tick);
        tick = std::cmp::min(tick * 2, max_tick);
    }
}

/// Wait for the given async predicate to return true or fail.
#[inline]
#[cfg(test)]
pub async fn eventually_async<F, Fut>(mut predicate: F, error_msg: &str)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = SystemTime::now();
    let tick = Duration::from_millis(10);
    let timeout = Duration::from_secs(3);

    loop {
        let elapsed = start.elapsed();
        if elapsed.is_err() {
            panic!("System time error");
        }
        let elapsed = elapsed.unwrap();

        if elapsed > timeout {
            panic!("{}", error_msg);
        }
        if predicate().await {
            return;
        }
        tokio::time::sleep(tick).await;
    }
}

#[cfg(test)]
pub fn write_wav<S: hound::Sample + Copy + 'static>(
    path: PathBuf,
    samples: Vec<Vec<S>>,
    sample_rate: u32,
) -> Result<(), Box<dyn Error>> {
    write_wav_with_bits(path, samples, sample_rate, 32)
}

#[cfg(test)]
pub fn write_wav_with_bits<S: hound::Sample + Copy + 'static>(
    path: PathBuf,
    samples: Vec<Vec<S>>,
    sample_rate: u32,
    bits_per_sample: u16,
) -> Result<(), Box<dyn Error>> {
    let tempwav = File::create(path)?;

    // Determine sample format based on the type
    let sample_format = if std::any::TypeId::of::<S>() == std::any::TypeId::of::<f32>() {
        SampleFormat::Float
    } else if std::any::TypeId::of::<S>() == std::any::TypeId::of::<i32>()
        || std::any::TypeId::of::<S>() == std::any::TypeId::of::<i16>()
    {
        SampleFormat::Int
    } else {
        return Err("Unsupported sample format".into());
    };

    let num_channels = samples.len();
    assert!(num_channels <= u16::MAX.into(), "Too many channels!");
    let mut writer = WavWriter::new(
        tempwav,
        WavSpec {
            channels: num_channels as u16,
            sample_rate,
            bits_per_sample,
            sample_format,
        },
    )?;

    // Write a simple set of samples to the wav file.
    for channel_samples in &samples {
        for sample in channel_samples {
            writer.write_sample(*sample)?;
        }
    }

    Ok(())
}

/// Audio test utilities for generating test signals and validating results
#[cfg(test)]
pub mod audio_test_utils {
    /// Calculate RMS (Root Mean Square) of a signal
    pub fn calculate_rms(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }

        let sum_squares: f32 = samples.iter().map(|&x| x * x).sum();
        (sum_squares / samples.len() as f32).sqrt()
    }

    /// Calculate Signal-to-Noise Ratio (SNR) in dB
    pub fn calculate_snr(original: &[f32], processed: &[f32]) -> f32 {
        if original.len() != processed.len() {
            return 0.0;
        }

        let signal_power = calculate_rms(original).powi(2);
        let noise_power = original
            .iter()
            .zip(processed.iter())
            .map(|(o, p)| (o - p).powi(2))
            .sum::<f32>()
            / original.len() as f32;

        if noise_power == 0.0 {
            return f32::INFINITY;
        }

        10.0 * (signal_power / noise_power).log10()
    }
}

/// Test utilities for parsing examples and verifying integration
#[cfg(test)]
pub mod examples {
    use std::path::PathBuf;

    /// Get the path to the examples directory
    pub fn examples_dir() -> PathBuf {
        PathBuf::from("examples")
    }

    /// Test that the main configuration file can be parsed
    #[test]
    fn test_parse_main_config() {
        // Test with a simple configuration instead of external file
        let config_content = r#"
audio:
  device: "test_device"
  sample_rate: 44100

midi:
  device: "test_midi"

dmx:
  dim_speed_modifier: 0.25
  playback_delay: "500ms"
  universes:
    - universe: 1
      name: "light-show"
  lighting:
    current_venue: "main_stage"
    groups:
      front_wash:
        name: "front_wash"
        constraints:
          - AllOf: ["wash", "front"]
          - MinCount: 4
          - MaxCount: 8
      back_wash:
        name: "back_wash"
        constraints:
          - AllOf: ["wash", "back"]
          - MinCount: 2
          - MaxCount: 6
    directories:
      fixture_types: "lighting/fixture_types"
      venues: "lighting/venues"

controllers: []

track_mappings:
  click: [1]
  cue: [2]

songs: "songs"
"#;

        // Create a temporary file for testing
        use std::io::Write;
        let mut temp_file =
            std::fs::File::create("test_config.yaml").expect("Failed to create temp file");
        temp_file
            .write_all(config_content.as_bytes())
            .expect("Failed to write config");
        drop(temp_file);

        let config = crate::config::Player::deserialize(std::path::Path::new("test_config.yaml"))
            .expect("Failed to parse main config");

        // Verify basic structure
        assert!(config.audio().is_some());
        assert!(config.midi().is_some());
        assert!(config.dmx().is_some());

        // Verify lighting configuration is present
        let dmx_config = config.dmx().unwrap();
        assert!(dmx_config.lighting().is_some());

        let lighting_config = dmx_config.lighting().unwrap();
        assert_eq!(lighting_config.current_venue(), Some("main_stage"));
        assert!(lighting_config.directories().is_some());

        // Verify groups are properly configured
        let groups = lighting_config.groups();
        assert!(groups.contains_key("front_wash"));
        assert!(groups.contains_key("back_wash"));

        // Clean up
        std::fs::remove_file("test_config.yaml").ok();
    }

    /// Test that songs with lighting can be parsed
    #[test]
    fn test_parse_songs_with_lighting() {
        // Test with a simple song configuration
        let song_config = crate::config::Song::new(
            "Test Song",
            None,
            None,
            None,
            None,
            Some(vec![
                crate::config::LightingShow::new("lighting/main_show.light".to_string()),
                crate::config::LightingShow::new("lighting/outro.light".to_string()),
            ]),
            vec![],
        );

        // Verify song has lighting shows
        assert!(song_config.lighting().is_some());
        let lighting_shows = song_config.lighting().unwrap();
        assert_eq!(lighting_shows.len(), 2);

        // Verify show files
        let show_files: Vec<&str> = lighting_shows.iter().map(|s| s.file()).collect();
        assert!(show_files.contains(&"lighting/main_show.light"));
        assert!(show_files.contains(&"lighting/outro.light"));

        // Verify show files
        let main_show = lighting_shows
            .iter()
            .find(|s| s.file() == "lighting/main_show.light")
            .unwrap();
        assert_eq!(main_show.file(), "lighting/main_show.light");

        let outro_show = lighting_shows
            .iter()
            .find(|s| s.file() == "lighting/outro.light")
            .unwrap();
        assert_eq!(outro_show.file(), "lighting/outro.light");
    }

    /// Test that DSL lighting shows can be parsed
    #[test]
    fn test_parse_dsl_lighting_shows() {
        // Test with a simple DSL first
        let simple_content = r#"show "Test Show" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
}"#;

        let simple_shows = crate::lighting::parser::parse_light_shows(simple_content)
            .expect("Failed to parse simple DSL");
        assert_eq!(simple_shows.len(), 1);

        let show = simple_shows.get("Test Show").expect("Test Show not found");
        assert_eq!(show.name, "Test Show");
        assert_eq!(show.cues.len(), 1);

        // Verify first cue
        let first_cue = &show.cues[0];
        assert_eq!(first_cue.time.as_secs(), 0);
        assert_eq!(first_cue.effects.len(), 1);

        let first_effect = &first_cue.effects[0];
        assert_eq!(first_effect.groups, vec!["front_wash"]);

        // Verify effect type is static
        match &first_effect.effect_type {
            crate::lighting::effects::EffectType::Static { parameters, .. } => {
                // Check if parameters contain the expected keys
                assert!(parameters.contains_key("dimmer"));
                assert!(
                    parameters.contains_key("red")
                        || parameters.contains_key("green")
                        || parameters.contains_key("blue")
                );
            }
            _ => panic!("Expected static effect"),
        }
    }

    /// Test that outro lighting show can be parsed
    #[test]
    fn test_parse_outro_lighting_show() {
        // Test with a simple outro show
        let simple_outro = r#"show "Outro Show" {
    @00:00.000
    all_fixtures: static color: "blue", dimmer: 20%
}"#;

        let shows = crate::lighting::parser::parse_light_shows(simple_outro)
            .expect("Failed to parse outro lighting show");
        assert_eq!(shows.len(), 1);

        let show = shows.get("Outro Show").expect("Outro Show not found");
        assert_eq!(show.name, "Outro Show");
        assert_eq!(show.cues.len(), 1);

        // Verify first cue
        let first_cue = &show.cues[0];
        assert_eq!(first_cue.time.as_secs(), 0);
        assert_eq!(first_cue.effects.len(), 1);

        let first_effect = &first_cue.effects[0];
        assert_eq!(first_effect.groups, vec!["all_fixtures"]);
    }

    /// Test complete integration flow
    #[test]
    fn test_complete_integration_flow() {
        // Load configuration
        let config_path = examples_dir().join("mtrack.yaml");
        let config =
            crate::config::Player::deserialize(&config_path).expect("Failed to parse config");

        // Load songs
        let songs_dir = examples_dir().join("songs");
        let songs = crate::songs::get_all_songs(&songs_dir).expect("Failed to load songs");

        // Find DSL light show song (if it exists)
        let songs_list = songs.list();
        let song_names: Vec<String> = songs_list.iter().map(|s| s.name().to_string()).collect();

        if song_names.contains(&"DSL Light Show Song".to_string()) {
            let dsl_song = songs
                .get(&"DSL Light Show Song".to_string())
                .expect("DSL light show song not found");

            // Verify song has lighting shows
            assert!(!dsl_song.dsl_lighting_shows().is_empty());
            let lighting_shows = dsl_song.dsl_lighting_shows();

            // Parse each lighting show
            for lighting_show in lighting_shows {
                let show_path = lighting_show.file_path();
                let content = std::fs::read_to_string(show_path).expect("Failed to read show file");
                let shows = crate::lighting::parser::parse_light_shows(&content)
                    .expect("Failed to parse show");
                assert!(!shows.is_empty());
            }
        }

        // Verify DMX configuration has lighting
        let dmx_config = config.dmx().unwrap();
        assert!(dmx_config.lighting().is_some());

        let lighting_config = dmx_config.lighting().unwrap();
        assert_eq!(lighting_config.current_venue(), Some("main_stage"));

        // Verify groups are configured
        let groups = lighting_config.groups();
        assert!(groups.contains_key("front_wash"));
        assert!(groups.contains_key("back_wash"));
        assert!(groups.contains_key("movers"));
        assert!(groups.contains_key("strobes"));
        assert!(groups.contains_key("all_lights"));

        // Verify directories are configured
        let directories = lighting_config.directories().unwrap();
        assert_eq!(directories.fixture_types(), Some("lighting/fixture_types"));
        assert_eq!(directories.venues(), Some("lighting/venues"));
    }

    /// Test that all example songs can be loaded
    #[test]
    fn test_all_example_songs_load() {
        let songs_dir = examples_dir().join("songs");
        let songs = crate::songs::get_all_songs(&songs_dir).expect("Failed to load songs");

        // Get all songs as a list
        let songs_list = songs.list();
        assert!(!songs_list.is_empty());

        // Debug: print actual song names
        let song_names: Vec<String> = songs_list.iter().map(|s| s.name().to_string()).collect();
        println!("Actual songs found: {:?}", song_names);

        // Verify we have some expected songs (not all may be present)
        let expected_songs = vec![
            "A really fast one",
            "Another cool song",
            "Outro tape",
            "Sound check",
        ];

        for expected_song in expected_songs {
            assert!(
                song_names.contains(&expected_song.to_string()),
                "Missing song: {}",
                expected_song
            );
        }

        // Verify DSL light show song has lighting (if it exists)
        if song_names.contains(&"DSL Light Show Song".to_string()) {
            let dsl_song = songs.get(&"DSL Light Show Song".to_string()).unwrap();
            assert!(!dsl_song.dsl_lighting_shows().is_empty());
        }

        // Verify other songs load without errors
        for song in songs_list {
            if song.name() != "DSL Light Show Song" {
                // Most songs don't have lighting, which is fine
                // We just want to make sure they load without errors
                assert!(!song.name().is_empty());
            }
        }
    }

    /// Test that playlist can be parsed
    #[test]
    fn test_parse_playlist() {
        let playlist_path = examples_dir().join("playlist.yaml");
        let playlist =
            crate::config::Playlist::deserialize(&playlist_path).expect("Failed to parse playlist");

        // Verify playlist structure
        assert!(!playlist.songs().is_empty());

        // Verify it contains some expected songs
        let songs = playlist.songs();
        assert!(songs.contains(&"A really cool song".to_string()));
        assert!(songs.contains(&"Sound check".to_string()));
    }

    /// Test that songs with lighting shows can be loaded and processed
    #[test]
    fn test_song_with_lighting_shows() {
        // Create a test song configuration with lighting shows
        let song_config = crate::config::Song::new(
            "Shieldbrother",
            None,
            None,
            None,
            None,
            Some(vec![crate::config::LightingShow::new(
                "show.light".to_string(),
            )]),
            vec![],
        );

        // Verify the song has lighting shows
        assert!(song_config.lighting().is_some());
        let lighting_shows = song_config.lighting().unwrap();
        assert_eq!(lighting_shows.len(), 1);

        // Verify the show file
        let show = &lighting_shows[0];
        assert_eq!(show.file(), "show.light");

        // Test that the DSL content can be parsed
        let dsl_content = r#"show "Shieldbrother" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 60%
    
    @00:05.000
    front_wash: static color: "red", dimmer: 80%
}"#;
        let shows = crate::lighting::parser::parse_light_shows(dsl_content).unwrap();
        assert_eq!(shows.len(), 1);
        assert!(shows.contains_key("Shieldbrother"));

        let show = shows.get("Shieldbrother").unwrap();
        assert_eq!(show.name, "Shieldbrother");
        assert_eq!(show.cues.len(), 2);
    }
}
