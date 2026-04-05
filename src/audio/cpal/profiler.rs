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
use std::time::Instant;

use tracing::debug;

pub(super) struct CallbackProfiler {
    enabled: bool,
    last_log: Instant,
    count: u64,
    sum_mix_us: u128,
    max_mix_us: u64,
    sum_convert_us: u128,
    max_convert_us: u64,
    last_cb: Option<Instant>,
    sum_gap_us: u128,
    gap_count: u64,
    max_gap_us: u64,
}

impl CallbackProfiler {
    pub(super) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            last_log: Instant::now(),
            count: 0,
            sum_mix_us: 0,
            max_mix_us: 0,
            sum_convert_us: 0,
            max_convert_us: 0,
            last_cb: None,
            sum_gap_us: 0,
            gap_count: 0,
            max_gap_us: 0,
        }
    }

    pub(super) fn on_cb_start(&mut self) -> Option<Instant> {
        if !self.enabled {
            return None;
        }
        let now = Instant::now();
        if let Some(last) = self.last_cb {
            let gap_us = now.duration_since(last).as_micros() as u64;
            self.sum_gap_us += gap_us as u128;
            self.gap_count += 1;
            if gap_us > self.max_gap_us {
                self.max_gap_us = gap_us;
            }
        }
        self.last_cb = Some(now);
        Some(now)
    }

    pub(super) fn on_mix_done(&mut self, start: Option<Instant>) {
        if !self.enabled {
            return;
        }
        let start = match start {
            Some(s) => s,
            None => return,
        };
        let mix_us = start.elapsed().as_micros() as u64;
        self.count += 1;
        self.sum_mix_us += mix_us as u128;
        if mix_us > self.max_mix_us {
            self.max_mix_us = mix_us;
        }
    }

    pub(super) fn on_convert_done(&mut self, start: Option<Instant>) {
        if !self.enabled {
            return;
        }
        let start = match start {
            Some(s) => s,
            None => return,
        };
        let convert_us = start.elapsed().as_micros() as u64;
        self.sum_convert_us += convert_us as u128;
        if convert_us > self.max_convert_us {
            self.max_convert_us = convert_us;
        }
    }

    pub(super) fn maybe_log_float(&mut self) {
        if !self.should_log() {
            return;
        }
        let mix_avg_us = self.avg(self.sum_mix_us, self.count);
        let cb_avg_gap_us = self.avg(self.sum_gap_us, self.gap_count);
        debug!(
            mix_avg_us,
            mix_max_us = self.max_mix_us,
            cb_avg_gap_us,
            cb_max_gap_us = self.max_gap_us,
            callbacks = self.count,
            "audio profile: mix (float)"
        );
        self.reset();
    }

    pub(super) fn maybe_log_int(&mut self) {
        if !self.should_log() {
            return;
        }
        let mix_avg_us = self.avg(self.sum_mix_us, self.count);
        let convert_avg_us = self.avg(self.sum_convert_us, self.count);
        let cb_avg_gap_us = self.avg(self.sum_gap_us, self.gap_count);
        debug!(
            mix_avg_us,
            mix_max_us = self.max_mix_us,
            convert_avg_us,
            convert_max_us = self.max_convert_us,
            cb_avg_gap_us,
            cb_max_gap_us = self.max_gap_us,
            callbacks = self.count,
            "audio profile: mix/convert (int)"
        );
        self.reset();
    }

    fn should_log(&self) -> bool {
        self.enabled && self.last_log.elapsed().as_secs_f32() >= 1.0
    }

    fn avg(&self, sum: u128, count: u64) -> u64 {
        if count > 0 {
            (sum / count as u128) as u64
        } else {
            0
        }
    }

    fn reset(&mut self) {
        self.last_log = Instant::now();
        self.count = 0;
        self.sum_mix_us = 0;
        self.max_mix_us = 0;
        self.sum_convert_us = 0;
        self.max_convert_us = 0;
        self.sum_gap_us = 0;
        self.gap_count = 0;
        self.max_gap_us = 0;
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::time::Duration;

    #[test]
    fn disabled_profiler_returns_none_on_cb_start() {
        let mut profiler = CallbackProfiler::new(false);
        assert!(profiler.on_cb_start().is_none());
    }

    #[test]
    fn enabled_profiler_returns_some_on_cb_start() {
        let mut profiler = CallbackProfiler::new(true);
        assert!(profiler.on_cb_start().is_some());
    }

    #[test]
    fn on_mix_done_noop_when_disabled() {
        let mut profiler = CallbackProfiler::new(false);
        profiler.on_mix_done(Some(Instant::now()));
        assert_eq!(profiler.count, 0);
        assert_eq!(profiler.sum_mix_us, 0);
    }

    #[test]
    fn on_mix_done_noop_when_start_is_none() {
        let mut profiler = CallbackProfiler::new(true);
        profiler.on_mix_done(None);
        assert_eq!(profiler.count, 0);
    }

    #[test]
    fn on_mix_done_tracks_stats() {
        let mut profiler = CallbackProfiler::new(true);
        let start = Instant::now();
        std::thread::sleep(Duration::from_micros(100));
        profiler.on_mix_done(Some(start));
        assert_eq!(profiler.count, 1);
        assert!(profiler.sum_mix_us > 0);
        assert!(profiler.max_mix_us > 0);
    }

    #[test]
    fn on_convert_done_noop_when_disabled() {
        let mut profiler = CallbackProfiler::new(false);
        profiler.on_convert_done(Some(Instant::now()));
        assert_eq!(profiler.sum_convert_us, 0);
    }

    #[test]
    fn on_convert_done_noop_when_start_is_none() {
        let mut profiler = CallbackProfiler::new(true);
        profiler.on_convert_done(None);
        assert_eq!(profiler.sum_convert_us, 0);
    }

    #[test]
    fn on_convert_done_tracks_stats() {
        let mut profiler = CallbackProfiler::new(true);
        let start = Instant::now();
        std::thread::sleep(Duration::from_micros(100));
        profiler.on_convert_done(Some(start));
        assert!(profiler.sum_convert_us > 0);
        assert!(profiler.max_convert_us > 0);
    }

    #[test]
    fn cb_start_tracks_gap_between_callbacks() {
        let mut profiler = CallbackProfiler::new(true);
        profiler.on_cb_start();
        std::thread::sleep(Duration::from_micros(100));
        profiler.on_cb_start();
        assert_eq!(profiler.gap_count, 1);
        assert!(profiler.sum_gap_us > 0);
        assert!(profiler.max_gap_us > 0);
    }

    #[test]
    fn avg_returns_zero_when_count_is_zero() {
        let profiler = CallbackProfiler::new(false);
        assert_eq!(profiler.avg(1000, 0), 0);
    }

    #[test]
    fn avg_computes_correctly() {
        let profiler = CallbackProfiler::new(false);
        assert_eq!(profiler.avg(300, 3), 100);
        assert_eq!(profiler.avg(10, 3), 3); // integer division
    }

    #[test]
    fn reset_clears_all_stats() {
        let mut profiler = CallbackProfiler::new(true);
        // Accumulate some stats.
        profiler.on_cb_start();
        std::thread::sleep(Duration::from_micros(50));
        let start = profiler.on_cb_start();
        profiler.on_mix_done(start);
        profiler.on_convert_done(Some(Instant::now()));

        profiler.reset();

        assert_eq!(profiler.count, 0);
        assert_eq!(profiler.sum_mix_us, 0);
        assert_eq!(profiler.max_mix_us, 0);
        assert_eq!(profiler.sum_convert_us, 0);
        assert_eq!(profiler.max_convert_us, 0);
        assert_eq!(profiler.sum_gap_us, 0);
        assert_eq!(profiler.gap_count, 0);
        assert_eq!(profiler.max_gap_us, 0);
    }

    #[test]
    fn should_log_returns_false_when_disabled() {
        let profiler = CallbackProfiler::new(false);
        assert!(!profiler.should_log());
    }

    #[test]
    fn should_log_returns_false_when_under_one_second() {
        let profiler = CallbackProfiler::new(true);
        // Just created, well under 1 second.
        assert!(!profiler.should_log());
    }

    #[test]
    fn max_mix_us_tracks_maximum() {
        let mut profiler = CallbackProfiler::new(true);

        // First callback - short sleep.
        let start1 = Instant::now();
        std::thread::sleep(Duration::from_micros(50));
        profiler.on_mix_done(Some(start1));
        let first_max = profiler.max_mix_us;

        // Second callback - longer sleep.
        let start2 = Instant::now();
        std::thread::sleep(Duration::from_millis(1));
        profiler.on_mix_done(Some(start2));

        assert!(profiler.max_mix_us >= first_max);
        assert_eq!(profiler.count, 2);
    }

    #[test]
    fn max_convert_us_tracks_maximum() {
        let mut profiler = CallbackProfiler::new(true);

        let start1 = Instant::now();
        std::thread::sleep(Duration::from_micros(50));
        profiler.on_convert_done(Some(start1));
        let first_max = profiler.max_convert_us;

        let start2 = Instant::now();
        std::thread::sleep(Duration::from_millis(1));
        profiler.on_convert_done(Some(start2));

        assert!(profiler.max_convert_us >= first_max);
    }

    #[test]
    fn max_gap_us_tracks_maximum() {
        let mut profiler = CallbackProfiler::new(true);

        // Three callbacks with increasing gaps.
        profiler.on_cb_start();
        std::thread::sleep(Duration::from_micros(50));
        profiler.on_cb_start();
        let first_max = profiler.max_gap_us;

        std::thread::sleep(Duration::from_millis(1));
        profiler.on_cb_start();

        assert!(profiler.max_gap_us >= first_max);
        assert_eq!(profiler.gap_count, 2);
    }

    #[test]
    fn maybe_log_float_logs_and_resets_after_one_second() {
        let mut profiler = CallbackProfiler::new(true);
        // Accumulate some stats.
        let start = profiler.on_cb_start();
        profiler.on_mix_done(start);

        // Backdate last_log so should_log() returns true.
        profiler.last_log = Instant::now() - Duration::from_secs(2);

        profiler.maybe_log_float();

        // After logging, reset should have zeroed stats.
        assert_eq!(profiler.count, 0);
        assert_eq!(profiler.sum_mix_us, 0);
        assert_eq!(profiler.max_mix_us, 0);
    }

    #[test]
    fn maybe_log_float_noop_when_disabled() {
        let mut profiler = CallbackProfiler::new(false);
        // Manually set stats since on_mix_done is also a noop when disabled.
        profiler.count = 5;
        profiler.sum_mix_us = 100;
        profiler.last_log = Instant::now() - Duration::from_secs(2);

        profiler.maybe_log_float();

        // Stats should not have been reset since logging is disabled.
        assert_eq!(profiler.count, 5);
    }

    #[test]
    fn maybe_log_int_logs_and_resets_after_one_second() {
        let mut profiler = CallbackProfiler::new(true);
        let start = profiler.on_cb_start();
        profiler.on_mix_done(start);
        profiler.on_convert_done(Some(Instant::now()));

        profiler.last_log = Instant::now() - Duration::from_secs(2);

        profiler.maybe_log_int();

        assert_eq!(profiler.count, 0);
        assert_eq!(profiler.sum_mix_us, 0);
        assert_eq!(profiler.sum_convert_us, 0);
    }

    #[test]
    fn maybe_log_int_noop_when_disabled() {
        let mut profiler = CallbackProfiler::new(false);
        profiler.count = 5;
        profiler.sum_convert_us = 100;
        profiler.last_log = Instant::now() - Duration::from_secs(2);

        profiler.maybe_log_int();

        assert_eq!(profiler.count, 5);
    }
}
