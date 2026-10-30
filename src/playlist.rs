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
use tracing::{info, span, Level, Span};

use crate::config;
use crate::songs::{Song, Songs};
use core::fmt;
use parking_lot::RwLock;
use std::sync::Arc;
use std::time::Duration;

/// Typed error for playlist creation so callers can distinguish e.g. missing song in registry.
#[derive(Debug, thiserror::Error)]
pub enum PlaylistError {
    #[error("Song not in registry: {0}")]
    SongNotFound(String),
    #[error("Invalid transition: {0}")]
    InvalidTransition(String),
    #[error("Invalid ambient: {0}")]
    InvalidAmbient(String),
}

/// What a loop break hands over to: the song it moves to, and the fades the
/// playlist sets for the handover.
pub struct LoopBreak {
    /// The song the break moves to.
    pub target: Option<Arc<Song>>,
    /// How the looping song fades out under it.
    pub fade_out: Option<config::Fade>,
    /// How it fades in under the looping song.
    pub fade_in: Option<config::Fade>,
}

/// Where a playlist stands: on an entry, or, after an edit took away the
/// song playing, off the list just before the entry at `index` (or after
/// the last, at the list's length) while that song plays on.
#[derive(Clone, Default)]
struct Cursor {
    /// The entry the playlist is on, or the one it sits before.
    index: usize,
    /// The song playing that an edit took off the list, if one did.
    off_list: Option<Arc<Song>>,
}

/// Playlist is a playlist for use by a player.
pub struct Playlist {
    /// The name of this playlist.
    name: String,
    /// The songs that this playlist will play.
    songs: Vec<String>,
    /// How each entry hands over to the next.
    transitions: Vec<config::EntryTransitions>,
    /// Each entry's fades as written, for an editor.
    fades: Vec<config::EntryFades>,
    /// Its ambient light, as written: overriding the player's.
    ambient: Option<config::ambient::PlaylistAmbient>,
    /// The current position of the playlist.
    position: Arc<RwLock<Cursor>>,
    /// The song registry.
    registry: Arc<Songs>,
    /// The logging span.
    span: Span,
}

impl fmt::Display for Playlist {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Playlist ({} songs):", self.songs.len())?;
        for song_name in self.songs.iter() {
            match self.registry.get(song_name) {
                Ok(song) => writeln!(f, "  - {} (Channels: {})", song.name(), song.num_channels())?,
                Err(_) => writeln!(f, "  - {} (unable to find song)", song_name)?,
            };
        }

        Ok(())
    }
}

impl Playlist {
    /// Creates a new playlist.
    pub fn new(
        name: &str,
        config: &config::Playlist,
        registry: Arc<Songs>,
    ) -> Result<Arc<Playlist>, PlaylistError> {
        // Verify that each song in the playlist exists in the registry.
        let song_names = config.songs();
        for song_name in song_names.iter() {
            registry
                .get(song_name)
                .map_err(|_| PlaylistError::SongNotFound(song_name.clone()))?;
        }

        let transitions = validate_transitions(config, &registry)?;

        Ok(Arc::new(Playlist {
            name: name.to_string(),
            songs: song_names,
            transitions,
            fades: config.entry_fades(),
            ambient: config.ambient().cloned(),
            position: Arc::new(RwLock::new(Cursor::default())),
            registry: Arc::clone(&registry),
            span: span!(Level::INFO, "playlist"),
        }))
    }

    /// Returns the name of this playlist.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its ambient light, as written: overriding the player's.
    pub fn ambient(&self) -> Option<&config::ambient::PlaylistAmbient> {
        self.ambient.as_ref()
    }

    /// Returns the current position in the playlist (0-indexed). Off the
    /// list, the entry the playlist sits before.
    pub fn position(&self) -> usize {
        self.position.read().index
    }

    /// The entry the playlist is on, or `None` while the song playing is one
    /// an edit took off the list.
    pub fn entry(&self) -> Option<usize> {
        let cursor = self.position.read();
        cursor.off_list.is_none().then_some(cursor.index)
    }

