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
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use crate::playsync::CancelHandle;

/// A playback clock that provides elapsed time since playback started.
///
/// When an audio device is present, the clock derives time from the audio
/// interface's hardware sample counter, ensuring MIDI and DMX stay synchronized
/// with audio. When no audio device is present, the clock falls back to
/// `Instant::now()` (system monotonic clock).
///
/// The clock must be `start()`ed (typically right after the playback barrier
/// releases) before `elapsed()` returns meaningful values.
#[derive(Clone)]
pub struct PlaybackClock {
    inner: Arc<ClockShared>,
}

struct ClockShared {
    source: ClockSource,
    /// Condvar+Mutex used to wake threads waiting for `start()`.
    start_condvar: Condvar,
    start_mutex: Mutex<bool>,
}

enum ClockSource {
    /// Derives time from the audio interface's sample counter.
    Audio {
        sample_counter: Arc<AtomicU64>,
        sample_rate: u32,
        /// Sentinel value u64::MAX means "not started yet".
        start_sample: AtomicU64,
    },
    /// Falls back to system monotonic clock.
    Wall {
        /// None means "not started yet".
        start_instant: parking_lot::Mutex<Option<Instant>>,
    },
}

impl PlaybackClock {
    /// Creates a clock backed by an audio mixer's sample counter.
    pub fn from_sample_counter(sample_counter: Arc<AtomicU64>, sample_rate: u32) -> Self {
        debug_assert!(sample_rate > 0, "sample_rate must be > 0");
        PlaybackClock {
            inner: Arc::new(ClockShared {
                source: ClockSource::Audio {
                    sample_counter,
                    sample_rate,
                    start_sample: AtomicU64::new(u64::MAX),
                },
                start_condvar: Condvar::new(),
                start_mutex: Mutex::new(false),
            }),
        }
    }

    /// Creates a clock backed by `Instant::now()`.
    pub fn wall() -> Self {
        PlaybackClock {
            inner: Arc::new(ClockShared {
                source: ClockSource::Wall {
                    start_instant: parking_lot::Mutex::new(None),
                },
                start_condvar: Condvar::new(),
                start_mutex: Mutex::new(false),
            }),
        }
    }

    /// Marks the clock's start point, resetting the epoch for a new song.
    /// Called by `play_files` once all subsystems have signaled readiness.
    /// Subsystems wait for `elapsed() > Duration::ZERO` as the "go" signal.
    pub fn start(&self) {
        match &self.inner.source {
            ClockSource::Audio {
                sample_counter,
                start_sample,
                ..
            } => {
                let current = sample_counter.load(Ordering::Relaxed);
                start_sample.store(current, Ordering::Relaxed);
            }
            ClockSource::Wall { start_instant } => {
                let mut guard = start_instant.lock();
                *guard = Some(Instant::now());
            }
        }
        // Wake any threads blocked in wait_for_start_or_cancel.
        {
            let mut started = self.inner.start_mutex.lock().unwrap();
            *started = true;
        }
        self.inner.start_condvar.notify_all();
    }

    /// Blocks until the clock has been started (`elapsed() > ZERO`) or the
    /// cancel handle is cancelled. Uses a condvar instead of spinning.
    pub fn wait_for_start_or_cancel(&self, cancel: &CancelHandle) {
        let mut started = self.inner.start_mutex.lock().unwrap();
        while !*started {
            if cancel.is_cancelled() {
                return;
            }
            // Use a short timeout so we can re-check the cancel handle.
            let (guard, _) = self
                .inner
                .start_condvar
                .wait_timeout(started, Duration::from_millis(10))
                .unwrap();
            started = guard;
        }
    }

