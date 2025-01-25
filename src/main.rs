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
mod midi;
mod player;
mod playlist;
mod playsync;
mod songs;
#[cfg(test)]
mod test;

use clap::{crate_version, Parser, Subcommand};
use config::audio::Audio;
use config::dmx::{Dmx, Universe};
use config::init_player_and_controller;
use config::midi::Midi;
use player::Player;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::path::PathBuf;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::playlist::Playlist;

const SYSTEMD_SERVICE: &str = r#"
[Unit]
Description=multitrack player

[Service]
Type=simple
Restart=on-failure
EnvironmentFile=-/etc/default/mtrack
ExecStart=/usr/local/bin/mtrack start "$MTRACK_CONFIG" "$PLAYLIST"
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
    },
    /// Lists the available audio output devices.
    Devices {},
    /// Lists the available MIDI input/output devices.
    MidiDevices {},
    /// Plays a song through the audio interface.
    Play {
        /// The device name to play through.
        device_name: String,
        /// The channel to device mappings. Should be in the form <TRACKNAME>=<CHANNEL>,...
        /// For example, click=1,cue=2,backing-l=3.
        mappings: String,
        /// The MIDI device name to play through.
        #[arg[short, long]]
        midi_device_name: Option<String>,
        /// The MIDI playback delay.
        midi_playback_delay: Option<String>,
        /// The path to the song repository.
        repository_path: String,
        /// The name of the song to play.
        song_name: String,
        /// The DMX dimming speed modifier.
        #[arg[short = 's', long]]
        dmx_dimming_speed_modifier: Option<f64>,
        /// The DMX playback delay.
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
        /// The path to the playlist.
        playlist_path: String,
    },
    /// Prints a systemd service definition to stdout.
    Systemd {},
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Songs { path } => {
            let songs = config::get_all_songs(&PathBuf::from(&path))?;

            if songs.is_empty() {
                println!("No songs found in {}.", path.as_str());
                return Ok(());
            }

            let mut all_tracks: HashSet<String> = HashSet::new();
            println!("Songs (count: {}):", songs.len());
            for song in songs.sorted_list() {
                // Record all of the tracks found in the song repository.
                for track in song.tracks.iter() {
                    all_tracks.insert(track.name.to_string());
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
        Commands::Play {
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

            let device = audio::get_device(Some(Audio::new(device_name, None)))?;
            let midi_device = match midi_device_name {
                Some(midi_device_name) => {
                    midi::get_device(Some(Midi::new(midi_device_name, midi_playback_delay)))?
                }
                None => None,
            };
            let dmx_engine = match dmx_universe_config {
                Some(dmx_universe_config) => {
                    let mut universe_configs: Vec<Universe> = Vec::new();
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

                        if universe.is_some() && name.is_some() {
                            universe_configs.push(Universe::new(universe.unwrap(), name.unwrap()));
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
                        dmx::create_engine(Some(Dmx::new(
                            dmx_dimming_speed_modifier,
                            dmx_playback_delay,
                            universe_configs,
                        )))?
                    }
                }
                None => None,
            };
            let songs = config::get_all_songs(&PathBuf::from(&repository_path))?;
            let playlist = Arc::new(Playlist::new(vec![song_name], Arc::clone(&songs))?);
            let player = Player::new(
                device,
                converted_mappings,
                midi_device,
                dmx_engine,
                playlist,
                Playlist::from_songs(songs)?,
                None,
            );

            player.play().await?;
            while !player.wait_for_current_song().await? {
                thread::sleep(Duration::from_millis(10));
            }
        }
        Commands::Playlist {
            repository_path,
            playlist_path,
        } => {
            let songs = config::get_all_songs(&PathBuf::from(&repository_path))?;
            let playlist = config::parse_playlist(&PathBuf::from(playlist_path), songs)?;

            println!("{}", playlist);
        }
        Commands::Start {
            player_path,
            playlist_path,
        } => {
            init_player_and_controller(&PathBuf::from(player_path), &PathBuf::from(playlist_path))?
                .join()
                .await?;
        }
        Commands::Systemd {} => {
            println!("{}", SYSTEMD_SERVICE)
        }
    }

    Ok(())
}
