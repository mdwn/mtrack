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

use super::{
    LoopExit, Player, PlaylistDirection, ReloadSongsError, SongSources, SwitchPlaylistError,
};

impl Player {
    /// Plays a specific song by name, starting from the given time.
    /// If the song is in the active playlist, navigates to it there — jumping
    /// to a song never kicks you out of the playlist you chose. Only songs
    /// outside the active playlist switch to the all_songs playlist
    /// (session-only, not persisted).
    /// Returns an error if the song is not found.
    pub async fn play_song_from(
        &self,
        song_name: &str,
        start_time: Duration,
    ) -> Result<Option<Arc<Song>>, Box<dyn Error>> {
        let mut join = self.join.lock().await;
        if join.is_some() {
            info!("Player is already playing a song.");
            return Ok(None);
        }
        // Asked before the playlist moves, so a start turned down leaves it
        // where it was; the start asks again, as every start does.
        if !*self.init_done_tx.borrow() {
            return Err("Hardware is still initializing".into());
        }

        // A start the song cannot play from is turned down before the
        // playlist moves to it (#584).
        if let Some(song) = self.get_playlist().get_song(song_name) {
            super::seek::check_start_position(&song, start_time, "play from")?;
        }

        // Moved as a navigation moves it, so that a reload cannot come
        // between looking the playlist up and moving it and lose the move,
        // and announced as a navigation announces it.
        let (_, song) = self.move_playlist(|playlist| -> Result<Arc<Song>, String> {
            if let Some(song) = playlist.navigate_to(song_name) {
                return Ok(song);
            }
            let all_songs = self
                .get_all_songs_playlist()
                .ok_or("all_songs playlist not available")?;
            let song = all_songs
                .navigate_to(song_name)
                .ok_or_else(|| format!("Song '{}' not found", song_name))?;
            *self.active_playlist.write() = "all_songs".to_string();
            Ok(song)
        });
        let song = song?;
        self.clear_pending_start();
        self.emit_song_change(&song);

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
        let mut join = self.join.lock().await;
        if join.is_some() {
            // A looping song ending: stopped a moment from now.
            if self.loop_exit() == LoopExit::End {
                info!("The song is ending; {} once it has.", direction);
                return self.get_playlist().current();
            }
            // A looping song at the end of the list has nowhere to move to:
            // it ends as a stop ends it, rather than starting over.
            let back = direction == PlaylistDirection::Prev;
            if self.is_current_song_looping() && !self.get_playlist().can_move(back) {
                info!("Nothing to go to {}: ending the looping song.", direction);
                if let Some(song) = self.end_looping_song(&join) {
                    return Some(song);
                }
                self.cut_locked(&mut join).await;
                return self.get_playlist().current();
            }
            // If the current song is looping, break out immediately with crossfade.
            // The cleanup task moves the playlist in this direction and plays
            // what it lands on a moment later; that song is the answer, not the
            // one still fading out (#545).
            if self.is_current_song_looping() {
                info!("Breaking out of song loop via {} navigation.", direction);
                return self.break_loop(&join, back);
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
        let (playlist, (from, before, song)) = self.move_playlist(|playlist| {
            let from = playlist.entry();
            let before = playlist.ambient_before(self.waiting_entry(playlist));
            // A paused song an edit took off the list after the last entry
            // has no entry to move on to: it stays paused, and current, as
            // at the end of the list, rather than losing its pause and
            // staying off the list (#576).
            if from.is_none() && !playlist.can_move(direction == PlaylistDirection::Prev) {
                return (from, before, None);
            }
            let song = match direction {
                PlaylistDirection::Next => playlist.next(),
                PlaylistDirection::Prev => playlist.prev(),
            };
            (from, before, Some(song))
        });
        let Some(song) = song else {
            info!("Nothing to go to {}: the paused song stays.", direction);
            return playlist.current();
        };
        let song = song?;
        // A paused song holds its lights, which a navigation leaves held.
        let paused = self.pending_start().is_some();
        self.clear_pending_start();
        // A pause's section loop is not resumed after a navigation.
        self.section_loop.reset();
        self.emit_song_change(&song);
        // Navigating onto a song with `auto_play` starts it; navigating away
        // from one waiting to start calls it off, and the lights go to
        // ambient as at any stop.
        let called_off = self.cancel_auto_play();
        // A song ending fades on out; the ambient light following it, now
        // this navigation's to bring.
        let ending = self.end_ending();
        let moved = playlist.entry() != from;
        if moved {
            self.transport.lock().past_last_entry = false;
        }
        // Stopped on another entry: the gap is now before it, and its
        // ambient light, if that is another, comes up in place of the one
        // before.
        let ambient_changed =
            !paused && playlist.ambient_before(self.waiting_entry(&playlist)) != before;
        match playlist.auto_play().filter(|_| moved) {
            Some(gap) => self.schedule_auto_play(gap),
            None if called_off || ending || ambient_changed => self.idle_ambient(true),
            None => {}
        }
        // A pause called off here, or an ending forgotten, which then no
        // longer runs a reload waiting for its fade: the reload looks again
        // now.
        self.reload_deferred_hardware();
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
    pub async fn switch_to_playlist(&self, name: &str) -> Result<(), SwitchPlaylistError> {
        // Held from the check through the switch: let go between them, a
        // play could start a song from the playlist being left, which the
        // switch then took out from under it.
        let join = self.join.lock().await;
        if join.is_some() {
            if let Some(current) = self.get_playlist().current() {
                info!(
                    current_song = current.name(),
                    "Can't switch to {}, player is active.", name
                );
            }
            return Err(SwitchPlaylistError::Playing);
        }

        // Validate the name exists.
        {
            let playlists = self.playlists.read();
            if !playlists.contains_key(name) {
                return Err(SwitchPlaylistError::NotFound(name.to_string()));
            }
        }

        // A paused song an edit took off the list stays current only to be
        // resumed, which the switch calls off: the playlist left goes on to
        // the entry that took its place.
        self.move_playlist(|playlist| playlist.settle());
        *self.active_playlist.write() = name.to_string();
        self.transport.lock().past_last_entry = false;
        self.clear_pending_start();
        self.section_loop.reset();
        self.cancel_auto_play();
        self.end_ending();
        // As for a navigation: a reload waiting on a pause or an ending
        // looks again.
        self.reload_deferred_hardware();
        // The switch is made: saving it to the config store need not hold
        // up playback.
        drop(join);

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
        // The new playlist's ambient light, if it has its own.
        self.idle_ambient(false);

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

    /// Moves the active playlist's place with `step`, returning the playlist
    /// and what `step` returns. A reload cannot come between looking the
    /// playlist up and moving it, so the move is not lost to it.
    pub(super) fn move_playlist<T>(&self, step: impl FnOnce(&Playlist) -> T) -> (Arc<Playlist>, T) {
        let _moves = self.playlist_moves.lock();
        let playlist = self.get_playlist();
        let result = step(&playlist);
        (playlist, result)
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
    /// rebuilding all playlists, from the player's song sources. Call this
    /// after any mutation that affects songs (import, create, config edit,
    /// etc.). The rescan reads every song from disk, so it runs on the
    /// blocking pool rather than on the caller's runtime worker. A rescan
    /// or rebuild that fails leaves the songs as they were, and says why, so
    /// the caller does not report a change the player has not taken up.
    ///
    /// The sources are resolved again from the config store first, so a
    /// runtime change to `songs:` or `playlists_dir:` is taken up, and
    /// become the player's once the reload from them succeeds.
    pub async fn reload_songs(&self) -> Result<(), ReloadSongsError> {
        let sources = self
            .resolve_song_sources()
            .await
            .ok_or(ReloadSongsError::NoSources)?;
        self.reload_songs_into(sources).await
    }

    /// Reloads the songs from `sources` on the blocking pool, and takes the
    /// sources up once that succeeds.
    pub(super) async fn reload_songs_into(
        &self,
        sources: SongSources,
    ) -> Result<(), ReloadSongsError> {
        let player = self.clone();
        tokio::task::spawn_blocking(move || {
            player.reload_songs_from(&sources)?;
            player.set_song_sources(sources);
            Ok(())
        })
        .await
        .map_err(|e| ReloadSongsError::Interrupted(e.to_string()))?
    }

    /// [`Player::reload_songs`] from `sources`, on the calling thread.
    pub(crate) fn reload_songs_from(&self, sources: &SongSources) -> Result<(), ReloadSongsError> {
        // One scan at a time, from its scan to its swap.
        let _scan = self.song_scans.lock();
        let new_songs =
            songs::get_all_songs_with_defaults(&sources.songs, self.default_metronome()).map_err(
                |e| {
                    warn!("Failed to rescan songs: {}", e);
                    ReloadSongsError::Songs(e.to_string())
                },
            )?;

        let new_playlists = super::load_playlists(
            sources.playlists_dir.as_deref(),
            sources.legacy_playlist.as_deref(),
            new_songs.clone(),
        )
        .map_err(|e| {
            warn!("Failed to rebuild playlists: {}", e);
            ReloadSongsError::Playlists(e.to_string())
        })?;

        // Copying each playlist's place and swapping the new ones in is one
        // step to a move of the active playlist's place, such as a song's
        // end advancing it: one made between the two would be made on the
        // old playlist, and lost.
        let moves = self.playlist_moves.lock();
        // The ambient light for the gap the player is in, as the edit finds
        // it.
        let ambient_before = self.gap_ambient();

        // Preserve active playlist name if it still exists; fall back to all_songs.
        let (was_active, active) = {
            let mut active = self.active_playlist.write();
            let was_active = active.clone();
            if !new_playlists.contains_key(active.as_str()) {
                *active = "all_songs".to_string();
            }
            (was_active, active.clone())
        };

        // Each playlist keeps its place by song, not by index: the song
        // playing stays current through an edit that moves it, or takes it
        // off the list, and the ending, loop-break and status logic, which
        // read the active playlist's current song, keep to it (#576). A
        // paused song counts as playing: play resumes it where it was, and
        // the entry after it is not the song to resume. Asked of the flag
        // kept with `join`, not of the join lock: every status poll takes
        // that lock, and a stopped song read as playing for one would keep
        // its place off the list.
        let playing = self.song_playing.load(Ordering::Relaxed) || self.pending_start().is_some();
        {
            let old_playlists = self.playlists.read();
            for (name, playlist) in &new_playlists {
                // With the active playlist gone, deleted or renamed, the
                // all_songs it falls back to takes over from it rather than
                // from its own place, so the song playing stays current.
                let from = if *name == active {
                    old_playlists.get(&was_active)
                } else {
                    old_playlists.get(name)
                };
                if let Some(old) = from {
                    playlist.follow(old, playing && *name == active);
                }
            }
        }

        *self.playlists.write() = new_playlists;
        drop(moves);
        info!("Reloaded song state");

        let called_off = self.recheck_auto_play();
        // Stopped, the gap's ambient light is the edited playlist's, as a
        // navigation onto another entry brings that entry's: a new show, or
        // another's settings. A song playing or paused holds its lights,
        // and a song waiting to start by itself the one before's, as a segue.
        let idle = !self.song_playing.load(Ordering::Relaxed)
            && self.pending_start().is_none()
            && self.auto_play_status().is_none();
        if called_off || (idle && self.gap_ambient() != ambient_before) {
            self.idle_ambient(true);
        }
        Ok(())
    }

    /// The ambient light for the gap the player is in, and whose settings
    /// it plays, as [`Player::idle_ambient`] would bring it up.
    fn gap_ambient(
        &self,
    ) -> (
        Option<crate::config::ambient::PlaylistAmbient>,
        playlist::AmbientSource,
    ) {
        let playlist = self.get_playlist();
        let entry = self.waiting_entry(&playlist);
        (
            playlist.ambient_before(entry),
            playlist.ambient_source(entry),
        )
    }

    /// Calls off an auto play waiting out its gap that a playlist edit has
    /// left with nothing to wait for: its song gone from the playlist's
    /// place, or its entry no longer starting by itself. One still waited
    /// for waits on, its gap as it was. Returns whether one was called off.
    fn recheck_auto_play(&self) -> bool {
        let Some((song, _)) = self.auto_play_status() else {
            return false;
        };
        let playlist = self.get_playlist();
        let still_waits = playlist.auto_play().is_some()
            && playlist
                .current()
                .is_some_and(|current| current.name() == song);
        if still_waits {
            return false;
        }
        info!(
            song,
            "Auto play called off: the playlist edit took it away."
        );
        self.cancel_auto_play()
    }
}
