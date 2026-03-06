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

mod local;
mod remote;

use crate::audio;
use crate::midi;
use clap::{crate_version, Parser, Subcommand};
use std::env;
use std::error::Error;
use std::fmt::Display;

const SYSTEMD_SERVICE: &str = r#"
[Unit]
Description=multitrack player

[Service]
Type=simple
Restart=on-failure
EnvironmentFile=-/etc/default/mtrack
ExecStart={{ CURRENT_EXECUTABLE }} start "$MTRACK_CONFIG"
ExecReload=/bin/kill -HUP $MAINPID

# User and group. Create with:
#   sudo useradd --system --no-create-home --shell /usr/sbin/nologin mtrack
#   sudo usermod -aG audio mtrack
User=mtrack
Group=mtrack
SupplementaryGroups=audio

# Allow setting thread/RT priority for real-time audio scheduling.
AmbientCapabilities=CAP_SYS_NICE
CapabilityBoundingSet=CAP_SYS_NICE

# Filesystem restrictions. The entire filesystem is read-only, which is
# sufficient since mtrack does not write to disk. /home is inaccessible.
ProtectSystem=strict
ProtectHome=true
PrivateTmp=true

# Kernel restrictions.
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectKernelLogs=true
ProtectControlGroups=true

# Additional hardening.
NoNewPrivileges=true
LockPersonality=true
RestrictNamespaces=true
RestrictSUIDSGID=true
MemoryDenyWriteExecute=true
SystemCallArchitectures=native
RestrictAddressFamilies=AF_INET AF_INET6 AF_UNIX

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
        /// Enable the terminal UI.
        #[arg(long)]
        tui: bool,
        /// Port for the web UI and lighting simulator (default: 8080).
        #[arg(long, default_value = "8080")]
        web_port: u16,
        /// Bind address for the web UI and lighting simulator (default: 127.0.0.1).
        #[arg(long, default_value = "127.0.0.1")]
        web_address: String,
    },
    /// Plays the current song in the playlist.
    Play {
        /// The host and port of the gRPC server.
        #[arg(short = 'H', long)]
        host_port: Option<String>,
        /// Start playback from a specific time (e.g., "1:23.456" or "45.5s").
        #[arg(long)]
        from: Option<String>,
    },
    /// Moves to the previous song in the playlist.
    Previous {
        /// The host and port of the gRPC server.
        #[arg(short = 'H', long)]
        host_port: Option<String>,
    },
    /// Moves to the next song in the playlist.
    Next {
        /// The host and port of the gRPC server.
        #[arg(short = 'H', long)]
        host_port: Option<String>,
    },
    /// Stops the currently playing song.
    Stop {
        /// The host and port of the gRPC server.
        #[arg(short = 'H', long)]
        host_port: Option<String>,
    },
    /// Switches to the given playlist.
    SwitchToPlaylist {
        /// The host and port of the gRPC server.
        #[arg(short = 'H', long)]
        host_port: Option<String>,
        /// The name of the playlist to switch to. Currently only supports "all_songs" and "playlist."
        playlist_name: String,
    },
    /// Gets the current status of the player from the gRPC server.
    Status {
        /// The host and port of the gRPC server.
        #[arg(short = 'H', long)]
        host_port: Option<String>,
    },
    /// Prints all active lighting effects from the gRPC server.
    ActiveEffects {
        /// The host and port of the gRPC server.
        #[arg(short = 'H', long)]
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
        #[arg(short = 'H', long)]
        host_port: Option<String>,
    },
    /// Auto-calibrate trigger detection parameters from a connected audio input device.
    CalibrateTriggers {
        /// Audio input device name (as shown by `mtrack devices`).
        device: String,
        /// Sample rate override.
        #[arg(long)]
        sample_rate: Option<u32>,
        /// Noise floor measurement duration in seconds.
        #[arg(long, default_value = "3")]
        duration: f32,
        /// Sample format override: "int" or "float".
        #[arg(long)]
        sample_format: Option<String>,
        /// Bits per sample override: 16 or 32.
        #[arg(long)]
        bits_per_sample: Option<u16>,
    },
    /// Verifies songs in a repository against the player config.
    Verify {
        /// The path to the mtrack.yaml player config file.
        config: String,
        /// Only check specific categories (e.g., "track-mappings"). Runs all checks if omitted.
        #[arg(long)]
        check: Option<Vec<String>>,
        /// Hostname to verify against. When audio_profiles are used, this filters which profiles
        /// to check. If omitted, all profiles are verified.
        #[arg(long)]
        hostname: Option<String>,
    },
}