    /// The entry [`Playlist::next`] moves to, or `None` at the end of the
    /// list.
    pub fn next_entry(&self) -> Option<usize> {
        let cursor = self.position.read();
        let index = match cursor.off_list {
            Some(_) => cursor.index,
            None => cursor.index + 1,
        };
        (index < self.songs.len()).then_some(index)
    }

    /// Takes up the place `old`, the playlist this one replaces, had got
    /// to: on the same song if it is still in the list, its entries matched
    /// up first to first, as an edit keeps each entry's settings with its
    /// song. A song `playing` that the edit took away stays current, off the
    /// list, until the playlist moves on; one that is not playing gives way
    /// to the entry now in its place.
    pub fn follow(&self, old: &Playlist, playing: bool) {
        let old_cursor = old.position.read().clone();
        let Some(song) = old.song_at(&old_cursor) else {
            return;
        };
        let name = song.name();
        // Which of the song's entries the old list was on: the first, the
        // second...
        let nth = match old_cursor.off_list {
            Some(_) => 0,
            None => old.songs[..old_cursor.index]
                .iter()
                .filter(|s| *s == name)
                .count(),
        };
        let entries: Vec<usize> = (0..self.songs.len())
            .filter(|&i| self.songs[i] == name)
            .collect();
        let cursor = match entries.get(nth).or(entries.last()) {
            Some(&index) => Cursor {
                index,
                off_list: None,
            },
            None if playing => Cursor {
                index: old_cursor.index.min(self.songs.len()),
                off_list: Some(song),
            },
            None => Cursor {
                index: old_cursor.index.min(self.songs.len().saturating_sub(1)),
                off_list: None,
            },
        };
        *self.position.write() = cursor;
    }

    /// The song at `cursor`: the one playing off the list, or its entry's.
    fn song_at(&self, cursor: &Cursor) -> Option<Arc<Song>> {
        if let Some(song) = &cursor.off_list {
            return Some(Arc::clone(song));
        }
        let name = self.songs.get(cursor.index)?;
        self.registry.get(name).ok()
    }

    /// Returns the list of songs in the playlist.
    pub fn songs(&self) -> &Vec<String> {
        &self.songs
    }

    /// Move to the next element of the playlist. If we're at the end of the playlist, the position will not
    /// increment. The song at the current position will be returned.
    /// Returns `None` if the playlist is empty.
    pub fn next(&self) -> Option<Arc<Song>> {
        let _enter = self.span.enter();

        if self.songs.is_empty() {
            return None;
        }

        let mut position = self.position.write();
        // Off the list, the next entry is the one it sits before; after the
        // last there is none, and it stays.
        if position.off_list.is_some() {
            if position.index < self.songs.len() {
                position.off_list = None;
            }
        } else if position.index < self.songs.len() - 1 {
            position.index += 1;
        }

        let current = self.song_at(&position)?;

        info!(
            position = position.index,
            song = current.name(),
            "Moving to next playlist position."
        );

        Some(current)
    }

    /// Move to the previous element of the playlist. If we're at the beginning of the playlist, the position
    /// will not decrement. The song at the current position will be returned.
    /// Returns `None` if the playlist is empty.
    pub fn prev(&self) -> Option<Arc<Song>> {
        if self.songs.is_empty() {
            return None;
        }

        let mut position = self.position.write();
        // Off the list, the previous entry is the one before the entry it
        // sits before.
        if position.off_list.take().is_some() {
            position.index = position.index.saturating_sub(1);
        } else if position.index > 0 {
            position.index -= 1;
        }

        let current = self.song_at(&position)?;

        info!(
            position = position.index,
            song = current.name(),
            "Moving to previous playlist position."
        );

        Some(current)
    }

    /// The song [`Playlist::next`] would move to, without moving.
    pub fn peek_next(&self) -> Option<Arc<Song>> {
        if self.songs.is_empty() {
            return None;
        }
        self.song_at(&self.target(false))
    }

    /// The song [`Playlist::prev`] would move to, without moving.
    pub fn peek_prev(&self) -> Option<Arc<Song>> {
        if self.songs.is_empty() {
            return None;
        }
        self.song_at(&self.target(true))
    }

