// Copyright (C) 2025 Michael Wilson <mike@mdwn.dev>
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
use crate::lighting::parser::*;
use std::time::Duration;

#[test]
fn test_sequence_definition_and_reference() {
    let content = r#"
sequence "color_cycle" {
    @0.000
    front_wash: static, color: "red"
    
    @2.000
    front_wash: static, color: "green"
    
    @4.000
    front_wash: static, color: "blue"
}

show "Test Show" {
    @0.000
    sequence "color_cycle"
    
    @6.000
    sequence "color_cycle"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse shows with sequences: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    assert_eq!(shows.len(), 1);

    let show = shows.get("Test Show").unwrap();
    // The sequence should be expanded into 6 cues (3 from first reference, 3 from second)
    assert_eq!(show.cues.len(), 6);

    // First sequence reference: cues at 0.000, 2.000, 4.000
    assert_eq!(show.cues[0].time, Duration::from_millis(0));
    assert_eq!(show.cues[1].time, Duration::from_millis(2000));
    assert_eq!(show.cues[2].time, Duration::from_millis(4000));

    // Second sequence reference: cues at 6.000, 8.000, 10.000
    assert_eq!(show.cues[3].time, Duration::from_millis(6000));
    assert_eq!(show.cues[4].time, Duration::from_millis(8000));
    assert_eq!(show.cues[5].time, Duration::from_millis(10000));
}

#[test]
fn test_sequence_with_effects_in_same_cue() {
    let content = r#"
sequence "simple_sequence" {
    @0.000
    front_wash: static, color: "red"
    
    @1.000
    front_wash: static, color: "blue"
}

show "Test Show" {
    @5.000
    back_wash: static, color: "green"
    sequence "simple_sequence"
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_ok(), "Failed to parse shows: {:?}", result.err());

    let shows = result.unwrap();
    let show = shows.get("Test Show").unwrap();

    // Should have 3 cues: one with the effect, plus 2 from the sequence
    // The effect should be added to the first expanded cue
    assert_eq!(show.cues.len(), 2);

    // First cue: at 5.000 with both the effect and the first sequence cue
    assert_eq!(show.cues[0].time, Duration::from_millis(5000));
    assert_eq!(show.cues[0].effects.len(), 2); // green effect + red from sequence

    // Second cue: at 6.000 with the second sequence cue
    assert_eq!(show.cues[1].time, Duration::from_millis(6000));
    assert_eq!(show.cues[1].effects.len(), 1); // blue from sequence
}

#[test]
fn test_sequence_not_found_error() {
    let content = r#"
show "Test Show" {
    @0.000
    sequence "nonexistent_sequence"
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_err(), "Should fail when sequence is not found");

    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("not found"),
        "Error should mention sequence not found"
    );
}

#[test]
fn test_sequence_with_measure_based_timing() {
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
}

sequence "measure_based_sequence" {
    @1/1
    front_wash: static, color: "red"
    
    @1/3
    front_wash: static, color: "green"
    
    @2/1
    front_wash: static, color: "blue"
}

show "Test Show" {
    @0.000
    sequence "measure_based_sequence"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse shows with measure-based sequence: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test Show").unwrap();

    // Sequence should be expanded into 3 cues
    assert_eq!(show.cues.len(), 3);

    // At 120 BPM, 4/4 time:
    // Measure 1, beat 1 = 0.0s
    // Measure 1, beat 3 = 1.0s (2 beats at 120 BPM = 1 second)
    // Measure 2, beat 1 = 2.0s (4 beats at 120 BPM = 2 seconds)
    // Since the sequence is referenced at 0.000, the times should be offset by 0

    // First cue: measure 1, beat 1 = 0.0s
    assert_eq!(show.cues[0].time, Duration::from_secs(0));

    // Second cue: measure 1, beat 3 = 1.0s
    assert_eq!(show.cues[1].time, Duration::from_secs(1));

    // Third cue: measure 2, beat 1 = 2.0s
    assert_eq!(show.cues[2].time, Duration::from_secs(2));
}

