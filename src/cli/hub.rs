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
//! `mtrack hub`: the hub client's commands ([`crate::hub`]). Only in a
//! player built with the `hub` feature.

use std::error::Error;

use clap::Subcommand;

use crate::hub::sync::{pair, Pulled, Rig, Uploaded};

#[derive(Subcommand)]
pub enum HubCommand {
    /// Pairs a project with the hub: shows a code to enter there.
    Pair {
        /// A project directory or its mtrack.yaml.
        #[arg(default_value = ".")]
        path: String,
        /// The hub's address.
        #[arg(long, default_value = crate::hub::DEFAULT_HUB)]
        hub: String,
        /// What this rig is called in the hub. This machine's name if not
        /// given.
        #[arg(long)]
        name: Option<String>,
    },
    /// Sends the project to the hub as its next revision.
    Upload {
        /// A project directory or its mtrack.yaml.
        #[arg(default_value = ".")]
        path: String,
    },
    /// Brings the project to the hub's current revision. Stop the player
    /// first: a running one is not told, and a file it writes meanwhile stops
    /// the pull.
    Pull {
        /// A project directory or its mtrack.yaml.
        #[arg(default_value = ".")]
        path: String,
        /// Replace files changed here since the last pull or upload. They
        /// are kept until the next pull, for `undo`.
        #[arg(long)]
        discard_local_changes: bool,
    },
    /// Puts back what the last pull replaced. Needs no network.
    Undo {
        /// A project directory or its mtrack.yaml.
        #[arg(default_value = ".")]
        path: String,
        /// Undo although files here changed since the pull. Those the pull
        /// had changed are replaced; those it had not stay as they are.
        #[arg(long)]
        discard_local_changes: bool,
    },
    /// Says what the project is paired to and where it stands. Finishes a
    /// pull that was cut short.
    Status {
        /// A project directory or its mtrack.yaml.
        #[arg(default_value = ".")]
        path: String,
    },
}

pub async fn run(command: HubCommand) -> Result<(), Box<dyn Error>> {
    match command {
        HubCommand::Pair { path, hub, name } => {
            let config = super::migrate::resolve_config_path(&path);
            let name = name.unwrap_or_else(crate::config::resolve_hostname);
            let paired = pair(&config, &hub, &name, |started| {
                println!(
                    "Enter this code at {}:\n\n    {}\n\nWaiting for it to be entered...",
                    started.verification_uri, started.user_code
                );
            })
            .await?;
            println!(
                "Paired as \"{}\" with the project \"{}\".",
                paired.rig.name, paired.project.name
            );
        }
        HubCommand::Upload { path } => {
            let rig = Rig::open(&super::migrate::resolve_config_path(&path))?;
            match rig.upload().await? {
                Uploaded::UpToDate(revision) => {
                    println!("The hub already has this as revision {}.", revision.number)
                }
                Uploaded::Saved {
                    revision,
                    sent,
                    report,
                } => {
                    println!(
                        "Saved as revision {} ({sent} file(s) sent): {} song(s), {} playlist(s).",
                        revision.number, report.songs, report.playlists
                    );
                    for (file, problem) in &report.problems {
                        println!("  {} does not load: {problem}", file.display());
                    }
                }
            }
        }
        HubCommand::Pull {
            path,
            discard_local_changes,
        } => {
            let rig = Rig::open(&super::migrate::resolve_config_path(&path))?;
            match rig.pull(discard_local_changes).await? {
                Pulled::UpToDate(revision) => {
                    println!("Already at revision {}.", revision.number)
                }
                Pulled::Updated {
                    revision,
                    fetched,
                    report,
                } => {
                    println!(
                        "Now at revision {} ({fetched} file(s) fetched): {} song(s), {} playlist(s).",
                        revision.number, report.songs, report.playlists
                    );
                    for (file, problem) in &report.problems {
                        println!("  {} does not load: {problem}", file.display());
                    }
                }
            }
        }
        HubCommand::Undo {
            path,
            discard_local_changes,
        } => {
            let rig = Rig::open(&super::migrate::resolve_config_path(&path))?;
            match rig.undo(discard_local_changes).await? {
                Some(revision) => println!("Back at revision {}.", revision.number),
                None => println!("Back to the files from before the first pull."),
            }
        }
        HubCommand::Status { path } => {
            let rig = Rig::open(&super::migrate::resolve_config_path(&path))?;
            let status = rig.status()?;
            println!(
                "Paired as \"{}\" with the project \"{}\" at {}.",
                status.credentials.rig.name,
                status.credentials.project.name,
                status.credentials.hub
            );
            match status.revision {
                Some(revision) => println!("At revision {}.", revision.number),
                None => println!("Never pulled or uploaded."),
            }
            if status.local_changes {
                println!("Files here have changed since.");
            }
            if status.undoable {
                println!("The last pull can be undone.");
            }
        }
    }
    Ok(())
}
