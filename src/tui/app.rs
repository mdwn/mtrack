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
use crate::state::{FixtureSnapshot, StateSnapshot};

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
            current_song_name: current
                .as_ref()
                .map(|s| s.name().to_string())
                .unwrap_or_default(),
            current_song_duration: current
                .as_ref()
                .map(|s| s.duration())
                .unwrap_or(std::time::Duration::ZERO),
            current_song_tracks: current
                .as_ref()
                .map(|s| s.tracks().iter().map(|t| t.name().to_string()).collect())
                .unwrap_or_default(),
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

        if let Some(current) = playlist.current() {
            self.current_song_name = current.name().to_string();
            self.current_song_duration = current.duration();
            self.current_song_tracks = current
                .tracks()
                .iter()
                .map(|t| t.name().to_string())
                .collect();
        } else {
            self.current_song_name = String::new();
            self.current_song_duration = std::time::Duration::ZERO;
            self.current_song_tracks = vec![];
        }

        // Update playback state
        self.is_playing = self.player.is_playing().await;
        self.elapsed = self.player.elapsed().await.unwrap_or(None);

        // Update lighting state from the shared watch channel
        let snapshot = self.state_rx.borrow_and_update().clone();
        self.fixture_colors = fixture_colors_from_snapshot(&snapshot.fixtures);
        self.active_effects = snapshot.active_effects.clone();

        // Update log buffer (acquire parking_lot mutex off the async runtime).
        if let Some(buffer) = super::logging::get_log_buffer() {
            let buffer = buffer.lock();
            self.log_lines = buffer.iter().cloned().collect();
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
                let _ = self.player.switch_to_playlist("all_songs").await;
                Action::None
            }
            KeyCode::Char('l') => {
                let name = self.player.persisted_playlist_name();
                let _ = self.player.switch_to_playlist(&name).await;
                Action::None
            }
            _ => Action::None,
        }
    }
}

