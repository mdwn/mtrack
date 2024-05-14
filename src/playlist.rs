// Copyright (C) 2024 Michael Wilson <mike@mdwn.dev>
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

use crate::songs::{Song, Songs};
use core::fmt;
use std::error::Error;
use std::sync::{Arc, RwLock};

/// Playlist is a playlist for use by a player.
pub struct Playlist {
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
                Ok(song) => writeln!(f, "  - {} (Channels: {})", song.name, song.num_channels)?,
                Err(_) => writeln!(f, "  - {} (unable to find song)", song_name)?,
            };
        }

        Ok(())
    }
}

impl Playlist {
    /// Creates a new playlist.
    pub fn new(song_names: Vec<String>, registry: Arc<Songs>) -> Result<Playlist, Box<dyn Error>> {
        // Verify that each song in the playlist exists in the registry.
        for song_name in song_names.iter() {
            registry.get(song_name)?;
        }

        Ok(Playlist {
            songs: song_names,
            position: Arc::new(RwLock::new(0)),
            registry: Arc::clone(&registry),
            span: span!(Level::INFO, "playlist"),
        })
    }

    /// Creates an alphabetized playlist from all available songs.
    pub fn from_songs(songs: Arc<Songs>) -> Result<Arc<Playlist>, Box<dyn Error>> {
        // The easiest thing to do here is to gather the names of all of the songs and pass them
        // to new. This is a little silly, since new is just going to double check that they
        // all exist and then do an explicit mapping each time. However, the easiest way to
        // make from_file work is to do it this way, so we'll just do this rigamarole for now.
        let sorted = Vec::from_iter(
            songs
                .sorted_list()
                .into_iter()
                .map(|song| song.name.clone()),
        );
        Ok(Arc::new(Playlist::new(sorted, songs)?))
    }

    /// Move to the next element of the playlist. If we're at the end of the playlist, the position will not
    /// increment. The song at the current position will be returned.
    pub fn next(&self) -> Arc<Song> {
        let _enter = self.span.enter();

        let mut position = self.position.write().expect("unable to get lock");
        if *position < self.songs.len() - 1 {
            *position += 1;
        }

        let current = &self
            .registry
            .get(&self.songs[*position])
            .expect("unable to get song from the registry");

        info!(
            position = *position,
            song = current.name,
            "Moving to next playlist position."
        );

        current.clone()
    }

    /// Move to the previous element of the playlist. If we're at the beginning of the playlist, the position
    /// will not decrement. The song at the current position will be returned.
    pub fn prev(&self) -> Arc<Song> {
        let mut position = self.position.write().unwrap();
        if *position > 0 {
            *position -= 1;
        }

        let current = &self.registry.get(&self.songs[*position]).unwrap();

        info!(
            position = *position,
            song = current.name,
            "Moving to next previous position."
        );

        current.clone()
    }

    /// Return the song at the current position of the playlist.
    pub fn current(&self) -> Arc<Song> {
        let position = self.position.read().expect("unable to get lock");
        Arc::clone(
            &self
                .registry
                .get(&self.songs[*position])
                .expect("unable to find song in the registry"),
        )
    }
}

#[cfg(test)]
mod test {
    use std::path::PathBuf;

    use crate::config;

    #[test]
    fn test_playlist() {
        let songs = config::get_all_songs(&PathBuf::from("assets/songs"))
            .expect("Parse songs should have succeeded.");

        let playlist =
            super::Playlist::new(vec!["Song 1".to_string(), "Song 2".to_string()], songs)
                .expect("Unable to create playlist");

        // Starts at the first element in the list.
        assert_eq!("Song 1", playlist.current().name);

        // Previous should just stay at the beginning of the list, since it's at the start.
        playlist.prev();
        assert_eq!("Song 1", playlist.current().name);

        // Next goes to the next entry.
        playlist.next();
        assert_eq!("Song 2", playlist.current().name);

        // Next should just stay at the end of the list, since it's at the end.
        playlist.next();
        assert_eq!("Song 2", playlist.current().name);

        // Prev goes to the previous entry.
        playlist.prev();
        assert_eq!("Song 1", playlist.current().name);
    }
}
