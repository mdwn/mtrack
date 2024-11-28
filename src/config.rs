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
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use midly::live::LiveEvent;
use serde::Deserialize;
use tracing::error;

use crate::player::StatusEvents;
use crate::{audio, dmx};

use self::midi::ToMidiEvent;
use self::player::Player;
use self::playlist::Playlist;

mod controller;
mod midi;
mod player;
mod playlist;
mod song;
mod track;
mod trackmappings;

/// Parses songs from a YAML file.
pub fn parse_songs(file: &PathBuf) -> Result<Vec<crate::songs::Song>, Box<dyn Error>> {
    let mut songs: Vec<song::Song> = Vec::new();

    for document in serde_yaml::Deserializer::from_str(&fs::read_to_string(file)?) {
        let mut song = match song::Song::deserialize(document) {
            Ok(song) => song,
            Err(e) => return Err(format!("error parsing file {}: {}", file.display(), e).into()),
        };
        song.song_file = file.canonicalize()?;
        songs.push(song);
    }

    songs
        .into_iter()
        .map(|song| song.to_song())
        .collect::<Result<Vec<crate::songs::Song>, Box<dyn Error>>>()
}

/// Recurse into the given path and return all valid songs found.
pub fn get_all_songs(path: &PathBuf) -> Result<Arc<crate::songs::Songs>, Box<dyn Error>> {
    let mut songs: HashMap<String, Arc<crate::songs::Song>> = HashMap::new();
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            get_all_songs(&path)?.list().iter().for_each(|song| {
                songs.insert(song.name.to_string(), song.clone());
            });
        }

        let extension = path.extension();
        if extension.is_some_and(|ext| ext == "yaml" || ext == "yml") {
            match parse_songs(&path) {
                Ok(parsed) => parsed.into_iter().for_each(|song| {
                    songs.insert(song.name.clone(), Arc::new(song));
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
    let device = audio::get_device(&player_config.audio_device)?;
    let midi_device = player_config
        .midi_device
        .map(|midi_device| crate::midi::get_device(&midi_device))
        .map_or(Ok(None), |result| result.map(Some))?;
    let dmx_device = dmx::get_device();
    let songs = get_all_songs(&PathBuf::from(player_config.songs))?;
    let playlist = parse_playlist(&PathBuf::from(playlist_path), Arc::clone(&songs))?;
    let status_events = match player_config.status_events {
        Some(status_events) => Some(StatusEvents::new(
            status_events
                .off_events
                .iter()
                .map(|event| event.to_midi_event())
                .collect::<Result<Vec<LiveEvent<'static>>, Box<dyn Error>>>()?,
            status_events
                .idling_events
                .iter()
                .map(|event| event.to_midi_event())
                .collect::<Result<Vec<LiveEvent<'static>>, Box<dyn Error>>>()?,
            status_events
                .playing_events
                .iter()
                .map(|event| event.to_midi_event())
                .collect::<Result<Vec<LiveEvent<'static>>, Box<dyn Error>>>()?,
        )),
        None => None,
    };

    let player = crate::player::Player::new(
        device,
        player_config.track_mappings.track_mappings,
        midi_device.clone(),
        dmx_device,
        playlist,
        crate::playlist::Playlist::from_songs(songs)?,
        status_events,
    );
    let controller = crate::controller::Controller::new(
        player,
        player_config.controller.driver(midi_device.clone())?,
    )?;
    Ok(controller)
}

/// Parse a playlist from a YAML file.
pub fn parse_playlist(
    file: &PathBuf,
    songs: Arc<crate::songs::Songs>,
) -> Result<Arc<crate::playlist::Playlist>, Box<dyn Error>> {
    let playlist: Playlist = serde_yaml::from_str(&fs::read_to_string(file)?)?;
    Ok(Arc::new(crate::playlist::Playlist::new(
        playlist.songs,
        songs,
    )?))
}
