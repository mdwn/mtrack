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
//! How long a dragged effect is, as mtrack writes it: the distance between
//! two times on a tempo map in beats and bars, a dragged length snapped to
//! whole bars or beats, and the duration a length is written as
//! (`1measure`, `2beats`, `500ms`). Shared by every editor of a light show:
//! the web UI through `mtrack-rules-wasm`, which compiles this file as it
//! is, and anything editing a show on a server.

use std::time::Duration;

use serde::Serialize;

use crate::lighting::parser::writer;
use crate::tempo::TempoMap;

/// The longest time on a song, in seconds: past it, a time is not one.
const MAX_SECS: f64 = 1.0e9;

/// `ms` milliseconds as a time on a song: `None` for a negative one or one
/// past [`MAX_SECS`].
fn at_ms(ms: f64) -> Option<Duration> {
    (ms >= 0.0 && secs(ms / 1000.0)).then(|| Duration::from_secs_f64(ms / 1000.0))
}

fn secs(secs: f64) -> bool {
    secs.is_finite() && secs.abs() <= MAX_SECS
}

/// The distance between two times on a tempo map, as the position walk
/// counts it: in milliseconds, in quarter-note beats
/// (`TempoMap::beats_between`) and in bars of the metres between
/// (`TempoMap::measures_between`, fractional within a bar). What a dragged
/// length is named by.
#[cfg_attr(feature = "tsify", derive(tsify::Tsify))]
#[derive(Serialize, Clone, Copy, Debug, PartialEq)]
pub struct TempoDistance {
    pub ms: f64,
    pub beats: f64,
    pub measures: f64,
}

impl TempoDistance {
    /// The distance from `from_ms` to `to_ms` on `map`; `None` when a time
    /// is not one.
    pub fn between(map: &TempoMap, from_ms: f64, to_ms: f64) -> Option<TempoDistance> {
        let (from, to) = (at_ms(from_ms)?, at_ms(to_ms)?);
        Some(TempoDistance {
            ms: to_ms - from_ms,
            beats: map.beats_between(from, to),
            measures: map.measures_between(from, to),
        })
    }
}

/// The duration an effect dragged to `ms` long from `at_ms` is written as
/// (`writer::duration_text`): whole measures or beats of the tempo map
/// there when there is one and they are whole, else time.
pub fn length_name(map: Option<&TempoMap>, at_ms: f64, ms: f64) -> String {
    let span = map
        .and_then(|map| TempoDistance::between(map, at_ms, at_ms + ms))
        .map(|d| (d.measures, d.beats));
    writer::duration_text(span, ms)
}

/// A length dragged from `at_ms`, snapped to whole measures (`step`
/// `None`) or to whole steps of a beat, in milliseconds as the parser reads
/// that length there. `None` when a time is not one.
pub fn snapped_length(
    map: &TempoMap,
    at_ms: f64,
    duration_ms: f64,
    step: Option<f64>,
) -> Option<f64> {
    if step.is_some_and(|step| !(step.is_finite() && step > 0.0)) {
        return None;
    }
    let span = TempoDistance::between(map, at_ms, at_ms + duration_ms)?;
    let round = writer::round;
    let from = self::at_ms(at_ms)?;
    // The player's own length of that many bars or beats from there.
    let length = match step {
        None => map.measures_to_duration(round(span.measures).max(1.0), from, 0.0),
        Some(step) => map.beats_to_duration((round(span.beats / step) * step).max(step), from, 0.0),
    };
    // A length past any song (a step so fine its count overflows) is none.
    let end = from.checked_add(length)?.as_secs_f64();
    secs(end).then_some(end * 1000.0 - at_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tempo::TimeSignature;

    /// 120 bpm in 4/4: a beat is 500 ms, a bar 2000 ms.
    fn map() -> TempoMap {
        TempoMap::new(Duration::ZERO, 120.0, TimeSignature::new(4, 4), Vec::new())
    }

    #[test]
    fn a_whole_number_of_bars_or_beats_is_written_so() {
        let map = map();
        assert_eq!(length_name(Some(&map), 0.0, 4000.0), "2measures");
        assert_eq!(length_name(Some(&map), 0.0, 1500.0), "3beats");
        assert_eq!(length_name(Some(&map), 0.0, 1234.0), "1234ms");
        // No tempo: time.
        assert_eq!(length_name(None, 0.0, 3000.0), "3s");
    }

    #[test]
    fn a_dragged_length_snaps_to_bars_or_beats() {
        let map = map();
        assert_eq!(snapped_length(&map, 0.0, 4700.0, None), Some(4000.0));
        assert_eq!(snapped_length(&map, 0.0, 1300.0, Some(1.0)), Some(1500.0));
        assert_eq!(
            snapped_length(&map, 0.0, 100.0, Some(1.0)),
            Some(500.0),
            "at least one"
        );
        assert_eq!(snapped_length(&map, 0.0, 1300.0, Some(0.0)), None);
        assert_eq!(snapped_length(&map, -5.0, 1000.0, None), None);
    }

    #[test]
    fn the_distance_counts_beats_and_bars() {
        let d = TempoDistance::between(&map(), 1000.0, 5000.0).unwrap();
        assert_eq!(d.ms, 4000.0);
        assert!((d.beats - 8.0).abs() < 1e-9);
        assert!((d.measures - 2.0).abs() < 1e-9);
    }
}
