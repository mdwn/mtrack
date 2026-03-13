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

use std::collections::HashSet;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use tracing::info;

use crate::audio::format::SampleFormat;
use crate::calibrate;
use crate::config;
use crate::lighting::parser::parse_light_shows;
use crate::lighting::validation::validate_groups;
use crate::playlist::Playlist;
use crate::songs;
use crate::verify;

/// Default thread priority on the [0; 100) scale when MTRACK_THREAD_PRIORITY is unset.
/// On Linux (normal scheduling) this is roughly in the nice -8 to -10 range—higher than default 0 but below max.
const DEFAULT_THREAD_PRIORITY: u8 = 70;

/// Resolves the thread priority from an optional environment variable value.
/// Returns a value in [0, 100) or the default (70) if the input is None, unparseable, or out of range.
fn resolve_thread_priority(env_value: Option<&str>) -> u8 {
    env_value
        .and_then(|v| {
            let n = v.parse::<u8>().ok()?;
            (n < 100).then_some(n)
        })
        .unwrap_or(DEFAULT_THREAD_PRIORITY)
}

/// Sets the current thread's priority for better audio scheduling (cross-platform).
/// Uses the [0; 100) scale: higher value = higher priority.
/// Default is 70 when MTRACK_THREAD_PRIORITY is unset; set to 0-99 to override.
/// Fails silently if lacking permission. Use high values with care; they can starve other threads.
fn apply_thread_priority() {
    use thread_priority::{set_current_thread_priority, ThreadPriority, ThreadPriorityValue};

    let value = resolve_thread_priority(std::env::var("MTRACK_THREAD_PRIORITY").ok().as_deref());
    let priority = ThreadPriorityValue::try_from(value).unwrap();

    match set_current_thread_priority(ThreadPriority::Crossplatform(priority)) {
        Ok(()) => info!("Set thread priority for audio"),
        Err(e) => tracing::warn!(
            error = %e,
            "Could not set thread priority (e.g. run as root or with CAP_SYS_NICE on Linux)"
        ),
    }
}

pub fn songs(path: &str, init: bool) -> Result<(), Box<dyn Error>> {
    if init {
        info!("Initializing songs");
        songs::initialize_songs(Path::new(path))?;
    } else {
        info!("Not initializing songs");
    }

    let songs = songs::get_all_songs(Path::new(path))?;

    if songs.is_empty() {
        println!("No songs found in {}.", path);
        return Ok(());
    }

    let mut all_tracks: HashSet<String> = HashSet::new();
    println!("Songs (count: {}):", songs.len());
    for song in songs.sorted_list() {
        // Record all of the tracks found in the song repository.
        for track in song.tracks().iter() {
            all_tracks.insert(track.name().to_string());
        }

        println!("- {}", song);
    }

    // Sort the tracks so that the output is consistent.
    let mut tracks: Vec<String> = all_tracks.into_iter().collect();
    tracks.sort();

    println!("\nTracks (count: {}):", tracks.len());
    for track in tracks.iter() {
        println!("- {}", track)
    }

    Ok(())
}

pub fn playlist(repository_path: &str, playlist_path: &str) -> Result<(), Box<dyn Error>> {
    let songs = songs::get_all_songs(Path::new(repository_path))?;
    let playlist = Playlist::new(
        "playlist",
        &config::Playlist::deserialize(Path::new(playlist_path))?,
        songs,
    )?;

    println!("{}", playlist);
    Ok(())
}

