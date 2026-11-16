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
use crate::lighting::parser::grammar::{LightingParser, Rule};
use pest::Parser;

// ========================================================================
// BEAT/MEASURE-BASED EFFECT DURATIONS
// ========================================================================

#[test]
fn test_effect_duration_in_beats() {
    // Test effect durations specified in beats
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Beat Durations" {
    @1/1
    front_wash: pulse color: "blue", duration: 4beats
    
    @2/1
    back_wash: static color: "red", duration: 2beats
}"#;

    let result = LightingParser::parse(Rule::file, content);
    if let Err(e) = &result {
        println!("Beat durations parsing error: {}", e);
    }
    assert!(result.is_ok(), "Effect durations in beats should parse");
    println!(
        "Note: Implementation must convert beat durations to absolute time using active tempo.\n\
         At 120 BPM: 1 beat = 0.5s, so 4beats = 2s"
    );
}

#[test]
fn test_effect_duration_in_measures() {
    // Test effect durations specified in measures
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Measure Durations" {
    @1/1
    front_wash: cycle color: "red", color: "blue", duration: 2measures, loop: loop
    
    @4/1
    back_wash: static color: "green", duration: 1measures
}"#;

    let result = LightingParser::parse(Rule::file, content);
    if let Err(e) = &result {
        println!("Measure durations parsing error: {}", e);
    }
    assert!(result.is_ok(), "Effect durations in measures should parse");
    println!(
        "Note: Implementation must convert measure durations to absolute time.\n\
         At 120 BPM in 4/4: 1 measure = 2s, so 2measures = 4s"
    );
}

#[test]
fn test_fractional_beat_durations() {
    // Test fractional beat and measure durations
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Fractional Durations" {
    @1/1
    front_wash: pulse color: "blue", duration: 2.5beats
    
    @2/1
    back_wash: static color: "red", duration: 1.5measures
    
    @4/1
    side_wash: strobe frequency: 4, duration: 0.5beats
}"#;

    let result = LightingParser::parse(Rule::file, content);
    if let Err(e) = &result {
        println!("Fractional beat/measure durations parsing error: {}", e);
    }
    assert!(
        result.is_ok(),
        "Fractional beat/measure durations should parse"
    );
}

#[test]
fn test_beat_based_fade_times() {
    // Test beat-based fade times (up_time, down_time, fade_in, fade_out)
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Beat Fades" {
    @1/1
    front_wash: static color: "blue", up_time: 2beats
    
    @3/1
    back_wash: static color: "red", up_time: 1beats, down_time: 1beats, duration: 4beats
    
    @8/1
    side_wash: pulse color: "green", duration: 2measures
}"#;

    let result = LightingParser::parse(Rule::file, content);
    if let Err(e) = &result {
        println!("Beat-based fade times parsing error: {}", e);
    }
    assert!(result.is_ok(), "Beat-based fade times should parse");
    println!("Note: Crossfade times in beats allow effects to fade musically in sync.");
}

#[test]
fn test_beat_durations_across_tempo_change() {
    // Test that beat durations work across tempo changes
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
    @8/1 { bpm: 160, transition: 4 }
    ]
}

show "Beat Duration Tempo Change" {
    @7/1
    front_wash: pulse color: "blue", duration: 4beats
    
    @9/1
    back_wash: static color: "red", duration: 4beats
}"#;

    let result = LightingParser::parse(Rule::file, content);
    if let Err(e) = &result {
        println!("Beat durations across tempo change parsing error: {}", e);
    }
    assert!(
        result.is_ok(),
        "Beat durations across tempo changes should parse"
    );
    println!(
        "Note: Implementation must handle tempo-aware durations:\n\
         - Effect at @7/1 starts at BPM 120, lasts 4 beats\n\
         - During this effect, tempo begins transitioning at @8/1\n\
         - Effect at @9/1 starts after tempo is fully 160 BPM\n\
         - Each effect's 4beats will have different absolute durations!"
    );
}

#[test]
fn test_measure_durations_across_time_signature_change() {
    // Test measure durations across time signature changes
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
    changes: [
    @8/1 { time_signature: 3/4 }
    ]
}

show "Measure Duration Time Sig Change" {
    @1/1
    front_wash: cycle color: "red", color: "blue", duration: 2measures, loop: loop
    
    @9/1
    back_wash: cycle color: "green", color: "yellow", duration: 2measures, loop: loop
}"#;

    let result = LightingParser::parse(Rule::file, content);
    if let Err(e) = &result {
        println!(
            "Measure durations across time signature change parsing error: {}",
            e
        );
    }
    assert!(
        result.is_ok(),
        "Measure durations across time signature changes should parse"
    );
    println!(
        "Note: Implementation must handle time signature changes:\n\
         - At @1/1 in 4/4: 2measures = 8 beats = 4s (at 120 BPM)\n\
         - At @9/1 in 3/4: 2measures = 6 beats = 3s (at 120 BPM)\n\
         - Same measure count, different absolute duration!"
    );
}

#[test]
fn test_mixed_time_units_in_effects() {
    // Test mixing absolute time and musical time in the same show
    let content = r#"tempo {
    start: 0.0s
    bpm: 120
    time_signature: 4/4
}

show "Mixed Time Units" {
    @1/1
    front_wash: pulse color: "blue", duration: 500ms
    
    @2/1
    back_wash: static color: "red", duration: 2beats
    
    @3/1
    side_wash: strobe frequency: 4, duration: 1measures
    
    @5/1
    top_wash: static color: "green", duration: 2s
}"#;

    let result = LightingParser::parse(Rule::file, content);
    if let Err(e) = &result {
        println!("Mixed time units parsing error: {}", e);
    }
    assert!(
        result.is_ok(),
        "Mixing absolute and musical time units should parse"
    );
    println!(
        "Note: Implementation supports both:\n\
         - Absolute time: ms, s (fixed duration regardless of tempo)\n\
         - Musical time: beats, measures (duration adapts to tempo)"
    );
}

#[test]
fn test_beat_duration_without_tempo_section() {
    // Test that beat/measure durations require semantic validation
    let content = r#"show "No Tempo Section" {
    @00:00.000
    front_wash: pulse color: "blue", duration: 4beats
}"#;

    let result = LightingParser::parse(Rule::file, content);
    assert!(
        result.is_ok(),
        "Beat durations without tempo section parse syntactically"
    );
    println!(
        "Note: Implementation should require tempo section when using beat/measure durations.\n\
     Grammar allows it syntactically, but semantic validation should enforce:\n\
     - If any effect uses beats/measures, tempo section must exist\n\
     - Should raise helpful error: 'Beat-based duration requires tempo section'"
    );
}

// ============================================
// END-TO-END FUNCTIONALITY TESTS
// These tests verify that the parsed show actually works correctly,
// not just that it parses.
// ============================================
