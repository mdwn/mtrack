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
mod songs;
#[cfg(test)]
mod testutil;
mod util;

use crate::playlist::Playlist;
use clap::{crate_version, Parser, Subcommand};
use player::Player;
use proto::player::v1::player_service_client::PlayerServiceClient;
use proto::player::v1::{
    NextRequest, PlayRequest, PreviousRequest, Song, StatusRequest, StopRequest,
    SwitchToPlaylistRequest,
};
use std::collections::{HashMap, HashSet};
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
    PlayDirect {
        /// The device name to play through.
        device_name: String,
        /// The channel to device mappings. Should be in the form <TRACKNAME>=<CHANNEL>,...
        /// For example, click=1,cue=2,backing-l=3.
        mappings: String,
        /// The MIDI device name to play through.
        #[arg[short, long]]
        midi_device_name: Option<String>,
        /// The MIDI playback delay.
        #[arg[long]]
        midi_playback_delay: Option<String>,
        /// The path to the song repository.
        repository_path: String,
        /// The name of the song to play.
        song_name: String,
        /// The DMX dimming speed modifier.
        #[arg[short = 's', long]]
        dmx_dimming_speed_modifier: Option<f64>,
        /// The DMX playback delay.
        #[arg[long]]
        dmx_playback_delay: Option<String>,
        /// The DMX universe configuration. Should be in the form: universe=<OLA-UNIVERSE>,name=<NAME>;...
        /// For example, universe=1,name=light-show;universe=2,name=another-light-show
        #[arg[short, long]]
        dmx_universe_config: Option<String>,
    },
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
    /// Prints a systemd service definition to stdout.
    Systemd {},
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt::init();

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
        Commands::PlayDirect {
            device_name,
            mappings,
            midi_device_name,
            midi_playback_delay,
            dmx_dimming_speed_modifier,
            dmx_playback_delay,
            dmx_universe_config,
            repository_path,
            song_name,
        } => {
            let mut converted_mappings: HashMap<String, Vec<u16>> = HashMap::new();
            for mapping in mappings.split(',') {
                let track_and_channel: Vec<&str> = mapping.split('=').collect();
                if track_and_channel.len() != 2 {
                    return Err("malformed track to channel mapping".into());
                };
                let track = track_and_channel[0];
                let channel = track_and_channel[1].parse::<u16>()?;
                if !converted_mappings.contains_key(track) {
                    converted_mappings.insert(track.into(), vec![]);
                }
                converted_mappings
                    .get_mut(track)
                    .expect("expected mapping")
                    .push(channel);
            }

            let audio_config = config::Audio::new(device_name.as_str());
            let midi_config = midi_device_name.map(|midi_device_name| {
                config::Midi::new(midi_device_name.as_str(), midi_playback_delay)
            });
            let dmx_config = match dmx_universe_config {
                Some(dmx_universe_config) => {
                    let mut universe_configs: Vec<config::Universe> = Vec::new();
                    for universe_config in dmx_universe_config.split(';') {
                        let config_fields: Vec<&str> = universe_config.split(',').collect();

                        let mut universe: Option<u16> = None;
                        let mut name: Option<String> = None;
                        for config in config_fields.into_iter() {
                            let config_parts: Vec<&str> = config.split('=').collect();

                            if config_parts.len() != 2 {
                                return Err(format!(
                                    "malformed DMX configuration '{}'",
                                    universe_config
                                )
                                .into());
                            }

                            // Parse the DMX configuration.
                            match config_parts[0] {
                                "universe" => {
                                    let universe_num: u16 = config_parts[1].parse()?;
                                    universe = Some(universe_num);
                                }
                                "name" => name = Some(config_parts[1].into()),
                                _ => {} // Do nothing
                            }
                        }

                        if let (Some(universe_id), Some(universe_name)) = (universe, name) {
                            universe_configs
                                .push(config::Universe::new(universe_id, universe_name));
                        } else {
                            return Err(format!(
                                "Missing device specified for config {}",
                                universe_config
                            )
                            .into());
                        }
                    }

                    if universe_configs.is_empty() {
                        None
                    } else {
                        Some(config::Dmx::new(
                            dmx_dimming_speed_modifier,
                            dmx_playback_delay,
                            None,
                            universe_configs,
                            None, // lighting configuration
                        ))
                    }
                }
                None => None,
            };

            let songs = songs::get_all_songs(Path::new(&repository_path))?;
            let playlist = Playlist::new(&config::Playlist::new(&[song_name]), Arc::clone(&songs))?;

            let player = Player::new(
                songs,
                playlist,
                &config::Player::new(
                    vec![config::Controller::Keyboard {}],
                    audio_config,
                    midi_config,
                    dmx_config,
                    converted_mappings,
                    &repository_path,
                ),
                None,
            )?;

            player.play().await;
            while !player.wait_for_current_song().await? {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        }
        Commands::Playlist {
            repository_path,
            playlist_path,
        } => {
            let songs = songs::get_all_songs(Path::new(&repository_path))?;
            let playlist = Playlist::new(
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
        Commands::Play { host_port } => {
            let mut client = connect(host_port).await?;
            let response = client.play(Request::new(PlayRequest {})).await?;
            println!("Playing the song:");
            print_song(response.into_inner().song)?;
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
                let elapsed = util::duration_minutes_seconds(Duration::try_from(
                    response.elapsed.unwrap_or_default(),
                )?);
                println!("Elapsed: {}/{}", elapsed, song_duration);
            }
            println!("Playing: {}", response.playing);
            println!("Playlist name: {}", response.playlist_name)
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