pub async fn start(
    path: &str,
    playlist_path: Option<String>,
    web_config: crate::webui::server::WebConfig,
    tui_mode: bool,
) -> Result<(), Box<dyn Error>> {
    apply_thread_priority();

    // Resolve path: if it's a file or has a YAML extension, use it directly (legacy).
    // Otherwise treat it as a project directory and look for mtrack.yaml inside.
    let input = Path::new(path);
    let player_path =
        &if input.is_file() || input.extension().is_some_and(|e| e == "yaml" || e == "yml") {
            input.to_path_buf()
        } else {
            input.join("mtrack.yaml")
        };

    // Load or create config
    let player_config = match config::Player::deserialize(player_path) {
        Ok(cfg) => cfg,
        Err(e) => {
            // Check if this is a file-not-found error
            if player_path.exists() {
                return Err(e.into());
            }
            info!(
                "Config file not found at {:?}, creating with defaults",
                player_path
            );
            let mut default_config = config::Player::default();
            // If the project root already has discoverable songs, point songs
            // at "." instead of creating an empty "songs" subdirectory.
            if let Some(parent) = player_path.parent() {
                if !parent.exists() {
                    std::fs::create_dir_all(parent)?;
                }
                if !songs::get_all_songs(parent)
                    .map(|s| s.is_empty())
                    .unwrap_or(true)
                {
                    default_config.set_songs(".");
                }
            }
            let yaml = crate::util::to_yaml_string(&default_config)?;
            std::fs::write(player_path, &yaml)?;
            default_config
        }
    };

    let config_store = Arc::new(config::ConfigStore::new(
        player_config.clone(),
        player_path.to_path_buf(),
    ));

    // Ensure songs directory exists
    let songs_path = player_config.songs(player_path);
    if !songs_path.exists() {
        info!("Creating songs directory at {:?}", songs_path);
        std::fs::create_dir_all(&songs_path)?;
    }

    let songs = songs::get_all_songs(&songs_path)?;

    // Resolve playlist — gracefully handle missing
    let mut playlist_path = playlist_path
        .as_ref()
        .map(PathBuf::from)
        .or(player_config.playlist());

    // Make playlist path absolute if relative
    if let Some(ref mut pp) = playlist_path {
        if !pp.is_absolute() {
            *pp = if let Some(parent) = player_path.parent() {
                parent.join(&pp)
            } else {
                return Err(format!(
                    "Unable to determine playlist path (config base path has no parent): {}. \
                     Please specify the playlist path using an absolute path.",
                    pp.display()
                )
                .into());
            };
        }
    }

    let playlist = if let Some(ref pp) = playlist_path {
        match config::Playlist::deserialize(pp.as_path()) {
            Ok(playlist_config) => {
                match Playlist::new("playlist", &playlist_config, songs.clone()) {
                    Ok(pl) => pl,
                    Err(e) => {
                        info!("Playlist references missing songs ({}); using all songs", e);
                        crate::playlist::from_songs(songs.clone())?
                    }
                }
            }
            Err(_) => {
                info!("Playlist file not found; using all songs");
                crate::playlist::from_songs(songs.clone())?
            }
        }
    } else {
        info!("No playlist configured; using all songs");
        crate::playlist::from_songs(songs.clone())?
    };

    // Default playlist_path for web UI (so config writes have somewhere to go)
    let playlist_path = playlist_path.unwrap_or_else(|| {
        player_path
            .parent()
            .unwrap_or(Path::new("."))
            .join("playlist.yaml")
    });

    let player = Arc::new(crate::player::Player::new(
        songs,
        playlist,
        &player_config,
        player_path.parent(),
    )?);
    player.set_config_store(config_store);

    // Create the state watch channel upfront. The sampler will be started
    // by init_hardware_async when the DMX engine becomes available.
    let (state_tx, state_rx) =
        tokio::sync::watch::channel(std::sync::Arc::new(crate::state::StateSnapshot::default()));
    player.set_state_tx(state_tx);

    // Start the unified web server (dashboard + gRPC-Web + REST API)
    let webui_handle = {
        // Create a broadcast channel for the web UI (shared by dashboard WS and DMX file watcher)
        let (broadcast_tx, _) = tokio::sync::broadcast::channel::<String>(128);

        // Store the broadcast channel on the player so async init can wire
        // it to the DMX engine when it comes up.
        player.set_broadcast_tx(broadcast_tx.clone());

        let webui_state = crate::webui::server::WebUiState {
            player: player.clone(),
            state_rx: state_rx.clone(),
            broadcast_tx,
            config_path: player_path.to_path_buf(),
            songs_path: player_config.songs(player_path),
            playlist_path: playlist_path.clone(),
            waveform_cache: crate::webui::state::new_waveform_cache(),
            calibration: std::sync::Arc::new(parking_lot::Mutex::new(None)),
        };

        match crate::webui::server::start(webui_state, web_config.address.clone(), web_config.port)
            .await
        {
            Ok(handle) => Some(handle),
            Err(e) => {
                tracing::warn!("Failed to start web UI: {}", e);
                None
            }
        }
    };

    let hostname = config::resolve_hostname();
    let controllers = player_config
        .profiles(&hostname)
        .first()
        .map(|p| p.controllers().to_vec())
        .unwrap_or_default();
    let controller = crate::controller::Controller::new(controllers, player.clone())?;

    if tui_mode {
        crate::tui::run(player, controller, state_rx).await?;
    } else {
        controller.join().await?;
        // If no controllers are configured (e.g. zero-config start), keep
        // the process alive so the web UI remains accessible.
        if webui_handle.is_some() {
            info!("No controllers configured; web UI is running. Press Ctrl+C to stop.");
            std::future::pending::<()>().await;
        }
    }

    Ok(())
}

