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
use std::{
    collections::HashMap,
    error::Error,
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
use tracing::{error, info, warn};

use crate::{
    playlist::{self, Playlist},
    songs::{self, Song, Songs},
};

use super::{Player, PlaylistDirection};

impl Player {
    /// Plays a specific song by name, starting from the given time.
    /// Switches to the all_songs playlist (session-only, not persisted) and
    /// navigates to the song before calling play_from.
    /// Returns an error if the song is not found.
    pub async fn play_song_from(
        &self,
        song_name: &str,
        start_time: Duration,
    ) -> Result<Option<Arc<Song>>, Box<dyn Error>> {
        // Reject playback if hardware hasn't finished initializing.
        if !*self.init_done_tx.borrow() {
            return Err("Hardware is still initializing".into());
        }

        let mut join = self.join.lock().await;
        if join.is_some() {
            info!("Player is already playing a song.");
            return Ok(None);
        }

        let all_songs = self
            .get_all_songs_playlist()
            .ok_or_else(|| -> Box<dyn Error> { "all_songs playlist not available".into() })?;
        if all_songs.navigate_to(song_name).is_none() {
            return Err(format!("Song '{}' not found", song_name).into());
        }
        *self.active_playlist.write() = "all_songs".to_string();

        // Start playback with the lock already held.
        self.play_from_locked(start_time, &mut join).await
    }

    /// Navigates the playlist in the given direction, emitting the song-change
    /// event. Returns the current song if the player is active.
    ///
    /// Holds the join lock across the entire operation to prevent a concurrent
    /// `play_from()` from starting between the is-playing check and the
    /// playlist position advance.
    pub(super) async fn navigate(&self, direction: PlaylistDirection) -> Option<Arc<Song>> {
        let join = self.join.lock().await;
        if join.is_some() {
            // If the current song is looping, break out immediately with crossfade.
            if self.is_current_song_looping() {
                info!("Breaking out of song loop via {} navigation.", direction);
                self.fade_out_current_audio();
                self.loop_break.store(true, Ordering::Relaxed);
                if let Some(ref handles) = *join {
                    handles.cancel.cancel();
                }
                return self.get_playlist().current();
            }
            let current = self.get_playlist().current();
            if let Some(ref song) = current {
                info!(
                    current_song = song.name(),
                    "Can't go to {}, player is active.", direction
                );
            }
            return current;
        }
        let playlist = self.get_playlist();
        let song = match direction {
            PlaylistDirection::Next => playlist.next()?,
            PlaylistDirection::Prev => playlist.prev()?,
        };
        self.emit_song_change(&song);
        drop(join);
        self.load_song_samples(&song);
        Some(song)
    }

    /// Next goes to the next entry in the playlist.
    pub async fn next(&self) -> Option<Arc<Song>> {
        self.navigate(PlaylistDirection::Next).await
    }

    /// Prev goes to the previous entry in the playlist.
    pub async fn prev(&self) -> Option<Arc<Song>> {
        self.navigate(PlaylistDirection::Prev).await
    }

    /// Switches the active playlist by name. Returns an error if the name
    /// doesn't exist in the map or if the player is currently playing.
    /// Switching to "all_songs" is session-only (not persisted to config).
    pub async fn switch_to_playlist(&self, name: &str) -> Result<(), String> {
        {
            let join = self.join.lock().await;
            if join.is_some() {
                if let Some(current) = self.get_playlist().current() {
                    info!(
                        current_song = current.name(),
                        "Can't switch to {}, player is active.", name
                    );
                }
                return Err("Cannot switch playlist while playing".to_string());
            }
        }

        // Validate the name exists.
        {
            let playlists = self.playlists.read();
            if !playlists.contains_key(name) {
                return Err(format!("Playlist '{}' not found", name));
            }
        }

        *self.active_playlist.write() = name.to_string();

        // Persist the choice to the config store, unless it's "all_songs" (session-only).
        if name != "all_songs" {
            *self.persisted_playlist.write() = name.to_string();
            if let Some(store) = self.config_store() {
                if let Err(e) = store.set_active_playlist(name.to_string()).await {
                    warn!("Failed to persist active playlist: {}", e);
                }
            }
        }

        if let Some(song) = self.get_playlist().current() {
            self.emit_song_change(&song);
        }

        Ok(())
    }

    /// Returns the persisted active playlist name (the last non-all_songs choice).
    /// This is what MIDI/OSC `Playlist` action uses to "go back to my real playlist".
    pub fn persisted_playlist_name(&self) -> String {
        self.persisted_playlist.read().clone()
    }

    /// Returns a sorted list of all playlist names.
    pub fn list_playlists(&self) -> Vec<String> {
        let playlists = self.playlists.read();
        let mut names: Vec<String> = playlists.keys().cloned().collect();
        names.sort();
        names
    }

    /// Returns a snapshot of all playlists.
    pub fn playlists_snapshot(&self) -> HashMap<String, Arc<Playlist>> {
        self.playlists.read().clone()
    }

    /// Gets the all-songs playlist (every song in the registry).
    pub fn get_all_songs_playlist(&self) -> Option<Arc<Playlist>> {
        let playlists = self.playlists.read();
        let result = playlists.get("all_songs").cloned();
        if result.is_none() {
            error!("all_songs playlist missing from player state");
        }
        result
    }

    /// Gets the current playlist used by the player.
    pub fn get_playlist(&self) -> Arc<Playlist> {
        let name = self.active_playlist.read().clone();
        let playlists = self.playlists.read();
        match playlists.get(&name).or_else(|| playlists.get("all_songs")) {
            Some(playlist) => playlist.clone(),
            None => {
                // This should never happen because all_songs is always created
                // during initialization, but handle it gracefully instead of panicking.
                error!("No playlist available (not even all_songs) — returning empty fallback");
                drop(playlists);
                let empty_songs = Arc::new(Songs::new(std::collections::HashMap::new()));
                playlist::from_songs(empty_songs).expect("empty playlist construction cannot fail")
            }
        }
    }

    /// Returns the song registry from the all-songs playlist.
    pub fn songs(&self) -> Arc<Songs> {
        let playlists = self.playlists.read();
        match playlists.get("all_songs") {
            Some(playlist) => playlist.registry().clone(),
            None => {
                error!("all_songs playlist missing from player state");
                Arc::new(Songs::new(std::collections::HashMap::new()))
            }
        }
    }

    /// Reinitializes all song-related state by rescanning songs from disk and
    /// rebuilding all playlists. Call this after any mutation that affects songs
    /// (import, create, config edit, etc.).
    pub fn reload_songs(
        &self,
        songs_path: &std::path::Path,
        playlists_dir: Option<&std::path::Path>,
        legacy_playlist_path: Option<&std::path::Path>,
    ) {
        let new_songs = match songs::get_all_songs(songs_path) {
            Ok(s) => s,
            Err(e) => {
                warn!("Failed to rescan songs: {}", e);
                return;
            }
        };

        let new_playlists =
            match super::load_playlists(playlists_dir, legacy_playlist_path, new_songs.clone()) {
                Ok(p) => p,
                Err(e) => {
                    warn!("Failed to rebuild playlists: {}", e);
                    return;
                }
            };

        // Preserve active playlist name if it still exists; fall back to all_songs.
        {
            let mut active = self.active_playlist.write();
            if !new_playlists.contains_key(active.as_str()) {
                *active = "all_songs".to_string();
            }
        }

        *self.playlists.write() = new_playlists;
        info!("Reloaded song state");
    }
}
