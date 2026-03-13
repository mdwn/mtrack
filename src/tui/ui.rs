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
use std::time::Duration;

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::Frame;

use super::app::App;

/// Renders the entire TUI layout.
pub fn draw(frame: &mut Frame, app: &App) {
    let size = frame.area();

    // Main vertical split: top area (content) | bottom area (log + keys)
    let main_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(10),   // Content area
            Constraint::Length(8), // Log panel
            Constraint::Length(1), // Key hints bar
        ])
        .split(size);

    // Top area: left (playlist) | right (now playing + fixtures + effects)
    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(30), Constraint::Percentage(70)])
        .split(main_chunks[0]);

    draw_playlist(frame, app, top_chunks[0]);
    draw_right_panel(frame, app, top_chunks[1]);
    draw_log(frame, app, main_chunks[1]);
    draw_key_hints(frame, main_chunks[2]);
}

/// Draws the playlist panel on the left.
fn draw_playlist(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .song_names
        .iter()
        .enumerate()
        .map(|(i, name)| {
            let prefix = if i == app.current_index { ">" } else { " " };
            ListItem::new(format!("{}{:2}. {}", prefix, i + 1, name))
        })
        .collect();

    let title = format!(" {} ", app.playlist_name);
    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(title))
        .highlight_style(
            Style::default()
                .add_modifier(Modifier::BOLD)
                .fg(Color::Yellow),
        );

    let mut state = ListState::default();
    state.select(Some(app.current_index));
    frame.render_stateful_widget(list, area, &mut state);
}

/// Draws the right panel: now playing, fixtures, and active effects.
fn draw_right_panel(frame: &mut Frame, app: &App, area: Rect) {
    // Decide how many sections to show based on available content
    let has_fixtures = !app.fixture_colors.is_empty();
    let has_effects = !app.active_effects.is_empty();

    let constraints = match (has_fixtures, has_effects) {
        (true, true) => vec![
            Constraint::Length(5), // Now Playing
            Constraint::Min(3),    // Fixtures
            Constraint::Length(5), // Active Effects
        ],
        (true, false) => vec![
            Constraint::Length(5), // Now Playing
            Constraint::Min(3),    // Fixtures
        ],
        (false, true) => vec![
            Constraint::Length(5), // Now Playing
            Constraint::Min(3),    // Active Effects
        ],
        (false, false) => vec![
            Constraint::Min(5), // Now Playing (takes all space)
        ],
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    draw_now_playing(frame, app, chunks[0]);

    match (has_fixtures, has_effects) {
        (true, true) => {
            draw_fixtures(frame, app, chunks[1]);
            draw_active_effects(frame, app, chunks[2]);
        }
        (true, false) => {
            draw_fixtures(frame, app, chunks[1]);
        }
        (false, true) => {
            draw_active_effects(frame, app, chunks[1]);
        }
        (false, false) => {}
    }
}

/// Draws the "Now Playing" panel.
fn draw_now_playing(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Now Playing ");

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height < 2 {
        return;
    }

    let play_indicator = if app.is_playing { "▶" } else { "■" };
    let title_line = Line::from(vec![
        Span::styled(
            format!(" {} ", play_indicator),
            Style::default().fg(if app.is_playing {
                Color::Green
            } else {
                Color::Red
            }),
        ),
        Span::raw(&app.current_song_name),
    ]);

    let title_paragraph = Paragraph::new(title_line);
    frame.render_widget(title_paragraph, Rect::new(inner.x, inner.y, inner.width, 1));

    // Progress bar
    if inner.height >= 2 {
        let elapsed = app.elapsed.unwrap_or(Duration::ZERO);
        let total = app.current_song_duration;

        let ratio = if total.as_secs_f64() > 0.0 {
            (elapsed.as_secs_f64() / total.as_secs_f64()).min(1.0)
        } else {
            0.0
        };

        let elapsed_str = format_duration(elapsed);
        let total_str = format_duration(total);
        let label = format!("{} / {}", elapsed_str, total_str);

        let gauge = Gauge::default()
            .gauge_style(Style::default().fg(Color::Cyan))
            .ratio(ratio)
            .label(label);

        frame.render_widget(
            gauge,
            Rect::new(inner.x + 1, inner.y + 1, inner.width.saturating_sub(2), 1),
        );
    }

    // Track names
    if inner.height >= 3 {
        let tracks = format!(" Tracks: {}", app.current_song_tracks.join(", "));
        let tracks_paragraph = Paragraph::new(tracks).style(Style::default().fg(Color::DarkGray));
        frame.render_widget(
            tracks_paragraph,
            Rect::new(inner.x, inner.y + 2, inner.width, 1),
        );
    }
}

/// Draws the fixtures panel with colored blocks.
fn draw_fixtures(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" Fixtures ");

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Calculate how many fixtures per row (each fixture takes ~14 chars)
    let fixture_width = 14_u16;
    let cols = (inner.width / fixture_width).max(1);

    let mut lines: Vec<Line> = Vec::new();
    let mut current_line_spans: Vec<Span> = Vec::new();
    let mut col = 0;

    for fixture in &app.fixture_colors {
        let color_block = Span::styled(
            "\u{2588}\u{2588}",
            Style::default().fg(Color::Rgb(fixture.r, fixture.g, fixture.b)),
        );

        // Truncate name to fit
        let max_name_len = (fixture_width as usize).saturating_sub(4);
        let display_name: String = if fixture.name.len() > max_name_len {
            fixture.name[..max_name_len].to_string()
        } else {
            fixture.name.clone()
        };
        let name_span = Span::styled(
            format!(" {:<width$}", display_name, width = max_name_len),
            Style::default().fg(Color::White),
        );

        current_line_spans.push(color_block);
        current_line_spans.push(name_span);
        current_line_spans.push(Span::raw(" "));

        col += 1;
        if col >= cols {
            lines.push(Line::from(std::mem::take(&mut current_line_spans)));
            col = 0;
        }
    }

    if !current_line_spans.is_empty() {
        lines.push(Line::from(current_line_spans));
    }

    let paragraph = Paragraph::new(lines);
    frame.render_widget(paragraph, inner);
}

