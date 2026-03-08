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
use std::time::Duration;

use super::playback::TempoEntry;

/// Number of MIDI clock pulses per quarter note.
const CLOCKS_PER_BEAT: u64 = 24;

/// Pre-computed MIDI beat clock: a list of absolute timestamps for each 0xF8 tick.
pub(crate) struct PrecomputedBeatClock {
    /// Absolute timestamps for each clock tick.
    ticks: Vec<Duration>,
}

impl PrecomputedBeatClock {
    /// Builds a pre-computed beat clock from a tempo map.
    ///
    /// Walks through the tempo map, computing the absolute time of every clock tick
    /// (24 per quarter note). Between tempo changes, ticks are evenly spaced at the
    /// current tempo's interval.
    pub(crate) fn from_tempo_info(
        tempo_map: &[TempoEntry],
        ticks_per_beat: u16,
        total_ticks: u64,
    ) -> Self {
        let tpb = ticks_per_beat as f64;
        // Default tempo: 120 BPM = 500_000 microseconds per beat
        let default_micros_per_tick = 500_000.0 / tpb;

        // Use fractional tracking in case ticks_per_beat isn't evenly divisible by 24
        let clock_interval_ticks_f = tpb / CLOCKS_PER_BEAT as f64;

        let mut ticks = Vec::new();
        let mut micros_per_tick = default_micros_per_tick;
        let mut elapsed_micros: f64 = 0.0;
        let mut current_tick: f64 = 0.0;
        let mut tempo_idx: usize = 0;

        // Walk through the entire MIDI timeline, generating clock ticks
        while (current_tick as u64) < total_ticks {
            let next_clock_tick = current_tick + clock_interval_ticks_f;

            // Check if any tempo changes occur before the next clock tick
            while tempo_idx < tempo_map.len()
                && (tempo_map[tempo_idx].tick as f64) <= next_clock_tick
            {
                let entry = &tempo_map[tempo_idx];
                if (entry.tick as f64) > current_tick {
                    // Accumulate time up to the tempo change
                    elapsed_micros += (entry.tick as f64 - current_tick) * micros_per_tick;
                    current_tick = entry.tick as f64;
                }
                micros_per_tick = entry.micros_per_tick;
                tempo_idx += 1;
            }

            // Accumulate time from current position to the next clock tick
            if next_clock_tick > current_tick {
                elapsed_micros += (next_clock_tick - current_tick) * micros_per_tick;
            }
            current_tick = next_clock_tick;

            if (current_tick as u64) <= total_ticks {
                ticks.push(Duration::from_micros(elapsed_micros.round() as u64));
            }
        }

        PrecomputedBeatClock { ticks }
    }

    /// Returns the slice of ticks starting from the first tick at or after `start_time`.
    pub(crate) fn ticks_from(&self, start_time: Duration) -> &[Duration] {
        let idx = self.ticks.partition_point(|t| *t < start_time);
        &self.ticks[idx..]
    }

