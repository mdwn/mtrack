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
mod migrate;
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
ExecStart={{ CURRENT_EXECUTABLE }} start "$MTRACK_PATH"
ExecReload=/bin/kill -HUP $MAINPID

# User and group. Create with:
#   sudo useradd --system --no-create-home --shell /usr/sbin/nologin mtrack
#   sudo usermod -aG audio mtrack
#
# That user also needs read/write access to your library and config directory —
# the $MTRACK_PATH set in /etc/default/mtrack. mtrack writes configuration,
# songs, playlists and lighting files there, and a freshly created system user
# owns none of it. The sandbox settings below govern which paths this service may
# write at all; they do not grant the user permission to write them, which is a
# separate thing and the more common cause of a failed start. Grant it with,
# e.g.:
{{ CHOWN_HINT }}# (or add the mtrack user to a group that already owns the directory).
User=mtrack
Group=mtrack
SupplementaryGroups=audio

# Allow setting thread/RT priority for real-time audio scheduling.
AmbientCapabilities=CAP_SYS_NICE
CapabilityBoundingSet=CAP_SYS_NICE

# Filesystem restrictions. mtrack writes configuration, songs, playlists and
# lighting files to the project directory, and needs that one path writable.
{{ PROTECT_SYSTEM }}PrivateTmp=true
{{ READ_WRITE_PATHS }}

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
    /// Opens the configured audio device and confirms audio is actually flowing.
    ///
    /// Silent: no sound is produced, so this is safe to run before a set with
    /// the PA live. Proves what `devices` cannot — that the device accepts the
    /// configured format and its output callback actually runs.
    TestAudio {
        /// The path to the player configuration.
        config: String,
        /// Override the hostname used to select a hardware profile.
        #[arg(long)]
        hostname: Option<String>,
    },
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
        /// Path to a project directory or config file. If a directory, looks for
        /// mtrack.yaml inside it. If a file (or has .yaml/.yml extension), uses it
        /// directly. Defaults to the current directory.
        #[arg(default_value = ".")]
        path: String,
        /// The path to the playlist. Must be specified if the playlist is not specified in the player config.
        playlist_path: Option<String>,
        /// Enable the terminal UI.
        #[arg(long)]
        tui: bool,
        /// Port for the web UI and lighting simulator (default: 8080).
        #[arg(long, default_value = "8080")]
        web_port: u16,
        /// Bind address for the web UI and lighting simulator (default: 0.0.0.0).
        #[arg(long, default_value = "0.0.0.0")]
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
    /// Seeks within the current song to a time (e.g. "1:23.456" or "45.5s")
    /// or to the start of a named section (e.g. "chorus").
    Seek {
        /// The host and port of the gRPC server.
        #[arg(short = 'H', long)]
        host_port: Option<String>,
        /// The target: a time string or a section name.
        to: String,
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
    Systemd {
        /// Directories the service must be able to write — your library, plus
        /// any songs, playlists, profiles or samples directory configured
        /// outside it. Given at least one, the unit is hardened with
        /// ProtectSystem=strict and excepts exactly these paths.
        paths: Vec<String>,
    },
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
    /// Migrate inline config to directory-based files.
    Migrate {
        /// Path to a project directory or config file.
        #[arg(default_value = ".")]
        path: String,
        /// Actually write changes (default is dry-run).
        #[arg(long)]
        apply: bool,
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
/// A path `systemd` will accept in `ReadWritePaths=`, or why it will not.
///
/// systemd does not fail on a bad value here — it logs and drops the assignment
/// while leaving `ProtectSystem=strict` in force, so the service starts with
/// nothing writable and dies on its first write with `Read-only file system
/// (os error 30)`. That is the cryptic failure this whole change exists to
/// remove, so a path that cannot work is refused at generation time instead.
fn checked_writable_path(path: &str) -> Result<String, String> {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err("a writable path cannot be empty".to_string());
    }
    if !trimmed.starts_with('/') {
        return Err(format!(
            "`{trimmed}` is not absolute. systemd ignores a relative ReadWritePaths and \
             leaves the filesystem read-only, so the service would start and then fail to \
             write. Use the full path."
        ));
    }
    if trimmed.trim_end_matches('/').is_empty() {
        return Err(
            "`/` would make the whole filesystem writable, which leaves ProtectSystem=strict \
             doing nothing. Name the directories mtrack writes instead."
                .to_string(),
        );
    }
    if trimmed.contains('"') || trimmed.contains('\\') {
        return Err(format!(
            "`{trimmed}` contains a quote or backslash, which systemd unquotes into something \
             else. Move the library somewhere without them."
        ));
    }
    // `%` starts a specifier in a unit value: `/srv/100%cotton` becomes
    // something else entirely, and with the leading `-` the resulting path
    // silently does not exist. Doubling escapes it.
    Ok(trimmed.replace('%', "%%"))
}

/// Renders the systemd unit, excepting `writable_paths` from the sandbox.
///
/// With at least one path the unit can name what must stay writable, so the
/// filesystem is locked down with `ProtectSystem=strict`. With none it cannot,
/// and `strict` would leave mtrack unable to write its own config — so it falls
/// back to `full`, which is what the unit shipped before this.
fn render_systemd_service(
    executable_path: &str,
    writable_paths: &[String],
) -> Result<String, Box<dyn Error>> {
    let checked: Vec<String> = writable_paths
        .iter()
        .map(|p| checked_writable_path(p))
        .collect::<Result<_, _>>()?;

    let chown_hint = match checked.first() {
        Some(path) => format!("#   sudo chown -R mtrack:mtrack \"{path}\"\n"),
        None => "#   sudo chown -R mtrack:mtrack /path/to/library\n".to_string(),
    };

    let protect_system = if checked.is_empty() {
        "# /usr, /boot and /efi are read-only; other paths stay writable, since\n\
         # this unit was generated without a library path and cannot name the one\n\
         # directory to except. Regenerate as `mtrack systemd /your/library` to\n\
         # get the stricter sandbox.\n\
         ProtectSystem=full\n"
            .to_string()
    } else {
        "# The whole filesystem is read-only except the paths named below.\n\
         ProtectSystem=strict\n"
            .to_string()
    };

    let read_write_paths = if checked.is_empty() {
        String::new()
    } else {
        let declared = checked
            .iter()
            .map(|p| format!("ReadWritePaths=-\"{p}\"\n"))
            .collect::<String>();
        format!(
            "\n# The paths excepted from ProtectSystem=strict above. Without these the\n\
             # service cannot write its own configuration and fails on startup with\n\
             # `Read-only file system (os error 30)`.\n\
             #\n\
             # These are baked in at generation time and are not read from\n\
             # $MTRACK_PATH — keep them in step with it, and add any songs,\n\
             # playlists, profiles or samples directory configured outside the\n\
             # library, since mtrack writes to those too.\n\
             #\n\
             # The leading `-` keeps a directory that is missing at boot — an\n\
             # unmounted drive, say — from failing the unit outright.\n\
             {declared}"
        )
    };

    Ok(SYSTEMD_SERVICE
        .replace("{{ CURRENT_EXECUTABLE }}", executable_path)
        .replace("{{ CHOWN_HINT }}", &chown_hint)
        .replace("{{ PROTECT_SYSTEM }}", &protect_system)
        .replace("{{ READ_WRITE_PATHS }}\n", &read_write_paths)
        .replace("{{ READ_WRITE_PATHS }}", &read_write_paths))
}

pub async fn run(tui_mode: bool) -> Result<(), Box<dyn Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Songs { path, init } => local::songs(&path, init)?,
        // `list_device_info` rather than `list_devices`: the latter builds every
        // device, and each one holds an ALSA handle that stops its siblings
        // being described, so it under-reports itself -- 8 of 19 on the test
        // rig.
        Commands::Devices {} => print_device_list(audio::list_device_info()?, "No devices found."),
        Commands::MidiDevices {} => print_device_list(midi::list_devices()?, "No devices found."),
        Commands::TestAudio { config, hostname } => local::test_audio(&config, hostname)?,
        Commands::Playlist {
            repository_path,
            playlist_path,
        } => local::playlist(&repository_path, &playlist_path)?,
        Commands::Start {
            path,
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
            local::start(&path, playlist_path, web_config, effective_tui).await?
        }
        Commands::Play { host_port, from } => remote::play(host_port, from).await?,
        Commands::Seek { host_port, to } => remote::seek(host_port, &to).await?,
        Commands::Previous { host_port } => remote::previous(host_port).await?,
        Commands::Next { host_port } => remote::next(host_port).await?,
        Commands::Stop { host_port } => remote::stop(host_port).await?,
        Commands::SwitchToPlaylist {
            host_port,
            playlist_name,
        } => remote::switch_to_playlist(host_port, &playlist_name).await?,
        Commands::Status { host_port } => remote::status(host_port).await?,
        Commands::ActiveEffects { host_port } => remote::active_effects(host_port).await?,
        Commands::Systemd { paths } => {
            let current_executable_path = env::current_exe()?;
            println!(
                "{}",
                render_systemd_service(&current_executable_path.to_string_lossy(), &paths)?
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
        Commands::Migrate { path, apply } => migrate::migrate(&path, apply)?,
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
            let result = render_systemd_service("/usr/local/bin/mtrack", &[]).unwrap();
            assert!(result.contains("ExecStart=/usr/local/bin/mtrack start"));
            assert!(!result.contains("{{ CURRENT_EXECUTABLE }}"));
        }

        #[test]
        fn preserves_service_structure() {
            let result = render_systemd_service("/usr/bin/mtrack", &[]).unwrap();
            assert!(result.contains("[Unit]"));
            assert!(result.contains("[Service]"));
            assert!(result.contains("[Install]"));
            assert!(result.contains("Type=simple"));
        }

        #[test]
        fn preserves_hardening_directives() {
            let result = render_systemd_service("/usr/bin/mtrack", &[]).unwrap();
            assert!(result.contains("ProtectSystem=full"));
            assert!(result.contains("NoNewPrivileges=true"));
            assert!(result.contains("MemoryDenyWriteExecute=true"));
        }

        /// The unit has to say that the mtrack user needs access to the
        /// library, whether or not the path is known.
        ///
        /// #351: the generated unit creates a system user and points at
        /// `$MTRACK_PATH`, but a freshly created user owns none of that
        /// directory. `ProtectSystem=full` leaves it writable as far as the
        /// sandbox is concerned, which is a different thing from the user being
        /// permitted to write it, and nothing pointed at permissions as the
        /// cause when the service then failed.
        #[test]
        fn documents_the_library_permissions_without_a_path() {
            let result = render_systemd_service("/usr/local/bin/mtrack", &[]).unwrap();
            assert!(result.contains("read/write access"));
            assert!(
                result.contains("sudo chown -R mtrack:mtrack /path/to/library"),
                "the hint has to be pasteable even when the path is unknown"
            );
            assert!(
                !result.contains("ReadWritePaths"),
                "there is no path to declare, and an empty directive would be invalid"
            );
        }

        #[test]
        fn names_the_library_when_it_is_given() {
            let result =
                render_systemd_service("/usr/local/bin/mtrack", &["/srv/songs".to_string()])
                    .unwrap();
            assert!(result.contains("sudo chown -R mtrack:mtrack \"/srv/songs\""));
            assert!(
                result.contains("ReadWritePaths=-\"/srv/songs\""),
                "declaring it keeps the unit correct if the sandbox is ever tightened to strict"
            );
            assert!(
                !result.contains("/path/to/library"),
                "the placeholder must not survive alongside the real path"
            );
        }

        /// A library path containing a space must not read as two paths.
        ///
        /// `ExecStart` already quotes `"$MTRACK_PATH"` for this reason; the
        /// hint and the directive have the same problem — `chown` would take
        /// two arguments and systemd two paths.
        #[test]
        fn quotes_a_library_path_with_spaces() {
            let result =
                render_systemd_service("/usr/local/bin/mtrack", &["/opt/my songs".to_string()])
                    .unwrap();
            assert!(result.contains("sudo chown -R mtrack:mtrack \"/opt/my songs\""));
            assert!(result.contains("ReadWritePaths=-\"/opt/my songs\""));
        }

        /// A missing library directory must not stop the service starting.
        ///
        /// systemd refuses to start a unit whose `ReadWritePaths` does not
        /// exist, and this unit is generated before the library necessarily
        /// does — the deployment guide has the operator create the user, then
        /// generate the unit. Without the leading `-` this change would have
        /// introduced the same class of cryptic startup failure #351 is about.
        #[test]
        fn a_missing_library_does_not_fail_the_unit() {
            let result =
                render_systemd_service("/usr/local/bin/mtrack", &["/srv/songs".to_string()])
                    .unwrap();
            assert!(
                result.contains("ReadWritePaths=-"),
                "the directive has to tolerate a path that does not exist yet"
            );
        }

        /// A path systemd would silently drop is refused at generation time.
        ///
        /// systemd does not fail on a bad `ReadWritePaths=`: it logs, drops the
        /// assignment, and leaves `ProtectSystem=strict` in force — so the
        /// service starts with nothing writable and dies on its first write.
        /// Emitting such a unit would reintroduce the cryptic failure this
        /// change removes. All three were confirmed against
        /// `systemd-analyze verify`.
        #[test]
        fn refuses_a_path_systemd_would_drop() {
            let relative = render_systemd_service("/usr/bin/mtrack", &["songs".to_string()]);
            assert!(
                relative.is_err(),
                "a relative path is ignored by systemd, leaving the filesystem read-only"
            );

            let empty = render_systemd_service("/usr/bin/mtrack", &[String::new()]);
            assert!(empty.is_err());

            let root = render_systemd_service("/usr/bin/mtrack", &["/".to_string()]);
            assert!(
                root.is_err(),
                "`/` makes everything writable, so strict would do nothing while reading as \
                 hardened"
            );
        }

        /// A `%` in a path is a systemd specifier and has to be escaped.
        ///
        /// `/srv/100%cotton` expands `%c` into something else, and with the
        /// leading `-` the resulting path silently does not exist — strict with
        /// no working exception.
        #[test]
        fn escapes_a_specifier_in_a_path() {
            let result =
                render_systemd_service("/usr/bin/mtrack", &["/srv/100%cotton".to_string()])
                    .unwrap();
            assert!(
                result.contains("ReadWritePaths=-\"/srv/100%%cotton\""),
                "the percent must be doubled or systemd rewrites the path: {result}"
            );
        }

        /// Every writable directory is declared, not just the library.
        ///
        /// `songs`, `playlists_dir`, `profiles_dir` and `samples` are used as
        /// given when absolute, so a library at /var/lib/mtrack with
        /// `songs: /mnt/nas/songs` writes to both. Under strict, a directory
        /// that is not named cannot be written.
        #[test]
        fn declares_every_writable_path() {
            let result = render_systemd_service(
                "/usr/bin/mtrack",
                &["/var/lib/mtrack".to_string(), "/mnt/nas/songs".to_string()],
            )
            .unwrap();
            assert!(result.contains("ReadWritePaths=-\"/var/lib/mtrack\""));
            assert!(result.contains("ReadWritePaths=-\"/mnt/nas/songs\""));
            assert!(sets_directive(&result, "ProtectSystem=strict"));
        }

        /// systemd itself must accept the unit we generate.
        ///
        /// The other tests here match strings, and every directive systemd
        /// silently drops renders and passes them — a relative
        /// `ReadWritePaths`, an unescaped `%` specifier — leaving
        /// `ProtectSystem=strict` in force with nothing writable. The container
        /// test catches that but only runs on main, so this asks systemd's own
        /// validator on every run instead.
        ///
        /// Skipped where `systemd-analyze` is not installed, but never on Linux
        /// CI, where its absence fails rather than quietly retiring the check.
        #[test]
        fn systemd_accepts_the_generated_unit() {
            let available = std::process::Command::new("systemd-analyze")
                .arg("--version")
                .output()
                .is_ok_and(|probe| probe.status.success());
            if !available {
                // A skip and a pass look identical in CI output, so on Linux CI
                // -- where systemd-analyze is expected -- refuse to skip. Losing
                // the tool there would otherwise retire this check silently.
                assert!(
                    !(cfg!(target_os = "linux") && std::env::var_os("CI").is_some()),
                    "systemd-analyze is missing on Linux CI: the generated unit \
                     is no longer being validated by systemd"
                );
                eprintln!("skipping: systemd-analyze is not installed");
                return;
            }

            let dir = tempfile::tempdir().expect("tempdir");
            let library = dir.path().join("library");
            std::fs::create_dir(&library).expect("library");

            for paths in [
                vec![],
                vec![library.to_string_lossy().into_owned()],
                vec![
                    library.to_string_lossy().into_owned(),
                    dir.path().join("songs").to_string_lossy().into_owned(),
                ],
            ] {
                // `/bin/sh` because it exists: an ExecStart that does not draws
                // its own warning and says nothing about the sandbox.
                let unit = render_systemd_service("/bin/sh", &paths).unwrap();
                let path = dir.path().join("mtrack.service");
                std::fs::write(&path, &unit).expect("write unit");

                let output = std::process::Command::new("systemd-analyze")
                    .arg("verify")
                    .arg(&path)
                    .output()
                    .expect("systemd-analyze verify");
                let complaints = format!(
                    "{}{}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                );

                for directive in ["ReadWritePaths", "ProtectSystem", "Specifier"] {
                    assert!(
                        !complaints.contains(directive),
                        "systemd objected to {directive} for {paths:?}: {complaints}"
                    );
                }
            }
        }

        /// The template must not leak a placeholder into the rendered unit.
        #[test]
        fn leaves_no_placeholders_behind() {
            for paths in [vec![], vec!["/srv/songs".to_string()]] {
                let result = render_systemd_service("/usr/local/bin/mtrack", &paths).unwrap();
                assert!(
                    !result.contains("{{"),
                    "an unrendered placeholder survived for {paths:?}: {result}"
                );
            }
        }

        #[test]
        fn handles_path_with_spaces() {
            let result = render_systemd_service("/opt/my apps/mtrack", &[]).unwrap();
            assert!(result.contains("ExecStart=/opt/my apps/mtrack start"));
        }

        #[test]
        fn does_not_contain_protect_home() {
            // ProtectHome was removed because the project directory is often
            // under /home and mtrack needs write access to it.
            let result = render_systemd_service("/usr/bin/mtrack", &[]).unwrap();
            assert!(
                !result.contains("ProtectHome"),
                "ProtectHome should not be in the systemd template — the project \
                 directory may be under /home and mtrack needs write access to it"
            );
        }

        /// Whether the unit sets a directive, ignoring any comment that
        /// mentions it. A substring search cannot tell the two apart, and the
        /// generated unit deliberately names directives in its own prose.
        fn sets_directive(unit: &str, directive: &str) -> bool {
            unit.lines()
                .any(|line| line.trim_start().starts_with(directive))
        }

        #[test]
        fn does_not_contain_protect_system_strict_without_a_library_path() {
            // Without a path the unit cannot name the directory to except, and
            // `strict` would leave mtrack unable to write its own config: the
            // service fails on startup with `Read-only file system (os error
            // 30)`, verified in the container test.
            let result = render_systemd_service("/usr/bin/mtrack", &[]).unwrap();
            assert!(
                !sets_directive(&result, "ProtectSystem=strict"),
                "ProtectSystem=strict with no ReadWritePaths makes the project directory \
                 read-only and the service fails to start"
            );
        }

        /// `strict` and a writable library must arrive together, always.
        ///
        /// Either alone is a broken unit: `strict` without the exception cannot
        /// write the config, and this is the pairing the whole hardening rests
        /// on, so it is asserted rather than left to the two branches agreeing
        /// by construction.
        #[test]
        fn protect_system_strict_never_appears_without_a_writable_library() {
            for paths in [
                vec![],
                vec!["/srv/songs".to_string()],
                vec!["/opt/my songs".to_string()],
            ] {
                let result = render_systemd_service("/usr/bin/mtrack", &paths).unwrap();
                if sets_directive(&result, "ProtectSystem=strict") {
                    assert!(
                        sets_directive(&result, "ReadWritePaths="),
                        "strict without a writable path for {paths:?}: the service could not \
                         write its own configuration"
                    );
                }
                if sets_directive(&result, "ReadWritePaths=") {
                    assert!(
                        sets_directive(&result, "ProtectSystem=strict"),
                        "a writable path was declared for {paths:?} but the filesystem was not \
                         restricted, so the declaration does nothing"
                    );
                }
            }
        }

        /// A library path buys the stricter sandbox.
        #[test]
        fn a_library_path_hardens_the_filesystem() {
            let result =
                render_systemd_service("/usr/bin/mtrack", &["/srv/songs".to_string()]).unwrap();
            assert!(sets_directive(&result, "ProtectSystem=strict"));
            assert!(!sets_directive(&result, "ProtectSystem=full"));
        }

        #[test]
        fn allows_write_access_to_project_dir() {
            // Without a library path: `full` makes /usr, /boot and /efi
            // read-only and leaves everything else writable, so mtrack can
            // still write config, songs, playlists and lighting files.
            let result = render_systemd_service("/usr/bin/mtrack", &[]).unwrap();
            assert!(
                sets_directive(&result, "ProtectSystem=full"),
                "with no library path to except, ProtectSystem must stay 'full' or the \
                 project directory becomes read-only"
            );
            assert!(!sets_directive(&result, "ProtectSystem=strict"));
        }

        #[test]
        fn uses_environment_file() {
            // MTRACK_PATH is defined in /etc/default/mtrack
            let result = render_systemd_service("/usr/bin/mtrack", &[]).unwrap();
            assert!(result.contains("EnvironmentFile=-/etc/default/mtrack"));
            assert!(result.contains("\"$MTRACK_PATH\""));
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
                    path,
                    playlist_path,
                    tui,
                    web_port,
                    web_address,
                } => {
                    assert_eq!(path, "config.yaml");
                    assert!(playlist_path.is_none());
                    assert!(!tui);
                    assert_eq!(web_port, 8080);
                    assert_eq!(web_address, "0.0.0.0");
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
                    path,
                    playlist_path,
                    tui,
                    web_port,
                    web_address,
                } => {
                    assert_eq!(path, "config.yaml");
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
            assert!(matches!(cli.command, Commands::Systemd { ref paths } if paths.is_empty()));
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
            assert!(Cli::try_parse_from(["mtrack", "verify"]).is_err());
        }

        #[test]
        fn start_defaults_to_current_dir() {
            let cli = Cli::try_parse_from(["mtrack", "start"]).unwrap();
            match cli.command {
                Commands::Start { path, .. } => {
                    assert_eq!(path, ".");
                }
                _ => panic!("expected Start command"),
            }
        }

        #[test]
        fn unknown_command_fails() {
            assert!(Cli::try_parse_from(["mtrack", "nonexistent"]).is_err());
        }

        #[test]
        fn parse_playlist_command() {
            let cli =
                Cli::try_parse_from(["mtrack", "playlist", "/path/to/songs", "playlist.yaml"])
                    .unwrap();
            match cli.command {
                Commands::Playlist {
                    repository_path,
                    playlist_path,
                } => {
                    assert_eq!(repository_path, "/path/to/songs");
                    assert_eq!(playlist_path, "playlist.yaml");
                }
                _ => panic!("expected Playlist command"),
            }
        }

        #[test]
        fn parse_playlist_missing_args_fails() {
            assert!(Cli::try_parse_from(["mtrack", "playlist"]).is_err());
            assert!(Cli::try_parse_from(["mtrack", "playlist", "/path"]).is_err());
        }

        #[test]
        fn parse_previous_command() {
            let cli = Cli::try_parse_from(["mtrack", "previous"]).unwrap();
            match cli.command {
                Commands::Previous { host_port } => {
                    assert!(host_port.is_none());
                }
                _ => panic!("expected Previous command"),
            }
        }

        #[test]
        fn parse_previous_with_host() {
            let cli = Cli::try_parse_from(["mtrack", "previous", "-H", "localhost:50051"]).unwrap();
            match cli.command {
                Commands::Previous { host_port } => {
                    assert_eq!(host_port.as_deref(), Some("localhost:50051"));
                }
                _ => panic!("expected Previous command"),
            }
        }

        #[test]
        fn parse_next_command() {
            let cli = Cli::try_parse_from(["mtrack", "next"]).unwrap();
            match cli.command {
                Commands::Next { host_port } => {
                    assert!(host_port.is_none());
                }
                _ => panic!("expected Next command"),
            }
        }

        #[test]
        fn parse_next_with_short_host_flag() {
            let cli = Cli::try_parse_from(["mtrack", "next", "-H", "192.168.1.1:43234"]).unwrap();
            match cli.command {
                Commands::Next { host_port } => {
                    assert_eq!(host_port.as_deref(), Some("192.168.1.1:43234"));
                }
                _ => panic!("expected Next command"),
            }
        }

        #[test]
        fn parse_stop_command() {
            let cli = Cli::try_parse_from(["mtrack", "stop"]).unwrap();
            match cli.command {
                Commands::Stop { host_port } => {
                    assert!(host_port.is_none());
                }
                _ => panic!("expected Stop command"),
            }
        }

        #[test]
        fn parse_stop_with_host() {
            let cli =
                Cli::try_parse_from(["mtrack", "stop", "--host-port", "10.0.0.1:43234"]).unwrap();
            match cli.command {
                Commands::Stop { host_port } => {
                    assert_eq!(host_port.as_deref(), Some("10.0.0.1:43234"));
                }
                _ => panic!("expected Stop command"),
            }
        }

        #[test]
        fn parse_status_command() {
            let cli = Cli::try_parse_from(["mtrack", "status"]).unwrap();
            match cli.command {
                Commands::Status { host_port } => {
                    assert!(host_port.is_none());
                }
                _ => panic!("expected Status command"),
            }
        }

        #[test]
        fn parse_status_with_host() {
            let cli = Cli::try_parse_from(["mtrack", "status", "-H", "localhost:9999"]).unwrap();
            match cli.command {
                Commands::Status { host_port } => {
                    assert_eq!(host_port.as_deref(), Some("localhost:9999"));
                }
                _ => panic!("expected Status command"),
            }
        }

        #[test]
        fn parse_active_effects_command() {
            let cli = Cli::try_parse_from(["mtrack", "active-effects"]).unwrap();
            match cli.command {
                Commands::ActiveEffects { host_port } => {
                    assert!(host_port.is_none());
                }
                _ => panic!("expected ActiveEffects command"),
            }
        }

        #[test]
        fn parse_active_effects_with_host() {
            let cli = Cli::try_parse_from(["mtrack", "active-effects", "--host-port", "host:1234"])
                .unwrap();
            match cli.command {
                Commands::ActiveEffects { host_port } => {
                    assert_eq!(host_port.as_deref(), Some("host:1234"));
                }
                _ => panic!("expected ActiveEffects command"),
            }
        }

        #[test]
        fn parse_cues_command() {
            let cli = Cli::try_parse_from(["mtrack", "cues"]).unwrap();
            match cli.command {
                Commands::Cues { host_port } => {
                    assert!(host_port.is_none());
                }
                _ => panic!("expected Cues command"),
            }
        }

        #[test]
        fn parse_cues_with_host() {
            let cli = Cli::try_parse_from(["mtrack", "cues", "-H", "server:43234"]).unwrap();
            match cli.command {
                Commands::Cues { host_port } => {
                    assert_eq!(host_port.as_deref(), Some("server:43234"));
                }
                _ => panic!("expected Cues command"),
            }
        }

        #[test]
        fn parse_play_no_args() {
            let cli = Cli::try_parse_from(["mtrack", "play"]).unwrap();
            match cli.command {
                Commands::Play { host_port, from } => {
                    assert!(host_port.is_none());
                    assert!(from.is_none());
                }
                _ => panic!("expected Play command"),
            }
        }

        #[test]
        fn parse_play_with_both_host_and_from() {
            let cli = Cli::try_parse_from([
                "mtrack",
                "play",
                "-H",
                "localhost:50051",
                "--from",
                "2:30.000",
            ])
            .unwrap();
            match cli.command {
                Commands::Play { host_port, from } => {
                    assert_eq!(host_port.as_deref(), Some("localhost:50051"));
                    assert_eq!(from.as_deref(), Some("2:30.000"));
                }
                _ => panic!("expected Play command"),
            }
        }

        #[test]
        fn parse_switch_to_playlist_with_host() {
            let cli = Cli::try_parse_from([
                "mtrack",
                "switch-to-playlist",
                "-H",
                "server:43234",
                "playlist",
            ])
            .unwrap();
            match cli.command {
                Commands::SwitchToPlaylist {
                    host_port,
                    playlist_name,
                } => {
                    assert_eq!(host_port.as_deref(), Some("server:43234"));
                    assert_eq!(playlist_name, "playlist");
                }
                _ => panic!("expected SwitchToPlaylist command"),
            }
        }

        #[test]
        fn parse_switch_to_playlist_missing_name_fails() {
            assert!(Cli::try_parse_from(["mtrack", "switch-to-playlist"]).is_err());
        }

        #[test]
        fn parse_verify_light_show_without_config() {
            let cli = Cli::try_parse_from(["mtrack", "verify-light-show", "show.light"]).unwrap();
            match cli.command {
                Commands::VerifyLightShow { show_path, config } => {
                    assert_eq!(show_path, "show.light");
                    assert!(config.is_none());
                }
                _ => panic!("expected VerifyLightShow command"),
            }
        }

        #[test]
        fn parse_calibrate_triggers_defaults() {
            let cli = Cli::try_parse_from(["mtrack", "calibrate-triggers", "USB Audio"]).unwrap();
            match cli.command {
                Commands::CalibrateTriggers {
                    device,
                    sample_rate,
                    duration,
                    sample_format,
                    bits_per_sample,
                } => {
                    assert_eq!(device, "USB Audio");
                    assert!(sample_rate.is_none());
                    assert_eq!(duration, 3.0); // default
                    assert!(sample_format.is_none());
                    assert!(bits_per_sample.is_none());
                }
                _ => panic!("expected CalibrateTriggers command"),
            }
        }

        #[test]
        fn parse_calibrate_triggers_with_all_options() {
            let cli = Cli::try_parse_from([
                "mtrack",
                "calibrate-triggers",
                "USB Audio",
                "--sample-rate",
                "48000",
                "--duration",
                "5",
                "--sample-format",
                "float",
                "--bits-per-sample",
                "32",
            ])
            .unwrap();
            match cli.command {
                Commands::CalibrateTriggers {
                    device,
                    sample_rate,
                    duration,
                    sample_format,
                    bits_per_sample,
                } => {
                    assert_eq!(device, "USB Audio");
                    assert_eq!(sample_rate, Some(48000));
                    assert_eq!(duration, 5.0);
                    assert_eq!(sample_format.as_deref(), Some("float"));
                    assert_eq!(bits_per_sample, Some(32));
                }
                _ => panic!("expected CalibrateTriggers command"),
            }
        }

        #[test]
        fn parse_verify_without_optional_args() {
            let cli = Cli::try_parse_from(["mtrack", "verify", "mtrack.yaml"]).unwrap();
            match cli.command {
                Commands::Verify {
                    config,
                    check,
                    hostname,
                } => {
                    assert_eq!(config, "mtrack.yaml");
                    assert!(check.is_none());
                    assert!(hostname.is_none());
                }
                _ => panic!("expected Verify command"),
            }
        }

        #[test]
        fn parse_verify_with_multiple_checks() {
            let cli = Cli::try_parse_from([
                "mtrack",
                "verify",
                "mtrack.yaml",
                "--check",
                "track-mappings",
                "--check",
                "other-check",
            ])
            .unwrap();
            match cli.command {
                Commands::Verify { check, .. } => {
                    let checks = check.unwrap();
                    assert_eq!(checks.len(), 2);
                    assert!(checks.contains(&"track-mappings".to_string()));
                    assert!(checks.contains(&"other-check".to_string()));
                }
                _ => panic!("expected Verify command"),
            }
        }

        #[test]
        fn parse_start_with_playlist_path() {
            let cli =
                Cli::try_parse_from(["mtrack", "start", "config.yaml", "custom-playlist.yaml"])
                    .unwrap();
            match cli.command {
                Commands::Start {
                    path,
                    playlist_path,
                    ..
                } => {
                    assert_eq!(path, "config.yaml");
                    assert_eq!(playlist_path.as_deref(), Some("custom-playlist.yaml"));
                }
                _ => panic!("expected Start command"),
            }
        }

        #[test]
        fn parse_migrate_defaults() {
            let cli = Cli::try_parse_from(["mtrack", "migrate"]).unwrap();
            match cli.command {
                Commands::Migrate { path, apply } => {
                    assert_eq!(path, ".");
                    assert!(!apply);
                }
                _ => panic!("expected Migrate command"),
            }
        }

        #[test]
        fn parse_migrate_with_path_and_apply() {
            let cli =
                Cli::try_parse_from(["mtrack", "migrate", "/path/to/project", "--apply"]).unwrap();
            match cli.command {
                Commands::Migrate { path, apply } => {
                    assert_eq!(path, "/path/to/project");
                    assert!(apply);
                }
                _ => panic!("expected Migrate command"),
            }
        }

        #[test]
        fn no_subcommand_fails() {
            assert!(Cli::try_parse_from(["mtrack"]).is_err());
        }
    }
}