pub fn calibrate_triggers(
    device: &str,
    sample_rate: Option<u32>,
    duration: f32,
    sample_format: Option<String>,
    bits_per_sample: Option<u16>,
) -> Result<(), Box<dyn Error>> {
    if duration <= 0.0 || !duration.is_finite() {
        return Err("--duration must be a positive finite number".into());
    }

    let fmt = sample_format
        .as_deref()
        .map(SampleFormat::from_str)
        .transpose()?;

    calibrate::run(calibrate::CalibrationConfig {
        device_name: device.to_string(),
        sample_rate,
        noise_floor_duration_secs: duration,
        sample_format: fmt,
        bits_per_sample,
    })
}

pub fn verify_light_show(show_path: &str, config_path: Option<&str>) -> Result<(), Box<dyn Error>> {
    let path = Path::new(show_path);

    if !path.exists() {
        return Err(format!("Light show file not found: {}", show_path).into());
    }

    // Read and parse the light show
    let content = std::fs::read_to_string(path)?;
    let shows = match parse_light_shows(&content) {
        Ok(shows) => shows,
        Err(e) => {
            eprintln!("❌ Syntax error in light show:");
            eprintln!("{}", e);
            return Err(e);
        }
    };

    if shows.is_empty() {
        eprintln!("⚠️  Warning: No shows found in file");
        return Ok(());
    }

    println!("✅ Light show syntax is valid");
    println!("   Found {} show(s):", shows.len());
    for (name, show) in &shows {
        println!("   - \"{}\" ({} cues)", name, show.cues.len());
    }

    // Get lighting config if provided
    let (lighting_config, valid_groups_count, valid_fixtures_count) = if let Some(config_path) =
        config_path
    {
        let config_file = Path::new(config_path);
        if !config_file.exists() {
            eprintln!("⚠️  Warning: Config file not found: {}", config_path);
            (None, 0, 0)
        } else {
            match config::Player::deserialize(config_file) {
                Ok(player_config) => {
                    if let Some(dmx) = player_config.dmx() {
                        if let Some(lighting) = dmx.lighting() {
                            let groups_count = lighting.groups().len();
                            let fixtures_count = lighting.fixtures().len();
                            // Clone the lighting config to own it
                            (Some(lighting.clone()), groups_count, fixtures_count)
                        } else {
                            eprintln!("⚠️  Warning: No lighting configuration found in DMX config");
                            (None, 0, 0)
                        }
                    } else {
                        eprintln!("⚠️  Warning: No DMX configuration found in config file");
                        (None, 0, 0)
                    }
                }
                Err(e) => {
                    eprintln!("⚠️  Warning: Failed to parse config file: {}", e);
                    (None, 0, 0)
                }
            }
        }
    } else {
        (None, 0, 0)
    };

    // Use validation module to check groups
    let validation_result = validate_groups(&shows, lighting_config.as_ref());

    if validation_result.groups.is_empty() {
        println!("⚠️  Warning: No fixture groups found in show");
    } else {
        println!("\n   Groups used in show:");
        let mut sorted_groups: Vec<String> = validation_result.groups.iter().cloned().collect();
        sorted_groups.sort();
        for group in &sorted_groups {
            println!("   - {}", group);
        }
    }

    // Report validation results
    if lighting_config.is_some() {
        if !validation_result.is_valid() {
            eprintln!("\n❌ Invalid groups/fixtures referenced in show:");
            for group in &validation_result.invalid_groups {
                eprintln!("   - {} (not found in config)", group);
            }
            return Err(format!(
                "Show references {} invalid group(s)",
                validation_result.invalid_groups.len()
            )
            .into());
        } else {
            println!("\n✅ All groups/fixtures are valid in config");
            println!(
                "   Validated against {} group(s) and {} fixture(s) in config",
                valid_groups_count, valid_fixtures_count
            );
        }
    }

    Ok(())
}