    /// Returns all ticks.
    #[cfg(test)]
    pub(crate) fn ticks(&self) -> &[Duration] {
        &self.ticks
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_tempo_tick_count() {
        // 480 ticks per beat, 120 BPM (default), 4 beats = 1920 total ticks
        // Should produce 4 * 24 = 96 clock ticks
        let beat_clock = PrecomputedBeatClock::from_tempo_info(&[], 480, 1920);
        assert_eq!(beat_clock.ticks().len(), 96);
    }

    #[test]
    fn constant_tempo_tick_spacing() {
        // 480 tpb, default 120 BPM = 500_000 µs/beat
        // Clock interval = 500_000 / 24 ≈ 20_833 µs
        let beat_clock = PrecomputedBeatClock::from_tempo_info(&[], 480, 480);
        assert_eq!(beat_clock.ticks().len(), 24);

        // First tick should be at ~20_833 µs (one clock interval)
        let expected_interval = Duration::from_micros((500_000.0_f64 / 24.0).round() as u64);
        assert_eq!(beat_clock.ticks()[0], expected_interval);

        // Last tick should be at ~500_000 µs (one beat)
        let last = beat_clock.ticks()[23];
        assert!(
            last >= Duration::from_micros(499_000) && last <= Duration::from_micros(501_000),
            "last tick at {:?}",
            last
        );
    }

    #[test]
    fn tempo_change_adjusts_spacing() {
        // Start at 120 BPM (500_000 µs/beat), change to 60 BPM (1_000_000 µs/beat) at beat 1
        let tpb = 480u16;
        let tempo_map = vec![TempoEntry {
            tick: 480,
            micros_per_tick: 1_000_000.0 / 480.0,
        }];
        // 2 beats total = 960 ticks
        let beat_clock = PrecomputedBeatClock::from_tempo_info(&tempo_map, tpb, 960);
        // 2 beats * 24 = 48 ticks
        assert_eq!(beat_clock.ticks().len(), 48);

        // First beat ticks should be spaced at ~20_833 µs (120 BPM)
        let first_beat_interval =
            beat_clock.ticks()[1].as_micros() - beat_clock.ticks()[0].as_micros();
        assert!(
            (first_beat_interval as i64 - 20_833).unsigned_abs() < 10,
            "first beat interval: {}",
            first_beat_interval
        );

        // Second beat ticks should be spaced at ~41_667 µs (60 BPM)
        let second_beat_interval =
            beat_clock.ticks()[25].as_micros() - beat_clock.ticks()[24].as_micros();
        assert!(
            (second_beat_interval as i64 - 41_667).unsigned_abs() < 10,
            "second beat interval: {}",
            second_beat_interval
        );
    }

    #[test]
    fn ticks_from_seeks_correctly() {
        let beat_clock = PrecomputedBeatClock::from_tempo_info(&[], 480, 960);
        // 48 ticks total (2 beats)
        assert_eq!(beat_clock.ticks().len(), 48);

        // Seek to the start
        assert_eq!(beat_clock.ticks_from(Duration::ZERO).len(), 48);

        // Seek past one beat (500_001 µs) — should skip the first 24 ticks
        // (the 24th tick lands at exactly 500_000 µs)
        let from_half = beat_clock.ticks_from(Duration::from_micros(500_001));
        assert_eq!(from_half.len(), 24);
    }

    #[test]
    fn empty_song_produces_no_ticks() {
        let beat_clock = PrecomputedBeatClock::from_tempo_info(&[], 480, 0);
        assert_eq!(beat_clock.ticks().len(), 0);
    }

    #[test]
    fn multiple_tempo_changes() {
        // Three tempo zones: 120 BPM, 60 BPM, 240 BPM
        let tpb = 480u16;
        let tempo_map = vec![
            TempoEntry {
                tick: 480,
                micros_per_tick: 1_000_000.0 / 480.0, // 60 BPM
            },
            TempoEntry {
                tick: 960,
                micros_per_tick: 250_000.0 / 480.0, // 240 BPM
            },
        ];
        // 3 beats total = 1440 ticks
        let beat_clock = PrecomputedBeatClock::from_tempo_info(&tempo_map, tpb, 1440);
        // 3 beats * 24 = 72 ticks
        assert_eq!(beat_clock.ticks().len(), 72);
    }

    #[test]
    fn tempo_change_at_tick_zero() {
        // Tempo change at tick 0 sets the initial tempo (exercises entry.tick == current_tick path)
        let tpb = 480u16;
        let tempo_map = vec![TempoEntry {
            tick: 0,
            micros_per_tick: 1_000_000.0 / 480.0, // 60 BPM
        }];
        // 1 beat = 480 ticks at 60 BPM = 1_000_000 µs
        let beat_clock = PrecomputedBeatClock::from_tempo_info(&tempo_map, tpb, 480);
        assert_eq!(beat_clock.ticks().len(), 24);

        // Ticks should be spaced at ~41_667 µs (60 BPM)
        let interval = beat_clock.ticks()[1].as_micros() - beat_clock.ticks()[0].as_micros();
        assert!(
            (interval as i64 - 41_667).unsigned_abs() < 10,
            "interval: {}",
            interval
        );
    }

    #[test]
    fn non_standard_ticks_per_beat() {
        // 96 tpb (common in older MIDI files), not evenly divisible by 24
        // 96 / 24 = 4, so actually it is evenly divisible. Use 100 instead.
        let tpb = 100u16;
        let tempo_map = vec![TempoEntry {
            tick: 0,
            micros_per_tick: 500_000.0 / 100.0, // 120 BPM
        }];
        // 1 beat = 100 ticks
        let beat_clock = PrecomputedBeatClock::from_tempo_info(&tempo_map, tpb, 100);
        assert_eq!(beat_clock.ticks().len(), 24);
    }

    #[test]
    fn ticks_from_past_end_returns_empty() {
        let beat_clock = PrecomputedBeatClock::from_tempo_info(&[], 480, 480);
        let from_far = beat_clock.ticks_from(Duration::from_secs(100));
        assert_eq!(from_far.len(), 0);
    }
}
