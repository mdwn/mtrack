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

/// Typed error for playlist creation so callers can distinguish e.g. missing song in registry.
#[derive(Debug, thiserror::Error)]
pub enum PlaylistError {
    #[error("Song not in registry: {0}")]
    SongNotFound(String),
}

/// Playlist is a playlist for use by a player.
pub struct Playlist {
    /// The name of this playlist.
    name: String,
    /// The songs that this playlist will play.
    songs: Vec<String>,
    /// The current position of the playlist.
    position: Arc<RwLock<usize>>,
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

        Ok(Arc::new(Playlist {
            name: name.to_string(),
            songs: song_names.clone(),
            position: Arc::new(RwLock::new(0)),
            registry: Arc::clone(&registry),
            span: span!(Level::INFO, "playlist"),
        }))
    }

    /// Returns the name of this playlist.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the current position in the playlist (0-indexed).
    pub fn position(&self) -> usize {
        *self.position.read()
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
        if *position < self.songs.len() - 1 {
            *position += 1;
        }

        let current = &self
            .registry
            .get(&self.songs[*position])
            .expect("unable to get song from the registry");

        info!(
            position = *position,
            song = current.name(),
            "Moving to next playlist position."
        );

        Some(current.clone())
    }

    /// Move to the previous element of the playlist. If we're at the beginning of the playlist, the position
    /// will not decrement. The song at the current position will be returned.
    /// Returns `None` if the playlist is empty.
    pub fn prev(&self) -> Option<Arc<Song>> {
        if self.songs.is_empty() {
            return None;
        }

        let mut position = self.position.write();
        if *position > 0 {
            *position -= 1;
        }

        let current = &self
            .registry
            .get(&self.songs[*position])
            .expect("unable to find song in the registry");

        info!(
            position = *position,
            song = current.name(),
            "Moving to next previous position."
        );

        Some(current.clone())
    }

    /// Returns the underlying song registry.
    pub fn registry(&self) -> &Arc<Songs> {
        &self.registry
    }

    /// Look up a song by name from the underlying registry.
    pub fn get_song(&self, name: &str) -> Option<Arc<Song>> {
        self.registry.get(name).ok()
    }

    /// Return the song at the current position of the playlist.
    /// Returns `None` if the playlist is empty.
    pub fn current(&self) -> Option<Arc<Song>> {
        if self.songs.is_empty() {
            return None;
        }
        let position = self.position.read();
        Some(Arc::clone(
            &self
                .registry
                .get(&self.songs[*position])
                .expect("unable to find song in the registry"),
        ))
    }
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
            position: Arc::new(parking_lot::RwLock::new(0)),
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
    fn display_with_missing_song_shows_error() {
        // Construct a Playlist directly (bypassing new()'s registry check)
        // to exercise the Display path for a song not in the registry.
        let registry = test_registry();
        let playlist = super::Playlist {
            name: "broken".to_string(),
            songs: vec!["Song 1".to_string(), "Ghost Song".to_string()],
            position: Arc::new(parking_lot::RwLock::new(0)),
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
}