#[test]
fn test_sequence_with_own_tempo_and_measure_timing() {
    let content = r#"
sequence "sequence_with_tempo" {
    tempo {
    bpm: 60
    time_signature: 4/4
    }
    
    @1/1
    front_wash: static, color: "red"
    
    @2/1
    front_wash: static, color: "blue"
}

show "Test Show" {
    @0.000
    sequence "sequence_with_tempo"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse shows with sequence having own tempo: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test Show").unwrap();

    // Sequence should be expanded into 2 cues
    assert_eq!(show.cues.len(), 2);

    // At 60 BPM, 4/4 time:
    // Measure 1, beat 1 = 0.0s
    // Measure 2, beat 1 = 4.0s (4 beats at 60 BPM = 4 seconds)

    // First cue: measure 1, beat 1 = 0.0s
    assert_eq!(show.cues[0].time, Duration::from_secs(0));

    // Second cue: measure 2, beat 1 = 4.0s
    assert_eq!(show.cues[1].time, Duration::from_secs(4));
}

#[test]
fn test_sequence_measure_timing_with_offset() {
    let content = r#"
tempo {
    bpm: 120
    time_signature: 4/4
}

sequence "measure_sequence" {
    @1/1
    front_wash: static, color: "red"
    
    @1/3
    front_wash: static, color: "green"
}

show "Test Show" {
    @10.000
    sequence "measure_sequence"
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let shows = result.unwrap();
    let show = shows.get("Test Show").unwrap();

    // Sequence should be expanded with times offset by 10 seconds
    assert_eq!(show.cues.len(), 2);

    // At 120 BPM:
    // Measure 1, beat 1 = 0.0s, offset to 10.0s
    // Measure 1, beat 3 = 1.0s, offset to 11.0s

    assert_eq!(show.cues[0].time, Duration::from_secs(10));
    assert_eq!(show.cues[1].time, Duration::from_secs(11));
}

#[test]
fn test_sequence_looping_finite() {
    let content = r#"
sequence "simple_sequence" {
    @0.000
    front_wash: static, color: "red"
    
    @1.000
    front_wash: static, color: "blue"
}

show "Test Show" {
    @0.000
    sequence "simple_sequence", loop: 3
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let shows = result.unwrap();
    let show = shows.get("Test Show").unwrap();

    // Should have 6 cues (2 cues per iteration × 3 iterations)
    assert_eq!(show.cues.len(), 6);

    // First iteration: 0s, 1s
    assert_eq!(show.cues[0].time, Duration::from_secs(0));
    assert_eq!(show.cues[1].time, Duration::from_secs(1));

    // Second iteration: 1s (last cue time), 2s
    // Sequence duration is 1s (last cue time since effects are perpetual)
    assert_eq!(show.cues[2].time, Duration::from_secs(1));
    assert_eq!(show.cues[3].time, Duration::from_secs(2));

    // Third iteration: 2s, 3s
    assert_eq!(show.cues[4].time, Duration::from_secs(2));
    assert_eq!(show.cues[5].time, Duration::from_secs(3));

    // All effects should be marked with sequence name
    for cue in &show.cues {
        for effect in &cue.effects {
            assert_eq!(effect.sequence_name, Some("simple_sequence".to_string()));
        }
    }
}

#[test]
fn test_sequence_looping_infinite() {
    let content = r#"
sequence "infinite_sequence" {
    @0.000
    front_wash: static, color: "red"
    
    @1.000
    front_wash: static, color: "blue"
}

show "Test Show" {
    @0.000
    sequence "infinite_sequence", loop: loop
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let shows = result.unwrap();
    let show = shows.get("Test Show").unwrap();

    // Should have 20000 cues (2 cues per iteration × 10000 iterations)
    assert_eq!(show.cues.len(), 20000);

    // Check first few iterations
    // Sequence duration is 1s (last cue time since effects are perpetual)
    assert_eq!(show.cues[0].time, Duration::from_secs(0));
    assert_eq!(show.cues[1].time, Duration::from_secs(1));
    assert_eq!(show.cues[2].time, Duration::from_secs(1)); // Second iteration starts at 1s
    assert_eq!(show.cues[3].time, Duration::from_secs(2));
}

#[test]
fn test_sequence_looping_once() {
    let content = r#"
sequence "once_sequence" {
    @0.000
    front_wash: static, color: "red"
    
    @1.000
    front_wash: static, color: "blue"
}

show "Test Show" {
    @0.000
    sequence "once_sequence", loop: once
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let shows = result.unwrap();
    let show = shows.get("Test Show").unwrap();

    // Should have 2 cues (default behavior, same as no loop parameter)
    assert_eq!(show.cues.len(), 2);
}

#[test]
fn test_stop_sequence_command() {
    let content = r#"
sequence "looping_sequence" {
    @0.000
    front_wash: static, color: "red"
    
    @1.000
    front_wash: static, color: "blue"
}

show "Test Show" {
    @0.000
    sequence "looping_sequence", loop: loop
    
    @10.000
    stop sequence "looping_sequence"
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let shows = result.unwrap();
    let show = shows.get("Test Show").unwrap();

    // Find the cue at 10 seconds with stop_sequences
    // Note: The looping sequence may also create a cue at 10 seconds, so we need to find the one with stop_sequences
    let stop_cue = show
        .cues
        .iter()
        .find(|c| c.time == Duration::from_secs(10) && !c.stop_sequences.is_empty());
    let cue_times: Vec<_> = show.cues.iter().map(|c| c.time).collect();
    let cues_at_10: Vec<_> = show
        .cues
        .iter()
        .filter(|c| c.time == Duration::from_secs(10))
        .map(|c| (c.time, c.stop_sequences.clone(), c.effects.len()))
        .collect();
    assert!(
        stop_cue.is_some(),
        "Should have a cue at 10 seconds with stop_sequences. Cue times: {:?}, Cues at 10s: {:?}",
        cue_times,
        cues_at_10
    );

    let stop_cue = stop_cue.unwrap();
    assert_eq!(
        stop_cue.stop_sequences,
        vec!["looping_sequence"],
        "Stop sequences: {:?}",
        stop_cue.stop_sequences
    );
}

#[test]
fn test_stop_multiple_sequences() {
    let content = r#"
sequence "seq1" {
    @0.000
    front_wash: static, color: "red"
}

sequence "seq2" {
    @0.000
    back_wash: static, color: "blue"
}

show "Test Show" {
    @0.000
    sequence "seq1", loop: loop
    sequence "seq2", loop: loop
    
    @5.000
    stop sequence "seq1"
    stop sequence "seq2"
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_ok(), "Failed to parse: {:?}", result.err());

    let shows = result.unwrap();
    let show = shows.get("Test Show").unwrap();

    // Find the cue at 5 seconds with stop_sequences
    // Note: The looping sequences may also create cues at 5 seconds, so we need to find the one with stop_sequences
    let stop_cue = show
        .cues
        .iter()
        .find(|c| c.time == Duration::from_secs(5) && !c.stop_sequences.is_empty());
    let cue_times: Vec<_> = show.cues.iter().map(|c| c.time).collect();
    let cues_at_5: Vec<_> = show
        .cues
        .iter()
        .filter(|c| c.time == Duration::from_secs(5))
        .map(|c| (c.time, c.stop_sequences.clone(), c.effects.len()))
        .collect();
    assert!(
        stop_cue.is_some(),
        "Should have a cue at 5 seconds with stop_sequences. Cue times: {:?}, Cues at 5s: {:?}",
        cue_times,
        cues_at_5
    );

    let stop_cue = stop_cue.unwrap();
    assert_eq!(stop_cue.stop_sequences.len(), 2);
    assert!(stop_cue.stop_sequences.contains(&"seq1".to_string()));
    assert!(stop_cue.stop_sequences.contains(&"seq2".to_string()));
}

#[test]
fn test_nested_sequences() {
    let content = r#"
sequence "base_pattern" {
    @0.000
    front_wash: static, color: "red"
    
    @1.000
    front_wash: static, color: "blue"
}

sequence "complex_pattern" {
    @0.000
    sequence "base_pattern"
    back_wash: static, color: "green"
    
    @3.000
    sequence "base_pattern"
}

show "Test Show" {
    @0.000
    sequence "complex_pattern"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse nested sequences: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test Show").unwrap();

    // complex_pattern expands to:
    // - base_pattern at 0s (2 cues: 0s, 1s) + green effect at 0s (merged into first cue)
    // - base_pattern at 3s (2 cues: 3s, 4s)
    // Total: 4 cues (green effect is merged with first base_pattern cue)
    assert_eq!(show.cues.len(), 4);

    // First base_pattern iteration (with green effect merged)
    assert_eq!(show.cues[0].time, Duration::from_secs(0));
    assert_eq!(show.cues[1].time, Duration::from_secs(1));

    // Second base_pattern iteration
    assert_eq!(show.cues[2].time, Duration::from_secs(3));
    assert_eq!(show.cues[3].time, Duration::from_secs(4));
}

#[test]
fn test_circular_sequence_reference() {
    // With two-pass parsing, forward references are now supported
    // The circular reference will be detected when seq_a tries to expand seq_b,
    // which then tries to expand seq_a (already in recursion stack)
    let content = r#"
sequence "seq_a" {
    @0.000
    front_wash: static, color: "red"
    @1.000
    sequence "seq_b"
}

sequence "seq_b" {
    @0.000
    sequence "seq_a"
}

show "Test Show" {
    @0.000
    sequence "seq_a"
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_err(), "Should fail with circular reference");

    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("Circular sequence reference"),
        "Error should mention circular reference: {}",
        error
    );
}

#[test]
fn test_nested_sequences_measure_timing() {
    // Test that nested sequences with measure-based timing work correctly
    // When "verse" is referenced at @17/1, its @1/1 cue should map to @17/1 in the show
    let content = r#"tempo {
    start: 3.0s
    bpm: 160
    time_signature: 4/4
}

sequence "verse-start" {
    @1/1
    all_wash: static, color: "white"
}

sequence "verse" {
    @1/1
    sequence "verse-start", loop: 1
    @13/1
    all_wash: static, color: "red"
}

show "Test" {
    @17/1
    sequence "verse"
}
"#;

    let result = parse_light_shows(content);
    assert!(
        result.is_ok(),
        "Failed to parse nested sequences with measure timing: {:?}",
        result.err()
    );

    let shows = result.unwrap();
    let show = shows.get("Test").unwrap();

    // At 160 BPM in 4/4: 1 beat = 0.375s, 1 measure = 1.5s
    // Measure 17, beat 1 = 3.0s (start offset) + (16 measures * 1.5s) = 3.0s + 24.0s = 27.0s
    // "verse-start" should start at measure 17 of the show = 27.0s
    // "verse" @13/1 should be at measure 17 + 12 measures = measure 29 = 3.0s + (28 * 1.5s) = 3.0s + 42.0s = 45.0s

    // Find the first cue (should be verse-start at 27.0s)
    // verse-start should start at measure 17 = 27.0s
    let expected_time = Duration::from_secs_f64(27.0);
    assert!(!show.cues.is_empty(), "Should have at least one cue");
    let first_cue_time = show.cues[0].time;
    assert!(
        (first_cue_time.as_secs_f64() - expected_time.as_secs_f64()).abs() < 0.001,
        "verse-start should start at measure 17 (27.0s), got {:?}",
        first_cue_time
    );
}

#[test]
fn test_self_referencing_sequence() {
    let content = r#"
sequence "self_ref" {
    @0.000
    sequence "self_ref"
}

show "Test Show" {
    @0.000
    sequence "self_ref"
}
"#;

    let result = parse_light_shows(content);
    assert!(result.is_err(), "Should fail with self-reference");

    let error = result.unwrap_err();
    assert!(
        error.to_string().contains("Circular sequence reference"),
        "Error should mention circular reference: {}",
        error
    );
}
