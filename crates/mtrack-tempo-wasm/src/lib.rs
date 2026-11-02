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
//! mtrack's tempo map and cue clock for the web UI, as a WebAssembly module.
//!
//! The module is the player's own `src/tempo.rs` (and `src/tempo/`), compiled
//! as it is, so the lighting timeline puts bars, beats, tempo changes, ramps
//! and offsets exactly where the player does. Nothing here does any tempo
//! arithmetic of its own; it only carries numbers across the boundary.
//!
//! The interface is plain C functions of numbers, so the module needs no
//! bindings generator and imports nothing. Times are in seconds. Lists go in
//! through [`input_buffer`] and come back through [`output_buffer`]; maps and
//! clocks are referred to by handle (0 is none). The TypeScript side is
//! `src/webui/svelte/src/lib/tempo-wasm.ts`.

#[path = "../../../src/tempo.rs"]
#[allow(dead_code)]
pub mod tempo;

use std::cell::RefCell;
use std::time::Duration;

use tempo::{
    CueClock, CuePosition, TempoChange, TempoChangePosition, TempoMap, TempoTransition,
    TimeSignature, TransitionCurve,
};

thread_local! {
    static INPUT: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
    static OUTPUT: RefCell<Vec<f64>> = const { RefCell::new(Vec::new()) };
    static MAPS: RefCell<Vec<Option<TempoMap>>> = const { RefCell::new(Vec::new()) };
    static CLOCKS: RefCell<Vec<Option<CueClock>>> = const { RefCell::new(Vec::new()) };
}

/// A buffer of `len` numbers for the caller to fill before a call that
/// reads its input from it.
#[no_mangle]
pub extern "C" fn input_buffer(len: usize) -> *mut f64 {
    INPUT.with(|input| {
        let mut input = input.borrow_mut();
        input.clear();
        input.resize(len, 0.0);
        input.as_mut_ptr()
    })
}

/// Where the last call that returns a list left it.
#[no_mangle]
pub extern "C" fn output_buffer() -> *const f64 {
    OUTPUT.with(|output| output.borrow().as_ptr())
}

fn set_output(values: &[f64]) {
    OUTPUT.with(|output| {
        let mut output = output.borrow_mut();
        output.clear();
        output.extend_from_slice(values);
    });
}

fn insert<T>(slab: &RefCell<Vec<Option<T>>>, value: T) -> u32 {
    let mut slab = slab.borrow_mut();
    let index = match slab.iter().position(Option::is_none) {
        Some(free) => {
            slab[free] = Some(value);
            free
        }
        None => {
            slab.push(Some(value));
            slab.len() - 1
        }
    };
    index as u32 + 1
}

fn remove<T>(slab: &RefCell<Vec<Option<T>>>, handle: u32) {
    if let Some(slot) = (handle as usize)
        .checked_sub(1)
        .and_then(|i| slab.borrow_mut().get_mut(i).map(Option::take))
    {
        drop(slot);
    }
}

/// The largest time, in seconds either side of 0, a call takes: about 11
/// days, far past any song, and small enough that every walk through bars
/// and beats from there is quick.
const MAX_SECS: f64 = 1.0e6;
/// The largest beat position or count a call takes.
const MAX_BEATS: f64 = 1.0e7;
/// The largest bar (or count of bars) a call takes.
const MAX_MEASURES: f64 = 1.0e6;

/// Every call checks its numbers before any reaches the tempo map, so
/// nothing past here can panic -- which in WebAssembly is a trap that would
/// leave a map or clock borrowed for good. A call given one it does not take
/// answers NaN (0 for a handle or a status).
fn ok_secs(secs: f64) -> bool {
    secs.is_finite() && secs.abs() <= MAX_SECS
}

/// An offset: a time, never negative.
fn ok_offset(secs: f64) -> bool {
    ok_secs(secs) && secs >= 0.0
}

fn ok_beats(beats: f64) -> bool {
    beats.is_finite() && beats.abs() <= MAX_BEATS
}

fn ok_measures(measures: f64) -> bool {
    measures.is_finite() && measures.abs() <= MAX_MEASURES
}

/// A bar and beat: a bar from 1, a beat from 1 within reach.
fn ok_position(measure: f64, beat: f64) -> bool {
    ok_measures(measure) && measure >= 1.0 && ok_beats(beat) && beat >= 1.0
}

fn ok_bpm(bpm: f64) -> bool {
    bpm.is_finite() && bpm > 0.0 && bpm <= 1.0e4
}

