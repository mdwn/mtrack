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

//! A drop-in replacement for nodi's `Ticker` that uses `quanta::Instant` instead
//! of `std::time::Instant` for all timing. On ARM64 (e.g. Raspberry Pi 5), this
//! is ~2.7x faster per clock read, which significantly reduces the CPU overhead
//! of nodi's hybrid-sleep spin-lock.

use std::time::Duration;

use midly::Timing;
use nodi::{Event, Moment, Timer};

/// A metrical MIDI timer using `quanta::Instant` for high-performance timing.
///
/// This is functionally identical to nodi's `Ticker` but replaces all
/// `std::time::Instant` usage with `quanta::Instant`.
#[derive(Debug, Copy, Clone)]
pub struct QuantaTicker {
    ticks_per_beat: u16,
    micros_per_tick: f64,
    last_instant: Option<quanta::Instant>,
    /// Speed modifier, a value of `1.0` is the default and affects nothing.
    pub speed: f32,
}

impl QuantaTicker {
    /// Creates a new `QuantaTicker` with the given ticks-per-beat.
    pub const fn new(ticks_per_beat: u16) -> Self {
        Self {
            ticks_per_beat,
            micros_per_tick: 0.0,
            last_instant: None,
            speed: 1.0,
        }
    }

    /// Calculate the duration of `n_ticks` ticks, without accounting for the
    /// last time this ticker ticked.
    fn sleep_duration_without_readjustment(&self, n_ticks: u32) -> Duration {
        let t = self.micros_per_tick * n_ticks as f64 / self.speed as f64;
        if t > 0.0 {
            Duration::from_micros(t as u64)
        } else {
            Duration::default()
        }
    }
}

impl Timer for QuantaTicker {
    fn change_tempo(&mut self, tempo: u32) {
        self.micros_per_tick = tempo as f64 / self.ticks_per_beat as f64;
    }

    fn sleep_duration(&mut self, n_ticks: u32) -> Duration {
        let mut t = self.sleep_duration_without_readjustment(n_ticks);

        match self.last_instant {
            Some(last_instant) => {
                self.last_instant = Some(last_instant + t);
                t = t.checked_sub(last_instant.elapsed()).unwrap_or(t);
            }
            None => self.last_instant = Some(quanta::Instant::now()),
        }

        t
    }

    fn sleep(&mut self, n_ticks: u32) {
        let t = self.sleep_duration(n_ticks);
        if !t.is_zero() {
            hybrid_sleep(t);
        }
    }

    fn duration(&mut self, moments: &[Moment]) -> Duration {
        let mut counter = Duration::default();

        for moment in moments {
            counter += self.sleep_duration_without_readjustment(1);

            for event in &moment.events {
                if let Event::Tempo(val) = event {
                    self.change_tempo(*val);
                }
            }
        }

        counter
    }
}

impl TryFrom<Timing> for QuantaTicker {
    type Error = nodi::timers::TimeFormatError;

    fn try_from(t: Timing) -> Result<Self, Self::Error> {
        match t {
            Timing::Metrical(n) => Ok(Self::new(u16::from(n))),
            _ => Err(nodi::timers::TimeFormatError),
        }
    }
}

/// Hybrid sleep: OS sleep for the bulk of the duration, then spin-lock the
/// last 3ms using `quanta::Instant` for the timing reads.
fn hybrid_sleep(t: Duration) {
    const LIMIT: Duration = Duration::from_millis(3);

    let t = if t < LIMIT {
        t
    } else {
        let mut last = quanta::Instant::now();
        let mut remaining = t;
        loop {
            std::thread::sleep(Duration::from_millis(1));
            let now = quanta::Instant::now();
            remaining = remaining.checked_sub(now - last).unwrap_or_default();
            if remaining <= LIMIT {
                break remaining;
            }
            last = now;
        }
    };

    // Spin-lock for the final portion.
    let now = quanta::Instant::now();
    while now.elapsed() < t {
        std::hint::spin_loop();
    }
}