    /// Where previous (`back`) or next would move to, clamped at either end
    /// of the list as they are.
    fn target(&self, back: bool) -> Cursor {
        let cursor = self.position.read().clone();
        let index = match (back, &cursor.off_list) {
            (true, _) => cursor.index.saturating_sub(1),
            // After the last entry there is nothing to go on to.
            (false, Some(_)) if cursor.index >= self.songs.len() => return cursor,
            (false, Some(_)) => cursor.index,
            (false, None) => (cursor.index + 1).min(self.songs.len().saturating_sub(1)),
        };
        Cursor {
            index,
            off_list: None,
        }
    }

    /// What breaking the current song's loop hands over to, moving back
    /// (previous) or on (play, next): the song it lands on and the fades for
    /// the handover -- the current entry's `fade_out` and the target's
    /// `fade_in`.
    pub fn loop_break(&self, back: bool) -> LoopBreak {
        if self.songs.is_empty() {
            return LoopBreak {
                target: None,
                fade_out: None,
                fade_in: None,
            };
        }
        // A song off the list has no entry, and so no fade out of its own.
        let from = self.entry();
        let to = self.target(back);
        let onto = to.off_list.is_none().then_some(to.index);
        LoopBreak {
            target: self.song_at(&to),
            fade_out: from.and_then(|from| self.transitions[from].fade_out.clone()),
            fade_in: onto
                .and_then(|to| self.transitions.get(to))
                .and_then(|t| t.fade_in.clone()),
        }
    }

    /// Whether there is an entry before the current one (`back`) or after
    /// it: false at either end of the list, where next and previous clamp.
    pub fn can_move(&self, back: bool) -> bool {
        let cursor = self.position.read();
        match (back, &cursor.off_list) {
            (true, _) => cursor.index > 0,
            (false, Some(_)) => cursor.index < self.songs.len(),
            (false, None) => cursor.index + 1 < self.songs.len(),
        }
    }

    /// The gap before the song at the current position starts by itself,
    /// reached by the song before it ending or by next/previous while
    /// stopped; `None` when it waits for play.
    pub fn auto_play(&self) -> Option<Duration> {
        let position = self.entry()?;
        self.transitions.get(position).and_then(|t| t.auto_play)
    }

    /// Each entry's `auto_play` gap, in order: `None` where it waits for
    /// play.
    pub fn entry_auto_play(&self) -> Vec<Option<Duration>> {
        self.transitions.iter().map(|t| t.auto_play).collect()
    }

    /// Each entry's fades as written, in order.
    pub fn entry_fades(&self) -> &[config::EntryFades] {
        &self.fades
    }

    /// Sets the playlist position to the song matching the given name.
    /// Returns the song if found, None otherwise.
    pub fn navigate_to(&self, name: &str) -> Option<Arc<Song>> {
        let idx = self.songs.iter().position(|s| s == name)?;
        let song = self.registry.get(name).ok()?;
        *self.position.write() = Cursor {
            index: idx,
            off_list: None,
        };
        Some(song)
    }

    /// Returns the underlying song registry.
    pub fn registry(&self) -> &Arc<Songs> {
        &self.registry
    }

    /// Look up a song by name from the underlying registry.
    pub fn get_song(&self, name: &str) -> Option<Arc<Song>> {
        self.registry.get(name).ok()
    }

    /// Return the song at the current position of the playlist: off the
    /// list, the song playing there. Returns `None` if the playlist is empty.
    pub fn current(&self) -> Option<Arc<Song>> {
        self.song_at(&self.position.read())
    }
}

/// Parses a playlist's transitions and checks each entry's `exclude` names
/// tracks its song has. A song not in `registry` is not checked.
pub fn validate_transitions(
    config: &config::Playlist,
    registry: &Songs,
) -> Result<Vec<config::EntryTransitions>, PlaylistError> {
    if let Some(ambient) = config.ambient() {
        ambient.validate().map_err(PlaylistError::InvalidAmbient)?;
    }
    let transitions = config
        .entry_transitions()
        .map_err(PlaylistError::InvalidTransition)?;
    for (song_name, entry) in config.songs().iter().zip(&transitions) {
        let Ok(song) = registry.get(song_name) else {
            continue;
        };
        let fades = [
            ("fade_out", entry.fade_out.as_ref()),
            ("fade_in", entry.fade_in.as_ref()),
        ];
        for (field, fade) in fades {
            let tracks = fade.and_then(|f| f.exclude.as_deref()).unwrap_or_default();
            if let Some(track) = tracks.iter().find(|t| !song.has_track(t)) {
                return Err(PlaylistError::InvalidTransition(format!(
                    "song {song_name}: {field}.exclude names track {track:?}, \
                     which the song does not have"
                )));
            }
        }
    }
    Ok(transitions)
}

