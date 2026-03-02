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

/// Sets the current thread's priority for better audio scheduling (cross-platform).
/// Uses the [0; 100) scale: higher value = higher priority.
/// Default is 70 when MTRACK_THREAD_PRIORITY is unset; set to 0-99 to override.
/// Fails silently if lacking permission. Use high values with care; they can starve other threads.
fn apply_thread_priority() {
    use thread_priority::{set_current_thread_priority, ThreadPriority, ThreadPriorityValue};

    let priority = std::env::var("MTRACK_THREAD_PRIORITY")
        .ok()
        .and_then(|v| {
            let n = v.parse::<u8>().ok()?;
            (n < 100).then(|| ThreadPriorityValue::try_from(n).ok())?
        })
        .unwrap_or_else(|| ThreadPriorityValue::try_from(DEFAULT_THREAD_PRIORITY).unwrap());

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
    player_path: &str,
    playlist_path: Option<String>,
    web_config: crate::webui::server::WebConfig,
    tui_mode: bool,
) -> Result<(), Box<dyn Error>> {
    apply_thread_priority();
    let player_path = &Path::new(player_path);
    let player_config = config::Player::deserialize(player_path)?;
    let mut playlist_path = playlist_path
        .as_ref()
        .map(PathBuf::from)
        .or(player_config.playlist())
        .ok_or(
            "Must supply the playlist from the command line or inside the mtrack configuration",
        )?;
    if !playlist_path.is_absolute() {
        playlist_path = if let Some(parent) = player_path.parent() {
            parent.join(playlist_path)
        } else {
            return Err(format!(
                "Unable to determine playlist path (config base path has no parent): {}. \
                 Please specify the playlist path using an absolute path.",
                playlist_path.display()
            )
            .into());
        };
    };
    let songs = songs::get_all_songs(&player_config.songs(player_path))?;
    let playlist = Playlist::new(
        "playlist",
        &config::Playlist::deserialize(playlist_path.as_path())?,
        songs.clone(),
    )?;

    let player = Arc::new(crate::player::Player::new(
        songs,
        playlist,
        &player_config,
        player_path.parent(),
    )?);

    // Start the shared state sampler if a DMX engine (with effect engine) is present.
    // Both the TUI and the simulator subscribe to this channel.
    let (state_rx, _sampler_handle) = if let Some(effect_engine) = player.effect_engine() {
        let (rx, handle) = crate::state::start_sampler(effect_engine);
        (rx, Some(handle))
    } else {
        let (_, rx) = tokio::sync::watch::channel(std::sync::Arc::new(
            crate::state::StateSnapshot::default(),
        ));
        (rx, None)
    };

    // Start the unified web server (dashboard + gRPC-Web + REST API)
    let _webui_handle = {
        // Create a broadcast channel for the web UI (shared by dashboard WS and DMX file watcher)
        let (broadcast_tx, _) = tokio::sync::broadcast::channel::<String>(128);

        // Build metadata JSON and wire the broadcast channel to the DMX engine
        let metadata_json = if let Some(handles) = player.broadcast_handles() {
            let metadata =
                crate::webui::state::build_metadata_json(handles.lighting_system.as_ref());
            // Pass the broadcast channel to the DmxEngine for file watcher hot-reload
            player.set_broadcast_tx(broadcast_tx.clone());
            metadata
        } else {
            // No DMX engine — empty metadata
            crate::webui::state::build_metadata_json(None)
        };

        let webui_state = crate::webui::server::WebUiState {
            player: player.clone(),
            state_rx: state_rx.clone(),
            broadcast_tx,
            config_path: player_path.to_path_buf(),
            songs_path: player_config.songs(player_path),
            playlist_path: playlist_path.clone(),
            metadata_json: std::sync::Arc::new(metadata_json),
            waveform_cache: crate::webui::state::new_waveform_cache(),
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
                    verify::check_all_track_mappings(&songs, audio_config.track_mappings());
                report.merge(track_report);
            }
        } else {
            // Single profile (legacy or single profile): verify as before.
            let track_report =
                verify::check_all_track_mappings(&songs, player_config.track_mappings());
            report.merge(track_report);
        }
    }

    verify::print_report(&report, &songs);

    if report.has_errors() {
        std::process::exit(1);
    }

    Ok(())
}