/// Runs `f` on the map `handle` names; NaN for none.
fn with_map(handle: u32, f: impl FnOnce(&TempoMap) -> f64) -> f64 {
    MAPS.with(|maps| {
        let maps = maps.borrow();
        match (handle as usize)
            .checked_sub(1)
            .and_then(|i| maps.get(i))
            .and_then(Option::as_ref)
        {
            Some(map) => f(map),
            None => f64::NAN,
        }
    })
}

fn secs(time: Duration) -> f64 {
    time.as_secs_f64()
}

/// A time in seconds; none before 0.
fn time(secs: f64) -> Duration {
    Duration::try_from_secs_f64(secs.max(0.0)).unwrap_or(Duration::ZERO)
}

/// Numbers per tempo change in a map's input.
pub const CHANGE_LEN: usize = 8;

/// Reads a tempo map from numbers: `[start, bpm, numerator, denominator,
/// change count]`, then [`CHANGE_LEN`] numbers per change: `[kind (0 at a
/// time, 1 at a bar), time or bar, beat, bpm (NaN for none), numerator (0
/// for none), denominator, transition (0 snap, 1 beats, 2 measures),
/// transition length]`. A tempo that is not positive is none (120 for the
/// starting one), and so is a transition that is not.
pub fn read_map(input: &[f64]) -> Option<TempoMap> {
    let header = input.get(..5)?;
    if !(ok_offset(header[0]) && header[4].is_finite() && header[4] >= 0.0) {
        return None;
    }
    let positive = |bpm: f64| ok_bpm(bpm).then_some(bpm);
    let signature = |num: f64, den: f64| {
        (num >= 1.0 && den >= 1.0 && num <= 1024.0 && den <= 1024.0)
            .then(|| TimeSignature::new(num as u32, den as u32))
    };
    let count = header[4] as usize;
    let end = count.checked_mul(CHANGE_LEN)?.checked_add(5)?;
    let mut changes = Vec::with_capacity(count);
    for change in input.get(5..end)?.chunks(CHANGE_LEN) {
        let at_ok = if change[0] == 0.0 {
            ok_offset(change[1])
        } else {
            ok_position(change[1], change[2])
        };
        let length_ok = change[6] == 0.0 || (change[7].is_finite() && change[7] <= MAX_MEASURES);
        if !at_ok || !length_ok {
            return None;
        }
        let position = if change[0] == 0.0 {
            TempoChangePosition::Time(time(change[1]))
        } else {
            TempoChangePosition::MeasureBeat((change[1].max(1.0)) as u32, change[2].max(1.0))
        };
        let length = change[7];
        let transition = match change[6] {
            k if k == 1.0 && length > 0.0 => {
                TempoTransition::Beats(length, TransitionCurve::Linear)
            }
            k if k == 2.0 && length > 0.0 => {
                TempoTransition::Measures(length, TransitionCurve::Linear)
            }
            _ => TempoTransition::Snap,
        };
        changes.push(TempoChange {
            position,
            original_measure_beat: None,
            bpm: positive(change[3]),
            time_signature: signature(change[4], change[5]),
            transition,
        });
    }
    Some(TempoMap::new(
        time(header[0]),
        positive(header[1]).unwrap_or(tempo::DEFAULT_BPM),
        signature(header[2], header[3]).unwrap_or(TimeSignature::new(4, 4)),
        changes,
    ))
}

/// A tempo map read from the first `len` numbers of the input buffer (see
/// [`read_map`]); 0 when they are not one.
#[no_mangle]
pub extern "C" fn tm_new(len: usize) -> u32 {
    let map = INPUT.with(|input| read_map(input.borrow().get(..len).unwrap_or(&[])));
    match map {
        Some(map) => MAPS.with(|maps| insert(maps, map)),
        None => 0,
    }
}

#[no_mangle]
pub extern "C" fn tm_free(map: u32) {
    MAPS.with(|maps| remove(maps, map));
}

/// When bar 1 starts.
#[no_mangle]
pub extern "C" fn tm_start(map: u32) -> f64 {
    with_map(map, |tm| secs(tm.start_offset))
}

/// The time of a quarter-note beat position (0 at bar 1 beat 1), under an
/// offset. Before bar 1 (and before 0), at the starting tempo.
#[no_mangle]
pub extern "C" fn tm_time_at_beats(map: u32, beats: f64, offset: f64) -> f64 {
    if !(ok_beats(beats) && ok_offset(offset)) {
        return f64::NAN;
    }
    with_map(map, |tm| {
        if beats < 0.0 {
            secs(tm.start_offset) + beats * 60.0 / tm.initial_bpm
        } else {
            secs(tm.time_at_beats(beats, offset))
        }
    })
}