pub fn verify(
    config: &str,
    check: Option<Vec<String>>,
    hostname: Option<String>,
) -> Result<(), Box<dyn Error>> {
    let config_path = Path::new(config);
    let player_config = config::Player::deserialize(config_path)?;
    let songs_path = player_config.songs(config_path);
    let songs = songs::get_all_songs(&songs_path)?;

    if songs.is_empty() {
        println!("No songs found in {}.", songs_path.display());
        return Ok(());
    }

    let run_all = check.is_none();
    let checks: Vec<String> = check.unwrap_or_default();

    let mut report = verify::VerificationReport::default();

    // Track mapping checks.
    if run_all || checks.iter().any(|c| c == "track-mappings") {
        let all_profiles = player_config.all_profiles();

        if all_profiles.len() > 1 {
            // Profile mode: verify each profile's track mappings.
            let profiles_to_check: Vec<&config::Profile> = match &hostname {
                Some(h) => {
                    let filtered = player_config.profiles(h);
                    if filtered.is_empty() {
                        eprintln!("Warning: no profiles match hostname '{}'", h);
                    }
                    filtered
                }
                None => all_profiles.iter().collect(),
            };

            for (i, profile) in profiles_to_check.iter().enumerate() {
                let audio_config = match profile.audio_config() {
                    Some(ac) => ac,
                    None => {
                        println!("Skipping profile {} (no audio configured)", i);
                        continue;
                    }
                };
                let label = match profile.hostname() {
                    Some(h) => format!(
                        "profile {} (hostname: {}, device: {})",
                        i,
                        h,
                        audio_config.audio().device()
                    ),
                    None => format!(
                        "profile {} (any host, device: {})",
                        i,
                        audio_config.audio().device()
                    ),
                };
                println!("Checking track mappings for {}...", label);
                let track_report =
                    verify::check_all_track_mappings(&songs, &audio_config.track_mappings_hash());
                report.merge(track_report);
            }
        } else {
            // Single profile (legacy or single profile): verify as before.
            let track_report =
                verify::check_all_track_mappings(&songs, &player_config.track_mappings());
            report.merge(track_report);
        }
    }

    verify::print_report(&report, &songs);

    if report.has_errors() {
        std::process::exit(1);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    mod resolve_thread_priority_tests {
        use super::*;

        #[test]
        fn none_returns_default() {
            assert_eq!(resolve_thread_priority(None), DEFAULT_THREAD_PRIORITY);
        }

        #[test]
        fn valid_value() {
            assert_eq!(resolve_thread_priority(Some("50")), 50);
        }

        #[test]
        fn zero_is_valid() {
            assert_eq!(resolve_thread_priority(Some("0")), 0);
        }

        #[test]
        fn ninety_nine_is_valid() {
            assert_eq!(resolve_thread_priority(Some("99")), 99);
        }

        #[test]
        fn one_hundred_is_out_of_range() {
            assert_eq!(
                resolve_thread_priority(Some("100")),
                DEFAULT_THREAD_PRIORITY
            );
        }

        #[test]
        fn large_value_out_of_range() {
            assert_eq!(
                resolve_thread_priority(Some("255")),
                DEFAULT_THREAD_PRIORITY
            );
        }

        #[test]
        fn negative_value_unparseable() {
            assert_eq!(resolve_thread_priority(Some("-1")), DEFAULT_THREAD_PRIORITY);
        }

        #[test]
        fn non_numeric_returns_default() {
            assert_eq!(
                resolve_thread_priority(Some("high")),
                DEFAULT_THREAD_PRIORITY
            );
        }

        #[test]
        fn empty_string_returns_default() {
            assert_eq!(resolve_thread_priority(Some("")), DEFAULT_THREAD_PRIORITY);
        }

        #[test]
        fn boundary_value_one() {
            assert_eq!(resolve_thread_priority(Some("1")), 1);
        }
    }

    mod songs_tests {
        use super::*;

        #[test]
        fn empty_directory_reports_no_songs() {
            let tmp = tempfile::tempdir().unwrap();
            assert!(songs(tmp.path().to_str().unwrap(), false).is_ok());
        }

        #[test]
        fn lists_songs_with_tracks() {
            let tmp = tempfile::tempdir().unwrap();
            let song_dir = tmp.path().join("My Song");
            std::fs::create_dir(&song_dir).unwrap();
            crate::testutil::write_wav(
                song_dir.join("guitar.wav"),
                vec![vec![1_i32, 2, 3, 4, 5]],
                44100,
            )
            .unwrap();
            crate::testutil::write_wav(
                song_dir.join("bass.wav"),
                vec![vec![1_i32, 2, 3, 4, 5]],
                44100,
            )
            .unwrap();
            assert!(songs(tmp.path().to_str().unwrap(), false).is_ok());
        }

        #[test]
        fn with_init_creates_yaml() {
            let tmp = tempfile::tempdir().unwrap();
            let song_dir = tmp.path().join("Init Song");
            std::fs::create_dir(&song_dir).unwrap();
            crate::testutil::write_wav(
                song_dir.join("track.wav"),
                vec![vec![1_i32, 2, 3, 4, 5]],
                44100,
            )
            .unwrap();
            assert!(songs(tmp.path().to_str().unwrap(), true).is_ok());
            // init should have created a song.yaml
            assert!(song_dir.join("song.yaml").exists());
        }

        #[test]
        fn nonexistent_path_returns_error() {
            assert!(songs("/nonexistent/path/to/songs", false).is_err());
        }

        #[test]
        fn multiple_songs_with_overlapping_tracks() {
            let tmp = tempfile::tempdir().unwrap();
            // Create two songs that share track names
            let song1_dir = tmp.path().join("Song One");
            std::fs::create_dir(&song1_dir).unwrap();
            crate::testutil::write_wav(
                song1_dir.join("guitar.wav"),
                vec![vec![1_i32, 2, 3, 4, 5]],
                44100,
            )
            .unwrap();
            crate::testutil::write_wav(
                song1_dir.join("bass.wav"),
                vec![vec![1_i32, 2, 3, 4, 5]],
                44100,
            )
            .unwrap();

            let song2_dir = tmp.path().join("Song Two");
            std::fs::create_dir(&song2_dir).unwrap();
            crate::testutil::write_wav(
                song2_dir.join("guitar.wav"),
                vec![vec![1_i32, 2, 3, 4, 5]],
                44100,
            )
            .unwrap();
            crate::testutil::write_wav(
                song2_dir.join("drums.wav"),
                vec![vec![1_i32, 2, 3, 4, 5]],
                44100,
            )
            .unwrap();

            // This exercises the track dedup via HashSet and sorting
            assert!(songs(tmp.path().to_str().unwrap(), false).is_ok());
        }

        #[test]
        fn init_with_multiple_songs() {
            let tmp = tempfile::tempdir().unwrap();
            let song1_dir = tmp.path().join("Song A");
            std::fs::create_dir(&song1_dir).unwrap();
            crate::testutil::write_wav(
                song1_dir.join("track1.wav"),
                vec![vec![1_i32, 2, 3, 4, 5]],
                44100,
            )
            .unwrap();

            let song2_dir = tmp.path().join("Song B");
            std::fs::create_dir(&song2_dir).unwrap();
            crate::testutil::write_wav(
                song2_dir.join("track2.wav"),
                vec![vec![1_i32, 2, 3, 4, 5]],
                44100,
            )
            .unwrap();

            assert!(songs(tmp.path().to_str().unwrap(), true).is_ok());
            assert!(song1_dir.join("song.yaml").exists());
            assert!(song2_dir.join("song.yaml").exists());
        }
    }

    mod playlist_tests {
        use super::*;

        #[test]
        fn valid_playlist() {
            let tmp = tempfile::tempdir().unwrap();
            let song_dir = tmp.path().join("Cool Song");
            std::fs::create_dir(&song_dir).unwrap();
            crate::testutil::write_wav(
                song_dir.join("track.wav"),
                vec![vec![1_i32, 2, 3, 4, 5]],
                44100,
            )
            .unwrap();
            crate::songs::initialize_songs(tmp.path()).unwrap();

            let playlist_path = tmp.path().join("playlist.yaml");
            std::fs::write(&playlist_path, "songs:\n- Cool Song\n").unwrap();

            assert!(playlist(
                tmp.path().to_str().unwrap(),
                playlist_path.to_str().unwrap()
            )
            .is_ok());
        }

        #[test]
        fn invalid_playlist_path() {
            let tmp = tempfile::tempdir().unwrap();
            assert!(playlist(tmp.path().to_str().unwrap(), "/nonexistent/playlist.yaml").is_err());
        }
    }

    mod calibrate_triggers_tests {
        use super::*;

        #[test]
        fn negative_duration_returns_error() {
            let result = calibrate_triggers("device", None, -1.0, None, None);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("positive finite"));
        }

        #[test]
        fn zero_duration_returns_error() {
            let result = calibrate_triggers("device", None, 0.0, None, None);
            assert!(result.is_err());
        }

        #[test]
        fn infinite_duration_returns_error() {
            let result = calibrate_triggers("device", None, f32::INFINITY, None, None);
            assert!(result.is_err());
        }

        #[test]
        fn nan_duration_returns_error() {
            let result = calibrate_triggers("device", None, f32::NAN, None, None);
            assert!(result.is_err());
        }

        #[test]
        fn invalid_sample_format_returns_error() {
            let result = calibrate_triggers("device", None, 3.0, Some("invalid".to_string()), None);
            assert!(result.is_err());
        }

        #[test]
        fn valid_int_sample_format_passes_validation() {
            // The sample format "int" is valid, but will fail at calibrate::run
            // because the device doesn't exist. This exercises the fmt parsing path.
            let result = calibrate_triggers(
                "nonexistent-device",
                None,
                3.0,
                Some("int".to_string()),
                None,
            );
            assert!(result.is_err());
            // The error should NOT be about sample format parsing
            let err_msg = result.unwrap_err().to_string();
            assert!(!err_msg.contains("Unsupported sample format"));
        }

        #[test]
        fn valid_float_sample_format_passes_validation() {
            let result = calibrate_triggers(
                "nonexistent-device",
                None,
                3.0,
                Some("float".to_string()),
                None,
            );
            assert!(result.is_err());
            let err_msg = result.unwrap_err().to_string();
            assert!(!err_msg.contains("Unsupported sample format"));
        }

        #[test]
        fn neg_infinity_duration_returns_error() {
            let result = calibrate_triggers("device", None, f32::NEG_INFINITY, None, None);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("positive finite"));
        }

        #[test]
        fn with_sample_rate_and_bits_per_sample() {
            // Valid parameters but nonexistent device
            let result = calibrate_triggers(
                "nonexistent-device",
                Some(48000),
                3.0,
                Some("int".to_string()),
                Some(16),
            );
            assert!(result.is_err());
        }
    }

    mod verify_light_show_tests {
        use super::*;

        #[test]
        fn nonexistent_file_returns_error() {
            let result = verify_light_show("/nonexistent/show.light", None);
            assert!(result.is_err());
            assert!(result.unwrap_err().to_string().contains("not found"));
        }

        #[test]
        fn valid_show_file() {
            let tmp = tempfile::tempdir().unwrap();
            let show_path = tmp.path().join("show.light");
            std::fs::write(
                &show_path,
                r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
}"#,
            )
            .unwrap();
            assert!(verify_light_show(show_path.to_str().unwrap(), None).is_ok());
        }

        #[test]
        fn invalid_syntax_returns_error() {
            let tmp = tempfile::tempdir().unwrap();
            let show_path = tmp.path().join("bad.light");
            std::fs::write(&show_path, "this is not valid light show syntax {{{").unwrap();
            assert!(verify_light_show(show_path.to_str().unwrap(), None).is_err());
        }

        #[test]
        fn empty_show_file() {
            let tmp = tempfile::tempdir().unwrap();
            let show_path = tmp.path().join("empty.light");
            std::fs::write(&show_path, "").unwrap();
            // Empty file should parse but produce a warning about no shows
            assert!(verify_light_show(show_path.to_str().unwrap(), None).is_ok());
        }

        #[test]
        fn with_valid_config() {
            let show_dir = tempfile::tempdir().unwrap();
            let show_path = show_dir.path().join("show.light");
            std::fs::write(
                &show_path,
                r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
}"#,
            )
            .unwrap();

            // Use the example config which has DMX/lighting setup
            let config_path = PathBuf::from("examples/mtrack.yaml");
            if config_path.exists() {
                let result = verify_light_show(
                    show_path.to_str().unwrap(),
                    Some(config_path.to_str().unwrap()),
                );
                // Result depends on whether the groups match the config; either way it shouldn't panic
                let _ = result;
            }
        }

        #[test]
        fn with_nonexistent_config() {
            let show_dir = tempfile::tempdir().unwrap();
            let show_path = show_dir.path().join("show.light");
            std::fs::write(
                &show_path,
                r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
}"#,
            )
            .unwrap();

            // Non-existent config should produce a warning but not fail
            assert!(verify_light_show(
                show_path.to_str().unwrap(),
                Some("/nonexistent/mtrack.yaml")
            )
            .is_ok());
        }

        #[test]
        fn with_config_missing_dmx() {
            let tmp = tempfile::tempdir().unwrap();
            let show_path = tmp.path().join("show.light");
            std::fs::write(
                &show_path,
                r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
}"#,
            )
            .unwrap();

            // Config with no DMX section
            let config_path = tmp.path().join("mtrack.yaml");
            std::fs::write(&config_path, "songs: songs\naudio:\n  device: mock\n").unwrap();

            assert!(verify_light_show(
                show_path.to_str().unwrap(),
                Some(config_path.to_str().unwrap())
            )
            .is_ok());
        }

        #[test]
        fn multiple_shows_in_file() {
            let tmp = tempfile::tempdir().unwrap();
            let show_path = tmp.path().join("multi.light");
            std::fs::write(
                &show_path,
                r#"show "show1" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
}

show "show2" {
    @00:00.000
    rear_wash: static color: "red", dimmer: 50%
}"#,
            )
            .unwrap();
            assert!(verify_light_show(show_path.to_str().unwrap(), None).is_ok());
        }

        #[test]
        fn with_config_dmx_but_no_lighting_section() {
            let tmp = tempfile::tempdir().unwrap();
            let show_path = tmp.path().join("show.light");
            std::fs::write(
                &show_path,
                r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
}"#,
            )
            .unwrap();

            // Config with DMX but no lighting section
            let config_path = tmp.path().join("mtrack.yaml");
            std::fs::write(
                &config_path,
                "songs: songs\naudio:\n  device: mock\ndmx:\n  universes:\n  - universe: 1\n    name: light-show\n",
            )
            .unwrap();

            assert!(verify_light_show(
                show_path.to_str().unwrap(),
                Some(config_path.to_str().unwrap())
            )
            .is_ok());
        }

        #[test]
        fn with_config_having_lighting_and_valid_groups() {
            let tmp = tempfile::tempdir().unwrap();
            let show_path = tmp.path().join("show.light");
            // Use a group name that matches the config
            std::fs::write(
                &show_path,
                r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
}"#,
            )
            .unwrap();

            let config_path = tmp.path().join("mtrack.yaml");
            std::fs::write(
                &config_path,
                r#"songs: songs
audio:
  device: mock
dmx:
  universes:
    - universe: 1
      name: light-show
  lighting:
    fixtures:
      front_wash: "Front Wash @ 1:1"
    groups:
      front_wash:
        name: front_wash
        constraints:
          - AllOf: ["wash"]
          - AllowEmpty: true
"#,
            )
            .unwrap();

            let result = verify_light_show(
                show_path.to_str().unwrap(),
                Some(config_path.to_str().unwrap()),
            );
            // Result depends on validation details, but should not panic
            let _ = result;
        }

        #[test]
        fn with_invalid_config_file_syntax() {
            let tmp = tempfile::tempdir().unwrap();
            let show_path = tmp.path().join("show.light");
            std::fs::write(
                &show_path,
                r#"show "test" {
    @00:00.000
    front_wash: static color: "blue", dimmer: 100%
}"#,
            )
            .unwrap();

            // Write a config file with invalid YAML
            let config_path = tmp.path().join("mtrack.yaml");
            std::fs::write(&config_path, "{{invalid yaml!!").unwrap();

            // Should produce a warning about parsing, but not crash
            assert!(verify_light_show(
                show_path.to_str().unwrap(),
                Some(config_path.to_str().unwrap()),
            )
            .is_ok());
        }

        #[test]
        fn show_with_no_groups() {
            let tmp = tempfile::tempdir().unwrap();
            let show_path = tmp.path().join("show.light");
            // A show with a cue but no fixture group references
            std::fs::write(
                &show_path,
                r#"show "empty_show" {
    @00:00.000
}"#,
            )
            .unwrap();
            assert!(verify_light_show(show_path.to_str().unwrap(), None).is_ok());
        }
    }

    mod verify_tests {
        use super::*;

        #[test]
        fn empty_songs_dir() {
            let tmp = tempfile::tempdir().unwrap();
            let songs_dir = tmp.path().join("songs");
            std::fs::create_dir(&songs_dir).unwrap();
            let config_path = tmp.path().join("mtrack.yaml");
            std::fs::write(&config_path, format!("songs: {}\n", songs_dir.display())).unwrap();
            assert!(verify(config_path.to_str().unwrap(), None, None).is_ok());
        }

        #[test]
        fn invalid_config_path() {
            assert!(verify("/nonexistent/mtrack.yaml", None, None).is_err());
        }

        /// Helper: create a songs dir with one song having a given set of track names.
        fn create_songs_dir(base: &Path, song_name: &str, track_names: &[&str]) -> PathBuf {
            let songs_dir = base.join("songs");
            std::fs::create_dir_all(&songs_dir).unwrap();
            let song_dir = songs_dir.join(song_name);
            std::fs::create_dir_all(&song_dir).unwrap();
            for track in track_names {
                crate::testutil::write_wav(
                    song_dir.join(format!("{}.wav", track)),
                    vec![vec![1_i32, 2, 3, 4, 5]],
                    44100,
                )
                .unwrap();
            }
            crate::songs::initialize_songs(&songs_dir).unwrap();
            songs_dir
        }

        #[test]
        fn verify_single_profile_with_track_mappings() {
            let tmp = tempfile::tempdir().unwrap();
            let songs_dir = create_songs_dir(tmp.path(), "Test Song", &["click", "cue"]);
            let config_path = tmp.path().join("mtrack.yaml");
            std::fs::write(
                &config_path,
                format!(
                    "songs: {}\naudio:\n  device: mock\ntrack_mappings:\n  click:\n  - 1\n  cue:\n  - 2\n",
                    songs_dir.display()
                ),
            )
            .unwrap();
            // run_all = true (check is None), single profile
            assert!(verify(config_path.to_str().unwrap(), None, None).is_ok());
        }

        #[test]
        fn verify_with_specific_check_track_mappings() {
            let tmp = tempfile::tempdir().unwrap();
            let songs_dir = create_songs_dir(tmp.path(), "Test Song", &["click"]);
            let config_path = tmp.path().join("mtrack.yaml");
            std::fs::write(
                &config_path,
                format!(
                    "songs: {}\naudio:\n  device: mock\ntrack_mappings:\n  click:\n  - 1\n",
                    songs_dir.display()
                ),
            )
            .unwrap();
            // Provide a specific check
            assert!(verify(
                config_path.to_str().unwrap(),
                Some(vec!["track-mappings".to_string()]),
                None
            )
            .is_ok());
        }

        #[test]
        fn verify_with_unrelated_check_skips_track_mappings() {
            let tmp = tempfile::tempdir().unwrap();
            let songs_dir = create_songs_dir(tmp.path(), "Test Song", &["click"]);
            let config_path = tmp.path().join("mtrack.yaml");
            std::fs::write(
                &config_path,
                format!(
                    "songs: {}\naudio:\n  device: mock\ntrack_mappings:\n  click:\n  - 1\n",
                    songs_dir.display()
                ),
            )
            .unwrap();
            // Provide a check that doesn't match track-mappings, so that branch is skipped
            assert!(verify(
                config_path.to_str().unwrap(),
                Some(vec!["other-check".to_string()]),
                None
            )
            .is_ok());
        }

        #[test]
        fn verify_multi_profile_with_hostname_filter() {
            let tmp = tempfile::tempdir().unwrap();
            let songs_dir = create_songs_dir(tmp.path(), "Test Song", &["click", "cue"]);
            let config_path = tmp.path().join("mtrack.yaml");
            std::fs::write(
                &config_path,
                format!(
                    r#"songs: {}
profiles:
  - hostname: pi-a
    audio:
      device: mock-a
      track_mappings:
        click:
          - 1
        cue:
          - 2
  - hostname: pi-b
    audio:
      device: mock-b
      track_mappings:
        click:
          - 3
        cue:
          - 4
"#,
                    songs_dir.display()
                ),
            )
            .unwrap();
            // Verify with hostname filter matching one profile
            assert!(verify(
                config_path.to_str().unwrap(),
                None,
                Some("pi-a".to_string())
            )
            .is_ok());
        }

        #[test]
        fn verify_multi_profile_without_hostname() {
            let tmp = tempfile::tempdir().unwrap();
            let songs_dir = create_songs_dir(tmp.path(), "Test Song", &["click", "cue"]);
            let config_path = tmp.path().join("mtrack.yaml");
            std::fs::write(
                &config_path,
                format!(
                    r#"songs: {}
profiles:
  - hostname: pi-a
    audio:
      device: mock-a
      track_mappings:
        click:
          - 1
        cue:
          - 2
  - hostname: pi-b
    audio:
      device: mock-b
      track_mappings:
        click:
          - 3
        cue:
          - 4
"#,
                    songs_dir.display()
                ),
            )
            .unwrap();
            // Verify without hostname filter - checks all profiles
            assert!(verify(config_path.to_str().unwrap(), None, None).is_ok());
        }

        #[test]
        fn verify_multi_profile_with_nonmatching_hostname() {
            let tmp = tempfile::tempdir().unwrap();
            let songs_dir = create_songs_dir(tmp.path(), "Test Song", &["click"]);
            let config_path = tmp.path().join("mtrack.yaml");
            std::fs::write(
                &config_path,
                format!(
                    r#"songs: {}
profiles:
  - hostname: pi-a
    audio:
      device: mock-a
      track_mappings:
        click:
          - 1
  - hostname: pi-b
    audio:
      device: mock-b
      track_mappings:
        click:
          - 3
"#,
                    songs_dir.display()
                ),
            )
            .unwrap();
            // Hostname that doesn't match any profile
            assert!(verify(
                config_path.to_str().unwrap(),
                None,
                Some("nonexistent-host".to_string())
            )
            .is_ok());
        }

        #[test]
        fn verify_profile_without_audio_skips() {
            let tmp = tempfile::tempdir().unwrap();
            let songs_dir = create_songs_dir(tmp.path(), "Test Song", &["click"]);
            let config_path = tmp.path().join("mtrack.yaml");
            std::fs::write(
                &config_path,
                format!(
                    r#"songs: {}
profiles:
  - hostname: lighting-node
    dmx:
      universes:
        - universe: 1
          name: light-show
  - hostname: audio-node
    audio:
      device: mock
      track_mappings:
        click:
          - 1
"#,
                    songs_dir.display()
                ),
            )
            .unwrap();
            // The first profile has no audio, so it should be skipped
            assert!(verify(config_path.to_str().unwrap(), None, None).is_ok());
        }

        #[test]
        fn verify_profile_without_hostname_shows_any_host_label() {
            let tmp = tempfile::tempdir().unwrap();
            let songs_dir = create_songs_dir(tmp.path(), "Test Song", &["click"]);
            let config_path = tmp.path().join("mtrack.yaml");
            std::fs::write(
                &config_path,
                format!(
                    r#"songs: {}
profiles:
  - hostname: pi-a
    audio:
      device: mock-a
      track_mappings:
        click:
          - 1
  - audio:
      device: fallback
      track_mappings:
        click:
          - 1
"#,
                    songs_dir.display()
                ),
            )
            .unwrap();
            // Second profile has no hostname - exercises the "any host" label path
            assert!(verify(config_path.to_str().unwrap(), None, None).is_ok());
        }
    }
}
