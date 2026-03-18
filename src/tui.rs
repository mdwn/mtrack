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
mod app;
pub mod logging;
mod ui;

use std::error::Error;
use std::io;
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use tokio::sync::{mpsc, watch};

use crate::player::Player;
use crate::state::StateSnapshot;

use app::{Action, App};

/// Runs the TUI as the main blocking loop.
///
/// Controllers (gRPC/OSC/MIDI) continue running in background tokio tasks.
/// The TUI replaces `Controller::join()` as the main loop when stdin is a TTY.
pub async fn run(
    player: Arc<Player>,
    state_rx: watch::Receiver<Arc<StateSnapshot>>,
) -> Result<(), Box<dyn Error>> {
    // Set up terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    // Install a panic hook that restores the terminal before printing the panic
    let original_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
        original_hook(panic_info);
    }));

    // Spawn a dedicated OS thread for crossterm event reading.
    // crossterm::event::read() is blocking, so we can't use it in async directly.
    let (event_tx, mut event_rx) = mpsc::channel::<Event>(32);
    std::thread::Builder::new()
        .name("tui-events".to_string())
        .spawn(move || {
            loop {
                // Poll with a timeout so the thread can detect when the receiver is dropped
                if event::poll(Duration::from_millis(100)).unwrap_or(false) {
                    if let Ok(evt) = event::read() {
                        if event_tx.blocking_send(evt).is_err() {
                            // Receiver dropped, TUI is shutting down
                            break;
                        }
                    }
                }
            }
        })?;

    let mut app = App::new(player, state_rx);

    // Initial tick to populate state before first render
    app.tick().await;

    let tick_rate = Duration::from_millis(66); // ~15 FPS

    let result = run_loop(&mut terminal, &mut app, &mut event_rx, tick_rate).await;

    // Restore terminal
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    result
}

/// The main event loop.
async fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
    event_rx: &mut mpsc::Receiver<Event>,
    tick_rate: Duration,
) -> Result<(), Box<dyn Error>> {
    let mut tick_interval = tokio::time::interval(tick_rate);

    loop {
        // Draw
        terminal.draw(|frame| ui::draw(frame, app))?;

        tokio::select! {
            _ = tick_interval.tick() => {
                app.tick().await;
            }
            Some(event) = event_rx.recv() => {
                if let Event::Key(key) = event {
                    // Only handle key press events (not release/repeat)
                    if key.kind == KeyEventKind::Press {
                        match app.handle_key_event(key).await {
                            Action::Quit => return Ok(()),
                            Action::None => {}
                        }
                    }
                }
            }
        }
    }
}