/// Formats a list of devices into a display string.
fn format_device_list<T: Display>(devices: &[T], empty_msg: &str) -> String {
    if devices.is_empty() {
        return empty_msg.to_string();
    }
    let mut output = String::from("Devices:");
    for device in devices {
        output.push_str(&format!("\n- {}", device));
    }
    output
}

/// Prints a list of devices for the Devices and MidiDevices subcommands.
fn print_device_list<T: Display>(devices: Vec<T>, empty_msg: &str) {
    println!("{}", format_device_list(&devices, empty_msg));
}

/// Renders the systemd service template with the given executable path.
fn render_systemd_service(executable_path: &str) -> String {
    SYSTEMD_SERVICE.replace("{{ CURRENT_EXECUTABLE }}", executable_path)
}

pub async fn run(tui_mode: bool) -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Songs { path, init } => local::songs(&path, init)?,
        Commands::Devices {} => print_device_list(audio::list_devices()?, "No devices found."),
        Commands::MidiDevices {} => print_device_list(midi::list_devices()?, "No devices found."),
        Commands::Playlist {
            repository_path,
            playlist_path,
        } => local::playlist(&repository_path, &playlist_path)?,
        Commands::Start {
            player_path,
            playlist_path,
            tui,
            web_port,
            web_address,
        } => {
            let web_config = crate::webui::server::WebConfig {
                port: web_port,
                address: web_address,
            };
            let effective_tui = tui_mode && tui;
            local::start(&player_path, playlist_path, web_config, effective_tui).await?
        }
        Commands::Play { host_port, from } => remote::play(host_port, from).await?,
        Commands::Previous { host_port } => remote::previous(host_port).await?,
        Commands::Next { host_port } => remote::next(host_port).await?,
        Commands::Stop { host_port } => remote::stop(host_port).await?,
        Commands::SwitchToPlaylist {
            host_port,
            playlist_name,
        } => remote::switch_to_playlist(host_port, &playlist_name).await?,
        Commands::Status { host_port } => remote::status(host_port).await?,
        Commands::ActiveEffects { host_port } => remote::active_effects(host_port).await?,
        Commands::Systemd {} => {
            let current_executable_path = env::current_exe()?;
            println!(
                "{}",
                render_systemd_service(
                    current_executable_path
                        .to_str()
                        .expect("unable to convert current executable path to string")
                )
            )
        }
        Commands::CalibrateTriggers {
            device,
            sample_rate,
            duration,
            sample_format,
            bits_per_sample,
        } => local::calibrate_triggers(
            &device,
            sample_rate,
            duration,
            sample_format,
            bits_per_sample,
        )?,
        Commands::VerifyLightShow { show_path, config } => {
            local::verify_light_show(&show_path, config.as_deref())?
        }
        Commands::Cues { host_port } => remote::cues(host_port).await?,
        Commands::Verify {
            config,
            check,
            hostname,
        } => local::verify(&config, check, hostname)?,
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    mod format_device_list_tests {
        use super::*;

        #[test]
        fn empty_list_returns_empty_msg() {
            let devices: Vec<String> = vec![];
            assert_eq!(
                format_device_list(&devices, "No devices found."),
                "No devices found."
            );
        }

        #[test]
        fn single_device() {
            let devices = vec!["My Speaker"];
            let result = format_device_list(&devices, "No devices found.");
            assert_eq!(result, "Devices:\n- My Speaker");
        }

        #[test]
        fn multiple_devices() {
            let devices = vec!["Speaker A", "Speaker B", "Headphones"];
            let result = format_device_list(&devices, "");
            assert!(result.starts_with("Devices:"));
            assert!(result.contains("- Speaker A"));
            assert!(result.contains("- Speaker B"));
            assert!(result.contains("- Headphones"));
        }

        #[test]
        fn works_with_display_types() {
            let devices: Vec<i32> = vec![1, 2, 3];
            let result = format_device_list(&devices, "");
            assert!(result.contains("- 1"));
            assert!(result.contains("- 2"));
            assert!(result.contains("- 3"));
        }

        #[test]
        fn custom_empty_message() {
            let devices: Vec<String> = vec![];
            assert_eq!(
                format_device_list(&devices, "Nothing here!"),
                "Nothing here!"
            );
        }
    }

    mod render_systemd_service_tests {
        use super::*;

        #[test]
        fn substitutes_executable_path() {
            let result = render_systemd_service("/usr/local/bin/mtrack");
            assert!(result.contains("ExecStart=/usr/local/bin/mtrack start"));
            assert!(!result.contains("{{ CURRENT_EXECUTABLE }}"));
        }

        #[test]
        fn preserves_service_structure() {
            let result = render_systemd_service("/usr/bin/mtrack");
            assert!(result.contains("[Unit]"));
            assert!(result.contains("[Service]"));
            assert!(result.contains("[Install]"));
            assert!(result.contains("Type=simple"));
        }

        #[test]
        fn preserves_hardening_directives() {
            let result = render_systemd_service("/usr/bin/mtrack");
            assert!(result.contains("ProtectSystem=strict"));
            assert!(result.contains("NoNewPrivileges=true"));
            assert!(result.contains("MemoryDenyWriteExecute=true"));
        }

        #[test]
        fn handles_path_with_spaces() {
            let result = render_systemd_service("/opt/my apps/mtrack");
            assert!(result.contains("ExecStart=/opt/my apps/mtrack start"));
        }
    }

    mod cli_parsing_tests {
        use super::*;

        #[test]
        fn parse_songs_command() {
            let cli = Cli::try_parse_from(["mtrack", "songs", "/path/to/songs"]).unwrap();
            match cli.command {
                Commands::Songs { path, init } => {
                    assert_eq!(path, "/path/to/songs");
                    assert!(!init);
                }
                _ => panic!("expected Songs command"),
            }
        }

        #[test]
        fn parse_songs_with_init() {
            let cli = Cli::try_parse_from(["mtrack", "songs", "/path", "--init"]).unwrap();
            match cli.command {
                Commands::Songs { init, .. } => assert!(init),
                _ => panic!("expected Songs command"),
            }
        }

        #[test]
        fn parse_devices_command() {
            let cli = Cli::try_parse_from(["mtrack", "devices"]).unwrap();
            assert!(matches!(cli.command, Commands::Devices {}));
        }

        #[test]
        fn parse_midi_devices_command() {
            let cli = Cli::try_parse_from(["mtrack", "midi-devices"]).unwrap();
            assert!(matches!(cli.command, Commands::MidiDevices {}));
        }

        #[test]
        fn parse_start_command_defaults() {
            let cli = Cli::try_parse_from(["mtrack", "start", "config.yaml"]).unwrap();
            match cli.command {
                Commands::Start {
                    player_path,
                    playlist_path,
                    tui,
                    web_port,
                    web_address,
                } => {
                    assert_eq!(player_path, "config.yaml");
                    assert!(playlist_path.is_none());
                    assert!(!tui);
                    assert_eq!(web_port, 8080);
                    assert_eq!(web_address, "127.0.0.1");
                }
                _ => panic!("expected Start command"),
            }
        }

        #[test]
        fn parse_start_with_all_options() {
            let cli = Cli::try_parse_from([
                "mtrack",
                "start",
                "config.yaml",
                "playlist.yaml",
                "--tui",
                "--web-port",
                "9090",
                "--web-address",
                "0.0.0.0",
            ])
            .unwrap();
            match cli.command {
                Commands::Start {
                    player_path,
                    playlist_path,
                    tui,
                    web_port,
                    web_address,
                } => {
                    assert_eq!(player_path, "config.yaml");
                    assert_eq!(playlist_path.as_deref(), Some("playlist.yaml"));
                    assert!(tui);
                    assert_eq!(web_port, 9090);
                    assert_eq!(web_address, "0.0.0.0");
                }
                _ => panic!("expected Start command"),
            }
        }

        #[test]
        fn parse_play_with_from() {
            let cli = Cli::try_parse_from(["mtrack", "play", "--from", "1:23.456"]).unwrap();
            match cli.command {
                Commands::Play { host_port, from } => {
                    assert!(host_port.is_none());
                    assert_eq!(from.as_deref(), Some("1:23.456"));
                }
                _ => panic!("expected Play command"),
            }
        }

        #[test]
        fn parse_play_with_host() {
            let cli =
                Cli::try_parse_from(["mtrack", "play", "--host-port", "localhost:50051"]).unwrap();
            match cli.command {
                Commands::Play { host_port, .. } => {
                    assert_eq!(host_port.as_deref(), Some("localhost:50051"));
                }
                _ => panic!("expected Play command"),
            }
        }

        #[test]
        fn parse_switch_to_playlist() {
            let cli = Cli::try_parse_from(["mtrack", "switch-to-playlist", "all_songs"]).unwrap();
            match cli.command {
                Commands::SwitchToPlaylist {
                    playlist_name,
                    host_port,
                } => {
                    assert_eq!(playlist_name, "all_songs");
                    assert!(host_port.is_none());
                }
                _ => panic!("expected SwitchToPlaylist command"),
            }
        }

        #[test]
        fn parse_systemd_command() {
            let cli = Cli::try_parse_from(["mtrack", "systemd"]).unwrap();
            assert!(matches!(cli.command, Commands::Systemd {}));
        }

        #[test]
        fn parse_verify_light_show() {
            let cli = Cli::try_parse_from([
                "mtrack",
                "verify-light-show",
                "show.yaml",
                "--config",
                "mtrack.yaml",
            ])
            .unwrap();
            match cli.command {
                Commands::VerifyLightShow { show_path, config } => {
                    assert_eq!(show_path, "show.yaml");
                    assert_eq!(config.as_deref(), Some("mtrack.yaml"));
                }
                _ => panic!("expected VerifyLightShow command"),
            }
        }

        #[test]
        fn parse_calibrate_triggers() {
            let cli = Cli::try_parse_from([
                "mtrack",
                "calibrate-triggers",
                "USB Audio",
                "--sample-rate",
                "48000",
                "--duration",
                "5",
            ])
            .unwrap();
            match cli.command {
                Commands::CalibrateTriggers {
                    device,
                    sample_rate,
                    duration,
                    ..
                } => {
                    assert_eq!(device, "USB Audio");
                    assert_eq!(sample_rate, Some(48000));
                    assert_eq!(duration, 5.0);
                }
                _ => panic!("expected CalibrateTriggers command"),
            }
        }

        #[test]
        fn parse_verify_command() {
            let cli = Cli::try_parse_from([
                "mtrack",
                "verify",
                "mtrack.yaml",
                "--check",
                "track-mappings",
                "--hostname",
                "stage-left",
            ])
            .unwrap();
            match cli.command {
                Commands::Verify {
                    config,
                    check,
                    hostname,
                } => {
                    assert_eq!(config, "mtrack.yaml");
                    assert_eq!(check.as_deref(), Some(&["track-mappings".to_string()][..]));
                    assert_eq!(hostname.as_deref(), Some("stage-left"));
                }
                _ => panic!("expected Verify command"),
            }
        }

        #[test]
        fn missing_required_args_fails() {
            assert!(Cli::try_parse_from(["mtrack", "songs"]).is_err());
            assert!(Cli::try_parse_from(["mtrack", "start"]).is_err());
            assert!(Cli::try_parse_from(["mtrack", "verify"]).is_err());
        }

        #[test]
        fn unknown_command_fails() {
            assert!(Cli::try_parse_from(["mtrack", "nonexistent"]).is_err());
        }
    }
}
