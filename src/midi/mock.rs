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
    error::Error,
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Barrier, Mutex,
    },
    thread,
    time::Duration,
};

use midly::live::LiveEvent;
use tokio::{sync::mpsc::Sender, task::JoinHandle};
use tracing::{info, span, Level};

use crate::{playsync::CancelHandle, songs::Song};

/// A mock device. Doesn't actually play anything.
#[derive(Clone)]
pub struct Device {
    name: String,
    barrier: Arc<Barrier>,
    closed: Arc<AtomicBool>,
    event: Arc<Mutex<Vec<u8>>>,
    emit_called: Arc<Mutex<Option<Vec<u8>>>>,
    event_thread: Arc<Mutex<Option<JoinHandle<()>>>>,
}

impl Device {
    /// Gets the given mock device.
    pub fn get(name: &str) -> Device {
        Device {
            name: name.to_string(),
            closed: Arc::new(AtomicBool::new(false)),
            barrier: Arc::new(Barrier::new(2)),
            event: Arc::new(Mutex::new(Vec::new())),
            emit_called: Arc::new(Mutex::new(None)),
            event_thread: Arc::new(Mutex::new(None)),
        }
    }

    #[cfg(test)]
    /// Sends the mock event through to the sender.
    pub fn mock_event(&self, event: &[u8]) {
        {
            let mut mutex_event = self.event.lock().expect("unable to get event lock");
            *mutex_event = event.to_vec();
        }
        // Wait until the thread goes to receive the event.
        self.barrier.wait();
        // Wait until the thread has locked the mutex.
        self.barrier.wait();
    }

    #[cfg(test)]
    // Gets the last event emitted.
    pub fn get_emitted_event(&self) -> Option<Vec<u8>> {
        let emit_called = self
            .emit_called
            .lock()
            .expect("unable to get emit called lock");
        emit_called.as_ref().map(|event| event.to_vec())
    }

    #[cfg(test)]
    // Resets the last emitted event to none.
    pub fn reset_emitted_event(&self) {
        let mut emit_called = self
            .emit_called
            .lock()
            .expect("unable to get emit called lock");
        *emit_called = None;
    }
}

impl super::Device for Device {
    /// Watches MIDI input for events and sends them to the given sender.
    fn watch_events(&self, sender: Sender<Vec<u8>>) -> Result<(), Box<dyn Error>> {
        let mut event_thread = self.event_thread.lock().expect("unable to get lock");
        if event_thread.is_some() {
            return Err("Already watching events.".into());
        }

        let barrier = self.barrier.clone();
        let event = self.event.clone();
        let closed = self.closed.clone();
        *event_thread = Some(tokio::task::spawn_blocking(move || loop {
            barrier.wait();

            {
                if closed.load(Ordering::Relaxed) {
                    return;
                }
                let event = event.lock().expect("unable to get event lock");
                sender
                    .blocking_send(event.to_vec())
                    .expect("error sending event");
            }
            barrier.wait();
        }));

        Ok(())
    }

    /// Stops watching events.
    fn stop_watch_events(&self) {
        self.closed.store(true, Ordering::Relaxed);
        // Wait for watcher thread to move to next loop iteration.
        self.barrier.wait();
    }

    /// Plays the given song through the MIDI interface, starting from a specific time.
    fn play_from(
        &self,
        song: Arc<Song>,
        cancel_handle: CancelHandle,
        play_barrier: Arc<Barrier>,
        start_time: Duration,
    ) -> Result<(), Box<dyn Error>> {
        let span = span!(Level::INFO, "play song (mock)");
        let _enter = span.enter();

        let remaining_duration = song.duration().saturating_sub(start_time);
        info!(
            device = self.name,
            song = song.name(),
            duration = song.duration_string(),
            start_time = ?start_time,
            "Playing song."
        );

        let (sleep_tx, sleep_rx) = mpsc::channel::<()>();

        let finished = Arc::new(AtomicBool::new(false));
        let join_handle = {
            let cancel_handle = cancel_handle.clone();
            let finished = finished.clone();
            // Wait until the song is cancelled or until the song is done.
            thread::spawn(move || {
                play_barrier.wait();
                // Wait for a signal or until we hit cancellation.
                let _ = sleep_rx.recv_timeout(remaining_duration);

                // Expire at the end of playback.
                finished.store(true, Ordering::Relaxed);
                cancel_handle.notify();
            })
        };

        cancel_handle.wait(finished);
        sleep_tx.send(())?;
        if join_handle.join().is_err() {
            return Err("Error while joining thread!".into());
        }

        Ok(())
    }

    /// Emits an event.
    fn emit(&self, midi_event: Option<LiveEvent<'static>>) -> Result<(), Box<dyn Error>> {
        if let Some(midi_event) = midi_event {
            let mut emit_called = self
                .emit_called
                .lock()
                .expect("unable to get emit called lock");

            let mut buf: Vec<u8> = Vec::with_capacity(8);
            midi_event.write(&mut buf)?;
            *emit_called = Some(buf);
        }

        Ok(())
    }

    #[cfg(test)]
    fn to_mock(&self) -> Result<Arc<Device>, Box<dyn Error>> {
        Ok(Arc::new(self.clone()))
    }
}

impl fmt::Display for Device {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (Mock)", self.name,)
    }
}
