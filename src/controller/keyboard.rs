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
use std::io;

use tokio::{sync::mpsc::Sender, task::JoinHandle};
use tracing::{info, span, warn, Level};

use super::Event;

const PLAY: &str = "play";
const PREV: &str = "prev";
const NEXT: &str = "next";
const STOP: &str = "stop";
const ALL_SONGS: &str = "all_songs";
const PLAYLIST: &str = "playlist";

/// A controller that controls a player using the keyboard.
pub struct Driver {}

impl Driver {
    pub fn new() -> Driver {
        Driver {}
    }

    fn monitor_io<R, W>(
        events_tx: &Sender<Event>,
        mut reader: R,
        mut writer: W,
    ) -> Result<(), io::Error>
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
            PLAY => events_tx.blocking_send(Event::Play),
            PREV => events_tx.blocking_send(Event::Prev),
            NEXT => events_tx.blocking_send(Event::Next),
            STOP => events_tx.blocking_send(Event::Stop),
            ALL_SONGS => events_tx.blocking_send(Event::AllSongs),
            PLAYLIST => events_tx.blocking_send(Event::Playlist),
            _ => {
                warn!(input = input, "Unrecognized input");
                Ok(())
            }
        }
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        Ok(())
    }
}

impl super::Driver for Driver {
    fn monitor_events(&self, events_tx: Sender<Event>) -> JoinHandle<Result<(), io::Error>> {
        tokio::task::spawn_blocking(move || {
            let span = span!(Level::INFO, "keyboard driver");
            let _enter = span.enter();

            info!("Keyboard driver started.");

            loop {
                Self::monitor_io(&events_tx, io::stdin().lock(), io::stdout())?;
            }
        })
    }
}

#[cfg(test)]
mod test {
    use std::io::{self, BufReader, BufWriter};

    use tokio::sync::mpsc;

    use crate::controller::{keyboard::*, Event};

    use super::{Driver, PLAY};

    fn get_event(event: &str) -> Result<Option<Event>, io::Error> {
        let (sender, mut receiver) = mpsc::channel::<Event>(1);

        let reader_bytes = event.as_bytes();
        let reader = BufReader::new(reader_bytes);

        let writer_bytes: Vec<u8> = vec![0; 255];
        let writer = BufWriter::new(writer_bytes);
        Driver::monitor_io(&sender, reader, writer)?;

        // Force the sender to close.
        drop(sender);
        Ok(receiver.blocking_recv())
    }

    #[test]
    fn test_keyboard_events() -> Result<(), io::Error> {
        assert_eq!(Event::Play, get_event(PLAY)?.unwrap());
        assert_eq!(Event::Prev, get_event(PREV)?.unwrap());
        assert_eq!(Event::Next, get_event(NEXT)?.unwrap());
        assert_eq!(Event::Stop, get_event(STOP)?.unwrap());
        assert_eq!(Event::AllSongs, get_event(ALL_SONGS)?.unwrap());
        assert_eq!(Event::Playlist, get_event(PLAYLIST)?.unwrap());
        assert_eq!(None, get_event("unrecognized")?);
        Ok(())
    }
}