    /// Returns the elapsed time since `start()` was called.
    /// Returns `Duration::ZERO` if `start()` has not been called yet.
    pub fn elapsed(&self) -> Duration {
        match &self.inner.source {
            ClockSource::Audio {
                sample_counter,
                sample_rate,
                start_sample,
            } => {
                let start = start_sample.load(Ordering::Relaxed);
                if start == u64::MAX {
                    return Duration::ZERO;
                }
                let current = sample_counter.load(Ordering::Relaxed);
                let delta = current.saturating_sub(start);
                Duration::from_secs_f64(delta as f64 / *sample_rate as f64)
            }
            ClockSource::Wall { start_instant } => {
                let guard = start_instant.lock();
                match *guard {
                    Some(instant) => instant.elapsed(),
                    None => Duration::ZERO,
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_clock_zero_before_start() {
        let clock = PlaybackClock::wall();
        assert_eq!(clock.elapsed(), Duration::ZERO);
    }

    #[test]
    fn wall_clock_positive_after_start() {
        let clock = PlaybackClock::wall();
        clock.start();
        // Sleep briefly to ensure elapsed > 0
        std::thread::sleep(Duration::from_millis(1));
        assert!(clock.elapsed() > Duration::ZERO);
    }

    #[test]
    fn audio_clock_zero_before_start() {
        let counter = Arc::new(AtomicU64::new(1000));
        let clock = PlaybackClock::from_sample_counter(counter, 48000);
        assert_eq!(clock.elapsed(), Duration::ZERO);
    }

    #[test]
    fn audio_clock_correct_after_start() {
        let counter = Arc::new(AtomicU64::new(0));
        let clock = PlaybackClock::from_sample_counter(counter.clone(), 48000);
        clock.start();
        // Advance by 48000 samples = 1 second
        counter.store(48000, Ordering::Relaxed);
        let elapsed = clock.elapsed();
        assert!(
            (elapsed.as_secs_f64() - 1.0).abs() < 0.001,
            "elapsed: {:?}",
            elapsed
        );
    }

    #[test]
    fn audio_clock_nonzero_start() {
        let counter = Arc::new(AtomicU64::new(10000));
        let clock = PlaybackClock::from_sample_counter(counter.clone(), 48000);
        clock.start();
        // Advance by 24000 samples from start = 0.5 seconds
        counter.store(34000, Ordering::Relaxed);
        let elapsed = clock.elapsed();
        assert!(
            (elapsed.as_secs_f64() - 0.5).abs() < 0.001,
            "elapsed: {:?}",
            elapsed
        );
    }

    #[test]
    fn start_resets_epoch() {
        let counter = Arc::new(AtomicU64::new(0));
        let clock = PlaybackClock::from_sample_counter(counter.clone(), 48000);
        clock.start();
        counter.store(1000, Ordering::Relaxed);
        clock.start(); // resets epoch to current sample (1000)
        counter.store(48000, Ordering::Relaxed);
        let elapsed = clock.elapsed();
        // Should be (48000-1000)/48000 ≈ 0.979s
        assert!(
            (elapsed.as_secs_f64() - 47000.0 / 48000.0).abs() < 0.001,
            "elapsed: {:?}",
            elapsed
        );
    }

    #[test]
    fn clone_shares_state() {
        let counter = Arc::new(AtomicU64::new(0));
        let clock1 = PlaybackClock::from_sample_counter(counter.clone(), 48000);
        let clock2 = clock1.clone();
        clock1.start();
        counter.store(24000, Ordering::Relaxed);
        // Both clones should see the same elapsed time
        assert!(
            (clock2.elapsed().as_secs_f64() - 0.5).abs() < 0.001,
            "clock2 elapsed: {:?}",
            clock2.elapsed()
        );
    }

    #[test]
    fn wall_start_resets_epoch() {
        let clock = PlaybackClock::wall();
        clock.start();
        std::thread::sleep(Duration::from_millis(10));
        let elapsed1 = clock.elapsed();
        clock.start(); // resets epoch
        let elapsed2 = clock.elapsed();
        // elapsed2 should be near zero since we just reset
        assert!(elapsed2 < elapsed1);
    }

    #[test]
    fn wall_clone_shares_state() {
        let clock1 = PlaybackClock::wall();
        let clock2 = clock1.clone();
        clock1.start();
        std::thread::sleep(Duration::from_millis(1));
        assert!(clock2.elapsed() > Duration::ZERO);
    }

    #[test]
    fn fresh_audio_clock_is_zero() {
        // Creating a new clock (as play_files does per song) starts at zero.
        let counter = Arc::new(AtomicU64::new(48000));
        let clock = PlaybackClock::from_sample_counter(counter.clone(), 48000);
        assert_eq!(clock.elapsed(), Duration::ZERO);
        clock.start();
        counter.store(96000, Ordering::Relaxed);
        assert!(clock.elapsed() > Duration::ZERO);

        // A fresh clock on the same counter starts at zero again.
        let clock2 = PlaybackClock::from_sample_counter(counter.clone(), 48000);
        assert_eq!(clock2.elapsed(), Duration::ZERO);
    }

    #[test]
    fn fresh_wall_clock_is_zero() {
        let clock = PlaybackClock::wall();
        clock.start();
        std::thread::sleep(Duration::from_millis(1));
        assert!(clock.elapsed() > Duration::ZERO);

        // A fresh wall clock starts at zero.
        let clock2 = PlaybackClock::wall();
        assert_eq!(clock2.elapsed(), Duration::ZERO);
    }
}