/// Checks a playlist about to be written as [`validate_transitions`] does,
/// and that no fade outlasts the song it plays under, counted on the grid
/// it will be: a fade-out under the next entry's song (its own at the end
/// of the list), a fade-in under its own. The player cuts such a fade short
/// rather than drop the playlist on load, so it is refused here, where the
/// one writing it can fix it (#577).
///
/// Each song must be in `registry` too: the player drops a playlist naming
/// one that is not, and a save would make it vanish (#578). And an ambient
/// show must be a path inside the project, which the player will read it
/// from and no further.
pub fn validate_for_write(
    config: &config::Playlist,
    registry: &Songs,
) -> Result<(), PlaylistError> {
    if let Some(missing) = config
        .songs()
        .iter()
        .find(|name| registry.get(name).is_err())
    {
        return Err(PlaylistError::SongNotFound(missing.clone()));
    }
    let show = match config.ambient() {
        Some(config::ambient::PlaylistAmbient::On(ambient)) => ambient.show(),
        _ => None,
    };
    if let Some(show) = show {
        let path = std::path::Path::new(show);
        let escapes = path.is_absolute()
            || path
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir));
        if escapes {
            return Err(PlaylistError::InvalidAmbient(format!(
                "ambient show {show:?}: must be a path inside the project, relative to it \
                 and without \"..\""
            )));
        }
    }
    let transitions = validate_transitions(config, registry)?;
    let songs: Vec<Option<Arc<Song>>> = config
        .songs()
        .iter()
        .map(|name| registry.get(name).ok())
        .collect();
    for (i, entry) in transitions.iter().enumerate() {
        let Some(song) = &songs[i] else {
            continue;
        };
        let next = songs.get(i + 1).cloned().flatten();
        let fades = [
            ("fade_out", entry.fade_out.as_ref(), false),
            ("fade_in", entry.fade_in.as_ref(), true),
        ];
        for (field, fade, fade_in) in fades {
            let Some(fade) = fade else {
                continue;
            };
            song.check_fade(fade, next.as_deref(), fade_in)
                .map_err(|e| {
                    PlaylistError::InvalidTransition(format!("song {}: {field}: {e}", song.name()))
                })?;
        }
    }
    Ok(())
}

/// Creates an alphabetized playlist from all available songs.
pub fn from_songs(songs: Arc<Songs>) -> Result<Arc<Playlist>, PlaylistError> {
    // The easiest thing to do here is to gather the names of all of the songs and pass them
    // to new. This is a little silly, since new is just going to double check that they
    // all exist and then do an explicit mapping each time. However, the easiest way to
    // make from_file work is to do it this way, so we'll just do this rigamarole for now.
    let sorted = Vec::from_iter(
        songs
            .sorted_list()
            .into_iter()
            .map(|song| song.name().to_string()),
    );
    Playlist::new("all_songs", &config::Playlist::new(&sorted), songs)
}

#[cfg(test)]
mod test {
    use std::path::Path;
    use std::sync::Arc;

    use crate::{config, songs, songs::Songs};

    fn test_registry() -> Arc<Songs> {
        songs::get_all_songs(Path::new("assets/songs")).expect("Parse songs should have succeeded.")
    }

    fn two_song_playlist(registry: Arc<Songs>) -> Arc<super::Playlist> {
        super::Playlist::new(
            "Test Playlist",
            &config::Playlist::new(&["Song 1".to_string(), "Song 2".to_string()]),
            registry,
        )
        .expect("Unable to create playlist")
    }

