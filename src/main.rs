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
mod build_info;
mod calibrate;
mod cli;
mod clock;
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
mod state;
#[cfg(test)]
mod testutil;
mod thread_priority;
mod trigger;
mod tui;
mod util;
mod verify;
mod webui;

#[tokio::main]
async fn main() {
    let tui_mode = std::env::args().any(|a| a == "--tui");

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
        // Headless: log to stderr AND capture in ring buffer for web UI log streaming
        use tracing_subscriber::layer::SubscriberExt;
        use tracing_subscriber::util::SubscriberInitExt;

        let tui_layer = tui::logging::init_tui_logging(1000);
        let fmt_layer = tracing_subscriber::fmt::layer();
        tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .with(tui_layer)
            .init();
    }

    if let Err(e) = cli::run(tui_mode).await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
