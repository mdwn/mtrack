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
//! Where a cue plays: its written position resolved through the tempo map and
//! the offsets of the cues before it.
//!
//! The light show parser places every cue (in a show, a sequence or an inline
//! loop) with a [`CueClock`], and the web UI's timeline places the cues it
//! draws with the same clock, compiled to WebAssembly.

use std::time::Duration;

use super::TempoMap;

/// A cue's position as written.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CuePosition {
    /// `@mm:ss.mmm`: a time.
    Time(Duration),
    /// `@bar/beat`: a position in the tempo map.
    MeasureBeat(u32, f64),
}

/// Where a cue plays, as [`CueClock::place`] resolves it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlacedCue {
    /// When the cue plays: its score time under the offset in force at it,
    /// plus that offset.
    pub time: Duration,
    /// Its position in the tempo map before any offset, for a cue written
    /// at a bar and beat.
    pub unshifted: Option<Duration>,
    /// The offset in force at the cue, in seconds.
    pub offset_secs: f64,
    /// How long the offset the cue itself issues lasts, in seconds (0 for
    /// none). It applies to the cues after it.
    pub issued_secs: f64,
}

/// The running offset of a list of cues, placing each in turn.
///
/// An `offset N measures` shifts every later cue by N bars at the tempo and
/// time signature at the cue that issues it ([`TempoMap::offset_duration`]);
/// a `reset_measures` drops the shift. Under an offset, the tempo changes
/// positioned in time move with the cues, so a cue at a bar and beat is
/// placed through the map as shifted, and then shifted itself.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct CueClock {
    offset_secs: f64,
    last_time: Option<Duration>,
}

impl CueClock {
    /// A clock at the start of a list of cues: no offset, no cue yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// The offset the next cue is placed under, in seconds.
    pub fn offset_secs(&self) -> f64 {
        self.offset_secs
    }

    /// Places the next cue, written at `at`, which issues an offset of
    /// `offset_measures` measures when `offsets` (several `offset` commands
    /// on one cue add up) and drops the running offset first when `reset`.
    pub fn place(
        &mut self,
        tempo_map: Option<&TempoMap>,
        at: CuePosition,
        offset_measures: u32,
        offsets: bool,
        reset: bool,
    ) -> Result<PlacedCue, String> {
        let applied = self.offset_secs;
        let (score_time, unshifted) = match at {
            CuePosition::Time(time) => (time, None),
            CuePosition::MeasureBeat(measure, beat) => {
                let tm = tempo_map.ok_or("Measure-based timing requires a tempo section")?;
                let invalid = || format!("Invalid measure/beat position: {}/{}", measure, beat);
                let unshifted = tm
                    .measure_to_time_with_offset(measure, beat, 0, 0.0)
                    .ok_or_else(invalid)?;
                let score_time = tm
                    .measure_to_time_with_offset(measure, beat, 0, applied)
                    .ok_or_else(invalid)?;
                (score_time, Some(unshifted))
            }
        };

        // An offset is sized at the cue that issues it, where the tempo map
        // has it before any offset; a cue at the very start sizes it where
        // the cue before it is.
        let mut issued_secs = 0.0;
        if offsets || reset {
            if reset {
                self.offset_secs = 0.0;
            }
            if offsets {
                let tm = tempo_map.ok_or("Offset command requires a tempo section")?;
                let anchor = match unshifted {
                    Some(time) if !time.is_zero() => time,
                    _ if !score_time.is_zero() => score_time,
                    _ => self
                        .last_time
                        .map(|t| {
                            t.saturating_sub(super::duration_from_secs(applied))
                                .saturating_sub(tm.start_offset)
                        })
                        .unwrap_or(Duration::ZERO),
                };
                issued_secs = tm
                    .offset_duration(f64::from(offset_measures), anchor)
                    .as_secs_f64();
                self.offset_secs += issued_secs;
            }
        }

        let time = score_time.saturating_add(super::duration_from_secs(applied));
        self.last_time = Some(time);
        Ok(PlacedCue {
            time,
            unshifted,
            offset_secs: applied,
            issued_secs,
        })
    }

    /// Records that the cue just placed reaches on to `time` (an inline loop
    /// or a sequence played from it), for the next cue's offset anchor.
    pub fn reached(&mut self, time: Duration) {
        if self.last_time.is_none_or(|last| time > last) {
            self.last_time = Some(time);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tempo::{TempoChange, TempoChangePosition, TempoTransition, TimeSignature};

    fn map(changes: Vec<TempoChange>) -> TempoMap {
        TempoMap::new(Duration::ZERO, 120.0, TimeSignature::new(4, 4), changes)
    }

    #[test]
    fn an_offset_shifts_the_cues_after_it_and_a_reset_drops_it() {
        let tm = map(vec![]);
        let mut clock = CueClock::new();
        let first = clock
            .place(Some(&tm), CuePosition::MeasureBeat(2, 1.0), 2, true, false)
            .unwrap();
        assert_eq!(first.time, Duration::from_secs(2));
        assert_eq!(first.offset_secs, 0.0);
        assert!((first.issued_secs - 4.0).abs() < 1e-9);

        let second = clock
            .place(Some(&tm), CuePosition::MeasureBeat(3, 1.0), 0, false, true)
            .unwrap();
        assert_eq!(second.time, Duration::from_secs(8));
        let third = clock
            .place(Some(&tm), CuePosition::MeasureBeat(4, 1.0), 0, false, false)
            .unwrap();
        assert_eq!(third.time, Duration::from_secs(6));
    }

    #[test]
    fn an_offset_is_sized_at_the_tempo_where_it_is_issued() {
        let tm = map(vec![TempoChange {
            position: TempoChangePosition::MeasureBeat(3, 1.0),
            original_measure_beat: None,
            bpm: Some(60.0),
            time_signature: None,
            transition: TempoTransition::Snap,
        }]);
        let mut clock = CueClock::new();
        let placed = clock
            .place(Some(&tm), CuePosition::MeasureBeat(3, 1.0), 1, true, false)
            .unwrap();
        assert!((placed.issued_secs - 4.0).abs() < 1e-9);
    }

    #[test]
    fn bars_need_a_tempo_map() {
        let mut clock = CueClock::new();
        assert!(clock
            .place(None, CuePosition::MeasureBeat(1, 1.0), 0, false, false)
            .is_err());
        assert!(clock
            .place(None, CuePosition::Time(Duration::ZERO), 1, true, false)
            .is_err());
    }
}
