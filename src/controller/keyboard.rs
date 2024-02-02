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
use std::io::{self, Write};

use tokio::{sync::mpsc::Sender, task::JoinHandle};
use tracing::{info, span, Level};

use super::Event;

/// A controller that controls a player using the keyboard.
pub struct Driver {}

impl Driver {
    pub fn new() -> Driver {
        Driver {}
    }
}

impl super::Driver for Driver {
    fn monitor_events(&self, events_tx: Sender<Event>) -> JoinHandle<Result<(), io::Error>> {
        tokio::task::spawn_blocking(move || {
            let span = span!(Level::INFO, "keyboard driver");
            let _enter = span.enter();

            info!("Keyboard driver started.");

            loop {
                print!("Command (play, prev, next, stop, all_songs, playlist): ");
                io::stdout().flush()?;
                let mut input: String = String::default();
                io::stdin().read_line(&mut input)?;

                match input.as_str() {
                    "play" => events_tx.blocking_send(Event::Play),
                    "prev" => events_tx.blocking_send(Event::Prev),
                    "next" => events_tx.blocking_send(Event::Next),
                    "stop" => events_tx.blocking_send(Event::Stop),
                    "all_songs" => events_tx.blocking_send(Event::AllSongs),
                    "playlist" => events_tx.blocking_send(Event::Playlist),
                    _ => Ok(()),
                }
                .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
            }
        })
    }
}