    #[test]
    fn test_playlist() {
        let playlist = two_song_playlist(test_registry());

        // Starts at the first element in the list.
        assert_eq!("Song 1", playlist.current().unwrap().name());

        // Previous should just stay at the beginning of the list, since it's at the start.
        playlist.prev();
        assert_eq!("Song 1", playlist.current().unwrap().name());

        // Next goes to the next entry.
        playlist.next();
        assert_eq!("Song 2", playlist.current().unwrap().name());

        // Next should just stay at the end of the list, since it's at the end.
        playlist.next();
        assert_eq!("Song 2", playlist.current().unwrap().name());

        // Prev goes to the previous entry.
        playlist.prev();
        assert_eq!("Song 1", playlist.current().unwrap().name());
    }

    #[test]
    fn position_tracking() {
        let playlist = two_song_playlist(test_registry());
        assert_eq!(playlist.position(), 0);

        playlist.next();
        assert_eq!(playlist.position(), 1);

        playlist.prev();
        assert_eq!(playlist.position(), 0);
    }

    #[test]
    fn empty_playlist() {
        let registry = test_registry();
        let playlist = super::Playlist {
            name: "empty".to_string(),
            songs: vec![],
            transitions: vec![],
            fades: vec![],
            ambient: None,
            position: Default::default(),
            registry,
            span: tracing::span!(tracing::Level::INFO, "test"),
        };
        assert!(playlist.current().is_none());
        assert!(playlist.next().is_none());
        assert!(playlist.prev().is_none());
    }

    #[test]
    fn name() {
        let playlist = two_song_playlist(test_registry());
        assert_eq!(playlist.name(), "Test Playlist");
    }

    #[test]
    fn songs_list() {
        let playlist = two_song_playlist(test_registry());
        assert_eq!(playlist.songs(), &["Song 1", "Song 2"]);
    }

    #[test]
    fn get_song_found() {
        let playlist = two_song_playlist(test_registry());
        let song = playlist.get_song("Song 1");
        assert!(song.is_some());
        assert_eq!(song.unwrap().name(), "Song 1");
    }

    #[test]
    fn get_song_not_found() {
        let playlist = two_song_playlist(test_registry());
        assert!(playlist.get_song("Nonexistent Song").is_none());
    }

    #[test]
    fn song_not_in_registry_error() {
        let registry = test_registry();
        let result = super::Playlist::new(
            "Bad Playlist",
            &config::Playlist::new(&["Song 1".to_string(), "No Such Song".to_string()]),
            registry,
        );
        let err = result.err().expect("should be an error");
        assert!(
            err.to_string().contains("No Such Song"),
            "error should mention missing song name: {}",
            err
        );
    }

    #[test]
    fn from_songs_all_songs_playlist() {
        let registry = test_registry();
        let all = super::from_songs(Arc::clone(&registry)).expect("from_songs");
        assert_eq!(all.name(), "all_songs");
        // Should contain all songs in the registry, sorted alphabetically.
        let names: Vec<&str> = all.songs().iter().map(|s| s.as_str()).collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        assert!(!names.is_empty());
    }

    #[test]
    fn next_returns_correct_song() {
        let playlist = two_song_playlist(test_registry());
        let song = playlist.next().unwrap();
        assert_eq!(song.name(), "Song 2");
    }

    #[test]
    fn prev_returns_correct_song() {
        let playlist = two_song_playlist(test_registry());
        playlist.next(); // move to Song 2
        let song = playlist.prev().unwrap();
        assert_eq!(song.name(), "Song 1");
    }

    #[test]
    fn display_impl() {
        let playlist = two_song_playlist(test_registry());
        let display = format!("{}", playlist);
        assert!(display.contains("Playlist (2 songs):"));
        assert!(display.contains("Song 1"));
        assert!(display.contains("Song 2"));
    }

    #[test]
    fn navigate_to_found() {
        let playlist = two_song_playlist(test_registry());
        assert_eq!(playlist.position(), 0);
        let song = playlist.navigate_to("Song 2");
        assert!(song.is_some());
        assert_eq!(song.unwrap().name(), "Song 2");
        assert_eq!(playlist.position(), 1);
        assert_eq!(playlist.current().unwrap().name(), "Song 2");
    }

