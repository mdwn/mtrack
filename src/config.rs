// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Deserialize;
use tracing::{debug, error};

use self::player::Player;
use self::playlist::Playlist;

mod audio;
mod controller;
mod dmx;
mod midi;
mod player;
mod playlist;
mod song;
mod statusevents;
mod track;
mod trackmappings;

pub use crate::config::audio::Audio;
pub use crate::config::controller::Controller;
pub use crate::config::controller::MidiController;
pub use crate::config::dmx::Dmx;
pub use crate::config::dmx::Universe;
pub use crate::config::midi::Midi;
pub use crate::config::song::LightShow;
pub use crate::config::song::MidiPlayback;
pub use crate::config::song::Song;
pub use crate::config::statusevents::StatusEvents;
pub use crate::config::track::Track;

/// Parses songs from a YAML file.
pub fn parse_songs(file: &PathBuf) -> Result<Vec<crate::songs::Song>, Box<dyn Error>> {
    let start_path = match file.canonicalize()?.parent() {
        Some(file) => PathBuf::from(file),
        None => return Err(format!("error finding parent for {}", file.display()).into()),
    };
    let mut songs: Vec<crate::songs::Song> = Vec::new();

    for document in serde_yaml::Deserializer::from_str(&fs::read_to_string(file)?) {
        let song = match song::Song::deserialize(document) {
            Ok(song) => song,
            Err(e) => return Err(format!("error parsing file {}: {}", file.display(), e).into()),
        };
        songs.push(crate::songs::Song::new(&start_path, song)?);
    }

    Ok(songs)
}

/// Recurse into the given path and return all valid songs found.
pub fn get_all_songs(path: &PathBuf) -> Result<Arc<crate::songs::Songs>, Box<dyn Error>> {
    debug!("Getting songs for directory {path:?}");
    let mut songs: HashMap<String, Arc<crate::songs::Song>> = HashMap::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            get_all_songs(&path)?.list().iter().for_each(|song| {
                songs.insert(song.name().to_string(), song.clone());
            });
        }

        let extension = path.extension();
        if extension.is_some_and(|ext| ext == "yaml" || ext == "yml") {
            match parse_songs(&path) {
                Ok(parsed) => parsed.into_iter().for_each(|song| {
                    songs.insert(song.name().to_string(), Arc::new(song));
                }),
                Err(e) => error!(err = e.as_ref(), "Error while parsing files"),
            }
        }
    }

    Ok(Arc::new(crate::songs::Songs::new(songs)))
}

/// Initializes the player and controller from the given config files and returns the controller.
/// The controller owns the player, which can be waited on until it exits. Realistically, the
/// controller is not expected to exit.
pub fn init_player_and_controller(
    player_path: &PathBuf,
    playlist_path: &PathBuf,
) -> Result<crate::controller::Controller, Box<dyn Error>> {
    let player_config: Player = serde_yaml::from_str(&fs::read_to_string(player_path)?)?;
    let controller_config = player_config.controller();
    let device = crate::audio::get_device(player_config.audio())?;
    let midi_device = crate::midi::get_device(player_config.midi())?;
    let dmx_engine = crate::dmx::create_engine(player_config.dmx())?;
    let songs_path = get_songs_path(player_path, player_config.songs());
    let songs = get_all_songs(&songs_path)?;
    let playlist = parse_playlist(&PathBuf::from(playlist_path), Arc::clone(&songs))?;
    let status_events = crate::player::StatusEvents::new(player_config.status_events())?;

    let player = crate::player::Player::new(
        device,
        player_config.track_mappings().clone(),
        midi_device.clone(),
        dmx_engine,
        playlist,
        crate::playlist::Playlist::from_songs(songs)?,
        status_events,
    );
    crate::controller::Controller::new(player, midi_device, controller_config.clone())
}

fn get_songs_path(player_path: &PathBuf, songs: &str) -> PathBuf {
    let songs_path_config = PathBuf::from(songs);
    if songs_path_config.is_absolute() {
        return songs_path_config;
    }
    let player_path_directory = match player_path.parent() {
        Some(path) => path,
        None => {
            error!("Could not find parent of player path {player_path:?}");
            return songs_path_config;
        }
    };
    player_path_directory.join(songs)
}

/// Parse a playlist from a YAML file.
pub fn parse_playlist(
    file: &PathBuf,
    songs: Arc<crate::songs::Songs>,
) -> Result<Arc<crate::playlist::Playlist>, Box<dyn Error>> {
    let playlist: Playlist = serde_yaml::from_str(&fs::read_to_string(file)?)?;
    Ok(Arc::new(crate::playlist::Playlist::new(
        playlist.songs().clone(),
        songs,
    )?))
}
