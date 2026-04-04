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

/// Helper to convert duration to frequency/cycles per second
#[inline]
fn duration_to_rate(duration_secs: f64) -> f64 {
    if duration_secs <= 0.0 {
        0.0
    } else {
        1.0 / duration_secs
    }
}

/// Helper to convert measures to duration using tempo map or fallback
fn measures_to_duration_secs(
    measures: f64,
    tempo_map: Option<&crate::lighting::tempo::TempoMap>,
    at_time: Duration,
) -> f64 {
    if let Some(tm) = tempo_map {
        let duration = tm.measures_to_duration(measures, at_time, 0.0);
        duration.as_secs_f64()
    } else {
        // Fallback: assume default BPM, 4/4 time
        let beats = measures * 4.0;
        beats * 60.0 / crate::lighting::tempo::DEFAULT_BPM
    }
}

/// Helper to convert beats to duration using tempo map or fallback
fn beats_to_duration_secs(
    beats: f64,
    tempo_map: Option<&crate::lighting::tempo::TempoMap>,
    at_time: Duration,
) -> f64 {
    if let Some(tm) = tempo_map {
        let duration = tm.beats_to_duration(beats, at_time, 0.0);
        duration.as_secs_f64()
    } else {
        // Fallback: assume default BPM
        beats * 60.0 / crate::lighting::tempo::DEFAULT_BPM
    }
}

/// Tempo-aware value specification that can adapt to tempo changes.
///
/// Used for both speed (cycles per second) and frequency (Hz) parameters,
/// since both resolve identically: the value either is a fixed rate or is
/// derived from a musical duration (measures, beats, seconds) via 1/period.
#[derive(Debug, Clone, PartialEq)]
pub enum TempoAwareValue {
    /// Fixed rate (not tempo-aware)
    Fixed(f64),
    /// Rate specified in measures (tempo-aware)
    Measures(f64),
    /// Rate specified in beats (tempo-aware)
    Beats(f64),
    /// Rate specified in seconds (fixed, not tempo-aware)
    Seconds(f64),
}

impl TempoAwareValue {
    /// Get the current rate (cycles per second / Hz), using tempo map if available
    pub fn to_rate(
        &self,
        tempo_map: Option<&crate::lighting::tempo::TempoMap>,
        at_time: Duration,
    ) -> f64 {
        match self {
            TempoAwareValue::Fixed(rate) => *rate,
            TempoAwareValue::Seconds(duration) => duration_to_rate(*duration),
            TempoAwareValue::Measures(measures) => {
                if *measures <= 0.0 {
                    return 0.0;
                }
                let duration_secs = measures_to_duration_secs(*measures, tempo_map, at_time);
                duration_to_rate(duration_secs)
            }
            TempoAwareValue::Beats(beats) => {
                if *beats <= 0.0 {
                    return 0.0;
                }
                let duration_secs = beats_to_duration_secs(*beats, tempo_map, at_time);
                duration_to_rate(duration_secs)
            }
        }
    }

    /// Alias for `to_rate` — reads naturally when the value represents speed (cycles per second).
    #[inline]
    pub fn to_cycles_per_second(
        &self,
        tempo_map: Option<&crate::lighting::tempo::TempoMap>,
        at_time: Duration,
    ) -> f64 {
        self.to_rate(tempo_map, at_time)
    }

    /// Alias for `to_rate` — reads naturally when the value represents frequency (Hz).
    #[inline]
    pub fn to_hz(
        &self,
        tempo_map: Option<&crate::lighting::tempo::TempoMap>,
        at_time: Duration,
    ) -> f64 {
        self.to_rate(tempo_map, at_time)
    }
}

/// Type alias for tempo-aware speed parameters (cycles per second).
pub type TempoAwareSpeed = TempoAwareValue;

/// Type alias for tempo-aware frequency parameters (Hz).
pub type TempoAwareFrequency = TempoAwareValue;

#[cfg(test)]
mod tests {
    use super::*;

    // ── duration_to_rate ───────────────────────────────────────────

    #[test]
    fn duration_to_rate_positive() {
        assert!((duration_to_rate(2.0) - 0.5).abs() < 1e-9);
    }

