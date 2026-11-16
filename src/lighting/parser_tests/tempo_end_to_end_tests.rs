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
//! A tempo map made from a click-derived beat grid.

use crate::lighting::parser::*;

/// A map derived from a click-derived grid drives cueing the same way a
/// hand-written block does — the point of #337.
#[test]
fn a_grid_derived_map_resolves_bar_beat_cues() {
    // Four 4/4 bars at 120bpm, starting 1.5s in.
    let spacing = 0.5_f64;
    let lead_in = 1.5_f64;
    let mut beats = Vec::new();
    let mut measure_starts = Vec::new();
    for measure in 0..5 {
        measure_starts.push(beats.len());
        for beat in 0..4 {
            beats.push(lead_in + (measure * 4 + beat) as f64 * spacing);
        }
    }
    let grid = crate::audio::click_analysis::BeatGrid {
        beats,
        measure_starts,
    };
    let map = grid.to_tempo_map().expect("grid should yield a map");

    let source = r#"
show "T" {
    @3/1
    wash: static color: "blue", duration: 1measures
}
"#;
    let shows = parse_light_shows_with_tempo(source, Some(&map)).expect("parses");
    let show = shows.get("T").expect("show");

    // Bar 1 is at the lead-in (1.5s); each bar is 2s, so bar 3 is at 5.5s.
    let cue = show.cues[0].time.as_secs_f64();
    assert!(
        (cue - 5.5).abs() < 0.05,
        "bar 3 should account for the 1.5s lead-in, got {cue}"
    );
}
