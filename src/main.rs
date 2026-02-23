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
mod calibrate;
mod cli;
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
mod simulator;
mod songs;
#[cfg(test)]
mod testutil;
mod trigger;
mod util;
mod verify;

#[tokio::main]
async fn main() {
    // Initialize tracing with a filter that sets default logging to off, with mtrack at info level.
    // This prevents noisy INFO messages from symphonia crates (which are suppressed by the default "off").
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("off,mtrack=info"));

    tracing_subscriber::fmt().with_env_filter(filter).init();

    if let Err(e) = cli::run().await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