    #[test]
    fn navigate_to_not_found() {
        let playlist = two_song_playlist(test_registry());
        let song = playlist.navigate_to("Nonexistent");
        assert!(song.is_none());
        // Position should not change
        assert_eq!(playlist.position(), 0);
    }

    #[test]
    fn navigate_to_first_song() {
        let playlist = two_song_playlist(test_registry());
        playlist.next(); // move to Song 2
        assert_eq!(playlist.position(), 1);
        let song = playlist.navigate_to("Song 1");
        assert!(song.is_some());
        assert_eq!(playlist.position(), 0);
    }

    #[test]
    fn display_with_missing_song_shows_error() {
        // Construct a Playlist directly (bypassing new()'s registry check)
        // to exercise the Display path for a song not in the registry.
        let registry = test_registry();
        let playlist = super::Playlist {
            name: "broken".to_string(),
            songs: vec!["Song 1".to_string(), "Ghost Song".to_string()],
            transitions: vec![Default::default(); 2],
            fades: vec![Default::default(); 2],
            ambient: None,
            position: Default::default(),
            registry,
            span: tracing::span!(tracing::Level::INFO, "test"),
        };
        let display = format!("{}", playlist);
        assert!(
            display.contains("unable to find song"),
            "display should show error for missing song: {}",
            display
        );
        assert!(display.contains("Ghost Song"));
    }

