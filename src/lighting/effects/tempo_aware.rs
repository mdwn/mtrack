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

/// Tempo-aware speed specification that can adapt to tempo changes
#[derive(Debug, Clone, PartialEq)]
pub enum TempoAwareSpeed {
    /// Fixed speed in cycles per second (not tempo-aware)
    Fixed(f64),
    /// Speed specified in measures (tempo-aware)
    Measures(f64),
    /// Speed specified in beats (tempo-aware)
    Beats(f64),
    /// Speed specified in seconds (fixed, not tempo-aware)
    Seconds(f64),
}

impl TempoAwareSpeed {
    /// Get the current speed in cycles per second, using tempo map if available
    pub fn to_cycles_per_second(
        &self,
        tempo_map: Option<&crate::lighting::tempo::TempoMap>,
        at_time: Duration,
    ) -> f64 {
        match self {
            TempoAwareSpeed::Fixed(speed) => *speed,
            TempoAwareSpeed::Seconds(duration) => duration_to_rate(*duration),
            TempoAwareSpeed::Measures(measures) => {
                if *measures <= 0.0 {
                    return 0.0; // Zero/negative measures means stopped
                }
                let duration_secs = measures_to_duration_secs(*measures, tempo_map, at_time);
                duration_to_rate(duration_secs)
            }
            TempoAwareSpeed::Beats(beats) => {
                if *beats <= 0.0 {
                    return 0.0; // Zero/negative beats means stopped
                }
                let duration_secs = beats_to_duration_secs(*beats, tempo_map, at_time);
                duration_to_rate(duration_secs)
            }
        }
    }
}

/// Tempo-aware frequency specification that can adapt to tempo changes
#[derive(Debug, Clone, PartialEq)]
pub enum TempoAwareFrequency {
    /// Fixed frequency in Hz (not tempo-aware)
    Fixed(f64),
    /// Frequency specified in measures (tempo-aware)
    Measures(f64),
    /// Frequency specified in beats (tempo-aware)
    Beats(f64),
    /// Frequency specified in seconds (fixed, not tempo-aware)
    Seconds(f64),
}

