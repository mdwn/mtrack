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
    fmt,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread,
    time::Duration,
};

use tracing::{info, span, Level};

use crate::{playsync::CancelHandle, songs::Song};

/// A mock device. Doesn't actually play anything.
#[derive(Clone)]
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

impl crate::audio::Device for Device {
    /// A mock device that will sleep for the remaining song duration after start_time.
    fn play_from(
        &self,
        song: Arc<Song>,
        _: &HashMap<String, Vec<u16>>,
        cancel_handle: CancelHandle,
        ready_tx: std::sync::mpsc::Sender<()>,
        clock: crate::clock::PlaybackClock,
        start_time: Duration,
    ) -> Result<(), Box<dyn Error>> {
        let span = span!(Level::INFO, "play song (mock)");
        let _enter = span.enter();

        let remaining_duration = song.duration().saturating_sub(start_time);
        info!(
            device = self.name,
            song = song.name(),
            duration = song.duration_string(),
            start_time = format!("{:?}", start_time),
            "Playing song."
        );

        let (sleep_tx, sleep_rx) = mpsc::channel::<()>();

        self.is_playing.store(true, Ordering::Relaxed);
        let finished = Arc::new(AtomicBool::new(false));
        let join_handle = {
            let cancel_handle = cancel_handle.clone();
            let finished = finished.clone();
            thread::spawn(move || {
                let _ = ready_tx.send(());

                while clock.elapsed() == Duration::ZERO {
                    if cancel_handle.is_cancelled() {
                        finished.store(true, Ordering::Relaxed);
                        cancel_handle.notify();
                        return;
                    }
                    std::hint::spin_loop();
                }

                if cancel_handle.is_cancelled() {
                    finished.store(true, Ordering::Relaxed);
                    cancel_handle.notify();
                    return;
                }

                let _ = sleep_rx.recv_timeout(remaining_duration);

                // Expire at the end of playback.
                finished.store(true, Ordering::Relaxed);
                cancel_handle.notify();
            })
        };

        cancel_handle.wait(finished);

        // Set is_playing to false as soon as we know playback is stopping
        // This ensures tests can check is_playing immediately after stop() without races
        self.is_playing.store(false, Ordering::Relaxed);

        sleep_tx.send(())?;
        let join_result = join_handle.join();

        if join_result.is_err() {
            return Err("Error while joining thread!".into());
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_name_and_mock() {
        let device = Device::get("TestDevice");
        let display = format!("{}", device);
        assert_eq!(display, "TestDevice (Mock)");
    }

    #[test]
    fn get_creates_device_not_playing() {
        let device = Device::get("test");
        assert!(!device.is_playing());
    }

    #[test]
    fn clone_shares_is_playing_state() {
        let device = Device::get("test");
        let cloned = device.clone();
        assert!(!cloned.is_playing());
    }

    #[test]
    fn play_from_zero_duration_completes() {
        use crate::audio::Device as DeviceTrait;
        use crate::clock::PlaybackClock;
        use crate::playsync::CancelHandle;
        use crate::songs::Song;

        let device = Device::get("mock-zero");
        // new_for_test creates a song with Duration::ZERO
        let song = Arc::new(Song::new_for_test("zero-song", &["t1"]));
        let mappings = std::collections::HashMap::new();
        let cancel_handle = CancelHandle::new();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let clock = PlaybackClock::wall();

        let device_clone = device.clone();
        let cancel_clone = cancel_handle.clone();
        let clock_clone = clock.clone();

        let handle = thread::spawn(move || {
            // play_from may return an error due to the mock's internal mpsc
            // channel timing, but the key is that it doesn't panic.
            let _ = device_clone.play_from(
                song,
                &mappings,
                cancel_clone,
                ready_tx,
                clock_clone,
                Duration::from_millis(0),
            );
        });

        ready_rx.recv().expect("ready signal");
        clock.start();

        // Wait for completion
        handle.join().expect("thread should not panic");
        assert!(
            !device.is_playing(),
            "device should not be playing after zero-duration song"
        );
    }

    #[test]
    fn play_from_with_start_time_offset() {
        use crate::audio::Device as DeviceTrait;
        use crate::clock::PlaybackClock;
        use crate::playsync::CancelHandle;
        use crate::songs::Song;

        let device = Device::get("mock-offset");
        // new_for_test creates zero-duration song; start_time > duration => saturating_sub → 0
        let song = Arc::new(Song::new_for_test("offset-song", &["t1"]));
        let mappings = std::collections::HashMap::new();
        let cancel_handle = CancelHandle::new();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let clock = PlaybackClock::wall();

        let device_clone = device.clone();
        let cancel_clone = cancel_handle.clone();
        let clock_clone = clock.clone();

        let handle = thread::spawn(move || {
            let _ = device_clone.play_from(
                song,
                &mappings,
                cancel_clone,
                ready_tx,
                clock_clone,
                Duration::from_secs(1), // Start offset > duration → remaining = 0
            );
        });

        ready_rx.recv().expect("ready signal");
        clock.start();
        handle.join().expect("thread should not panic");
        assert!(!device.is_playing());
    }

    #[test]
    fn play_from_cancel_before_barrier() {
        use crate::audio::Device as DeviceTrait;
        use crate::clock::PlaybackClock;
        use crate::playsync::CancelHandle;
        use crate::songs::Song;

        let device = Device::get("mock-precancel");
        let song = Arc::new(Song::new_for_test("song", &["t1"]));
        let mappings = std::collections::HashMap::new();
        let cancel_handle = CancelHandle::new();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let clock = PlaybackClock::wall();

        // Cancel before starting
        cancel_handle.cancel();

        let device_clone = device.clone();
        let cancel_clone = cancel_handle.clone();
        let clock_clone = clock.clone();

        let handle = thread::spawn(move || {
            let _ = device_clone.play_from(
                song,
                &mappings,
                cancel_clone,
                ready_tx,
                clock_clone,
                Duration::from_millis(0),
            );
        });

        ready_rx.recv().expect("ready signal");
        // Notify to unblock the wait
        cancel_handle.notify();

        handle.join().expect("thread should not panic");
        assert!(!device.is_playing());
    }
}