/// Draws the active effects panel.
fn draw_active_effects(frame: &mut Frame, app: &App, area: Rect) {
    let items: Vec<ListItem> = app
        .active_effects
        .iter()
        .map(|name| ListItem::new(format!("  {}", name)))
        .collect();

    let list = List::new(items).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Active Effects "),
    );

    frame.render_widget(list, area);
}

/// Draws the log panel.
fn draw_log(frame: &mut Frame, app: &App, area: Rect) {
    let block = Block::default().borders(Borders::ALL).title(" Log ");

    let inner = block.inner(area);
    let visible_lines = inner.height as usize;

    // Show the most recent log lines
    let start = app.log_lines.len().saturating_sub(visible_lines);
    let visible: Vec<Line> = app.log_lines[start..]
        .iter()
        .map(|line| {
            let color = log_line_color(line);
            Line::from(Span::styled(line.as_str(), Style::default().fg(color)))
        })
        .collect();

    let paragraph = Paragraph::new(visible).wrap(Wrap { trim: false });
    frame.render_widget(block, area);
    frame.render_widget(paragraph, inner);
}

/// Draws the key hints bar at the bottom.
fn draw_key_hints(frame: &mut Frame, area: Rect) {
    let hints = Line::from(vec![
        Span::styled(" Space", Style::default().fg(Color::Yellow)),
        Span::raw("=play/stop  "),
        Span::styled("\u{2190}/\u{2192}", Style::default().fg(Color::Yellow)),
        Span::raw("=prev/next  "),
        Span::styled("a", Style::default().fg(Color::Yellow)),
        Span::raw("=all songs  "),
        Span::styled("l", Style::default().fg(Color::Yellow)),
        Span::raw("=playlist  "),
        Span::styled("q", Style::default().fg(Color::Yellow)),
        Span::raw("=quit"),
    ]);

    let paragraph =
        Paragraph::new(hints).style(Style::default().bg(Color::DarkGray).fg(Color::White));
    frame.render_widget(paragraph, area);
}

/// Returns the color for a log line based on its level prefix.
fn log_line_color(line: &str) -> Color {
    if line.starts_with("ERROR") {
        Color::Red
    } else if line.starts_with("WARN") {
        Color::Yellow
    } else if line.starts_with("DEBUG") {
        Color::DarkGray
    } else {
        Color::Gray
    }
}