impl TempoAwareFrequency {
    /// Get the current frequency in Hz, using tempo map if available
    pub fn to_hz(
        &self,
        tempo_map: Option<&crate::lighting::tempo::TempoMap>,
        at_time: Duration,
    ) -> f64 {
        match self {
            TempoAwareFrequency::Fixed(freq) => *freq,
            TempoAwareFrequency::Seconds(duration) => duration_to_rate(*duration),
            TempoAwareFrequency::Measures(measures) => {
                if *measures <= 0.0 {
                    return 0.0; // Zero/negative measures means stopped
                }
                let duration_secs = measures_to_duration_secs(*measures, tempo_map, at_time);
                duration_to_rate(duration_secs)
            }
            TempoAwareFrequency::Beats(beats) => {
                if *beats <= 0.0 {
                    return 0.0; // Zero/negative beats means stopped
                }
                let duration_secs = beats_to_duration_secs(*beats, tempo_map, at_time);
                duration_to_rate(duration_secs)
            }
        }
    }
}

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

    // ── TempoAwareSpeed — Fixed ────────────────────────────────────

    #[test]
    fn speed_fixed() {
        let speed = TempoAwareSpeed::Fixed(2.5);
        assert!((speed.to_cycles_per_second(None, Duration::ZERO) - 2.5).abs() < 1e-9);
    }

    // ── TempoAwareSpeed — Seconds ──────────────────────────────────

    #[test]
    fn speed_seconds() {
        let speed = TempoAwareSpeed::Seconds(4.0);
        // 1 cycle per 4 seconds = 0.25 cps
        assert!((speed.to_cycles_per_second(None, Duration::ZERO) - 0.25).abs() < 1e-9);
    }

    #[test]
    fn speed_seconds_zero() {
        let speed = TempoAwareSpeed::Seconds(0.0);
        assert_eq!(speed.to_cycles_per_second(None, Duration::ZERO), 0.0);
    }

    // ── TempoAwareSpeed — Beats (no tempo map → fallback 120 BPM) ─

    #[test]
    fn speed_beats_no_tempo_map() {
        // 1 beat at 120 BPM = 0.5 seconds → rate = 2.0 cps
        let speed = TempoAwareSpeed::Beats(1.0);
        let cps = speed.to_cycles_per_second(None, Duration::ZERO);
        assert!((cps - 2.0).abs() < 1e-9);
    }

    #[test]
    fn speed_beats_two_beats_no_tempo_map() {
        // 2 beats at 120 BPM = 1.0 second → rate = 1.0 cps
        let speed = TempoAwareSpeed::Beats(2.0);
        let cps = speed.to_cycles_per_second(None, Duration::ZERO);
        assert!((cps - 1.0).abs() < 1e-9);
    }

    #[test]
    fn speed_beats_zero() {
        let speed = TempoAwareSpeed::Beats(0.0);
        assert_eq!(speed.to_cycles_per_second(None, Duration::ZERO), 0.0);
    }

    #[test]
    fn speed_beats_negative() {
        let speed = TempoAwareSpeed::Beats(-1.0);
        assert_eq!(speed.to_cycles_per_second(None, Duration::ZERO), 0.0);
    }

    // ── TempoAwareSpeed — Measures (no tempo map → fallback) ──────

    #[test]
    fn speed_measures_no_tempo_map() {
        // 1 measure = 4 beats at 120 BPM = 2.0 seconds → rate = 0.5 cps
        let speed = TempoAwareSpeed::Measures(1.0);
        let cps = speed.to_cycles_per_second(None, Duration::ZERO);
        assert!((cps - 0.5).abs() < 1e-9);
    }

    #[test]
    fn speed_measures_zero() {
        let speed = TempoAwareSpeed::Measures(0.0);
        assert_eq!(speed.to_cycles_per_second(None, Duration::ZERO), 0.0);
    }

    #[test]
    fn speed_measures_negative() {
        let speed = TempoAwareSpeed::Measures(-1.0);
        assert_eq!(speed.to_cycles_per_second(None, Duration::ZERO), 0.0);
    }

    // ── TempoAwareFrequency — Fixed ────────────────────────────────

    #[test]
    fn freq_fixed() {
        let freq = TempoAwareFrequency::Fixed(10.0);
        assert!((freq.to_hz(None, Duration::ZERO) - 10.0).abs() < 1e-9);
    }

    // ── TempoAwareFrequency — Seconds ──────────────────────────────

    #[test]
    fn freq_seconds() {
        let freq = TempoAwareFrequency::Seconds(0.5);
        // 1 / 0.5 = 2.0 Hz
        assert!((freq.to_hz(None, Duration::ZERO) - 2.0).abs() < 1e-9);
    }

    #[test]
    fn freq_seconds_zero() {
        let freq = TempoAwareFrequency::Seconds(0.0);
        assert_eq!(freq.to_hz(None, Duration::ZERO), 0.0);
    }

    // ── TempoAwareFrequency — Beats (no tempo map) ─────────────────

    #[test]
    fn freq_beats_no_tempo_map() {
        // 1 beat at 120 BPM = 0.5s → 2.0 Hz
        let freq = TempoAwareFrequency::Beats(1.0);
        let hz = freq.to_hz(None, Duration::ZERO);
        assert!((hz - 2.0).abs() < 1e-9);
    }

    #[test]
    fn freq_beats_zero() {
        let freq = TempoAwareFrequency::Beats(0.0);
        assert_eq!(freq.to_hz(None, Duration::ZERO), 0.0);
    }

    // ── TempoAwareFrequency — Measures (no tempo map) ──────────────

    #[test]
    fn freq_measures_no_tempo_map() {
        // 1 measure = 4 beats at 120 BPM = 2.0s → 0.5 Hz
        let freq = TempoAwareFrequency::Measures(1.0);
        let hz = freq.to_hz(None, Duration::ZERO);
        assert!((hz - 0.5).abs() < 1e-9);
    }

    #[test]
    fn freq_measures_zero() {
        let freq = TempoAwareFrequency::Measures(0.0);
        assert_eq!(freq.to_hz(None, Duration::ZERO), 0.0);
    }

    // ── TempoAwareSpeed with TempoMap ──────────────────────────────

    #[test]
    fn speed_beats_with_tempo_map() {
        use crate::lighting::tempo::{TempoMap, TimeSignature};
        let tm = TempoMap::new(Duration::ZERO, 90.0, TimeSignature::new(4, 4), vec![]);
        // 1 beat at 90 BPM = 60/90 = 0.667s → rate ≈ 1.5 cps
        let speed = TempoAwareSpeed::Beats(1.0);
        let cps = speed.to_cycles_per_second(Some(&tm), Duration::ZERO);
        assert!((cps - 1.5).abs() < 0.01);
    }

    #[test]
    fn speed_measures_with_tempo_map() {
        use crate::lighting::tempo::{TempoMap, TimeSignature};
        let tm = TempoMap::new(Duration::ZERO, 60.0, TimeSignature::new(4, 4), vec![]);
        // 1 measure = 4 beats at 60 BPM = 4.0s → rate = 0.25 cps
        let speed = TempoAwareSpeed::Measures(1.0);
        let cps = speed.to_cycles_per_second(Some(&tm), Duration::ZERO);
        assert!((cps - 0.25).abs() < 0.01);
    }

    #[test]
    fn freq_beats_with_tempo_map() {
        use crate::lighting::tempo::{TempoMap, TimeSignature};
        let tm = TempoMap::new(Duration::ZERO, 60.0, TimeSignature::new(4, 4), vec![]);
        // 1 beat at 60 BPM = 1.0s → 1.0 Hz
        let freq = TempoAwareFrequency::Beats(1.0);
        let hz = freq.to_hz(Some(&tm), Duration::ZERO);
        assert!((hz - 1.0).abs() < 0.01);
    }
}
