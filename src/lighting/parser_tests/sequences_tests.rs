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
//! Sequences, as the player's timeline and its analysis reckon their ends.

use std::time::Duration;

use crate::lighting::parser::*;

/// #534: at 180 BPM a beat is a third of a second, which whole nanoseconds
/// cannot hold. A looped one-bar sequence must still start every pass exactly
/// on its bar (no creep), and effects lasting to the next downbeat must end
/// exactly where the next cue starts (no nanosecond gap).
#[test]
fn test_looped_sequence_stays_on_the_grid_to_the_nanosecond() {
    let content = r#"
tempo {
    start: 29ms
    bpm: 180
    time_signature: 4/4
}

sequence "bar" {
    @1/1
    front_wash: static, color: "blue", duration: 1measure
    @1/4
    back_wash: static, color: "white", duration: 1beat
}

show "Test" {
    @2/1
    sequence "bar", loop: 8
    @10/1
    front_wash: static, color: "red", duration: 1measure
    back_wash: static, color: "red", duration: 1measure
}
"#;

    let shows = parse_light_shows(content).expect("show parses");
    let show = shows.get("Test").unwrap();
    let tm = show.tempo_map.as_ref().unwrap();

    for bar in 2..=10u32 {
        let downbeat = tm.measure_to_time_with_offset(bar, 1.0, 0, 0.0).unwrap();
        let cue = show
            .cues
            .iter()
            .find(|c| c.time.as_millis() == downbeat.as_millis())
            .unwrap_or_else(|| panic!("no cue near bar {bar}"));
        assert_eq!(cue.time, downbeat, "bar {bar} drifted off its downbeat");
    }

    let cue_times: Vec<Duration> = show.cues.iter().map(|c| c.time).collect();
    for cue in &show.cues {
        if cue.time >= tm.measure_to_time_with_offset(10, 1.0, 0, 0.0).unwrap() {
            continue;
        }
        for effect in &cue.effects {
            let end = cue.time + effect.total_duration();
            assert!(
                cue_times.contains(&end),
                "an effect from {:?} ends at {end:?}, on no cue",
                cue.time
            );
        }
    }

    let analysis = crate::lighting::analyze::analyze_show(shows.into_values().collect(), None);
    assert!(
        analysis.dark_windows.is_empty(),
        "abutting effects reported as dark: {:?}",
        analysis.dark_windows
    );
}

/// When each effect of `content`'s one show really ends, by cue order.
fn effect_ends(content: &str) -> Vec<Duration> {
    let shows = parse_light_shows(content).expect("show parses");
    let show = shows.values().next().unwrap();
    crate::lighting::timeline::effect_ends(&show.cues)
        .into_iter()
        .map(|end| end.until())
        .collect()
}

#[test]
fn test_a_pass_stops_the_inner_sequences_of_the_pass_before() {
    // Each pass of "outer" stops the pass before it, inner chase included:
    // the chases of passes 1 and 2 end where the next pass starts (2s, 4s),
    // and the show's `stop sequence "outer"` at @4/1 ends pass 3's at 6s,
    // with it still running.
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
}

sequence "inner" {
    @1/1
    x: chase, speed: 1beats, duration: 16beats, blend_mode: multiply
}

sequence "outer", length: 1measure {
    @1/1
    sequence "inner"
    o: static, color: "white", duration: 1s
}

show "Show" {
    @1/1
    sequence "outer", loop: 3

    @4/1
    stop sequence "outer"
}
"#;
    let ends = effect_ends(content);
    // Cue by cue: pass 1 (o, x), pass 2, pass 3; o lasts its second.
    assert_eq!(
        ends,
        vec![
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(3),
            Duration::from_secs(4),
            Duration::from_secs(5),
            Duration::from_secs(6),
        ]
    );
}

#[test]
fn test_stop_outer_ends_only_the_inner_started_through_it() {
    // "inner" started by the show itself, then through "outer": `stop
    // sequence "outer"` at 4s ends the second only; `stop sequence
    // "inner"` at 6s ends the first.
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
}

sequence "inner" {
    @1/1
    x: static, color: "red", duration: 16beats
}

sequence "outer" {
    @1/1
    sequence "inner"
}

show "Show" {
    @1/1
    sequence "inner"

    @2/1
    sequence "outer"

    @3/1
    stop sequence "outer"

    @4/1
    stop sequence "inner"
}
"#;
    assert_eq!(
        effect_ends(content),
        vec![Duration::from_secs(6), Duration::from_secs(4)]
    );
}

#[test]
fn test_stop_inner_from_the_show_ends_every_instance_started_through_it() {
    // "inner" started through "outer" is stopped by the outer's own `stop
    // sequence "inner"` at @2/1 (2s); the stop written in the same cue as
    // the start is a pass boundary and ends nothing. Started by the show
    // at @4/1 after a stop, it runs its 4s.
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
}

sequence "inner" {
    @1/1
    x: static, color: "red", duration: 8beats
}

sequence "outer" {
    @1/1
    sequence "inner"
    stop sequence "inner"
    o: static, color: "green", duration: 1s

    @2/1
    stop sequence "inner"
}

show "Show" {
    @1/1
    sequence "outer"

    @3/1
    stop sequence "inner"

    @4/1
    sequence "inner"
}
"#;
    assert_eq!(
        effect_ends(content),
        vec![
            Duration::from_secs(1),
            Duration::from_secs(2),
            Duration::from_secs(10)
        ]
    );
}
