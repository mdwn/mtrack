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
mod audio;
mod config;
mod controller;
mod dmx;
mod lighting;
mod midi;
mod player;
mod playlist;
mod playsync;
mod proto;
mod samples;
mod songs;
#[cfg(test)]
mod testutil;
mod util;

use crate::playlist::Playlist;
use clap::{crate_version, Parser, Subcommand};
use lighting::parser::{parse_light_shows, utils::parse_time_string};
use lighting::validation::validate_groups;
use proto::player::v1::player_service_client::PlayerServiceClient;
use proto::player::v1::{
    GetActiveEffectsRequest, GetCuesRequest, NextRequest, PlayFromRequest, PlayRequest,
    PreviousRequest, Song, StatusRequest, StopRequest, SwitchToPlaylistRequest,
};
use std::collections::HashSet;
use std::env;
use std::error::Error;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tonic::transport::Channel;
use tonic::Request;
use tracing::info;

const SYSTEMD_SERVICE: &str = r#"
[Unit]
Description=multitrack player

[Service]
Type=simple
Restart=on-failure
EnvironmentFile=-/etc/default/mtrack
ExecStart={{ CURRENT_EXECUTABLE }} start "$MTRACK_CONFIG"
ExecReload=/bin/kill -HUP $MAINPID

[Install]
WantedBy=multi-user.target
Alias=mtrack.service
"#;

#[derive(Parser)]
#[clap(
    author = "Michael Wilson",
    version = crate_version!(),
    about = "A multitrack player."
)]
struct Cli {
    #[clap(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Lists and verifies all songs in the given directory.
    Songs {
        /// The path to the songs repository on disk.
        path: String,
        /// Initialize song directories with YAML files containing default values.
        #[arg(long)]
        init: bool,
    },
    /// Lists the available audio output devices.
    Devices {},
    /// Lists the available MIDI input/output devices.
    MidiDevices {},
    /// Plays a song through the audio interface.
    /// Playlist will verify a playlist.
    Playlist {
        /// The path to the song repository.
        repository_path: String,
        /// The path to the playlist.
        playlist_path: String,
    },
    /// Start will start the multitrack player.
    Start {
        /// The path to the player config.
        player_path: String,
        /// The path to the playlist. Must be specified if the playlist is not specified in the player config.
        playlist_path: Option<String>,
    },
    /// Plays the current song in the playlist.
    Play {
        /// The host and port of the gRPC server.
        #[arg[short, long]]
        host_port: Option<String>,
        /// Start playback from a specific time (e.g., "1:23.456" or "45.5s").
        #[arg(long)]
        from: Option<String>,
    },
    /// Moves to the previous song in the playlist.
    Previous {
        /// The host and port of the gRPC server.
        #[arg[short, long]]
        host_port: Option<String>,
    },
    /// Moves to the next song in the playlist.
    Next {
        /// The host and port of the gRPC server.
        #[arg[short, long]]
        host_port: Option<String>,
    },
    /// Stops the currently playing song.
    Stop {
        /// The host and port of the gRPC server.
        #[arg[short, long]]
        host_port: Option<String>,
    },
    /// Switches to the given playlist.
    SwitchToPlaylist {
        /// The host and port of the gRPC server.
        #[arg[short, long]]
        host_port: Option<String>,
        /// The name of the playlist to switch to. Currently only supports "all_songs" and "playlist."
        playlist_name: String,
    },
    /// Gets the current status of the player from the gRPC server.
    Status {
        /// The host and port of the gRPC server.
        #[arg[short, long]]
        host_port: Option<String>,
    },
    /// Prints all active lighting effects from the gRPC server.
    ActiveEffects {
        /// The host and port of the gRPC server.
        #[arg[short, long]]
        host_port: Option<String>,
    },
    /// Prints a systemd service definition to stdout.
    Systemd {},
    /// Verifies the syntax of a light show file.
    VerifyLightShow {
        /// The path to the light show file to verify.
        show_path: String,
        /// Optional path to mtrack.yaml config file to validate group/fixture names.
        #[arg(short, long)]
        config: Option<String>,
    },
    /// Lists all cues in the current song's lighting timeline.
    Cues {
        /// The host and port of the gRPC server.
        #[arg[short, long]]
        host_port: Option<String>,
    },
}

