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
//! The module's C interface, called as the web UI calls it.

use mtrack_tempo_wasm::*;

/// The first `count` numbers the last list-returning call left.
fn output(count: usize) -> Vec<f64> {
    // SAFETY: the calls these follow leave at least `count` numbers there.
    unsafe { std::slice::from_raw_parts(output_buffer(), count).to_vec() }
}

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
    let out = output(5);
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

/// Lays out `source` through the module, as the web UI does.
fn layout(source: &str, map: u32) -> String {
    let ptr = text_buffer(source.len());
    // SAFETY: the buffer holds `source.len()` bytes.
    unsafe { std::ptr::copy_nonoverlapping(source.as_ptr(), ptr, source.len()) };
    let len = layout_sequences(source.len(), map);
    // SAFETY: the call left `len` bytes of UTF-8 there.
    let bytes = unsafe { std::slice::from_raw_parts(text_output(), len) };
    String::from_utf8(bytes.to_vec()).unwrap()
}

#[test]
fn a_shows_sequences_are_laid_out_through_the_module() {
    let source = r#"
sequence "s" {
    @0.000
    front_wash: static, color: "red", duration: 500ms
}

show "Show" {
    @0.000
    sequence "s", loop: 2
}
"#;
    let json = layout(source, 0);
    assert_eq!(
        json,
        "{\"shows\":[{\"name\":\"Show\",\"calls\":[{\"cue\":0,\"call\":0,\"name\":\"s\",\
         \"offset\":0,\"end\":1,\"cues\":[{\"time\":0,\"effects\":[{\"groups\":[\"front_wash\"],\
         \"layer\":\"background\",\"type\":\"static\",\"secs\":0.5}]},{\"time\":0.5,\"effects\":\
         [{\"groups\":[\"front_wash\"],\"layer\":\"background\",\"type\":\"static\",\"secs\":0.5}]}]}]}]}"
    );
}

#[test]
fn a_source_that_does_not_parse_is_an_error_and_leaves_the_module_working() {
    assert!(layout("show {", 0).starts_with("{\"error\":"));
    assert!(layout("show \"S\" { @0.000 }", 0).starts_with("{\"shows\":"));
}
