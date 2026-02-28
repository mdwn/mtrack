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
use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent};
use tokio::sync::watch;

use crate::player::Player;
use crate::state::StateSnapshot;

/// Actions the TUI main loop should take after handling an event.
pub enum Action {
    None,
    Quit,
}

/// Cached fixture color for rendering.
pub struct FixtureColor {
    pub name: String,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

/// Application state for the TUI, updated each tick.
pub struct App {
    player: Arc<Player>,
    state_rx: watch::Receiver<Arc<StateSnapshot>>,

    // Playlist state
    pub playlist_name: String,
    pub song_names: Vec<String>,
    pub current_index: usize,

    // Now Playing state
    pub current_song_name: String,
    pub current_song_duration: Duration,
    pub current_song_tracks: Vec<String>,
    pub is_playing: bool,
    pub elapsed: Option<Duration>,

    // Lighting state
    pub fixture_colors: Vec<FixtureColor>,
    pub active_effects: Vec<String>,

    // Log buffer
    pub log_lines: Vec<String>,
}

impl App {
    pub fn new(player: Arc<Player>, state_rx: watch::Receiver<Arc<StateSnapshot>>) -> Self {
        let playlist = player.get_playlist();
        let song_names: Vec<String> = playlist.songs().to_vec();
        let current = playlist.current();

        Self {
            player,
            state_rx,
            playlist_name: playlist.name().to_string(),
            song_names,
            current_index: 0,
            current_song_name: current.name().to_string(),
            current_song_duration: current.duration(),
            current_song_tracks: current
                .tracks()
                .iter()
                .map(|t| t.name().to_string())
                .collect(),
            is_playing: false,
            elapsed: None,
            fixture_colors: Vec::new(),
            active_effects: Vec::new(),
            log_lines: Vec::new(),
        }
    }

    /// Polls the player for current state. Called each tick (~15 FPS).
    pub async fn tick(&mut self) {
        // Update playlist state
        let playlist = self.player.get_playlist();
        self.playlist_name = playlist.name().to_string();
        self.song_names = playlist.songs().to_vec();
        self.current_index = playlist.position();

        let current = playlist.current();
        self.current_song_name = current.name().to_string();
        self.current_song_duration = current.duration();
        self.current_song_tracks = current
            .tracks()
            .iter()
            .map(|t| t.name().to_string())
            .collect();

        // Update playback state
        self.is_playing = self.player.is_playing().await;
        self.elapsed = self.player.elapsed().await.unwrap_or(None);

        // Update lighting state from the shared watch channel
        let snapshot = self.state_rx.borrow_and_update().clone();
        self.fixture_colors = snapshot
            .fixtures
            .iter()
            .map(|f| {
                let r = f.channels.get("red").copied().unwrap_or(0);
                let g = f.channels.get("green").copied().unwrap_or(0);
                let b = f.channels.get("blue").copied().unwrap_or(0);
                FixtureColor {
                    name: f.name.clone(),
                    r,
                    g,
                    b,
                }
            })
            .collect();
        self.active_effects = snapshot.active_effects.clone();

        // Update log buffer (acquire parking_lot mutex off the async runtime).
        if let Some(buffer) = super::logging::get_log_buffer() {
            if let Ok(lines) = tokio::task::spawn_blocking(move || {
                let buffer = buffer.lock();
                buffer.iter().cloned().collect::<Vec<String>>()
            })
            .await
            {
                self.log_lines = lines;
            }
        }
    }

    /// Processes a keyboard event and returns the action to take.
    pub async fn handle_key_event(&mut self, key: KeyEvent) -> Action {
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => {
                self.player.stop().await;
                Action::Quit
            }
            KeyCode::Char(' ') | KeyCode::Enter => {
                if self.is_playing {
                    self.player.stop().await;
                } else {
                    let _ = self.player.play().await;
                }
                Action::None
            }
            KeyCode::Right | KeyCode::Char('n') => {
                self.player.next().await;
                Action::None
            }
            KeyCode::Left | KeyCode::Char('p') => {
                self.player.prev().await;
                Action::None
            }
            KeyCode::Char('a') => {
                self.player.switch_to_all_songs().await;
                Action::None
            }
            KeyCode::Char('l') => {
                self.player.switch_to_playlist().await;
                Action::None
            }
            _ => Action::None,
        }
    }
}