/// Verifies a light show file, optionally validating against a config file.
fn verify_light_show(show_path: &str, config_path: Option<&str>) -> Result<(), Box<dyn Error>> {
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

#[tokio::main]
async fn main() {
    // Initialize tracing with a filter that sets default logging to off, with mtrack at info level
    // This prevents noisy INFO messages from symphonia crates (which are suppressed by the default "off")
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("off,mtrack=info"));

    tracing_subscriber::fmt().with_env_filter(filter).init();

    if let Err(e) = run().await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Songs { path, init } => {
            if init {
                info!("Initializing songs");
                songs::initialize_songs(Path::new(&path))?;
            } else {
                info!("Not initializing songs");
            }

            let songs = songs::get_all_songs(Path::new(&path))?;

            if songs.is_empty() {
                println!("No songs found in {}.", path.as_str());
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
        }
        Commands::Devices {} => {
            let devices = audio::list_devices()?;

            if devices.is_empty() {
                println!("No devices found.");
                return Ok(());
            }

            println!("Devices:");
            for device in devices {
                println!("- {}", device);
            }
        }
        Commands::MidiDevices {} => {
            let devices = midi::list_devices()?;

            if devices.is_empty() {
                println!("No devices found.");
                return Ok(());
            }

            println!("Devices:");
            for device in devices {
                println!("- {}", device);
            }
        }
        Commands::Playlist {
            repository_path,
            playlist_path,
        } => {
            let songs = songs::get_all_songs(Path::new(&repository_path))?;
            let playlist = Playlist::new(
                "playlist",
                &config::Playlist::deserialize(Path::new(&playlist_path))?,
                songs,
            )?;

            println!("{}", playlist);
        }
        Commands::Start {
            player_path,
            playlist_path,
        } => {
            let player_path = &Path::new(&player_path);
            let player_config = config::Player::deserialize(player_path)?;
            let mut playlist_path = playlist_path.as_ref().map(PathBuf::from).or(player_config.playlist()).ok_or(
                "Must supply the playlist from the command line or inside the mtrack configuration",
            )?;
            if !playlist_path.is_absolute() {
                playlist_path = if let Some(parent) = player_path.parent() {
                    parent.join(playlist_path)
                } else {
                    return Err("Unable to determining playlist path. Please specify the playlist path using an absolute path.")?;
                };
            };
            let songs = songs::get_all_songs(&player_config.songs(player_path))?;
            let playlist = Playlist::new(
                "playlist",
                &config::Playlist::deserialize(playlist_path.as_path())?,
                songs.clone(),
            )?;

            let player = Arc::new(player::Player::new(
                songs,
                playlist,
                &player_config,
                player_path.parent(),
            )?);
            crate::controller::Controller::new(player_config.controllers(), player)?
                .join()
                .await?;
        }
        Commands::Play { host_port, from } => {
            let mut client = connect(host_port).await?;
            if let Some(from_str) = from {
                // Parse the time string
                let start_time = parse_time_string(&from_str)?;
                let start_duration = prost_types::Duration::try_from(start_time)
                    .map_err(|e| format!("Failed to convert duration: {}", e))?;

                let response = client
                    .play_from(Request::new(PlayFromRequest {
                        start_time: Some(start_duration),
                    }))
                    .await?;
                println!("Playing the song from {}:", from_str);
                print_song(response.into_inner().song)?;
            } else {
                let response = client.play(Request::new(PlayRequest {})).await?;
                println!("Playing the song:");
                print_song(response.into_inner().song)?;
            }
        }
        Commands::Previous { host_port } => {
            let mut client = connect(host_port).await?;
            let response = client.previous(Request::new(PreviousRequest {})).await?;
            println!("Moved to previous song:");
            print_song(response.into_inner().song)?;
        }
        Commands::Next { host_port } => {
            let mut client = connect(host_port).await?;
            let response = client.next(Request::new(NextRequest {})).await?;
            println!("Moved to next song:");
            print_song(response.into_inner().song)?;
        }
        Commands::Stop { host_port } => {
            let mut client = connect(host_port).await?;
            let response = client.stop(Request::new(StopRequest {})).await?;
            println!("The song was stopped:");
            print_song(response.into_inner().song)?;
        }
        Commands::SwitchToPlaylist {
            host_port,
            playlist_name,
        } => {
            let mut client = connect(host_port).await?;
            let _ = client
                .switch_to_playlist(Request::new(SwitchToPlaylistRequest {
                    playlist_name: playlist_name.clone(),
                }))
                .await?;
            println!("Switched to playlist {}", playlist_name);
        }
        Commands::Status { host_port } => {
            let mut client = connect(host_port).await?;
            let response = client
                .status(Request::new(StatusRequest {}))
                .await?
                .into_inner();
            if let Some(song) = response.current_song {
                println!("Current song: {}", song.name);
                let song_duration = util::duration_minutes_seconds(Duration::try_from(
                    song.duration.unwrap_or_default(),
                )?);
                let elapsed = Duration::try_from(response.elapsed.unwrap_or_default())
                    .map(util::duration_minutes_seconds)?;
                println!("Elapsed: {}/{}", elapsed, song_duration);
            }
            println!("Playing: {}", response.playing);
            println!("Playlist name: {}", response.playlist_name)
        }
        Commands::ActiveEffects { host_port } => {
            let mut client = connect(host_port).await?;
            let response = client
                .get_active_effects(Request::new(GetActiveEffectsRequest {}))
                .await?;
            println!("{}", response.into_inner().active_effects);
        }
        Commands::Systemd {} => {
            let current_executable_path = env::current_exe()?;
            println!(
                "{}",
                SYSTEMD_SERVICE.replace(
                    "{{ CURRENT_EXECUTABLE }}",
                    current_executable_path
                        .to_str()
                        .expect("unable to convert current executable path to string")
                )
            )
        }
        Commands::VerifyLightShow { show_path, config } => {
            verify_light_show(&show_path, config.as_deref())?;
        }
        Commands::Cues { host_port } => {
            let mut client = connect(host_port).await?;
            let response = client
                .get_cues(Request::new(GetCuesRequest {}))
                .await?
                .into_inner();

            if response.cues.is_empty() {
                println!("No cues found in the current song.");
            } else {
                println!("Cues in current song ({} total):", response.cues.len());
                for cue in response.cues {
                    let time = cue
                        .time
                        .and_then(|d| Duration::try_from(d).ok())
                        .map(util::duration_minutes_seconds)
                        .unwrap_or_else(|| "unknown".to_string());
                    println!("  {}: {} (index {})", cue.index, time, cue.index);
                }
            }
        }
    }

    Ok(())
}

fn print_song(song: Option<Song>) -> Result<(), Box<dyn Error>> {
    if let Some(song) = song {
        println!("Name: {}", song.name);
        println!(
            "Duration: {}",
            util::duration_minutes_seconds(Duration::try_from(song.duration.unwrap_or_default())?)
        );
        println!("Tracks:");
        for track in song.tracks {
            println!("  - {}", track);
        }
    }

    Ok(())
}

async fn connect(
    host_port: Option<String>,
) -> Result<PlayerServiceClient<Channel>, Box<dyn Error>> {
    Ok(PlayerServiceClient::connect(
        "http://".to_owned()
            + &host_port.unwrap_or(format!("0.0.0.0:{}", config::DEFAULT_GRPC_PORT)),
    )
    .await?)
}