/// Converts fixture snapshots into display-ready colors by extracting RGB channels.
fn fixture_colors_from_snapshot(fixtures: &[FixtureSnapshot]) -> Vec<FixtureColor> {
    fixtures
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
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    use crate::config;
    use crate::player::PlayerDevices;
    use crate::playlist;
    use crate::songs::{Song, Songs};

    fn make_fixture(name: &str, channels: &[(&str, u8)]) -> FixtureSnapshot {
        let mut map = HashMap::new();
        for (k, v) in channels {
            map.insert(k.to_string(), *v);
        }
        FixtureSnapshot {
            name: name.to_string(),
            channels: map,
        }
    }

    /// Creates a test Player with no hardware devices.
    fn test_player(song_names: &[&str]) -> Arc<Player> {
        let mut map = HashMap::new();
        for name in song_names {
            map.insert(
                name.to_string(),
                Arc::new(Song::new_for_test(name, &["track1"])),
            );
        }
        let songs = Arc::new(Songs::new(map));
        let playlist_config =
            config::Playlist::new(&song_names.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let pl = playlist::Playlist::new("test", &playlist_config, songs.clone()).unwrap();
        let devices = PlayerDevices {
            audio: None,
            mappings: None,
            midi: None,
            dmx_engine: None,
            sample_engine: None,
            trigger_engine: None,
        };
        let mut playlists = HashMap::new();
        playlists.insert(
            "all_songs".to_string(),
            playlist::from_songs(songs.clone()).unwrap(),
        );
        playlists.insert(pl.name().to_string(), pl);
        Arc::new(Player::new_with_devices(devices, playlists, "test".to_string(), None).unwrap())
    }

    fn test_app(song_names: &[&str]) -> App {
        let player = test_player(song_names);
        let (state_tx, state_rx) = watch::channel(Arc::new(StateSnapshot::default()));
        let _ = state_tx; // keep alive
        App::new(player, state_rx)
    }

    mod fixture_colors_from_snapshot_tests {
        use super::*;

        #[test]
        fn empty_fixtures() {
            let result = fixture_colors_from_snapshot(&[]);
            assert!(result.is_empty());
        }

        #[test]
        fn full_rgb() {
            let fixtures = vec![make_fixture(
                "spot1",
                &[("red", 255), ("green", 128), ("blue", 64)],
            )];
            let colors = fixture_colors_from_snapshot(&fixtures);
            assert_eq!(colors.len(), 1);
            assert_eq!(colors[0].name, "spot1");
            assert_eq!(colors[0].r, 255);
            assert_eq!(colors[0].g, 128);
            assert_eq!(colors[0].b, 64);
        }

        #[test]
        fn missing_channels_default_to_zero() {
            let fixtures = vec![make_fixture("dimmer", &[("intensity", 200)])];
            let colors = fixture_colors_from_snapshot(&fixtures);
            assert_eq!(colors[0].r, 0);
            assert_eq!(colors[0].g, 0);
            assert_eq!(colors[0].b, 0);
        }

        #[test]
        fn partial_rgb() {
            let fixtures = vec![make_fixture("par", &[("red", 100), ("blue", 50)])];
            let colors = fixture_colors_from_snapshot(&fixtures);
            assert_eq!(colors[0].r, 100);
            assert_eq!(colors[0].g, 0);
            assert_eq!(colors[0].b, 50);
        }

        #[test]
        fn multiple_fixtures() {
            let fixtures = vec![
                make_fixture("wash1", &[("red", 255), ("green", 0), ("blue", 0)]),
                make_fixture("wash2", &[("red", 0), ("green", 255), ("blue", 0)]),
                make_fixture("wash3", &[("red", 0), ("green", 0), ("blue", 255)]),
            ];
            let colors = fixture_colors_from_snapshot(&fixtures);
            assert_eq!(colors.len(), 3);
            assert_eq!(colors[0].r, 255);
            assert_eq!(colors[1].g, 255);
            assert_eq!(colors[2].b, 255);
        }

        #[test]
        fn extra_channels_ignored() {
            let fixtures = vec![make_fixture(
                "moving_head",
                &[
                    ("red", 10),
                    ("green", 20),
                    ("blue", 30),
                    ("pan", 180),
                    ("tilt", 90),
                ],
            )];
            let colors = fixture_colors_from_snapshot(&fixtures);
            assert_eq!(colors[0].r, 10);
            assert_eq!(colors[0].g, 20);
            assert_eq!(colors[0].b, 30);
        }

        #[test]
        fn preserves_fixture_names() {
            let fixtures = vec![
                make_fixture("Front Wash Left", &[]),
                make_fixture("Back Spot", &[]),
            ];
            let colors = fixture_colors_from_snapshot(&fixtures);
            assert_eq!(colors[0].name, "Front Wash Left");
            assert_eq!(colors[1].name, "Back Spot");
        }
    }

    mod app_new_tests {
        use super::*;

        #[test]
        fn initializes_playlist_name() {
            let app = test_app(&["Song A", "Song B"]);
            assert_eq!(app.playlist_name, "test");
        }

        #[test]
        fn initializes_song_names() {
            let app = test_app(&["Song A", "Song B"]);
            assert_eq!(app.song_names, vec!["Song A", "Song B"]);
        }

        #[test]
        fn initializes_current_song() {
            let app = test_app(&["Song A", "Song B"]);
            assert_eq!(app.current_song_name, "Song A");
        }

        #[test]
        fn initializes_current_index_zero() {
            let app = test_app(&["Song A", "Song B"]);
            assert_eq!(app.current_index, 0);
        }

        #[test]
        fn initializes_not_playing() {
            let app = test_app(&["Song A"]);
            assert!(!app.is_playing);
        }

        #[test]
        fn initializes_elapsed_none() {
            let app = test_app(&["Song A"]);
            assert!(app.elapsed.is_none());
        }

        #[test]
        fn initializes_empty_fixtures() {
            let app = test_app(&["Song A"]);
            assert!(app.fixture_colors.is_empty());
        }

        #[test]
        fn initializes_empty_effects() {
            let app = test_app(&["Song A"]);
            assert!(app.active_effects.is_empty());
        }

        #[test]
        fn initializes_tracks_from_song() {
            let app = test_app(&["Song A"]);
            assert_eq!(app.current_song_tracks, vec!["track1"]);
        }
    }

    mod handle_key_event_tests {
        use super::*;
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        fn key(code: KeyCode) -> KeyEvent {
            KeyEvent::new(code, KeyModifiers::NONE)
        }

        #[tokio::test]
        async fn quit_on_q() {
            let mut app = test_app(&["Song A"]);
            let action = app.handle_key_event(key(KeyCode::Char('q'))).await;
            assert!(matches!(action, Action::Quit));
        }

        #[tokio::test]
        async fn quit_on_esc() {
            let mut app = test_app(&["Song A"]);
            let action = app.handle_key_event(key(KeyCode::Esc)).await;
            assert!(matches!(action, Action::Quit));
        }

        #[tokio::test]
        async fn space_toggles_play() {
            let mut app = test_app(&["Song A"]);
            let action = app.handle_key_event(key(KeyCode::Char(' '))).await;
            assert!(matches!(action, Action::None));
        }

        #[tokio::test]
        async fn enter_toggles_play() {
            let mut app = test_app(&["Song A"]);
            let action = app.handle_key_event(key(KeyCode::Enter)).await;
            assert!(matches!(action, Action::None));
        }

        #[tokio::test]
        async fn right_arrow_next() {
            let mut app = test_app(&["Song A", "Song B"]);
            let action = app.handle_key_event(key(KeyCode::Right)).await;
            assert!(matches!(action, Action::None));
        }

        #[tokio::test]
        async fn n_key_next() {
            let mut app = test_app(&["Song A", "Song B"]);
            let action = app.handle_key_event(key(KeyCode::Char('n'))).await;
            assert!(matches!(action, Action::None));
        }

        #[tokio::test]
        async fn left_arrow_prev() {
            let mut app = test_app(&["Song A", "Song B"]);
            let action = app.handle_key_event(key(KeyCode::Left)).await;
            assert!(matches!(action, Action::None));
        }

        #[tokio::test]
        async fn p_key_prev() {
            let mut app = test_app(&["Song A", "Song B"]);
            let action = app.handle_key_event(key(KeyCode::Char('p'))).await;
            assert!(matches!(action, Action::None));
        }

        #[tokio::test]
        async fn a_key_all_songs() {
            let mut app = test_app(&["Song A"]);
            let action = app.handle_key_event(key(KeyCode::Char('a'))).await;
            assert!(matches!(action, Action::None));
        }

        #[tokio::test]
        async fn l_key_playlist() {
            let mut app = test_app(&["Song A"]);
            let action = app.handle_key_event(key(KeyCode::Char('l'))).await;
            assert!(matches!(action, Action::None));
        }

        #[tokio::test]
        async fn unhandled_key_returns_none() {
            let mut app = test_app(&["Song A"]);
            let action = app.handle_key_event(key(KeyCode::Char('z'))).await;
            assert!(matches!(action, Action::None));
        }
    }

    mod tick_tests {
        use super::*;

        #[tokio::test]
        async fn tick_updates_state() {
            let player = test_player(&["Song A", "Song B"]);
            let snapshot = Arc::new(StateSnapshot {
                fixtures: vec![make_fixture("spot", &[("red", 255)])],
                active_effects: vec!["chase".to_string()],
            });
            let (_tx, state_rx) = watch::channel(snapshot);
            let mut app = App::new(player, state_rx);

            app.tick().await;

            assert_eq!(app.fixture_colors.len(), 1);
            assert_eq!(app.fixture_colors[0].r, 255);
            assert_eq!(app.active_effects, vec!["chase"]);
        }

        #[tokio::test]
        async fn tick_reflects_current_song() {
            let mut app = test_app(&["Song A", "Song B"]);
            app.tick().await;
            assert_eq!(app.current_song_name, "Song A");
            assert_eq!(app.current_index, 0);
        }

        #[tokio::test]
        async fn tick_after_next_updates_song() {
            let mut app = test_app(&["Song A", "Song B"]);
            app.handle_key_event(KeyEvent::new(
                KeyCode::Right,
                crossterm::event::KeyModifiers::NONE,
            ))
            .await;
            app.tick().await;
            assert_eq!(app.current_song_name, "Song B");
            assert_eq!(app.current_index, 1);
        }
    }
}
