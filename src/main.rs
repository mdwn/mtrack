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
#[cfg(feature = "simulator")]
mod simulator;
mod songs;
mod state;
#[cfg(test)]
mod testutil;
mod trigger;
mod tui;
mod util;
mod verify;

#[tokio::main]
async fn main() {
    use std::io::IsTerminal;

    let tui_mode = std::io::stdin().is_terminal() && !std::env::args().any(|a| a == "--no-tui");

    // Initialize tracing with a filter that sets default logging to off, with mtrack at info level.
    // This prevents noisy INFO messages from symphonia crates (which are suppressed by the default "off").
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("off,mtrack=info"));

    if tui_mode {
        // In TUI mode, route tracing output to an in-TUI log panel
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;

        let tui_layer = tui::logging::init_tui_logging(1000);
        tracing_subscriber::registry()
            .with(filter)
            .with(tui_layer)
            .init();
    } else {
        // Headless mode: log to stderr as before
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }

    if let Err(e) = cli::run(tui_mode).await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
