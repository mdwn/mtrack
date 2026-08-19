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
use mtrack::cli;
use mtrack::tui;

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

    // A panicking background task otherwise dies with output only on stderr —
    // invisible headless under systemd journal filtering and in the web UI log
    // stream. Log it through tracing first, then let the default hook (or the
    // TUI's terminal-restoring hook, which chains onto this one) run.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        tracing::error!("Thread panicked: {panic_info}");
        default_hook(panic_info);
    }));

    // Rayon's global pool (waveform generation) aborts the process if a job
    // panics and no handler is installed. Install one before first use so a
    // panic there logs and ends that job instead of ending playback.
    if let Err(e) = rayon::ThreadPoolBuilder::new()
        .panic_handler(|payload| {
            tracing::error!(
                "Panic in rayon global thread pool: {}",
                mtrack::util::panic_message(payload.as_ref())
            )
        })
        .build_global()
    {
        // Already initialized elsewhere; nothing to do.
        tracing::debug!("Global rayon pool already initialized: {e}");
    }

    if let Err(e) = cli::run(tui_mode).await {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