    #[test]
    fn duration_to_rate_one_second() {
        assert!((duration_to_rate(1.0) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn duration_to_rate_zero() {
        assert_eq!(duration_to_rate(0.0), 0.0);
    }

    #[test]
    fn duration_to_rate_negative() {
        assert_eq!(duration_to_rate(-1.0), 0.0);
    }

    // ── TempoAwareValue — Fixed ────────────────────────────────────

    #[test]
    fn value_fixed() {
        let val = TempoAwareValue::Fixed(2.5);
        assert!((val.to_rate(None, Duration::ZERO) - 2.5).abs() < 1e-9);
    }

    // ── TempoAwareValue — Seconds ──────────────────────────────────

    #[test]
    fn value_seconds() {
        let val = TempoAwareValue::Seconds(4.0);
        assert!((val.to_rate(None, Duration::ZERO) - 0.25).abs() < 1e-9);
    }

    #[test]
    fn value_seconds_zero() {
        let val = TempoAwareValue::Seconds(0.0);
        assert_eq!(val.to_rate(None, Duration::ZERO), 0.0);
    }

    // ── TempoAwareValue — Beats (no tempo map -> fallback 120 BPM) ─

    #[test]
    fn value_beats_no_tempo_map() {
        // 1 beat at 120 BPM = 0.5 seconds -> rate = 2.0
        let val = TempoAwareValue::Beats(1.0);
        let rate = val.to_rate(None, Duration::ZERO);
        assert!((rate - 2.0).abs() < 1e-9);
    }

    #[test]
    fn value_beats_two_beats_no_tempo_map() {
        // 2 beats at 120 BPM = 1.0 second -> rate = 1.0
        let val = TempoAwareValue::Beats(2.0);
        let rate = val.to_rate(None, Duration::ZERO);
        assert!((rate - 1.0).abs() < 1e-9);
    }

    #[test]
    fn value_beats_zero() {
        let val = TempoAwareValue::Beats(0.0);
        assert_eq!(val.to_rate(None, Duration::ZERO), 0.0);
    }

    #[test]
    fn value_beats_negative() {
        let val = TempoAwareValue::Beats(-1.0);
        assert_eq!(val.to_rate(None, Duration::ZERO), 0.0);
    }

    // ── TempoAwareValue — Measures (no tempo map -> fallback) ──────

    #[test]
    fn value_measures_no_tempo_map() {
        // 1 measure = 4 beats at 120 BPM = 2.0 seconds -> rate = 0.5
        let val = TempoAwareValue::Measures(1.0);
        let rate = val.to_rate(None, Duration::ZERO);
        assert!((rate - 0.5).abs() < 1e-9);
    }

    #[test]
    fn value_measures_zero() {
        let val = TempoAwareValue::Measures(0.0);
        assert_eq!(val.to_rate(None, Duration::ZERO), 0.0);
    }

    #[test]
    fn value_measures_negative() {
        let val = TempoAwareValue::Measures(-1.0);
        assert_eq!(val.to_rate(None, Duration::ZERO), 0.0);
    }

    // ── Aliases work identically ───────────────────────────────────

    #[test]
    fn speed_alias_works() {
        let speed = TempoAwareSpeed::Fixed(2.5);
        assert!((speed.to_cycles_per_second(None, Duration::ZERO) - 2.5).abs() < 1e-9);
    }

    #[test]
    fn frequency_alias_works() {
        let freq = TempoAwareFrequency::Fixed(10.0);
        assert!((freq.to_hz(None, Duration::ZERO) - 10.0).abs() < 1e-9);
    }

    // ── TempoAwareValue with TempoMap ──────────────────────────────

    #[test]
    fn value_beats_with_tempo_map() {
        use crate::lighting::tempo::{TempoMap, TimeSignature};
        let tm = TempoMap::new(Duration::ZERO, 90.0, TimeSignature::new(4, 4), vec![]);
        // 1 beat at 90 BPM = 60/90 = 0.667s -> rate ~ 1.5
        let val = TempoAwareValue::Beats(1.0);
        let rate = val.to_rate(Some(&tm), Duration::ZERO);
        assert!((rate - 1.5).abs() < 0.01);
    }

    #[test]
    fn value_measures_with_tempo_map() {
        use crate::lighting::tempo::{TempoMap, TimeSignature};
        let tm = TempoMap::new(Duration::ZERO, 60.0, TimeSignature::new(4, 4), vec![]);
        // 1 measure = 4 beats at 60 BPM = 4.0s -> rate = 0.25
        let val = TempoAwareValue::Measures(1.0);
        let rate = val.to_rate(Some(&tm), Duration::ZERO);
        assert!((rate - 0.25).abs() < 0.01);
    }

    #[test]
    fn value_beats_with_tempo_map_via_hz() {
        use crate::lighting::tempo::{TempoMap, TimeSignature};
        let tm = TempoMap::new(Duration::ZERO, 60.0, TimeSignature::new(4, 4), vec![]);
        // 1 beat at 60 BPM = 1.0s -> 1.0 Hz
        let freq = TempoAwareFrequency::Beats(1.0);
        let hz = freq.to_hz(Some(&tm), Duration::ZERO);
        assert!((hz - 1.0).abs() < 0.01);
    }
}