/// The quarter-note beat position of a time, under an offset: the inverse
/// of [`tm_time_at_beats`].
#[no_mangle]
pub extern "C" fn tm_beats_at(map: u32, at: f64, offset: f64) -> f64 {
    if !(ok_secs(at) && ok_offset(offset)) {
        return f64::NAN;
    }
    with_map(map, |tm| {
        if at < 0.0 {
            tm.beats_at(Duration::ZERO, 0.0) + at * tm.initial_bpm / 60.0
        } else {
            tm.beats_at(time(at), offset)
        }
    })
}

/// The quarter-note beat position of a bar and (quarter-note) beat.
#[no_mangle]
pub extern "C" fn tm_beats_at_position(map: u32, measure: f64, beat: f64) -> f64 {
    if !(ok_position(measure, beat)) {
        return f64::NAN;
    }
    with_map(map, |tm| {
        tm.beats_at_position(measure.max(1.0) as u32, beat)
    })
}

/// The bar of a quarter-note beat position; output `[bar, beat]`.
#[no_mangle]
pub extern "C" fn tm_position_at_beats(map: u32, beats: f64) -> f64 {
    if !(ok_beats(beats)) {
        return f64::NAN;
    }
    with_map(map, |tm| {
        let (measure, beat) = tm.position_at_beats(beats);
        set_output(&[f64::from(measure), beat]);
        f64::from(measure)
    })
}

/// Measures elapsed (0 at bar 1 beat 1) at a quarter-note beat position.
#[no_mangle]
pub extern "C" fn tm_measures_at_beats(map: u32, beats: f64) -> f64 {
    if !(ok_beats(beats)) {
        return f64::NAN;
    }
    with_map(map, |tm| tm.measures_elapsed_at_beats(beats))
}

/// The quarter-note beat position at a number of measures elapsed.
#[no_mangle]
pub extern "C" fn tm_beats_at_measures(map: u32, measures: f64) -> f64 {
    if !(ok_measures(measures)) {
        return f64::NAN;
    }
    with_map(map, |tm| tm.beats_at_measures_elapsed(measures))
}

/// The time signature bar `measure` is in; output `[numerator,
/// denominator]`.
#[no_mangle]
pub extern "C" fn tm_signature(map: u32, measure: f64) -> f64 {
    if !(ok_position(measure, 1.0)) {
        return f64::NAN;
    }
    with_map(map, |tm| {
        let ts = tm.time_signature_in_measure(measure.max(1.0) as u32);
        set_output(&[f64::from(ts.numerator), f64::from(ts.denominator)]);
        f64::from(ts.numerator)
    })
}

/// The tempo at a time (mid-ramp, the ramp's), under an offset.
#[no_mangle]
pub extern "C" fn tm_bpm_at(map: u32, at: f64, offset: f64) -> f64 {
    if !(ok_secs(at) && ok_offset(offset)) {
        return f64::NAN;
    }
    with_map(map, |tm| tm.bpm_at_time(time(at), offset))
}

/// The tempo the ramp in progress at `at` is heading for; NaN when the
/// tempo is steady there.
#[no_mangle]
pub extern "C" fn tm_ramp_target(map: u32, at: f64) -> f64 {
    if !(ok_secs(at)) {
        return f64::NAN;
    }
    with_map(map, |tm| tm.ramp_target_at(time(at)).unwrap_or(f64::NAN))
}

/// How long a ramp from `old_bpm` to `new_bpm` written over `beats` beats
/// lasts.
#[no_mangle]
pub extern "C" fn ramp_duration(old_bpm: f64, new_bpm: f64, beats: f64) -> f64 {
    if !(ok_bpm(old_bpm) && ok_bpm(new_bpm) && ok_beats(beats)) {
        return f64::NAN;
    }
    TransitionCurve::Linear.duration_for_beats(old_bpm, new_bpm, beats)
}

/// How long `beats` beats from `at` last, under an offset.
#[no_mangle]
pub extern "C" fn tm_beats_to_duration(map: u32, beats: f64, at: f64, offset: f64) -> f64 {
    if !(ok_beats(beats) && ok_secs(at) && ok_offset(offset)) {
        return f64::NAN;
    }
    with_map(map, |tm| {
        secs(tm.beats_to_duration(beats, time(at), offset))
    })
}

