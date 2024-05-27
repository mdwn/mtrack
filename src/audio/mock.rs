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
use std::{
    collections::HashMap,
    error::Error,
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Barrier,
    },
    thread,
};

use tracing::{info, span, Level};

use crate::{playsync::CancelHandle, songs::Song};

/// A mock device. Doesn't actually play anything.
pub struct Device {
    name: String,
    is_playing: Arc<AtomicBool>,
}

impl Device {
    /// Gets the given mock device.
    pub fn get(name: &str) -> Device {
        Device {
            name: name.to_string(),
            is_playing: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Returns true if the device is currently playing.
    #[cfg(test)]
    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Relaxed)
    }
}

impl super::Device for Device {
    /// A mock device that will sleep for the length of the song duration.
    fn play(
        &self,
        song: Arc<Song>,
        _: &HashMap<String, u16>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
    ) -> Result<(), Box<dyn Error>> {
        let span = span!(Level::INFO, "play song (mock)");
        let _enter = span.enter();

        info!(
            device = self.name,
            song = song.name,
            duration = song.duration_string(),
            "Playing song."
        );

        let (sleep_tx, sleep_rx) = mpsc::channel::<()>();

        self.is_playing.store(true, Ordering::Relaxed);
        let join_handle = {
            let cancel_handle = cancel_handle.clone();
            // Wait until the song is cancelled or until the song is done.
            thread::spawn(move || {
                play_barrier.wait();

                // Wait for a signal or until we hit cancellation.
                let _ = sleep_rx.recv_timeout(song.duration);

                // Expire at the end of playback.
                cancel_handle.expire();
            })
        };

        cancel_handle.wait();
        sleep_tx.send(())?;
        let join_result = join_handle.join();

        self.is_playing.store(false, Ordering::Relaxed);

        if join_result.is_err() {
            return Err("Error while joining thread!".into());
        }

        Ok(())
    }
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (Mock)", self.name,)
    }
}
