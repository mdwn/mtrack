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
use std::{io, sync::Arc};

use tokio::task::JoinHandle;
use tracing::{info, span, warn, Level};

use crate::player::Player;

const PLAY: &str = "play";
const PREV: &str = "prev";
const NEXT: &str = "next";
const STOP: &str = "stop";
const ALL_SONGS: &str = "all_songs";
const PLAYLIST: &str = "playlist";

/// A controller that controls a player using the keyboard.
pub struct Driver {
    player: Arc<Player>,
}

impl Driver {
    pub fn new(player: Arc<Player>) -> Arc<Self> {
        Arc::new(Driver { player })
    }

    fn monitor_io<R, W>(player: Arc<Player>, mut reader: R, mut writer: W) -> Result<(), io::Error>
    where
        R: io::BufRead,
        W: io::Write,
    {
        write!(
            writer,
            "Command ({}, {}, {}, {}, {}, {}): ",
            PLAY, PREV, NEXT, STOP, ALL_SONGS, PLAYLIST,
        )?;
        writer.flush()?;
        let mut input: String = String::default();
        reader.read_line(&mut input)?;

        match input.trim().to_lowercase().as_str() {
            PLAY => {
                let player = player.clone();
                tokio::spawn(async move { player.play().await });
            }
            PREV => {
                let player = player.clone();
                tokio::spawn(async move { player.prev().await });
            }
            NEXT => {
                let player = player.clone();
                tokio::spawn(async move { player.next().await });
            }
            STOP => {
                let player = player.clone();
                tokio::spawn(async move { player.stop().await });
            }
            ALL_SONGS => {
                let player = player.clone();
                tokio::spawn(async move { player.switch_to_all_songs().await });
            }
            PLAYLIST => {
                let player = player.clone();
                tokio::spawn(async move { player.switch_to_playlist().await });
            }
            _ => {
                warn!(input = input, "Unrecognized input");
            }
        }
        Ok(())
    }
}

impl super::Driver for Driver {
    fn monitor_events(&self) -> JoinHandle<Result<(), io::Error>> {
        let player = self.player.clone();
        tokio::task::spawn_blocking(move || {
            let span = span!(Level::INFO, "keyboard driver");
            let _enter = span.enter();

            info!("Keyboard driver started.");

            loop {
                Self::monitor_io(player.clone(), io::stdin().lock(), io::stdout())?;
            }
        })
    }
}