/// How long `measures` measures from `at` last, under an offset.
#[no_mangle]
pub extern "C" fn tm_measures_to_duration(map: u32, measures: f64, at: f64, offset: f64) -> f64 {
    if !(ok_measures(measures) && ok_secs(at) && ok_offset(offset)) {
        return f64::NAN;
    }
    with_map(map, |tm| {
        secs(tm.measures_to_duration(measures, time(at), offset))
    })
}

/// How long an `offset N measures` issued at `anchor` lasts.
#[no_mangle]
pub extern "C" fn tm_offset_duration(map: u32, measures: f64, anchor: f64) -> f64 {
    if !(ok_measures(measures) && measures >= 0.0 && ok_secs(anchor)) {
        return f64::NAN;
    }
    with_map(map, |tm| secs(tm.offset_duration(measures, time(anchor))))
}

/// The times the tempo or time signature in effect changes; output the
/// times, returns how many.
#[no_mangle]
pub extern "C" fn tm_boundaries(map: u32) -> f64 {
    with_map(map, |tm| {
        let bounds: Vec<f64> = tm.tempo_boundaries().into_iter().map(secs).collect();
        set_output(&bounds);
        bounds.len() as f64
    })
}

/// A cue clock at the start of a list of cues.
#[no_mangle]
pub extern "C" fn clock_new() -> u32 {
    CLOCKS.with(|clocks| insert(clocks, CueClock::new()))
}

#[no_mangle]
pub extern "C" fn clock_free(clock: u32) {
    CLOCKS.with(|clocks| remove(clocks, clock));
}

/// Places the next cue (see [`CueClock::place`]): at time `a` (`kind` 0) or
/// bar `a` beat `b` (`kind` 1), through `map` (0 for none). Output `[time,
/// unshifted time (NaN for none), offset in force, offset issued, offset
/// after]`; returns 1, or 0 when it cannot be placed.
#[no_mangle]
pub extern "C" fn clock_place(
    clock: u32,
    map: u32,
    kind: u32,
    a: f64,
    b: f64,
    offset_measures: u32,
    offsets: u32,
    reset: u32,
) -> u32 {
    let valid = if kind == 0 {
        ok_offset(a)
    } else {
        ok_position(a, b)
    };
    if !valid || f64::from(offset_measures) > MAX_MEASURES {
        return 0;
    }
    let at = if kind == 0 {
        CuePosition::Time(time(a))
    } else {
        CuePosition::MeasureBeat(a.max(1.0) as u32, b)
    };
    MAPS.with(|maps| {
        let maps = maps.borrow();
        let tm = (map as usize)
            .checked_sub(1)
            .and_then(|i| maps.get(i))
            .and_then(Option::as_ref);
        CLOCKS.with(|clocks: &RefCell<Vec<Option<CueClock>>>| {
            let mut clocks = clocks.borrow_mut();
            let Some(clock) = (clock as usize)
                .checked_sub(1)
                .and_then(|i| clocks.get_mut(i))
                .and_then(Option::as_mut)
            else {
                return 0;
            };
            match clock.place(tm, at, offset_measures, offsets != 0, reset != 0) {
                Ok(placed) => {
                    set_output(&[
                        secs(placed.time),
                        placed.unshifted.map_or(f64::NAN, secs),
                        placed.offset_secs,
                        placed.issued_secs,
                        clock.offset_secs(),
                    ]);
                    1
                }
                Err(_) => 0,
            }
        })
    })
}