/// Formats a Duration as "M:SS".
fn format_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    let minutes = total_secs / 60;
    let seconds = total_secs % 60;
    format!("{}:{:02}", minutes, seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    mod log_line_color_tests {
        use super::*;

        #[test]
        fn error_line_is_red() {
            assert_eq!(log_line_color("ERROR something went wrong"), Color::Red);
        }

        #[test]
        fn warn_line_is_yellow() {
            assert_eq!(
                log_line_color("WARN deprecated feature used"),
                Color::Yellow
            );
        }

        #[test]
        fn debug_line_is_dark_gray() {
            assert_eq!(log_line_color("DEBUG entering function"), Color::DarkGray);
        }

        #[test]
        fn info_line_is_gray() {
            assert_eq!(log_line_color("INFO server started"), Color::Gray);
        }

        #[test]
        fn trace_line_is_gray() {
            assert_eq!(log_line_color("TRACE detailed output"), Color::Gray);
        }

        #[test]
        fn empty_line_is_gray() {
            assert_eq!(log_line_color(""), Color::Gray);
        }

        #[test]
        fn case_sensitive_error() {
            // "error" (lowercase) should not match ERROR
            assert_eq!(log_line_color("error lowercase"), Color::Gray);
        }

        #[test]
        fn error_prefix_only() {
            assert_eq!(log_line_color("ERROR"), Color::Red);
        }
    }

    mod draw_tests {
        use super::*;
        use crate::tui::app::FixtureColor;
        use ratatui::backend::TestBackend;
        use ratatui::Terminal;

        fn make_app() -> App {
            use crate::config;
            use crate::player::PlayerDevices;
            use crate::playlist;
            use crate::songs::{Song, Songs};
            use std::collections::HashMap;
            use tokio::sync::watch;

            let mut map = HashMap::new();
            for name in &["Song A", "Song B"] {
                map.insert(
                    name.to_string(),
                    std::sync::Arc::new(Song::new_for_test(name, &["kick", "snare"])),
                );
            }
            let songs = std::sync::Arc::new(Songs::new(map));
            let playlist_config =
                config::Playlist::new(&["Song A".to_string(), "Song B".to_string()]);
            let playlist =
                playlist::Playlist::new("My Set", &playlist_config, songs.clone()).unwrap();
            let devices = PlayerDevices {
                audio: None,
                mappings: None,
                midi: None,
                dmx_engine: None,
                sample_engine: None,
                trigger_engine: None,
            };
            let player = std::sync::Arc::new(
                crate::player::Player::new_with_devices(devices, playlist, songs, None).unwrap(),
            );
            let (_tx, state_rx) =
                watch::channel(std::sync::Arc::new(crate::state::StateSnapshot::default()));
            App::new(player, state_rx)
        }

        #[test]
        fn draw_does_not_panic() {
            let app = make_app();
            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
        }

        #[test]
        fn draw_with_fixtures_and_effects() {
            let mut app = make_app();
            app.fixture_colors = vec![
                FixtureColor {
                    name: "wash1".to_string(),
                    r: 255,
                    g: 0,
                    b: 0,
                },
                FixtureColor {
                    name: "wash2".to_string(),
                    r: 0,
                    g: 255,
                    b: 0,
                },
            ];
            app.active_effects = vec!["chase".to_string(), "fade".to_string()];

            let backend = TestBackend::new(100, 30);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
        }

        #[test]
        fn draw_with_fixtures_only() {
            let mut app = make_app();
            app.fixture_colors = vec![FixtureColor {
                name: "spot".to_string(),
                r: 128,
                g: 128,
                b: 128,
            }];

            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
        }

        #[test]
        fn draw_with_effects_only() {
            let mut app = make_app();
            app.active_effects = vec!["strobe".to_string()];

            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
        }

        #[test]
        fn draw_playing_with_elapsed() {
            let mut app = make_app();
            app.is_playing = true;
            app.elapsed = Some(Duration::from_secs(30));
            app.current_song_duration = Duration::from_secs(180);

            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
        }

        #[test]
        fn draw_with_log_lines() {
            let mut app = make_app();
            app.log_lines = vec![
                "INFO mtrack: Started".to_string(),
                "WARN mtrack: Low buffer".to_string(),
                "ERROR mtrack: Device lost".to_string(),
                "DEBUG mtrack: Tick".to_string(),
            ];

            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
        }

        #[test]
        fn draw_small_terminal() {
            let app = make_app();
            let backend = TestBackend::new(40, 12);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
        }

        #[test]
        fn draw_many_fixtures_wraps_rows() {
            let mut app = make_app();
            app.fixture_colors = (0..20)
                .map(|i| FixtureColor {
                    name: format!("fixture_{}", i),
                    r: (i * 10) as u8,
                    g: 0,
                    b: 0,
                })
                .collect();

            let backend = TestBackend::new(80, 30);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
        }

        #[test]
        fn draw_zero_duration_song() {
            let mut app = make_app();
            app.current_song_duration = Duration::ZERO;
            app.elapsed = Some(Duration::ZERO);

            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
        }

        #[test]
        fn draw_long_fixture_name_truncated() {
            let mut app = make_app();
            app.fixture_colors = vec![FixtureColor {
                name: "A Very Long Fixture Name That Should Be Truncated".to_string(),
                r: 100,
                g: 200,
                b: 50,
            }];

            let backend = TestBackend::new(80, 24);
            let mut terminal = Terminal::new(backend).unwrap();
            terminal.draw(|frame| draw(frame, &app)).unwrap();
        }
    }

    mod format_duration_tests {
        use super::*;

        #[test]
        fn zero_duration() {
            assert_eq!(format_duration(Duration::ZERO), "0:00");
        }

        #[test]
        fn seconds_only() {
            assert_eq!(format_duration(Duration::from_secs(45)), "0:45");
        }

        #[test]
        fn one_minute() {
            assert_eq!(format_duration(Duration::from_secs(60)), "1:00");
        }

        #[test]
        fn minutes_and_seconds() {
            assert_eq!(format_duration(Duration::from_secs(185)), "3:05");
        }

        #[test]
        fn pads_single_digit_seconds() {
            assert_eq!(format_duration(Duration::from_secs(61)), "1:01");
        }

        #[test]
        fn large_duration() {
            assert_eq!(format_duration(Duration::from_secs(3661)), "61:01");
        }

        #[test]
        fn subsecond_truncated() {
            assert_eq!(format_duration(Duration::from_millis(59_999)), "0:59");
        }
    }
}