    /// A playlist of songs with one track, `t`, written as `yaml`.
    fn playlist_from(yaml: &str) -> Result<Arc<super::Playlist>, super::PlaylistError> {
        let mut map = std::collections::HashMap::new();
        for name in ["A", "B", "C"] {
            map.insert(
                name.to_string(),
                Arc::new(songs::Song::new_for_test(name, &["t"])),
            );
        }
        let config: config::Playlist = ::config::Config::builder()
            .add_source(::config::File::from_str(yaml, ::config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap();
        super::Playlist::new("p", &config, Arc::new(Songs::new(map)))
    }

    #[test]
    fn a_loop_break_hands_over_with_the_entries_fades() {
        let playlist = playlist_from(
            "songs:\n- song: A\n  fade_in: 1s\n- song: B\n  fade_out: 2s\n- song: C\n  fade_in: 3s\n",
        )
        .unwrap();
        let length = |f: Option<config::Fade>| f.map(|f| f.length.to_string());
        playlist.navigate_to("B");
        let on = playlist.loop_break(false);
        assert_eq!(on.target.unwrap().name(), "C");
        assert_eq!(length(on.fade_out), Some("2s".to_string()));
        assert_eq!(length(on.fade_in), Some("3s".to_string()));
        let back = playlist.loop_break(true);
        assert_eq!(back.target.unwrap().name(), "A");
        assert_eq!(length(back.fade_in), Some("1s".to_string()));
        // Breaking doesn't move: the cleanup does.
        assert_eq!(playlist.current().unwrap().name(), "B");
    }

    #[test]
    fn can_move_is_false_at_either_end() {
        let playlist = playlist_from("songs:\n- A\n- B\n").unwrap();
        assert!(!playlist.can_move(true));
        assert!(playlist.can_move(false));
        playlist.navigate_to("B");
        assert!(playlist.can_move(true));
        assert!(!playlist.can_move(false));
    }

    #[test]
    fn an_edit_keeps_the_place_by_song() {
        // #576: the new list started at its first entry, whatever was on it.
        let old = playlist_from("songs:\n- A\n- B\n- C\n").unwrap();
        old.navigate_to("B");
        let new = playlist_from("songs:\n- B\n- A\n- C\n").unwrap();
        new.follow(&old, true);
        assert_eq!(new.current().unwrap().name(), "B");
        assert_eq!(new.entry(), Some(0));

        // On the second of a song's entries, the second it keeps.
        let old = playlist_from("songs:\n- A\n- B\n- A\n").unwrap();
        old.next();
        old.next();
        let new = playlist_from("songs:\n- C\n- A\n- A\n").unwrap();
        new.follow(&old, true);
        assert_eq!(new.entry(), Some(2));
    }

    /// A registry of 4-second songs on disk: A loops with no grid, B has
    /// one at 120 bpm in 4/4.
    fn songs_on_disk() -> (Arc<Songs>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        for (name, extra) in [
            ("A", "loop_playback: true\n"),
            ("B", "tempo:\n  bpm: 120\n  time_signature: 4/4\n"),
        ] {
            let song_dir = dir.path().join(name);
            std::fs::create_dir_all(&song_dir).unwrap();
            crate::testutil::write_wav(song_dir.join("t.wav"), vec![vec![0_i32; 8000 * 4]], 8000)
                .unwrap();
            std::fs::write(
                song_dir.join("song.yaml"),
                format!("name: {name}\ntracks:\n  - name: t\n    file: t.wav\n{extra}"),
            )
            .unwrap();
        }
        (songs::get_all_songs(dir.path()).unwrap(), dir)
    }

    /// The playlist written as `yaml`.
    fn config_from(yaml: &str) -> config::Playlist {
        ::config::Config::builder()
            .add_source(::config::File::from_str(yaml, ::config::FileFormat::Yaml))
            .build()
            .unwrap()
            .try_deserialize()
            .unwrap()
    }

    #[test]
    fn a_write_refuses_a_fade_longer_than_the_song_under_it() {
        // #577: seconds had a bound and measures none, so a million
        // measures saved and made a 33-day ending.
        let (registry, _dir) = songs_on_disk();
        let long = config_from("songs:\n- song: A\n  fade_out: 1000000measures\n- B\n");
        let err = super::validate_for_write(&long, &registry)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("song A: fade_out") && err.contains("B"),
            "{err}"
        );
        // Loading it is another matter: the player cuts it short instead.
        assert!(super::Playlist::new("p", &long, Arc::clone(&registry)).is_ok());

        let fits =
            config_from("songs:\n- song: A\n  fade_out: 1measure\n- song: B\n  fade_in: 1s\n");
        assert!(super::validate_for_write(&fits, &registry).is_ok());
        let long_in = config_from("songs:\n- A\n- song: B\n  fade_in: 3measures\n");
        assert!(super::validate_for_write(&long_in, &registry).is_err());
    }

    #[test]
    fn a_song_playing_that_an_edit_takes_away_stays_current() {
        let old = playlist_from("songs:\n- song: A\n  fade_out: 2s\n- B\n- C\n").unwrap();
        old.navigate_to("B");
        let new = playlist_from("songs:\n- A\n- C\n").unwrap();
        new.follow(&old, true);
        assert_eq!(new.current().unwrap().name(), "B");
        assert_eq!(new.entry(), None);
        // C took its place: next goes there, previous to A, and a loop
        // break has no fade out of B's to take.
        assert_eq!(new.next_entry(), Some(1));
        assert_eq!(new.peek_next().unwrap().name(), "C");
        assert_eq!(new.peek_prev().unwrap().name(), "A");
        assert!(new.can_move(true) && new.can_move(false));
        let on = new.loop_break(false);
        assert_eq!(on.target.unwrap().name(), "C");
        assert!(on.fade_out.is_none());
        assert_eq!(new.next().unwrap().name(), "C");
        assert_eq!(new.entry(), Some(1));

        // Taken from the end: nothing after it.
        let new = playlist_from("songs:\n- A\n").unwrap();
        new.follow(&old, true);
        assert!(!new.can_move(false));
        assert_eq!(new.next_entry(), None);
        assert_eq!(new.next().unwrap().name(), "B");

        // Not playing, it gives way to the entry in its place.
        let new = playlist_from("songs:\n- A\n- C\n").unwrap();
        new.follow(&old, false);
        assert_eq!(new.current().unwrap().name(), "C");
        assert_eq!(new.entry(), Some(1));
    }

    #[test]
    fn transitions_must_name_the_songs_tracks() {
        let err = playlist_from("songs:\n- song: A\n  fade_out: {length: 2s, exclude: [click]}\n")
            .err()
            .unwrap()
            .to_string();
        assert!(err.contains("song A") && err.contains("click"), "{err}");
    }
}