/// Records that the cue just placed reaches on to `at`.
#[no_mangle]
pub extern "C" fn clock_reached(clock: u32, at: f64) {
    if !ok_offset(at) {
        return;
    }
    CLOCKS.with(|clocks: &RefCell<Vec<Option<CueClock>>>| {
        let mut clocks = clocks.borrow_mut();
        if let Some(clock) = (clock as usize)
            .checked_sub(1)
            .and_then(|i| clocks.get_mut(i))
            .and_then(Option::as_mut)
        {
            clock.reached(time(at));
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steady(bpm: f64) -> u32 {
        let input = [0.0, bpm, 4.0, 4.0, 0.0];
        let ptr = input_buffer(input.len());
        // SAFETY: the buffer holds `input.len()` numbers.
        unsafe { std::ptr::copy_nonoverlapping(input.as_ptr(), ptr, input.len()) };
        tm_new(input.len())
    }

    #[test]
    fn a_map_round_trips_beats_and_times() {
        let map = steady(120.0);
        assert!((tm_time_at_beats(map, 8.0, 0.0) - 4.0).abs() < 1e-9);
        assert!((tm_beats_at(map, 4.0, 0.0) - 8.0).abs() < 1e-9);
        assert!((tm_time_at_beats(map, -2.0, 0.0) + 1.0).abs() < 1e-9);
        tm_free(map);
        assert!(tm_start(map).is_nan());
    }

    #[test]
    fn a_clock_places_through_a_map() {
        let map = steady(120.0);
        let clock = clock_new();
        assert_eq!(clock_place(clock, map, 1, 2.0, 1.0, 1, 1, 0), 1);
        assert_eq!(clock_place(clock, map, 1, 3.0, 1.0, 0, 0, 0), 1);
        let out = OUTPUT.with(|o| o.borrow().clone());
        assert!((out[0] - 6.0).abs() < 1e-9);
        assert!((out[2] - 2.0).abs() < 1e-9);
        assert_eq!(clock_place(clock, 0, 1, 3.0, 1.0, 0, 0, 0), 0);
        clock_free(clock);
        tm_free(map);
    }

    #[test]
    fn a_bad_map_is_none() {
        input_buffer(3);
        assert_eq!(tm_new(3), 0);
    }

    #[test]
    fn bad_numbers_are_refused_and_leave_the_module_working() {
        let map = steady(120.0);
        for bad in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, 1e300] {
            assert!(tm_time_at_beats(map, bad, 0.0).is_nan());
            assert!(tm_time_at_beats(map, 4.0, bad).is_nan());
            assert!(tm_beats_at(map, bad, 0.0).is_nan());
            assert!(tm_beats_at_position(map, bad, 1.0).is_nan());
            assert!(tm_beats_at_position(map, 2.0, bad).is_nan());
            assert!(tm_position_at_beats(map, bad).is_nan());
            assert!(tm_measures_at_beats(map, bad).is_nan());
            assert!(tm_beats_at_measures(map, bad).is_nan());
            assert!(tm_signature(map, bad).is_nan());
            assert!(tm_bpm_at(map, bad, 0.0).is_nan());
            assert!(tm_bpm_at(map, 1.0, bad).is_nan());
            assert!(tm_ramp_target(map, bad).is_nan());
            assert!(ramp_duration(bad, 120.0, 4.0).is_nan());
            assert!(tm_beats_to_duration(map, 4.0, 0.0, bad).is_nan());
            assert!(tm_measures_to_duration(map, bad, 0.0, 0.0).is_nan());
            assert!(tm_offset_duration(map, bad, 0.0).is_nan());
        }
        // A negative offset, a bar 0 and a beat before 1 are not positions.
        assert!(tm_time_at_beats(map, 4.0, -1.0).is_nan());
        assert!(tm_beats_at_position(map, 0.0, 1.0).is_nan());
        assert!(tm_beats_at_position(map, 2.0, 0.5).is_nan());
        assert!(ramp_duration(0.0, 120.0, 4.0).is_nan());
        let clock = clock_new();
        assert_eq!(clock_place(clock, map, 0, f64::NAN, 1.0, 0, 0, 0), 0);
        assert_eq!(clock_place(clock, map, 1, 2.0, f64::INFINITY, 0, 0, 0), 0);
        clock_reached(clock, f64::NAN);
        // Nothing was left borrowed: every call still answers.
        assert!((tm_time_at_beats(map, 8.0, 0.0) - 4.0).abs() < 1e-9);
        assert_eq!(clock_place(clock, map, 1, 2.0, 1.0, 0, 0, 0), 1);
        clock_free(clock);
        tm_free(map);
    }

    #[test]
    fn a_map_with_numbers_out_of_reach_is_none() {
        let bad_maps: [&[f64]; 5] = [
            &[f64::NAN, 120.0, 4.0, 4.0, 0.0],
            &[0.0, 120.0, 4.0, 4.0, f64::INFINITY],
            &[0.0, 120.0, 4.0, 4.0, 1e300],
            // A change at bar four billion.
            &[
                0.0, 120.0, 4.0, 4.0, 1.0, 1.0, 4e9, 1.0, 60.0, 0.0, 0.0, 0.0, 0.0,
            ],
            // A change at a time that is not one.
            &[
                0.0,
                120.0,
                4.0,
                4.0,
                1.0,
                0.0,
                f64::NAN,
                1.0,
                60.0,
                0.0,
                0.0,
                0.0,
                0.0,
            ],
        ];
        for input in bad_maps {
            let ptr = input_buffer(input.len());
            // SAFETY: the buffer holds `input.len()` numbers.
            unsafe { std::ptr::copy_nonoverlapping(input.as_ptr(), ptr, input.len()) };
            assert_eq!(tm_new(input.len()), 0, "{input:?}");
        }
    }
}
